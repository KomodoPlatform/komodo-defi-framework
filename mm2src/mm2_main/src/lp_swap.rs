//! Atomic swap loops and states
//!
//! # A note on the terminology used
//!
//! Alice = Buyer = Liquidity receiver = Taker
//! ("*The process of an atomic swap begins with the person who makes the initial request — this is the liquidity receiver*" - Komodo Whitepaper).
//!
//! Bob = Seller = Liquidity provider = Market maker
//! ("*On the other side of the atomic swap, we have the liquidity provider — we call this person, Bob*" - Komodo Whitepaper).
//!
//! # Algorithm updates
//!
//! At the end of 2018 most UTXO coins have BIP65 (https://github.com/bitcoin/bips/blob/master/bip-0065.mediawiki).
//! The previous swap protocol discussions took place at 2015-2016 when there were just a few
//! projects that implemented CLTV opcode support:
//! https://bitcointalk.org/index.php?topic=1340621.msg13828271#msg13828271
//! https://bitcointalk.org/index.php?topic=1364951
//! So the Tier Nolan approach is a bit outdated, the main purpose was to allow swapping of a coin
//! that doesn't have CLTV at least as Alice side (as APayment is 2of2 multisig).
//! Nowadays the protocol can be simplified to the following (UTXO coins, BTC and forks):
//!
//! 1. AFee: OP_DUP OP_HASH160 FEE_RMD160 OP_EQUALVERIFY OP_CHECKSIG
//!
//! 2. BPayment:
//! OP_IF
//! <now + LOCKTIME*2> OP_CLTV OP_DROP <bob_pub> OP_CHECKSIG
//! OP_ELSE
//! OP_SIZE 32 OP_EQUALVERIFY OP_HASH160 <hash(bob_privN)> OP_EQUALVERIFY <alice_pub> OP_CHECKSIG
//! OP_ENDIF
//!
//! 3. APayment:
//! OP_IF
//! <now + LOCKTIME> OP_CLTV OP_DROP <alice_pub> OP_CHECKSIG
//! OP_ELSE
//! OP_SIZE 32 OP_EQUALVERIFY OP_HASH160 <hash(bob_privN)> OP_EQUALVERIFY <bob_pub> OP_CHECKSIG
//! OP_ENDIF
//!

/******************************************************************************
 * Copyright © 2022 Atomic Private Limited and its contributors               *
 *                                                                            *
 * See the CONTRIBUTOR-LICENSE-AGREEMENT, COPYING, LICENSE-COPYRIGHT-NOTICE   *
 * and DEVELOPER-CERTIFICATE-OF-ORIGIN files in the LEGAL directory in        *
 * the top-level directory of this distribution for the individual copyright  *
 * holder information and the developer policies on copyright and licensing.  *
 *                                                                            *
 * Unless otherwise agreed in a custom licensing agreement, no part of the    *
 * AtomicDEX software, including this file may be copied, modified, propagated*
 * or distributed except according to the terms contained in the              *
 * LICENSE-COPYRIGHT-NOTICE file.                                             *
 *                                                                            *
 * Removal or modification of this copyright notice is prohibited.            *
 *                                                                            *
 ******************************************************************************/
//
//  lp_swap.rs
//  marketmaker
//

use super::lp_network::P2PRequestResult;
use crate::mm2::lp_network::{broadcast_p2p_msg, Libp2pPeerId, P2PRequestError};
use bitcrypto::{dhash160, sha256};
use coins::{lp_coinfind, lp_coinfind_or_err, CoinFindError, MmCoinEnum, TradeFee, TransactionEnum};
use common::log::{debug, warn};
use common::now_sec;
use common::time_cache::DuplicateCache;
use common::{bits256, calc_total_pages,
             executor::{spawn_abortable, AbortOnDropHandle, SpawnFuture, Timer},
             log::{error, info},
             var, HttpStatusCode, PagingOptions, StatusCode};
use derive_more::Display;
use http::Response;
use mm2_core::mm_ctx::{from_ctx, MmArc};
use mm2_err_handle::prelude::*;
use mm2_libp2p::{decode_signed, encode_and_sign, pub_sub_topic, PeerId, TopicPrefix};
use mm2_number::{BigDecimal, BigRational, MmNumber, MmNumberMultiRepr};
use parking_lot::Mutex as PaMutex;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use serde::Serialize;
use serde_json::{self as json, Value as Json};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use uuid::Uuid;

#[cfg(feature = "custom-swap-locktime")]
use std::sync::atomic::{AtomicU64, Ordering};

#[path = "lp_swap/check_balance.rs"] mod check_balance;
#[path = "lp_swap/maker_swap.rs"] mod maker_swap;
#[path = "lp_swap/max_maker_vol_rpc.rs"] mod max_maker_vol_rpc;
#[path = "lp_swap/my_swaps_storage.rs"] mod my_swaps_storage;
#[path = "lp_swap/pubkey_banning.rs"] mod pubkey_banning;
#[path = "lp_swap/recreate_swap_data.rs"] mod recreate_swap_data;
#[path = "lp_swap/saved_swap.rs"] mod saved_swap;
#[path = "lp_swap/swap_lock.rs"] mod swap_lock;
#[path = "lp_swap/swap_watcher.rs"] pub(crate) mod swap_watcher;
#[path = "lp_swap/taker_swap.rs"] mod taker_swap;
#[path = "lp_swap/trade_preimage.rs"] mod trade_preimage;

#[cfg(target_arch = "wasm32")]
#[path = "lp_swap/swap_wasm_db.rs"]
mod swap_wasm_db;

pub use check_balance::{check_other_coin_balance_for_swap, CheckBalanceError, CheckBalanceResult};
use crypto::CryptoCtx;
use keys::KeyPair;
use maker_swap::MakerSwapEvent;
pub use maker_swap::{calc_max_maker_vol, check_balance_for_maker_swap, get_max_maker_vol, maker_swap_trade_preimage,
                     run_maker_swap, CoinVolumeInfo, MakerSavedEvent, MakerSavedSwap, MakerSwap,
                     MakerSwapStatusChanged, MakerTradePreimage, RunMakerSwapInput, MAKER_PAYMENT_SENT_LOG};
pub use max_maker_vol_rpc::max_maker_vol;
use my_swaps_storage::{MySwapsOps, MySwapsStorage};
use pubkey_banning::BanReason;
pub use pubkey_banning::{ban_pubkey_rpc, is_pubkey_banned, list_banned_pubkeys_rpc, unban_pubkeys_rpc};
pub use recreate_swap_data::recreate_swap_data;
pub use saved_swap::{SavedSwap, SavedSwapError, SavedSwapIo, SavedSwapResult};
pub use swap_watcher::{process_watcher_msg, watcher_topic, TakerSwapWatcherData, MAKER_PAYMENT_SPEND_FOUND_LOG,
                       MAKER_PAYMENT_SPEND_SENT_LOG, TAKER_PAYMENT_REFUND_SENT_LOG, TAKER_SWAP_ENTRY_TIMEOUT_SEC,
                       WATCHER_PREFIX};
use taker_swap::TakerSwapEvent;
pub use taker_swap::{calc_max_taker_vol, check_balance_for_taker_swap, max_taker_vol, max_taker_vol_from_available,
                     run_taker_swap, taker_swap_trade_preimage, RunTakerSwapInput, TakerSavedSwap, TakerSwap,
                     TakerSwapData, TakerSwapPreparedParams, TakerTradePreimage, WATCHER_MESSAGE_SENT_LOG};
pub use trade_preimage::trade_preimage_rpc;

pub const SWAP_PREFIX: TopicPrefix = "swap";

pub const TX_HELPER_PREFIX: TopicPrefix = "txhlp";

cfg_wasm32! {
    use mm2_db::indexed_db::{ConstructibleDb, DbLocked};
    use swap_wasm_db::{InitDbResult, SwapDb};

    pub type SwapDbLocked<'a> = DbLocked<'a, SwapDb>;
}

#[derive(Clone, Debug, Eq, Deserialize, PartialEq, Serialize)]
pub enum SwapMsg {
    Negotiation(NegotiationDataMsg),
    NegotiationReply(NegotiationDataMsg),
    Negotiated(bool),
    TakerFee(SwapTxDataMsg),
    MakerPayment(SwapTxDataMsg),
    TakerPayment(Vec<u8>),
}

#[derive(Debug, Default)]
pub struct SwapMsgStore {
    negotiation: Option<NegotiationDataMsg>,
    negotiation_reply: Option<NegotiationDataMsg>,
    negotiated: Option<bool>,
    taker_fee: Option<SwapTxDataMsg>,
    maker_payment: Option<SwapTxDataMsg>,
    taker_payment: Option<Vec<u8>>,
    accept_only_from: bits256,
}

impl SwapMsgStore {
    pub fn new(accept_only_from: bits256) -> Self {
        SwapMsgStore {
            accept_only_from,
            ..Default::default()
        }
    }
}

/// Returns key-pair for signing P2P messages and an optional `PeerId` if it should be used forcibly
/// instead of local peer ID.
///
/// # Panic
///
/// This function panics if `CryptoCtx` hasn't been initialized yet.
pub fn p2p_keypair_and_peer_id_to_broadcast(ctx: &MmArc, p2p_privkey: Option<&KeyPair>) -> (KeyPair, Option<PeerId>) {
    match p2p_privkey {
        Some(keypair) => (*keypair, Some(keypair.libp2p_peer_id())),
        None => {
            let crypto_ctx = CryptoCtx::from_ctx(ctx).expect("CryptoCtx must be initialized already");
            (*crypto_ctx.mm2_internal_key_pair(), None)
        },
    }
}

