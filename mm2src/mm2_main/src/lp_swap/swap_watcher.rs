use super::{broadcast_p2p_tx_msg, lp_coinfind, tx_helper_topic, H256Json, SwapsContext, TransactionIdentifier,
            WAIT_CONFIRM_INTERVAL};
use coins::{MmCoinEnum, WatcherValidatePaymentInput};
use common::executor::spawn;
use common::log;
use common::log::{error, info};
use futures::compat::Future01CompatExt;
use futures::{select, FutureExt};
use mm2_core::mm_ctx::MmArc;
use mm2_libp2p::{decode_signed, pub_sub_topic, TopicPrefix};
use mm2_number::BigDecimal;
use parking_lot::Mutex as PaMutex;
use std::cmp::min;
use std::sync::Arc;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use uuid::Uuid;

pub const WATCHER_PREFIX: TopicPrefix = "swpwtchr";
const TAKER_SWAP_CONFIRMATIONS: u64 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SwapWatcherMsg {
    TakerSwapWatcherMsg(Box<TakerSwapWatcherData>),
}

pub struct Watcher {
    uuid: Uuid,
    ctx: MmArc,
    taker_coin: MmCoinEnum,
    maker_coin: MmCoinEnum,
    mutable: RwLock<WatcherMut>,
    errors: PaMutex<Vec<WatcherError>>,
    data: TakerSwapWatcherData,
}

