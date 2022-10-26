use super::{broadcast_p2p_tx_msg, lp_coinfind, tx_helper_topic, wait_for_taker_payment_conf_until, H256Json,
            SwapsContext, TransactionIdentifier, WAIT_CONFIRM_INTERVAL};
use crate::mm2::MmError;
use async_trait::async_trait;
use coins::{CanRefundHtlc, FoundSwapTxSpend, MmCoinEnum, WatcherSearchForSwapTxSpendInput, WatcherValidatePaymentInput};
use common::executor::{AbortSettings, SpawnAbortable, Timer};
use common::log::{error, info};
use common::state_machine::prelude::*;
use futures::compat::Future01CompatExt;
use mm2_core::mm_ctx::MmArc;
use mm2_libp2p::{decode_signed, pub_sub_topic, TopicPrefix};
use mm2_number::BigDecimal;
use rpc::v1::types::Bytes as BytesJson;
use std::cmp::min;
use std::sync::Arc;
use uuid::Uuid;

#[cfg(not(test))] use common::now_ms;

pub const WATCHER_PREFIX: TopicPrefix = "swpwtchr";
const TAKER_SWAP_CONFIRMATIONS: u64 = 1;
pub const TAKER_SWAP_ENTRY_TIMEOUT: u64 = 21600;
const WAIT_FOR_TAKER_REFUND: u64 = 1200; // How long?

