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
use bitcrypto::dhash160;
use btc_rpc::v1::types::{H160 as H160Json, H256 as H256Json, H264 as H264Json};
use coins::{MmCoinEnum, TransactionDetails};
use common::{bits256, dstr, HyRes, rpc_response, Timeout, swap_db_dir, str_to_malloc, lp};
use common::log::{TagParam};
use common::mm_ctx::MmArc;
use crc::crc32;
use futures::{Future};
use gstuff::{now_ms, slurp};
use rand::Rng;
use peers::SendHandler;
use primitives::hash::{H160, H264};
use serde_json::{self as json, Value as Json};
use serialization::{deserialize, serialize};
use std::fs::File;
use std::io::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Includes the grace time we add to the "normal" timeouts
/// in order to give different and/or heavy communication channels a chance.
const BASIC_COMM_TIMEOUT: u64 = 90;

/// Default atomic swap payment locktime.
/// Maker sends payment with LOCKTIME * 2
/// Taker sends payment with LOCKTIME
const PAYMENT_LOCKTIME: u64 = 3600 * 2 + 300 * 2;
const SWAP_DEFAULT_NUM_CONFIRMS: u32 = 1;
const SWAP_DEFAULT_MAX_CONFIRMS: u32 = 6;

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

fn payment_confirmations(maker_coin: &MmCoinEnum, taker_coin: &MmCoinEnum) -> (u32, u32) {
    let mut maker_confirmations = SWAP_DEFAULT_NUM_CONFIRMS;
    let mut taker_confirmations = SWAP_DEFAULT_NUM_CONFIRMS;
    if maker_coin.ticker() == "BTC" {
        maker_confirmations = 1;
    }

    if taker_coin.ticker() == "BTC" {
        taker_confirmations = 1;
    }

    if maker_coin.is_asset_chain() {
        if maker_coin.ticker() == "ETOMIC" {
            maker_confirmations = 1;
        } else {
            maker_confirmations = SWAP_DEFAULT_MAX_CONFIRMS / 2;
        }
    }

    if taker_coin.is_asset_chain() {
        if taker_coin.ticker() == "ETOMIC" {
            taker_confirmations = 1;
        } else {
            taker_confirmations = SWAP_DEFAULT_MAX_CONFIRMS / 2;
        }
    }

    // TODO recognize why the BAY case is special, ask JL777
    /*
        if ( strcmp("BAY",swap->I.req.src) != 0 && strcmp("BAY",swap->I.req.dest) != 0 )
    {
        swap->I.bobconfirms *= !swap->I.bobistrusted;
        swap->I.aliceconfirms *= !swap->I.aliceistrusted;
    }
    */

    (maker_confirmations, taker_confirmations)
}

// NB: Using a macro instead of a function in order to preserve the line numbers in the log.
macro_rules! send_ {
    ($ctx: expr, $to: expr, $subj: expr, $payload: expr) => {{
        // Checksum here helps us visually verify the logistics between the Maker and Taker logs.
        let crc = crc32::checksum_ieee (&$payload);
        log!("Sending '" ($subj) "' (" ($payload.len()) " bytes, crc " (crc) ")");

        peers::send ($ctx, $to, $subj.as_bytes(), $payload.into())
    }}
}

