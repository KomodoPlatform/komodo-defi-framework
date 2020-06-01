#![cfg_attr(not(feature = "native"), allow(dead_code))]

use atomic::Atomic;
use bigdecimal::BigDecimal;
use common::{
    bits256, now_ms, now_float, slurp, write, MM_VERSION,
    executor::Timer,
    file_lock::FileLock,
    mm_ctx::MmArc,
};
use coins::{FoundSwapTxSpend, MmCoinEnum, TradeInfo, TransactionDetails};
use crc::crc32;
use futures::{
    FutureExt, select,
    compat::Future01CompatExt,
    future::Either,
};
use futures01::Future;
use parking_lot::Mutex as PaMutex;
use peers::FixedValidator;
use primitives::hash::H264;
use rpc::v1::types::{H160 as H160Json, H256 as H256Json, H264 as H264Json};
use serde_json::{self as json};
use serialization::{deserialize, serialize};
use std::path::PathBuf;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::sync::atomic::Ordering;
use super::{ban_pubkey, broadcast_my_swap_status, dex_fee_amount, get_locked_amount_by_other_swaps,
  lp_atomic_locktime, my_swap_file_path, my_swaps_dir,
  AtomicSwap, LockedAmount, MySwapInfo, RecoveredSwap, RecoveredSwapAction,
  SavedSwap, SwapsContext, SwapError, SwapNegotiationData,
  BASIC_COMM_TIMEOUT, WAIT_CONFIRM_INTERVAL};

pub fn stats_taker_swap_file_path(ctx: &MmArc, uuid: &str) -> PathBuf {
    ctx.dbdir().join("SWAPS").join("STATS").join("TAKER").join(format!("{}.json", uuid))
}

