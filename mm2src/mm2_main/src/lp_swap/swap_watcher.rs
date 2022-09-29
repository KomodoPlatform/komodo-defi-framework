use crate::mm2::lp_swap::{broadcast_p2p_tx_msg, lp_coinfind, tx_helper_topic, H256Json, MmCoinEnum, SwapsContext,
                          TransactionIdentifier, WAIT_CONFIRM_INTERVAL};
use async_trait::async_trait;
use coins::{CanRefundHtlc, WatcherValidatePaymentInput};
use common::executor::{spawn, Timer};
use common::log::{self, error, info};
use common::state_machine::prelude::*;
use futures::compat::Future01CompatExt;
use mm2_core::mm_ctx::MmArc;
use mm2_libp2p::{decode_signed, pub_sub_topic, TopicPrefix};
use mm2_number::BigDecimal;
use std::cmp::min;
use uuid::Uuid;

pub const WATCHER_PREFIX: TopicPrefix = "swpwtchr";
const TAKER_SWAP_CONFIRMATIONS: u64 = 1;
pub const TAKER_SWAP_ENTRY_TIMEOUT: u64 = 3600; // How long?

struct WatcherContext {
    uuid: Uuid,
    ctx: MmArc,
    taker_coin: MmCoinEnum,
    maker_coin: MmCoinEnum,
    data: TakerSwapWatcherData,
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

struct Started {}
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
    MakerPaymentSpent,
    TakerPaymentRefunded,
    TakerPaymentWaitConfirmFailed(WatcherError),
    TakerPaymentValidateFailed(WatcherError),
    TakerPaymentWaitForSpendFailed(WatcherError),
    MakerPaymentSpendFailed(WatcherError),
}