macro_rules! recv_ {
    ($swap: expr, $subj: expr, $timeout_sec: expr, $ec: expr, $validator: block) => {{
        let recv_subject = fomat! (($subj) '@' ($swap.uuid));
        let validator = Box::new ($validator) as Box<Fn(&[u8]) -> Result<(), String> + Send>;
        let recv_f = peers::recv (&$swap.ctx, recv_subject.as_bytes(), Box::new ({
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

#[test]
fn test_serde_swap_negotiation_data() {
    let data = SwapNegotiationData::default();
    let bytes = serialize(&data);
    let deserialized = deserialize(bytes.as_slice()).unwrap();
    assert_eq!(data, deserialized);
}

fn my_swap_file_path(uuid: &str) -> PathBuf {
    let path = swap_db_dir();
    path.join("MY").join(format!("{}.json", uuid))
}

fn stats_maker_swap_file_path(uuid: &str) -> PathBuf {
    let path = swap_db_dir();
    path.join("STATS").join("MAKER").join(format!("{}.json", uuid))
}

fn stats_taker_swap_file_path(uuid: &str) -> PathBuf {
    let path = swap_db_dir();
    path.join("STATS").join("TAKER").join(format!("{}.json", uuid))
}

fn save_my_maker_swap_event(uuid: &str, event: MakerSavedEvent) -> Result<(), String> {
    let path = my_swap_file_path(uuid);
    let content = slurp(&path);
    let swap: SavedSwap = if content.is_empty() {
        SavedSwap::Maker(MakerSavedSwap {
            uuid: uuid.to_owned(),
            events: vec![],
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

fn save_my_taker_swap_event(uuid: &str, event: TakerSavedEvent) -> Result<(), String> {
    let path = my_swap_file_path(uuid);
    let content = slurp(&path);
    let swap: SavedSwap = if content.is_empty() {
        SavedSwap::Taker(TakerSavedSwap {
            uuid: uuid.to_owned(),
            events: vec![]
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

fn save_stats_swap(swap: SavedSwap) -> Result<(), String> {
    let (path, content) = match &swap {
        SavedSwap::Maker(maker_swap) => (stats_maker_swap_file_path(&maker_swap.uuid), try_s!(json::to_vec(&maker_swap))),
        SavedSwap::Taker(taker_swap) => (stats_taker_swap_file_path(&taker_swap.uuid), try_s!(json::to_vec(&taker_swap))),
    };
    let mut file = try_s!(File::create(path));
    try_s!(file.write_all(&content));
    Ok(())
}

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
struct MakerSwapData {
    taker_coin: String,
    maker_coin: String,
    taker: H256Json,
    secret: H256Json,
    my_persistent_pub: H264Json,
    lock_duration: u64,
    maker_amount: u64,
    taker_amount: u64,
    maker_payment_confirmations: u32,
    taker_payment_confirmations: u32,
    maker_payment_lock: u64,
    /// Allows to recognize one SWAP from the other in the logs. #274.
    uuid: String,
    started_at: u64,
}

pub struct MakerSwap {
    ctx: MmArc,
    maker_coin: MmCoinEnum,
    taker_coin: MmCoinEnum,
    maker_amount: u64,
    taker_amount: u64,
    my_persistent_pub: H264,
    taker: bits256,
    uuid: String,
    data: MakerSwapData,
    taker_payment_lock: u64,
    other_persistent_pub: H264,
    taker_fee: Option<TransactionDetails>,
    maker_payment: Option<TransactionDetails>,
    taker_payment: Option<TransactionDetails>,
    taker_payment_spend: Option<TransactionDetails>,
    maker_payment_refund: Option<TransactionDetails>,
    errors: Vec<String>,
    finished_at: u64,
}

enum MakerSwapCommand {
    Start,
    Negotiate,
    WaitForTakerFee(Arc<SendHandler>),
    SendPayment,
    WaitForTakerPayment(Arc<SendHandler>),
    SpendTakerPayment,
    RefundMakerPayment,
    Finish
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
enum MakerSwapEvent {
    Started(MakerSwapData),
    StartFailed(String),
    Negotiated((u64, H264Json)),
    NegotiateFailed(String),
    TakerFeeValidated(TransactionDetails),
    TakerFeeValidateFailed(String),
    MakerPaymentSent(TransactionDetails),
    MakerPaymentTransactionFailed(String),
    MakerPaymentDataSendFailed(String),
    TakerPaymentValidatedAndConfirmed(TransactionDetails),
    TakerPaymentValidateFailed(String),
    TakerPaymentSpent(TransactionDetails),
    TakerPaymentSpendFailed(String),
    MakerPaymentRefunded(TransactionDetails),
    MakerPaymentRefundFailed(String),
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
            MakerSwapEvent::TakerPaymentValidatedAndConfirmed(_) => "Taker payment validated and confirmed...".to_owned(),
            MakerSwapEvent::TakerPaymentValidateFailed(_) => "Taker payment validate failed...".to_owned(),
            MakerSwapEvent::TakerPaymentSpent(_) => "Taker payment spent...".to_owned(),
            MakerSwapEvent::TakerPaymentSpendFailed(_) => "Taker payment spend failed...".to_owned(),
            MakerSwapEvent::MakerPaymentRefunded(_) => "Maker payment refunded...".to_owned(),
            MakerSwapEvent::MakerPaymentRefundFailed(_) => "Maker payment refund failed...".to_owned(),
            MakerSwapEvent::Finished => "Finished".to_owned(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct MakerSavedEvent {
    timestamp: u64,
    event: MakerSwapEvent,
}

#[derive(Debug, Serialize, Deserialize)]
struct TakerSavedEvent {
    timestamp: u64,
    event: TakerSwapEvent,
}

#[derive(Debug, Serialize, Deserialize)]
struct MakerSavedSwap {
    uuid: String,
    events: Vec<MakerSavedEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TakerSavedSwap {
    uuid: String,
    events: Vec<TakerSavedEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum SavedSwap {
    Maker(MakerSavedSwap),
    Taker(TakerSavedSwap),
}

macro_rules! recv {
    ($selff: ident, $subj: expr, $desc: expr, $timeout_sec: expr, $ec: expr, $validator: block) => {
        recv_! ($selff, $subj, $timeout_sec, $ec, $validator)
    };
    // Use this form if there's a sending future to terminate upon receiving the answer.
    ($selff: ident, $sending_f: ident, $subj: expr, $desc: expr, $timeout_sec: expr, $ec: expr, $validator: block) => {{
        let payload = recv_! ($selff, $subj, $timeout_sec, $ec, $validator);
        drop ($sending_f);
        payload
    }};
}

impl MakerSwap {
    fn apply_event(&mut self, event: MakerSwapEvent) -> Result<(), String> {
        match event {
            MakerSwapEvent::Started(data) => self.data = data,
            MakerSwapEvent::StartFailed(err) => self.errors.push(err),
            MakerSwapEvent::Negotiated((taker_payment_locktime, taker_pub)) => {
                self.taker_payment_lock = taker_payment_locktime;
                self.other_persistent_pub = taker_pub.into();
            },
            MakerSwapEvent::NegotiateFailed(err) => self.errors.push(err),
            MakerSwapEvent::TakerFeeValidated(tx) => self.taker_fee = Some(tx),
            MakerSwapEvent::TakerFeeValidateFailed(err) => self.errors.push(err),
            MakerSwapEvent::MakerPaymentSent(tx) => self.maker_payment = Some(tx),
            MakerSwapEvent::MakerPaymentTransactionFailed(err) => self.errors.push(err),
            MakerSwapEvent::MakerPaymentDataSendFailed(err) => self.errors.push(err),
            MakerSwapEvent::TakerPaymentValidatedAndConfirmed(tx) => self.taker_payment = Some(tx),
            MakerSwapEvent::TakerPaymentValidateFailed(err) => self.errors.push(err),
            MakerSwapEvent::TakerPaymentSpent(tx) => self.taker_payment_spend = Some(tx),
            MakerSwapEvent::TakerPaymentSpendFailed(err) => self.errors.push(err),
            MakerSwapEvent::MakerPaymentRefunded(tx) => self.maker_payment_refund = Some(tx),
            MakerSwapEvent::MakerPaymentRefundFailed(err) => self.errors.push(err),
            MakerSwapEvent::Finished => self.finished_at = now_ms() / 1000,
        }
        Ok(())
    }

    fn handle_command(&self, command: MakerSwapCommand)
        -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        match command {
            MakerSwapCommand::Start => self.start(),
            MakerSwapCommand::Negotiate => self.negotiate(),
            MakerSwapCommand::WaitForTakerFee(sending_f) => self.wait_taker_fee(sending_f),
            MakerSwapCommand::SendPayment => self.maker_payment(),
            MakerSwapCommand::WaitForTakerPayment(sending_f) => self.wait_for_taker_payment(sending_f),
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
        maker_amount: u64,
        taker_amount: u64,
        my_persistent_pub: H264,
        uuid: String,
    ) -> Self {
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
        }
    }

    fn start(&self) -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        if let Err(e) = self.maker_coin.check_i_have_enough_to_trade(dstr(self.maker_amount as i64), true).wait() {
            return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::StartFailed(ERRL!("!check_i_have_enough_to_trade {}", e))],
            ));
        };

        let lock_duration = lp_atomic_locktime(self.maker_coin.ticker(), self.taker_coin.ticker());
        let (maker_payment_confirmations, taker_payment_confirmations) = payment_confirmations(&self.maker_coin, &self.taker_coin);
        let mut rng = rand::thread_rng();
        let secret: [u8; 32] = rng.gen();
        let started_at = now_ms() / 1000;

        let data = MakerSwapData {
            taker_coin: self.taker_coin.ticker().to_owned(),
            maker_coin: self.maker_coin.ticker().to_owned(),
            taker: unsafe { self.taker.bytes.into() },
            secret: secret.into(),
            started_at,
            lock_duration,
            maker_amount: self.maker_amount,
            taker_amount: self.taker_amount,
            maker_payment_confirmations,
            taker_payment_confirmations,
            maker_payment_lock: started_at + lock_duration * 2,
            my_persistent_pub: self.my_persistent_pub.clone().into(),
            uuid: self.uuid.clone(),
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
        let sending_f = match send_! (&self.ctx, self.taker, fomat!(("negotiation") '@' (self.uuid)), bytes.as_slice()) {
            Ok(f) => f,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::NegotiateFailed(ERRL!("{}", e))],
            )),
        };

        let data = match recv!(self, sending_f, "negotiation-reply", "for Negotiation reply", 90, -2000, {|_: &[u8]| Ok(())}) {
            Ok(d) => d,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::NegotiateFailed(ERRL!("{:?}", e))],
            )),
        };
        let taker_data: SwapNegotiationData = match deserialize(data.as_slice()) {
            Ok(d) => d,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::NegotiateFailed(ERRL!("{:?}", e))],
            )),
        };
        // TODO add taker negotiation data validation
        let negotiated = serialize(&true);
        let sending_f = match send_! (&self.ctx, self.taker, fomat!(("negotiated") '@' (self.uuid)), negotiated.as_slice()) {
            Ok(f) => f,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::NegotiateFailed(ERRL!("{}", e))],
            )),
        };

        Ok((
            Some(MakerSwapCommand::WaitForTakerFee(sending_f)),
            vec![MakerSwapEvent::Negotiated((taker_data.payment_locktime, taker_data.persistent_pubkey.into()))],
        ))
    }

    fn wait_taker_fee(&self, sending_f: Arc<SendHandler>) -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        let payload = match recv!(self, sending_f, "taker-fee", "for Taker fee", 600, -2003, {|_: &[u8]| Ok(())}) {
            Ok(d) => d,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::TakerFeeValidateFailed(ERRL!("{}", e))]
            ))
        };
        let taker_fee = match self.taker_coin.tx_enum_from_bytes(&payload) {
            Ok(tx) => tx,
            Err(e) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::TakerFeeValidateFailed(ERRL!("{}", e))]
            ))
        };

        log!({"Taker fee tx {:02x}", taker_fee.tx_hash()});

        let fee_addr_pub_key = unwrap!(hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06"));
        let fee_amount = self.taker_amount / 777;
        let fee_details = unwrap!(taker_fee.transaction_details(self.taker_coin.decimals()));
        match self.taker_coin.validate_fee(taker_fee, &fee_addr_pub_key, fee_amount as u64) {
            Ok(_) => (),
            Err(err) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::TakerFeeValidateFailed(ERRL!("{}", err))]
            ))
        };
        Ok((
            Some(MakerSwapCommand::SendPayment),
            vec![MakerSwapEvent::TakerFeeValidated(fee_details)]
        ))
    }

    fn maker_payment(&self) -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        let payment_fut = self.maker_coin.send_maker_payment(
            self.data.maker_payment_lock as u32,
            &*self.other_persistent_pub,
            &*dhash160(&self.data.secret.0),
            self.maker_amount,
        );

        let transaction = match payment_fut.wait() {
            Ok(t) => t,
            Err(err) => return Ok((
                Some(MakerSwapCommand::Finish),
                vec![MakerSwapEvent::MakerPaymentTransactionFailed(ERRL!("{}", err))],
            ))
        };
        let tx_details = unwrap!(transaction.transaction_details(self.maker_coin.decimals()));
        log!({"Maker payment tx {:02x}", transaction.tx_hash()});
        let sending_f = match send_! (&self.ctx, self.taker, fomat!(("maker-payment") '@' (self.uuid)), transaction.tx_hex()) {
            Ok(f) => f,
            Err(e) => return Ok((
                Some(MakerSwapCommand::RefundMakerPayment),
                vec![MakerSwapEvent::MakerPaymentSent(tx_details), MakerSwapEvent::MakerPaymentDataSendFailed(ERRL!("{}", e))]
            ))
        };

        Ok((
            Some(MakerSwapCommand::WaitForTakerPayment(sending_f)),
            vec![MakerSwapEvent::MakerPaymentSent(tx_details)]
        ))
    }

    fn wait_for_taker_payment(&self, sending_f: Arc<SendHandler>) -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        let wait_duration = self.data.lock_duration / 3;
        let wait_taker_payment = self.data.started_at + wait_duration;
        let payload = match recv!(self, sending_f, "taker-payment", "for Taker payment", wait_duration, -2006, {|_: &[u8]| Ok(())}) {
            Ok(p) => p,
            Err(e) => return Ok((
                Some(MakerSwapCommand::RefundMakerPayment),
                vec![MakerSwapEvent::TakerPaymentValidateFailed(e)],
            ))
        };

        let taker_payment = match self.taker_coin.tx_enum_from_bytes(&payload) {
            Ok(tx) => tx,
            Err(err) => return Ok((
                Some(MakerSwapCommand::RefundMakerPayment),
                vec![MakerSwapEvent::TakerFeeValidateFailed(ERRL!("!taker_coin.tx_enum_from_bytes: {}", err))]
            )),
        };

        let validated = self.taker_coin.validate_taker_payment(
            taker_payment.clone(),
            self.taker_payment_lock as u32,
            &*self.other_persistent_pub,
            &*dhash160(&self.data.secret.0),
            self.taker_amount,
        );

        if let Err(e) = validated {
            return Ok((
                Some(MakerSwapCommand::RefundMakerPayment),
                vec![MakerSwapEvent::TakerFeeValidateFailed(ERRL!("!taker_coin.validate_taker_payment: {}", e))]
            ))
        }

        log!({"Taker payment tx {:02x}", taker_payment.tx_hash()});
        let tx_details = unwrap!(taker_payment.transaction_details(self.taker_coin.decimals()));
        let wait = self.taker_coin.wait_for_confirmations(
            taker_payment,
            self.data.taker_payment_confirmations,
            wait_taker_payment,
        );

        if let Err(err) = wait {
            return Ok((
                Some(MakerSwapCommand::RefundMakerPayment),
                vec![MakerSwapEvent::TakerFeeValidateFailed(ERRL!("!taker_coin.wait_for_confirmations: {}", err))]
            ))
        }

        Ok((
            Some(MakerSwapCommand::SpendTakerPayment),
            vec![MakerSwapEvent::TakerPaymentValidatedAndConfirmed(tx_details)]
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
                vec![MakerSwapEvent::TakerPaymentSpendFailed(ERRL!("!taker_coin.send_maker_spends_taker_payment: {}", err))]
            ))
        };

        let tx_details = unwrap!(transaction.transaction_details(self.taker_coin.decimals()));

        log!({"Taker payment spend tx {:02x}", transaction.tx_hash()});
        Ok((
            Some(MakerSwapCommand::Finish),
            vec![MakerSwapEvent::TakerPaymentSpent(tx_details)]
        ))
    }

    fn refund_maker_payment(&self) -> Result<(Option<MakerSwapCommand>, Vec<MakerSwapEvent>), String> {
        while now_ms() / 1000 < self.data.maker_payment_lock {
            std::thread::sleep(Duration::from_secs(10));
        }

        let spend_fut = self.taker_coin.send_maker_refunds_payment(
            &unwrap!(self.maker_payment.clone()).tx_hex,
            self.data.maker_payment_lock as u32,
            &*self.other_persistent_pub,
            &*dhash160(&self.data.secret.0),
        );

        let transaction = match spend_fut.wait() {
            Ok(t) => t,
            Err(err) => return Ok((
                Some(MakerSwapCommand::RefundMakerPayment),
                vec![MakerSwapEvent::TakerPaymentSpendFailed(ERRL!("!taker_coin.send_maker_spends_taker_payment: {}", err))]
            ))
        };

        let tx_details = unwrap!(transaction.transaction_details(self.taker_coin.decimals()));

        log!({"Maker payment refund tx {:02x}", transaction.tx_hash()});
        Ok((
            Some(MakerSwapCommand::Finish),
            vec![MakerSwapEvent::TakerPaymentSpent(tx_details)],
        ))
    }
}