fn save_my_taker_swap_event(ctx: &MmArc, swap: &TakerSwap, event: TakerSavedEvent) -> Result<(), String> {
    let path = my_swap_file_path(ctx, &swap.uuid);
    let content = try_s!(slurp(&path));
    let swap: SavedSwap = if content.is_empty() {
        SavedSwap::Taker(TakerSavedSwap {
            uuid: swap.uuid.clone(),
            maker_amount: Some(swap.maker_amount.clone()),
            maker_coin: Some(swap.maker_coin.ticker().to_owned()),
            taker_amount: Some(swap.taker_amount.clone()),
            taker_coin: Some(swap.taker_coin.ticker().to_owned()),
            gui: ctx.gui().map(|g| g.to_owned()),
            mm_version: Some(MM_VERSION.to_owned()),
            events: vec![],
            success_events: vec!["Started".into(), "Negotiated".into(), "TakerFeeSent".into(),
                                 "MakerPaymentReceived".into(), "MakerPaymentWaitConfirmStarted".into(),
                                 "MakerPaymentValidatedAndConfirmed".into(), "TakerPaymentSent".into(),
                                 "TakerPaymentSpent".into(), "MakerPaymentSpent".into(), "Finished".into()],
            error_events: vec!["StartFailed".into(), "NegotiateFailed".into(), "TakerFeeSendFailed".into(),
                               "MakerPaymentValidateFailed".into(), "MakerPaymentWaitConfirmFailed".into(),
                               "TakerPaymentTransactionFailed".into(), "TakerPaymentWaitConfirmFailed".into(),
                               "TakerPaymentDataSendFailed".into(), "TakerPaymentWaitForSpendFailed".into(),
                               "MakerPaymentSpendFailed".into(), "TakerPaymentWaitRefundStarted".into(), "TakerPaymentRefunded".into(),
                               "TakerPaymentRefundFailed".into()],
        })
    } else {
        try_s!(json::from_slice(&content))
    };

    if let SavedSwap::Taker(mut taker_swap) = swap {
        taker_swap.events.push(event);
        let new_swap = SavedSwap::Taker(taker_swap);
        let new_content = try_s!(json::to_vec(&new_swap));
        try_s!(write(&path, &new_content));
        Ok(())
    } else {
        ERR!("Expected SavedSwap::Taker at {}, got {:?}", path.display(), swap)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TakerSavedEvent {
    timestamp: u64,
    event: TakerSwapEvent,
}

impl TakerSavedEvent {
    /// get the next swap command that must be executed after swap restore
    fn get_command(&self) -> Option<TakerSwapCommand> {
        match self.event {
            TakerSwapEvent::Started(_) => Some(TakerSwapCommand::Negotiate),
            TakerSwapEvent::StartFailed(_) => Some(TakerSwapCommand::Finish),
            TakerSwapEvent::Negotiated(_) => Some(TakerSwapCommand::SendTakerFee),
            TakerSwapEvent::NegotiateFailed(_) => Some(TakerSwapCommand::Finish),
            TakerSwapEvent::TakerFeeSent(_) => Some(TakerSwapCommand::WaitForMakerPayment),
            TakerSwapEvent::TakerFeeSendFailed(_) => Some(TakerSwapCommand::Finish),
            TakerSwapEvent::MakerPaymentReceived(_) => Some(TakerSwapCommand::ValidateMakerPayment),
            TakerSwapEvent::MakerPaymentWaitConfirmStarted => Some(TakerSwapCommand::ValidateMakerPayment),
            TakerSwapEvent::MakerPaymentValidatedAndConfirmed => Some(TakerSwapCommand::SendTakerPayment),
            TakerSwapEvent::MakerPaymentValidateFailed(_) => Some(TakerSwapCommand::Finish),
            TakerSwapEvent::MakerPaymentWaitConfirmFailed(_) => Some(TakerSwapCommand::Finish),
            TakerSwapEvent::TakerPaymentSent(_) => Some(TakerSwapCommand::WaitForTakerPaymentSpend),
            TakerSwapEvent::TakerPaymentTransactionFailed(_) => Some(TakerSwapCommand::Finish),
            TakerSwapEvent::TakerPaymentDataSendFailed(_) => Some(TakerSwapCommand::RefundTakerPayment),
            TakerSwapEvent::TakerPaymentSpent(_) => Some(TakerSwapCommand::SpendMakerPayment),
            TakerSwapEvent::TakerPaymentWaitForSpendFailed(_) => Some(TakerSwapCommand::RefundTakerPayment),
            TakerSwapEvent::TakerPaymentWaitConfirmFailed(_) => Some(TakerSwapCommand::RefundTakerPayment),
            TakerSwapEvent::MakerPaymentSpent(_) => Some(TakerSwapCommand::Finish),
            TakerSwapEvent::MakerPaymentSpendFailed(_) => Some(TakerSwapCommand::RefundTakerPayment),
            TakerSwapEvent::TakerPaymentWaitRefundStarted { wait_until: _ } => Some(TakerSwapCommand::RefundTakerPayment),
            TakerSwapEvent::TakerPaymentRefunded(_) => Some(TakerSwapCommand::Finish),
            TakerSwapEvent::TakerPaymentRefundFailed(_) => Some(TakerSwapCommand::Finish),
            TakerSwapEvent::Finished => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TakerSavedSwap {
    pub uuid: String,
    pub events: Vec<TakerSavedEvent>,
    maker_amount: Option<BigDecimal>,
    maker_coin: Option<String>,
    taker_amount: Option<BigDecimal>,
    taker_coin: Option<String>,
    gui: Option<String>,
    mm_version: Option<String>,
    success_events: Vec<String>,
    error_events: Vec<String>,
}

impl TakerSavedSwap {
    pub fn maker_coin(&self) -> Result<String, String> {
        match self.events.first() {
            Some(event) => match &event.event {
                TakerSwapEvent::Started(data) => Ok(data.maker_coin.clone()),
                _ => ERR!("First swap event must be Started"),
            },
            None => ERR!("Can't get maker coin, events are empty"),
        }
    }

    pub fn taker_coin(&self) -> Result<String, String> {
        match self.events.first() {
            Some(event) => match &event.event {
                TakerSwapEvent::Started(data) => Ok(data.taker_coin.clone()),
                _ => ERR!("First swap event must be Started"),
            },
            None => ERR!("Can't get maker coin, events are empty"),
        }
    }

    pub fn is_finished(&self) -> bool {
        match self.events.last() {
            Some(event) => event.event == TakerSwapEvent::Finished,
            None => false,
        }
    }

    pub fn get_my_info(&self) -> Option<MySwapInfo> {
        match self.events.first() {
            Some(event) => match &event.event {
                TakerSwapEvent::Started(data) => {
                    Some(MySwapInfo {
                        my_coin: data.taker_coin.clone(),
                        other_coin: data.maker_coin.clone(),
                        my_amount: data.taker_amount.clone(),
                        other_amount: data.maker_amount.clone(),
                        started_at: data.started_at,
                    })
                },
                _ => None,
            },
            None => None,
        }
    }

    pub fn is_recoverable(&self) -> bool {
        if !self.is_finished() { return false };
        for event in self.events.iter() {
            match event.event {
                TakerSwapEvent::StartFailed(_) | TakerSwapEvent::NegotiateFailed(_) |
                TakerSwapEvent::TakerFeeSendFailed(_) | TakerSwapEvent::MakerPaymentValidateFailed(_) |
                TakerSwapEvent::TakerPaymentRefunded(_) | TakerSwapEvent::MakerPaymentSpent(_) |
                TakerSwapEvent::MakerPaymentWaitConfirmFailed(_) => {
                    return false;
                }
                _ => (),
            }
        }
        true
    }
}

pub enum RunTakerSwapInput {
    StartNew(TakerSwap),
    KickStart {
        maker_coin: MmCoinEnum,
        taker_coin: MmCoinEnum,
        swap_uuid: String,
    },
}

impl RunTakerSwapInput {
    fn uuid(&self) -> &str {
        match self {
            RunTakerSwapInput::StartNew(swap) => &swap.uuid,
            RunTakerSwapInput::KickStart { swap_uuid, .. } => &swap_uuid,
        }
    }
}

/// Starts the taker swap and drives it to completion (until None next command received).
/// Panics in case of command or event apply fails, not sure yet how to handle such situations
/// because it's usually means that swap is in invalid state which is possible only if there's developer error
/// Every produced event is saved to local DB. Swap status is broadcasted to P2P network after completion.
pub async fn run_taker_swap(swap: RunTakerSwapInput, ctx: MmArc) {
    let uuid = swap.uuid().to_owned();
    let lock_path = my_swaps_dir(&ctx).join(fomat!((uuid) ".lock"));
    let mut attempts = 0;
    let file_lock = loop {
        match FileLock::lock(&lock_path, 40.) {
            Ok(Some(l)) => break l,
            Ok(None) => if attempts >= 1 {
                log!("Swap " (uuid) " file lock is acquired by another process/thread, aborting");
                return;
            } else {
                attempts += 1;
                Timer::sleep(40.).await;
            },
            Err(e) => {
                log!("Swap " (uuid) " file lock error " (e));
                return;
            }
        };
    };

    let (swap, mut command) = match swap {
        RunTakerSwapInput::StartNew(swap) => (swap, TakerSwapCommand::Start),
        RunTakerSwapInput::KickStart {
            maker_coin, taker_coin, swap_uuid
        } => match TakerSwap::load_from_db_by_uuid(ctx, maker_coin, taker_coin, &swap_uuid) {
            Ok((swap, command)) => match command {
                Some(c) => {
                    log!("Swap " (uuid) " kick started.");
                    (swap, c)
                },
                None => {
                    log!("Swap " (uuid) " has been finished already, aborting.");
                    return
                },
            },
            Err(e) => {
                log!("Error " (e) " loading swap " (uuid));
                return;
            }
        }
    };

    let mut touch_loop = Box::pin(async move {
        loop {
            match file_lock.touch() {
                Ok(_) => (),
                Err(e) => log!("Warning, touch error " (e) " for swap " (uuid)),
            };
            Timer::sleep(30.).await;
        }
    }.fuse());

    let ctx = swap.ctx.clone();
    let mut status = ctx.log.status_handle();
    let uuid = swap.uuid.clone();
    let running_swap = Arc::new(swap);
    let weak_ref = Arc::downgrade(&running_swap);
    let swap_ctx = unwrap!(SwapsContext::from_ctx(&ctx));
    unwrap!(swap_ctx.running_swaps.lock()).push(weak_ref);
    let shutdown_rx = swap_ctx.shutdown_rx.clone();
    let swap_for_log = running_swap.clone();

    let mut swap_fut = Box::pin(async move {
        let mut events;
        loop {
            let res = unwrap!(running_swap.handle_command(command).await, "!handle_command");
            events = res.1;
            for event in events {
                let to_save = TakerSavedEvent {
                    timestamp: now_ms(),
                    event: event.clone(),
                };
                unwrap!(save_my_taker_swap_event(&ctx, &running_swap, to_save), "!save_my_taker_swap_event");
                if event.should_ban_maker() { ban_pubkey(&ctx, running_swap.maker.bytes.into(), &running_swap.uuid, event.clone().into()) }
                status.status(&[&"swap", &("uuid", &uuid[..])], &event.status_str());
                unwrap!(running_swap.apply_event(event), "!apply_event");
            }
            match res.0 {
                Some(c) => { command = c; },
                None => {
                    if let Err(e) = broadcast_my_swap_status(&uuid, &ctx) {
                        log!("!broadcast_my_swap_status(" (uuid) "): " (e));
                    }
                    break;
                },
            }
        }
    }.fuse());
    let mut shutdown_fut = Box::pin(shutdown_rx.recv().fuse());
    select! {
        swap = swap_fut => (), // swap finished normally
        shutdown = shutdown_fut => log!("on_stop] swap " (swap_for_log.uuid) " stopped!"),
        touch = touch_loop => unreachable!("Touch loop can not stop!"),
    };
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TakerSwapData {
    taker_coin: String,
    maker_coin: String,
    maker: H256Json,
    my_persistent_pub: H264Json,
    lock_duration: u64,
    maker_amount: BigDecimal,
    taker_amount: BigDecimal,
    maker_payment_confirmations: u64,
    maker_payment_requires_nota: Option<bool>,
    taker_payment_confirmations: u64,
    taker_payment_requires_nota: Option<bool>,
    taker_payment_lock: u64,
    /// Allows to recognize one SWAP from the other in the logs. #274.
    uuid: String,
    started_at: u64,
    maker_payment_wait: u64,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
}

pub struct TakerSwapMut {
    data: TakerSwapData,
    other_persistent_pub: H264,
    taker_fee: Option<TransactionDetails>,
    maker_payment: Option<TransactionDetails>,
    taker_payment: Option<TransactionDetails>,
    taker_payment_spend: Option<TransactionDetails>,
    maker_payment_spend: Option<TransactionDetails>,
    taker_payment_refund: Option<TransactionDetails>,
    secret_hash: H160Json,
    secret: H256Json,
}

pub struct TakerSwap {
    ctx: MmArc,
    maker_coin: MmCoinEnum,
    taker_coin: MmCoinEnum,
    maker_amount: BigDecimal,
    taker_amount: BigDecimal,
    my_persistent_pub: H264,
    maker: bits256,
    uuid: String,
    maker_payment_lock: Atomic<u64>,
    maker_payment_confirmed: Atomic<bool>,
    errors: PaMutex<Vec<SwapError>>,
    finished_at: Atomic<u64>,
    mutable: RwLock<TakerSwapMut>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TakerPaymentSpentData {
    transaction: TransactionDetails,
    secret: H256Json,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MakerNegotiationData {
    maker_payment_locktime: u64,
    maker_pubkey: H264Json,
    secret_hash: H160Json,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum TakerSwapEvent {
    Started(TakerSwapData),
    StartFailed(SwapError),
    Negotiated(MakerNegotiationData),
    NegotiateFailed(SwapError),
    TakerFeeSent(TransactionDetails),
    TakerFeeSendFailed(SwapError),
    MakerPaymentReceived(TransactionDetails),
    MakerPaymentWaitConfirmStarted,
    MakerPaymentValidatedAndConfirmed,
    MakerPaymentValidateFailed(SwapError),
    MakerPaymentWaitConfirmFailed(SwapError),
    TakerPaymentSent(TransactionDetails),
    TakerPaymentTransactionFailed(SwapError),
    TakerPaymentDataSendFailed(SwapError),
    TakerPaymentWaitConfirmFailed(SwapError),
    TakerPaymentSpent(TakerPaymentSpentData),
    TakerPaymentWaitForSpendFailed(SwapError),
    MakerPaymentSpent(TransactionDetails),
    MakerPaymentSpendFailed(SwapError),
    TakerPaymentWaitRefundStarted { wait_until: u64 },
    TakerPaymentRefunded(TransactionDetails),
    TakerPaymentRefundFailed(SwapError),
    Finished,
}

impl TakerSwapEvent {
    fn status_str(&self) -> String {
        match self {
            TakerSwapEvent::Started(_) => "Started...".to_owned(),
            TakerSwapEvent::StartFailed(_) => "Start failed...".to_owned(),
            TakerSwapEvent::Negotiated(_) => "Negotiated...".to_owned(),
            TakerSwapEvent::NegotiateFailed(_) => "Negotiate failed...".to_owned(),
            TakerSwapEvent::TakerFeeSent(_) => "Taker fee sent...".to_owned(),
            TakerSwapEvent::TakerFeeSendFailed(_) => "Taker fee send failed...".to_owned(),
            TakerSwapEvent::MakerPaymentReceived(_) => "Maker payment received...".to_owned(),
            TakerSwapEvent::MakerPaymentWaitConfirmStarted => "Maker payment wait confirm started...".to_owned(),
            TakerSwapEvent::MakerPaymentValidatedAndConfirmed => "Maker payment validated and confirmed...".to_owned(),
            TakerSwapEvent::MakerPaymentValidateFailed(_) => "Maker payment validate failed...".to_owned(),
            TakerSwapEvent::MakerPaymentWaitConfirmFailed(_) => "Maker payment wait for confirmation failed...".to_owned(),
            TakerSwapEvent::TakerPaymentSent(_) => "Taker payment sent...".to_owned(),
            TakerSwapEvent::TakerPaymentTransactionFailed(_) => "Taker payment transaction failed...".to_owned(),
            TakerSwapEvent::TakerPaymentDataSendFailed(_) => "Taker payment data send failed...".to_owned(),
            TakerSwapEvent::TakerPaymentWaitConfirmFailed(_) => "Taker payment wait for confirmation failed...".to_owned(),
            TakerSwapEvent::TakerPaymentSpent(_) => "Taker payment spent...".to_owned(),
            TakerSwapEvent::TakerPaymentWaitForSpendFailed(_) => "Taker payment wait for spend failed...".to_owned(),
            TakerSwapEvent::MakerPaymentSpent(_) => "Maker payment spent...".to_owned(),
            TakerSwapEvent::MakerPaymentSpendFailed(_) => "Maker payment spend failed...".to_owned(),
            TakerSwapEvent::TakerPaymentWaitRefundStarted { wait_until } => format!("Taker payment wait refund till {} started...", wait_until),
            TakerSwapEvent::TakerPaymentRefunded(_) => "Taker payment refunded...".to_owned(),
            TakerSwapEvent::TakerPaymentRefundFailed(_) => "Taker payment refund failed...".to_owned(),
            TakerSwapEvent::Finished => "Finished".to_owned(),
        }
    }

    fn should_ban_maker(&self) -> bool {
        match self {
            TakerSwapEvent::NegotiateFailed(_) | TakerSwapEvent::MakerPaymentValidateFailed(_) |
            TakerSwapEvent::TakerPaymentWaitForSpendFailed(_) => true,
            _ => false,
        }
    }
}

#[derive(Debug)]
pub enum TakerSwapCommand {
    Start,
    Negotiate,
    SendTakerFee,
    WaitForMakerPayment,
    ValidateMakerPayment,
    SendTakerPayment,
    WaitForTakerPaymentSpend,
    SpendMakerPayment,
    RefundTakerPayment,
    Finish
}

impl TakerSwap {
    fn w(&self) -> RwLockWriteGuard<TakerSwapMut> {unwrap!(self.mutable.write())}
    fn r(&self) -> RwLockReadGuard<TakerSwapMut> {unwrap!(self.mutable.read())}

    fn wait_refund_until(&self) -> u64 {
        self.r().data.taker_payment_lock + 3700
    }

    fn apply_event(&self, event: TakerSwapEvent) -> Result<(), String> {
        match event {
            TakerSwapEvent::Started(data) => self.w().data = data,
            TakerSwapEvent::StartFailed(err) => self.errors.lock().push(err),
            TakerSwapEvent::Negotiated(data) => {
                self.maker_payment_lock.store(data.maker_payment_locktime, Ordering::Relaxed);
                self.w().other_persistent_pub = data.maker_pubkey.into();
                self.w().secret_hash = data.secret_hash;
            },
            TakerSwapEvent::NegotiateFailed(err) => self.errors.lock().push(err),
            TakerSwapEvent::TakerFeeSent(tx) => self.w().taker_fee = Some(tx),
            TakerSwapEvent::TakerFeeSendFailed(err) => self.errors.lock().push(err),
            TakerSwapEvent::MakerPaymentReceived(tx) => self.w().maker_payment = Some(tx),
            TakerSwapEvent::MakerPaymentWaitConfirmStarted => (),
            TakerSwapEvent::MakerPaymentValidatedAndConfirmed => self.maker_payment_confirmed.store(true, Ordering::Relaxed),
            TakerSwapEvent::MakerPaymentValidateFailed(err) => self.errors.lock().push(err),
            TakerSwapEvent::MakerPaymentWaitConfirmFailed(err) => self.errors.lock().push(err),
            TakerSwapEvent::TakerPaymentSent(tx) => self.w().taker_payment = Some(tx),
            TakerSwapEvent::TakerPaymentTransactionFailed(err) => self.errors.lock().push(err),
            TakerSwapEvent::TakerPaymentDataSendFailed(err) => self.errors.lock().push(err),
            TakerSwapEvent::TakerPaymentWaitConfirmFailed(err) => self.errors.lock().push(err),
            TakerSwapEvent::TakerPaymentSpent(data) => {
                self.w().taker_payment_spend = Some(data.transaction);
                self.w().secret = data.secret;
            },
            TakerSwapEvent::TakerPaymentWaitForSpendFailed(err) => self.errors.lock().push(err),
            TakerSwapEvent::MakerPaymentSpent(tx) => self.w().maker_payment_spend = Some(tx),
            TakerSwapEvent::MakerPaymentSpendFailed(err) => self.errors.lock().push(err),
            TakerSwapEvent::TakerPaymentWaitRefundStarted { wait_until: _ } => (),
            TakerSwapEvent::TakerPaymentRefunded(tx) => self.w().taker_payment_refund = Some(tx),
            TakerSwapEvent::TakerPaymentRefundFailed(err) => self.errors.lock().push(err),
            TakerSwapEvent::Finished => self.finished_at.store(now_ms() / 1000, Ordering::Relaxed),
        }
        Ok(())
    }

    async fn handle_command(&self, command: TakerSwapCommand)
                      -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        match command {
            TakerSwapCommand::Start => self.start().await,
            TakerSwapCommand::Negotiate => self.negotiate().await,
            TakerSwapCommand::SendTakerFee => self.send_taker_fee().await,
            TakerSwapCommand::WaitForMakerPayment => self.wait_for_maker_payment().await,
            TakerSwapCommand::ValidateMakerPayment => self.validate_maker_payment().await,
            TakerSwapCommand::SendTakerPayment => self.send_taker_payment().await,
            TakerSwapCommand::WaitForTakerPaymentSpend => self.wait_for_taker_payment_spend().await,
            TakerSwapCommand::SpendMakerPayment => self.spend_maker_payment().await,
            TakerSwapCommand::RefundTakerPayment => self.refund_taker_payment().await,
            TakerSwapCommand::Finish => Ok((None, vec![TakerSwapEvent::Finished])),
        }
    }

    pub fn new(
        ctx: MmArc,
        maker: bits256,
        maker_coin: MmCoinEnum,
        taker_coin: MmCoinEnum,
        maker_amount: BigDecimal,
        taker_amount: BigDecimal,
        my_persistent_pub: H264,
        uuid: String,
    ) -> Self {
        TakerSwap {
            ctx,
            maker_coin,
            taker_coin,
            maker_amount,
            taker_amount,
            my_persistent_pub,
            maker,
            uuid,
            maker_payment_confirmed: Atomic::new(false),
            finished_at: Atomic::new(0),
            maker_payment_lock: Atomic::new(0),
            errors: PaMutex::new(Vec::new()),
            mutable: RwLock::new(TakerSwapMut {
                data: TakerSwapData::default(),
                other_persistent_pub: H264::default(),
                taker_fee: None,
                maker_payment: None,
                taker_payment: None,
                taker_payment_spend: None,
                maker_payment_spend: None,
                taker_payment_refund: None,
                secret_hash: H160Json::default(),
                secret: H256Json::default(),
            })
        }
    }

    async fn start(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let my_balance = match self.taker_coin.my_balance().compat().await {
            Ok(balance) => balance,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::StartFailed(ERRL!("!my_balance {}", e).into())],
            ))
        };

        let locked = get_locked_amount_by_other_swaps(&self.ctx, &self.uuid, self.taker_coin.ticker());
        let available = &my_balance - &locked;
        if self.taker_amount > available {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::StartFailed(ERRL!("taker amount {} is larger than available {}, balance {}, locked by other swaps {}",
                    self.taker_amount, available, my_balance, locked
                ).into())],
            ));
        }

        let dex_fee_amount = dex_fee_amount(self.maker_coin.ticker(), self.taker_coin.ticker(), &self.taker_amount);
        if let Err(e) = self.taker_coin.check_i_have_enough_to_trade(&self.taker_amount.clone().into(), &my_balance.clone().into(), TradeInfo::Taker(dex_fee_amount)).compat().await {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::StartFailed(ERRL!("!check_i_have_enough_to_trade {}", e).into())],
            ))
        }

        if let Err(e) = self.maker_coin.can_i_spend_other_payment().compat().await {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::StartFailed(ERRL!("!can_i_spend_other_payment {}", e).into())],
            ));
        };

        let lock_duration = lp_atomic_locktime(self.maker_coin.ticker(), self.taker_coin.ticker());
        let started_at = now_ms() / 1000;

        let maker_coin_start_block = match self.maker_coin.current_block().compat().await {
            Ok(b) => b,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::StartFailed(ERRL!("!maker_coin.current_block {}", e).into())],
            ))
        };

        let taker_coin_start_block = match self.taker_coin.current_block().compat().await {
            Ok(b) => b,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::StartFailed(ERRL!("!taker_coin.current_block {}", e).into())],
            ))
        };

        let data = TakerSwapData {
            taker_coin: self.taker_coin.ticker().to_owned(),
            maker_coin: self.maker_coin.ticker().to_owned(),
            maker: self.maker.bytes.into(),
            started_at,
            lock_duration,
            maker_amount: self.maker_amount.clone(),
            taker_amount: self.taker_amount.clone(),
            maker_payment_confirmations: self.maker_coin.required_confirmations(),
            maker_payment_requires_nota: Some(self.maker_coin.requires_notarization()),
            taker_payment_confirmations: self.taker_coin.required_confirmations(),
            taker_payment_requires_nota: Some(self.taker_coin.requires_notarization()),
            taker_payment_lock: started_at + lock_duration,
            my_persistent_pub: self.my_persistent_pub.clone().into(),
            uuid: self.uuid.clone(),
            maker_payment_wait: started_at + (lock_duration * 2) / 5,
            maker_coin_start_block,
            taker_coin_start_block,
        };

        Ok((Some(TakerSwapCommand::Negotiate), vec![TakerSwapEvent::Started(data)]))
    }

    async fn negotiate(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let data = match recv!(self, "negotiation", 90, -1000, FixedValidator::AnythingGoes) {
            Ok(d) => d,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("{:?}", e).into())]
            )),
        };
        let maker_data: SwapNegotiationData = match deserialize(data.as_slice()) {
            Ok(d) => d,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("{:?}", e).into())]
            )),
        };

        let time_dif = (self.r().data.started_at as i64 - maker_data.started_at as i64).abs();
        if  time_dif > 60 {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("Started_at time_dif over 60 {}", time_dif).into())]
            ))
        }

        let expected_lock_time = maker_data.started_at + self.r().data.lock_duration * 2;
        if maker_data.payment_locktime != expected_lock_time {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("maker_data.payment_locktime {} not equal to expected {}", maker_data.payment_locktime, expected_lock_time).into())]
            ))
        }

        let taker_data = SwapNegotiationData {
            started_at: self.r().data.started_at,
            secret_hash: maker_data.secret_hash.clone(),
            payment_locktime: self.r().data.taker_payment_lock,
            persistent_pubkey: self.my_persistent_pub.clone(),
        };
        let bytes = serialize(&taker_data);
        let sending_f = match send!(self.ctx, self.maker, fomat!(("negotiation-reply") '@' (self.uuid)), 30, bytes.as_slice()) {
            Ok(f) => f,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("{}", e).into())]
            )),
        };
        let data = match recv!(self, sending_f, "negotiated", 90, -1000, FixedValidator::AnythingGoes) {
            Ok(d) => d,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("{:?}", e).into())]
            )),
        };
        let negotiated: bool = match deserialize(data.as_slice()) {
            Ok(n) => n,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("{:?}", e).into())]
            )),
        };

        if !negotiated {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("Maker sent negotiated = false").into())],
            ));
        }

        Ok((
            Some(TakerSwapCommand::SendTakerFee),
            vec![TakerSwapEvent::Negotiated(MakerNegotiationData {
                maker_payment_locktime: maker_data.payment_locktime,
                maker_pubkey: maker_data.persistent_pubkey.into(),
                secret_hash: maker_data.secret_hash.into()
            })],
        ))
    }

    async fn send_taker_fee(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let timeout = self.r().data.started_at + self.r().data.lock_duration / 3;
        let now = now_ms() / 1000;
        if now > timeout {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerFeeSendFailed(ERRL!("Timeout {} > {}", now, timeout).into())],
            ));
        }

        let fee_addr_pub_key = unwrap!(hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06"));
        let fee_amount = dex_fee_amount(&self.r().data.maker_coin, &self.r().data.taker_coin, &self.taker_amount);
        let fee_tx = self.taker_coin.send_taker_fee(&fee_addr_pub_key, fee_amount).compat().await;
        let transaction = match fee_tx {
            Ok (t) => t,
            Err (err) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerFeeSendFailed(ERRL!("{}", err).into())]
            )),
        };

        let hash = transaction.tx_hash();
        // we can attempt to get the details in loop here as transaction was already sent and
        // is present on blockchain so only transport errors are expected to happen
        let tx_details = loop {
            match self.taker_coin.tx_details_by_hash(&hash).compat().await {
                Ok(details) => break details,
                Err(e) => {
                    log!({"Error {} getting tx details of {:02x}", e, hash});
                    Timer::sleep(30.).await;
                    continue;
                }
            }
        };
        log!({"Taker fee tx hash {:02x}", hash});
        Ok((
            Some(TakerSwapCommand::WaitForMakerPayment),
            vec![TakerSwapEvent::TakerFeeSent(tx_details)],
        ))
    }

    async fn wait_for_maker_payment(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let tx_hex = self.r().taker_fee.as_ref().unwrap().tx_hex.clone();
        let sending_f = match send!(self.ctx, self.maker, fomat!(("taker-fee") '@' (self.uuid)), 60, tx_hex) {
            Ok(f) => f,
            Err (err) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerFeeSendFailed(ERRL!("{}", err).into())]
            )),
        };

        let payload = match recv!(self, sending_f, "maker-payment", 180, -1005, FixedValidator::AnythingGoes) {
            Ok(p) => p,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::MakerPaymentValidateFailed(ERRL!("Error waiting for 'maker-payment' data: {}", e).into())]
            )),
        };
        let maker_payment = match self.maker_coin.tx_enum_from_bytes(&payload) {
            Ok(p) => p,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::MakerPaymentValidateFailed(ERRL!("Error parsing the 'maker-payment': {}", e).into())]
            )),
        };

        let hash = maker_payment.tx_hash();
        log!({"Got maker payment {:02x}", hash});
        let mut attempts = 0;
        log!({"Before tx details"});
        let tx_details = loop {
            match self.maker_coin.tx_details_by_hash(&hash).compat().await {
                Ok(details) => break details,
                Err(e) => if attempts >= 3 {
                    return Ok((
                        Some(TakerSwapCommand::Finish),
                        vec![TakerSwapEvent::MakerPaymentValidateFailed(ERRL!("!maker_coin.tx_details_by_hash: {}", e).into())]
                    ))
                } else {
                    attempts += 1;
                    Timer::sleep(10.).await;
                },
            };
        };
        log!({"After tx details"});

        Ok((
            Some(TakerSwapCommand::ValidateMakerPayment),
            vec![TakerSwapEvent::MakerPaymentReceived(tx_details), TakerSwapEvent::MakerPaymentWaitConfirmStarted]
        ))
    }

    async fn validate_maker_payment(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let validated_f = self.maker_coin.validate_maker_payment(
            &unwrap!(self.r().maker_payment.clone()).tx_hex,
            self.maker_payment_lock.load(Ordering::Relaxed) as u32,
            &*self.r().other_persistent_pub,
            &self.r().secret_hash.0,
            self.maker_amount.clone(),
        );
        let validated = validated_f.compat().await;

        if let Err(e) = validated {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::MakerPaymentValidateFailed(ERRL!("!validate maker payment: {}", e).into())]
            ));
        }
        log!({"Before wait confirm"});

        let f = self.maker_coin.wait_for_confirmations(
            &unwrap!(self.r().maker_payment.clone()).tx_hex,
            self.r().data.maker_payment_confirmations,
            self.r().data.maker_payment_requires_nota.unwrap_or(false),
            self.r().data.maker_payment_wait,
            WAIT_CONFIRM_INTERVAL,
        );
        if let Err(err) = f.compat().await {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::MakerPaymentWaitConfirmFailed(ERRL!("!wait for maker payment confirmations: {}", err).into())]
            ));
        }
        log!({"After wait confirm"});

        Ok((
            Some(TakerSwapCommand::SendTakerPayment),
            vec![TakerSwapEvent::MakerPaymentValidatedAndConfirmed]
        ))
    }


    async fn send_taker_payment(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let timeout = self.r().data.started_at + self.r().data.lock_duration / 3;
        let now = now_ms() / 1000;
        if now > timeout {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerPaymentTransactionFailed(ERRL!("Timeout {} > {}", now, timeout).into())],
            ));
        }

        let f = self.taker_coin.check_if_my_payment_sent(
            self.r().data.taker_payment_lock as u32,
            &*self.r().other_persistent_pub,
            &self.r().secret_hash.0,
            self.r().data.taker_coin_start_block,
        );
        let transaction = match f.compat().await {
            Ok(res) => match res {
                Some(tx) => tx,
                None => {
                    let payment_fut = self.taker_coin.send_taker_payment(
                        self.r().data.taker_payment_lock as u32,
                        &*self.r().other_persistent_pub,
                        &self.r().secret_hash.0,
                        self.taker_amount.clone(),
                    );

                    match payment_fut.compat().await {
                        Ok(t) => t,
                        Err(e) => return Ok((
                            Some(TakerSwapCommand::Finish),
                            vec![TakerSwapEvent::TakerPaymentTransactionFailed(ERRL!("{}", e).into())]
                        ))
                    }
                }
            },
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerPaymentTransactionFailed(ERRL!("{}", e).into())]
            ))
        };

        let hash = transaction.tx_hash();
        log!({"Taker payment tx hash {:02x}", hash});
        // we can attempt to get the details in loop here as transaction was already sent and
        // is present on blockchain so only transport errors are expected to happen
        let tx_details = loop {
            match self.taker_coin.tx_details_by_hash(&hash).compat().await {
                Ok(details) => break details,
                Err(e) => {
                    log!({"Error {} getting tx details of {:02x}", e, hash});
                    Timer::sleep(30.).await;
                    continue;
                }
            }
        };

        Ok((
            Some(TakerSwapCommand::WaitForTakerPaymentSpend),
            vec![TakerSwapEvent::TakerPaymentSent(tx_details)],
        ))
    }

    async fn wait_for_taker_payment_spend(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let tx_hex = self.r().taker_payment.as_ref().unwrap().tx_hex.clone();
        let sending_f = match send!(self.ctx, self.maker, fomat!(("taker-payment") '@' (self.uuid)), 60, tx_hex) {
            Ok(f) => f,
            Err(e) => return Ok((
                Some(TakerSwapCommand::RefundTakerPayment),
                vec![
                    TakerSwapEvent::TakerPaymentDataSendFailed(e.into()),
                    TakerSwapEvent::TakerPaymentWaitRefundStarted { wait_until: self.wait_refund_until() },
                ]
            ))
        };

        let wait_duration = (self.r().data.lock_duration * 4) / 5;
        let wait_taker_payment = self.r().data.started_at + wait_duration;
        let wait_f = self.taker_coin.wait_for_confirmations(
            &unwrap!(self.r().taker_payment.clone()).tx_hex,
            self.r().data.taker_payment_confirmations,
            self.r().data.taker_payment_requires_nota.unwrap_or(false),
            wait_taker_payment,
            WAIT_CONFIRM_INTERVAL,
        ).compat();

        if let Err(err) = wait_f.await {
            return Ok((
                Some(TakerSwapCommand::RefundTakerPayment),
                vec![
                    TakerSwapEvent::TakerPaymentWaitConfirmFailed(ERRL!("!taker_coin.wait_for_confirmations: {}", err).into()),
                    TakerSwapEvent::TakerPaymentWaitRefundStarted { wait_until: self.wait_refund_until() },
                ]
            ))
        }

        let f = self.taker_coin.wait_for_tx_spend(
            &self.r().taker_payment.clone().unwrap().tx_hex,
            self.r().data.taker_payment_lock,
            self.r().data.taker_coin_start_block);
        let tx = match f.compat().await {
            Ok(t) => t,
            Err(e) => return Ok((
                Some(TakerSwapCommand::RefundTakerPayment),
                vec![
                    TakerSwapEvent::TakerPaymentWaitForSpendFailed(e.into()),
                    TakerSwapEvent::TakerPaymentWaitRefundStarted { wait_until: self.wait_refund_until() },
                ],
            ))
        };
        drop(sending_f);
        let hash = tx.tx_hash();
        log!({"Taker payment spend tx {:02x}", hash});
        // we can attempt to get the details in loop here as transaction was already sent and
        // is present on blockchain so only transport errors are expected to happen
        let tx_details = loop {
            match self.taker_coin.tx_details_by_hash(&hash).compat().await {
                Ok(details) => break details,
                Err(e) => {
                    log!({"Error {} getting tx details of {:02x}", e, hash});
                    Timer::sleep(30.).await;
                    continue;
                }
            }
        };
        let secret = match tx.extract_secret() {
            Ok(bytes) => H256Json::from(bytes.as_slice()),
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerPaymentWaitForSpendFailed(ERRL!("{}", e).into())],
            )),
        };

        Ok((
            Some(TakerSwapCommand::SpendMakerPayment),
            vec![TakerSwapEvent::TakerPaymentSpent(TakerPaymentSpentData {
                transaction: tx_details,
                secret,
            })],
        ))
    }

    async fn spend_maker_payment(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let spend_fut = self.maker_coin.send_taker_spends_maker_payment(
            &unwrap!(self.r().maker_payment.clone()).tx_hex,
            self.maker_payment_lock.load(Ordering::Relaxed) as u32,
            &*self.r().other_persistent_pub,
            &self.r().secret.0,
        );
        let transaction = match spend_fut.compat().await {
            Ok(t) => t,
            Err(err) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::MakerPaymentSpendFailed(ERRL!("{}", err).into())]
            )),
        };

        let hash = transaction.tx_hash();
        log!({"Maker payment spend tx {:02x}", hash});
        // we can attempt to get the details in loop here as transaction was already sent and
        // is present on blockchain so only transport errors are expected to happen
        let tx_details = loop {
            match self.maker_coin.tx_details_by_hash(&hash).compat().await {
                Ok(details) => break details,
                Err(e) => {
                    log!({"Error {} getting tx details of {:02x}", e, hash});
                    Timer::sleep(30.).await;
                    continue;
                }
            }
        };
        Ok((
            Some(TakerSwapCommand::Finish),
            vec![TakerSwapEvent::MakerPaymentSpent(tx_details)],
        ))
    }

    async fn refund_taker_payment(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        loop {
            // have to wait for 1 hour more because some coins have BIP113 activated so these will reject transactions with locktime == present time
            // https://github.com/bitcoin/bitcoin/blob/master/doc/release-notes/release-notes-0.11.2.md#bip113-mempool-only-locktime-enforcement-using-getmediantimepast
            if now_ms() / 1000 > self.wait_refund_until() {
                break;
            }
            Timer::sleep(10.).await;
        }
        let refund_fut = self.taker_coin.send_taker_refunds_payment(
            &self.r().taker_payment.clone().unwrap().tx_hex.0,
            self.r().data.taker_payment_lock as u32,
            &*self.r().other_persistent_pub,
            &self.r().secret_hash.0,
        );

        let transaction = match refund_fut.compat().await {
            Ok(t) => t,
            Err(err) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerPaymentRefundFailed(ERRL!("{}", err).into())]
            )),
        };

        let hash = transaction.tx_hash();
        log!({"Taker refund tx hash {:02x}", hash});
        // we can attempt to get the details in loop here as transaction was already sent and
        // is present on blockchain so only transport errors are expected to happen
        let tx_details = loop {
            match self.taker_coin.tx_details_by_hash(&hash).compat().await {
                Ok(details) => break details,
                Err(e) => {
                    log!({"Error {} getting tx details of {:02x}", e, hash});
                    Timer::sleep(30.).await;
                    continue;
                }
            }
        };
        Ok((
            Some(TakerSwapCommand::Finish),
            vec![TakerSwapEvent::TakerPaymentRefunded(tx_details)],
        ))
    }

    pub fn load_from_db_by_uuid(
        ctx: MmArc,
        maker_coin: MmCoinEnum,
        taker_coin: MmCoinEnum,
        swap_uuid: &str,
    ) -> Result<(Self, Option<TakerSwapCommand>), String> {
        let path = my_swap_file_path(&ctx, swap_uuid);
        let saved: SavedSwap = try_s!(json::from_slice(&try_s!(slurp(&path))));
        let saved = match saved {
            SavedSwap::Taker(swap) => swap,
            SavedSwap::Maker(_) => return ERR!("Can not load TakerSwap from SavedSwap::Maker uuid: {}", swap_uuid),
        };
        Self::load_from_saved(ctx, maker_coin, taker_coin, saved)
    }

    pub fn load_from_saved(
        ctx: MmArc,
        maker_coin: MmCoinEnum,
        taker_coin: MmCoinEnum,
        saved: TakerSavedSwap,
    ) -> Result<(Self, Option<TakerSwapCommand>), String> {
        if saved.events.is_empty() {
            return ERR!("Can't restore swap from empty events set");
        };

        match &saved.events[0].event {
            TakerSwapEvent::Started(data) => {
                let mut maker = bits256::from([0; 32]);
                maker.bytes = data.maker.0;
                let my_persistent_pub = H264::from(&**ctx.secp256k1_key_pair().public());

                let swap = TakerSwap::new(
                    ctx,
                    maker.into(),
                    maker_coin,
                    taker_coin,
                    data.maker_amount.clone(),
                    data.taker_amount.clone(),
                    my_persistent_pub,
                    saved.uuid,
                );
                let command = saved.events.last().unwrap().get_command();
                for saved_event in saved.events {
                    try_s!(swap.apply_event(saved_event.event));
                }
                Ok((swap, command))
            },
            _ => ERR!("First swap event must be Started"),
        }
    }

    pub fn recover_funds(&self) -> Result<RecoveredSwap, String> {
        if self.finished_at.load(Ordering::Relaxed) == 0 { return ERR!("Swap must be finished before recover funds attempt"); }

        if self.r().taker_payment_refund.is_some() { return ERR!("Taker payment is refunded, swap is not recoverable"); }

        if self.r().maker_payment_spend.is_some() { return ERR!("Maker payment is spent, swap is not recoverable"); }

        let maker_payment = match &self.r().maker_payment {
            Some(tx) => tx.tx_hex.0.clone(),
            None => return ERR!("No info about maker payment, swap is not recoverable"),
        };

        macro_rules! check_maker_payment_is_not_spent {
            // validate that maker payment is not spent
            () => {
                match self.maker_coin.search_for_swap_tx_spend_other(
                    self.maker_payment_lock.load(Ordering::Relaxed) as u32,
                    &*self.r().other_persistent_pub,
                    &self.r().secret_hash.0,
                    &maker_payment,
                    self.r().data.maker_coin_start_block,
                ) {
                    Ok(Some(FoundSwapTxSpend::Spent(tx))) => return ERR!("Maker payment was already spent by {} tx {:02x}", self.maker_coin.ticker(), tx.tx_hash()),
                    Ok(Some(FoundSwapTxSpend::Refunded(tx))) => return ERR!("Maker payment was already refunded by {} tx {:02x}", self.maker_coin.ticker(), tx.tx_hash()),
                    Err(e) => return ERR!("Error {} when trying to find maker payment spend", e),
                    Ok(None) => (), // payment is not spent, continue
                }
            }
        }

        let taker_payment = match &self.r().taker_payment {
            Some(tx) => tx.tx_hex.0.clone(),
            None => {
                let maybe_sent = try_s!(self.taker_coin.check_if_my_payment_sent(
                    self.r().data.taker_payment_lock as u32,
                    &*self.r().other_persistent_pub,
                    &self.r().secret_hash.0,
                    self.r().data.taker_coin_start_block,
                ).wait());
                match maybe_sent {
                    Some(tx) => tx.tx_hex(),
                    None => return ERR!("Taker payment is not found, swap is not recoverable"),
                }
            }
        };

        if self.r().taker_payment_spend.is_some() {
            check_maker_payment_is_not_spent!();
            let transaction = try_s!(self.maker_coin.send_taker_spends_maker_payment(
                &maker_payment,
                self.maker_payment_lock.load(Ordering::Relaxed) as u32,
                &*self.r().other_persistent_pub,
                &self.r().secret.0,
            ).wait());

            return Ok(RecoveredSwap {
                action: RecoveredSwapAction::SpentOtherPayment,
                coin: self.maker_coin.ticker().to_string(),
                transaction,
            });
        }

        let taker_payment_spend = try_s!(self.taker_coin.search_for_swap_tx_spend_my(
            self.r().data.taker_payment_lock as u32,
            &*self.r().other_persistent_pub,
            &self.r().secret_hash.0,
            &taker_payment,
            self.r().data.taker_coin_start_block,
        ));

        match taker_payment_spend {
            Some(spend) => match spend {
                FoundSwapTxSpend::Spent(tx) => {
                    check_maker_payment_is_not_spent!();
                    let secret = try_s!(tx.extract_secret());
                    let transaction = try_s!(self.maker_coin.send_taker_spends_maker_payment(
                        &maker_payment,
                        self.maker_payment_lock.load(Ordering::Relaxed) as u32,
                        &*self.r().other_persistent_pub,
                        &secret,
                    ).wait());

                    Ok(RecoveredSwap {
                        action: RecoveredSwapAction::SpentOtherPayment,
                        coin: self.maker_coin.ticker().to_string(),
                        transaction,
                    })
                },
                FoundSwapTxSpend::Refunded(tx) => {
                   ERR!("Taker payment has been refunded already by transaction {:02x}", tx.tx_hash())
                },
            },
            None => {
                if now_ms() / 1000 < self.r().data.taker_payment_lock + 3700 {
                    return ERR!("Too early to refund, wait until {}", self.r().data.taker_payment_lock + 3700);
                }

                let transaction = try_s!(self.taker_coin.send_taker_refunds_payment(
                    &taker_payment,
                    self.r().data.taker_payment_lock as u32,
                    &*self.r().other_persistent_pub,
                    &self.r().secret_hash.0,
                ).wait());

                Ok(RecoveredSwap {
                    action: RecoveredSwapAction::RefundedMyPayment,
                    coin: self.maker_coin.ticker().to_string(),
                    transaction,
                })
            },
        }
    }
}

