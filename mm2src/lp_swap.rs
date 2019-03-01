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
use coins::{MmCoinEnum, TransactionEnum};
use common::{bits256, dstr, Timeout};
use common::log::TagParam;
use common::mm_ctx::MmArc;
use crc::crc32;
use futures::{Future};
use gstuff::now_ms;
use rand::Rng;
use peers::SendHandler;
use primitives::hash::{H160, H256, H264};
use serialization::{deserialize, serialize};
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
    ($swap: expr, $status: expr, $swap_tags: expr, $subj: expr, $desc: expr, $timeout_sec: expr, $ec: expr, $validator: block) => {{
        let recv_subject = fomat! (($subj) '@' ($swap.uuid));
        $status.status ($swap_tags, &fomat! ("Waiting " ($desc) '…'));
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
        let payload = match recv_f.wait() {
            Ok (p) => p,
            Err (err) => {
                $status.append (&fomat! (" Error: " (err)));
                // cf. https://github.com/artemii235/SuperNET/blob/99217fe947dab67c304a9490a3ae6b57ad587110/iguana/exchanges/LP_swap.c#L985
                return Err (($ec, fomat! ("Error getting '" (recv_subject) "': " (err))))
            }
        };
        $status.append (" Done.");

        // Checksum here helps us visually verify the logistics between the Maker and Taker logs.
        let crc = crc32::checksum_ieee (&payload);
        log! ("Received '" (recv_subject) "' (" (payload.len()) " bytes, crc " (crc) ")");

        payload
    }}
}

// AG: The explicit state here constitutes an early and experimental design aimed towards
// serializable and resumable SWAP. The `AtomicSwapState` is essentially a list of `goto` labels,
// allowing us to jump anywhere in the SWAP loops.
// Given that the SWAP is the centerpiece of this software
// and improving the quality of the code here might reap us some noticeable benefits,
// we should probably take another go at designing this, as discussed in
// https://github.com/artemii235/SuperNET/commit/d66ab944bfd8c5e8fb17f1d36ac303797156b88e#r31674919
// In particular,
// 1) I'd like the design to emerge from a realistic save-resume scenario(s),
// that is, where the saves and resumes actually happen, at least from under a unit test;
// 2) I'd like the transitions to be implemented as pure functions,
// cf. https://github.com/artemii235/SuperNET/tree/mm2-dice/mm2src#purely-functional-core
// 3) Preferably untangling them from the portions of the shared state that are not relevant to them,
// that is, avoiding the "big ball of mud" and "object orgy" antipatterns of a single shared state structure.

/// Contains all available states of Atomic swap of both sides (maker and taker)
enum AtomicSwapState {
    Negotiation,
    SendTakerFee,
    WaitTakerFee {sending_f: Arc<SendHandler>},
    SendMakerPayment,
    WaitMakerPayment {sending_f: Arc<SendHandler>},
    SendTakerPayment,
    WaitTakerPayment {sending_f: Arc<SendHandler>},
    SpendTakerPayment,
    WaitTakerPaymentSpent {sending_f: Arc<SendHandler>},
    SpendMakerPayment,
    RefundTakerPayment,
    RefundMakerPayment,
}

pub struct AtomicSwap {
    ctx: MmArc,
    state: Option<AtomicSwapState>,
    taker_coin: MmCoinEnum,
    maker_coin: MmCoinEnum,
    taker_payment: Option<TransactionEnum>,
    taker_payment_lock: u64,
    maker_payment: Option<TransactionEnum>,
    maker_payment_lock: u64,
    taker: bits256,
    maker: bits256,
    session: String,
    secret: H256,
    secret_hash: H160,
    my_persistent_pub: H264,
    other_persistent_pub: H264,
    lock_duration: u64,
    maker_amount: u64,
    taker_amount: u64,
    maker_payment_confirmations: u32,
    taker_payment_confirmations: u32,
    /// Allows to regognize one SWAP from the other in the logs. #274.
    uuid: String,
}