pub struct WatcherMut {
    taker_payment_spend: Option<TransactionIdentifier>,
    maker_payment_spend: Option<TransactionIdentifier>,
    secret: H256Json,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TakerSwapWatcherData {
    pub uuid: Uuid,
    pub secret_hash: Vec<u8>,
    pub taker_spends_maker_payment_preimage: Vec<u8>,
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TakerPaymentSpentData {
    pub transaction: TransactionIdentifier,
    pub secret: H256Json,
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

#[allow(clippy::large_enum_variant)]
pub enum RunWatcherInput {
    StartNew(Watcher),
}

impl RunWatcherInput {
    fn uuid(&self) -> &Uuid {
        match self {
            RunWatcherInput::StartNew(swap) => &swap.uuid,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "data")]
#[allow(clippy::large_enum_variant)]
pub enum WatcherEvent {
    Started,
    StartFailed(WatcherError),
    TakerPaymentWaitConfirmFailed(WatcherError),
    TakerPaymentValidatedAndConfirmed,
    TakerPaymentValidateFailed(WatcherError),
    TakerPaymentSpent(TakerPaymentSpentData),
    TakerPaymentWaitForSpendFailed(WatcherError),
    MakerPaymentSpendFailed(WatcherError),
    MakerPaymentSpent(TransactionIdentifier),
    Finished,
}

impl WatcherEvent {
    pub fn status_str(&self) -> String {
        match self {
            WatcherEvent::Started => "Started...".to_owned(),
            WatcherEvent::StartFailed(_) => "Start failed...".to_owned(),
            WatcherEvent::TakerPaymentWaitConfirmFailed(_) => {
                "Taker payment wait for confirmation failed...".to_owned()
            },
            WatcherEvent::TakerPaymentValidatedAndConfirmed => "Taker payment validated and confirmed...".to_owned(),
            WatcherEvent::TakerPaymentValidateFailed(_) => "Taker payment validate failed...".to_owned(),
            WatcherEvent::TakerPaymentSpent(_) => "Taker payment spent...".to_owned(),
            WatcherEvent::TakerPaymentWaitForSpendFailed(_) => "Taker payment wait for spend failed...".to_owned(),
            WatcherEvent::MakerPaymentSpendFailed(_) => "Maker payment spend failed...".to_owned(),
            WatcherEvent::MakerPaymentSpent(_) => "Maker payment spent...".to_owned(),
            WatcherEvent::Finished => "Finished".to_owned(),
        }
    }
}

#[derive(Debug)]
pub enum WatcherCommand {
    Start,
    ValidateTakerPayment,
    WaitForTakerPaymentSpend,
    SpendMakerPayment,
    Finish,
}

impl Watcher {
    #[inline]
    fn w(&self) -> RwLockWriteGuard<WatcherMut> { self.mutable.write().unwrap() }

    #[inline]
    fn r(&self) -> RwLockReadGuard<WatcherMut> { self.mutable.read().unwrap() }

    #[inline]
    fn apply_event(&self, event: WatcherEvent) {
        match event {
            WatcherEvent::Started => (),
            WatcherEvent::StartFailed(err) => self.errors.lock().push(err),
            WatcherEvent::TakerPaymentWaitConfirmFailed(err) => self.errors.lock().push(err),
            WatcherEvent::TakerPaymentValidatedAndConfirmed => (),
            WatcherEvent::TakerPaymentValidateFailed(err) => self.errors.lock().push(err),
            WatcherEvent::TakerPaymentSpent(data) => {
                self.w().taker_payment_spend = Some(data.transaction);
                self.w().secret = data.secret;
            },
            WatcherEvent::TakerPaymentWaitForSpendFailed(err) => self.errors.lock().push(err),
            WatcherEvent::MakerPaymentSpendFailed(err) => self.errors.lock().push(err),
            WatcherEvent::MakerPaymentSpent(tx) => self.w().maker_payment_spend = Some(tx),
            WatcherEvent::Finished => (),
        }
    }

    async fn handle_command(
        &self,
        command: WatcherCommand,
    ) -> Result<(Option<WatcherCommand>, Vec<WatcherEvent>), String> {
        match command {
            WatcherCommand::Start => self.start().await,
            WatcherCommand::ValidateTakerPayment => self.validate_taker_payment().await,
            WatcherCommand::WaitForTakerPaymentSpend => self.wait_for_taker_payment_spend().await,
            WatcherCommand::SpendMakerPayment => self.spend_maker_payment().await,
            WatcherCommand::Finish => Ok((None, vec![WatcherEvent::Finished])),
        }
    }

    pub fn new(
        uuid: Uuid,
        ctx: MmArc,
        maker_coin: MmCoinEnum,
        taker_coin: MmCoinEnum,
        data: TakerSwapWatcherData,
    ) -> Self {
        Watcher {
            uuid,
            ctx,
            maker_coin,
            taker_coin,
            errors: PaMutex::new(Vec::new()),
            mutable: RwLock::new(WatcherMut {
                taker_payment_spend: None,
                maker_payment_spend: None,
                secret: H256Json::default(),
            }),
            data,
        }
    }

    async fn start(&self) -> Result<(Option<WatcherCommand>, Vec<WatcherEvent>), String> {
        Ok((Some(WatcherCommand::ValidateTakerPayment), vec![WatcherEvent::Started]))
    }

    // Do we need the exact same validation as the maker, or should we use a simpler validation process?
    async fn validate_taker_payment(&self) -> Result<(Option<WatcherCommand>, Vec<WatcherEvent>), String> {
        let wait_duration = (self.data.lock_duration * 4) / 5;
        let wait_taker_payment = self.data.swap_started_at + wait_duration;
        let confirmations = min(self.data.taker_payment_confirmations, TAKER_SWAP_CONFIRMATIONS);

        // Does the watcher have to wait for the confirmations like the maker does?
        let wait_f = self
            .taker_coin
            .wait_for_confirmations(
                &self.data.taker_payment_hex,
                confirmations,
                self.data.taker_payment_requires_nota.unwrap_or(false),
                wait_taker_payment,
                WAIT_CONFIRM_INTERVAL,
            )
            .compat();
        if let Err(err) = wait_f.await {
            return Ok((Some(WatcherCommand::Finish), vec![
                WatcherEvent::TakerPaymentWaitConfirmFailed(
                    ERRL!("!taker_coin.wait_for_confirmations: {}", err).into(),
                ),
            ]));
        }

        let validate_input = WatcherValidatePaymentInput {
            payment_tx: self.data.taker_payment_hex.clone(),
            time_lock: self.data.taker_payment_lock as u32,
            taker_pub: self.data.taker_pub.clone(),
            maker_pub: self.data.maker_pub.clone(),
            secret_hash: self.data.secret_hash.clone(),
            amount: self.data.taker_amount.clone(),
            try_spv_proof_until: wait_taker_payment,
            confirmations,
        };

        let validated_f = self.taker_coin.watcher_validate_taker_payment(validate_input).compat();

        if let Err(e) = validated_f.await {
            return Ok((Some(WatcherCommand::Finish), vec![
                WatcherEvent::TakerPaymentValidateFailed(
                    ERRL!("!taker_coin.watcher_validate_taker_payment: {}", e).into(),
                ),
            ]));
        }

        Ok((Some(WatcherCommand::WaitForTakerPaymentSpend), vec![
            WatcherEvent::TakerPaymentValidatedAndConfirmed,
        ]))
    }

    async fn wait_for_taker_payment_spend(&self) -> Result<(Option<WatcherCommand>, Vec<WatcherEvent>), String> {
        let f = self.taker_coin.wait_for_tx_spend(
            &self.data.taker_payment_hex[..],
            self.data.taker_payment_lock,
            self.data.taker_coin_start_block,
            &None,
        );

        let tx = match f.compat().await {
            Ok(t) => t,
            Err(err) => {
                return Ok((Some(WatcherCommand::Finish), vec![
                    WatcherEvent::TakerPaymentWaitForSpendFailed(err.get_plain_text_format().into()),
                ]));
            },
        };

        let tx_hash = tx.tx_hash();
        info!("Taker payment spend tx {:02x}", tx_hash);
        let tx_ident = TransactionIdentifier {
            tx_hex: tx.tx_hex().into(),
            tx_hash,
        };

        let secret = match self
            .taker_coin
            .extract_secret(&self.data.secret_hash[..], &tx_ident.tx_hex.0)
        {
            Ok(bytes) => H256Json::from(bytes.as_slice()),
            Err(e) => {
                return Ok((Some(WatcherCommand::Finish), vec![
                    WatcherEvent::TakerPaymentWaitForSpendFailed(ERRL!("{}", e).into()),
                ]));
            },
        };

        Ok((Some(WatcherCommand::SpendMakerPayment), vec![
            WatcherEvent::TakerPaymentSpent(TakerPaymentSpentData {
                transaction: tx_ident,
                secret,
            }),
        ]))
    }

    async fn spend_maker_payment(&self) -> Result<(Option<WatcherCommand>, Vec<WatcherEvent>), String> {
        let spend_fut = self.maker_coin.send_taker_spends_maker_payment_preimage(
            &self.data.taker_spends_maker_payment_preimage,
            &self.r().secret.0.clone(),
        );

        let transaction = match spend_fut.compat().await {
            Ok(t) => t,
            Err(err) => {
                if let Some(tx) = err.get_tx() {
                    broadcast_p2p_tx_msg(&self.ctx, tx_helper_topic(self.maker_coin.ticker()), &tx, &None);
                };

                return Ok((Some(WatcherCommand::Finish), vec![
                    WatcherEvent::MakerPaymentSpendFailed(ERRL!("{}", err.get_plain_text_format()).into()),
                ]));
            },
        };

        broadcast_p2p_tx_msg(
            &self.ctx,
            tx_helper_topic(self.maker_coin.ticker()),
            &transaction,
            &None,
        );

        let tx_hash = transaction.tx_hash();
        info!("Maker payment spend tx {:02x}", tx_hash);
        let tx_ident = TransactionIdentifier {
            tx_hex: transaction.tx_hex().into(),
            tx_hash,
        };

        Ok((Some(WatcherCommand::Finish), vec![WatcherEvent::MakerPaymentSpent(
            tx_ident,
        )]))
    }
}

pub async fn run_watcher(swap: RunWatcherInput, ctx: MmArc) {
    let swap_ctx = SwapsContext::from_ctx(&ctx).unwrap();
    let uuid = swap.uuid().to_owned();

    let (swap, mut command) = match swap {
        RunWatcherInput::StartNew(swap) => (swap, WatcherCommand::Start),
    };

    let ctx = swap.ctx.clone();
    let mut status = ctx.log.status_handle();
    let uuid_str = swap.uuid.to_string();
    let running_swap = Arc::new(swap);
    let shutdown_rx = swap_ctx.shutdown_rx.clone();
    let swap_for_log = running_swap.clone();

    let mut swap_fut = Box::pin(
        async move {
            let mut events;
            loop {
                let res = running_swap.handle_command(command).await.expect("!handle_command");
                events = res.1;
                for event in events {
                    status.status(&[&"swap", &("uuid", uuid_str.as_str())], &event.status_str());
                    running_swap.apply_event(event);
                }
                match res.0 {
                    Some(c) => {
                        command = c;
                    },
                    None => {
                        break;
                    },
                }
            }
        }
        .fuse(),
    );
    let mut shutdown_fut = Box::pin(shutdown_rx.recv().fuse());
    let do_nothing = (); // to fix https://rust-lang.github.io/rust-clippy/master/index.html#unused_unit
    select! {
        _swap = swap_fut => {
            swap_ctx.taker_swap_watchers.lock().remove(&uuid);
            do_nothing
        }, // swap finished normally
        _shutdown = shutdown_fut => {
            swap_ctx.taker_swap_watchers.lock().remove(&uuid);
            info!("swap {} stopped!", swap_for_log.uuid)
        },
    };
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
        SwapWatcherMsg::TakerSwapWatcherMsg(watcher_data) => spawn_taker_swap_watcher(ctx, *watcher_data),
    }
}

fn spawn_taker_swap_watcher(ctx: MmArc, watcher_data: TakerSwapWatcherData) {
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
                swap_ctx.taker_swap_watchers.lock().remove(&watcher_data.uuid);
                return;
            },
            Err(e) => {
                log::error!("!lp_coinfind({}): {}", watcher_data.taker_coin, e);
                let swap_ctx = SwapsContext::from_ctx(&ctx).unwrap();
                swap_ctx.taker_swap_watchers.lock().remove(&watcher_data.uuid);
                return;
            },
        };

        let maker_coin = match lp_coinfind(&ctx, &watcher_data.maker_coin).await {
            Ok(Some(c)) => c,
            Ok(None) => {
                log::error!("Coin {} is not found/enabled", watcher_data.maker_coin);
                let swap_ctx = SwapsContext::from_ctx(&ctx).unwrap();
                swap_ctx.taker_swap_watchers.lock().remove(&watcher_data.uuid);
                return;
            },
            Err(e) => {
                log::error!("!lp_coinfind({}): {}", watcher_data.maker_coin, e);
                let swap_ctx = SwapsContext::from_ctx(&ctx).unwrap();
                swap_ctx.taker_swap_watchers.lock().remove(&watcher_data.uuid);
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

        let watcher = Watcher::new(watcher_data.uuid, ctx.clone(), maker_coin, taker_coin, watcher_data);
        run_watcher(RunWatcherInput::StartNew(watcher), ctx).await
    });
}

pub fn watcher_topic(ticker: &str) -> String { pub_sub_topic(WATCHER_PREFIX, ticker) }