impl AtomicSwap for TakerSwap {
    fn locked_amount(&self) -> LockedAmount {
        // if taker payment is not sent yet the taker fee amount must be virtually locked
        let fee_amount = match self.r().taker_fee {
            Some(_) => 0.into(),
            None => dex_fee_amount(self.maker_coin.ticker(), self.taker_coin.ticker(), &self.taker_amount),
        };

        let amount = match self.r().taker_payment {
            Some(_) => 0.into(),
            None => fee_amount + &self.taker_amount,
        };

        LockedAmount {
            coin: self.taker_coin.ticker().to_string(),
            amount,
        }
    }

    fn uuid(&self) -> &str {
        &self.uuid
    }

    fn maker_coin(&self) -> &str { self.maker_coin.ticker() }

    fn taker_coin(&self) -> &str { self.taker_coin.ticker() }
}

#[cfg(test)]
mod taker_swap_tests {
    use coins::{FoundSwapTxSpend, MarketCoinOps, SwapOps, TestCoin};
    use coins::utxo::{UtxoTx};
    use coins::eth::{signed_eth_tx_from_bytes, SignedEthTx};
    use common::privkey::key_pair_from_seed;
    use common::mm_ctx::MmCtxBuilder;
    use mocktopus::mocking::*;
    use super::*;