impl AtomicSwap {
    pub fn new(
        ctx: MmArc,
        taker: bits256,
        maker: bits256,
        session: String,
        maker_coin: MmCoinEnum,
        taker_coin: MmCoinEnum,
        maker_amount: u64,
        taker_amount: u64,
        my_persistent_pub: H264,
        uuid: String,
    ) -> Result<AtomicSwap, String> {
        let lock_duration = lp_atomic_locktime(maker_coin.ticker(), taker_coin.ticker());
        let (maker_payment_confirmations, taker_payment_confirmations) = payment_confirmations(&maker_coin, &taker_coin);

        Ok(AtomicSwap {
            ctx,
            state: Some (AtomicSwapState::Negotiation),
            taker_coin,
            maker_coin,
            taker_payment: None,
            taker_payment_lock: 0,
            maker_payment: None,
            maker_payment_lock: 0,
            taker,
            maker,
            session,
            secret: [0; 32].into(),
            secret_hash: H160::default(),
            other_persistent_pub: H264::default(),
            lock_duration,
            maker_amount,
            taker_amount,
            maker_payment_confirmations,
            taker_payment_confirmations,
            my_persistent_pub,
            uuid,
        })
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

#[test]
fn test_serde_swap_negotiation_data() {
    let data = SwapNegotiationData::default();
    let bytes = serialize(&data);
    let deserialized = deserialize(bytes.as_slice()).unwrap();
    assert_eq!(data, deserialized);
}

pub fn maker_swap_loop(swap: &mut AtomicSwap) -> Result<(), (i32, String)> {
    // NB: We can communicate the SWAP status to UI progress indicators via documented tags,
    // cf. https://github.com/artemii235/SuperNET/commit/d66ab944bfd8c5e8fb17f1d36ac303797156b88e#r31676734
    // (but first we need to establish a use case for such indication with the UI guys,
    //  in order to avoid premature throw-away design, cf. https://www.agilealliance.org/glossary/simple-design).
    let mut status = swap.ctx.log.status_handle();
    let uuid = swap.uuid.clone();
    let swap_tags: &[&TagParam] = &[&"swap", &("uuid", &uuid[..])];
    macro_rules! send {
        ($ec: expr, $subj: expr, $slice: expr) => {
            match send_! (&swap.ctx, swap.taker, fomat!(($subj) '@' (swap.uuid)), $slice) {
                Ok(h) => h,
                Err(err) => err!($ec, "send error: "(err))
            }
    }   }
    macro_rules! recv {
        ($subj: expr, $desc: expr, $timeout_sec: expr, $ec: expr, $validator: block) => {
            recv_! (swap, status, $subj, $desc, $timeout_sec, $ec, $validator)
        };
        // Use this form if there's a sending future to terminate upon receiving the answer.
        ($sending_f: ident, $subj: expr, $desc: expr, $timeout_sec: expr, $ec: expr, $validator: block) => {{
            let payload = recv_! (swap, status, swap_tags, $subj, $desc, $timeout_sec, $ec, $validator);
            drop ($sending_f);
            payload
        }};
    }
    // Note that `err!` updates the current `status`. We assume there is no blind spots in the `status`.
    // NB: If we want to replace the `err!` with `?` then we should move the `status` ownership to the call site.
    //     (Which IMHO would break the status code flow and encapsulation a little).
    macro_rules! err {
        ($ec: expr, $($msg: tt)+) => {{
            let mut msg = fomat! (' ' $($msg)+);
            status.append (&msg);
            msg.remove (0);
            return Err (($ec, msg))
        }};
    }

    if let Err(e) = swap.maker_coin.check_i_have_enough_to_trade(dstr(swap.maker_amount as i64), true).wait() {
        err!(-2000, "!check_i_have_enough_to_trade" [e]);
    };

    let started_at = now_ms() / 1000;
    let mut rng = rand::thread_rng();
    let secret: [u8; 32] = rng.gen();
    swap.maker_payment_lock = started_at + swap.lock_duration * 2;
    let wait_taker_payment = started_at + swap.lock_duration / 3;

    swap.secret_hash = dhash160(&secret);
    swap.secret = secret.into();
    let maker_negotiation_data = SwapNegotiationData {
        started_at,
        payment_locktime: swap.maker_payment_lock,
        secret_hash: swap.secret_hash.clone(),
        persistent_pubkey: swap.my_persistent_pub.clone(),
    };

    loop {
        let next_state = match unwrap!(swap.state.take()) {
            AtomicSwapState::Negotiation => {
                let bytes = serialize(&maker_negotiation_data);
                let sending_f = send!(-2001, "negotiation", bytes.as_slice());

                let data = recv!(sending_f, "negotiation-reply", "for Negotiation reply", 90, -2000, {|_: &[u8]| Ok(())});
                let taker_data: SwapNegotiationData = match deserialize(data.as_slice()) {
                    Ok(d) => d,
                    Err(e) => err!(-2001, "!negotiation-reply-deserialize: " [e]),
                };

                swap.taker_payment_lock = taker_data.payment_locktime;
                swap.other_persistent_pub = taker_data.persistent_pubkey;

                let negotiated = serialize(&true);
                let sending_f = send!(-2001, "negotiated", negotiated.as_slice());

                AtomicSwapState::WaitTakerFee { sending_f }
            },
            AtomicSwapState::WaitTakerFee { sending_f } => {
                let payload = recv!(sending_f, "taker-fee", "for Taker fee", 600, -2003, {|_: &[u8]| Ok(())});
                let taker_fee = match swap.taker_coin.tx_enum_from_bytes(&payload) {
                    Ok(tx) => tx,
                    Err(err) => err!(-2003, "!tx_enum_from_bytes: "(err)),
                };

                log!("Taker fee tx " (taker_fee.tx_hash()));

                let fee_addr_pub_key = unwrap!(hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06"));
                let fee_amount = swap.taker_amount / 777;
                match swap.taker_coin.validate_fee(taker_fee, &fee_addr_pub_key, fee_amount as u64) {
                    Ok(_) => (),
                    Err(err) => err!(-2010, "!validate taker fee: "(err)),
                };
                AtomicSwapState::SendMakerPayment
            },
            AtomicSwapState::SendMakerPayment => {
                let payment_fut = swap.maker_coin.send_maker_payment(
                    swap.maker_payment_lock as u32,
                    &*swap.other_persistent_pub,
                    &*swap.secret_hash,
                    swap.maker_amount,
                );

                status.status(swap_tags, "Sending Maker payment…");
                let transaction = match payment_fut.wait() {
                    Ok(t) => t,
                    Err(err) => err!(-2006, "!send_maker_payment: "(err))
                };
                log!("Maker payment tx " (transaction.tx_hash()));
                let sending_f = send!(-2006, "maker-payment", transaction.to_raw_bytes());
                swap.maker_payment = Some(transaction.clone());

                AtomicSwapState::WaitTakerPayment {sending_f}
            },
            AtomicSwapState::WaitTakerPayment {sending_f} => {
                let payload = recv!(sending_f, "taker-payment", "for Taker payment", swap.lock_duration / 3, -2006, {|_: &[u8]| Ok(())});

                let taker_payment = match swap.taker_coin.tx_enum_from_bytes(&payload) {
                    Ok(tx) => tx,
                    Err(err) => err!(-2006, "!taker_coin.tx_enum_from_bytes: "(err))
                };

                let validated = swap.taker_coin.validate_taker_payment(
                    taker_payment.clone(),
                    swap.taker_payment_lock as u32,
                    &*swap.other_persistent_pub,
                    &*swap.secret_hash,
                    swap.taker_amount,
                );

                if let Err(e) = validated {
                    err!(-2011, "!validate taker payment: "(e));
                }

                log!("Taker payment tx " (taker_payment.tx_hash()));
                swap.taker_payment = Some(taker_payment.clone());

                status.status(swap_tags, "Waiting for Taker payment confirmation…");
                let wait = swap.taker_coin.wait_for_confirmations(
                    taker_payment,
                    swap.taker_payment_confirmations,
                    wait_taker_payment,
                );

                if let Err(err) = wait {err!(-2006, "!taker_coin.wait_for_confirmations: "(err))}

                AtomicSwapState::SpendTakerPayment
            },
            AtomicSwapState::SpendTakerPayment => {
                let spend_fut = swap.taker_coin.send_maker_spends_taker_payment(
                    swap.taker_payment.clone().unwrap(),
                    &*swap.secret,
                );

                status.status(swap_tags, "Waiting for Taker payment to be spent…");
                let transaction = match spend_fut.wait() {
                    Ok(t) => t,
                    Err(err) => err!(-2007, "!send_maker_spends_taker_payment: "(err))
                };

                log!("Taker payment spend tx " (transaction.tx_hash()));
                status.status(swap_tags, &format!("{}/{} Swap finished successfully.", swap.maker_coin.ticker(), swap.taker_coin.ticker()));
                return Ok(());
            },
            AtomicSwapState::RefundMakerPayment => {
                // TODO cover this case
                status.status(swap_tags, "TBD. Swap finished in RefundMakerPayment.");
                return Ok(());
            },
            _ => unimplemented!(),
        };
        swap.state = Some(next_state);
    }
}

pub fn taker_swap_loop(swap: &mut AtomicSwap) -> Result<(), (i32, String)> {
    // NB: We can communicate the SWAP status to UI progress indicators via documented tags,
    // cf. https://github.com/artemii235/SuperNET/commit/d66ab944bfd8c5e8fb17f1d36ac303797156b88e#r31676734
    // (but first we need to establish a use case for such indication with the UI guys,
    //  in order to avoid premature throw-away design, cf. https://www.agilealliance.org/glossary/simple-design).
    let mut status = swap.ctx.log.status_handle();
    let uuid = swap.uuid.clone();
    let swap_tags: &[&TagParam] = &[&"swap", &("uuid", &uuid[..])];

    macro_rules! send {
        ($ec: expr, $subj: expr, $slice: expr) => {
            match send_! (&swap.ctx, swap.maker, fomat!(($subj) '@' (swap.uuid)), $slice) {
                Ok(h) => h,
                Err(err) => err!($ec, "send error: "(err))
            }
    }   }
    macro_rules! recv {
        ($subj: expr, $desc: expr, $timeout_sec: expr, $ec: expr, $validator: block) => {
            recv_! (swap, status, swap_tags, $subj, $desc, $timeout_sec, $ec, $validator)
        };
        // Use this form if there's a sending future to terminate upon receiving the answer.
        ($sending_f: ident, $subj: expr, $desc: expr, $timeout_sec: expr, $ec: expr, $validator: block) => {{
            let payload = recv_! (swap, status, swap_tags, $subj, $desc, $timeout_sec, $ec, $validator);
            drop ($sending_f);
            payload
        }};
    }
    // Note that `err!` updates the current `status`. We assume there is no blind spots in the `status`.
    // NB: If we want to replace the `err!` with `?` then we should move the `status` ownership to the call site.
    //     (Which IMHO would break the status code flow and encapsulation a little).
    macro_rules! err {
        ($ec: expr, $($msg: tt)+) => {{
            let mut msg = fomat! (' ' $($msg)+);
            status.append (&msg);
            msg.remove (0);
            return Err (($ec, msg))
        }};
    }

    if let Err(e) = swap.taker_coin.check_i_have_enough_to_trade(dstr(swap.taker_amount as i64), true).wait() {
        err!(-1000, "!check_i_have_enough_to_trade" [e]);
    };

    let started_at = now_ms() / 1000;
    swap.taker_payment_lock = started_at + swap.lock_duration;
    let maker_payment_wait = started_at + swap.lock_duration / 3;

    loop {
        let next_state = match unwrap!(swap.state.take()) {
            AtomicSwapState::Negotiation => {
                let data = recv!("negotiation", "for Maker negotiation data", 90, -1000, {|_: &[u8]| Ok(())});
                let maker_data: SwapNegotiationData = match deserialize(data.as_slice()) {
                    Ok(d) => d,
                    Err(e) => err!(-1001, "!negotiation-deserialize: " [e]),
                };

                let time_dif = (started_at as i64 - maker_data.started_at as i64).abs();
                if  time_dif > 60 {
                    // AG: I see this check failing with `LP_AUTOTRADE_TIMEOUT` bumped from 30 to 120.
                    //err!(-1002, "Started_at time_dif over 60: "(time_dif))
                    log!("Started_at time_dif over 60: "(time_dif));
                }
                swap.other_persistent_pub = maker_data.persistent_pubkey;
                swap.maker_payment_lock = maker_data.payment_locktime;
                swap.secret_hash = maker_data.secret_hash.clone();

                let taker_data = SwapNegotiationData {
                    started_at,
                    secret_hash: maker_data.secret_hash,
                    payment_locktime: swap.taker_payment_lock,
                    persistent_pubkey: swap.my_persistent_pub.clone(),
                };
                let bytes = serialize(&taker_data);
                let sending_f = send!(-1001, "negotiation-reply", bytes.as_slice());
                let data = recv!(sending_f, "negotiated", "for Maker negotiated", 90, -1000, {|_: &[u8]| Ok(())});
                let negotiated: bool = match deserialize(data.as_slice()) {
                    Ok(n) => n,
                    Err(e) => err!(-1001, "!negotiation-deserialize: " [e]),
                };

                if !negotiated {
                    err!(-1001, "!negotiated");
                }

                AtomicSwapState::SendTakerFee
            },
            AtomicSwapState::SendTakerFee => {
                let fee_addr_pub_key = unwrap!(hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06"));
                let fee_amount = swap.taker_amount / 777;
                status.status(swap_tags, "Sending Taker fee…");
                let fee_tx = swap.taker_coin.send_taker_fee(&fee_addr_pub_key, fee_amount as u64).wait();
                let transaction = match fee_tx {
                    Ok (t) => t,
                    Err (err) => err!(-1004, "!send_taker_fee: " (err))
                };

                log!("Taker fee tx hash " (transaction.tx_hash()));
                let sending_f = send!(-1004, "taker-fee", transaction.to_raw_bytes());

                AtomicSwapState::WaitMakerPayment {sending_f}
            },
            AtomicSwapState::WaitMakerPayment {sending_f} => {
                let payload = recv!(sending_f, "maker-payment", "for Maker payment", 600, -1005, {|_: &[u8]| Ok(())});
                let maker_payment = match swap.maker_coin.tx_enum_from_bytes(&payload) {
                    Ok(p) => p,
                    Err(err) => err!(-1005, "Error parsing the 'maker-payment': "(err))
                };

                let validated = swap.maker_coin.validate_maker_payment(
                    maker_payment.clone(),
                    swap.maker_payment_lock as u32,
                    &*swap.other_persistent_pub,
                    &*swap.secret_hash,
                    swap.maker_amount,
                );

                if let Err(e) = validated {
                    err!(-1011, "!validate maker payment: "(e));
                }

                log!("Got maker payment " (maker_payment.tx_hash()));
                swap.maker_payment = Some(maker_payment.clone());

                status.status(swap_tags, "Waiting for the confirmation of the Maker payment…");
                if let Err(err) = swap.maker_coin.wait_for_confirmations(
                    maker_payment,
                    swap.maker_payment_confirmations,
                    maker_payment_wait,
                ) {
                    err!(-1005, "!maker_coin.wait_for_confirmations: "(err))
                }

                AtomicSwapState::SendTakerPayment
            },
            AtomicSwapState::SendTakerPayment => {
                let payment_fut = swap.taker_coin.send_taker_payment(
                    swap.taker_payment_lock as u32,
                    &*swap.other_persistent_pub,
                    &*swap.secret_hash,
                    swap.taker_amount,
                );

                status.status(swap_tags, "Sending the Taker fee…");
                let transaction = match payment_fut.wait() {
                    Ok(t) => t,
                    Err(err) => err!(-1006, "!send_taker_payment: "(err))
                };

                log!("Taker payment tx hash " (transaction.tx_hash()));
                let msg = transaction.to_raw_bytes();

                let sending_f = send!(-1006, "taker-payment", msg);
                swap.taker_payment = Some(transaction.clone());

                AtomicSwapState::WaitTakerPaymentSpent {sending_f}
            },
            AtomicSwapState::WaitTakerPaymentSpent {sending_f} => {
                status.status(swap_tags, "Waiting for taker payment spend…");
                let got = swap.taker_coin.wait_for_tx_spend(swap.taker_payment.clone().unwrap(), swap.taker_payment_lock);
                drop(sending_f);

                match got {
                    Ok(transaction) => {
                        log!("Taker payment spend tx " (transaction.tx_hash()));
                        let secret = transaction.extract_secret();
                        if let Ok(bytes) = secret {
                            swap.secret = H256::from(bytes.as_slice());
                            AtomicSwapState::SpendMakerPayment
                        } else {
                            AtomicSwapState::RefundTakerPayment
                        }
                    },
                    Err(err) => {
                        status.append(&fomat!(" Error: "(err)));
                        AtomicSwapState::RefundTakerPayment
                    }
                }
            },
            AtomicSwapState::SpendMakerPayment => {
                // TODO: A human-readable label for send_taker_spends_maker_payment.
                status.status(swap_tags, "Spending maker payment…");
                let spend_fut = swap.maker_coin.send_taker_spends_maker_payment(
                    swap.maker_payment.clone().unwrap(),
                    &*swap.secret,
                );

                let transaction = match spend_fut.wait() {
                    Ok(t) => t,
                    Err(err) => err!(-1, "Error: "(err))
                };

                log!("Maker payment spend tx " (transaction.tx_hash()));
                status.status(swap_tags, &format!("{}/{} Swap finished successfully.", swap.maker_coin.ticker(), swap.taker_coin.ticker()));
                return Ok(());
            },
            AtomicSwapState::RefundTakerPayment => {
                status.status(swap_tags, "Refunding the Taker payment…");
                status.status(swap_tags, "Wait until payment is refundable…");
                loop {
                    if now_ms() / 1000 > swap.taker_payment_lock + 10 {
                        break;
                    }
                }
                let refund_fut = swap.taker_coin.send_taker_refunds_payment(
                    swap.taker_payment.clone().unwrap(),
                );

                let transaction = match refund_fut.wait() {
                    Ok(t) => t,
                    Err(err) => err!(-1, "Error: "(err))
                };
                log!("Taker refund tx hash " (transaction.tx_hash()));
                status.status(swap_tags, "Swap finished with refund.");
                return Ok(());
            },
            _ => unimplemented!(),
        };
        swap.state = Some(next_state);
    }
}