/// Returns private key for signing P2P messages and an optional `PeerId` if it should be used forcibly
/// instead of local peer ID.
///
/// # Panic
///
/// This function panics if `CryptoCtx` hasn't been initialized yet.
pub fn p2p_private_and_peer_id_to_broadcast(ctx: &MmArc, p2p_privkey: Option<&KeyPair>) -> ([u8; 32], Option<PeerId>) {
    match p2p_privkey {
        Some(keypair) => (keypair.private_bytes(), Some(keypair.libp2p_peer_id())),
        None => {
            let crypto_ctx = CryptoCtx::from_ctx(ctx).expect("CryptoCtx must be initialized already");
            (crypto_ctx.mm2_internal_privkey_secret().take(), None)
        },
    }
}

/// Spawns the loop that broadcasts message every `interval` seconds returning the AbortOnDropHandle
/// to stop it
pub fn broadcast_swap_msg_every<T: 'static + Serialize + Clone + Send>(
    ctx: MmArc,
    topic: String,
    msg: T,
    interval_sec: f64,
    p2p_privkey: Option<KeyPair>,
) -> AbortOnDropHandle {
    let fut = async move {
        loop {
            broadcast_swap_message(&ctx, topic.clone(), msg.clone(), &p2p_privkey);
            Timer::sleep(interval_sec).await;
        }
    };
    spawn_abortable(fut)
}

/// Spawns the loop that broadcasts message every `interval` seconds returning the AbortOnDropHandle
/// to stop it. This function waits for interval seconds first before starting the broadcast.
pub fn broadcast_swap_msg_every_delayed<T: 'static + Serialize + Clone + Send>(
    ctx: MmArc,
    topic: String,
    msg: T,
    interval_sec: f64,
    p2p_privkey: Option<KeyPair>,
) -> AbortOnDropHandle {
    let fut = async move {
        loop {
            Timer::sleep(interval_sec).await;
            broadcast_swap_message(&ctx, topic.clone(), msg.clone(), &p2p_privkey);
        }
    };
    spawn_abortable(fut)
}

/// Broadcast the swap message once
pub fn broadcast_swap_message<T: Serialize>(ctx: &MmArc, topic: String, msg: T, p2p_privkey: &Option<KeyPair>) {
    let (p2p_private, from) = p2p_private_and_peer_id_to_broadcast(ctx, p2p_privkey.as_ref());
    let encoded_msg = encode_and_sign(&msg, &p2p_private).unwrap();
    broadcast_p2p_msg(ctx, vec![topic], encoded_msg, from);
}

/// Broadcast the tx message once
pub fn broadcast_p2p_tx_msg(ctx: &MmArc, topic: String, msg: &TransactionEnum, p2p_privkey: &Option<KeyPair>) {
    if !msg.supports_tx_helper() {
        return;
    }

    let (p2p_private, from) = p2p_private_and_peer_id_to_broadcast(ctx, p2p_privkey.as_ref());
    let encoded_msg = encode_and_sign(&msg.tx_hex(), &p2p_private).unwrap();
    broadcast_p2p_msg(ctx, vec![topic], encoded_msg, from);
}

pub async fn process_swap_msg(ctx: MmArc, topic: &str, msg: &[u8]) -> P2PRequestResult<()> {
    let uuid = Uuid::from_str(topic).map_to_mm(|e| P2PRequestError::DecodeError(e.to_string()))?;

    let msg = match decode_signed::<SwapMsg>(msg) {
        Ok(m) => m,
        Err(swap_msg_err) => {
            #[cfg(not(target_arch = "wasm32"))]
            return match json::from_slice::<SwapStatus>(msg) {
                Ok(mut status) => {
                    status.data.fetch_and_set_usd_prices().await;
                    if let Err(e) = save_stats_swap(&ctx, &status.data).await {
                        error!("Error saving the swap {} status: {}", status.data.uuid(), e);
                    }
                    Ok(())
                },
                Err(swap_status_err) => {
                    let error = format!(
                        "Couldn't deserialize swap msg to either 'SwapMsg': {} or to 'SwapStatus': {}",
                        swap_msg_err, swap_status_err
                    );
                    MmError::err(P2PRequestError::DecodeError(error))
                },
            };

            #[cfg(target_arch = "wasm32")]
            return MmError::err(P2PRequestError::DecodeError(format!(
                "Couldn't deserialize 'SwapMsg': {}",
                swap_msg_err
            )));
        },
    };

    debug!("Processing swap msg {:?} for uuid {}", msg, uuid);
    let swap_ctx = SwapsContext::from_ctx(&ctx).unwrap();
    let mut msgs = swap_ctx.swap_msgs.lock().unwrap();
    if let Some(msg_store) = msgs.get_mut(&uuid) {
        if msg_store.accept_only_from.bytes == msg.2.unprefixed() {
            match msg.0 {
                SwapMsg::Negotiation(data) => msg_store.negotiation = Some(data),
                SwapMsg::NegotiationReply(data) => msg_store.negotiation_reply = Some(data),
                SwapMsg::Negotiated(negotiated) => msg_store.negotiated = Some(negotiated),
                SwapMsg::TakerFee(data) => msg_store.taker_fee = Some(data),
                SwapMsg::MakerPayment(data) => msg_store.maker_payment = Some(data),
                SwapMsg::TakerPayment(taker_payment) => msg_store.taker_payment = Some(taker_payment),
            }
        } else {
            warn!("Received message from unexpected sender for swap {}", uuid);
        }
    };

    Ok(())
}

pub fn swap_topic(uuid: &Uuid) -> String { pub_sub_topic(SWAP_PREFIX, &uuid.to_string()) }

/// Formats and returns a topic format for `txhlp`.
///
/// # Usage
/// ```ignore
/// let topic = tx_helper_topic("BTC");
/// // Returns topic format `txhlp/BTC` as String type.
/// ```
#[inline(always)]
pub fn tx_helper_topic(coin: &str) -> String { pub_sub_topic(TX_HELPER_PREFIX, coin) }

async fn recv_swap_msg<T>(
    ctx: MmArc,
    mut getter: impl FnMut(&mut SwapMsgStore) -> Option<T>,
    uuid: &Uuid,
    timeout: u64,
) -> Result<T, String> {
    let started = now_sec();
    let timeout = BASIC_COMM_TIMEOUT + timeout;
    let wait_until = started + timeout;
    loop {
        Timer::sleep(1.).await;
        let swap_ctx = SwapsContext::from_ctx(&ctx).unwrap();
        let mut msgs = swap_ctx.swap_msgs.lock().unwrap();
        if let Some(msg_store) = msgs.get_mut(uuid) {
            if let Some(msg) = getter(msg_store) {
                return Ok(msg);
            }
        }
        let now = now_sec();
        if now > wait_until {
            return ERR!("Timeout ({} > {})", now - started, timeout);
        }
    }
}

/// Includes the grace time we add to the "normal" timeouts
/// in order to give different and/or heavy communication channels a chance.
const BASIC_COMM_TIMEOUT: u64 = 90;

#[cfg(not(feature = "custom-swap-locktime"))]
/// Default atomic swap payment locktime, in seconds.
/// Maker sends payment with LOCKTIME * 2
/// Taker sends payment with LOCKTIME
const PAYMENT_LOCKTIME: u64 = 3600 * 2 + 300 * 2;

#[cfg(feature = "custom-swap-locktime")]
/// Default atomic swap payment locktime, in seconds.
/// Maker sends payment with LOCKTIME * 2
/// Taker sends payment with LOCKTIME
pub(crate) static PAYMENT_LOCKTIME: AtomicU64 = AtomicU64::new(super::CUSTOM_PAYMENT_LOCKTIME_DEFAULT);

#[inline]
/// Returns `PAYMENT_LOCKTIME`
pub fn get_payment_locktime() -> u64 {
    #[cfg(not(feature = "custom-swap-locktime"))]
    return PAYMENT_LOCKTIME;
    #[cfg(feature = "custom-swap-locktime")]
    PAYMENT_LOCKTIME.load(Ordering::Relaxed)
}

#[inline]
pub fn taker_payment_spend_duration(locktime: u64) -> u64 { (locktime * 4) / 5 }

#[inline]
pub fn taker_payment_spend_deadline(swap_started_at: u64, locktime: u64) -> u64 {
    swap_started_at + taker_payment_spend_duration(locktime)
}

#[inline]
pub fn wait_for_maker_payment_conf_duration(locktime: u64) -> u64 { (locktime * 2) / 5 }

#[inline]
pub fn wait_for_maker_payment_conf_until(swap_started_at: u64, locktime: u64) -> u64 {
    swap_started_at + wait_for_maker_payment_conf_duration(locktime)
}

const _SWAP_DEFAULT_NUM_CONFIRMS: u32 = 1;
const _SWAP_DEFAULT_MAX_CONFIRMS: u32 = 6;
/// MM2 checks that swap payment is confirmed every WAIT_CONFIRM_INTERVAL seconds
const WAIT_CONFIRM_INTERVAL_SEC: u64 = 15;