struct WatcherContext {
    ctx: MmArc,
    taker_coin: MmCoinEnum,
    maker_coin: MmCoinEnum,
    data: TakerSwapWatcherData,
    verified_pub: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SwapWatcherMsg {
    TakerSwapWatcherMsg(TakerSwapWatcherData),
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TakerSwapWatcherData {
    pub uuid: Uuid,
    pub secret_hash: Vec<u8>,
    pub taker_spends_maker_payment_preimage: Vec<u8>,
    pub taker_refunds_payment: Vec<u8>,
    pub swap_started_at: u64,
    pub lock_duration: u64,
    pub taker_coin: String,
    pub taker_fee_hash: Vec<u8>,
    pub taker_payment_hex: Vec<u8>,
    pub taker_payment_lock: u64,
    pub taker_pub: Vec<u8>,
    pub taker_coin_start_block: u64,
    pub taker_payment_confirmations: u64,
    pub taker_payment_requires_nota: Option<bool>,
    pub taker_amount: BigDecimal,
    pub maker_coin: String,
    pub maker_pub: Vec<u8>,
}

struct ValidatePublicKeys {}
struct ValidateTakerFee {}
struct ValidateTakerPayment {}
struct WaitForTakerPaymentSpend {}

struct RefundTakerPayment {}

struct SpendMakerPayment {
    secret: H256Json,
}

impl SpendMakerPayment {
    fn new(secret: H256Json) -> Self { SpendMakerPayment { secret } }
}

struct Stopped {
    _stop_reason: StopReason,
}

#[derive(Debug)]
enum StopReason {
    Finished(WatcherSuccess),
    Error(MmError<WatcherError>),
}

#[derive(Debug)]
enum WatcherSuccess {
    MakerPaymentSpent,
    TakerPaymentRefunded,
    TakerPaymentAlreadySpent,
    TakerPaymentAlreadyRefunded,
}

#[derive(Debug)]
enum WatcherError {
    InvalidValidatePublicKey(String),
    InvalidTakerFee(String),
    TakerPaymentNotConfirmed(String),
    TakerPaymentSearchForSwapFailed(String),
    InvalidTakerPayment(String),
    UnableToExtractSecret(String),
    MakerPaymentSpendFailed(String),
    TakerPaymentRefundFailed(String),
}

impl Stopped {
    fn from_reason(stop_reason: StopReason) -> Stopped {
        Stopped {
            _stop_reason: stop_reason,
        }
    }
}

impl TransitionFrom<ValidatePublicKeys> for ValidateTakerFee {}
impl TransitionFrom<ValidateTakerFee> for ValidateTakerPayment {}
impl TransitionFrom<ValidateTakerPayment> for WaitForTakerPaymentSpend {}
impl TransitionFrom<WaitForTakerPaymentSpend> for SpendMakerPayment {}
impl TransitionFrom<WaitForTakerPaymentSpend> for RefundTakerPayment {}
impl TransitionFrom<ValidatePublicKeys> for Stopped {}
impl TransitionFrom<ValidateTakerFee> for Stopped {}
impl TransitionFrom<ValidateTakerPayment> for Stopped {}
impl TransitionFrom<WaitForTakerPaymentSpend> for Stopped {}
impl TransitionFrom<RefundTakerPayment> for Stopped {}
impl TransitionFrom<SpendMakerPayment> for Stopped {}

#[async_trait]
impl State for ValidatePublicKeys {
    type Ctx = WatcherContext;
    type Result = ();

    async fn on_changed(self: Box<Self>, watcher_ctx: &mut WatcherContext) -> StateResult<Self::Ctx, Self::Result> {
        let redeem_pub_valid = match watcher_ctx
            .taker_coin
            .check_tx_signed_by_pub(&watcher_ctx.data.taker_payment_hex, &watcher_ctx.verified_pub)
        {
            Ok(is_valid) => is_valid,
            Err(err) => {
                return Self::change_state(Stopped::from_reason(StopReason::Error(
                    WatcherError::InvalidValidatePublicKey(err).into(),
                )))
            },
        };

        if !redeem_pub_valid || watcher_ctx.verified_pub != watcher_ctx.data.taker_pub {
            return Self::change_state(Stopped::from_reason(StopReason::Error(
                WatcherError::InvalidValidatePublicKey("Public key does not belong to taker payment".to_string())
                    .into(),
            )));
        }

        Self::change_state(ValidateTakerFee {})
    }
}

#[async_trait]
impl State for ValidateTakerFee {
    type Ctx = WatcherContext;
    type Result = ();

    async fn on_changed(self: Box<Self>, watcher_ctx: &mut WatcherContext) -> StateResult<Self::Ctx, Self::Result> {
        let validated_f = watcher_ctx
            .taker_coin
            .watcher_validate_taker_fee(
                watcher_ctx.data.taker_fee_hash.clone(),
                watcher_ctx.verified_pub.clone(),
            )
            .compat();
        if let Err(err) = validated_f.await {
            Self::change_state(Stopped::from_reason(StopReason::Error(
                WatcherError::InvalidTakerFee(err.to_string()).into(),
            )));
        }
        Self::change_state(ValidateTakerPayment {})
    }
}

// TODO: Do this check periodically while waiting for taker payment spend
#[async_trait]
impl State for ValidateTakerPayment {
    type Ctx = WatcherContext;
    type Result = ();

    async fn on_changed(self: Box<Self>, watcher_ctx: &mut WatcherContext) -> StateResult<Self::Ctx, Self::Result> {
        let search_input = WatcherSearchForSwapTxSpendInput {
            time_lock: watcher_ctx.data.taker_payment_lock as u32,
            taker_pub: &watcher_ctx.data.taker_pub,
            maker_pub: &watcher_ctx.data.maker_pub,
            secret_hash: &watcher_ctx.data.secret_hash,
            tx: &watcher_ctx.data.taker_payment_hex,
            search_from_block: watcher_ctx.data.taker_coin_start_block,
            swap_contract_address: &None,
        };

        match watcher_ctx
            .taker_coin
            .watcher_search_for_swap_tx_spend(search_input)
            .await
        {
            Ok(Some(FoundSwapTxSpend::Spent(_))) => {
                return Self::change_state(Stopped::from_reason(StopReason::Finished(
                    WatcherSuccess::TakerPaymentAlreadySpent,
                )))
            },
            Ok(Some(FoundSwapTxSpend::Refunded(_))) => {
                return Self::change_state(Stopped::from_reason(StopReason::Finished(
                    WatcherSuccess::TakerPaymentAlreadyRefunded,
                )))
            },
            Err(err) => {
                return Self::change_state(Stopped::from_reason(StopReason::Error(
                    WatcherError::TakerPaymentSearchForSwapFailed(err).into(),
                )))
            },
            Ok(None) => (),
        }

        let wait_taker_payment =
            wait_for_taker_payment_conf_until(watcher_ctx.data.swap_started_at, watcher_ctx.data.lock_duration);
        let confirmations = min(watcher_ctx.data.taker_payment_confirmations, TAKER_SWAP_CONFIRMATIONS);

        let wait_f = watcher_ctx
            .taker_coin
            .wait_for_confirmations(
                &watcher_ctx.data.taker_payment_hex,
                confirmations,
                watcher_ctx.data.taker_payment_requires_nota.unwrap_or(false),
                wait_taker_payment,
                WAIT_CONFIRM_INTERVAL,
            )
            .compat();
        if let Err(err) = wait_f.await {
            Self::change_state(Stopped::from_reason(StopReason::Error(
                WatcherError::TakerPaymentNotConfirmed(err).into(),
            )));
        }

        let validate_input = WatcherValidatePaymentInput {
            payment_tx: watcher_ctx.data.taker_payment_hex.clone(),
            time_lock: watcher_ctx.data.taker_payment_lock as u32,
            taker_pub: watcher_ctx.data.taker_pub.clone(),
            maker_pub: watcher_ctx.data.maker_pub.clone(),
            secret_hash: watcher_ctx.data.secret_hash.clone(),
            amount: watcher_ctx.data.taker_amount.clone(),
            try_spv_proof_until: wait_taker_payment,
            confirmations,
        };

        let validated_f = watcher_ctx
            .taker_coin
            .watcher_validate_taker_payment(validate_input)
            .compat();

        if let Err(err) = validated_f.await {
            Self::change_state(Stopped::from_reason(StopReason::Error(
                WatcherError::InvalidTakerPayment(err.to_string()).into(),
            )));
        }

        Self::change_state(WaitForTakerPaymentSpend {})
    }
}

#[async_trait]
impl State for WaitForTakerPaymentSpend {
    type Ctx = WatcherContext;
    type Result = ();

    async fn on_changed(self: Box<Self>, watcher_ctx: &mut WatcherContext) -> StateResult<Self::Ctx, Self::Result> {
        #[cfg(not(test))]
        {
            // Sleep for half the locktime to allow the taker to spend the maker payment first
            let now = now_ms() / 1000;
            let wait_for_taker_until =
                wait_for_taker_payment_conf_until(watcher_ctx.data.swap_started_at, watcher_ctx.data.lock_duration);
            let sleep_duration = (wait_for_taker_until - now + 1) as f64;

            if now < wait_for_taker_until {
                Timer::sleep(sleep_duration).await;
            }
        }

        let f = watcher_ctx.taker_coin.wait_for_htlc_tx_spend(
            &watcher_ctx.data.taker_payment_hex,
            &[],
            watcher_ctx.data.taker_payment_lock,
            watcher_ctx.data.taker_coin_start_block,
            &None,
        );

        let tx = match f.compat().await {
            Ok(t) => t,
            Err(err) => {
                error!("{}", err.get_plain_text_format());
                return Self::change_state(RefundTakerPayment {});
            },
        };

        let tx_hash = tx.tx_hash();
        info!("Taker payment spend tx {:02x}", tx_hash);
        let tx_ident = TransactionIdentifier {
            tx_hex: BytesJson::from(tx.tx_hex()),
            tx_hash,
        };

        let secret = match watcher_ctx
            .taker_coin
            .extract_secret(&watcher_ctx.data.secret_hash, &tx_ident.tx_hex.0)
            .await
        {
            Ok(bytes) => H256Json::from(bytes.as_slice()),
            Err(err) => {
                return Self::change_state(Stopped::from_reason(StopReason::Error(
                    WatcherError::UnableToExtractSecret(err).into(),
                )))
            },
        };

        Self::change_state(SpendMakerPayment::new(secret))
    }
}

#[async_trait]
impl State for SpendMakerPayment {
    type Ctx = WatcherContext;
    type Result = ();

    async fn on_changed(self: Box<Self>, watcher_ctx: &mut WatcherContext) -> StateResult<Self::Ctx, Self::Result> {
        let spend_fut = watcher_ctx.maker_coin.send_taker_spends_maker_payment_preimage(
            &watcher_ctx.data.taker_spends_maker_payment_preimage,
            &self.secret.0,
        );

        let transaction = match spend_fut.compat().await {
            Ok(t) => t,
            Err(err) => {
                if let Some(tx) = err.get_tx() {
                    broadcast_p2p_tx_msg(
                        &watcher_ctx.ctx,
                        tx_helper_topic(watcher_ctx.maker_coin.ticker()),
                        &tx,
                        &None,
                    );
                };
                return Self::change_state(Stopped::from_reason(StopReason::Error(
                    WatcherError::MakerPaymentSpendFailed(err.get_plain_text_format()).into(),
                )));
            },
        };

        broadcast_p2p_tx_msg(
            &watcher_ctx.ctx,
            tx_helper_topic(watcher_ctx.maker_coin.ticker()),
            &transaction,
            &None,
        );

        let tx_hash = transaction.tx_hash();
        info!("Sent maker payment spend tx {:02x} as watcher", tx_hash);

        Self::change_state(Stopped::from_reason(StopReason::Finished(
            WatcherSuccess::MakerPaymentSpent,
        )))
    }
}

#[async_trait]
impl State for RefundTakerPayment {
    type Ctx = WatcherContext;
    type Result = ();

    async fn on_changed(self: Box<Self>, watcher_ctx: &mut WatcherContext) -> StateResult<Self::Ctx, Self::Result> {
        let locktime = watcher_ctx.data.taker_payment_lock;
        loop {
            match watcher_ctx
                .taker_coin
                .can_refund_htlc(locktime + WAIT_FOR_TAKER_REFUND)
                .compat()
                .await
            {
                Ok(CanRefundHtlc::CanRefundNow) => break,
                Ok(CanRefundHtlc::HaveToWait(to_sleep)) => Timer::sleep(to_sleep as f64).await,
                Err(e) => {
                    error!("Error {} on can_refund_htlc, retrying in 30 seconds", e);
                    Timer::sleep(30.).await;
                },
            }
        }

        let refund_fut = watcher_ctx
            .taker_coin
            .send_watcher_refunds_taker_payment_preimage(&watcher_ctx.data.taker_refunds_payment);
        let transaction = match refund_fut.compat().await {
            Ok(t) => t,
            Err(err) => {
                if let Some(tx) = err.get_tx() {
                    broadcast_p2p_tx_msg(
                        &watcher_ctx.ctx,
                        tx_helper_topic(watcher_ctx.taker_coin.ticker()),
                        &tx,
                        &None,
                    );
                }

                return Self::change_state(Stopped::from_reason(StopReason::Error(
                    WatcherError::TakerPaymentRefundFailed(err.get_plain_text_format()).into(),
                )));
            },
        };

        broadcast_p2p_tx_msg(
            &watcher_ctx.ctx,
            tx_helper_topic(watcher_ctx.taker_coin.ticker()),
            &transaction,
            &None,
        );

        let wait_fut = watcher_ctx.taker_coin.wait_for_confirmations(
            &transaction.tx_hex(),
            1,
            false,
            watcher_ctx.data.taker_payment_lock + WAIT_FOR_TAKER_REFUND + 3600,
            WAIT_CONFIRM_INTERVAL,
        );
        if let Err(err) = wait_fut.compat().await {
            return Self::change_state(Stopped::from_reason(StopReason::Error(
                WatcherError::TakerPaymentRefundFailed(err).into(),
            )));
        }

        let tx_hash = transaction.tx_hash();
        info!("Sent taker refund tx {:02x} as watcher", tx_hash);
        Self::change_state(Stopped::from_reason(StopReason::Finished(
            WatcherSuccess::TakerPaymentRefunded,
        )))
    }
}

#[async_trait]
impl LastState for Stopped {
    type Ctx = WatcherContext;
    type Result = ();
    async fn on_changed(self: Box<Self>, _watcher_ctx: &mut Self::Ctx) -> Self::Result {}
}

pub async fn process_watcher_msg(ctx: MmArc, msg: &[u8]) {
    let msg = match decode_signed::<SwapWatcherMsg>(msg) {
        Ok(m) => m,
        Err(watcher_msg_err) => {
            error!("Couldn't deserialize 'SwapWatcherMsg': {:?}", watcher_msg_err);
            // Drop it to avoid dead_code warning
            drop(watcher_msg_err);
            return;
        },
    };

    let watcher_data = msg.0;
    let verified_pubkey = msg.2;
    match watcher_data {
        SwapWatcherMsg::TakerSwapWatcherMsg(watcher_data) => {
            spawn_taker_swap_watcher(ctx, watcher_data, verified_pubkey.to_bytes())
        },
    }
}

/// Currently, Taker Swap Watcher is supported only.
enum WatcherType {
    Taker,
}

/// The `SwapWatcherLock` is used to lock the given taker fee hash as the running Swap Watcher,
/// (i.e. insert the fee hash into either [`SwapsContext::taker_swap_watchers`] or [`SwapsContext::maker_swap_watchers`]),
/// and to unlock it (i.e remove the hash from corresponding watcher collection) once `SwapWatcherLock` is dropped.
struct SwapWatcherLock {
    swap_ctx: Arc<SwapsContext>,
    fee_hash: Vec<u8>,
    watcher_type: WatcherType,
}

impl SwapWatcherLock {
    /// Locks the given taker fee hash as the running Swap Watcher,
    /// so inserts the hash into the [`SwapsContext::taker_swap_watchers`] collection.
    ///
    /// Returns `None` if there is an ongoing Taker Swap Watcher already.
    fn lock_taker(swap_ctx: Arc<SwapsContext>, fee_hash: Vec<u8>) -> Option<Self> {
        {
            let mut guard = swap_ctx.taker_swap_watchers.lock();
            if !guard.insert(fee_hash.clone()) {
                // There is the same hash already.
                return None;
            }
        }

        Some(SwapWatcherLock {
            swap_ctx,
            fee_hash,
            watcher_type: WatcherType::Taker,
        })
    }
}

impl Drop for SwapWatcherLock {
    fn drop(&mut self) {
        match self.watcher_type {
            WatcherType::Taker => self.swap_ctx.taker_swap_watchers.lock().remove(self.fee_hash.clone()),
        };
    }
}

fn spawn_taker_swap_watcher(ctx: MmArc, watcher_data: TakerSwapWatcherData, verified_pub: Vec<u8>) {
    let swap_ctx = SwapsContext::from_ctx(&ctx).unwrap();
    if swap_ctx.swap_msgs.lock().unwrap().contains_key(&watcher_data.uuid) {
        return;
    }
    let taker_watcher_lock = match SwapWatcherLock::lock_taker(swap_ctx, watcher_data.taker_fee_hash.clone()) {
        Some(lock) => lock,
        // There is an ongoing Taker Swap Watcher already.
        None => return,
    };

    let spawner = ctx.spawner();
    let fee_hash = H256Json::from(watcher_data.taker_fee_hash.as_slice());

    let fut = async move {
        let taker_coin = match lp_coinfind(&ctx, &watcher_data.taker_coin).await {
            Ok(Some(c)) => c,
            Ok(None) => {
                error!("Coin {} is not found/enabled", watcher_data.taker_coin);
                return;
            },
            Err(e) => {
                error!("!lp_coinfind({}): {}", watcher_data.taker_coin, e);
                return;
            },
        };

        let maker_coin = match lp_coinfind(&ctx, &watcher_data.maker_coin).await {
            Ok(Some(c)) => c,
            Ok(None) => {
                error!("Coin {} is not found/enabled", watcher_data.maker_coin);
                return;
            },
            Err(e) => {
                error!("!lp_coinfind({}): {}", watcher_data.maker_coin, e);
                return;
            },
        };

        log_tag!(
            ctx,
            "";
            fmt = "Entering the taker swap watcher loop {}/{} with taker fee hash: {}",
            maker_coin.ticker(),
            taker_coin.ticker(),
            fee_hash
        );

        let watcher_ctx = WatcherContext {
            ctx,
            maker_coin,
            taker_coin,
            data: watcher_data,
            verified_pub,
        };
        let state_machine: StateMachine<_, ()> = StateMachine::from_ctx(watcher_ctx);
        state_machine.run(ValidatePublicKeys {}).await;

        // This allows to move the `taker_watcher_lock` value into this async block to keep it alive
        // until the Swap Watcher finishes.
        drop(taker_watcher_lock);
    };

    let settings = AbortSettings::info_on_abort(format!("taker swap watcher {fee_hash} stopped!"));
    // Please note that `taker_watcher_lock` will be dropped once `MmCtx` is stopped
    // since this `fut` will be aborted.
    spawner.spawn_with_settings(fut, settings);
}

pub fn watcher_topic(ticker: &str) -> String { pub_sub_topic(WATCHER_PREFIX, ticker) }
