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
 * Copyright © 2014-2018 The SuperNET Developers.                             *
 *                                                                            *
 * See the AUTHORS, DEVELOPER-AGREEMENT and LICENSE files at                  *
 * the top-level directory of this distribution for the individual copyright  *
 * holder information and the developer policies on copyright and licensing.  *
 *                                                                            *
 * Unless otherwise agreed in a custom licensing agreement, no part of the    *
 * SuperNET software, including this file may be copied, modified, propagated *
 * or distributed except according to the terms contained in the LICENSE file *
 *                                                                            *
 * Removal or modification of this copyright notice is prohibited.            *
 *                                                                            *
 ******************************************************************************/
//
//  lp_swap.rs
//  marketmaker
//
use bitcrypto::{ChecksumType, dhash160};
use rpc::v1::types::{H160 as H160Json, H256 as H256Json, H264 as H264Json};
use coins::{lp_coinfind, MmCoinEnum, TradeInfo, TransactionDetails};
use coins::utxo::compressed_pub_key_from_priv_raw;
use common::{bits256, HyRes, lp, rpc_response};
use common::wio::Timeout;
use common::log::{TagParam};
use common::mm_ctx::{from_ctx, MmArc};
use crc::crc32;
use futures::{Future};
use gstuff::{now_ms, slurp};
use hashbrown::{HashSet, HashMap};
use hashbrown::hash_map::Entry;
use http::Response;
use primitives::hash::{H160, H264};
use rand::Rng;
use serde_json::{self as json, Value as Json};
use serialization::{deserialize, serialize};
use std::ffi::OsStr;
use std::fs::{File, DirEntry};
use std::io::prelude::*;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};
use bigdecimal::BigDecimal;

/// Includes the grace time we add to the "normal" timeouts
/// in order to give different and/or heavy communication channels a chance.
const BASIC_COMM_TIMEOUT: u64 = 90;

/// Default atomic swap payment locktime, in seconds.
/// Maker sends payment with LOCKTIME * 2
/// Taker sends payment with LOCKTIME
const PAYMENT_LOCKTIME: u64 = 3600 * 2 + 300 * 2;
const _SWAP_DEFAULT_NUM_CONFIRMS: u32 = 1;
const _SWAP_DEFAULT_MAX_CONFIRMS: u32 = 6;

/// Represents the amount of a coin locked by ongoing swap
struct LockedAmount {
    coin: String,
    amount: BigDecimal,
}

struct SwapsContext {
    locked_amounts: Mutex<HashMap<String, LockedAmount>>,
}

impl SwapsContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    fn from_ctx (ctx: &MmArc) -> Result<Arc<SwapsContext>, String> {
        Ok (try_s! (from_ctx (&ctx.swaps_ctx, move || {
            Ok (SwapsContext {
                locked_amounts: Mutex::new(HashMap::new()),
            })
        })))
    }
}

/// Virtually locks the amount of a coin, called when swap is instantiated
fn lock_amount(ctx: &MmArc, uuid: String, coin: String, amount: BigDecimal) {
    let swap_ctx = unwrap!(SwapsContext::from_ctx(&ctx));
    let mut locked = unwrap!(swap_ctx.locked_amounts.lock());
    locked.insert(uuid, LockedAmount {
        coin,
        amount,
    });
}

/// Virtually unlocks the amount of a coin, called when swap transaction is sent so the real balance
/// is updated and virtual lock is not required.
fn unlock_amount(ctx: &MmArc, uuid: &str, amount: &BigDecimal) {
    let swap_ctx = unwrap!(SwapsContext::from_ctx(&ctx));
    let mut locked = unwrap!(swap_ctx.locked_amounts.lock());
    match locked.entry(uuid.into()) {
        Entry::Occupied(mut e) => {
            let entry = e.get_mut();
            if &entry.amount <= amount {
                e.remove();
            } else {
                entry.amount -= amount;
            };
        },
        Entry::Vacant(_) => (),
    };
}

/// Get total amount of selected coin locked by all currently ongoing swaps
pub fn get_locked_amount(ctx: &MmArc, coin: &str) -> BigDecimal {
    let swap_ctx = unwrap!(SwapsContext::from_ctx(&ctx));
    let locked = unwrap!(swap_ctx.locked_amounts.lock());
    locked.iter().fold(
        0.into(),
        |total, (_, locked)| if locked.coin == coin {
            total + &locked.amount
        } else {
            total
        }
    )
}

/// Get total amount of selected coin locked by all currently ongoing swaps except the one with selected uuid
fn get_locked_amount_by_other_swaps(ctx: &MmArc, except_uuid: &str, coin: &str) -> BigDecimal {
    let swap_ctx = unwrap!(SwapsContext::from_ctx(&ctx));
    let locked = unwrap!(swap_ctx.locked_amounts.lock());
    locked.iter().fold(
        0.into(),
        |total, (uuid, locked)| if uuid != except_uuid && locked.coin == coin {
            total + &locked.amount
        } else {
            total
        }
    )
}

/// Some coins are "slow" (block time is high - e.g. BTC average block time is ~10 minutes).
/// https://bitinfocharts.com/comparison/bitcoin-confirmationtime.html
/// We need to increase payment locktime accordingly when at least 1 side of swap uses "slow" coin.
fn lp_atomic_locktime(base: &str, rel: &str) -> u64 {
    if base == "BTC" || rel == "BTC" {
        PAYMENT_LOCKTIME * 10
    } else if base == "BCH" || rel == "BCH" || base == "BTG" || rel == "BTG" || base == "SBTC" || rel == "SBTC" {
        PAYMENT_LOCKTIME * 4
    } else {
        PAYMENT_LOCKTIME
    }
}

fn payment_confirmations(_maker_coin: &MmCoinEnum, _taker_coin: &MmCoinEnum) -> (u32, u32) {
    /*
    let mut maker_confirmations = SWAP_DEFAULT_NUM_CONFIRMS;
    let mut taker_confirmations = SWAP_DEFAULT_NUM_CONFIRMS;
    if maker_coin.ticker() == "BTC" {
        maker_confirmations = 1;
    }

    if taker_coin.ticker() == "BTC" {
        taker_confirmations = 1;
    }

    if maker_coin.is_asset_chain() {
        maker_confirmations = 1;
    }

    if taker_coin.is_asset_chain() {
        taker_confirmations = 1;
    }
    */

    // TODO recognize why the BAY case is special, ask JL777
    /*
        if ( strcmp("BAY",swap->I.req.src) != 0 && strcmp("BAY",swap->I.req.dest) != 0 )
    {
        swap->I.bobconfirms *= !swap->I.bobistrusted;
        swap->I.aliceconfirms *= !swap->I.aliceistrusted;
    }
    */
    (1, 1)
}

fn dex_fee_rate(base: &str, rel: &str) -> BigDecimal {
    if base == "KMD" || rel == "KMD" {
        // 1/777 - 10%
        BigDecimal::from(9) / BigDecimal::from(7770)
    } else {
        BigDecimal::from(1) / BigDecimal::from(777)
    }
}

pub fn dex_fee_amount(base: &str, rel: &str, trade_amount: &BigDecimal) -> BigDecimal {
    let rate = dex_fee_rate(base, rel);
    let min_fee = unwrap!("0.0001".parse());
    let fee_amount = trade_amount * rate;
    if fee_amount < min_fee {
        min_fee
    } else {
        fee_amount
    }
}

// NB: Using a macro instead of a function in order to preserve the line numbers in the log.
macro_rules! send {
    ($ctx: expr, $to: expr, $subj: expr, $fallback: expr, $payload: expr) => {{
        // Checksum here helps us visually verify the logistics between the Maker and Taker logs.
        let crc = crc32::checksum_ieee (&$payload);
        log!("Sending '" ($subj) "' (" ($payload.len()) " bytes, crc " (crc) ")");

        peers::send ($ctx, $to, $subj.as_bytes(), $fallback, $payload.into())
    }}
}

macro_rules! recv_ {
    ($swap: expr, $subj: expr, $timeout_sec: expr, $ec: expr, $validator: block) => {{
        let recv_subject = fomat! (($subj) '@' ($swap.uuid));
        let validator = Box::new ($validator) as Box<dyn Fn(&[u8]) -> Result<(), String> + Send>;
        let fallback = ($timeout_sec / 3) .min (30) .max (60) as u8;
        let recv_f = peers::recv (&$swap.ctx, recv_subject.as_bytes(), fallback, Box::new ({
            // NB: `peers::recv` is generic and not responsible for handling errors.
            //     Here, on the other hand, we should know enough to log the errors.
            //     Also through the macros the logging statements will carry informative line numbers on them.
            move |payload: &[u8]| -> bool {
                match validator (payload) {
                    Ok (()) => true,
                    Err (err) => {
                        log! ("Error validating payload '" ($subj) "' (" (payload.len()) " bytes, crc " (crc32::checksum_ieee (payload)) "): " (err) ". Retrying…");
                        false
                    }
                }
            }
        }));
        let recv_f = Timeout::new (recv_f, Duration::from_secs (BASIC_COMM_TIMEOUT + $timeout_sec));
        recv_f.wait().map(|payload| {
            // Checksum here helps us visually verify the logistics between the Maker and Taker logs.
            let crc = crc32::checksum_ieee (&payload);
            log! ("Received '" (recv_subject) "' (" (payload.len()) " bytes, crc " (crc) ")");
            payload
        })
    }}
}

/// Data to be exchanged and validated on swap start, the replacement of LP_pubkeys_data, LP_choosei_data, etc.
#[derive(Debug, Default, Deserializable, Eq, PartialEq, Serializable)]
struct SwapNegotiationData {
    started_at: u64,
    payment_locktime: u64,
    secret_hash: H160,
    persistent_pubkey: H264,
}

fn my_swaps_dir(ctx: &MmArc) -> PathBuf {
    ctx.dbdir().join("SWAPS").join("MY")
}