#[derive(Debug, PartialEq, Serialize)]
pub enum RecoveredSwapAction {
    RefundedMyPayment,
    SpentOtherPayment,
}

#[derive(Debug, PartialEq)]
pub struct RecoveredSwap {
    action: RecoveredSwapAction,
    coin: String,
    transaction: TransactionEnum,
}

/// Represents the amount of a coin locked by ongoing swap
#[derive(Debug)]
pub struct LockedAmount {
    coin: String,
    amount: MmNumber,
    trade_fee: Option<TradeFee>,
}

pub trait AtomicSwap: Send + Sync {
    fn locked_amount(&self) -> Vec<LockedAmount>;

    fn uuid(&self) -> &Uuid;

    fn maker_coin(&self) -> &str;

    fn taker_coin(&self) -> &str;

    fn unique_swap_data(&self) -> Vec<u8>;
}

#[derive(Serialize)]
#[serde(tag = "type", content = "event")]
pub enum SwapEvent {
    Maker(MakerSwapEvent),
    Taker(TakerSwapEvent),
}

impl From<MakerSwapEvent> for SwapEvent {
    fn from(maker_event: MakerSwapEvent) -> Self { SwapEvent::Maker(maker_event) }
}

impl From<TakerSwapEvent> for SwapEvent {
    fn from(taker_event: TakerSwapEvent) -> Self { SwapEvent::Taker(taker_event) }
}

struct SwapsContext {
    running_swaps: Mutex<Vec<Weak<dyn AtomicSwap>>>,
    banned_pubkeys: Mutex<HashMap<H256Json, BanReason>>,
    swap_msgs: Mutex<HashMap<Uuid, SwapMsgStore>>,
    taker_swap_watchers: PaMutex<DuplicateCache<Vec<u8>>>,
    #[cfg(target_arch = "wasm32")]
    swap_db: ConstructibleDb<SwapDb>,
}

impl SwapsContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    fn from_ctx(ctx: &MmArc) -> Result<Arc<SwapsContext>, String> {
        Ok(try_s!(from_ctx(&ctx.swaps_ctx, move || {
            Ok(SwapsContext {
                running_swaps: Mutex::new(vec![]),
                banned_pubkeys: Mutex::new(HashMap::new()),
                swap_msgs: Mutex::new(HashMap::new()),
                taker_swap_watchers: PaMutex::new(DuplicateCache::new(Duration::from_secs(
                    TAKER_SWAP_ENTRY_TIMEOUT_SEC,
                ))),
                #[cfg(target_arch = "wasm32")]
                swap_db: ConstructibleDb::new(ctx),
            })
        })))
    }

    pub fn init_msg_store(&self, uuid: Uuid, accept_only_from: bits256) {
        let store = SwapMsgStore::new(accept_only_from);
        self.swap_msgs.lock().unwrap().insert(uuid, store);
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn swap_db(&self) -> InitDbResult<SwapDbLocked<'_>> { self.swap_db.get_or_initialize().await }
}

#[derive(Debug, Deserialize)]
pub struct GetLockedAmountReq {
    coin: String,
}

#[derive(Serialize)]
pub struct GetLockedAmountResp {
    coin: String,
    locked_amount: MmNumberMultiRepr,
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetLockedAmountRpcError {
    #[display(fmt = "No such coin: {}", coin)]
    NoSuchCoin { coin: String },
}

impl HttpStatusCode for GetLockedAmountRpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetLockedAmountRpcError::NoSuchCoin { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for GetLockedAmountRpcError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => GetLockedAmountRpcError::NoSuchCoin { coin },
        }
    }
}

pub async fn get_locked_amount_rpc(
    ctx: MmArc,
    req: GetLockedAmountReq,
) -> Result<GetLockedAmountResp, MmError<GetLockedAmountRpcError>> {
    lp_coinfind_or_err(&ctx, &req.coin).await?;
    let locked_amount = get_locked_amount(&ctx, &req.coin);

    Ok(GetLockedAmountResp {
        coin: req.coin,
        locked_amount: locked_amount.into(),
    })
}

/// Get total amount of selected coin locked by all currently ongoing swaps
pub fn get_locked_amount(ctx: &MmArc, coin: &str) -> MmNumber {
    let swap_ctx = SwapsContext::from_ctx(ctx).unwrap();
    let swap_lock = swap_ctx.running_swaps.lock().unwrap();

    swap_lock
        .iter()
        .filter_map(|swap| swap.upgrade())
        .flat_map(|swap| swap.locked_amount())
        .fold(MmNumber::from(0), |mut total_amount, locked| {
            if locked.coin == coin {
                total_amount += locked.amount;
            }
            if let Some(trade_fee) = locked.trade_fee {
                if trade_fee.coin == coin && !trade_fee.paid_from_trading_vol {
                    total_amount += trade_fee.amount;
                }
            }
            total_amount
        })
}

/// Get number of currently running swaps
pub fn running_swaps_num(ctx: &MmArc) -> u64 {
    let swap_ctx = SwapsContext::from_ctx(ctx).unwrap();
    let swaps = swap_ctx.running_swaps.lock().unwrap();
    swaps.iter().fold(0, |total, swap| match swap.upgrade() {
        Some(_) => total + 1,
        None => total,
    })
}

/// Get total amount of selected coin locked by all currently ongoing swaps except the one with selected uuid
fn get_locked_amount_by_other_swaps(ctx: &MmArc, except_uuid: &Uuid, coin: &str) -> MmNumber {
    let swap_ctx = SwapsContext::from_ctx(ctx).unwrap();
    let swap_lock = swap_ctx.running_swaps.lock().unwrap();

    swap_lock
        .iter()
        .filter_map(|swap| swap.upgrade())
        .filter(|swap| swap.uuid() != except_uuid)
        .flat_map(|swap| swap.locked_amount())
        .fold(MmNumber::from(0), |mut total_amount, locked| {
            if locked.coin == coin {
                total_amount += locked.amount;
            }
            if let Some(trade_fee) = locked.trade_fee {
                if trade_fee.coin == coin && !trade_fee.paid_from_trading_vol {
                    total_amount += trade_fee.amount;
                }
            }
            total_amount
        })
}

pub fn active_swaps_using_coins(ctx: &MmArc, coins: &HashSet<String>) -> Result<Vec<Uuid>, String> {
    let swap_ctx = try_s!(SwapsContext::from_ctx(ctx));
    let swaps = try_s!(swap_ctx.running_swaps.lock());
    let mut uuids = vec![];
    for swap in swaps.iter() {
        if let Some(swap) = swap.upgrade() {
            if coins.contains(&swap.maker_coin().to_string()) || coins.contains(&swap.taker_coin().to_string()) {
                uuids.push(*swap.uuid())
            }
        }
    }
    Ok(uuids)
}

pub fn active_swaps(ctx: &MmArc) -> Result<Vec<Uuid>, String> {
    let swap_ctx = try_s!(SwapsContext::from_ctx(ctx));
    let swaps = try_s!(swap_ctx.running_swaps.lock());
    let mut uuids = vec![];
    for swap in swaps.iter() {
        if let Some(swap) = swap.upgrade() {
            uuids.push(*swap.uuid())
        }
    }
    Ok(uuids)
}

#[derive(Clone, Copy, Debug)]
pub struct SwapConfirmationsSettings {
    pub maker_coin_confs: u64,
    pub maker_coin_nota: bool,
    pub taker_coin_confs: u64,
    pub taker_coin_nota: bool,
}

impl SwapConfirmationsSettings {
    pub fn requires_notarization(&self) -> bool { self.maker_coin_nota || self.taker_coin_nota }
}

fn coin_with_4x_locktime(ticker: &str) -> bool { matches!(ticker, "BCH" | "BTG" | "SBTC") }

#[derive(Debug)]
pub enum AtomicLocktimeVersion {
    V1,
    V2 {
        my_conf_settings: SwapConfirmationsSettings,
        other_conf_settings: SwapConfirmationsSettings,
    },
}

pub fn lp_atomic_locktime_v1(maker_coin: &str, taker_coin: &str) -> u64 {
    if maker_coin == "BTC" || taker_coin == "BTC" {
        get_payment_locktime() * 10
    } else if coin_with_4x_locktime(maker_coin) || coin_with_4x_locktime(taker_coin) {
        get_payment_locktime() * 4
    } else {
        get_payment_locktime()
    }
}

pub fn lp_atomic_locktime_v2(
    maker_coin: &str,
    taker_coin: &str,
    my_conf_settings: &SwapConfirmationsSettings,
    other_conf_settings: &SwapConfirmationsSettings,
) -> u64 {
    if taker_coin.contains("-lightning") {
        // A good value for lightning taker locktime is about 24 hours to find a good 3 hop or less path for the payment
        get_payment_locktime() * 12
    } else if maker_coin == "BTC"
        || taker_coin == "BTC"
        || coin_with_4x_locktime(maker_coin)
        || coin_with_4x_locktime(taker_coin)
        || my_conf_settings.requires_notarization()
        || other_conf_settings.requires_notarization()
    {
        get_payment_locktime() * 4
    } else {
        get_payment_locktime()
    }
}