    fn eth_tx_for_test() -> SignedEthTx {
        // raw transaction bytes of https://etherscan.io/tx/0x0869be3e5d4456a29d488a533ad6c118620fef450f36778aecf31d356ff8b41f
        let tx_bytes = [248, 240, 3, 133, 1, 42, 5, 242, 0, 131, 2, 73, 240, 148, 133, 0, 175, 192, 188, 82, 20, 114, 128, 130, 22, 51, 38, 194, 255, 12, 115, 244, 168, 113, 135, 110, 205, 245, 24, 127, 34, 254, 184, 132, 21, 44, 243, 175, 73, 33, 143, 82, 117, 16, 110, 27, 133, 82, 200, 114, 233, 42, 140, 198, 35, 21, 201, 249, 187, 180, 20, 46, 148, 40, 9, 228, 193, 130, 71, 199, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 152, 41, 132, 9, 201, 73, 19, 94, 237, 137, 35, 61, 4, 194, 207, 239, 152, 75, 175, 245, 157, 174, 10, 214, 161, 207, 67, 70, 87, 246, 231, 212, 47, 216, 119, 68, 237, 197, 125, 141, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 93, 72, 125, 102, 28, 159, 180, 237, 198, 97, 87, 80, 82, 200, 104, 40, 245, 221, 7, 28, 122, 104, 91, 99, 1, 159, 140, 25, 131, 101, 74, 87, 50, 168, 146, 187, 90, 160, 51, 1, 123, 247, 6, 108, 165, 181, 188, 40, 56, 47, 211, 229, 221, 73, 5, 15, 89, 81, 117, 225, 216, 108, 98, 226, 119, 232, 94, 184, 42, 106];
        unwrap!(signed_eth_tx_from_bytes(&tx_bytes))
    }

    #[test]
    fn test_recover_funds_taker_swap_maker_payment_spend_errored() {
        let taker_saved_json = r#"{"error_events":["StartFailed","NegotiateFailed","TakerFeeSendFailed","MakerPaymentValidateFailed","TakerPaymentTransactionFailed","TakerPaymentDataSendFailed","TakerPaymentWaitForSpendFailed","MakerPaymentSpendFailed","TakerPaymentRefunded","TakerPaymentRefundFailed"],"events":[{"event":{"data":{"lock_duration":7800,"maker":"1bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","maker_amount":"0.12596566232185483","maker_coin":"KMD","maker_coin_start_block":1458035,"maker_payment_confirmations":1,"maker_payment_wait":1564053079,"my_persistent_pub":"0326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0a","started_at":1564050479,"taker_amount":"50.000000000000001504212457800000","taker_coin":"DOGE","taker_coin_start_block":2823448,"taker_payment_confirmations":1,"taker_payment_lock":1564058279,"uuid":"41383f43-46a5-478c-9386-3b2cce0aca20"},"type":"Started"},"timestamp":1564050480269},{"event":{"data":{"maker_payment_locktime":1564066080,"maker_pubkey":"031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","secret_hash":"3669eb83a007a3c507448d79f45a9f06ec2f36a8"},"type":"Negotiated"},"timestamp":1564050540991},{"event":{"data":{"block_height":0,"coin":"DOGE","fee_details":{"amount":5},"from":["DBNHC8sQS8SCwCrKzG57G7ZVCh1zaih2tx"],"internal_id":"bdde828b492d6d1cc25cd2322fd592dafd722fcc7d8b0fedce4d3bb4a1a8c8ff","my_balance_change":-5.05791505,"received_by_me":96.85084225,"spent_by_me":101.9087573,"timestamp":0,"to":["DBNHC8sQS8SCwCrKzG57G7ZVCh1zaih2tx","DPZnzesTGPD42AXY1qX8BQp78jLbmzpRT7"],"total_amount":101.9087573,"tx_hash":"bdde828b492d6d1cc25cd2322fd592dafd722fcc7d8b0fedce4d3bb4a1a8c8ff","tx_hex":"0100000002c7efa995c8b7be0a8b6c2d526c6c444c1634d65584e9ee89904e9d8675eac88c010000006a473044022051f34d5e3b7d0b9098d5e35333f3550f9cb9e57df83d5e4635b7a8d2986d6d5602200288c98da05de6950e01229a637110a1800ba643e75cfec59d4eb1021ad9b40801210326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0affffffffae6c233989efa7c7d2aa6534adc96078917ff395b7f09f734a147b2f44ade164000000006a4730440220393a784c2da74d0e2a28ec4f7df6c8f9d8b2af6ae6957f1e68346d744223a8fd02201b7a96954ac06815a43a6c7668d829ae9cbb5de76fa77189ddfd9e3038df662c01210326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0affffffff02115f5800000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac41a84641020000001976a914444f0e1099709ba4d742454a7d98a5c9c162ceab88ac6d84395d"},"type":"TakerFeeSent"},"timestamp":1564050545296},{"event":{"data":{"block_height":0,"coin":"KMD","fee_details":{"amount":1e-05},"from":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd"],"internal_id":"0a0f11fa82802c2c30862c50ab2162185dae8de7f7235f32c506f814c142b382","my_balance_change":0,"received_by_me":0,"spent_by_me":0,"timestamp":0,"to":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd","bQTa5QiudricscFpKeJpcvi3rqFW4YEBcs"],"total_amount":1.10033066,"tx_hash":"0a0f11fa82802c2c30862c50ab2162185dae8de7f7235f32c506f814c142b382","tx_hex":"0400008085202f8902ace337db2dd4c56b0697f58fb8cfb6bd1cd6f469d925fc0376d1dcfb7581bf82000000006b483045022100d1f95be235c5c8880f5d703ace287e2768548792c58c5dbd27f5578881b30ea70220030596106e21c7e0057ee0dab283f9a1fe273f15208cba80870c447bd559ef0d0121031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ffffffff9f339752567c404427fd77f2b35cecdb4c21489edc64e25e729fdb281785e423000000006a47304402203179e95877dbc107123a417f1e648e3ff13d384890f1e4a67b6dd5087235152e0220102a8ab799fadb26b5d89ceb9c7bc721a7e0c2a0d0d7e46bbe0cf3d130010d430121031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ffffffff025635c0000000000017a91480a95d366d65e34a465ab17b0c9eb1d5a33bae08876cbfce05000000001976a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac8d7c395d000000000000000000000000000000"},"type":"MakerPaymentReceived"},"timestamp":1564050588176},{"event":{"type":"MakerPaymentWaitConfirmStarted"},"timestamp":1564050588178},{"event":{"type":"MakerPaymentValidatedAndConfirmed"},"timestamp":1564050693585},{"event":{"data":{"block_height":0,"coin":"DOGE","fee_details":{"amount":5},"from":["DBNHC8sQS8SCwCrKzG57G7ZVCh1zaih2tx"],"internal_id":"539cb6dbdc25465bbccc575554f05d1bb04c70efce4316e41194e747375c3659","my_balance_change":-55,"received_by_me":41.85084225,"spent_by_me":96.85084225,"timestamp":0,"to":["AEb1iUQHozeuCifeFxbcvKmoKdFToF4p27","DBNHC8sQS8SCwCrKzG57G7ZVCh1zaih2tx"],"total_amount":96.85084225,"tx_hash":"539cb6dbdc25465bbccc575554f05d1bb04c70efce4316e41194e747375c3659","tx_hex":"0100000001ffc8a8a1b43b4dceed0f8b7dcc2f72fdda92d52f32d25cc21c6d2d498b82debd010000006a47304402203967b7f9f5532fa47116585c7d1bcba51861ea2059cca00409f34660db18e33a0220640991911852533a12fdfeb039fb9c8ca2c45482c6993bd84636af3670d49c1501210326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0affffffff0200f2052a0100000017a914f2fa08ae416b576779ae5da975e5442663215fce87415173f9000000001976a914444f0e1099709ba4d742454a7d98a5c9c162ceab88ac0585395d"},"type":"TakerPaymentSent"},"timestamp":1564050695611},{"event":{"data":{"secret":"1b8886b8a2cdb62505699400b694ac20f04d7bd4abd80e1ab154aa8d861fc093","transaction":{"block_height":0,"coin":"DOGE","fee_details":{"amount":5},"from":["AEb1iUQHozeuCifeFxbcvKmoKdFToF4p27"],"internal_id":"cc5af1cf68d246419fee49c3d74c0cd173599d115b86efe274368a614951bc47","my_balance_change":0,"received_by_me":0,"spent_by_me":0,"timestamp":1564050913,"to":["DP1GH73GKRdRz8Qxc8mxe2hFtu5NRwTBXV"],"total_amount":50,"tx_hash":"cc5af1cf68d246419fee49c3d74c0cd173599d115b86efe274368a614951bc47","tx_hex":"010000000159365c3747e79411e41643ceef704cb01b5df0545557ccbc5b4625dcdbb69c5300000000d747304402200e78e27d2f1c18676f98ca3dfa4e4a9eeaa8209b55f57b4dd5d9e1abdf034cfa0220623b5c22b62234cec230342aa306c497e43494b44ec2425b84e236b1bf01257001201b8886b8a2cdb62505699400b694ac20f04d7bd4abd80e1ab154aa8d861fc093004c6b6304a7a2395db175210326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0aac6782012088a9143669eb83a007a3c507448d79f45a9f06ec2f36a88821031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ac68ffffffff01008d380c010000001976a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac8c77395d"}},"type":"TakerPaymentSpent"},"timestamp":1564051092890},{"event":{"data":{"error":"lp_swap:1981] utxo:891] rpc_clients:738] JsonRpcError { request: JsonRpcRequest { jsonrpc: \"2.0\", id: \"67\", method: \"blockchain.transaction.broadcast\", params: [String(\"0400008085202f890182b342c114f806c5325f23f7e78dae5d186221ab502c86302c2c8082fa110f0a00000000d7473044022035791ea5548f87484065c9e1f0bdca9ebc699f2c7f51182c84f360102e32dc3d02200612ed53bca52d9c2568437f087598531534badf26229fe0f652ea72ddf03ca501201b8886b8a2cdb62505699400b694ac20f04d7bd4abd80e1ab154aa8d861fc093004c6b630420c1395db17521031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ac6782012088a9143669eb83a007a3c507448d79f45a9f06ec2f36a888210326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0aac68ffffffff01460ec000000000001976a914444f0e1099709ba4d742454a7d98a5c9c162ceab88ac967e395d000000000000000000000000000000\")] }, error: Transport(\"rpc_clients:668] All electrums are currently disconnected\") }"},"type":"MakerPaymentSpendFailed"},"timestamp":1564051092897},{"event":{"type":"Finished"},"timestamp":1564051092900}],"success_events":["Started","Negotiated","TakerFeeSent","MakerPaymentReceived","MakerPaymentWaitConfirmStarted","MakerPaymentValidatedAndConfirmed","TakerPaymentSent","TakerPaymentSpent","MakerPaymentSpent","Finished"],"uuid":"41383f43-46a5-478c-9386-3b2cce0aca20"}"#;
        let taker_saved_swap: TakerSavedSwap = unwrap!(json::from_str(taker_saved_json));
        let key_pair = unwrap!(key_pair_from_seed("spice describe gravity federal blast come thank unfair canal monkey style afraid"));
        let ctx = MmCtxBuilder::default().with_secp256k1_key_pair(key_pair).into_mm_arc();

        TestCoin::ticker.mock_safe(|_| MockResult::Return("ticker"));
        static mut MAKER_PAYMENT_SPEND_CALLED: bool = false;
        TestCoin::send_taker_spends_maker_payment.mock_safe(|_, _, _, _, _| {
            unsafe { MAKER_PAYMENT_SPEND_CALLED = true };
            MockResult::Return(Box::new(futures01::future::ok(eth_tx_for_test().into())))
        });
        TestCoin::search_for_swap_tx_spend_other.mock_safe(|_, _, _, _, _, _| MockResult::Return(Ok(None)));
        let maker_coin = MmCoinEnum::Test(TestCoin {});
        let taker_coin = MmCoinEnum::Test(TestCoin {});
        let (taker_swap, _) = unwrap!(TakerSwap::load_from_saved(ctx, maker_coin, taker_coin, taker_saved_swap));
        let actual = unwrap!(taker_swap.recover_funds());
        let expected = RecoveredSwap {
            action: RecoveredSwapAction::SpentOtherPayment,
            coin: "ticker".to_string(),
            transaction: eth_tx_for_test().into(),
        };
        assert_eq!(expected, actual);
        assert!(unsafe { MAKER_PAYMENT_SPEND_CALLED });
    }