/// Starts the maker swap and drives it to completion (until None next command received).
/// Panics in case of command or event apply fails, not sure yet how to handle such situations
/// because it's usually means that swap is in invalid state which is possible only if there's developer error.
/// Every produced event is saved to local DB. Swap status is broadcasted to P2P network after completion.
pub fn run_maker_swap(mut swap: MakerSwap) {
    let mut command = MakerSwapCommand::Start;
    let mut events;
    let ctx = swap.ctx.clone();
    let mut status = ctx.log.status_handle();
    let uuid = swap.uuid.clone();
    let swap_tags: &[&TagParam] = &[&"swap", &("uuid", &uuid[..])];
    loop {
        let res = unwrap!(swap.handle_command(command));
        events = res.1;
        for event in events {
            let to_save = MakerSavedEvent {
                timestamp: now_ms(),
                event: event.clone(),
            };
            unwrap!(save_my_maker_swap_event(&swap.uuid, to_save));
            status.status(swap_tags, &event.status_str());
            unwrap!(swap.apply_event(event));
        }
        match res.0 {
            Some(c) => { command = c; },
            None => {
                unwrap!(broadcast_my_swap_status(&swap.uuid));
                break;
            },
        }
    }
}

/// Starts the taker swap and drives it to completion (until None next command received).
/// Panics in case of command or event apply fails, not sure yet how to handle such situations
/// because it's usually means that swap is in invalid state which is possible only if there's developer error
/// Every produced event is saved to local DB. Swap status is broadcasted to P2P network after completion.
pub fn run_taker_swap(mut swap: TakerSwap) {
    let mut command = TakerSwapCommand::Start;
    let mut events;
    let ctx = swap.ctx.clone();
    let mut status = ctx.log.status_handle();
    let uuid = swap.uuid.clone();
    let swap_tags: &[&TagParam] = &[&"swap", &("uuid", &uuid[..])];
    loop {
        let res = unwrap!(swap.handle_command(command));
        events = res.1;
        for event in events {
            let to_save = TakerSavedEvent {
                timestamp: now_ms(),
                event: event.clone(),
            };
            unwrap!(save_my_taker_swap_event(&swap.uuid, to_save));
            status.status(swap_tags, &event.status_str());
            unwrap!(swap.apply_event(event));
        }
        match res.0 {
            Some(c) => { command = c; },
            None => {
                unwrap!(broadcast_my_swap_status(&swap.uuid));
                break;
            },
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
struct TakerSwapData {
    taker_coin: String,
    maker_coin: String,
    maker: H256Json,
    my_persistent_pub: H264Json,
    lock_duration: u64,
    maker_amount: u64,
    taker_amount: u64,
    maker_payment_confirmations: u32,
    taker_payment_confirmations: u32,
    taker_payment_lock: u64,
    /// Allows to recognize one SWAP from the other in the logs. #274.
    uuid: String,
    started_at: u64,
    maker_payment_wait: u64,
}

pub struct TakerSwap {
    ctx: MmArc,
    maker_coin: MmCoinEnum,
    taker_coin: MmCoinEnum,
    maker_amount: u64,
    taker_amount: u64,
    my_persistent_pub: H264,
    maker: bits256,
    uuid: String,
    data: TakerSwapData,
    maker_payment_lock: u64,
    other_persistent_pub: H264,
    taker_fee: Option<TransactionDetails>,
    maker_payment: Option<TransactionDetails>,
    taker_payment: Option<TransactionDetails>,
    taker_payment_spend: Option<TransactionDetails>,
    maker_payment_spend: Option<TransactionDetails>,
    taker_payment_refund: Option<TransactionDetails>,
    errors: Vec<String>,
    finished_at: u64,
    secret_hash: H160Json,
    secret: H256Json,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
enum TakerSwapEvent {
    Started(TakerSwapData),
    StartFailed(String),
    Negotiated((u64, H264Json, H160Json)),
    NegotiateFailed(String),
    TakerFeeSent(TransactionDetails),
    TakerFeeSendFailed(String),
    MakerPaymentValidatedAndConfirmed(TransactionDetails),
    MakerPaymentValidateFailed(String),
    TakerPaymentSent(TransactionDetails),
    TakerPaymentTransactionFailed(String),
    TakerPaymentDataSendFailed(String),
    TakerPaymentSpent((TransactionDetails, H256Json)),
    TakerPaymentWaitForSpendFailed(String),
    MakerPaymentSpent(TransactionDetails),
    MakerPaymentSpendFailed(String),
    TakerPaymentRefunded(TransactionDetails),
    TakerPaymentRefundFailed(String),
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
            TakerSwapEvent::MakerPaymentValidatedAndConfirmed(_) => "Maker payment validated and confirmed...".to_owned(),
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

enum TakerSwapCommand {
    Start,
    Negotiate,
    SendTakerFee,
    WaitForMakerPayment(Arc<SendHandler>),
    SendTakerPayment,
    WaitForTakerPaymentSpend(Arc<SendHandler>),
    SpendMakerPayment,
    RefundTakerPayment,
    Finish
}

impl TakerSwap {
    fn apply_event(&mut self, event: TakerSwapEvent) -> Result<(), String> {
        match event {
            TakerSwapEvent::Started(data) => self.data = data,
            TakerSwapEvent::StartFailed(err) => self.errors.push(err),
            TakerSwapEvent::Negotiated((maker_payment_locktime, maker_pub, secret_hash)) => {
                self.maker_payment_lock = maker_payment_locktime;
                self.other_persistent_pub = maker_pub.into();
                self.secret_hash = secret_hash;
            },
            TakerSwapEvent::NegotiateFailed(err) => self.errors.push(err),
            TakerSwapEvent::TakerFeeSent(tx) => self.taker_fee = Some(tx),
            TakerSwapEvent::TakerFeeSendFailed(err) => self.errors.push(err),
            TakerSwapEvent::MakerPaymentValidatedAndConfirmed(tx) => self.maker_payment = Some(tx),
            TakerSwapEvent::MakerPaymentValidateFailed(err) => self.errors.push(err),
            TakerSwapEvent::TakerPaymentSent(tx) => self.taker_payment = Some(tx),
            TakerSwapEvent::TakerPaymentTransactionFailed(err) => self.errors.push(err),
            TakerSwapEvent::TakerPaymentDataSendFailed(err) => self.errors.push(err),
            TakerSwapEvent::TakerPaymentSpent((tx, secret)) => {
                self.taker_payment_spend = Some(tx);
                self.secret = secret;
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
            TakerSwapCommand::WaitForMakerPayment(sending_f) => self.wait_for_maker_payment(sending_f),
            TakerSwapCommand::SendTakerPayment => self.send_taker_payment(),
            TakerSwapCommand::WaitForTakerPaymentSpend(sending_f) => self.wait_for_taker_payment_spend(sending_f),
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
        maker_amount: u64,
        taker_amount: u64,
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
            data: TakerSwapData::default(),
            other_persistent_pub: H264::default(),
            taker_fee: None,
            maker_payment: None,
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
        if let Err(e) = self.taker_coin.check_i_have_enough_to_trade(dstr(self.taker_amount as i64), true).wait() {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::StartFailed(ERRL!("{}", e))],
            ))
        }

        let lock_duration = lp_atomic_locktime(self.maker_coin.ticker(), self.taker_coin.ticker());
        let (maker_payment_confirmations, taker_payment_confirmations) = payment_confirmations(&self.maker_coin, &self.taker_coin);
        let started_at = now_ms() / 1000;

        let data = TakerSwapData {
            taker_coin: self.taker_coin.ticker().to_owned(),
            maker_coin: self.maker_coin.ticker().to_owned(),
            maker: unsafe { self.maker.bytes.into() },
            started_at,
            lock_duration,
            maker_amount: self.maker_amount,
            taker_amount: self.taker_amount,
            maker_payment_confirmations,
            taker_payment_confirmations,
            taker_payment_lock: started_at + lock_duration,
            my_persistent_pub: self.my_persistent_pub.clone().into(),
            uuid: self.uuid.clone(),
            maker_payment_wait: started_at + lock_duration / 3,
        };

        Ok((Some(TakerSwapCommand::Negotiate), vec![TakerSwapEvent::Started(data)]))
    }

    fn negotiate(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let data = match recv!(self, "negotiation", "for Maker negotiation data", 90, -1000, {|_: &[u8]| Ok(())}) {
            Ok(d) => d,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("{:?}", e))]
            )),
        };
        let maker_data: SwapNegotiationData = match deserialize(data.as_slice()) {
            Ok(d) => d,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("{:?}", e))]
            )),
        };

        let time_dif = (self.data.started_at as i64 - maker_data.started_at as i64).abs();
        if  time_dif > 60 {
            // AG: I see this check failing with `LP_AUTOTRADE_TIMEOUT` bumped from 30 to 120.
            //err!(-1002, "Started_at time_dif over 60: "(time_dif))
            log!("Started_at time_dif over 60: "(time_dif));
        }

        let taker_data = SwapNegotiationData {
            started_at: self.data.started_at,
            secret_hash: maker_data.secret_hash.clone(),
            payment_locktime: self.data.taker_payment_lock,
            persistent_pubkey: self.my_persistent_pub.clone(),
        };
        let bytes = serialize(&taker_data);
        let sending_f = match send_! (&self.ctx, self.maker, fomat!(("negotiation-reply") '@' (self.uuid)), bytes.as_slice()) {
            Ok(f) => f,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("{}", e))]
            )),
        };
        let data = match recv!(self, sending_f, "negotiated", "for Maker negotiated", 90, -1000, {|_: &[u8]| Ok(())}) {
            Ok(d) => d,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("{:?}", e))]
            )),
        };
        let negotiated: bool = match deserialize(data.as_slice()) {
            Ok(n) => n,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("{:?}", e))]
            )),
        };

        if !negotiated {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::NegotiateFailed(ERRL!("Maker sent negotiated = false"))],
            ));
        }

        Ok((
            Some(TakerSwapCommand::SendTakerFee),
            vec![TakerSwapEvent::Negotiated((
                maker_data.payment_locktime,
                maker_data.persistent_pubkey.into(),
                maker_data.secret_hash.into()
            ))],
        ))
    }

    fn send_taker_fee(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let fee_addr_pub_key = unwrap!(hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06"));
        let fee_amount = self.taker_amount / 777;
        let fee_tx = self.taker_coin.send_taker_fee(&fee_addr_pub_key, fee_amount as u64).wait();
        let transaction = match fee_tx {
            Ok (t) => t,
            Err (err) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerFeeSendFailed(ERRL!("{}", err))]
            )),
        };

        log!({"Taker fee tx hash {:02x}", transaction.tx_hash()});
        let sending_f = match send_! (&self.ctx, self.maker, fomat!(("taker-fee") '@' (self.uuid)), transaction.tx_hex()) {
            Ok(f) => f,
            Err (err) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerFeeSendFailed(ERRL!("{}", err))]
            )),
        };
        Ok((
            Some(TakerSwapCommand::WaitForMakerPayment(sending_f)),
            vec![TakerSwapEvent::TakerFeeSent(transaction.transaction_details(self.taker_coin.decimals()).unwrap())],
        ))
    }

    fn wait_for_maker_payment(&self, sending_f: Arc<SendHandler>) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let payload = match recv!(self, sending_f, "maker-payment", "for Maker payment", 600, -1005, {|_: &[u8]| Ok(())}) {
            Ok(p) => p,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::MakerPaymentValidateFailed(ERRL!("Error waiting for 'maker-payment' data: {}", e))]
            )),
        };
        let maker_payment = match self.maker_coin.tx_enum_from_bytes(&payload) {
            Ok(p) => p,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::MakerPaymentValidateFailed(ERRL!("Error parsing the 'maker-payment': {}", e))]
            )),
        };

        let validated = self.maker_coin.validate_maker_payment(
            maker_payment.clone(),
            self.maker_payment_lock as u32,
            &*self.other_persistent_pub,
            &self.secret_hash.0,
            self.maker_amount,
        );

        if let Err(e) = validated {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::MakerPaymentValidateFailed(ERRL!("!validate maker payment: {}", e))]
            ));
        }

        log!({"Got maker payment {:02x}", maker_payment.tx_hash()});
        let tx_details = maker_payment.transaction_details(self.maker_coin.decimals()).unwrap();
        if let Err(err) = self.maker_coin.wait_for_confirmations(
            maker_payment,
            self.data.maker_payment_confirmations,
            self.data.maker_payment_wait,
        ) {
            return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::MakerPaymentValidateFailed(ERRL!("!wait for maker payment confirmations: {}", err))]
            ));
        }

        Ok((
            Some(TakerSwapCommand::SendTakerPayment),
            vec![TakerSwapEvent::MakerPaymentValidatedAndConfirmed(tx_details)]
        ))
    }

    fn send_taker_payment(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let payment_fut = self.taker_coin.send_taker_payment(
            self.data.taker_payment_lock as u32,
            &*self.other_persistent_pub,
            &self.secret_hash.0,
            self.taker_amount,
        );

        let transaction = match payment_fut.wait() {
            Ok(t) => t,
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerPaymentTransactionFailed(ERRL!("{}", e))]
            ))
        };

        log!({"Taker payment tx hash {:02x}", transaction.tx_hash()});
        let tx_details = transaction.transaction_details(self.taker_coin.decimals()).unwrap();

        let sending_f = match send_! (&self.ctx, self.maker, fomat!(("taker-payment") '@' (self.uuid)), transaction.tx_hex()) {
            Ok(f) => f,
            Err(e) => return Ok((
                Some(TakerSwapCommand::RefundTakerPayment),
                vec![TakerSwapEvent::TakerPaymentSent(tx_details), TakerSwapEvent::TakerPaymentDataSendFailed(e)]
            ))
        };

        Ok((
            Some(TakerSwapCommand::WaitForTakerPaymentSpend(sending_f)),
            vec![TakerSwapEvent::TakerPaymentSent(tx_details)],
        ))
    }

    fn wait_for_taker_payment_spend(&self, sending_f: Arc<SendHandler>) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let tx = match self.taker_coin.wait_for_tx_spend(&self.taker_payment.clone().unwrap().tx_hex, self.data.taker_payment_lock) {
            Ok(t) => t,
            Err(e) => return Ok((
                Some(TakerSwapCommand::RefundTakerPayment),
                vec![TakerSwapEvent::TakerPaymentWaitForSpendFailed(e)],
            ))
        };
        drop(sending_f);
        log!({"Taker payment spend tx {:02x}", tx.tx_hash()});
        let tx_details = tx.transaction_details(self.taker_coin.decimals()).unwrap();
        let secret = match tx.extract_secret() {
            Ok(bytes) => H256Json::from(bytes.as_slice()),
            Err(e) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::TakerPaymentWaitForSpendFailed(ERRL!("{}", e))],
            )),
        };

        Ok((
            Some(TakerSwapCommand::SpendMakerPayment),
            vec![TakerSwapEvent::TakerPaymentSpent((tx_details, secret))],
        ))
    }

    fn spend_maker_payment(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        let spend_fut = self.maker_coin.send_taker_spends_maker_payment(
            &self.maker_payment.clone().unwrap().tx_hex.0,
            self.maker_payment_lock as u32,
            &*self.other_persistent_pub,
            &self.secret.0,
        );

        let transaction = match spend_fut.wait() {
            Ok(t) => t,
            Err(err) => return Ok((
                Some(TakerSwapCommand::Finish),
                vec![TakerSwapEvent::MakerPaymentSpendFailed(ERRL!("{}", err))]
            )),
        };

        log!({"Maker payment spend tx {:02x}", transaction.tx_hash()});
        let tx_details = transaction.transaction_details(self.maker_coin.decimals()).unwrap();
        Ok((
            Some(TakerSwapCommand::Finish),
            vec![TakerSwapEvent::MakerPaymentSpent(tx_details)],
        ))
    }

    fn refund_taker_payment(&self) -> Result<(Option<TakerSwapCommand>, Vec<TakerSwapEvent>), String> {
        loop {
            if now_ms() / 1000 > self.data.taker_payment_lock + 10 {
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
                vec![TakerSwapEvent::TakerPaymentRefundFailed(ERRL!("{}", err))]
            )),
        };
        log!({"Taker refund tx hash {:02x}", transaction.tx_hash()});
        let tx_details = transaction.transaction_details(self.taker_coin.decimals()).unwrap();
        Ok((
            Some(TakerSwapCommand::Finish),
            vec![TakerSwapEvent::TakerPaymentRefunded(tx_details)],
        ))
    }
}