/// Some coins are "slow" (block time is high - e.g. BTC average block time is ~10 minutes).
/// https://bitinfocharts.com/comparison/bitcoin-confirmationtime.html
/// We need to increase payment locktime accordingly when at least 1 side of swap uses "slow" coin.
pub fn lp_atomic_locktime(maker_coin: &str, taker_coin: &str, version: AtomicLocktimeVersion) -> u64 {
    match version {
        AtomicLocktimeVersion::V1 => lp_atomic_locktime_v1(maker_coin, taker_coin),
        AtomicLocktimeVersion::V2 {
            my_conf_settings,
            other_conf_settings,
        } => lp_atomic_locktime_v2(maker_coin, taker_coin, &my_conf_settings, &other_conf_settings),
    }
}

pub fn dex_fee_threshold(min_tx_amount: MmNumber) -> MmNumber {
    // Todo: This should be reduced for lightning swaps.
    // 0.0001
    let min_fee = MmNumber::from((1, 10000));
    if min_fee < min_tx_amount {
        min_tx_amount
    } else {
        min_fee
    }
}

fn dex_fee_rate(base: &str, rel: &str) -> MmNumber {
    let fee_discount_tickers: &[&str] = if var("MYCOIN_FEE_DISCOUNT").is_ok() {
        &["KMD", "MYCOIN"]
    } else {
        &["KMD"]
    };
    if fee_discount_tickers.contains(&base) || fee_discount_tickers.contains(&rel) {
        // 1/777 - 10%
        BigRational::new(9.into(), 7770.into()).into()
    } else {
        BigRational::new(1.into(), 777.into()).into()
    }
}

pub fn dex_fee_amount(base: &str, rel: &str, trade_amount: &MmNumber, dex_fee_threshold: &MmNumber) -> MmNumber {
    let rate = dex_fee_rate(base, rel);
    let fee_amount = trade_amount * &rate;
    if &fee_amount < dex_fee_threshold {
        dex_fee_threshold.clone()
    } else {
        fee_amount
    }
}

pub fn dex_fee_amount_from_taker_coin(taker_coin: &MmCoinEnum, maker_coin: &str, trade_amount: &MmNumber) -> MmNumber {
    let min_tx_amount = MmNumber::from(taker_coin.min_tx_amount());
    let dex_fee_threshold = dex_fee_threshold(min_tx_amount);
    dex_fee_amount(taker_coin.ticker(), maker_coin, trade_amount, &dex_fee_threshold)
}

#[derive(Clone, Debug, Eq, Deserialize, PartialEq, Serialize)]
pub struct NegotiationDataV1 {
    started_at: u64,
    payment_locktime: u64,
    secret_hash: [u8; 20],
    persistent_pubkey: Vec<u8>,
}

#[derive(Clone, Debug, Eq, Deserialize, PartialEq, Serialize)]
pub struct NegotiationDataV2 {
    started_at: u64,
    payment_locktime: u64,
    secret_hash: Vec<u8>,
    persistent_pubkey: Vec<u8>,
    maker_coin_swap_contract: Vec<u8>,
    taker_coin_swap_contract: Vec<u8>,
}

#[derive(Clone, Debug, Eq, Deserialize, PartialEq, Serialize)]
pub struct NegotiationDataV3 {
    started_at: u64,
    payment_locktime: u64,
    secret_hash: Vec<u8>,
    maker_coin_swap_contract: Vec<u8>,
    taker_coin_swap_contract: Vec<u8>,
    maker_coin_htlc_pub: Vec<u8>,
    taker_coin_htlc_pub: Vec<u8>,
}

#[derive(Clone, Debug, Eq, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum NegotiationDataMsg {
    V1(NegotiationDataV1),
    V2(NegotiationDataV2),
    V3(NegotiationDataV3),
}

impl NegotiationDataMsg {
    pub fn started_at(&self) -> u64 {
        match self {
            NegotiationDataMsg::V1(v1) => v1.started_at,
            NegotiationDataMsg::V2(v2) => v2.started_at,
            NegotiationDataMsg::V3(v3) => v3.started_at,
        }
    }

    pub fn payment_locktime(&self) -> u64 {
        match self {
            NegotiationDataMsg::V1(v1) => v1.payment_locktime,
            NegotiationDataMsg::V2(v2) => v2.payment_locktime,
            NegotiationDataMsg::V3(v3) => v3.payment_locktime,
        }
    }

    pub fn secret_hash(&self) -> &[u8] {
        match self {
            NegotiationDataMsg::V1(v1) => &v1.secret_hash,
            NegotiationDataMsg::V2(v2) => &v2.secret_hash,
            NegotiationDataMsg::V3(v3) => &v3.secret_hash,
        }
    }

    pub fn maker_coin_htlc_pub(&self) -> &[u8] {
        match self {
            NegotiationDataMsg::V1(v1) => &v1.persistent_pubkey,
            NegotiationDataMsg::V2(v2) => &v2.persistent_pubkey,
            NegotiationDataMsg::V3(v3) => &v3.maker_coin_htlc_pub,
        }
    }

    pub fn taker_coin_htlc_pub(&self) -> &[u8] {
        match self {
            NegotiationDataMsg::V1(v1) => &v1.persistent_pubkey,
            NegotiationDataMsg::V2(v2) => &v2.persistent_pubkey,
            NegotiationDataMsg::V3(v3) => &v3.taker_coin_htlc_pub,
        }
    }

    pub fn maker_coin_swap_contract(&self) -> Option<&[u8]> {
        match self {
            NegotiationDataMsg::V1(_) => None,
            NegotiationDataMsg::V2(v2) => Some(&v2.maker_coin_swap_contract),
            NegotiationDataMsg::V3(v3) => Some(&v3.maker_coin_swap_contract),
        }
    }

    pub fn taker_coin_swap_contract(&self) -> Option<&[u8]> {
        match self {
            NegotiationDataMsg::V1(_) => None,
            NegotiationDataMsg::V2(v2) => Some(&v2.taker_coin_swap_contract),
            NegotiationDataMsg::V3(v3) => Some(&v3.taker_coin_swap_contract),
        }
    }
}

#[derive(Clone, Debug, Eq, Deserialize, PartialEq, Serialize)]
pub struct PaymentWithInstructions {
    data: Vec<u8>,
    // Next step instructions for the other side whether taker or maker.
    // An example for this is a maker/taker sending the taker/maker a lightning invoice to be payed.
    next_step_instructions: Vec<u8>,
}

#[derive(Clone, Debug, Eq, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SwapTxDataMsg {
    Regular(Vec<u8>),
    WithInstructions(PaymentWithInstructions),
}

impl SwapTxDataMsg {
    #[inline]
    pub fn data(&self) -> &[u8] {
        match self {
            SwapTxDataMsg::Regular(data) => data,
            SwapTxDataMsg::WithInstructions(p) => &p.data,
        }
    }

    #[inline]
    pub fn instructions(&self) -> Option<&[u8]> {
        match self {
            SwapTxDataMsg::Regular(_) => None,
            SwapTxDataMsg::WithInstructions(p) => Some(&p.next_step_instructions),
        }
    }