fn my_swap_file_path(ctx: &MmArc, uuid: &str) -> PathBuf {
    my_swaps_dir(ctx).join(format!("{}.json", uuid))
}

fn stats_maker_swap_file_path(ctx: &MmArc, uuid: &str) -> PathBuf {
    ctx.dbdir().join("SWAPS").join("STATS").join("MAKER").join(format!("{}.json", uuid))
}

fn stats_taker_swap_file_path(ctx: &MmArc, uuid: &str) -> PathBuf {
    ctx.dbdir().join("SWAPS").join("STATS").join("TAKER").join(format!("{}.json", uuid))
}

fn save_my_maker_swap_event(ctx: &MmArc, uuid: &str, event: MakerSavedEvent) -> Result<(), String> {
    let path = my_swap_file_path(ctx, uuid);
    let content = slurp(&path);
    let swap: SavedSwap = if content.is_empty() {
        SavedSwap::Maker(MakerSavedSwap {
            uuid: uuid.to_owned(),
            events: vec![],
            success_events: vec!["Started".into(), "Negotiated".into(), "TakerFeeValidated".into(),
                                 "MakerPaymentSent".into(), "TakerPaymentReceived".into(),
                                 "TakerPaymentWaitConfirmStarted".into(), "TakerPaymentValidatedAndConfirmed".into(),
                                 "TakerPaymentSpent".into(), "Finished".into()],
            error_events: vec!["StartFailed".into(), "NegotiateFailed".into(), "TakerFeeValidateFailed".into(),
                               "MakerPaymentTransactionFailed".into(), "MakerPaymentDataSendFailed".into(),
                               "TakerPaymentValidateFailed".into(), "TakerPaymentSpendFailed".into(), "MakerPaymentRefunded".into(),
                               "MakerPaymentRefundFailed".into()],
        })
    } else {
        try_s!(json::from_slice(&content))
    };

    if let SavedSwap::Maker(mut maker_swap) = swap {
        maker_swap.events.push(event);
        let new_swap = SavedSwap::Maker(maker_swap);
        let new_content = try_s!(json::to_vec(&new_swap));
        let mut file = try_s!(File::create(path));
        try_s!(file.write_all(&new_content));
        Ok(())
    } else {
        ERR!("Expected SavedSwap::Maker at {}, got {:?}", path.display(), swap)
    }
}

fn save_my_taker_swap_event(ctx: &MmArc, uuid: &str, event: TakerSavedEvent) -> Result<(), String> {
    let path = my_swap_file_path(ctx, uuid);
    let content = slurp(&path);
    let swap: SavedSwap = if content.is_empty() {
        SavedSwap::Taker(TakerSavedSwap {
            uuid: uuid.to_owned(),
            events: vec![],
            success_events: vec!["Started".into(), "Negotiated".into(), "TakerFeeSent".into(),
                                 "MakerPaymentReceived".into(), "MakerPaymentWaitConfirmStarted".into(),
                                 "MakerPaymentValidatedAndConfirmed".into(), "TakerPaymentSent".into(),
                                 "TakerPaymentSpent".into(), "MakerPaymentSpent".into(), "Finished".into()],
            error_events: vec!["StartFailed".into(), "NegotiateFailed".into(), "TakerFeeSendFailed".into(),
                               "MakerPaymentValidateFailed".into(), "TakerPaymentTransactionFailed".into(),
                               "TakerPaymentDataSendFailed".into(), "TakerPaymentWaitForSpendFailed".into(),
                               "MakerPaymentSpendFailed".into(), "TakerPaymentRefunded".into(),
                               "TakerPaymentRefundFailed".into()],
        })
    } else {
        try_s!(json::from_slice(&content))
    };

    if let SavedSwap::Taker(mut taker_swap) = swap {
        taker_swap.events.push(event);
        let new_swap = SavedSwap::Taker(taker_swap);
        let new_content = try_s!(json::to_vec(&new_swap));
        let mut file = try_s!(File::create(path));
        try_s!(file.write_all(&new_content));
        Ok(())
    } else {
        ERR!("Expected SavedSwap::Taker at {}, got {:?}", path.display(), swap)
    }
}

fn save_stats_swap(ctx: &MmArc, swap: &SavedSwap) -> Result<(), String> {
    let (path, content) = match &swap {
        SavedSwap::Maker(maker_swap) => (stats_maker_swap_file_path(ctx, &maker_swap.uuid), try_s!(json::to_vec(&maker_swap))),
        SavedSwap::Taker(taker_swap) => (stats_taker_swap_file_path(ctx, &taker_swap.uuid), try_s!(json::to_vec(&taker_swap))),
    };
    let mut file = try_s!(File::create(path));
    try_s!(file.write_all(&content));
    Ok(())
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
struct MakerSwapData {
    taker_coin: String,
    maker_coin: String,
    taker: H256Json,
    secret: H256Json,
    my_persistent_pub: H264Json,
    lock_duration: u64,
    maker_amount: BigDecimal,
    taker_amount: BigDecimal,
    maker_payment_confirmations: u32,
    taker_payment_confirmations: u32,
    maker_payment_lock: u64,
    /// Allows to recognize one SWAP from the other in the logs. #274.
    uuid: String,
    started_at: u64,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
}

pub struct MakerSwap {
    ctx: MmArc,
    maker_coin: MmCoinEnum,
    taker_coin: MmCoinEnum,
    maker_amount: BigDecimal,
    taker_amount: BigDecimal,
    my_persistent_pub: H264,
    taker: bits256,
    uuid: String,
    data: MakerSwapData,
    taker_payment_lock: u64,
    other_persistent_pub: H264,
    taker_fee: Option<TransactionDetails>,
    maker_payment: Option<TransactionDetails>,
    taker_payment: Option<TransactionDetails>,
    taker_payment_confirmed: bool,
    taker_payment_spend: Option<TransactionDetails>,
    maker_payment_refund: Option<TransactionDetails>,
    errors: Vec<SwapError>,
    finished_at: u64,
}

pub enum MakerSwapCommand {
    Start,
    Negotiate,
    WaitForTakerFee,
    SendPayment,
    WaitForTakerPayment,
    ValidateTakerPayment,
    SpendTakerPayment,
    RefundMakerPayment,
    Finish
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "data")]
enum MakerSwapEvent {
    Started(MakerSwapData),
    StartFailed(SwapError),
    Negotiated(TakerNegotiationData),
    NegotiateFailed(SwapError),
    TakerFeeValidated(TransactionDetails),
    TakerFeeValidateFailed(SwapError),
    MakerPaymentSent(TransactionDetails),
    MakerPaymentTransactionFailed(SwapError),
    MakerPaymentDataSendFailed(SwapError),
    TakerPaymentReceived(TransactionDetails),
    TakerPaymentWaitConfirmStarted,
    TakerPaymentValidatedAndConfirmed,
    TakerPaymentValidateFailed(SwapError),
    TakerPaymentSpent(TransactionDetails),
    TakerPaymentSpendFailed(SwapError),
    MakerPaymentRefunded(TransactionDetails),
    MakerPaymentRefundFailed(SwapError),
    Finished,
}