    #[test]
    fn test_recover_funds_taker_swap_taker_payment_errored_but_sent_not_spent() {
        let taker_saved_json = r#"{"error_events":["StartFailed","NegotiateFailed","TakerFeeSendFailed","MakerPaymentValidateFailed","TakerPaymentTransactionFailed","TakerPaymentDataSendFailed","TakerPaymentWaitForSpendFailed","MakerPaymentSpendFailed","TakerPaymentRefunded","TakerPaymentRefundFailed"],"events":[{"event":{"data":{"lock_duration":7800,"maker":"1bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","maker_amount":"3.54932734","maker_coin":"KMD","maker_coin_start_block":1452970,"maker_payment_confirmations":1,"maker_payment_wait":1563746537,"my_persistent_pub":"03101ace6b08605b9424b0582b5cce044b70a3c8d8d10cb2965e039b0967ae92b9","started_at":1563743937,"taker_amount":"0.02004833998671660000000000","taker_coin":"ETH","taker_coin_start_block":8196380,"taker_payment_confirmations":1,"taker_payment_lock":1563751737,"uuid":"3447b727-fe93-4357-8e5a-8cf2699b7e86"},"type":"Started"},"timestamp":1563743937741},{"event":{"data":{"maker_payment_locktime":1563759539,"maker_pubkey":"031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","secret_hash":"432c8272ac59b47dea2d299b5cf1ee64ea1917b9"},"type":"Negotiated"},"timestamp":1563744003530},{"event":{"data":{"block_height":0,"coin":"ETH","fee_details":null,"from":["0x3D6a2f4Dd6085b34EeD6cBc2D3aaABd0D3B697C1"],"internal_id":"00","my_balance_change":-0.0001,"received_by_me":0,"spent_by_me":0.0001,"timestamp":1563744020,"to":["0xD8997941Dd1346e9231118D5685d866294f59e5b"],"total_amount":0.0001,"tx_hash":"a59203eb2328827de00bed699a29389792906e4f39fdea145eb40dc6b3821bd6","tx_hex":"f8690284ee6b280082520894d8997941dd1346e9231118d5685d866294f59e5b865af3107a4000801ca0743d2b7c9fad65805d882179062012261be328d7628ae12ee08eff8d7657d993a07eecbd051f49d35279416778faa4664962726d516ce65e18755c9b9406a9c2fd"},"type":"TakerFeeSent"},"timestamp":1563744020598},{"event":{"data":{"block_height":0,"coin":"KMD","fee_details":{"amount":1e-05},"from":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd"],"internal_id":"0cf4acbcefde53645851c5c6053ea61fe0cbb5f828a906d69eb809e0b071a03b","my_balance_change":0,"received_by_me":0,"spent_by_me":0,"timestamp":0,"to":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd","bYAJo9wvPf3cBgPJ7NVwhoeAY1meGNxEGh"],"total_amount":12.51998004,"tx_hash":"0cf4acbcefde53645851c5c6053ea61fe0cbb5f828a906d69eb809e0b071a03b","tx_hex":"0400008085202f89025d5ae3e8c87418c9b735f8f2f7d29e26820c33c9f30d53f2d31f8b99ea9b1490010000006a47304402201185c06ca575261c539b287175751b7de642eb7466c59128639a19b4c2dd2f9b02201c8c4167d581864bedd4d1deb5596472e6e3ce29fe9e7996907a7b59c905d5490121031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ffffffff06dbf9971c8dfd4a0c8c49f4f15c51de59ba13b2efa702682e26869843af9a87000000006a473044022012b47c12c7f6ad7d8b778fc4b5dcfd56a39325daf302f56e7b84753ba5216cfa022076bf571cf9e20facf70d2f134e8ed2de67aa08581a27ff3128bf93a9b594ac770121031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ffffffff02fed727150000000017a914d5268b31131a652f9b6ddf57db62f02285cdfad1874e1d7835000000001976a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac37cf345d000000000000000000000000000000"},"type":"MakerPaymentReceived"},"timestamp":1563744071778},{"event":{"type":"MakerPaymentWaitConfirmStarted"},"timestamp":1563744071781},{"event":{"type":"MakerPaymentValidatedAndConfirmed"},"timestamp":1563744118073},{"event":{"data":{"error":"lp_swap:1888] eth:654] RPC error: Error { code: ServerError(-32010), message: \"Transaction with the same hash was already imported.\", data: None }"},"type":"TakerPaymentTransactionFailed"},"timestamp":1563744118577},{"event":{"type":"Finished"},"timestamp":1563744118580}],"success_events":["Started","Negotiated","TakerFeeSent","MakerPaymentReceived","MakerPaymentWaitConfirmStarted","MakerPaymentValidatedAndConfirmed","TakerPaymentSent","TakerPaymentSpent","MakerPaymentSpent","Finished"],"uuid":"3447b727-fe93-4357-8e5a-8cf2699b7e86"}"#;
        let taker_saved_swap: TakerSavedSwap = unwrap!(json::from_str(taker_saved_json));
        let key_pair = unwrap!(key_pair_from_seed("spice describe gravity federal blast come thank unfair canal monkey style afraid"));
        let ctx = MmCtxBuilder::default().with_secp256k1_key_pair(key_pair).into_mm_arc();

        TestCoin::ticker.mock_safe(|_| MockResult::Return("ticker"));

        static mut MY_PAYMENT_SENT_CALLED: bool = false;
        TestCoin::check_if_my_payment_sent.mock_safe(|_, _, _, _, _| {
            unsafe { MY_PAYMENT_SENT_CALLED = true };
            MockResult::Return(Box::new(futures01::future::ok(Some(eth_tx_for_test().into()))))
        });

        static mut TX_SPEND_CALLED: bool = false;
        TestCoin::search_for_swap_tx_spend_my.mock_safe(|_, _, _, _, _, _| {
            unsafe { TX_SPEND_CALLED = true };
            MockResult::Return(Ok(None))
        });

        static mut TAKER_PAYMENT_REFUND_CALLED: bool = false;
        TestCoin::send_taker_refunds_payment.mock_safe(|_, _, _, _, _| {
            unsafe { TAKER_PAYMENT_REFUND_CALLED = true };
            MockResult::Return(Box::new(futures01::future::ok(eth_tx_for_test().into())))
        });
        let maker_coin = MmCoinEnum::Test(TestCoin {});
        let taker_coin = MmCoinEnum::Test(TestCoin {});
        let (taker_swap, _) = unwrap!(TakerSwap::load_from_saved(ctx, maker_coin, taker_coin, taker_saved_swap));
        let actual = unwrap!(taker_swap.recover_funds());
        let expected = RecoveredSwap {
            action: RecoveredSwapAction::RefundedMyPayment,
            coin: "ticker".to_string(),
            transaction: eth_tx_for_test().into(),
        };
        assert_eq!(expected, actual);
        assert!(unsafe { MY_PAYMENT_SENT_CALLED });
        assert!(unsafe { TX_SPEND_CALLED });
        assert!(unsafe { TAKER_PAYMENT_REFUND_CALLED });
    }

    #[test]
    fn test_recover_funds_taker_swap_taker_payment_errored_but_sent_and_spent_by_maker() {
        let taker_saved_json = r#"{"error_events":["StartFailed","NegotiateFailed","TakerFeeSendFailed","MakerPaymentValidateFailed","TakerPaymentTransactionFailed","TakerPaymentDataSendFailed","TakerPaymentWaitForSpendFailed","MakerPaymentSpendFailed","TakerPaymentRefunded","TakerPaymentRefundFailed"],"events":[{"event":{"data":{"lock_duration":7800,"maker":"1bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","maker_amount":"3.54932734","maker_coin":"KMD","maker_coin_start_block":1452970,"maker_payment_confirmations":1,"maker_payment_wait":1563746537,"my_persistent_pub":"03101ace6b08605b9424b0582b5cce044b70a3c8d8d10cb2965e039b0967ae92b9","started_at":1563743937,"taker_amount":"0.02004833998671660000000000","taker_coin":"ETH","taker_coin_start_block":8196380,"taker_payment_confirmations":1,"taker_payment_lock":1563751737,"uuid":"3447b727-fe93-4357-8e5a-8cf2699b7e86"},"type":"Started"},"timestamp":1563743937741},{"event":{"data":{"maker_payment_locktime":1563759539,"maker_pubkey":"031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","secret_hash":"432c8272ac59b47dea2d299b5cf1ee64ea1917b9"},"type":"Negotiated"},"timestamp":1563744003530},{"event":{"data":{"block_height":0,"coin":"ETH","fee_details":null,"from":["0x3D6a2f4Dd6085b34EeD6cBc2D3aaABd0D3B697C1"],"internal_id":"00","my_balance_change":-0.0001,"received_by_me":0,"spent_by_me":0.0001,"timestamp":1563744020,"to":["0xD8997941Dd1346e9231118D5685d866294f59e5b"],"total_amount":0.0001,"tx_hash":"a59203eb2328827de00bed699a29389792906e4f39fdea145eb40dc6b3821bd6","tx_hex":"f8690284ee6b280082520894d8997941dd1346e9231118d5685d866294f59e5b865af3107a4000801ca0743d2b7c9fad65805d882179062012261be328d7628ae12ee08eff8d7657d993a07eecbd051f49d35279416778faa4664962726d516ce65e18755c9b9406a9c2fd"},"type":"TakerFeeSent"},"timestamp":1563744020598},{"event":{"data":{"block_height":0,"coin":"KMD","fee_details":{"amount":1e-05},"from":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd"],"internal_id":"0cf4acbcefde53645851c5c6053ea61fe0cbb5f828a906d69eb809e0b071a03b","my_balance_change":0,"received_by_me":0,"spent_by_me":0,"timestamp":0,"to":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd","bYAJo9wvPf3cBgPJ7NVwhoeAY1meGNxEGh"],"total_amount":12.51998004,"tx_hash":"0cf4acbcefde53645851c5c6053ea61fe0cbb5f828a906d69eb809e0b071a03b","tx_hex":"0400008085202f89025d5ae3e8c87418c9b735f8f2f7d29e26820c33c9f30d53f2d31f8b99ea9b1490010000006a47304402201185c06ca575261c539b287175751b7de642eb7466c59128639a19b4c2dd2f9b02201c8c4167d581864bedd4d1deb5596472e6e3ce29fe9e7996907a7b59c905d5490121031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ffffffff06dbf9971c8dfd4a0c8c49f4f15c51de59ba13b2efa702682e26869843af9a87000000006a473044022012b47c12c7f6ad7d8b778fc4b5dcfd56a39325daf302f56e7b84753ba5216cfa022076bf571cf9e20facf70d2f134e8ed2de67aa08581a27ff3128bf93a9b594ac770121031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ffffffff02fed727150000000017a914d5268b31131a652f9b6ddf57db62f02285cdfad1874e1d7835000000001976a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac37cf345d000000000000000000000000000000"},"type":"MakerPaymentReceived"},"timestamp":1563744071778},{"event":{"type":"MakerPaymentWaitConfirmStarted"},"timestamp":1563744071781},{"event":{"type":"MakerPaymentValidatedAndConfirmed"},"timestamp":1563744118073},{"event":{"data":{"error":"lp_swap:1888] eth:654] RPC error: Error { code: ServerError(-32010), message: \"Transaction with the same hash was already imported.\", data: None }"},"type":"TakerPaymentTransactionFailed"},"timestamp":1563744118577},{"event":{"type":"Finished"},"timestamp":1563744118580}],"success_events":["Started","Negotiated","TakerFeeSent","MakerPaymentReceived","MakerPaymentWaitConfirmStarted","MakerPaymentValidatedAndConfirmed","TakerPaymentSent","TakerPaymentSpent","MakerPaymentSpent","Finished"],"uuid":"3447b727-fe93-4357-8e5a-8cf2699b7e86"}"#;
        let taker_saved_swap: TakerSavedSwap = unwrap!(json::from_str(taker_saved_json));
        let key_pair = unwrap!(key_pair_from_seed("spice describe gravity federal blast come thank unfair canal monkey style afraid"));
        let ctx = MmCtxBuilder::default().with_secp256k1_key_pair(key_pair).into_mm_arc();

        TestCoin::ticker.mock_safe(|_| MockResult::Return("ticker"));

        static mut MY_PAYMENT_SENT_CALLED: bool = false;
        TestCoin::check_if_my_payment_sent.mock_safe(|_, _, _, _, _| {
            unsafe { MY_PAYMENT_SENT_CALLED = true };
            MockResult::Return(Box::new(futures01::future::ok(Some(eth_tx_for_test().into()))))
        });

        static mut SEARCH_TX_SPEND_CALLED: bool = false;
        TestCoin::search_for_swap_tx_spend_my.mock_safe(|_, _, _, _, _, _| {
            unsafe { SEARCH_TX_SPEND_CALLED = true };
            let tx: UtxoTx = "0100000001de7aa8d29524906b2b54ee2e0281f3607f75662cbc9080df81d1047b78e21dbc00000000d7473044022079b6c50820040b1fbbe9251ced32ab334d33830f6f8d0bf0a40c7f1336b67d5b0220142ccf723ddabb34e542ed65c395abc1fbf5b6c3e730396f15d25c49b668a1a401209da937e5609680cb30bff4a7661364ca1d1851c2506fa80c443f00a3d3bf7365004c6b6304f62b0e5cb175210270e75970bb20029b3879ec76c4acd320a8d0589e003636264d01a7d566504bfbac6782012088a9142fb610d856c19fd57f2d0cffe8dff689074b3d8a882103f368228456c940ac113e53dad5c104cf209f2f102a409207269383b6ab9b03deac68ffffffff01d0dc9800000000001976a9146d9d2b554d768232320587df75c4338ecc8bf37d88ac40280e5c".into();
            MockResult::Return(Ok(Some(FoundSwapTxSpend::Spent(tx.into()))))
        });

        TestCoin::search_for_swap_tx_spend_other.mock_safe(|_, _, _, _, _, _| MockResult::Return(Ok(None)));

        static mut MAKER_PAYMENT_SPEND_CALLED: bool = false;
        TestCoin::send_taker_spends_maker_payment.mock_safe(|_, _, _, _, _| {
            unsafe { MAKER_PAYMENT_SPEND_CALLED = true };
            MockResult::Return(Box::new(futures01::future::ok(eth_tx_for_test().into())))
        });
        let maker_coin = MmCoinEnum::Test(TestCoin {});
        let taker_coin = MmCoinEnum::Test(TestCoin {});
        let (taker_swap, _) = unwrap!(TakerSwap::load_from_saved(ctx, maker_coin, taker_coin, taker_saved_swap));
        let actual = unwrap!(taker_swap.recover_funds());
        let expected = RecoveredSwap {
            action: RecoveredSwapAction::SpentOtherPayment,
            coin: "ticker".to_string(),
            transaction: eth_tx_for_test().into(),
        };
        assert_eq!(expected, actual);
        assert!(unsafe { MY_PAYMENT_SENT_CALLED });
        assert!(unsafe { SEARCH_TX_SPEND_CALLED });
        assert!(unsafe { MAKER_PAYMENT_SPEND_CALLED });
    }