    #[inline]
    pub fn new(data: Vec<u8>, instructions: Option<Vec<u8>>) -> Self {
        match instructions {
            Some(next_step_instructions) => SwapTxDataMsg::WithInstructions(PaymentWithInstructions {
                data,
                next_step_instructions,
            }),
            None => SwapTxDataMsg::Regular(data),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TransactionIdentifier {
    /// Raw bytes of signed transaction in hexadecimal string, this should be sent as is to send_raw_transaction RPC to broadcast the transaction.
    /// Some payments like lightning payments don't have a tx_hex, for such payments tx_hex will be equal to tx_hash.
    tx_hex: BytesJson,
    /// Transaction hash in hexadecimal format
    tx_hash: BytesJson,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn my_swaps_dir(ctx: &MmArc) -> PathBuf { ctx.dbdir().join("SWAPS").join("MY") }

#[cfg(not(target_arch = "wasm32"))]
pub fn my_swap_file_path(ctx: &MmArc, uuid: &Uuid) -> PathBuf { my_swaps_dir(ctx).join(format!("{}.json", uuid)) }

pub async fn insert_new_swap_to_db(
    ctx: MmArc,
    my_coin: &str,
    other_coin: &str,
    uuid: Uuid,
    started_at: u64,
) -> Result<(), String> {
    MySwapsStorage::new(ctx)
        .save_new_swap(my_coin, other_coin, uuid, started_at)
        .await
        .map_err(|e| ERRL!("{}", e))
}

#[cfg(not(target_arch = "wasm32"))]
fn add_swap_to_db_index(ctx: &MmArc, swap: &SavedSwap) {
    if let Some(conn) = ctx.sqlite_conn_opt() {
        crate::mm2::database::stats_swaps::add_swap_to_index(&conn, swap)
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn save_stats_swap(ctx: &MmArc, swap: &SavedSwap) -> Result<(), String> {
    try_s!(swap.save_to_stats_db(ctx).await);
    add_swap_to_db_index(ctx, swap);
    Ok(())
}

/// The helper structure that makes easier to parse the response for GUI devs
/// They won't have to parse the events themselves handling possible errors, index out of bounds etc.
#[derive(Debug, Serialize, Deserialize)]
pub struct MySwapInfo {
    pub my_coin: String,
    pub other_coin: String,
    pub my_amount: BigDecimal,
    pub other_amount: BigDecimal,
    pub started_at: u64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct SavedTradeFee {
    coin: String,
    amount: BigDecimal,
    #[serde(default)]
    paid_from_trading_vol: bool,
}

impl From<SavedTradeFee> for TradeFee {
    fn from(orig: SavedTradeFee) -> Self {
        // used to calculate locked amount so paid_from_trading_vol doesn't matter here
        TradeFee {
            coin: orig.coin,
            amount: orig.amount.into(),
            paid_from_trading_vol: orig.paid_from_trading_vol,
        }
    }
}

impl From<TradeFee> for SavedTradeFee {
    fn from(orig: TradeFee) -> Self {
        SavedTradeFee {
            coin: orig.coin,
            amount: orig.amount.into(),
            paid_from_trading_vol: orig.paid_from_trading_vol,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SwapError {
    error: String,
}

impl From<String> for SwapError {
    fn from(error: String) -> Self { SwapError { error } }
}

impl From<&str> for SwapError {
    fn from(e: &str) -> Self { SwapError { error: e.to_owned() } }
}

#[derive(Serialize)]
struct MySwapStatusResponse {
    #[serde(flatten)]
    swap: SavedSwap,
    my_info: Option<MySwapInfo>,
    recoverable: bool,
}

impl From<SavedSwap> for MySwapStatusResponse {
    fn from(mut swap: SavedSwap) -> MySwapStatusResponse {
        swap.hide_secrets();
        MySwapStatusResponse {
            my_info: swap.get_my_info(),
            recoverable: swap.is_recoverable(),
            swap,
        }
    }
}

/// Returns the status of swap performed on `my` node
pub async fn my_swap_status(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let uuid: Uuid = try_s!(json::from_value(req["params"]["uuid"].clone()));
    let status = match SavedSwap::load_my_swap_from_db(&ctx, uuid).await {
        Ok(Some(status)) => status,
        Ok(None) => return Err("swap data is not found".to_owned()),
        Err(e) => return ERR!("{}", e),
    };

    let res_js = json!({ "result": MySwapStatusResponse::from(status) });
    let res = try_s!(json::to_vec(&res_js));
    Ok(try_s!(Response::builder().body(res)))
}

#[cfg(target_arch = "wasm32")]
pub async fn stats_swap_status(_ctx: MmArc, _req: Json) -> Result<Response<Vec<u8>>, String> {
    ERR!("'stats_swap_status' is only supported in native mode")
}

/// Returns the status of requested swap, typically performed by other nodes and saved by `save_stats_swap_status`
#[cfg(not(target_arch = "wasm32"))]
pub async fn stats_swap_status(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let uuid: Uuid = try_s!(json::from_value(req["params"]["uuid"].clone()));

    let maker_status = try_s!(SavedSwap::load_from_maker_stats_db(&ctx, uuid).await);
    let taker_status = try_s!(SavedSwap::load_from_taker_stats_db(&ctx, uuid).await);

    if maker_status.is_none() && taker_status.is_none() {
        return ERR!("swap data is not found");
    }

    let res_js = json!({
        "result": {
            "maker": maker_status,
            "taker": taker_status,
        }
    });
    let res = try_s!(json::to_vec(&res_js));
    Ok(try_s!(Response::builder().body(res)))
}

#[derive(Debug, Deserialize, Serialize)]
struct SwapStatus {
    method: String,
    data: SavedSwap,
}

/// Broadcasts `my` swap status to P2P network
async fn broadcast_my_swap_status(ctx: &MmArc, uuid: Uuid) -> Result<(), String> {
    let mut status = match try_s!(SavedSwap::load_my_swap_from_db(ctx, uuid).await) {
        Some(status) => status,
        None => return ERR!("swap data is not found"),
    };
    status.hide_secrets();

    #[cfg(not(target_arch = "wasm32"))]
    try_s!(save_stats_swap(ctx, &status).await);

    let status = SwapStatus {
        method: "swapstatus".into(),
        data: status,
    };
    let msg = json::to_vec(&status).expect("Swap status ser should never fail");
    broadcast_p2p_msg(ctx, vec![swap_topic(&uuid)], msg, None);
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct MySwapsFilter {
    pub my_coin: Option<String>,
    pub other_coin: Option<String>,
    pub from_timestamp: Option<u64>,
    pub to_timestamp: Option<u64>,
}

// TODO: Should return the result from SQL like in order history. So it can be clear the exact started_at time
// and the coins if they are not included in the filter request
/// Returns *all* uuids of swaps, which match the selected filter.
pub async fn all_swaps_uuids_by_filter(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let filter: MySwapsFilter = try_s!(json::from_value(req));
    let db_result = try_s!(
        MySwapsStorage::new(ctx)
            .my_recent_swaps_with_filters(&filter, None)
            .await
    );

    let res_js = json!({
        "result": {
            "uuids": db_result.uuids,
            "my_coin": filter.my_coin,
            "other_coin": filter.other_coin,
            "from_timestamp": filter.from_timestamp,
            "to_timestamp": filter.to_timestamp,
            "found_records": db_result.uuids.len(),
        },
    });
    let res = try_s!(json::to_vec(&res_js));
    Ok(try_s!(Response::builder().body(res)))
}

#[derive(Debug, Deserialize)]
pub struct MyRecentSwapsReq {
    #[serde(flatten)]
    pub paging_options: PagingOptions,
    #[serde(flatten)]
    pub filter: MySwapsFilter,
}

#[derive(Debug, Default, PartialEq)]
pub struct MyRecentSwapsUuids {
    /// UUIDs of swaps matching the query
    pub uuids: Vec<Uuid>,
    /// Total count of swaps matching the query
    pub total_count: usize,
    /// The number of skipped UUIDs
    pub skipped: usize,
}

#[derive(Debug, Display, Deserialize, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum LatestSwapsErr {
    #[display(fmt = "No such swap with the uuid '{}'", _0)]
    UUIDNotPresentInDb(Uuid),
    UnableToLoadSavedSwaps(SavedSwapError),
    #[display(fmt = "Unable to query swaps storage")]
    UnableToQuerySwapStorage,
}

pub async fn latest_swaps_for_pair(
    ctx: MmArc,
    my_coin: String,
    other_coin: String,
    limit: usize,
) -> Result<Vec<SavedSwap>, MmError<LatestSwapsErr>> {
    let filter = MySwapsFilter {
        my_coin: Some(my_coin),
        other_coin: Some(other_coin),
        from_timestamp: None,
        to_timestamp: None,
    };

    let paging_options = PagingOptions {
        limit,
        page_number: NonZeroUsize::new(1).expect("1 > 0"),
        from_uuid: None,
    };

    let db_result = match MySwapsStorage::new(ctx.clone())
        .my_recent_swaps_with_filters(&filter, Some(&paging_options))
        .await
    {
        Ok(x) => x,
        Err(_) => return Err(MmError::new(LatestSwapsErr::UnableToQuerySwapStorage)),
    };

    let mut swaps = Vec::with_capacity(db_result.uuids.len());
    for uuid in db_result.uuids.iter() {
        let swap = match SavedSwap::load_my_swap_from_db(&ctx, *uuid).await {
            Ok(Some(swap)) => swap,
            Ok(None) => {
                error!("No such swap with the uuid '{}'", uuid);
                continue;
            },
            Err(e) => return Err(MmError::new(LatestSwapsErr::UnableToLoadSavedSwaps(e.into_inner()))),
        };
        swaps.push(swap);
    }

    Ok(swaps)
}

/// Returns the data of recent swaps of `my` node.
pub async fn my_recent_swaps_rpc(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: MyRecentSwapsReq = try_s!(json::from_value(req));
    let db_result = try_s!(
        MySwapsStorage::new(ctx.clone())
            .my_recent_swaps_with_filters(&req.filter, Some(&req.paging_options))
            .await
    );

    // iterate over uuids trying to parse the corresponding files content and add to result vector
    let mut swaps = Vec::with_capacity(db_result.uuids.len());
    for uuid in db_result.uuids.iter() {
        match SavedSwap::load_my_swap_from_db(&ctx, *uuid).await {
            Ok(Some(swap)) => {
                let swap_json = json::to_value(MySwapStatusResponse::from(swap)).unwrap();
                swaps.push(swap_json)
            },
            Ok(None) => warn!("No such swap with the uuid '{}'", uuid),
            Err(e) => error!("Error loading a swap with the uuid '{}': {}", uuid, e),
        }
    }

    let res_js = json!({
        "result": {
            "swaps": swaps,
            "from_uuid": req.paging_options.from_uuid,
            "skipped": db_result.skipped,
            "limit": req.paging_options.limit,
            "total": db_result.total_count,
            "page_number": req.paging_options.page_number,
            "total_pages": calc_total_pages(db_result.total_count, req.paging_options.limit),
            "found_records": db_result.uuids.len(),
        },
    });
    let res = try_s!(json::to_vec(&res_js));
    Ok(try_s!(Response::builder().body(res)))
}

/// Find out the swaps that need to be kick-started, continue from the point where swap was interrupted
/// Return the tickers of coins that must be enabled for swaps to continue
pub async fn swap_kick_starts(ctx: MmArc) -> Result<HashSet<String>, String> {
    let mut coins = HashSet::new();
    let swaps = try_s!(SavedSwap::load_all_my_swaps_from_db(&ctx).await);
    for swap in swaps {
        if swap.is_finished() {
            continue;
        }

        info!("Kick starting the swap {}", swap.uuid());
        let maker_coin_ticker = match swap.maker_coin_ticker() {
            Ok(t) => t,
            Err(e) => {
                error!("Error {} getting maker coin of swap: {}", e, swap.uuid());
                continue;
            },
        };
        let taker_coin_ticker = match swap.taker_coin_ticker() {
            Ok(t) => t,
            Err(e) => {
                error!("Error {} getting taker coin of swap {}", e, swap.uuid());
                continue;
            },
        };
        coins.insert(maker_coin_ticker.clone());
        coins.insert(taker_coin_ticker.clone());

        let fut = kickstart_thread_handler(ctx.clone(), swap, maker_coin_ticker, taker_coin_ticker);
        ctx.spawner().spawn(fut);
    }
    Ok(coins)
}

async fn kickstart_thread_handler(ctx: MmArc, swap: SavedSwap, maker_coin_ticker: String, taker_coin_ticker: String) {
    let taker_coin = loop {
        match lp_coinfind(&ctx, &taker_coin_ticker).await {
            Ok(Some(c)) => break c,
            Ok(None) => {
                info!(
                    "Can't kickstart the swap {} until the coin {} is activated",
                    swap.uuid(),
                    taker_coin_ticker
                );
                Timer::sleep(5.).await;
            },
            Err(e) => {
                error!("Error {} on {} find attempt", e, taker_coin_ticker);
                return;
            },
        };
    };

    let maker_coin = loop {
        match lp_coinfind(&ctx, &maker_coin_ticker).await {
            Ok(Some(c)) => break c,
            Ok(None) => {
                info!(
                    "Can't kickstart the swap {} until the coin {} is activated",
                    swap.uuid(),
                    maker_coin_ticker
                );
                Timer::sleep(5.).await;
            },
            Err(e) => {
                error!("Error {} on {} find attempt", e, maker_coin_ticker);
                return;
            },
        };
    };
    match swap {
        SavedSwap::Maker(saved_swap) => {
            run_maker_swap(
                RunMakerSwapInput::KickStart {
                    maker_coin,
                    taker_coin,
                    swap_uuid: saved_swap.uuid,
                },
                ctx,
            )
            .await;
        },
        SavedSwap::Taker(saved_swap) => {
            run_taker_swap(
                RunTakerSwapInput::KickStart {
                    maker_coin,
                    taker_coin,
                    swap_uuid: saved_swap.uuid,
                },
                ctx,
            )
            .await;
        },
    }
}

pub async fn coins_needed_for_kick_start(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    let res = try_s!(json::to_vec(&json!({
        "result": *(try_s!(ctx.coins_needed_for_kick_start.lock()))
    })));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn recover_funds_of_swap(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let uuid: Uuid = try_s!(json::from_value(req["params"]["uuid"].clone()));
    let swap = match SavedSwap::load_my_swap_from_db(&ctx, uuid).await {
        Ok(Some(swap)) => swap,
        Ok(None) => return ERR!("swap data is not found"),
        Err(e) => return ERR!("{}", e),
    };

    let recover_data = try_s!(swap.recover_funds(ctx).await);
    let res = try_s!(json::to_vec(&json!({
        "result": {
            "action": recover_data.action,
            "coin": recover_data.coin,
            "tx_hash": recover_data.transaction.tx_hash(),
            "tx_hex": BytesJson::from(recover_data.transaction.tx_hex()),
        }
    })));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn import_swaps(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let swaps: Vec<SavedSwap> = try_s!(json::from_value(req["swaps"].clone()));
    let mut imported = vec![];
    let mut skipped = HashMap::new();
    for swap in swaps {
        match swap.save_to_db(&ctx).await {
            Ok(_) => {
                if let Some(info) = swap.get_my_info() {
                    if let Err(e) = insert_new_swap_to_db(
                        ctx.clone(),
                        &info.my_coin,
                        &info.other_coin,
                        *swap.uuid(),
                        info.started_at,
                    )
                    .await
                    {
                        error!("Error {} on new swap insertion", e);
                    }
                }
                imported.push(swap.uuid().to_owned());
            },
            Err(e) => {
                skipped.insert(swap.uuid().to_owned(), ERRL!("{}", e));
            },
        }
    }
    let res = try_s!(json::to_vec(&json!({
        "result": {
            "imported": imported,
            "skipped": skipped,
        }
    })));
    Ok(try_s!(Response::builder().body(res)))
}

#[derive(Deserialize)]
struct ActiveSwapsReq {
    #[serde(default)]
    include_status: bool,
}

#[derive(Serialize)]
struct ActiveSwapsRes {
    uuids: Vec<Uuid>,
    statuses: Option<HashMap<Uuid, SavedSwap>>,
}

pub async fn active_swaps_rpc(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: ActiveSwapsReq = try_s!(json::from_value(req));
    let uuids = try_s!(active_swaps(&ctx));
    let statuses = if req.include_status {
        let mut map = HashMap::new();
        for uuid in uuids.iter() {
            let status = match SavedSwap::load_my_swap_from_db(&ctx, *uuid).await {
                Ok(Some(status)) => status,
                Ok(None) => continue,
                Err(e) => {
                    error!("Error on loading_from_db: {}", e);
                    continue;
                },
            };
            map.insert(*uuid, status);
        }
        Some(map)
    } else {
        None
    };
    let result = ActiveSwapsRes { uuids, statuses };
    let res = try_s!(json::to_vec(&result));
    Ok(try_s!(Response::builder().body(res)))
}

enum SecretHashAlgo {
    /// ripemd160(sha256(secret))
    DHASH160,
    /// sha256(secret)
    SHA256,
}

impl Default for SecretHashAlgo {
    fn default() -> Self { SecretHashAlgo::DHASH160 }
}

impl SecretHashAlgo {
    fn hash_secret(&self, secret: &[u8]) -> Vec<u8> {
        match self {
            SecretHashAlgo::DHASH160 => dhash160(secret).take().into(),
            SecretHashAlgo::SHA256 => sha256(secret).take().into(),
        }
    }
}

// Todo: Maybe add a secret_hash_algo method to the SwapOps trait instead
#[cfg(not(target_arch = "wasm32"))]
fn detect_secret_hash_algo(maker_coin: &MmCoinEnum, taker_coin: &MmCoinEnum) -> SecretHashAlgo {
    match (maker_coin, taker_coin) {
        (MmCoinEnum::Tendermint(_) | MmCoinEnum::TendermintToken(_) | MmCoinEnum::LightningCoin(_), _) => {
            SecretHashAlgo::SHA256
        },
        // If taker is lightning coin the SHA256 of the secret will be sent as part of the maker signed invoice
        (_, MmCoinEnum::Tendermint(_) | MmCoinEnum::TendermintToken(_)) => SecretHashAlgo::SHA256,
        (_, _) => SecretHashAlgo::DHASH160,
    }
}

#[cfg(target_arch = "wasm32")]
fn detect_secret_hash_algo(maker_coin: &MmCoinEnum, taker_coin: &MmCoinEnum) -> SecretHashAlgo {
    match (maker_coin, taker_coin) {
        (MmCoinEnum::Tendermint(_) | MmCoinEnum::TendermintToken(_), _) => SecretHashAlgo::SHA256,
        (_, MmCoinEnum::Tendermint(_) | MmCoinEnum::TendermintToken(_)) => SecretHashAlgo::SHA256,
        (_, _) => SecretHashAlgo::DHASH160,
    }
}

pub struct SwapPubkeys {
    pub maker: String,
    pub taker: String,
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod lp_swap_tests {
    use super::*;
    use crate::mm2::lp_native_dex::{fix_directories, init_p2p};
    use coins::utxo::rpc_clients::ElectrumRpcRequest;
    use coins::utxo::utxo_standard::utxo_standard_coin_with_priv_key;
    use coins::utxo::{UtxoActivationParams, UtxoRpcMode};
    use coins::MarketCoinOps;
    use coins::PrivKeyActivationPolicy;
    use common::{block_on, new_uuid};
    use mm2_core::mm_ctx::MmCtxBuilder;
    use mm2_test_helpers::for_tests::{morty_conf, rick_conf, MORTY_ELECTRUM_ADDRS, RICK_ELECTRUM_ADDRS};

    #[test]
    fn test_dex_fee_amount() {
        let dex_fee_threshold = MmNumber::from("0.0001");

        let base = "BTC";
        let rel = "ETH";
        let amount = 1.into();
        let actual_fee = dex_fee_amount(base, rel, &amount, &dex_fee_threshold);
        let expected_fee = amount / 777u64.into();
        assert_eq!(expected_fee, actual_fee);

        let base = "KMD";
        let rel = "ETH";
        let amount = 1.into();
        let actual_fee = dex_fee_amount(base, rel, &amount, &dex_fee_threshold);
        let expected_fee = amount * (9, 7770).into();
        assert_eq!(expected_fee, actual_fee);

        let base = "BTC";
        let rel = "KMD";
        let amount = 1.into();
        let actual_fee = dex_fee_amount(base, rel, &amount, &dex_fee_threshold);
        let expected_fee = amount * (9, 7770).into();
        assert_eq!(expected_fee, actual_fee);

        let base = "BTC";
        let rel = "KMD";
        let amount: MmNumber = "0.001".parse::<BigDecimal>().unwrap().into();
        let actual_fee = dex_fee_amount(base, rel, &amount, &dex_fee_threshold);
        assert_eq!(dex_fee_threshold, actual_fee);
    }

    #[test]
    fn test_lp_atomic_locktime() {
        let maker_coin = "KMD";
        let taker_coin = "DEX";
        let my_conf_settings = SwapConfirmationsSettings {
            maker_coin_confs: 2,
            maker_coin_nota: true,
            taker_coin_confs: 2,
            taker_coin_nota: true,
        };
        let other_conf_settings = SwapConfirmationsSettings {
            maker_coin_confs: 1,
            maker_coin_nota: false,
            taker_coin_confs: 1,
            taker_coin_nota: false,
        };
        let expected = get_payment_locktime() * 4;
        let version = AtomicLocktimeVersion::V2 {
            my_conf_settings,
            other_conf_settings,
        };
        let actual = lp_atomic_locktime(maker_coin, taker_coin, version);
        assert_eq!(expected, actual);

        let maker_coin = "KMD";
        let taker_coin = "DEX";
        let my_conf_settings = SwapConfirmationsSettings {
            maker_coin_confs: 2,
            maker_coin_nota: true,
            taker_coin_confs: 2,
            taker_coin_nota: false,
        };
        let other_conf_settings = SwapConfirmationsSettings {
            maker_coin_confs: 1,
            maker_coin_nota: false,
            taker_coin_confs: 1,
            taker_coin_nota: false,
        };
        let expected = get_payment_locktime() * 4;
        let version = AtomicLocktimeVersion::V2 {
            my_conf_settings,
            other_conf_settings,
        };
        let actual = lp_atomic_locktime(maker_coin, taker_coin, version);
        assert_eq!(expected, actual);

        let maker_coin = "KMD";
        let taker_coin = "DEX";
        let my_conf_settings = SwapConfirmationsSettings {
            maker_coin_confs: 2,
            maker_coin_nota: false,
            taker_coin_confs: 2,
            taker_coin_nota: true,
        };
        let other_conf_settings = SwapConfirmationsSettings {
            maker_coin_confs: 1,
            maker_coin_nota: false,
            taker_coin_confs: 1,
            taker_coin_nota: false,
        };
        let expected = get_payment_locktime() * 4;
        let version = AtomicLocktimeVersion::V2 {
            my_conf_settings,
            other_conf_settings,
        };
        let actual = lp_atomic_locktime(maker_coin, taker_coin, version);
        assert_eq!(expected, actual);

        let maker_coin = "KMD";
        let taker_coin = "DEX";
        let my_conf_settings = SwapConfirmationsSettings {
            maker_coin_confs: 2,
            maker_coin_nota: false,
            taker_coin_confs: 2,
            taker_coin_nota: false,
        };
        let other_conf_settings = SwapConfirmationsSettings {
            maker_coin_confs: 1,
            maker_coin_nota: false,
            taker_coin_confs: 1,
            taker_coin_nota: false,
        };
        let expected = get_payment_locktime();
        let version = AtomicLocktimeVersion::V2 {
            my_conf_settings,
            other_conf_settings,
        };
        let actual = lp_atomic_locktime(maker_coin, taker_coin, version);
        assert_eq!(expected, actual);

        let maker_coin = "BTC";
        let taker_coin = "DEX";
        let my_conf_settings = SwapConfirmationsSettings {
            maker_coin_confs: 2,
            maker_coin_nota: false,
            taker_coin_confs: 2,
            taker_coin_nota: false,
        };
        let other_conf_settings = SwapConfirmationsSettings {
            maker_coin_confs: 1,
            maker_coin_nota: false,
            taker_coin_confs: 1,
            taker_coin_nota: false,
        };
        let expected = get_payment_locktime() * 4;
        let version = AtomicLocktimeVersion::V2 {
            my_conf_settings,
            other_conf_settings,
        };
        let actual = lp_atomic_locktime(maker_coin, taker_coin, version);
        assert_eq!(expected, actual);

        let maker_coin = "KMD";
        let taker_coin = "BTC";
        let my_conf_settings = SwapConfirmationsSettings {
            maker_coin_confs: 2,
            maker_coin_nota: false,
            taker_coin_confs: 2,
            taker_coin_nota: false,
        };
        let other_conf_settings = SwapConfirmationsSettings {
            maker_coin_confs: 1,
            maker_coin_nota: false,
            taker_coin_confs: 1,
            taker_coin_nota: false,
        };
        let expected = get_payment_locktime() * 4;
        let version = AtomicLocktimeVersion::V2 {
            my_conf_settings,
            other_conf_settings,
        };
        let actual = lp_atomic_locktime(maker_coin, taker_coin, version);
        assert_eq!(expected, actual);

        let maker_coin = "KMD";
        let taker_coin = "DEX";
        let expected = get_payment_locktime();
        let actual = lp_atomic_locktime(maker_coin, taker_coin, AtomicLocktimeVersion::V1);
        assert_eq!(expected, actual);

        let maker_coin = "KMD";
        let taker_coin = "DEX";
        let expected = get_payment_locktime();
        let actual = lp_atomic_locktime(maker_coin, taker_coin, AtomicLocktimeVersion::V1);
        assert_eq!(expected, actual);

        let maker_coin = "KMD";
        let taker_coin = "DEX";
        let expected = get_payment_locktime();
        let actual = lp_atomic_locktime(maker_coin, taker_coin, AtomicLocktimeVersion::V1);
        assert_eq!(expected, actual);

        let maker_coin = "KMD";
        let taker_coin = "DEX";
        let expected = get_payment_locktime();
        let actual = lp_atomic_locktime(maker_coin, taker_coin, AtomicLocktimeVersion::V1);
        assert_eq!(expected, actual);

        let maker_coin = "BTC";
        let taker_coin = "DEX";
        let expected = get_payment_locktime() * 10;
        let actual = lp_atomic_locktime(maker_coin, taker_coin, AtomicLocktimeVersion::V1);
        assert_eq!(expected, actual);

        let maker_coin = "KMD";
        let taker_coin = "BTC";
        let expected = get_payment_locktime() * 10;
        let actual = lp_atomic_locktime(maker_coin, taker_coin, AtomicLocktimeVersion::V1);
        assert_eq!(expected, actual);
    }

    #[test]
    fn check_negotiation_data_serde() {
        // old message format should be deserialized to NegotiationDataMsg::V1
        let v1 = NegotiationDataV1 {
            started_at: 0,
            payment_locktime: 0,
            secret_hash: [0; 20],
            persistent_pubkey: vec![1; 33],
        };

        let expected = NegotiationDataMsg::V1(NegotiationDataV1 {
            started_at: 0,
            payment_locktime: 0,
            secret_hash: [0; 20],
            persistent_pubkey: vec![1; 33],
        });

        let serialized = rmp_serde::to_vec_named(&v1).unwrap();

        let deserialized: NegotiationDataMsg = rmp_serde::from_slice(serialized.as_slice()).unwrap();

        assert_eq!(deserialized, expected);

        // new message format should be deserialized to old
        let v2 = NegotiationDataMsg::V2(NegotiationDataV2 {
            started_at: 0,
            payment_locktime: 0,
            secret_hash: vec![0; 20],
            persistent_pubkey: vec![1; 33],
            maker_coin_swap_contract: vec![1; 20],
            taker_coin_swap_contract: vec![1; 20],
        });

        let expected = NegotiationDataV1 {
            started_at: 0,
            payment_locktime: 0,
            secret_hash: [0; 20],
            persistent_pubkey: vec![1; 33],
        };

        let serialized = rmp_serde::to_vec_named(&v2).unwrap();

        let deserialized: NegotiationDataV1 = rmp_serde::from_slice(serialized.as_slice()).unwrap();

        assert_eq!(deserialized, expected);

        // new message format should be deserialized to new
        let v2 = NegotiationDataMsg::V2(NegotiationDataV2 {
            started_at: 0,
            payment_locktime: 0,
            secret_hash: vec![0; 20],
            persistent_pubkey: vec![1; 33],
            maker_coin_swap_contract: vec![1; 20],
            taker_coin_swap_contract: vec![1; 20],
        });

        let serialized = rmp_serde::to_vec(&v2).unwrap();

        let deserialized: NegotiationDataMsg = rmp_serde::from_slice(serialized.as_slice()).unwrap();

        assert_eq!(deserialized, v2);

        let v3 = NegotiationDataMsg::V3(NegotiationDataV3 {
            started_at: 0,
            payment_locktime: 0,
            secret_hash: vec![0; 20],
            maker_coin_swap_contract: vec![1; 20],
            taker_coin_swap_contract: vec![1; 20],
            maker_coin_htlc_pub: vec![1; 33],
            taker_coin_htlc_pub: vec![1; 33],
        });

        // v3 must be deserialized to v3, backward compatibility is not required
        let serialized = rmp_serde::to_vec(&v3).unwrap();

        let deserialized: NegotiationDataMsg = rmp_serde::from_slice(serialized.as_slice()).unwrap();

        assert_eq!(deserialized, v3);
    }

    #[test]
    fn check_payment_data_serde() {
        const MSG_DATA_INSTRUCTIONS: [u8; 300] = [1; 300];

        #[derive(Clone, Debug, Eq, Deserialize, PartialEq, Serialize)]
        enum SwapMsgOld {
            Negotiation(NegotiationDataMsg),
            NegotiationReply(NegotiationDataMsg),
            Negotiated(bool),
            TakerFee(Vec<u8>),
            MakerPayment(Vec<u8>),
            TakerPayment(Vec<u8>),
        }

        // old message format should be deserialized to PaymentDataMsg::Regular
        let old = SwapMsgOld::MakerPayment(MSG_DATA_INSTRUCTIONS.to_vec());

        let expected = SwapMsg::MakerPayment(SwapTxDataMsg::Regular(MSG_DATA_INSTRUCTIONS.to_vec()));

        let serialized = rmp_serde::to_vec_named(&old).unwrap();

        let deserialized: SwapMsg = rmp_serde::from_slice(serialized.as_slice()).unwrap();

        assert_eq!(deserialized, expected);

        // PaymentDataMsg::Regular should be deserialized to old message format
        let v1 = SwapMsg::MakerPayment(SwapTxDataMsg::Regular(MSG_DATA_INSTRUCTIONS.to_vec()));

        let expected = old;

        let serialized = rmp_serde::to_vec_named(&v1).unwrap();

        let deserialized: SwapMsgOld = rmp_serde::from_slice(serialized.as_slice()).unwrap();

        assert_eq!(deserialized, expected);

        // PaymentDataMsg::Regular should be deserialized to PaymentDataMsg::Regular
        let v1 = SwapMsg::MakerPayment(SwapTxDataMsg::Regular(MSG_DATA_INSTRUCTIONS.to_vec()));

        let serialized = rmp_serde::to_vec_named(&v1).unwrap();

        let deserialized: SwapMsg = rmp_serde::from_slice(serialized.as_slice()).unwrap();

        assert_eq!(deserialized, v1);

        // PaymentDataMsg::WithInstructions should be deserialized to PaymentDataMsg::WithInstructions
        let v2 = SwapMsg::MakerPayment(SwapTxDataMsg::WithInstructions(PaymentWithInstructions {
            data: MSG_DATA_INSTRUCTIONS.to_vec(),
            next_step_instructions: MSG_DATA_INSTRUCTIONS.to_vec(),
        }));

        let serialized = rmp_serde::to_vec_named(&v2).unwrap();

        let deserialized: SwapMsg = rmp_serde::from_slice(serialized.as_slice()).unwrap();

        assert_eq!(deserialized, v2);

        // PaymentDataMsg::WithInstructions shouldn't be deserialized to old message format, new nodes with payment instructions can't swap with old nodes without it.
        let v2 = SwapMsg::MakerPayment(SwapTxDataMsg::WithInstructions(PaymentWithInstructions {
            data: MSG_DATA_INSTRUCTIONS.to_vec(),
            next_step_instructions: MSG_DATA_INSTRUCTIONS.to_vec(),
        }));

        let serialized = rmp_serde::to_vec_named(&v2).unwrap();

        let deserialized: Result<SwapMsgOld, rmp_serde::decode::Error> = rmp_serde::from_slice(serialized.as_slice());

        assert!(deserialized.is_err());
    }

    fn utxo_activation_params(electrums: &[&str]) -> UtxoActivationParams {
        UtxoActivationParams {
            mode: UtxoRpcMode::Electrum {
                servers: electrums
                    .iter()
                    .map(|url| ElectrumRpcRequest {
                        url: url.to_string(),
                        protocol: Default::default(),
                        disable_cert_verification: false,
                    })
                    .collect(),
            },
            utxo_merge_params: None,
            tx_history: false,
            required_confirmations: Some(0),
            requires_notarization: None,
            address_format: None,
            gap_limit: None,
            enable_params: Default::default(),
            priv_key_policy: PrivKeyActivationPolicy::ContextPrivKey,
            check_utxo_maturity: None,
        }
    }

    #[test]
    #[ignore]
    fn gen_recoverable_swap() {
        let maker_passphrase = std::env::var("BOB_PASSPHRASE").expect("BOB_PASSPHRASE env must be set");
        let maker_fail_at = std::env::var("MAKER_FAIL_AT").map(maker_swap::FailAt::from).ok();
        let taker_passphrase = std::env::var("ALICE_PASSPHRASE").expect("ALICE_PASSPHRASE env must be set");
        let taker_fail_at = std::env::var("TAKER_FAIL_AT").map(taker_swap::FailAt::from).ok();
        let lock_duration = match std::env::var("LOCK_DURATION") {
            Ok(maybe_num) => maybe_num.parse().expect("LOCK_DURATION must be a number of seconds"),
            Err(_) => 30,
        };

        if maker_fail_at.is_none() && taker_fail_at.is_none() {
            panic!("At least one of MAKER_FAIL_AT/TAKER_FAIL_AT must be provided");
        }

        let maker_ctx_conf = json!({
            "netid": 1234,
            "p2p_in_memory": true,
            "p2p_in_memory_port": 777,
            "i_am_seed": true,
        });

        let maker_ctx = MmCtxBuilder::default().with_conf(maker_ctx_conf).into_mm_arc();
        let maker_key_pair = *CryptoCtx::init_with_iguana_passphrase(maker_ctx.clone(), &maker_passphrase)
            .unwrap()
            .mm2_internal_key_pair();

        fix_directories(&maker_ctx).unwrap();
        block_on(init_p2p(maker_ctx.clone())).unwrap();
        maker_ctx.init_sqlite_connection().unwrap();

        let rick_activation_params = utxo_activation_params(RICK_ELECTRUM_ADDRS);
        let morty_activation_params = utxo_activation_params(MORTY_ELECTRUM_ADDRS);

        let rick_maker = block_on(utxo_standard_coin_with_priv_key(
            &maker_ctx,
            "RICK",
            &rick_conf(),
            &rick_activation_params,
            maker_key_pair.private().secret,
        ))
        .unwrap();

        println!("Maker address {}", rick_maker.my_address().unwrap());

        let morty_maker = block_on(utxo_standard_coin_with_priv_key(
            &maker_ctx,
            "MORTY",
            &morty_conf(),
            &morty_activation_params,
            maker_key_pair.private().secret,
        ))
        .unwrap();

        let taker_ctx_conf = json!({
            "netid": 1234,
            "p2p_in_memory": true,
            "seednodes": vec!["/memory/777"]
        });

        let taker_ctx = MmCtxBuilder::default().with_conf(taker_ctx_conf).into_mm_arc();
        let taker_key_pair = *CryptoCtx::init_with_iguana_passphrase(taker_ctx.clone(), &taker_passphrase)
            .unwrap()
            .mm2_internal_key_pair();

        fix_directories(&taker_ctx).unwrap();
        block_on(init_p2p(taker_ctx.clone())).unwrap();
        taker_ctx.init_sqlite_connection().unwrap();

        let rick_taker = block_on(utxo_standard_coin_with_priv_key(
            &taker_ctx,
            "RICK",
            &rick_conf(),
            &rick_activation_params,
            taker_key_pair.private().secret,
        ))
        .unwrap();

        let morty_taker = block_on(utxo_standard_coin_with_priv_key(
            &taker_ctx,
            "MORTY",
            &morty_conf(),
            &morty_activation_params,
            taker_key_pair.private().secret,
        ))
        .unwrap();

        println!("Taker address {}", rick_taker.my_address().unwrap());

        let uuid = new_uuid();
        let maker_amount = BigDecimal::from_str("0.1").unwrap();
        let taker_amount = BigDecimal::from_str("0.1").unwrap();
        let conf_settings = SwapConfirmationsSettings {
            maker_coin_confs: 0,
            maker_coin_nota: false,
            taker_coin_confs: 0,
            taker_coin_nota: false,
        };

        let mut maker_swap = MakerSwap::new(
            maker_ctx.clone(),
            taker_key_pair.public().compressed_unprefixed().unwrap().into(),
            maker_amount.clone(),
            taker_amount.clone(),
            maker_key_pair.public_slice().into(),
            uuid,
            None,
            conf_settings,
            rick_maker.into(),
            morty_maker.into(),
            lock_duration,
            None,
            Default::default(),
        );

        maker_swap.fail_at = maker_fail_at;

        let mut taker_swap = TakerSwap::new(
            taker_ctx.clone(),
            maker_key_pair.public().compressed_unprefixed().unwrap().into(),
            maker_amount.into(),
            taker_amount.into(),
            taker_key_pair.public_slice().into(),
            uuid,
            None,
            conf_settings,
            rick_taker.into(),
            morty_taker.into(),
            lock_duration,
            None,
        );

        taker_swap.fail_at = taker_fail_at;

        block_on(futures::future::join(
            run_maker_swap(RunMakerSwapInput::StartNew(maker_swap), maker_ctx.clone()),
            run_taker_swap(RunTakerSwapInput::StartNew(taker_swap), taker_ctx.clone()),
        ));

        println!(
            "Maker swap path {}",
            std::fs::canonicalize(my_swap_file_path(&maker_ctx, &uuid))
                .unwrap()
                .display()
        );
        println!(
            "Taker swap path {}",
            std::fs::canonicalize(my_swap_file_path(&taker_ctx, &uuid))
                .unwrap()
                .display()
        );
    }

    #[test]
    fn test_deserialize_iris_swap_status() {
        let _: SavedSwap = json::from_str(include_str!("for_tests/iris_nimda_rick_taker_swap.json")).unwrap();
        let _: SavedSwap = json::from_str(include_str!("for_tests/iris_nimda_rick_maker_swap.json")).unwrap();
    }
}