impl MakerSwapEvent {
    fn status_str(&self) -> String {
        match self {
            MakerSwapEvent::Started(_) => "Started...".to_owned(),
            MakerSwapEvent::StartFailed(_) => "Start failed...".to_owned(),
            MakerSwapEvent::Negotiated(_) => "Negotiated...".to_owned(),
            MakerSwapEvent::NegotiateFailed(_) => "Negotiate failed...".to_owned(),
            MakerSwapEvent::TakerFeeValidated(_) => "Taker fee validated...".to_owned(),
            MakerSwapEvent::TakerFeeValidateFailed(_) => "Taker fee validate failed...".to_owned(),
            MakerSwapEvent::MakerPaymentSent(_) => "Maker payment sent...".to_owned(),
            MakerSwapEvent::MakerPaymentTransactionFailed(_) => "Maker payment failed...".to_owned(),
            MakerSwapEvent::MakerPaymentDataSendFailed(_) => "Maker payment failed...".to_owned(),
            MakerSwapEvent::TakerPaymentReceived(_) => "Taker payment received...".to_owned(),
            MakerSwapEvent::TakerPaymentWaitConfirmStarted => "Taker payment wait confirm started...".to_owned(),
            MakerSwapEvent::TakerPaymentValidatedAndConfirmed => "Taker payment validated and confirmed...".to_owned(),
            MakerSwapEvent::TakerPaymentValidateFailed(_) => "Taker payment validate failed...".to_owned(),
            MakerSwapEvent::TakerPaymentSpent(_) => "Taker payment spent...".to_owned(),
            MakerSwapEvent::TakerPaymentSpendFailed(_) => "Taker payment spend failed...".to_owned(),
            MakerSwapEvent::MakerPaymentRefunded(_) => "Maker payment refunded...".to_owned(),
            MakerSwapEvent::MakerPaymentRefundFailed(_) => "Maker payment refund failed...".to_owned(),
            MakerSwapEvent::Finished => "Finished".to_owned(),
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct MakerSavedEvent {
    timestamp: u64,
    event: MakerSwapEvent,
}

impl MakerSavedEvent {
    /// next command that must be executed after swap is restored
    fn get_command(&self) -> Option<MakerSwapCommand> {
        match self.event {
            MakerSwapEvent::Started(_) => Some(MakerSwapCommand::Negotiate),
            MakerSwapEvent::StartFailed(_) => Some(MakerSwapCommand::Finish),
            MakerSwapEvent::Negotiated(_) => Some(MakerSwapCommand::WaitForTakerFee),
            MakerSwapEvent::NegotiateFailed(_) => Some(MakerSwapCommand::Finish),
            MakerSwapEvent::TakerFeeValidated(_) => Some(MakerSwapCommand::SendPayment),
            MakerSwapEvent::TakerFeeValidateFailed(_) => Some(MakerSwapCommand::Finish),
            MakerSwapEvent::MakerPaymentSent(_) => Some(MakerSwapCommand::WaitForTakerPayment),
            MakerSwapEvent::MakerPaymentTransactionFailed(_) => Some(MakerSwapCommand::Finish),
            MakerSwapEvent::MakerPaymentDataSendFailed(_) => Some(MakerSwapCommand::RefundMakerPayment),
            MakerSwapEvent::TakerPaymentReceived(_) => Some(MakerSwapCommand::ValidateTakerPayment),
            MakerSwapEvent::TakerPaymentWaitConfirmStarted => Some(MakerSwapCommand::ValidateTakerPayment),
            MakerSwapEvent::TakerPaymentValidatedAndConfirmed => Some(MakerSwapCommand::SpendTakerPayment),
            MakerSwapEvent::TakerPaymentValidateFailed(_) => Some(MakerSwapCommand::RefundMakerPayment),
            MakerSwapEvent::TakerPaymentSpent(_) => Some(MakerSwapCommand::Finish),
            MakerSwapEvent::TakerPaymentSpendFailed(_) => Some(MakerSwapCommand::RefundMakerPayment),
            MakerSwapEvent::MakerPaymentRefunded(_) => Some(MakerSwapCommand::Finish),
            MakerSwapEvent::MakerPaymentRefundFailed(_) => Some(MakerSwapCommand::Finish),
            MakerSwapEvent::Finished => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TakerSavedEvent {
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
            TakerSwapEvent::TakerPaymentSent(_) => Some(TakerSwapCommand::WaitForTakerPaymentSpend),
            TakerSwapEvent::TakerPaymentTransactionFailed(_) => Some(TakerSwapCommand::Finish),
            TakerSwapEvent::TakerPaymentDataSendFailed(_) => Some(TakerSwapCommand::RefundTakerPayment),
            TakerSwapEvent::TakerPaymentSpent(_) => Some(TakerSwapCommand::SpendMakerPayment),
            TakerSwapEvent::TakerPaymentWaitForSpendFailed(_) => Some(TakerSwapCommand::RefundTakerPayment),
            TakerSwapEvent::MakerPaymentSpent(_) => Some(TakerSwapCommand::Finish),
            TakerSwapEvent::MakerPaymentSpendFailed(_) => Some(TakerSwapCommand::RefundTakerPayment),
            TakerSwapEvent::TakerPaymentRefunded(_) => Some(TakerSwapCommand::Finish),
            TakerSwapEvent::TakerPaymentRefundFailed(_) => Some(TakerSwapCommand::Finish),
            TakerSwapEvent::Finished => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct MakerSavedSwap {
    uuid: String,
    events: Vec<MakerSavedEvent>,
    success_events: Vec<String>,
    error_events: Vec<String>,
}

impl MakerSavedSwap {
    fn maker_coin(&self) -> Result<String, String> {
        match self.events.first() {
            Some(event) => match &event.event {
                MakerSwapEvent::Started(data) => Ok(data.maker_coin.clone()),
                _ => ERR!("First swap event must be Started"),
            },
            None => ERR!("Can't get maker coin, events are empty"),
        }
    }

    fn taker_coin(&self) -> Result<String, String> {
        match self.events.first() {
            Some(event) => match &event.event {
                MakerSwapEvent::Started(data) => Ok(data.taker_coin.clone()),
                _ => ERR!("First swap event must be Started"),
            },
            None => ERR!("Can't get maker coin, events are empty"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TakerSavedSwap {
    uuid: String,
    events: Vec<TakerSavedEvent>,
    success_events: Vec<String>,
    error_events: Vec<String>,
}

impl TakerSavedSwap {
    fn maker_coin(&self) -> Result<String, String> {
        match self.events.first() {
            Some(event) => match &event.event {
                TakerSwapEvent::Started(data) => Ok(data.maker_coin.clone()),
                _ => ERR!("First swap event must be Started"),
            },
            None => ERR!("Can't get maker coin, events are empty"),
        }
    }

    fn taker_coin(&self) -> Result<String, String> {
        match self.events.first() {
            Some(event) => match &event.event {
                TakerSwapEvent::Started(data) => Ok(data.taker_coin.clone()),
                _ => ERR!("First swap event must be Started"),
            },
            None => ERR!("Can't get maker coin, events are empty"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum SavedSwap {
    Maker(MakerSavedSwap),
    Taker(TakerSavedSwap),
}

/// The helper structure that makes easier to parse the response for GUI devs
/// They won't have to parse the events themselves handling possible errors, index out of bounds etc.
#[derive(Debug, Serialize, Deserialize)]
struct MySwapInfo {
    my_coin: String,
    other_coin: String,
    my_amount: BigDecimal,
    other_amount: BigDecimal,
    started_at: u64,
}

impl SavedSwap {
    fn is_finished(&self) -> bool {
        match self {
            SavedSwap::Maker(swap) => match swap.events.last() {
                Some(event) => event.event == MakerSwapEvent::Finished,
                None => false,
            }
            SavedSwap::Taker(swap) => match swap.events.last() {
                Some(event) => event.event == TakerSwapEvent::Finished,
                None => false,
            },
        }
    }

    fn uuid(&self) -> &str {
        match self {
            SavedSwap::Maker(swap) => &swap.uuid,
            SavedSwap::Taker(swap) => &swap.uuid,
        }
    }

    fn maker_coin_ticker(&self) -> Result<String, String> {
        match self {
            SavedSwap::Maker(swap) => swap.maker_coin(),
            SavedSwap::Taker(swap) => swap.maker_coin(),
        }
    }

    fn taker_coin_ticker(&self) -> Result<String, String> {
        match self {
            SavedSwap::Maker(swap) => swap.taker_coin(),
            SavedSwap::Taker(swap) => swap.taker_coin(),
        }
    }

    fn get_my_info(&self) -> Option<MySwapInfo> {
        match self {
            SavedSwap::Maker(swap) => {
                match swap.events.first() {
                    Some(event) => match &event.event {
                        MakerSwapEvent::Started(data) => {
                            Some(MySwapInfo {
                                my_coin: data.maker_coin.clone(),
                                other_coin: data.taker_coin.clone(),
                                my_amount: data.maker_amount.clone(),
                                other_amount: data.taker_amount.clone(),
                                started_at: data.started_at,
                            })
                        },
                        _ => None,
                    },
                    None => None,
                }
            },
            SavedSwap::Taker(swap) => match swap.events.first() {
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
            },
        }
    }
}

macro_rules! recv {
    ($selff: ident, $subj: expr, $timeout_sec: expr, $ec: expr, $validator: block) => {
        recv_! ($selff, $subj, $timeout_sec, $ec, $validator)
    };
    // Use this form if there's a sending future to terminate upon receiving the answer.
    ($selff: ident, $sending_f: ident, $subj: expr, $timeout_sec: expr, $ec: expr, $validator: block) => {{
        let payload = recv_! ($selff, $subj, $timeout_sec, $ec, $validator);
        drop ($sending_f);
        payload
    }};
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct TakerNegotiationData {
    taker_payment_locktime: u64,
    taker_pubkey: H264Json,
}

impl MakerSwap {
    fn apply_event(&mut self, event: MakerSwapEvent) -> Result<(), String> {
        match event {
            MakerSwapEvent::Started(data) => self.data = data,
            MakerSwapEvent::StartFailed(err) => self.errors.push(err),
            MakerSwapEvent::Negotiated(data) => {
                self.taker_payment_lock = data.taker_payment_locktime;
                self.other_persistent_pub = data.taker_pubkey.into();
            },
            MakerSwapEvent::NegotiateFailed(err) => self.errors.push(err),
            MakerSwapEvent::TakerFeeValidated(tx) => self.taker_fee = Some(tx),
            MakerSwapEvent::TakerFeeValidateFailed(err) => self.errors.push(err),
            MakerSwapEvent::MakerPaymentSent(tx) => {
                self.maker_payment = Some(tx);
                unlock_amount(&self.ctx, &self.uuid, &self.maker_amount);
            },
            MakerSwapEvent::MakerPaymentTransactionFailed(err) => self.errors.push(err),
            MakerSwapEvent::MakerPaymentDataSendFailed(err) => self.errors.push(err),
            MakerSwapEvent::TakerPaymentReceived(tx) => self.taker_payment = Some(tx),
            MakerSwapEvent::TakerPaymentWaitConfirmStarted => (),
            MakerSwapEvent::TakerPaymentValidatedAndConfirmed => self.taker_payment_confirmed = true,
            MakerSwapEvent::TakerPaymentValidateFailed(err) => self.errors.push(err),
            MakerSwapEvent::TakerPaymentSpent(tx) => self.taker_payment_spend = Some(tx),
            MakerSwapEvent::TakerPaymentSpendFailed(err) => self.errors.push(err),
            MakerSwapEvent::MakerPaymentRefunded(tx) => self.maker_payment_refund = Some(tx),
            MakerSwapEvent::MakerPaymentRefundFailed(err) => self.errors.push(err),
            MakerSwapEvent::Finished => {
                self.finished_at = now_ms() / 1000;
                unlock_amount(&self.ctx, &self.uuid, &self.maker_amount);
            },
        }
        Ok(())
    }

    fn handle_command(&self, command: MakerSwapCommand)
                      -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        match command {
            MakerSwapCommand::Start => self.start(),
            MakerSwapCommand::Negotiate => self.negotiate(),
            MakerSwapCommand::WaitForTakerFee => self.wait_taker_fee(),
            MakerSwapCommand::SendPayment => self.maker_payment(),
            MakerSwapCommand::WaitForTakerPayment => self.wait_for_taker_payment(),
            MakerSwapCommand::ValidateTakerPayment => self.validate_taker_payment(),
            MakerSwapCommand::SpendTakerPayment => self.spend_taker_payment(),
            MakerSwapCommand::RefundMakerPayment => self.refund_maker_payment(),
            MakerSwapCommand::Finish => Ok((None, vec![MakerSwapEvent::Finished])),
        }
    }

    pub fn new(
        ctx: MmArc,
        taker: bits256,
        maker_coin: MmCoinEnum,
        taker_coin: MmCoinEnum,
        maker_amount: BigDecimal,
        taker_amount: BigDecimal,
        my_persistent_pub: H264,
        uuid: String,
    ) -> Self {
        lock_amount(&ctx, uuid.clone(), maker_coin.ticker().into(), maker_amount.clone());
        MakerSwap {
            ctx: ctx.clone(),
            maker_coin,
            taker_coin,
            maker_amount,
            taker_amount,
            my_persistent_pub,
            taker,
            uuid,
            data: MakerSwapData::default(),
            taker_payment_lock: 0,
            other_persistent_pub: H264::default(),
            taker_fee: None,
            maker_payment: None,
            taker_payment: None,
            taker_payment_spend: None,
            maker_payment_refund: None,
            errors: vec![],
            finished_at: 0,
            taker_payment_confirmed: false,
        }
    }

    fn start(&self) -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        let my_balance = match self.maker_coin.my_balance().wait() {
            Ok(balance) => balance,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::StartFailed(ERRL!("!my_balance {}", e).into())],
            ))
        };

        let locked = get_locked_amount_by_other_swaps(&self.ctx, &self.uuid, self.maker_coin.ticker());
        let available = &my_balance - &locked;
        if self.maker_amount > available {
            return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::StartFailed(ERRL!("maker amount {} is larger than available {}, balance {}, locked by other swaps {}",
                    self.maker_amount, available, my_balance, locked
                ).into())],
            ));
        }

        if let Err(e) = self.maker_coin.check_i_have_enough_to_trade(&self.maker_amount, &my_balance, TradeInfo::Maker).wait() {
            return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::StartFailed(ERRL!("!check_i_have_enough_to_trade {}", e).into())],
            ));
        };

        if let Err(e) = self.taker_coin.can_i_spend_other_payment().wait() {
            return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::StartFailed(ERRL!("!can_i_spend_other_payment {}", e).into())],
            ));
        };

        let lock_duration = lp_atomic_locktime(self.maker_coin.ticker(), self.taker_coin.ticker());
        let (maker_payment_confirmations, taker_payment_confirmations) = payment_confirmations(&self.maker_coin, &self.taker_coin);
        let mut rng = rand::thread_rng();
        let secret: [u8; 32] = rng.gen();
        let started_at = now_ms() / 1000;

        let maker_coin_start_block = match self.maker_coin.current_block().wait() {
            Ok(b) => b,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::StartFailed(ERRL!("!maker_coin.current_block {}", e).into())],
            ))
        };