    #[test]
    fn test_recover_funds_taker_swap_taker_payment_refund_failed_not_spent() {
        let taker_saved_json = r#"{"error_events":["StartFailed","NegotiateFailed","TakerFeeSendFailed","MakerPaymentValidateFailed","TakerPaymentTransactionFailed","TakerPaymentDataSendFailed","TakerPaymentWaitForSpendFailed","MakerPaymentSpendFailed","TakerPaymentRefunded","TakerPaymentRefundFailed"],"events":[{"event":{"data":{"lock_duration":7800,"maker":"1bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","maker_amount":"0.58610590","maker_coin":"KMD","maker_coin_start_block":1450923,"maker_payment_confirmations":1,"maker_payment_wait":1563623475,"my_persistent_pub":"02713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91","started_at":1563620875,"taker_amount":"0.0077700000552410000000000","taker_coin":"LTC","taker_coin_start_block":1670837,"taker_payment_confirmations":1,"taker_payment_lock":1563628675,"uuid":"9db641f5-4300-4527-9fa6-f1c391d42c35"},"type":"Started"},"timestamp":1563620875766},{"event":{"data":{"maker_payment_locktime":1563636475,"maker_pubkey":"031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","secret_hash":"7ed38daab6085c1a1e4426e61dc87a3c2c081a95"},"type":"Negotiated"},"timestamp":1563620955014},{"event":{"data":{"block_height":0,"coin":"LTC","fee_details":{"amount":0.001},"from":["LKquWDGkJHEcFn85Dzw4FV5XwYp8GT3WvD"],"internal_id":"6740136eaaa615d9d231969e3a9599d0fc59e53989237a8d31cd6fc86c160013","my_balance_change":-0.0011,"received_by_me":0.0168204,"spent_by_me":0.0179204,"timestamp":0,"to":["LKquWDGkJHEcFn85Dzw4FV5XwYp8GT3WvD","LdeeicEe3dYpjy36TPWrufiGToyaaEP2Zs"],"total_amount":0.0179204,"tx_hash":"6740136eaaa615d9d231969e3a9599d0fc59e53989237a8d31cd6fc86c160013","tx_hex":"0100000001a2586ea8294cedc55741bef625ba72c646399903391a7f6c604a58c6263135f2000000006b4830450221009c78c8ba4a7accab6b09f9a95da5bc59c81f4fc1e60b288ec3c5462b4d02ef01022056b63be1629cf17751d3cc5ffec51bcb1d7f9396e9ce9ca254d0f34104f7263a012102713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91ffffffff0210270000000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac78aa1900000000001976a91406ccabfd5f9075ecd5e8d0d31c0e973a54d51e8288ac5bf6325d"},"type":"TakerFeeSent"},"timestamp":1563620958220},{"event":{"data":{"block_height":0,"coin":"KMD","fee_details":{"amount":1e-05},"from":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd"],"internal_id":"d0f6e664cea9d89fe7b5cf8005fdca070d1ab1d05a482aaef95c08cdaecddf0a","my_balance_change":0,"received_by_me":0,"spent_by_me":0,"timestamp":0,"to":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd","bLVo4svJDxUF6C2fVivmV91HJqVjrkkAf4"],"total_amount":0.99998999,"tx_hash":"d0f6e664cea9d89fe7b5cf8005fdca070d1ab1d05a482aaef95c08cdaecddf0a","tx_hex":"0400008085202f89019f1cbda354342cdf982046b331bbd3791f53b692efc6e4becc36be495b2977d9000000006b483045022100fa9d4557394141f6a8b9bfb8cd594a521fd8bcd1965dbf8bc4e04abc849ac66e0220589f521814c10a7561abfd5e432f7a2ee60d4875fe4604618af3207dae531ac00121031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ffffffff029e537e030000000017a9145534898009f1467191065f6890b96914b39a1c018791857702000000001976a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac72ee325d000000000000000000000000000000"},"type":"MakerPaymentReceived"},"timestamp":1563620999307},{"event":{"type":"MakerPaymentWaitConfirmStarted"},"timestamp":1563620999310},{"event":{"type":"MakerPaymentValidatedAndConfirmed"},"timestamp":1563621244153},{"event":{"data":{"block_height":0,"coin":"LTC","fee_details":{"amount":0.001},"from":["LKquWDGkJHEcFn85Dzw4FV5XwYp8GT3WvD"],"internal_id":"1e883eb2f3991e84ba27f53651f89b7dda708678a5b9813d043577f222b9ca30","my_balance_change":-0.00877,"received_by_me":0.0080504,"spent_by_me":0.0168204,"timestamp":0,"to":["3DgMcEEjxwXfnEVapgQSCBVy2tz9X41RmR","LKquWDGkJHEcFn85Dzw4FV5XwYp8GT3WvD"],"total_amount":0.0168204,"tx_hash":"1e883eb2f3991e84ba27f53651f89b7dda708678a5b9813d043577f222b9ca30","tx_hex":"01000000011300166cc86fcd318d7a238939e559fcd099953a9e9631d2d915a6aa6e134067010000006a47304402206781d5f2db2ff13d2ec7e266f774ea5630cc2dba4019e18e9716131b8b026051022006ebb33857b6d180f13aa6be2fc532f9734abde9d00ae14757e7d7ba3741c08c012102713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91ffffffff0228db0b000000000017a91483818667161bf94adda3964a81a231cbf6f5338187b0480c00000000001976a91406ccabfd5f9075ecd5e8d0d31c0e973a54d51e8288ac7cf7325d"},"type":"TakerPaymentSent"},"timestamp":1563621246370},{"event":{"data":{"error":"utxo:1145] rpc_clients:782] Waited too long until 1563628675 for output TransactionOutput { value: 777000, script_pubkey: a91483818667161bf94adda3964a81a231cbf6f5338187 } to be spent "},"type":"TakerPaymentWaitForSpendFailed"},"timestamp":1563638060370},{"event":{"data":{"error":"lp_swap:2025] utxo:938] rpc_clients:719] JsonRpcError { request: JsonRpcRequest { jsonrpc: \"2.0\", id: \"9\", method: \"blockchain.transaction.broadcast\", params: [String(\"010000000130cab922f27735043d81b9a5788670da7d9bf85136f527ba841e99f3b23e881e00000000b6473044022058a0c1da6bcf8c1418899ff8475f3ab6dddbff918528451c1fe71c2f7dad176302204c2e0bcf8f9b5f09e02ccfeb9256e9b34fb355ea655a5704a8a3fa920079b91501514c6b63048314335db1752102713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91ac6782012088a9147ed38daab6085c1a1e4426e61dc87a3c2c081a958821031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ac68feffffff0188540a00000000001976a91406ccabfd5f9075ecd5e8d0d31c0e973a54d51e8288ac1c2b335d\")] }, error: Response(Object({\"code\": Number(1), \"message\": String(\"the transaction was rejected by network rules.\\n\\nMissing inputs\\n[010000000130cab922f27735043d81b9a5788670da7d9bf85136f527ba841e99f3b23e881e00000000b6473044022058a0c1da6bcf8c1418899ff8475f3ab6dddbff918528451c1fe71c2f7dad176302204c2e0bcf8f9b5f09e02ccfeb9256e9b34fb355ea655a5704a8a3fa920079b91501514c6b63048314335db1752102713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91ac6782012088a9147ed38daab6085c1a1e4426e61dc87a3c2c081a958821031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ac68feffffff0188540a00000000001976a91406ccabfd5f9075ecd5e8d0d31c0e973a54d51e8288ac1c2b335d]\")})) }"},"type":"TakerPaymentRefundFailed"},"timestamp":1563638060583},{"event":{"type":"Finished"},"timestamp":1563638060585}],"success_events":["Started","Negotiated","TakerFeeSent","MakerPaymentReceived","MakerPaymentWaitConfirmStarted","MakerPaymentValidatedAndConfirmed","TakerPaymentSent","TakerPaymentSpent","MakerPaymentSpent","Finished"],"uuid":"9db641f5-4300-4527-9fa6-f1c391d42c35"}"#;
        let taker_saved_swap: TakerSavedSwap = unwrap!(json::from_str(taker_saved_json));
        let key_pair = unwrap!(key_pair_from_seed("spice describe gravity federal blast come thank unfair canal monkey style afraid"));
        let ctx = MmCtxBuilder::default().with_secp256k1_key_pair(key_pair).into_mm_arc();

        TestCoin::ticker.mock_safe(|_| MockResult::Return("ticker"));

        static mut SEARCH_TX_SPEND_CALLED: bool = false;
        TestCoin::search_for_swap_tx_spend_my.mock_safe(|_, _, _, _, _, _| {
            unsafe { SEARCH_TX_SPEND_CALLED = true };
            MockResult::Return(Ok(None))
        });

        static mut REFUND_CALLED: bool = false;
        TestCoin::send_taker_refunds_payment.mock_safe(|_, _, _, _, _| {
            unsafe { REFUND_CALLED = true };
            MockResult::Return(Box::new(futures01::future::ok(eth_tx_for_test().into())))
        });
        let maker_coin = MmCoinEnum::Test(TestCoin {});
        let taker_coin = MmCoinEnum::Test(TestCoin {});
        let (taker_swap, _) = unwrap!(TakerSwap::load_from_saved(ctx, maker_coin, taker_coin, taker_saved_swap));
        let actual = unwrap!(taker_swap.recover_funds());
        let expected = RecoveredSwap {
            action: RecoveredSwapAction::RefundedMyPayment,
            coin: "ticker".to_string(),
            transaction: eth_tx_for_test().into(),
        };
        assert_eq!(expected, actual);
        assert!(unsafe { SEARCH_TX_SPEND_CALLED });
        assert!(unsafe { REFUND_CALLED });
    }

    #[test]
    fn test_recover_funds_taker_swap_taker_payment_refund_failed_not_spent_too_early_to_refund() {
        let taker_saved_json = r#"{"error_events":["StartFailed","NegotiateFailed","TakerFeeSendFailed","MakerPaymentValidateFailed","TakerPaymentTransactionFailed","TakerPaymentDataSendFailed","TakerPaymentWaitForSpendFailed","MakerPaymentSpendFailed","TakerPaymentRefunded","TakerPaymentRefundFailed"],"events":[{"event":{"data":{"lock_duration":7800,"maker":"1bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","maker_amount":"0.58610590","maker_coin":"KMD","maker_coin_start_block":1450923,"maker_payment_confirmations":1,"maker_payment_wait":1563623475,"my_persistent_pub":"02713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91","started_at":1563620875,"taker_amount":"0.0077700000552410000000000","taker_coin":"LTC","taker_coin_start_block":1670837,"taker_payment_confirmations":1,"taker_payment_lock":1563628675,"uuid":"9db641f5-4300-4527-9fa6-f1c391d42c35"},"type":"Started"},"timestamp":1563620875766},{"event":{"data":{"maker_payment_locktime":1563636475,"maker_pubkey":"031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","secret_hash":"7ed38daab6085c1a1e4426e61dc87a3c2c081a95"},"type":"Negotiated"},"timestamp":1563620955014},{"event":{"data":{"block_height":0,"coin":"LTC","fee_details":{"amount":0.001},"from":["LKquWDGkJHEcFn85Dzw4FV5XwYp8GT3WvD"],"internal_id":"6740136eaaa615d9d231969e3a9599d0fc59e53989237a8d31cd6fc86c160013","my_balance_change":-0.0011,"received_by_me":0.0168204,"spent_by_me":0.0179204,"timestamp":0,"to":["LKquWDGkJHEcFn85Dzw4FV5XwYp8GT3WvD","LdeeicEe3dYpjy36TPWrufiGToyaaEP2Zs"],"total_amount":0.0179204,"tx_hash":"6740136eaaa615d9d231969e3a9599d0fc59e53989237a8d31cd6fc86c160013","tx_hex":"0100000001a2586ea8294cedc55741bef625ba72c646399903391a7f6c604a58c6263135f2000000006b4830450221009c78c8ba4a7accab6b09f9a95da5bc59c81f4fc1e60b288ec3c5462b4d02ef01022056b63be1629cf17751d3cc5ffec51bcb1d7f9396e9ce9ca254d0f34104f7263a012102713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91ffffffff0210270000000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac78aa1900000000001976a91406ccabfd5f9075ecd5e8d0d31c0e973a54d51e8288ac5bf6325d"},"type":"TakerFeeSent"},"timestamp":1563620958220},{"event":{"data":{"block_height":0,"coin":"KMD","fee_details":{"amount":1e-05},"from":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd"],"internal_id":"d0f6e664cea9d89fe7b5cf8005fdca070d1ab1d05a482aaef95c08cdaecddf0a","my_balance_change":0,"received_by_me":0,"spent_by_me":0,"timestamp":0,"to":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd","bLVo4svJDxUF6C2fVivmV91HJqVjrkkAf4"],"total_amount":0.99998999,"tx_hash":"d0f6e664cea9d89fe7b5cf8005fdca070d1ab1d05a482aaef95c08cdaecddf0a","tx_hex":"0400008085202f89019f1cbda354342cdf982046b331bbd3791f53b692efc6e4becc36be495b2977d9000000006b483045022100fa9d4557394141f6a8b9bfb8cd594a521fd8bcd1965dbf8bc4e04abc849ac66e0220589f521814c10a7561abfd5e432f7a2ee60d4875fe4604618af3207dae531ac00121031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ffffffff029e537e030000000017a9145534898009f1467191065f6890b96914b39a1c018791857702000000001976a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac72ee325d000000000000000000000000000000"},"type":"MakerPaymentReceived"},"timestamp":1563620999307},{"event":{"type":"MakerPaymentWaitConfirmStarted"},"timestamp":1563620999310},{"event":{"type":"MakerPaymentValidatedAndConfirmed"},"timestamp":1563621244153},{"event":{"data":{"block_height":0,"coin":"LTC","fee_details":{"amount":0.001},"from":["LKquWDGkJHEcFn85Dzw4FV5XwYp8GT3WvD"],"internal_id":"1e883eb2f3991e84ba27f53651f89b7dda708678a5b9813d043577f222b9ca30","my_balance_change":-0.00877,"received_by_me":0.0080504,"spent_by_me":0.0168204,"timestamp":0,"to":["3DgMcEEjxwXfnEVapgQSCBVy2tz9X41RmR","LKquWDGkJHEcFn85Dzw4FV5XwYp8GT3WvD"],"total_amount":0.0168204,"tx_hash":"1e883eb2f3991e84ba27f53651f89b7dda708678a5b9813d043577f222b9ca30","tx_hex":"01000000011300166cc86fcd318d7a238939e559fcd099953a9e9631d2d915a6aa6e134067010000006a47304402206781d5f2db2ff13d2ec7e266f774ea5630cc2dba4019e18e9716131b8b026051022006ebb33857b6d180f13aa6be2fc532f9734abde9d00ae14757e7d7ba3741c08c012102713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91ffffffff0228db0b000000000017a91483818667161bf94adda3964a81a231cbf6f5338187b0480c00000000001976a91406ccabfd5f9075ecd5e8d0d31c0e973a54d51e8288ac7cf7325d"},"type":"TakerPaymentSent"},"timestamp":1563621246370},{"event":{"data":{"error":"utxo:1145] rpc_clients:782] Waited too long until 1563628675 for output TransactionOutput { value: 777000, script_pubkey: a91483818667161bf94adda3964a81a231cbf6f5338187 } to be spent "},"type":"TakerPaymentWaitForSpendFailed"},"timestamp":1563638060370},{"event":{"data":{"error":"lp_swap:2025] utxo:938] rpc_clients:719] JsonRpcError { request: JsonRpcRequest { jsonrpc: \"2.0\", id: \"9\", method: \"blockchain.transaction.broadcast\", params: [String(\"010000000130cab922f27735043d81b9a5788670da7d9bf85136f527ba841e99f3b23e881e00000000b6473044022058a0c1da6bcf8c1418899ff8475f3ab6dddbff918528451c1fe71c2f7dad176302204c2e0bcf8f9b5f09e02ccfeb9256e9b34fb355ea655a5704a8a3fa920079b91501514c6b63048314335db1752102713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91ac6782012088a9147ed38daab6085c1a1e4426e61dc87a3c2c081a958821031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ac68feffffff0188540a00000000001976a91406ccabfd5f9075ecd5e8d0d31c0e973a54d51e8288ac1c2b335d\")] }, error: Response(Object({\"code\": Number(1), \"message\": String(\"the transaction was rejected by network rules.\\n\\nMissing inputs\\n[010000000130cab922f27735043d81b9a5788670da7d9bf85136f527ba841e99f3b23e881e00000000b6473044022058a0c1da6bcf8c1418899ff8475f3ab6dddbff918528451c1fe71c2f7dad176302204c2e0bcf8f9b5f09e02ccfeb9256e9b34fb355ea655a5704a8a3fa920079b91501514c6b63048314335db1752102713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91ac6782012088a9147ed38daab6085c1a1e4426e61dc87a3c2c081a958821031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ac68feffffff0188540a00000000001976a91406ccabfd5f9075ecd5e8d0d31c0e973a54d51e8288ac1c2b335d]\")})) }"},"type":"TakerPaymentRefundFailed"},"timestamp":1563638060583},{"event":{"type":"Finished"},"timestamp":1563638060585}],"success_events":["Started","Negotiated","TakerFeeSent","MakerPaymentReceived","MakerPaymentWaitConfirmStarted","MakerPaymentValidatedAndConfirmed","TakerPaymentSent","TakerPaymentSpent","MakerPaymentSpent","Finished"],"uuid":"9db641f5-4300-4527-9fa6-f1c391d42c35"}"#;
        let taker_saved_swap: TakerSavedSwap = unwrap!(json::from_str(taker_saved_json));
        let key_pair = unwrap!(key_pair_from_seed("spice describe gravity federal blast come thank unfair canal monkey style afraid"));
        let ctx = MmCtxBuilder::default().with_secp256k1_key_pair(key_pair).into_mm_arc();

        TestCoin::ticker.mock_safe(|_| MockResult::Return("ticker"));

        static mut SEARCH_TX_SPEND_CALLED: bool = false;
        TestCoin::search_for_swap_tx_spend_my.mock_safe(|_, _, _, _, _, _| {
            unsafe {SEARCH_TX_SPEND_CALLED = true};
            MockResult::Return(Ok(None))
        });
        let maker_coin = MmCoinEnum::Test(TestCoin {});
        let taker_coin = MmCoinEnum::Test(TestCoin {});
        let (taker_swap, _) = unwrap!(TakerSwap::load_from_saved(ctx, maker_coin, taker_coin, taker_saved_swap));
        taker_swap.w().data.taker_payment_lock = (now_ms() / 1000) - 3690;
        assert!(taker_swap.recover_funds().is_err());
        assert!(unsafe { SEARCH_TX_SPEND_CALLED });
    }