/// Returns the status of swap performed on `my` node
pub fn my_swap_status(req: Json) -> HyRes {
    let uuid = try_h!(req["params"]["uuid"].as_str().ok_or("uuid parameter is not set or is not string"));
    let path = my_swap_file_path(uuid);
    let content = slurp(&path);
    let status: SavedSwap = try_h!(json::from_slice(&content));

    rpc_response(200, json!({
        "result": status
    }).to_string())
}

/// Returns the status of requested swap, typically performed by other nodes and saved by `save_stats_swap_status`
pub fn stats_swap_status(req: Json) -> HyRes {
    let uuid = try_h!(req["params"]["uuid"].as_str().ok_or("uuid parameter is not set or is not string"));
    let maker_path = stats_maker_swap_file_path(uuid);
    let taker_path = stats_taker_swap_file_path(uuid);
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

    rpc_response(200, json!({
        "result": {
            "maker": maker_status,
            "taker": taker_status,
        }
    }).to_string())
}

/// Broadcasts `my` swap status to P2P network
fn broadcast_my_swap_status(uuid: &str) -> Result<(), String> {
    let path = my_swap_file_path(uuid);
    let content = slurp(&path);
    let status: SavedSwap = try_s!(json::from_slice(&content));
    let status_string = json!({
        "method": "swapstatus",
        "data": status,
    }).to_string();
    let status_c_string = str_to_malloc(&status_string);
    let zero = lp::bits256::default();
    unsafe { lp::LP_reserved_msg(0, zero, status_c_string); }
    Ok(())
}

/// Saves the swap status notification received from P2P network to local DB.
pub fn save_stats_swap_status(data: Json) -> HyRes {
    let swap: SavedSwap = try_h!(json::from_value(data));
    try_h!(save_stats_swap(swap));
    rpc_response(200, json!({
        "result": "success"
    }).to_string())
}
