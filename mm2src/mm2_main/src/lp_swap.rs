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

use crate::mm2::lp_network::{broadcast_p2p_msg, Libp2pPeerId};
use async_std::sync as async_std_sync;
use coins::{lp_coinfind, MmCoinEnum, TradeFee, TransactionEnum};
use common::log::{debug, warn};
use common::{bits256, calc_total_pages,
             executor::{spawn, Timer},
             log::{error, info},
             mm_number::{BigDecimal, BigRational, MmNumber},
             now_ms, spawn_abortable, var, AbortOnDropHandle, PagingOptions};
use derive_more::Display;
use http::Response;
use mm2_core::mm_ctx::{from_ctx, MmArc};
use mm2_err_handle::prelude::*;
use mm2_libp2p::{decode_signed, encode_and_sign, pub_sub_topic, TopicPrefix};
use primitives::hash::{H160, H264};
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use serde_json::{self as json, Value as Json};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex, Weak};
use uuid::Uuid;

#[cfg(feature = "custom-swap-locktime")]
use std::sync::atomic::{AtomicU64, Ordering};

#[path = "lp_swap/check_balance.rs"] mod check_balance;
#[path = "lp_swap/maker_swap.rs"] mod maker_swap;
#[path = "lp_swap/my_swaps_storage.rs"] mod my_swaps_storage;
#[path = "lp_swap/pubkey_banning.rs"] mod pubkey_banning;
#[path = "lp_swap/recreate_swap_data.rs"] mod recreate_swap_data;
#[path = "lp_swap/saved_swap.rs"] mod saved_swap;
#[path = "lp_swap/swap_lock.rs"] mod swap_lock;
#[path = "lp_swap/taker_swap.rs"] mod taker_swap;
#[path = "lp_swap/trade_preimage.rs"] mod trade_preimage;

#[cfg(target_arch = "wasm32")]
#[path = "lp_swap/swap_wasm_db.rs"]
mod swap_wasm_db;

pub use check_balance::{check_other_coin_balance_for_swap, CheckBalanceError};
use keys::KeyPair;
use maker_swap::MakerSwapEvent;
pub use maker_swap::{calc_max_maker_vol, check_balance_for_maker_swap, maker_swap_trade_preimage, run_maker_swap,
                     MakerSavedEvent, MakerSavedSwap, MakerSwap, MakerSwapStatusChanged, MakerTradePreimage,
                     RunMakerSwapInput};
use my_swaps_storage::{MySwapsOps, MySwapsStorage};
use pubkey_banning::BanReason;
pub use pubkey_banning::{ban_pubkey_rpc, is_pubkey_banned, list_banned_pubkeys_rpc, unban_pubkeys_rpc};
pub use recreate_swap_data::recreate_swap_data;
pub use saved_swap::{SavedSwap, SavedSwapError, SavedSwapIo, SavedSwapResult};
use taker_swap::TakerSwapEvent;
pub use taker_swap::{calc_max_taker_vol, check_balance_for_taker_swap, max_taker_vol, max_taker_vol_from_available,
                     run_taker_swap, taker_swap_trade_preimage, RunTakerSwapInput, TakerSavedSwap, TakerSwap,
                     TakerSwapPreparedParams, TakerTradePreimage};
pub use trade_preimage::trade_preimage_rpc;

pub const SWAP_PREFIX: TopicPrefix = "swap";

pub const TX_HELPER_PREFIX: TopicPrefix = "txhlp";