impl Stopped {
    fn from_reason(stop_reason: StopReason) -> Stopped {
        Stopped {
            _stop_reason: stop_reason,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct WatcherError {
    error: String,
}

impl From<String> for WatcherError {
    fn from(error: String) -> Self { WatcherError { error } }
}

impl From<&str> for WatcherError {
    fn from(e: &str) -> Self { WatcherError { error: e.to_owned() } }
}

impl Started {}
impl ValidateTakerPayment {}
impl WaitForTakerPaymentSpend {}
impl SpendMakerPayment {}
impl Stopped {}

impl TransitionFrom<Started> for ValidateTakerPayment {}
impl TransitionFrom<ValidateTakerPayment> for WaitForTakerPaymentSpend {}
impl TransitionFrom<WaitForTakerPaymentSpend> for SpendMakerPayment {}
impl TransitionFrom<WaitForTakerPaymentSpend> for RefundTakerPayment {}
impl TransitionFrom<ValidateTakerPayment> for Stopped {}
impl TransitionFrom<WaitForTakerPaymentSpend> for Stopped {}
impl TransitionFrom<RefundTakerPayment> for Stopped {}
impl TransitionFrom<SpendMakerPayment> for Stopped {}

#[async_trait]
impl State for Started {
    type Ctx = WatcherContext;
    type Result = ();

    async fn on_changed(self: Box<Self>, _: &mut WatcherContext) -> StateResult<WatcherContext, ()> {
        Self::change_state(ValidateTakerPayment {})
    }
}

#[async_trait]
impl State for ValidateTakerPayment {
    type Ctx = WatcherContext;
    type Result = ();

    async fn on_changed(self: Box<Self>, watcher_ctx: &mut WatcherContext) -> StateResult<WatcherContext, ()> {
        let wait_duration = (watcher_ctx.data.lock_duration * 4) / 5;
        let wait_taker_payment = watcher_ctx.data.swap_started_at + wait_duration;
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
            Self::change_state(Stopped::from_reason(StopReason::TakerPaymentWaitConfirmFailed(
                ERRL!("!watcher.wait_for_confirmations: {}", err).into(),
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

        if let Err(e) = validated_f.await {
            Self::change_state(Stopped::from_reason(StopReason::TakerPaymentValidateFailed(
                ERRL!("!watcher.watcher_validate_taker_payment: {}", e).into(),
            )));
        }

        Self::change_state(WaitForTakerPaymentSpend {})
    }
}

#[async_trait]
impl State for WaitForTakerPaymentSpend {
    type Ctx = WatcherContext;
    type Result = ();

    async fn on_changed(self: Box<Self>, watcher_ctx: &mut WatcherContext) -> StateResult<WatcherContext, ()> {
        let f = watcher_ctx.taker_coin.wait_for_tx_spend(
            &watcher_ctx.data.taker_payment_hex[..],
            watcher_ctx.data.taker_payment_lock,
            watcher_ctx.data.taker_coin_start_block,
            &None,
        );

        let tx = match f.compat().await {
            Ok(t) => t,
            Err(_) => {
                return Self::change_state(RefundTakerPayment {});
            },
        };

        let tx_hash = tx.tx_hash();
        info!("Taker payment spend tx {:02x}", tx_hash);
        let tx_ident = TransactionIdentifier {
            tx_hex: tx.tx_hex().into(),
            tx_hash,
        };

        let secret = match watcher_ctx
            .taker_coin
            .extract_secret(&watcher_ctx.data.secret_hash[..], &tx_ident.tx_hex.0)
        {
            Ok(bytes) => H256Json::from(bytes.as_slice()),
            Err(e) => {
                return Self::change_state(Stopped::from_reason(StopReason::TakerPaymentWaitForSpendFailed(
                    ERRL!("{}", e).into(),
                )))
            },
        };

        Self::change_state(SpendMakerPayment::new(secret))
    }
}

#[async_trait]
impl State for RefundTakerPayment {
    type Ctx = WatcherContext;
    type Result = ();

    async fn on_changed(self: Box<Self>, watcher_ctx: &mut WatcherContext) -> StateResult<WatcherContext, ()> {
        let locktime = watcher_ctx.data.taker_payment_lock;
        loop {
            match watcher_ctx.taker_coin.can_refund_htlc(locktime).compat().await {
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
            .send_watcher_refunds_taker_payment(&watcher_ctx.data.taker_refunds_payment);
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

                return Self::change_state(Stopped::from_reason(StopReason::MakerPaymentSpendFailed(
                    ERRL!("{}", err.get_plain_text_format()).into(),
                )));
            },
        };

        broadcast_p2p_tx_msg(
            &watcher_ctx.ctx,
            tx_helper_topic(watcher_ctx.taker_coin.ticker()),
            &transaction,
            &None,
        );

        let tx_hash = transaction.tx_hash();
        info!("Taker refund tx hash {:02x}", tx_hash);
        Self::change_state(Stopped::from_reason(StopReason::TakerPaymentRefunded))
    }
}

#[async_trait]
impl State for SpendMakerPayment {
    type Ctx = WatcherContext;
    type Result = ();

    async fn on_changed(self: Box<Self>, watcher_ctx: &mut WatcherContext) -> StateResult<WatcherContext, ()> {
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
                return Self::change_state(Stopped::from_reason(StopReason::MakerPaymentSpendFailed(
                    ERRL!("{}", err.get_plain_text_format()).into(),
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
        info!("Maker payment spend tx {:02x}", tx_hash);
        Self::change_state(Stopped::from_reason(StopReason::MakerPaymentSpent))
    }
}

#[async_trait]
impl LastState for Stopped {
    type Ctx = WatcherContext;
    type Result = ();
    async fn on_changed(self: Box<Self>, watcher_ctx: &mut Self::Ctx) -> Self::Result {
        let swap_ctx = SwapsContext::from_ctx(&watcher_ctx.ctx).unwrap();
        swap_ctx.taker_swap_watchers.lock().remove(watcher_ctx.uuid);
    }
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

    match msg.0 {
        SwapWatcherMsg::TakerSwapWatcherMsg(watcher_data) => spawn_taker_swap_watcher(ctx, watcher_data).await,
    }
}

async fn spawn_taker_swap_watcher(ctx: MmArc, watcher_data: TakerSwapWatcherData) {
    let swap_ctx = SwapsContext::from_ctx(&ctx).unwrap();
    if swap_ctx.swap_msgs.lock().unwrap().contains_key(&watcher_data.uuid) {
        return;
    }
    let mut taker_swap_watchers = swap_ctx.taker_swap_watchers.lock();
    if taker_swap_watchers.contains(&watcher_data.uuid) {
        return;
    }
    taker_swap_watchers.insert(watcher_data.uuid);
    drop(taker_swap_watchers);

    spawn(async move {
        let taker_coin = match lp_coinfind(&ctx, &watcher_data.taker_coin).await {
            Ok(Some(c)) => c,
            Ok(None) => {
                log::error!("Coin {} is not found/enabled", watcher_data.taker_coin);
                let swap_ctx = SwapsContext::from_ctx(&ctx).unwrap();
                swap_ctx.taker_swap_watchers.lock().remove(watcher_data.uuid);
                return;
            },
            Err(e) => {
                log::error!("!lp_coinfind({}): {}", watcher_data.taker_coin, e);
                let swap_ctx = SwapsContext::from_ctx(&ctx).unwrap();
                swap_ctx.taker_swap_watchers.lock().remove(watcher_data.uuid);
                return;
            },
        };

        let maker_coin = match lp_coinfind(&ctx, &watcher_data.maker_coin).await {
            Ok(Some(c)) => c,
            Ok(None) => {
                log::error!("Coin {} is not found/enabled", watcher_data.maker_coin);
                let swap_ctx = SwapsContext::from_ctx(&ctx).unwrap();
                swap_ctx.taker_swap_watchers.lock().remove(watcher_data.uuid);
                return;
            },
            Err(e) => {
                log::error!("!lp_coinfind({}): {}", watcher_data.maker_coin, e);
                let swap_ctx = SwapsContext::from_ctx(&ctx).unwrap();
                swap_ctx.taker_swap_watchers.lock().remove(watcher_data.uuid);
                return;
            },
        };

        let uuid = watcher_data.uuid;
        log_tag!(
            ctx,
            "";
            fmt = "Entering the watcher_swap_loop {}/{} with uuid: {}",
            maker_coin.ticker(),
            taker_coin.ticker(),
            uuid
        );

        let watcher_ctx = WatcherContext {
            uuid: watcher_data.uuid,
            ctx: ctx.clone(),
            maker_coin,
            taker_coin,
            data: watcher_data,
        };
        let state_machine: StateMachine<_, ()> = StateMachine::from_ctx(watcher_ctx);
        state_machine.run(Started {}).await;
    });
}

pub fn watcher_topic(ticker: &str) -> String { pub_sub_topic(WATCHER_PREFIX, ticker) }