        let taker_coin_start_block = match self.taker_coin.current_block().wait() {
            Ok(b) => b,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::StartFailed(ERRL!("!taker_coin.current_block {}", e).into())],
            ))
        };

        let data = MakerSwapData {
            taker_coin: self.taker_coin.ticker().to_owned(),
            maker_coin: self.maker_coin.ticker().to_owned(),
            taker: self.taker.bytes.into(),
            secret: secret.into(),
            started_at,
            lock_duration,
            maker_amount: self.maker_amount.clone(),
            taker_amount: self.taker_amount.clone(),
            maker_payment_confirmations,
            taker_payment_confirmations,
            maker_payment_lock: started_at + lock_duration * 2,
            my_persistent_pub: self.my_persistent_pub.clone().into(),
            uuid: self.uuid.clone(),
            maker_coin_start_block,
            taker_coin_start_block,
        };

        Ok((Some(MakerSwapCommand::Negotiate), vec![MakerSwapEvent::Started(data)]))
    }

    fn negotiate(&self) -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        let maker_negotiation_data = SwapNegotiationData {
            started_at: self.data.started_at,
            payment_locktime: self.data.maker_payment_lock,
            secret_hash: dhash160(&self.data.secret.0),
            persistent_pubkey: self.my_persistent_pub.clone(),
        };

        let bytes = serialize(&maker_negotiation_data);
        let sending_f = match send!(&self.ctx, self.taker, fomat!(("negotiation") '@' (self.uuid)), 30, bytes.as_slice()) {
            Ok(f) => f,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::NegotiateFailed(ERRL!("{}", e).into())],
            )),
        };

        let data = match recv!(self, sending_f, "negotiation-reply", 90, -2000, {|_: &[u8]| Ok(())}) {
            Ok(d) => d,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::NegotiateFailed(ERRL!("{:?}", e).into())],
            )),
        };
        let taker_data: SwapNegotiationData = match deserialize(data.as_slice()) {
            Ok(d) => d,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::NegotiateFailed(ERRL!("{:?}", e).into())],
            )),
        };
        let time_dif = (self.data.started_at as i64 - taker_data.started_at as i64).abs();
        if  time_dif > 60 {
            return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::NegotiateFailed(ERRL!("Started_at time_dif over 60 {}", time_dif).into())]
            ))
        }

        let expected_lock_time = taker_data.started_at + self.data.lock_duration;
        if taker_data.payment_locktime != expected_lock_time {
            return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::NegotiateFailed(ERRL!("taker_data.payment_locktime {} not equal to expected {}", taker_data.payment_locktime, expected_lock_time).into())]
            ))
        }

        Ok((
            Some(MakerSwapCommand::WaitForTakerFee),
            vec![MakerSwapEvent::Negotiated(
                TakerNegotiationData {
                    taker_payment_locktime: taker_data.payment_locktime,
                    taker_pubkey: taker_data.persistent_pubkey.into(),
                })
            ],
        ))
    }

    fn wait_taker_fee(&self) -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        let negotiated = serialize(&true);
        let sending_f = match send!(&self.ctx, self.taker, fomat!(("negotiated") '@' (self.uuid)), 30, negotiated.as_slice()) {
            Ok(f) => f,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::NegotiateFailed(ERRL!("{}", e).into())],
            )),
        };

        let payload = match recv!(self, sending_f, "taker-fee", 600, -2003, {|_: &[u8]| Ok(())}) {
            Ok(d) => d,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::TakerFeeValidateFailed(ERRL!("{}", e).into())]
            ))
        };
        let taker_fee = match self.taker_coin.tx_enum_from_bytes(&payload) {
            Ok(tx) => tx,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::TakerFeeValidateFailed(ERRL!("{}", e).into())]
            ))
        };

        let hash = taker_fee.tx_hash();
        log!({ "Taker fee tx {:02x}", hash });

        let fee_addr_pub_key = unwrap!(hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06"));
        let fee_amount = dex_fee_amount(&self.data.maker_coin, &self.data.taker_coin, &self.taker_amount);

        let mut attempts = 0;
        loop {
            match self.taker_coin.validate_fee(&taker_fee, &fee_addr_pub_key, &fee_amount) {
                Ok(_) => break,
                Err(err) => if attempts >= 3 {
                    return Ok((
                        Some(MakerSwapCommand::Finish),
                        vec![MakerSwapEvent::TakerFeeValidateFailed(ERRL!("{}", err).into())]
                    ))
                } else {
                    attempts += 1;
                    thread::sleep(Duration::from_secs(10));
                }
            };
        };

        let mut attempts = 0;
        let fee_details = loop {
            match self.taker_coin.tx_details_by_hash(&hash) {
                Ok(details) => break details,
                Err(err) => if attempts >= 3 {
                    return Ok((
                        Some(MakerSwapCommand::Finish),
                        vec![MakerSwapEvent::TakerFeeValidateFailed(ERRL!("Taker fee tx_details_by_hash failed {}", err).into())]
                    ))
                } else {
                    attempts += 1;
                    thread::sleep(Duration::from_secs(10));
                }
            };
        };

        Ok((
            Some(MakerSwapCommand::SendPayment),
            vec![MakerSwapEvent::TakerFeeValidated(fee_details)]
        ))
    }

    fn maker_payment(&self) -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        let timeout = self.data.started_at + self.data.lock_duration / 3;
        let now = now_ms() / 1000;
        if now > timeout {
            return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::MakerPaymentTransactionFailed(ERRL!("Timeout {} > {}", now, timeout).into())],
            ));
        }

        let transaction = match self.maker_coin.check_if_my_payment_sent(
            self.data.maker_payment_lock as u32,
            &*self.other_persistent_pub,
            &*dhash160(&self.data.secret.0),
            self.data.maker_coin_start_block,
        ) {
            Ok(res) => match res {
                Some(tx) => tx,
                None => {
                    let payment_fut = self.maker_coin.send_maker_payment(
                        self.data.maker_payment_lock as u32,
                        &*self.other_persistent_pub,
                        &*dhash160(&self.data.secret.0),
                        self.maker_amount.clone(),
                    );

                    match payment_fut.wait() {
                        Ok(t) => t,
                        Err(err) => return Ok((
                            Some(MakerSwapCommand::Finish),
                            vec![MakerSwapEvent::MakerPaymentTransactionFailed(ERRL!("{}", err).into())],
                        ))
                    }
                }
            },
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::MakerPaymentTransactionFailed(ERRL!("{}", e).into())],
            ))
        };

        let hash = transaction.tx_hash();
        log!({ "Maker payment tx {:02x}", hash });
        // we can attempt to get the details in loop here as transaction was already sent and
        // is present on blockchain so only transport errors are expected to happen
        let tx_details = loop {
            match self.maker_coin.tx_details_by_hash(&hash) {
                Ok(details) => break details,
                Err(e) => {
                    log!({"Error {} getting tx details of {:02x}", e, hash});
                    thread::sleep(Duration::from_secs(30));
                    continue;
                }
            }
        };

        Ok((
            Some(MakerSwapCommand::WaitForTakerPayment),
            vec![MakerSwapEvent::MakerPaymentSent(tx_details)]
        ))
    }

    fn wait_for_taker_payment(&self) -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        let maker_payment_hex = self.maker_payment.as_ref().unwrap().tx_hex.clone();
        let sending_f = match send!(&self.ctx, self.taker, fomat!(("maker-payment") '@' (self.uuid)), 60, maker_payment_hex) {
            Ok(f) => f,
            Err(e) => return Ok((
                Some(MakerSwapCommand::RefundMakerPayment),
                vec![MakerSwapEvent::MakerPaymentDataSendFailed(ERRL!("{}", e).into())]
            ))
        };

        let wait_duration = self.data.lock_duration / 3;
        let payload = match recv!(self, sending_f, "taker-payment", wait_duration, -2006, {|_: &[u8]| Ok(())}) {
            Ok(p) => p,
            Err(e) => return Ok((
                Some(MakerSwapCommand::RefundMakerPayment),
                vec![MakerSwapEvent::TakerPaymentValidateFailed(e.into())],
            ))
        };

        let taker_payment = match self.taker_coin.tx_enum_from_bytes(&payload) {
            Ok(tx) => tx,
            Err(err) => return Ok((
                Some(MakerSwapCommand::RefundMakerPayment),
                vec![MakerSwapEvent::TakerPaymentValidateFailed(ERRL!("!taker_coin.tx_enum_from_bytes: {}", err).into())]
            )),
        };

        let hash = taker_payment.tx_hash();
        log!({ "Taker payment tx {:02x}", hash });
        let mut attempts = 0;
        let tx_details = loop {
            match self.taker_coin.tx_details_by_hash(&hash) {
                Ok(details) => break details,
                Err(err) => if attempts >= 3 {
                    return Ok((
                        Some(MakerSwapCommand::RefundMakerPayment),
                        vec![MakerSwapEvent::TakerPaymentValidateFailed(ERRL!("!taker_coin.tx_details_by_hash: {}", err).into())]
                    ))
                } else {
                    attempts += 1;
                    thread::sleep(Duration::from_secs(10));
                }
            };
        };

        Ok((
            Some(MakerSwapCommand::ValidateTakerPayment),
            vec![MakerSwapEvent::TakerPaymentReceived(tx_details), MakerSwapEvent::TakerPaymentWaitConfirmStarted]
        ))
    }

    fn validate_taker_payment(&self) -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        let wait_duration = self.data.lock_duration / 3;
        let wait_taker_payment = self.data.started_at + wait_duration;

        let validated = self.taker_coin.validate_taker_payment(
            &unwrap!(self.taker_payment.clone()).tx_hex,
            self.taker_payment_lock as u32,
            &*self.other_persistent_pub,
            &*dhash160(&self.data.secret.0),
            self.taker_amount.clone(),
        );

        if let Err(e) = validated {
            return Ok((
                Some(MakerSwapCommand::RefundMakerPayment),
                vec![MakerSwapEvent::TakerPaymentValidateFailed(ERRL!("!taker_coin.validate_taker_payment: {}", e).into())]
            ))
        }

        let wait = self.taker_coin.wait_for_confirmations(
            &unwrap!(self.taker_payment.clone()).tx_hex,
            self.data.taker_payment_confirmations,
            wait_taker_payment,
        );

        if let Err(err) = wait {
            return Ok((
                Some(MakerSwapCommand::RefundMakerPayment),
                vec![MakerSwapEvent::TakerPaymentValidateFailed(ERRL!("!taker_coin.wait_for_confirmations: {}", err).into())]
            ))
        }

        Ok((
            Some(MakerSwapCommand::SpendTakerPayment),
            vec![MakerSwapEvent::TakerPaymentValidatedAndConfirmed]
        ))
    }

    fn spend_taker_payment(&self) -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        let spend_fut = self.taker_coin.send_maker_spends_taker_payment(
            &unwrap!(self.taker_payment.clone()).tx_hex,
            self.taker_payment_lock as u32,
            &*self.other_persistent_pub,
            &self.data.secret.0,
        );

        let transaction = match spend_fut.wait() {
            Ok(t) => t,
            Err(err) => return Ok((
                Some(MakerSwapCommand::RefundMakerPayment),
                vec![MakerSwapEvent::TakerPaymentSpendFailed(ERRL!("!taker_coin.send_maker_spends_taker_payment: {}", err).into())]
            ))
        };

        let hash = transaction.tx_hash();
        log!({ "Taker payment spend tx {:02x}", hash });

        // we can attempt to get the details in loop here as transaction was already sent and
        // is present on blockchain so only transport errors are expected to happen
        let tx_details = loop {
            match self.taker_coin.tx_details_by_hash(&hash) {
                Ok(details) => break details,
                Err(e) => {
                    log!({"Error {} getting tx details of {:02x}", e, hash});
                    thread::sleep(Duration::from_secs(30));
                    continue;
                }
            }
        };
        Ok((
            Some(MakerSwapCommand::Finish),
            vec![MakerSwapEvent::TakerPaymentSpent(tx_details)]
        ))
    }

    fn refund_maker_payment(&self) -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        // have to wait for 1 hour more due as some coins have BIP113 activated so these will reject transactions with locktime == present time
        // https://github.com/bitcoin/bitcoin/blob/master/doc/release-notes/release-notes-0.11.2.md#bip113-mempool-only-locktime-enforcement-using-getmediantimepast
        while now_ms() / 1000 < self.data.maker_payment_lock + 3700 {
            std::thread::sleep(Duration::from_secs(10));
        }

        let spend_fut = self.maker_coin.send_maker_refunds_payment(
            &unwrap!(self.maker_payment.clone()).tx_hex,
            self.data.maker_payment_lock as u32,
            &*self.other_persistent_pub,
            &*dhash160(&self.data.secret.0),
        );

        let transaction = match spend_fut.wait() {
            Ok(t) => t,
            Err(err) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::MakerPaymentRefundFailed(ERRL!("!maker_coin.send_maker_refunds_payment: {}", err).into())]
            ))
        };
        let hash = transaction.tx_hash();
        log!({ "Maker payment refund tx {:02x}", hash });

        // we can attempt to get the details in loop here as transaction was already sent and
        // is present on blockchain so only transport errors are expected to happen
        let tx_details = loop {
            match self.maker_coin.tx_details_by_hash(&hash) {
                Ok(details) => break details,
                Err(e) => {
                    log!({"Error {} getting tx details of {:02x}", e, hash});
                    thread::sleep(Duration::from_secs(30));
                    continue;
                }
            }
        };
        Ok((
            Some(MakerSwapCommand::Finish),
            vec![MakerSwapEvent::MakerPaymentRefunded(tx_details)],
        ))
    }

    fn load_from_saved(ctx: MmArc, saved: MakerSavedSwap) -> Result<(Self, Option<MakerSwapCommand>), String> {
        if saved.events.is_empty() {
            return ERR!("Can't restore swap from empty events set");
        };

        match &saved.events[0].event {
            MakerSwapEvent::Started(data) => {
                let mut taker = bits256::from([0; 32]);
                taker.bytes = data.taker.0;
                let mut taker_coin = None;
                while taker_coin.is_none() {
                    thread::sleep(Duration::from_secs(5));
                    log!("Can't kickstart the swap " (saved.uuid) " until the coin " (data.taker_coin) " is activated");
                    taker_coin = try_s!(lp_coinfind(&ctx, &data.taker_coin));
                };

                let mut maker_coin = None;
                while maker_coin.is_none() {
                    thread::sleep(Duration::from_secs(5));
                    log!("Can't kickstart the swap " (saved.uuid) " until the coin " (data.maker_coin) " is activated");
                    maker_coin = try_s!(lp_coinfind(&ctx, &data.maker_coin));
                };
                let my_persistent_pub = unsafe { unwrap!(compressed_pub_key_from_priv_raw(&lp::G.LP_privkey.bytes, ChecksumType::DSHA256)) };

                let mut swap = MakerSwap::new(
                    ctx,
                    taker.into(),
                    maker_coin.unwrap(),
                    taker_coin.unwrap(),
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
}

impl Drop for MakerSwap {
    fn drop(&mut self) {
        unlock_amount(&self.ctx, &self.uuid, &self.maker_amount);
    }
}

/// Starts the maker swap and drives it to completion (until None next command received).
/// Panics in case of command or event apply fails, not sure yet how to handle such situations
/// because it's usually means that swap is in invalid state which is possible only if there's developer error.
/// Every produced event is saved to local DB. Swap status is broadcasted to P2P network after completion.
pub fn run_maker_swap(mut swap: MakerSwap, initial_command: Option<MakerSwapCommand>) {
    let mut command = initial_command.unwrap_or(MakerSwapCommand::Start);
    let mut events;
    let ctx = swap.ctx.clone();
    let mut status = ctx.log.status_handle();
    let uuid = swap.uuid.clone();
    let swap_tags: &[&dyn TagParam] = &[&"swap", &("uuid", &uuid[..])];
    loop {
        let res = unwrap!(swap.handle_command(command));
        events = res.1;
        for event in events {
            let to_save = MakerSavedEvent {
                timestamp: now_ms(),
                event: event.clone(),
            };
            unwrap!(save_my_maker_swap_event(&ctx, &swap.uuid, to_save));
            status.status(swap_tags, &event.status_str());
            unwrap!(swap.apply_event(event));
        }
        match res.0 {
            Some(c) => { command = c; },
            None => {
                if let Err(e) = broadcast_my_swap_status(&swap.uuid, &swap.ctx) {
                    log!("!broadcast_my_swap_status(" (swap.uuid) "): " (e));
                }
                break;
            },
        }
    }
}

/// Starts the taker swap and drives it to completion (until None next command received).
/// Panics in case of command or event apply fails, not sure yet how to handle such situations
/// because it's usually means that swap is in invalid state which is possible only if there's developer error
/// Every produced event is saved to local DB. Swap status is broadcasted to P2P network after completion.
pub fn run_taker_swap(mut swap: TakerSwap, initial_command: Option<TakerSwapCommand>) {
    let mut command = initial_command.unwrap_or(TakerSwapCommand::Start);
    let mut events;
    let ctx = swap.ctx.clone();
    let mut status = ctx.log.status_handle();
    let uuid = swap.uuid.clone();
    let swap_tags: &[&dyn TagParam] = &[&"swap", &("uuid", &uuid[..])];
    loop {
        let res = unwrap!(swap.handle_command(command));
        events = res.1;
        for event in events {
            let to_save = TakerSavedEvent {
                timestamp: now_ms(),
                event: event.clone(),
            };
            unwrap!(save_my_taker_swap_event(&ctx, &swap.uuid, to_save));
            status.status(swap_tags, &event.status_str());
            unwrap!(swap.apply_event(event));
        }
        match res.0 {
            Some(c) => { command = c; },
            None => {
                if let Err(e) = broadcast_my_swap_status(&swap.uuid, &swap.ctx) {
                    log!("!broadcast_my_swap_status(" (swap.uuid) "): " (e));
                }
                break;
            },
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
struct TakerSwapData {
    taker_coin: String,
    maker_coin: String,
    maker: H256Json,
    my_persistent_pub: H264Json,
    lock_duration: u64,
    maker_amount: BigDecimal,
    taker_amount: BigDecimal,
    maker_payment_confirmations: u32,
    taker_payment_confirmations: u32,
    taker_payment_lock: u64,
    /// Allows to recognize one SWAP from the other in the logs. #274.
    uuid: String,
    started_at: u64,
    maker_payment_wait: u64,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct MakerNegotiationData {
    maker_payment_locktime: u64,
    maker_pubkey: H264Json,
    secret_hash: H160Json,
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
    data: TakerSwapData,
    maker_payment_lock: u64,
    other_persistent_pub: H264,
    taker_fee: Option<TransactionDetails>,
    maker_payment: Option<TransactionDetails>,
    maker_payment_confirmed: bool,
    taker_payment: Option<TransactionDetails>,
    taker_payment_spend: Option<TransactionDetails>,
    maker_payment_spend: Option<TransactionDetails>,
    taker_payment_refund: Option<TransactionDetails>,
    errors: Vec<SwapError>,
    finished_at: u64,
    secret_hash: H160Json,
    secret: H256Json,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct SwapError {
    error: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TakerPaymentSpentData {
    transaction: TransactionDetails,
    secret: H256Json,
}

impl Into<SwapError> for String {
    fn into(self) -> SwapError {
        SwapError {
            error: self
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "data")]
enum TakerSwapEvent {
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
    TakerPaymentSent(TransactionDetails),
    TakerPaymentTransactionFailed(SwapError),
    TakerPaymentDataSendFailed(SwapError),
    TakerPaymentSpent(TakerPaymentSpentData),
    TakerPaymentWaitForSpendFailed(SwapError),
    MakerPaymentSpent(TransactionDetails),
    MakerPaymentSpendFailed(SwapError),
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
            TakerSwapEvent::TakerPaymentSent(_) => "Taker payment sent...".to_owned(),
            TakerSwapEvent::TakerPaymentTransactionFailed(_) => "Taker payment transaction failed...".to_owned(),
            TakerSwapEvent::TakerPaymentDataSendFailed(_) => "Taker payment data send failed...".to_owned(),
            TakerSwapEvent::TakerPaymentSpent(_) => "Taker payment spent...".to_owned(),
            TakerSwapEvent::TakerPaymentWaitForSpendFailed(_) => "Taker payment wait for spend failed...".to_owned(),
            TakerSwapEvent::MakerPaymentSpent(_) => "Maker payment spent...".to_owned(),
            TakerSwapEvent::MakerPaymentSpendFailed(_) => "Maker payment spend failed...".to_owned(),
            TakerSwapEvent::TakerPaymentRefunded(_) => "Taker payment refunded...".to_owned(),
            TakerSwapEvent::TakerPaymentRefundFailed(_) => "Taker payment refund failed...".to_owned(),
            TakerSwapEvent::Finished => "Finished".to_owned(),
        }
    }
}

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
    fn apply_event(&mut self, event: TakerSwapEvent) -> Result<(), String> {
        match event {
            TakerSwapEvent::Started(data) => self.data = data,
            TakerSwapEvent::StartFailed(err) => self.errors.push(err),
            TakerSwapEvent::Negotiated(data) => {
                self.maker_payment_lock = data.maker_payment_locktime;
                self.other_persistent_pub = data.maker_pubkey.into();
                self.secret_hash = data.secret_hash;
            },
            TakerSwapEvent::NegotiateFailed(err) => self.errors.push(err),
            TakerSwapEvent::TakerFeeSent(tx) => {
                self.taker_fee = Some(tx);
                let fee_amount = &self.taker_amount / 777;
                unlock_amount(&self.ctx, &self.uuid, &fee_amount);
            },
            TakerSwapEvent::TakerFeeSendFailed(err) => self.errors.push(err),
            TakerSwapEvent::MakerPaymentReceived(tx) => self.maker_payment = Some(tx),
            TakerSwapEvent::MakerPaymentWaitConfirmStarted => (),
            TakerSwapEvent::MakerPaymentValidatedAndConfirmed => self.maker_payment_confirmed = true,
            TakerSwapEvent::MakerPaymentValidateFailed(err) => self.errors.push(err),
            TakerSwapEvent::TakerPaymentSent(tx) => {
                self.taker_payment = Some(tx);
                unlock_amount(&self.ctx, &self.uuid, &self.taker_amount);
            },
            TakerSwapEvent::TakerPaymentTransactionFailed(err) => self.errors.push(err),
            TakerSwapEvent::TakerPaymentDataSendFailed(err) => self.errors.push(err),
            TakerSwapEvent::TakerPaymentSpent(data) => {
                self.taker_payment_spend = Some(data.transaction);
                self.secret = data.secret;
            },
            TakerSwapEvent::TakerPaymentWaitForSpendFailed(err) => self.errors.push(err),
            TakerSwapEvent::MakerPaymentSpent(tx) => self.maker_payment_spend = Some(tx),
            TakerSwapEvent::MakerPaymentSpendFailed(err) => self.errors.push(err),
            TakerSwapEvent::TakerPaymentRefunded(tx) => self.taker_payment_refund = Some(tx),
            TakerSwapEvent::TakerPaymentRefundFailed(err) => self.errors.push(err),
            TakerSwapEvent::Finished => self.finished_at = now_ms() / 1000,
        }
        Ok(())
    }

    fn handle_command(&self, command: TakerSwapCommand)
                      -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        match command {
            TakerSwapCommand::Start => self.start(),
            TakerSwapCommand::Negotiate => self.negotiate(),
            TakerSwapCommand::SendTakerFee => self.send_taker_fee(),
            TakerSwapCommand::WaitForMakerPayment => self.wait_for_maker_payment(),
            TakerSwapCommand::ValidateMakerPayment => self.validate_maker_payment(),
            TakerSwapCommand::SendTakerPayment => self.send_taker_payment(),
            TakerSwapCommand::WaitForTakerPaymentSpend => self.wait_for_taker_payment_spend(),
            TakerSwapCommand::SpendMakerPayment => self.spend_maker_payment(),
            TakerSwapCommand::RefundTakerPayment => self.refund_taker_payment(),
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
        lock_amount(&ctx, uuid.clone(), taker_coin.ticker().into(), &taker_amount + &taker_amount / 777);
        TakerSwap {
            ctx,
            maker_coin,
            taker_coin,
            maker_amount,
            taker_amount,
            my_persistent_pub,
            maker,
            uuid,
            data: TakerSwapData::default(),
            other_persistent_pub: H264::default(),
            taker_fee: None,
            maker_payment: None,
            maker_payment_confirmed: false,
            taker_payment: None,
            taker_payment_spend: None,
            maker_payment_spend: None,
            taker_payment_refund: None,
            finished_at: 0,
            maker_payment_lock: 0,
            errors: vec![],
            secret_hash: H160Json::default(),
            secret: H256Json::default(),
        }
    }

    fn start(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let my_balance = match self.taker_coin.my_balance().wait() {
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
        if let Err(e) = self.taker_coin.check_i_have_enough_to_trade(&self.taker_amount, &my_balance, TradeInfo::Taker(dex_fee_amount)).wait() {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::StartFailed(ERRL!("!check_i_have_enough_to_trade {}", e).into())],
            ))
        }

        if let Err(e) = self.maker_coin.can_i_spend_other_payment().wait() {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::StartFailed(ERRL!("!can_i_spend_other_payment {}", e).into())],
            ));
        };

        let lock_duration = lp_atomic_locktime(self.maker_coin.ticker(), self.taker_coin.ticker());
        let (maker_payment_confirmations, taker_payment_confirmations) = payment_confirmations(&self.maker_coin, &self.taker_coin);
        let started_at = now_ms() / 1000;

        let maker_coin_start_block = match self.maker_coin.current_block().wait() {
            Ok(b) => b,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::StartFailed(ERRL!("!maker_coin.current_block {}", e).into())],
            ))
        };

        let taker_coin_start_block = match self.taker_coin.current_block().wait() {
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
            maker_payment_confirmations,
            taker_payment_confirmations,
            taker_payment_lock: started_at + lock_duration,
            my_persistent_pub: self.my_persistent_pub.clone().into(),
            uuid: self.uuid.clone(),
            maker_payment_wait: started_at + lock_duration / 3,
            maker_coin_start_block,
            taker_coin_start_block,
        };

        Ok((Some(TakerSwapCommand::Negotiate), vec![TakerSwapEvent::Started(data)]))
    }

    fn negotiate(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let data = match recv!(self, "negotiation", 90, -1000, {|_: &[u8]| Ok(())}) {
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

        let time_dif = (self.data.started_at as i64 - maker_data.started_at as i64).abs();
        if  time_dif > 60 {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("Started_at time_dif over 60 {}", time_dif).into())]
            ))
        }

        let expected_lock_time = maker_data.started_at + self.data.lock_duration * 2;
        if maker_data.payment_locktime != expected_lock_time {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("maker_data.payment_locktime {} not equal to expected {}", maker_data.payment_locktime, expected_lock_time).into())]
            ))
        }

        let taker_data = SwapNegotiationData {
            started_at: self.data.started_at,
            secret_hash: maker_data.secret_hash.clone(),
            payment_locktime: self.data.taker_payment_lock,
            persistent_pubkey: self.my_persistent_pub.clone(),
        };
        let bytes = serialize(&taker_data);
        let sending_f = match send! (&self.ctx, self.maker, fomat!(("negotiation-reply") '@' (self.uuid)), 30, bytes.as_slice()) {
            Ok(f) => f,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("{}", e).into())]
            )),
        };
        let data = match recv!(self, sending_f, "negotiated", 90, -1000, {|_: &[u8]| Ok(())}) {
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

    fn send_taker_fee(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let timeout = self.data.started_at + self.data.lock_duration / 3;
        let now = now_ms() / 1000;
        if now > timeout {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerFeeSendFailed(ERRL!("Timeout {} > {}", now, timeout).into())],
            ));
        }

        let fee_addr_pub_key = unwrap!(hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06"));
        let fee_amount = dex_fee_amount(&self.data.maker_coin, &self.data.taker_coin, &self.taker_amount);
        let fee_tx = self.taker_coin.send_taker_fee(&fee_addr_pub_key, fee_amount).wait();
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
            match self.taker_coin.tx_details_by_hash(&hash) {
                Ok(details) => break details,
                Err(e) => {
                    log!({"Error {} getting tx details of {:02x}", e, hash});
                    thread::sleep(Duration::from_secs(30));
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

    fn wait_for_maker_payment(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let tx_hex = self.taker_fee.as_ref().unwrap().tx_hex.clone();
        let sending_f = match send! (&self.ctx, self.maker, fomat!(("taker-fee") '@' (self.uuid)), 60, tx_hex) {
            Ok(f) => f,
            Err (err) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerFeeSendFailed(ERRL!("{}", err).into())]
            )),
        };
        let payload = match recv!(self, sending_f, "maker-payment", 600, -1005, {|_: &[u8]| Ok(())}) {
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
        let tx_details = loop {
            match self.maker_coin.tx_details_by_hash(&hash) {
                Ok(details) => break details,
                Err(e) => if attempts >= 3 {
                    return Ok((
                        Some(TakerSwapCommand::Finish),
                        vec![TakerSwapEvent::MakerPaymentValidateFailed(ERRL!("!maker_coin.tx_details_by_hash: {}", e).into())]
                    ))
                } else {
                    attempts += 1;
                    thread::sleep(Duration::from_secs(10));
                },
            };
        };

        Ok((
            Some(TakerSwapCommand::ValidateMakerPayment),
            vec![TakerSwapEvent::MakerPaymentReceived(tx_details), TakerSwapEvent::MakerPaymentWaitConfirmStarted]
        ))
    }

    fn validate_maker_payment(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let validated = self.maker_coin.validate_maker_payment(
            &unwrap!(self.maker_payment.clone()).tx_hex,
            self.maker_payment_lock as u32,
            &*self.other_persistent_pub,
            &self.secret_hash.0,
            self.maker_amount.clone(),
        );

        if let Err(e) = validated {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::MakerPaymentValidateFailed(ERRL!("!validate maker payment: {}", e).into())]
            ));
        }

        if let Err(err) = self.maker_coin.wait_for_confirmations(
            &unwrap!(self.maker_payment.clone()).tx_hex,
            self.data.maker_payment_confirmations,
            self.data.maker_payment_wait,
        ) {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::MakerPaymentValidateFailed(ERRL!("!wait for maker payment confirmations: {}", err).into())]
            ));
        }

        Ok((
            Some(TakerSwapCommand::SendTakerPayment),
            vec![TakerSwapEvent::MakerPaymentValidatedAndConfirmed]
        ))
    }


    fn send_taker_payment(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let timeout = self.data.started_at + self.data.lock_duration / 3;
        let now = now_ms() / 1000;
        if now > timeout {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerPaymentTransactionFailed(ERRL!("Timeout {} > {}", now, timeout).into())],
            ));
        }

        let transaction = match self.taker_coin.check_if_my_payment_sent(
            self.data.taker_payment_lock as u32,
            &*self.other_persistent_pub,
            &self.secret_hash.0,
            self.data.taker_coin_start_block,
        ) {
            Ok(res) => match res {
                Some(tx) => tx,
                None => {
                    let payment_fut = self.taker_coin.send_taker_payment(
                        self.data.taker_payment_lock as u32,
                        &*self.other_persistent_pub,
                        &self.secret_hash.0,
                        self.taker_amount.clone(),
                    );

                    match payment_fut.wait() {
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
            match self.taker_coin.tx_details_by_hash(&hash) {
                Ok(details) => break details,
                Err(e) => {
                    log!({"Error {} getting tx details of {:02x}", e, hash});
                    thread::sleep(Duration::from_secs(30));
                    continue;
                }
            }
        };

        Ok((
            Some(TakerSwapCommand::WaitForTakerPaymentSpend),
            vec![TakerSwapEvent::TakerPaymentSent(tx_details)],
        ))
    }

    fn wait_for_taker_payment_spend(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let tx_hex = self.taker_payment.as_ref().unwrap().tx_hex.clone();
        let sending_f = match send! (&self.ctx, self.maker, fomat!(("taker-payment") '@' (self.uuid)), 60, tx_hex) {
            Ok(f) => f,
            Err(e) => return Ok((
                Some(TakerSwapCommand::RefundTakerPayment),
                vec![TakerSwapEvent::TakerPaymentDataSendFailed(e.into())]
            ))
        };

        let tx = match self.taker_coin.wait_for_tx_spend(&self.taker_payment.clone().unwrap().tx_hex, self.data.taker_payment_lock, self.data.taker_coin_start_block) {
            Ok(t) => t,
            Err(e) => return Ok((
                Some(TakerSwapCommand::RefundTakerPayment),
                vec![TakerSwapEvent::TakerPaymentWaitForSpendFailed(e.into())],
            ))
        };
        drop(sending_f);
        let hash = tx.tx_hash();
        log!({"Taker payment spend tx {:02x}", hash});
        // we can attempt to get the details in loop here as transaction was already sent and
        // is present on blockchain so only transport errors are expected to happen
        let tx_details = loop {
            match self.taker_coin.tx_details_by_hash(&hash) {
                Ok(details) => break details,
                Err(e) => {
                    log!({"Error {} getting tx details of {:02x}", e, hash});
                    thread::sleep(Duration::from_secs(30));
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

    fn spend_maker_payment(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let spend_fut = self.maker_coin.send_taker_spends_maker_payment(
            &unwrap!(self.maker_payment.clone()).tx_hex,
            self.maker_payment_lock as u32,
            &*self.other_persistent_pub,
            &self.secret.0,
        );

        let transaction = match spend_fut.wait() {
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
            match self.maker_coin.tx_details_by_hash(&hash) {
                Ok(details) => break details,
                Err(e) => {
                    log!({"Error {} getting tx details of {:02x}", e, hash});
                    thread::sleep(Duration::from_secs(30));
                    continue;
                }
            }
        };
        Ok((
            Some(TakerSwapCommand::Finish),
            vec![TakerSwapEvent::MakerPaymentSpent(tx_details)],
        ))
    }

    fn refund_taker_payment(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        loop {
            // have to wait for 1 hour more because some coins have BIP113 activated so these will reject transactions with locktime == present time
            // https://github.com/bitcoin/bitcoin/blob/master/doc/release-notes/release-notes-0.11.2.md#bip113-mempool-only-locktime-enforcement-using-getmediantimepast
            if now_ms() / 1000 > self.data.taker_payment_lock + 3700 {
                break;
            }
            std::thread::sleep(Duration::from_secs(10));
        }
        let refund_fut = self.taker_coin.send_taker_refunds_payment(
            &self.taker_payment.clone().unwrap().tx_hex.0,
            self.data.taker_payment_lock as u32,
            &*self.other_persistent_pub,
            &self.secret_hash.0,
        );

        let transaction = match refund_fut.wait() {
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
            match self.taker_coin.tx_details_by_hash(&hash) {
                Ok(details) => break details,
                Err(e) => {
                    log!({"Error {} getting tx details of {:02x}", e, hash});
                    thread::sleep(Duration::from_secs(30));
                    continue;
                }
            }
        };
        Ok((
            Some(TakerSwapCommand::Finish),
            vec![TakerSwapEvent::TakerPaymentRefunded(tx_details)],
        ))
    }

    fn load_from_saved(ctx: MmArc, saved: TakerSavedSwap) -> Result<(Self, Option<TakerSwapCommand>), String> {
        if saved.events.is_empty() {
            return ERR!("Can't restore swap from empty events set");
        };

        match &saved.events[0].event {
            TakerSwapEvent::Started(data) => {
                let mut maker = bits256::from([0; 32]);
                maker.bytes = data.maker.0;
                let mut taker_coin = None;
                while taker_coin.is_none() {
                    thread::sleep(Duration::from_secs(5));
                    log!("Can't kickstart the swap " (saved.uuid) " until the coin " (data.taker_coin) " is activated");
                    taker_coin = try_s!(lp_coinfind(&ctx, &data.taker_coin));
                };

                let mut maker_coin = None;
                while maker_coin.is_none() {
                    thread::sleep(Duration::from_secs(5));
                    log!("Can't kickstart the swap " (saved.uuid) " until the coin " (data.maker_coin) " is activated");
                    maker_coin = try_s!(lp_coinfind(&ctx, &data.maker_coin));
                };
                let my_persistent_pub = H264::from(&**ctx.secp256k1_key_pair().public());

                let mut swap = TakerSwap::new(
                    ctx,
                    maker.into(),
                    maker_coin.unwrap(),
                    taker_coin.unwrap(),
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
}

impl Drop for TakerSwap {
    fn drop(&mut self) {
        let amount_to_unlock = &self.taker_amount + &self.taker_amount / 777;
        unlock_amount(&self.ctx, &self.uuid, &amount_to_unlock);
    }
}

/// Returns the status of swap performed on `my` node
pub fn my_swap_status(ctx: MmArc, req: Json) -> HyRes {
    let uuid = try_h!(req["params"]["uuid"].as_str().ok_or("uuid parameter is not set or is not string"));
    let path = my_swap_file_path(&ctx, uuid);
    let content = slurp(&path);
    if content.is_empty() {
        return rpc_response(404, json!({
            "error": "swap data is not found"
        }).to_string());
    }
    let status: SavedSwap = try_h!(json::from_slice(&content));
    let my_info = status.get_my_info();
    let mut json = try_h!(json::to_value(status));
    json["my_info"] = try_h!(json::to_value(my_info));

    rpc_response(200, json!({
        "result": json
    }).to_string())
}

/// Returns the status of requested swap, typically performed by other nodes and saved by `save_stats_swap_status`
pub fn stats_swap_status(ctx: MmArc, req: Json) -> HyRes {
    let uuid = try_h!(req["params"]["uuid"].as_str().ok_or("uuid parameter is not set or is not string"));
    let maker_path = stats_maker_swap_file_path(&ctx, uuid);
    let taker_path = stats_taker_swap_file_path(&ctx, uuid);
    let maker_content = slurp(&maker_path);
    let taker_content = slurp(&taker_path);
    let maker_status: Option<MakerSavedSwap> = if maker_content.is_empty() {
        None
    } else {
        Some(try_h!(json::from_slice(&maker_content)))
    };

    let taker_status: Option<TakerSavedSwap> = if taker_content.is_empty() {
        None
    } else {
        Some(try_h!(json::from_slice(&taker_content)))
    };

    if maker_status.is_none() && taker_status.is_none() {
        return rpc_response(404, json!({
            "error": "swap data is not found"
        }).to_string());
    }

    rpc_response(200, json!({
        "result": {
            "maker": maker_status,
            "taker": taker_status,
        }
    }).to_string())
}

/// Broadcasts `my` swap status to P2P network
fn broadcast_my_swap_status(uuid: &str, ctx: &MmArc) -> Result<(), String> {
    let path = my_swap_file_path(ctx, uuid);
    let content = slurp(&path);
    let mut status: SavedSwap = try_s!(json::from_slice(&content));
    match &mut status {
        SavedSwap::Taker(_) => (), // do nothing for taker
        SavedSwap::Maker(ref mut swap) => {
            match swap.events.first_mut() {
                Some(ref mut event) => match &mut event.event {
                    MakerSwapEvent::Started(ref mut data) => data.secret = H256Json::default(),
                    _ => (),
                }
                None => (),
            }
        }
    };
    try_s!(save_stats_swap(ctx, &status));
    let status_string = json!({
        "method": "swapstatus",
        "data": status,
    }).to_string();
    ctx.broadcast_p2p_msg(&status_string);
    Ok(())
}

/// Saves the swap status notification received from P2P network to local DB.
pub fn save_stats_swap_status(ctx: &MmArc, data: Json) -> HyRes {
    let swap: SavedSwap = try_h!(json::from_value(data));
    try_h!(save_stats_swap(ctx, &swap));
    rpc_response(200, json!({
        "result": "success"
    }).to_string())
}

/// Returns the data of recent swaps of `my` node. Returns no more than `limit` records (default: 10).
/// Skips the first `skip` records (default: 0).
pub fn my_recent_swaps(ctx: MmArc, req: Json) -> HyRes {
    let limit = req["limit"].as_u64().unwrap_or(10);
    let from_uuid = req["from_uuid"].as_str();
    let mut entries: Vec<(SystemTime, DirEntry)> = try_h!(my_swaps_dir(&ctx).read_dir()).filter_map(|dir_entry| {
        let entry = match dir_entry {
            Ok(ent) => ent,
            Err(e) => {
                log!("Error " (e) " reading from dir " (my_swaps_dir(&ctx).display()));
                return None;
            }
        };

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                log!("Error " (e) " getting file " (entry.path().display()) " meta");
                return None;
            }
        };

        let m_time = match metadata.modified() {
            Ok(time) => time,
            Err(e) => {
                log!("Error " (e) " getting file " (entry.path().display()) " m_time");
                return None;
            }
        };

        if entry.path().extension() == Some(OsStr::new("json")) {
            Some((m_time, entry))
        } else {
            None
        }
    }).collect();
    // sort by m_time in descending order
    entries.sort_by(|(a, _), (b, _)| b.cmp(&a));

    let skip = match from_uuid {
        Some(uuid) => try_h!(entries.iter().position(|(_, entry)| entry.path() == my_swap_file_path(&ctx, uuid)).ok_or(format!("from_uuid {} swap is not found", uuid))) + 1,
        None => 0,
    };

    // iterate over file entries trying to parse the file contents and add to result vector
    let swaps: Vec<Json> = entries.iter().skip(skip).take(limit as usize).map(|(_, entry)|
        match json::from_slice::<SavedSwap>(&slurp(&entry.path())) {
            Ok(swap) => {
                let my_info = swap.get_my_info();
                let mut json = unwrap!(json::to_value(swap));
                json["my_info"] = unwrap!(json::to_value(my_info));
                json
            },
            Err(e) => {
                log!("Error " (e) " parsing JSON from " (entry.path().display()));
                Json::Null
            },
        },
    ).collect();

    rpc_response(200, json!({
        "result": {
            "swaps": swaps,
            "from_uuid": from_uuid,
            "skipped": skip,
            "limit": limit,
            "total": entries.len(),
        },
    }).to_string())
}

/// Find out the swaps that need to be kick-started, continue from the point where swap was interrupted
/// Return the tickers of coins that must be enabled for swaps to continue
pub fn swap_kick_starts(ctx: MmArc) -> HashSet<String> {
    let mut coins = HashSet::new();
    let entries: Vec<DirEntry> = unwrap!(my_swaps_dir(&ctx).read_dir()).filter_map(|dir_entry| {
        let entry = match dir_entry {
            Ok(ent) => ent,
            Err(e) => {
                log!("Error " (e) " reading from dir " (my_swaps_dir(&ctx).display()));
                return None;
            }
        };

        if entry.path().extension() == Some(OsStr::new("json")) {
            Some(entry)
        } else {
            None
        }
    }).collect();

    entries.iter().for_each(|entry| {
        match json::from_slice::<SavedSwap>(&slurp(&entry.path())) {
            Ok(swap) => {
                if !swap.is_finished() {
                    log!("Kick starting the swap " [swap.uuid()]);
                    match swap.maker_coin_ticker() {
                        Ok(t) => coins.insert(t),
                        Err(e) => {
                            log!("Error " (e) " getting maker coin of swap " (swap.uuid()));
                            return;
                        }
                    };
                    match swap.taker_coin_ticker() {
                        Ok(t) => coins.insert(t),
                        Err(e) => {
                            log!("Error " (e) " getting taker coin of swap " (swap.uuid()));
                            return;
                        }
                    };
                    thread::spawn({
                        let ctx = ctx.clone();
                        move ||
                            match swap {
                                SavedSwap::Maker(swap) => match MakerSwap::load_from_saved(ctx, swap) {
                                    Ok((maker, command)) => run_maker_swap(maker, command),
                                    Err(e) => log!([e]),
                                },
                                SavedSwap::Taker(swap) => match TakerSwap::load_from_saved(ctx, swap) {
                                    Ok((taker, command)) => run_taker_swap(taker, command),
                                    Err(e) => log!([e]),
                                },
                            }
                    });
                }
            },
            Err(_) => (),
        }
    });
    coins
}

pub async fn coins_needed_for_kick_start(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    let res = try_s!(json::to_vec(&json!({
        "result": *(try_s!(ctx.coins_needed_for_kick_start.lock()))
    })));
    Ok(try_s!(Response::builder().body(res)))
}

#[cfg(test)]
mod lp_swap_tests {
    use super::*;

    #[test]
    fn test_dex_fee_amount() {
        let base = "BTC";
        let rel = "ETH";
        let amount = 1.into();
        let actual_fee = dex_fee_amount(base, rel, &amount);
        let expected_fee = amount / 777;
        assert_eq!(expected_fee, actual_fee);

        let base = "KMD";
        let rel = "ETH";
        let amount = 1.into();
        let actual_fee = dex_fee_amount(base, rel, &amount);
        let expected_fee = amount * BigDecimal::from(9) / 7770;
        assert_eq!(expected_fee, actual_fee);

        let base = "BTC";
        let rel = "KMD";
        let amount = 1.into();
        let actual_fee = dex_fee_amount(base, rel, &amount);
        let expected_fee = amount * BigDecimal::from(9) / 7770;
        assert_eq!(expected_fee, actual_fee);

        let base = "BTC";
        let rel = "KMD";
        let amount = unwrap!("0.001".parse());
        let actual_fee = dex_fee_amount(base, rel, &amount);
        let expected_fee: BigDecimal = unwrap!("0.0001".parse());
        assert_eq!(expected_fee, actual_fee);
    }

    #[test]
    fn test_serde_swap_negotiation_data() {
        let data = SwapNegotiationData::default();
        let bytes = serialize(&data);
        let deserialized = unwrap!(deserialize(bytes.as_slice()));
        assert_eq!(data, deserialized);
    }
}