cfg_wasm32! {
    use mm2_db::indexed_db::{ConstructibleDb, DbLocked};
    use swap_wasm_db::{InitDbResult, SwapDb};

    pub type SwapDbLocked<'a> = DbLocked<'a, SwapDb>;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SwapMsg {
    Negotiation(NegotiationDataMsg),
    NegotiationReply(NegotiationDataMsg),
    Negotiated(bool),
    TakerFee(Vec<u8>),
    MakerPayment(Vec<u8>),
    TakerPayment(Vec<u8>),
}

#[derive(Debug, Default)]
pub struct SwapMsgStore {
    negotiation: Option<NegotiationDataMsg>,
    negotiation_reply: Option<NegotiationDataMsg>,
    negotiated: Option<bool>,
    taker_fee: Option<Vec<u8>>,
    maker_payment: Option<Vec<u8>>,
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

/// Spawns the loop that broadcasts message every `interval` seconds returning the AbortOnDropHandle
/// to stop it
pub fn broadcast_swap_message_every(
    ctx: MmArc,
    topic: String,
    msg: SwapMsg,
    interval: f64,
    p2p_privkey: Option<KeyPair>,
) -> AbortOnDropHandle {
    let fut = async move {
        loop {
            broadcast_swap_message(&ctx, topic.clone(), msg.clone(), &p2p_privkey);
            Timer::sleep(interval).await;
        }
    };
    spawn_abortable(fut)
}

/// Broadcast the swap message once
pub fn broadcast_swap_message(ctx: &MmArc, topic: String, msg: SwapMsg, p2p_privkey: &Option<KeyPair>) {
    let (p2p_private, from) = match p2p_privkey {
        Some(keypair) => (keypair.private_bytes(), Some(keypair.libp2p_peer_id())),
        None => (ctx.secp256k1_key_pair().private().secret.take(), None),
    };
    let encoded_msg = encode_and_sign(&msg, &p2p_private).unwrap();
    broadcast_p2p_msg(ctx, vec![topic], encoded_msg, from);
}

/// Broadcast the tx message once
pub fn broadcast_p2p_tx_msg(ctx: &MmArc, topic: String, msg: &TransactionEnum, p2p_privkey: &Option<KeyPair>) {
    let (p2p_private, from) = match p2p_privkey {
        Some(keypair) => (keypair.private_bytes(), Some(keypair.libp2p_peer_id())),
        None => (ctx.secp256k1_key_pair().private().secret.take(), None),
    };

    let encoded_msg = encode_and_sign(&msg.tx_hex(), &p2p_private).unwrap();
    broadcast_p2p_msg(ctx, vec![topic], encoded_msg, from);
}

pub async fn process_msg(ctx: MmArc, topic: &str, msg: &[u8]) {
    let uuid = match Uuid::from_str(topic) {
        Ok(u) => u,
        Err(_) => return,
    };
    let msg = match decode_signed::<SwapMsg>(msg) {
        Ok(m) => m,
        Err(swap_msg_err) => {
            #[cfg(not(target_arch = "wasm32"))]
            match json::from_slice::<SwapStatus>(msg) {
                Ok(mut status) => {
                    status.data.fetch_and_set_usd_prices().await;
                    if let Err(e) = save_stats_swap(&ctx, &status.data).await {
                        error!("Error saving the swap {} status: {}", status.data.uuid(), e);
                    }
                },
                Err(swap_status_err) => {
                    error!("Couldn't deserialize 'SwapMsg': {:?}", swap_msg_err);
                    error!("Couldn't deserialize 'SwapStatus': {:?}", swap_status_err);
                },
            };
            // Drop it to avoid dead_code warning
            drop(swap_msg_err);
            return;
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
                SwapMsg::TakerFee(taker_fee) => msg_store.taker_fee = Some(taker_fee),
                SwapMsg::MakerPayment(maker_payment) => msg_store.maker_payment = Some(maker_payment),
                SwapMsg::TakerPayment(taker_payment) => msg_store.taker_payment = Some(taker_payment),
            }
        } else {
            warn!("Received message from unexpected sender for swap {}", uuid);
        }
    }
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
    let started = now_ms() / 1000;
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
        let now = now_ms() / 1000;
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

const _SWAP_DEFAULT_NUM_CONFIRMS: u32 = 1;
const _SWAP_DEFAULT_MAX_CONFIRMS: u32 = 6;
/// MM2 checks that swap payment is confirmed every WAIT_CONFIRM_INTERVAL seconds
const WAIT_CONFIRM_INTERVAL: u64 = 15;

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
    /// The cloneable receiver of multi-consumer async channel awaiting for shutdown_tx.send() to be
    /// invoked to stop all running swaps.
    /// MM2 is used as static lib on some platforms e.g. iOS so it doesn't run as separate process.
    /// So when stop was invoked the swaps could stay running on shared executors causing
    /// Very unpleasant consequences
    shutdown_rx: async_std_sync::Receiver<()>,
    swap_msgs: Mutex<HashMap<Uuid, SwapMsgStore>>,
    #[cfg(target_arch = "wasm32")]
    swap_db: ConstructibleDb<SwapDb>,
}

impl SwapsContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    fn from_ctx(ctx: &MmArc) -> Result<Arc<SwapsContext>, String> {
        Ok(try_s!(from_ctx(&ctx.swaps_ctx, move || {
            let (shutdown_tx, shutdown_rx) = async_std_sync::channel(1);
            let mut shutdown_tx = Some(shutdown_tx);
            ctx.on_stop(Box::new(move || {
                if let Some(shutdown_tx) = shutdown_tx.take() {
                    info!("on_stop] firing shutdown_tx!");
                    spawn(async move {
                        shutdown_tx.send(()).await;
                    });
                    Ok(())
                } else {
                    ERR!("on_stop callback called twice!")
                }
            }));

            Ok(SwapsContext {
                running_swaps: Mutex::new(vec![]),
                banned_pubkeys: Mutex::new(HashMap::new()),
                shutdown_rx,
                swap_msgs: Mutex::new(HashMap::new()),
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
    pub async fn swap_db(&self) -> InitDbResult<SwapDbLocked<'_>> { Ok(self.swap_db.get_or_initialize().await?) }
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

pub fn active_swaps_using_coin(ctx: &MmArc, coin: &str) -> Result<Vec<Uuid>, String> {
    let swap_ctx = try_s!(SwapsContext::from_ctx(ctx));
    let swaps = try_s!(swap_ctx.running_swaps.lock());
    let mut uuids = vec![];
    for swap in swaps.iter() {
        if let Some(swap) = swap.upgrade() {
            if swap.maker_coin() == coin || swap.taker_coin() == coin {
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
    if maker_coin == "BTC"
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

fn dex_fee_threshold(min_tx_amount: MmNumber) -> MmNumber {
    // 0.0001
    let min_fee = MmNumber::from((1, 10000));
    if min_fee < min_tx_amount {
        min_tx_amount
    } else {
        min_fee
    }
}

fn dex_fee_rate(base: &str, rel: &str) -> MmNumber {
    let fee_discount_tickers: &[&str] = if cfg!(test) && var("MYCOIN_FEE_DISCOUNT").is_ok() {
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

/// Data to be exchanged and validated on swap start, the replacement of LP_pubkeys_data, LP_choosei_data, etc.
#[derive(Debug, Default, Deserializable, Eq, PartialEq, Serializable)]
struct SwapNegotiationData {
    started_at: u64,
    payment_locktime: u64,
    secret_hash: H160,
    persistent_pubkey: H264,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TransactionIdentifier {
    /// Raw bytes of signed transaction in hexadecimal string, this should be sent as is to send_raw_transaction RPC to broadcast the transaction
    tx_hex: BytesJson,
    /// Transaction hash in hexadecimal format
    tx_hash: BytesJson,
}

pub fn my_swaps_dir(ctx: &MmArc) -> PathBuf { ctx.dbdir().join("SWAPS").join("MY") }

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
    crate::mm2::database::stats_swaps::add_swap_to_index(&ctx.sqlite_connection(), swap)
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
        let swap_json = match SavedSwap::load_my_swap_from_db(&ctx, *uuid).await {
            Ok(Some(swap)) => json::to_value(MySwapStatusResponse::from(swap)).unwrap(),
            Ok(None) => {
                error!("No such swap with the uuid '{}'", uuid);
                Json::Null
            },
            Err(e) => {
                error!("Error loading a swap with the uuid '{}': {}", uuid, e);
                Json::Null
            },
        };
        swaps.push(swap_json);
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

        let ctx = ctx.clone();

        // kick-start the swap in a separate thread.
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(move || {
            common::block_on(kickstart_thread_handler(
                ctx.clone(),
                swap,
                maker_coin_ticker,
                taker_coin_ticker,
            ))
        });

        #[cfg(target_arch = "wasm32")]
        common::executor::spawn(async move {
            kickstart_thread_handler(ctx, swap, maker_coin_ticker, taker_coin_ticker).await
        });
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

#[cfg(test)]
mod lp_swap_tests {
    use serialization::{deserialize, serialize};

    use super::*;

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
    fn test_serde_swap_negotiation_data() {
        let data = SwapNegotiationData::default();
        let bytes = serialize(&data);
        let deserialized = deserialize(bytes.as_slice()).unwrap();
        assert_eq!(data, deserialized);
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

        let serialized = rmp_serde::to_vec(&v1).unwrap();

        let deserialized: NegotiationDataMsg = rmp_serde::from_read_ref(serialized.as_slice()).unwrap();

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

        let serialized = rmp_serde::to_vec(&v2).unwrap();

        let deserialized: NegotiationDataV1 = rmp_serde::from_read_ref(serialized.as_slice()).unwrap();

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

        let deserialized: NegotiationDataMsg = rmp_serde::from_read_ref(serialized.as_slice()).unwrap();

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

        let deserialized: NegotiationDataMsg = rmp_serde::from_read_ref(serialized.as_slice()).unwrap();

        assert_eq!(deserialized, v3);
    }
}