    #[test]
    fn test_recover_funds_taker_swap_taker_payment_refund_failed_spent_by_maker() {
        let taker_saved_json = r#"{"error_events":["StartFailed","NegotiateFailed","TakerFeeSendFailed","MakerPaymentValidateFailed","TakerPaymentTransactionFailed","TakerPaymentDataSendFailed","TakerPaymentWaitForSpendFailed","MakerPaymentSpendFailed","TakerPaymentRefunded","TakerPaymentRefundFailed"],"events":[{"event":{"data":{"lock_duration":7800,"maker":"1bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","maker_amount":"0.58610590","maker_coin":"KMD","maker_coin_start_block":1450923,"maker_payment_confirmations":1,"maker_payment_wait":1563623475,"my_persistent_pub":"02713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91","started_at":1563620875,"taker_amount":"0.0077700000552410000000000","taker_coin":"LTC","taker_coin_start_block":1670837,"taker_payment_confirmations":1,"taker_payment_lock":1563628675,"uuid":"9db641f5-4300-4527-9fa6-f1c391d42c35"},"type":"Started"},"timestamp":1563620875766},{"event":{"data":{"maker_payment_locktime":1563636475,"maker_pubkey":"031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","secret_hash":"7ed38daab6085c1a1e4426e61dc87a3c2c081a95"},"type":"Negotiated"},"timestamp":1563620955014},{"event":{"data":{"block_height":0,"coin":"LTC","fee_details":{"amount":0.001},"from":["LKquWDGkJHEcFn85Dzw4FV5XwYp8GT3WvD"],"internal_id":"6740136eaaa615d9d231969e3a9599d0fc59e53989237a8d31cd6fc86c160013","my_balance_change":-0.0011,"received_by_me":0.0168204,"spent_by_me":0.0179204,"timestamp":0,"to":["LKquWDGkJHEcFn85Dzw4FV5XwYp8GT3WvD","LdeeicEe3dYpjy36TPWrufiGToyaaEP2Zs"],"total_amount":0.0179204,"tx_hash":"6740136eaaa615d9d231969e3a9599d0fc59e53989237a8d31cd6fc86c160013","tx_hex":"0100000001a2586ea8294cedc55741bef625ba72c646399903391a7f6c604a58c6263135f2000000006b4830450221009c78c8ba4a7accab6b09f9a95da5bc59c81f4fc1e60b288ec3c5462b4d02ef01022056b63be1629cf17751d3cc5ffec51bcb1d7f9396e9ce9ca254d0f34104f7263a012102713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91ffffffff0210270000000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac78aa1900000000001976a91406ccabfd5f9075ecd5e8d0d31c0e973a54d51e8288ac5bf6325d"},"type":"TakerFeeSent"},"timestamp":1563620958220},{"event":{"data":{"block_height":0,"coin":"KMD","fee_details":{"amount":1e-05},"from":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd"],"internal_id":"d0f6e664cea9d89fe7b5cf8005fdca070d1ab1d05a482aaef95c08cdaecddf0a","my_balance_change":0,"received_by_me":0,"spent_by_me":0,"timestamp":0,"to":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd","bLVo4svJDxUF6C2fVivmV91HJqVjrkkAf4"],"total_amount":0.99998999,"tx_hash":"d0f6e664cea9d89fe7b5cf8005fdca070d1ab1d05a482aaef95c08cdaecddf0a","tx_hex":"0400008085202f89019f1cbda354342cdf982046b331bbd3791f53b692efc6e4becc36be495b2977d9000000006b483045022100fa9d4557394141f6a8b9bfb8cd594a521fd8bcd1965dbf8bc4e04abc849ac66e0220589f521814c10a7561abfd5e432f7a2ee60d4875fe4604618af3207dae531ac00121031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ffffffff029e537e030000000017a9145534898009f1467191065f6890b96914b39a1c018791857702000000001976a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac72ee325d000000000000000000000000000000"},"type":"MakerPaymentReceived"},"timestamp":1563620999307},{"event":{"type":"MakerPaymentWaitConfirmStarted"},"timestamp":1563620999310},{"event":{"type":"MakerPaymentValidatedAndConfirmed"},"timestamp":1563621244153},{"event":{"data":{"block_height":0,"coin":"LTC","fee_details":{"amount":0.001},"from":["LKquWDGkJHEcFn85Dzw4FV5XwYp8GT3WvD"],"internal_id":"1e883eb2f3991e84ba27f53651f89b7dda708678a5b9813d043577f222b9ca30","my_balance_change":-0.00877,"received_by_me":0.0080504,"spent_by_me":0.0168204,"timestamp":0,"to":["3DgMcEEjxwXfnEVapgQSCBVy2tz9X41RmR","LKquWDGkJHEcFn85Dzw4FV5XwYp8GT3WvD"],"total_amount":0.0168204,"tx_hash":"1e883eb2f3991e84ba27f53651f89b7dda708678a5b9813d043577f222b9ca30","tx_hex":"01000000011300166cc86fcd318d7a238939e559fcd099953a9e9631d2d915a6aa6e134067010000006a47304402206781d5f2db2ff13d2ec7e266f774ea5630cc2dba4019e18e9716131b8b026051022006ebb33857b6d180f13aa6be2fc532f9734abde9d00ae14757e7d7ba3741c08c012102713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91ffffffff0228db0b000000000017a91483818667161bf94adda3964a81a231cbf6f5338187b0480c00000000001976a91406ccabfd5f9075ecd5e8d0d31c0e973a54d51e8288ac7cf7325d"},"type":"TakerPaymentSent"},"timestamp":1563621246370},{"event":{"data":{"error":"utxo:1145] rpc_clients:782] Waited too long until 1563628675 for output TransactionOutput { value: 777000, script_pubkey: a91483818667161bf94adda3964a81a231cbf6f5338187 } to be spent "},"type":"TakerPaymentWaitForSpendFailed"},"timestamp":1563638060370},{"event":{"data":{"error":"lp_swap:2025] utxo:938] rpc_clients:719] JsonRpcError { request: JsonRpcRequest { jsonrpc: \"2.0\", id: \"9\", method: \"blockchain.transaction.broadcast\", params: [String(\"010000000130cab922f27735043d81b9a5788670da7d9bf85136f527ba841e99f3b23e881e00000000b6473044022058a0c1da6bcf8c1418899ff8475f3ab6dddbff918528451c1fe71c2f7dad176302204c2e0bcf8f9b5f09e02ccfeb9256e9b34fb355ea655a5704a8a3fa920079b91501514c6b63048314335db1752102713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91ac6782012088a9147ed38daab6085c1a1e4426e61dc87a3c2c081a958821031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ac68feffffff0188540a00000000001976a91406ccabfd5f9075ecd5e8d0d31c0e973a54d51e8288ac1c2b335d\")] }, error: Response(Object({\"code\": Number(1), \"message\": String(\"the transaction was rejected by network rules.\\n\\nMissing inputs\\n[010000000130cab922f27735043d81b9a5788670da7d9bf85136f527ba841e99f3b23e881e00000000b6473044022058a0c1da6bcf8c1418899ff8475f3ab6dddbff918528451c1fe71c2f7dad176302204c2e0bcf8f9b5f09e02ccfeb9256e9b34fb355ea655a5704a8a3fa920079b91501514c6b63048314335db1752102713015d3fa4d30259e90be5f131beb593bf0131f3af2dcdb304e3322d8d52b91ac6782012088a9147ed38daab6085c1a1e4426e61dc87a3c2c081a958821031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ac68feffffff0188540a00000000001976a91406ccabfd5f9075ecd5e8d0d31c0e973a54d51e8288ac1c2b335d]\")})) }"},"type":"TakerPaymentRefundFailed"},"timestamp":1563638060583},{"event":{"type":"Finished"},"timestamp":1563638060585}],"success_events":["Started","Negotiated","TakerFeeSent","MakerPaymentReceived","MakerPaymentWaitConfirmStarted","MakerPaymentValidatedAndConfirmed","TakerPaymentSent","TakerPaymentSpent","MakerPaymentSpent","Finished"],"uuid":"9db641f5-4300-4527-9fa6-f1c391d42c35"}"#;
        let taker_saved_swap: TakerSavedSwap = unwrap!(json::from_str(taker_saved_json));
        let key_pair = unwrap!(key_pair_from_seed("spice describe gravity federal blast come thank unfair canal monkey style afraid"));
        let ctx = MmCtxBuilder::default().with_secp256k1_key_pair(key_pair).into_mm_arc();

        TestCoin::ticker.mock_safe(|_| MockResult::Return("ticker"));

        static mut SEARCH_TX_SPEND_CALLED: bool = false;
        TestCoin::search_for_swap_tx_spend_my.mock_safe(|_, _, _, _, _, _| {
            unsafe { SEARCH_TX_SPEND_CALLED = true };
            let tx: UtxoTx = "0100000001de7aa8d29524906b2b54ee2e0281f3607f75662cbc9080df81d1047b78e21dbc00000000d7473044022079b6c50820040b1fbbe9251ced32ab334d33830f6f8d0bf0a40c7f1336b67d5b0220142ccf723ddabb34e542ed65c395abc1fbf5b6c3e730396f15d25c49b668a1a401209da937e5609680cb30bff4a7661364ca1d1851c2506fa80c443f00a3d3bf7365004c6b6304f62b0e5cb175210270e75970bb20029b3879ec76c4acd320a8d0589e003636264d01a7d566504bfbac6782012088a9142fb610d856c19fd57f2d0cffe8dff689074b3d8a882103f368228456c940ac113e53dad5c104cf209f2f102a409207269383b6ab9b03deac68ffffffff01d0dc9800000000001976a9146d9d2b554d768232320587df75c4338ecc8bf37d88ac40280e5c".into();
            MockResult::Return(Ok(Some(FoundSwapTxSpend::Spent(tx.into()))))
        });

        TestCoin::search_for_swap_tx_spend_other.mock_safe(|_, _, _, _, _, _| MockResult::Return(Ok(None)));

        static mut MAKER_PAYMENT_SPEND_CALLED: bool = false;
        TestCoin::send_taker_spends_maker_payment.mock_safe(|_, _, _, _, _| {
            unsafe { MAKER_PAYMENT_SPEND_CALLED = true };
            MockResult::Return(Box::new(futures01::future::ok(eth_tx_for_test().into())))
        });
        let maker_coin = MmCoinEnum::Test(TestCoin {});
        let taker_coin = MmCoinEnum::Test(TestCoin {});
        let (taker_swap, _) = unwrap!(TakerSwap::load_from_saved(ctx, maker_coin, taker_coin, taker_saved_swap));
        let actual = unwrap!(taker_swap.recover_funds());
        let expected = RecoveredSwap {
            action: RecoveredSwapAction::SpentOtherPayment,
            coin: "ticker".to_string(),
            transaction: eth_tx_for_test().into(),
        };
        assert_eq!(expected, actual);
        assert!(unsafe { SEARCH_TX_SPEND_CALLED });
        assert!(unsafe { MAKER_PAYMENT_SPEND_CALLED });
    }

    #[test]
    fn test_recover_funds_taker_swap_not_finished() {
        // the json doesn't have Finished event at the end
        let taker_saved_json = r#"{"error_events":["StartFailed","NegotiateFailed","TakerFeeSendFailed","MakerPaymentValidateFailed","TakerPaymentTransactionFailed","TakerPaymentDataSendFailed","TakerPaymentWaitForSpendFailed","MakerPaymentSpendFailed","TakerPaymentRefunded","TakerPaymentRefundFailed"],"events":[{"event":{"data":{"lock_duration":7800,"maker":"1bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","maker_amount":"0.12596566232185483","maker_coin":"KMD","maker_coin_start_block":1458035,"maker_payment_confirmations":1,"maker_payment_wait":1564053079,"my_persistent_pub":"0326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0a","started_at":1564050479,"taker_amount":"50.000000000000001504212457800000","taker_coin":"DOGE","taker_coin_start_block":2823448,"taker_payment_confirmations":1,"taker_payment_lock":1564058279,"uuid":"41383f43-46a5-478c-9386-3b2cce0aca20"},"type":"Started"},"timestamp":1564050480269},{"event":{"data":{"maker_payment_locktime":1564066080,"maker_pubkey":"031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","secret_hash":"3669eb83a007a3c507448d79f45a9f06ec2f36a8"},"type":"Negotiated"},"timestamp":1564050540991},{"event":{"data":{"block_height":0,"coin":"DOGE","fee_details":{"amount":5},"from":["DBNHC8sQS8SCwCrKzG57G7ZVCh1zaih2tx"],"internal_id":"bdde828b492d6d1cc25cd2322fd592dafd722fcc7d8b0fedce4d3bb4a1a8c8ff","my_balance_change":-5.05791505,"received_by_me":96.85084225,"spent_by_me":101.9087573,"timestamp":0,"to":["DBNHC8sQS8SCwCrKzG57G7ZVCh1zaih2tx","DPZnzesTGPD42AXY1qX8BQp78jLbmzpRT7"],"total_amount":101.9087573,"tx_hash":"bdde828b492d6d1cc25cd2322fd592dafd722fcc7d8b0fedce4d3bb4a1a8c8ff","tx_hex":"0100000002c7efa995c8b7be0a8b6c2d526c6c444c1634d65584e9ee89904e9d8675eac88c010000006a473044022051f34d5e3b7d0b9098d5e35333f3550f9cb9e57df83d5e4635b7a8d2986d6d5602200288c98da05de6950e01229a637110a1800ba643e75cfec59d4eb1021ad9b40801210326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0affffffffae6c233989efa7c7d2aa6534adc96078917ff395b7f09f734a147b2f44ade164000000006a4730440220393a784c2da74d0e2a28ec4f7df6c8f9d8b2af6ae6957f1e68346d744223a8fd02201b7a96954ac06815a43a6c7668d829ae9cbb5de76fa77189ddfd9e3038df662c01210326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0affffffff02115f5800000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac41a84641020000001976a914444f0e1099709ba4d742454a7d98a5c9c162ceab88ac6d84395d"},"type":"TakerFeeSent"},"timestamp":1564050545296},{"event":{"data":{"block_height":0,"coin":"KMD","fee_details":{"amount":1e-05},"from":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd"],"internal_id":"0a0f11fa82802c2c30862c50ab2162185dae8de7f7235f32c506f814c142b382","my_balance_change":0,"received_by_me":0,"spent_by_me":0,"timestamp":0,"to":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd","bQTa5QiudricscFpKeJpcvi3rqFW4YEBcs"],"total_amount":1.10033066,"tx_hash":"0a0f11fa82802c2c30862c50ab2162185dae8de7f7235f32c506f814c142b382","tx_hex":"0400008085202f8902ace337db2dd4c56b0697f58fb8cfb6bd1cd6f469d925fc0376d1dcfb7581bf82000000006b483045022100d1f95be235c5c8880f5d703ace287e2768548792c58c5dbd27f5578881b30ea70220030596106e21c7e0057ee0dab283f9a1fe273f15208cba80870c447bd559ef0d0121031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ffffffff9f339752567c404427fd77f2b35cecdb4c21489edc64e25e729fdb281785e423000000006a47304402203179e95877dbc107123a417f1e648e3ff13d384890f1e4a67b6dd5087235152e0220102a8ab799fadb26b5d89ceb9c7bc721a7e0c2a0d0d7e46bbe0cf3d130010d430121031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ffffffff025635c0000000000017a91480a95d366d65e34a465ab17b0c9eb1d5a33bae08876cbfce05000000001976a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac8d7c395d000000000000000000000000000000"},"type":"MakerPaymentReceived"},"timestamp":1564050588176},{"event":{"type":"MakerPaymentWaitConfirmStarted"},"timestamp":1564050588178},{"event":{"type":"MakerPaymentValidatedAndConfirmed"},"timestamp":1564050693585},{"event":{"data":{"block_height":0,"coin":"DOGE","fee_details":{"amount":5},"from":["DBNHC8sQS8SCwCrKzG57G7ZVCh1zaih2tx"],"internal_id":"539cb6dbdc25465bbccc575554f05d1bb04c70efce4316e41194e747375c3659","my_balance_change":-55,"received_by_me":41.85084225,"spent_by_me":96.85084225,"timestamp":0,"to":["AEb1iUQHozeuCifeFxbcvKmoKdFToF4p27","DBNHC8sQS8SCwCrKzG57G7ZVCh1zaih2tx"],"total_amount":96.85084225,"tx_hash":"539cb6dbdc25465bbccc575554f05d1bb04c70efce4316e41194e747375c3659","tx_hex":"0100000001ffc8a8a1b43b4dceed0f8b7dcc2f72fdda92d52f32d25cc21c6d2d498b82debd010000006a47304402203967b7f9f5532fa47116585c7d1bcba51861ea2059cca00409f34660db18e33a0220640991911852533a12fdfeb039fb9c8ca2c45482c6993bd84636af3670d49c1501210326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0affffffff0200f2052a0100000017a914f2fa08ae416b576779ae5da975e5442663215fce87415173f9000000001976a914444f0e1099709ba4d742454a7d98a5c9c162ceab88ac0585395d"},"type":"TakerPaymentSent"},"timestamp":1564050695611},{"event":{"data":{"secret":"1b8886b8a2cdb62505699400b694ac20f04d7bd4abd80e1ab154aa8d861fc093","transaction":{"block_height":0,"coin":"DOGE","fee_details":{"amount":5},"from":["AEb1iUQHozeuCifeFxbcvKmoKdFToF4p27"],"internal_id":"cc5af1cf68d246419fee49c3d74c0cd173599d115b86efe274368a614951bc47","my_balance_change":0,"received_by_me":0,"spent_by_me":0,"timestamp":1564050913,"to":["DP1GH73GKRdRz8Qxc8mxe2hFtu5NRwTBXV"],"total_amount":50,"tx_hash":"cc5af1cf68d246419fee49c3d74c0cd173599d115b86efe274368a614951bc47","tx_hex":"010000000159365c3747e79411e41643ceef704cb01b5df0545557ccbc5b4625dcdbb69c5300000000d747304402200e78e27d2f1c18676f98ca3dfa4e4a9eeaa8209b55f57b4dd5d9e1abdf034cfa0220623b5c22b62234cec230342aa306c497e43494b44ec2425b84e236b1bf01257001201b8886b8a2cdb62505699400b694ac20f04d7bd4abd80e1ab154aa8d861fc093004c6b6304a7a2395db175210326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0aac6782012088a9143669eb83a007a3c507448d79f45a9f06ec2f36a88821031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ac68ffffffff01008d380c010000001976a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac8c77395d"}},"type":"TakerPaymentSpent"},"timestamp":1564051092890},{"event":{"data":{"error":"lp_swap:1981] utxo:891] rpc_clients:738] JsonRpcError { request: JsonRpcRequest { jsonrpc: \"2.0\", id: \"67\", method: \"blockchain.transaction.broadcast\", params: [String(\"0400008085202f890182b342c114f806c5325f23f7e78dae5d186221ab502c86302c2c8082fa110f0a00000000d7473044022035791ea5548f87484065c9e1f0bdca9ebc699f2c7f51182c84f360102e32dc3d02200612ed53bca52d9c2568437f087598531534badf26229fe0f652ea72ddf03ca501201b8886b8a2cdb62505699400b694ac20f04d7bd4abd80e1ab154aa8d861fc093004c6b630420c1395db17521031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ac6782012088a9143669eb83a007a3c507448d79f45a9f06ec2f36a888210326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0aac68ffffffff01460ec000000000001976a914444f0e1099709ba4d742454a7d98a5c9c162ceab88ac967e395d000000000000000000000000000000\")] }, error: Transport(\"rpc_clients:668] All electrums are currently disconnected\") }"},"type":"MakerPaymentSpendFailed"},"timestamp":1564051092897}],"success_events":["Started","Negotiated","TakerFeeSent","MakerPaymentReceived","MakerPaymentWaitConfirmStarted","MakerPaymentValidatedAndConfirmed","TakerPaymentSent","TakerPaymentSpent","MakerPaymentSpent","Finished"],"uuid":"41383f43-46a5-478c-9386-3b2cce0aca20"}"#;
        let taker_saved_swap: TakerSavedSwap = unwrap!(json::from_str(taker_saved_json));
        let key_pair = unwrap!(key_pair_from_seed("spice describe gravity federal blast come thank unfair canal monkey style afraid"));
        let ctx = MmCtxBuilder::default().with_secp256k1_key_pair(key_pair).into_mm_arc();

        TestCoin::ticker.mock_safe(|_| MockResult::Return("ticker"));
        let maker_coin = MmCoinEnum::Test(TestCoin {});
        let taker_coin = MmCoinEnum::Test(TestCoin {});
        let (taker_swap, _) = unwrap!(TakerSwap::load_from_saved(ctx, maker_coin, taker_coin, taker_saved_swap));
        assert!(taker_swap.recover_funds().is_err());
    }

    #[test]
    fn test_taker_swap_event_should_ban() {
        let event = TakerSwapEvent::TakerPaymentWaitConfirmFailed("err".into());
        assert!(!event.should_ban_maker());

        let event = TakerSwapEvent::MakerPaymentWaitConfirmFailed("err".into());
        assert!(!event.should_ban_maker());

        let event = TakerSwapEvent::NegotiateFailed("err".into());
        assert!(event.should_ban_maker());

        let event = TakerSwapEvent::MakerPaymentValidateFailed("err".into());
        assert!(event.should_ban_maker());

        let event = TakerSwapEvent::TakerPaymentWaitForSpendFailed("err".into());
        assert!(event.should_ban_maker());
    }

    #[test]
    // https://github.com/KomodoPlatform/atomicDEX-API/issues/647
    fn test_recoverable() {
        // Swap ended with MakerPaymentWaitConfirmFailed event.
        // MM2 did not attempt to send the payment in this case so swap is not recoverable.
        let swap: TakerSavedSwap = json::from_str(r#"{"error_events":["StartFailed","NegotiateFailed","TakerFeeSendFailed","MakerPaymentValidateFailed","MakerPaymentWaitConfirmFailed","TakerPaymentTransactionFailed","TakerPaymentWaitConfirmFailed","TakerPaymentDataSendFailed","TakerPaymentWaitForSpendFailed","MakerPaymentSpendFailed","TakerPaymentWaitRefundStarted","TakerPaymentRefunded","TakerPaymentRefundFailed"],"events":[{"event":{"data":{"lock_duration":7800,"maker":"1bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","maker_amount":"0.12596566232185483","maker_coin":"KMD","maker_coin_start_block":1458035,"maker_payment_confirmations":1,"maker_payment_wait":1564053079,"my_persistent_pub":"0326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0a","started_at":1564050479,"taker_amount":"50.000000000000001504212457800000","taker_coin":"DOGE","taker_coin_start_block":2823448,"taker_payment_confirmations":1,"taker_payment_lock":1564058279,"uuid":"41383f43-46a5-478c-9386-3b2cce0aca20"},"type":"Started"},"timestamp":1564050480269},{"event":{"data":{"maker_payment_locktime":1564066080,"maker_pubkey":"031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8","secret_hash":"3669eb83a007a3c507448d79f45a9f06ec2f36a8"},"type":"Negotiated"},"timestamp":1564050540991},{"event":{"data":{"block_height":0,"coin":"DOGE","fee_details":{"amount":5},"from":["DBNHC8sQS8SCwCrKzG57G7ZVCh1zaih2tx"],"internal_id":"bdde828b492d6d1cc25cd2322fd592dafd722fcc7d8b0fedce4d3bb4a1a8c8ff","my_balance_change":-5.05791505,"received_by_me":96.85084225,"spent_by_me":101.9087573,"timestamp":0,"to":["DBNHC8sQS8SCwCrKzG57G7ZVCh1zaih2tx","DPZnzesTGPD42AXY1qX8BQp78jLbmzpRT7"],"total_amount":101.9087573,"tx_hash":"bdde828b492d6d1cc25cd2322fd592dafd722fcc7d8b0fedce4d3bb4a1a8c8ff","tx_hex":"0100000002c7efa995c8b7be0a8b6c2d526c6c444c1634d65584e9ee89904e9d8675eac88c010000006a473044022051f34d5e3b7d0b9098d5e35333f3550f9cb9e57df83d5e4635b7a8d2986d6d5602200288c98da05de6950e01229a637110a1800ba643e75cfec59d4eb1021ad9b40801210326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0affffffffae6c233989efa7c7d2aa6534adc96078917ff395b7f09f734a147b2f44ade164000000006a4730440220393a784c2da74d0e2a28ec4f7df6c8f9d8b2af6ae6957f1e68346d744223a8fd02201b7a96954ac06815a43a6c7668d829ae9cbb5de76fa77189ddfd9e3038df662c01210326846707a52a233cfc49a61ef51b1698bbe6aa78fa8b8d411c02743c09688f0affffffff02115f5800000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac41a84641020000001976a914444f0e1099709ba4d742454a7d98a5c9c162ceab88ac6d84395d"},"type":"TakerFeeSent"},"timestamp":1564050545296},{"event":{"data":{"block_height":0,"coin":"KMD","fee_details":{"amount":0.00001},"from":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd"],"internal_id":"0a0f11fa82802c2c30862c50ab2162185dae8de7f7235f32c506f814c142b382","my_balance_change":0,"received_by_me":0,"spent_by_me":0,"timestamp":0,"to":["RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd","bQTa5QiudricscFpKeJpcvi3rqFW4YEBcs"],"total_amount":1.10033066,"tx_hash":"0a0f11fa82802c2c30862c50ab2162185dae8de7f7235f32c506f814c142b382","tx_hex":"0400008085202f8902ace337db2dd4c56b0697f58fb8cfb6bd1cd6f469d925fc0376d1dcfb7581bf82000000006b483045022100d1f95be235c5c8880f5d703ace287e2768548792c58c5dbd27f5578881b30ea70220030596106e21c7e0057ee0dab283f9a1fe273f15208cba80870c447bd559ef0d0121031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ffffffff9f339752567c404427fd77f2b35cecdb4c21489edc64e25e729fdb281785e423000000006a47304402203179e95877dbc107123a417f1e648e3ff13d384890f1e4a67b6dd5087235152e0220102a8ab799fadb26b5d89ceb9c7bc721a7e0c2a0d0d7e46bbe0cf3d130010d430121031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ffffffff025635c0000000000017a91480a95d366d65e34a465ab17b0c9eb1d5a33bae08876cbfce05000000001976a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac8d7c395d000000000000000000000000000000"},"type":"MakerPaymentReceived"},"timestamp":1564050588176},{"event":{"type":"MakerPaymentWaitConfirmStarted"},"timestamp":1564050588178},{"event":{"data":{"error":"error"},"type":"MakerPaymentWaitConfirmFailed"},"timestamp":1564051092897},{"event":{"type":"Finished"},"timestamp":1564051092900}],"success_events":["Started","Negotiated","TakerFeeSent","MakerPaymentReceived","MakerPaymentWaitConfirmStarted","MakerPaymentValidatedAndConfirmed","TakerPaymentSent","TakerPaymentSpent","MakerPaymentSpent","Finished"],"uuid":"41383f43-46a5-478c-9386-3b2cce0aca20"}"#).unwrap();
        assert!(!swap.is_recoverable());
    }
}
