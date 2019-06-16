
/******************************************************************************
 * Copyright © 2014-2019 The SuperNET Developers.                             *
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
//  ordermatch.rs
//  marketmaker
//
use bigdecimal::BigDecimal;
use bitcrypto::sha256;
use common::{lp, SMALLVAL, rpc_response, rpc_err_response, HyRes};
use common::mm_ctx::{from_ctx, MmArc, MmWeak};
use coins::{lp_coinfind, MmCoinEnum};
use coins::utxo::{compressed_pub_key_from_priv_raw, ChecksumType};
use futures::future::{Either, Future};
use gstuff::{now_ms, slurp};
use hashbrown::HashSet;
use hashbrown::hash_map::{Entry, HashMap};
use keys::{Public, Signature};
use libc::c_char;
use num_traits::cast::ToPrimitive;
use primitives::hash::H256;
use rpc::v1::types::{H256 as H256Json};
use serde_json::{self as json, Value as Json};
use std::ffi::{CString, OsStr};
use std::fs::{self, DirEntry};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::thread;
use uuid::Uuid;

use crate::mm2::lp_swap::{MakerSwap, run_maker_swap, TakerSwap, run_taker_swap};

#[cfg(test)]
#[path = "ordermatch_tests.rs"]
mod ordermatch_tests;

#[derive(Clone, Debug, Deserialize, Serialize)]
enum TakerAction {
    Buy,
    Sell,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TakerRequest {
    base: String,
    rel: String,
    base_amount: BigDecimal,
    rel_amount: BigDecimal,
    action: TakerAction,
    uuid: Uuid,
    method: String,
    sender_pubkey: H256Json,
    dest_pub_key: H256Json,
}

#[derive(Clone, Deserialize, Serialize)]
struct TakerOrder {
    created_at: u64,
    request: TakerRequest,
    matches: HashMap<Uuid, TakerMatch>
}

/// Result of match_reserved function
#[derive(Debug, PartialEq)]
enum MatchReservedResult {
    /// Order and reserved message matched,
    Matched,
    /// Order and reserved didn't match
    NotMatched,
}

impl TakerOrder {
    fn match_reserved(&self, reserved: &MakerReserved) -> MatchReservedResult {
        match self.request.action {
            TakerAction::Buy => if self.request.base == reserved.base && self.request.rel == reserved.rel
                && self.request.base_amount == reserved.base_amount && reserved.rel_amount <= self.request.rel_amount {
                MatchReservedResult::Matched
            } else {
                MatchReservedResult::NotMatched
            },
            TakerAction::Sell => if self.request.base == reserved.rel && self.request.rel == reserved.base
                && self.request.base_amount == reserved.rel_amount && self.request.rel_amount <= reserved.base_amount {
                MatchReservedResult::Matched
            } else {
                MatchReservedResult::NotMatched
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
/// Market maker order
/// The "action" is missing here because it's easier to always consider maker order as "sell"
/// So upon ordermatch with request we have only 2 combinations "sell":"sell" and "sell":"buy"
/// Adding "action" to maker order will just double possible combinations making order match more complex.
pub struct MakerOrder {
    pub max_base_vol: BigDecimal,
    pub min_base_vol: BigDecimal,
    pub price: BigDecimal,
    pub created_at: u64,
    pub base: String,
    pub rel: String,
    matches: HashMap<Uuid, MakerMatch>,
    started_swaps: Vec<Uuid>,
    uuid: Uuid,
}

impl MakerOrder {
    fn available_amount(&self) -> BigDecimal {
        let reserved: BigDecimal = self.matches.iter().fold(
            0.into(),
            |reserved, (_, order_match)| reserved + &order_match.reserved.base_amount
        );
        &self.max_base_vol - reserved
    }
}

impl Into<MakerOrder> for TakerOrder {
    fn into(self) -> MakerOrder {
        let order = match self.request.action {
            TakerAction::Sell => MakerOrder {
                price: &self.request.rel_amount / &self.request.base_amount,
                max_base_vol: self.request.base_amount,
                min_base_vol: 0.into(),
                created_at: now_ms(),
                base: self.request.base,
                rel: self.request.rel,
                matches: HashMap::new(),
                started_swaps: Vec::new(),
                uuid: self.request.uuid,
            },
            // The "buy" taker order is recreated with reversed pair as Maker order is always considered as "sell"
            TakerAction::Buy => MakerOrder {
                price: &self.request.base_amount / &self.request.rel_amount,
                max_base_vol: self.request.rel_amount,
                min_base_vol: 0.into(),
                created_at: now_ms(),
                base: self.request.rel,
                rel: self.request.base,
                matches: HashMap::new(),
                started_swaps: Vec::new(),
                uuid: self.request.uuid,
            },
        };
        order
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TakerConnect {
    taker_order_uuid: Uuid,
    maker_order_uuid: Uuid,
    method: String,
    sender_pubkey: H256Json,
    dest_pub_key: H256Json,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct MakerReserved {
    base: String,
    rel: String,
    base_amount: BigDecimal,
    rel_amount: BigDecimal,
    taker_order_uuid: Uuid,
    maker_order_uuid: Uuid,
    method: String,
    sender_pubkey: H256Json,
    dest_pub_key: H256Json,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct MakerConnected {
    taker_order_uuid: Uuid,
    maker_order_uuid: Uuid,
    method: String,
    sender_pubkey: H256Json,
    dest_pub_key: H256Json,
}

struct OrdermatchContext {
    pub my_maker_orders: Mutex<HashMap<Uuid, MakerOrder>>,
    pub my_taker_orders: Mutex<HashMap<Uuid, TakerOrder>>,
    pub cancelled_orders: Mutex<HashMap<Uuid, MakerOrder>>,
}

impl OrdermatchContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    fn from_ctx (ctx: &MmArc) -> Result<Arc<OrdermatchContext>, String> {
        Ok (try_s! (from_ctx (&ctx.ordermatch_ctx, move || {
            Ok (OrdermatchContext {
                my_taker_orders: Mutex::new (HashMap::default()),
                my_maker_orders: Mutex::new (HashMap::default()),
                cancelled_orders: Mutex::new (HashMap::default()),
            })
        })))
    }

    /// Obtains a reference to this crate context, creating it if necessary.
    #[allow(dead_code)]
    fn from_ctx_weak (ctx_weak: &MmWeak) -> Result<Arc<OrdermatchContext>, String> {
        let ctx = try_s! (MmArc::from_weak (ctx_weak) .ok_or ("Context expired"));
        Self::from_ctx (&ctx)
    }
}

unsafe fn lp_connect_start_bob(ctx: &MmArc, maker_match: &MakerMatch) -> i32 {
    let mut retval = -1;
    let loop_thread = thread::Builder::new().name("maker_loop".into()).spawn({
        let taker_coin = unwrap!(unwrap! (lp_coinfind (ctx, &maker_match.reserved.rel)));
        let maker_coin = unwrap!(unwrap! (lp_coinfind (ctx, &maker_match.reserved.base)));
        let ctx = ctx.clone();
        let mut alice = lp::bits256::default();
        alice.bytes = maker_match.request.sender_pubkey.0;
        let maker_amount = maker_match.reserved.base_amount.clone();
        let taker_amount = maker_match.reserved.rel_amount.clone();
        let my_persistent_pub = unwrap!(compressed_pub_key_from_priv_raw(&lp::G.LP_privkey.bytes, ChecksumType::DSHA256));
        let uuid = maker_match.request.uuid.to_string();
        move || {
            log!("Entering the maker_swap_loop " (maker_coin.ticker()) "/" (taker_coin.ticker()));
            let maker_swap = MakerSwap::new(
                ctx,
                alice,
                maker_coin,
                taker_coin,
                maker_amount,
                taker_amount,
                my_persistent_pub,
                uuid,
            );
            run_maker_swap(maker_swap, None);
        }
    });
    match loop_thread {
        Ok(_h) => {
            retval = 0;
        },
        Err(e) => {
            log!({ "Got error launching bob swap loop: {}", e });
        }
    }
    retval
}

unsafe fn lp_connected_alice(ctx: &MmArc, taker_match: &TakerMatch) { // alice
    let alice_loop_thread = thread::Builder::new().name("taker_loop".into()).spawn({
        let ctx = ctx.clone();
        let mut maker = lp::bits256::default();
        maker.bytes = taker_match.reserved.sender_pubkey.0;
        let taker_coin = unwrap!(unwrap! (lp_coinfind (&ctx, &taker_match.reserved.rel)));
        let maker_coin = unwrap!(unwrap! (lp_coinfind (&ctx, &taker_match.reserved.base)));
        let my_persistent_pub = unwrap!(compressed_pub_key_from_priv_raw(&lp::G.LP_privkey.bytes, ChecksumType::DSHA256));
        let maker_amount = taker_match.reserved.base_amount.clone();
        let taker_amount = taker_match.reserved.rel_amount.clone();
        let uuid = taker_match.reserved.taker_order_uuid.to_string();
        move || {
            log!("Entering the taker_swap_loop " (maker_coin.ticker()) "/" (taker_coin.ticker()));
            let taker_swap = TakerSwap::new(
                ctx,
                maker,
                maker_coin,
                taker_coin,
                maker_amount,
                taker_amount,
                my_persistent_pub,
                uuid,
            );
            run_taker_swap(taker_swap, None);
        }
    });
    match alice_loop_thread {
        Ok(_) => (),
        Err(e) => {
            log!({ "Got error trying to start taker loop {}", e });
        }
    }
}

pub fn lp_trades_loop(ctx: MmArc) {
    const ORDERMATCH_TIMEOUT: u64 = 30000;
    let mut last_price_broadcast = 0;

    loop {
        if ctx.is_stopping() { break }
        let ordermatch_ctx = unwrap!(OrdermatchContext::from_ctx(&ctx));
        let mut my_taker_orders = unwrap!(ordermatch_ctx.my_taker_orders.lock());
        let mut my_maker_orders = unwrap!(ordermatch_ctx.my_maker_orders.lock());
        // move the timed out and unmatched taker orders to maker
        *my_taker_orders = my_taker_orders.drain().filter_map(|(uuid, order)| if order.created_at + ORDERMATCH_TIMEOUT < now_ms() {
            delete_my_taker_order(&ctx, &order);
            if order.matches.is_empty() {
                let maker_order = order.into();
                save_my_maker_order(&ctx, &maker_order);
                my_maker_orders.insert(uuid, maker_order);
            }
            None
        } else {
            Some((uuid, order))
        }).collect();
        // remove timed out unfinished matches to unlock the reserved amount
        my_maker_orders.iter_mut().for_each(|(_, order)| {
            order.matches = order.matches.drain().filter(
                |(_, order_match)| order_match.last_updated + ORDERMATCH_TIMEOUT > now_ms() || order_match.connected.is_some()
            ).collect();
            save_my_maker_order(&ctx, order);
        });
        drop(my_taker_orders);
        drop(my_maker_orders);

        if now_ms() > last_price_broadcast + 10000 {
            if let Err(e) = broadcast_my_maker_orders(&ctx) {
                ctx.log.log("", &[&"broadcast_my_maker_orders"], &format!("error {}", e));
            }
            last_price_broadcast = now_ms();
        }
        thread::sleep(Duration::from_secs(1));
    }
}

pub unsafe fn lp_trade_command(
    ctx: MmArc,
    json: Json,
) -> i32 {
    let method = json["method"].as_str();
    let ordermatch_ctx = unwrap!(OrdermatchContext::from_ctx(&ctx));
    if method == Some("reserved") {
        let reserved_msg: MakerReserved = match json::from_value(json.clone()) {
            Ok(r) => r,
            Err(_) => return 1,
        };
        if H256Json::from(lp::G.LP_mypub25519.bytes) != reserved_msg.dest_pub_key {
            // ignore the messages that do not target our node
            return 1;
        }

        let mut my_taker_orders = unwrap!(ordermatch_ctx.my_taker_orders.lock());
        let my_order = match my_taker_orders.entry(reserved_msg.taker_order_uuid) {
            Entry::Vacant(_) => {
                log!("Our node doesn't have the order with uuid " (reserved_msg.taker_order_uuid));
                return 1;
            },
            Entry::Occupied(entry) => entry.into_mut()
        };

        if my_order.request.dest_pub_key != H256Json::default() && my_order.request.dest_pub_key != reserved_msg.sender_pubkey {
            log!("got reserved response from different node " (hex::encode(&reserved_msg.sender_pubkey.0)));
            return 1;
        }

        // send "connect" message if reserved message targets our pubkey AND
        // reserved amounts match our order AND order is NOT reserved by someone else (empty matches)
        if my_order.match_reserved(&reserved_msg) == MatchReservedResult::Matched && my_order.matches.is_empty() {
            let connect = TakerConnect {
                sender_pubkey: H256Json::from(lp::G.LP_mypub25519.bytes),
                dest_pub_key: reserved_msg.sender_pubkey.clone(),
                method: "connect".into(),
                taker_order_uuid: reserved_msg.taker_order_uuid,
                maker_order_uuid: reserved_msg.maker_order_uuid,
            };
            ctx.broadcast_p2p_msg(&unwrap!(json::to_string(&connect)));
            let taker_match = TakerMatch {
                reserved: reserved_msg,
                connect,
                connected: None,
                last_updated: now_ms(),
            };
            my_order.matches.insert(taker_match.reserved.maker_order_uuid, taker_match);
            save_my_taker_order(&ctx, &my_order);
        }
        return 1;
    }
    if method == Some("connected") {
        let connected: MakerConnected = match json::from_value(json.clone()) {
            Ok(c) => c,
            Err(_) => return 1,
        };
        if H256Json::from(lp::G.LP_mypub25519.bytes) == connected.dest_pub_key && H256Json::from(lp::G.LP_mypub25519.bytes) != connected.sender_pubkey {
            let mut my_taker_orders = unwrap!(ordermatch_ctx.my_taker_orders.lock());
            let my_order = match my_taker_orders.get_mut(&connected.taker_order_uuid) {
                Some(o) => o,
                None => {
                    log!("Our node doesn't have the order with uuid "(connected.taker_order_uuid));
                    return 1;
                },
            };
            let order_match = match my_order.matches.get_mut(&connected.maker_order_uuid) {
                Some(o) => o,
                None => {
                    log!("Our node doesn't have the match with uuid "(connected.maker_order_uuid));
                    return 1;
                }
            };
            order_match.connected = Some(connected);
            order_match.last_updated = now_ms();
            // alice
            lp_connected_alice(
                &ctx,
                order_match,
            );
            save_my_taker_order(&ctx, &my_order);
            // AG: Bob's p2p ID (`LP_mypub25519`) is in `json["srchash"]`.
            log!("CONNECTED.(" (json) ")");
        }
        return 1;
    }
    // bob
    if method == Some("request") {
        let taker_request: TakerRequest = match json::from_value(json.clone()) {
            Ok(r) => r,
            Err(_) => return 1,
        };
        if lp::G.LP_mypub25519.bytes == taker_request.dest_pub_key.0 {
            log!("Skip the request originating from our pubkey");
            return 1;
        }
        let ordermatch_ctx = unwrap!(OrdermatchContext::from_ctx(&ctx));
        let mut my_orders = unwrap!(ordermatch_ctx.my_maker_orders.lock());

        for (uuid, order) in my_orders.iter_mut() {
            if let OrderMatchResult::Matched((base_amount, rel_amount)) = match_order_and_request(order, &taker_request) {
                let reserved = MakerReserved {
                    dest_pub_key: taker_request.sender_pubkey.clone(),
                    sender_pubkey: lp::G.LP_mypub25519.bytes.into(),
                    base: order.base.clone(),
                    base_amount,
                    rel_amount,
                    rel: order.rel.clone(),
                    method: "reserved".into(),
                    taker_order_uuid: taker_request.uuid,
                    maker_order_uuid: *uuid,
                };
                ctx.broadcast_p2p_msg(&unwrap!(json::to_string(&reserved)));
                let maker_match = MakerMatch {
                    request: taker_request,
                    reserved,
                    connect: None,
                    connected: None,
                    last_updated: now_ms(),
                };
                order.matches.insert(maker_match.request.uuid, maker_match);
                save_my_maker_order(&ctx, &order);
                return 1;
            }
        }
    }

    if method == Some("connect") {
        // bob
        let connect_msg: TakerConnect = match json::from_value(json.clone()) {
            Ok(m) => m,
            Err(_) => return 1,
        };
        if lp::G.LP_mypub25519.bytes == connect_msg.dest_pub_key.0 && lp::G.LP_mypub25519.bytes != connect_msg.sender_pubkey.0 {
            let mut maker_orders = unwrap!(ordermatch_ctx.my_maker_orders.lock());
            let my_order = match maker_orders.get_mut(&connect_msg.maker_order_uuid) {
                Some(o) => o,
                None => {
                    log!("Our node doesn't have the order with uuid " (connect_msg.maker_order_uuid));
                    return 1;
                },
            };
            let order_match = match my_order.matches.get_mut(&connect_msg.taker_order_uuid) {
                Some(o) => o,
                None => {
                    log!("Our node doesn't have the match with uuid " (connect_msg.taker_order_uuid));
                    return 1;
                },
            };

            let connected = MakerConnected {
                sender_pubkey: lp::G.LP_mypub25519.bytes.into(),
                dest_pub_key: connect_msg.sender_pubkey.clone(),
                taker_order_uuid: connect_msg.taker_order_uuid,
                maker_order_uuid: connect_msg.maker_order_uuid,
                method: "connected".into(),
            };
            ctx.broadcast_p2p_msg(&unwrap!(json::to_string(&connected)));
            order_match.connect = Some(connect_msg);
            order_match.connected = Some(connected);
            my_order.started_swaps.push(order_match.request.uuid);
            lp_connect_start_bob(&ctx, order_match);
            save_my_maker_order(&ctx, &my_order);
        }
        return 1;
    }
    -1
}

#[derive(Deserialize, Debug)]
pub struct AutoBuyInput {
    base: String,
    rel: String,
    price: BigDecimal,
    volume: BigDecimal,
    timeout: Option<u32>,
    /// Not used. Deprecated.
    duration: Option<u32>,
    // TODO: remove this field on API refactoring, method should be separated from params
    method: String,
    gui: Option<String>,
    #[serde(rename="destpubkey")]
    #[serde(default)]
    dest_pub_key: H256Json
}

pub fn buy(ctx: MmArc, json: Json) -> HyRes {
    let input : AutoBuyInput = try_h!(json::from_value(json.clone()));
    if input.base == input.rel {
        return rpc_err_response(500, "Base and rel must be different coins");
    }
    let rel_coin = try_h!(lp_coinfind(&ctx, &input.rel));
    let rel_coin = match rel_coin {Some(c) => c, None => return rpc_err_response(500, "Rel coin is not found or inactive")};
    let base_coin = try_h!(lp_coinfind(&ctx, &input.base));
    let base_coin: MmCoinEnum = match base_coin {Some(c) => c, None => return rpc_err_response(500, "Base coin is not found or inactive")};
    Box::new(rel_coin.check_i_have_enough_to_trade(&input.volume * &input.price, false).and_then(move |_|
        base_coin.can_i_spend_other_payment().and_then(move |_|
            rpc_response(200, try_h!(lp_auto_buy(&ctx, input)))
        )
    ))
}

pub fn sell(ctx: MmArc, json: Json) -> HyRes {
    let input : AutoBuyInput = try_h!(json::from_value(json.clone()));
    if input.base == input.rel {
        return rpc_err_response(500, "Base and rel must be different coins");
    }
    let base_coin = try_h!(lp_coinfind(&ctx, &input.base));
    let base_coin = match base_coin {Some(c) => c, None => return rpc_err_response(500, "Base coin is not found or inactive")};
    let rel_coin = try_h!(lp_coinfind(&ctx, &input.rel));
    let rel_coin = match rel_coin {Some(c) => c, None => return rpc_err_response(500, "Rel coin is not found or inactive")};
    Box::new(base_coin.check_i_have_enough_to_trade(input.volume.clone(), false).and_then(move |_|
        rel_coin.can_i_spend_other_payment().and_then(move |_|
            rpc_response(200, try_h!(lp_auto_buy(&ctx, input)))
        )
    ))
}

/// Created when maker order is matched with taker request
#[derive(Clone, Debug, Deserialize, Serialize)]
struct MakerMatch {
    request: TakerRequest,
    reserved: MakerReserved,
    connect: Option<TakerConnect>,
    connected: Option<MakerConnected>,
    last_updated: u64,
}

/// Created upon taker request broadcast
#[derive(Clone, Deserialize, Serialize)]
struct TakerMatch {
    reserved: MakerReserved,
    connect: TakerConnect,
    connected: Option<MakerConnected>,
    last_updated: u64,
}

pub fn lp_auto_buy(ctx: &MmArc, input: AutoBuyInput) -> Result<String, String> {
    if input.price < SMALLVAL.into() {
        return ERR!("Price is too low, minimum is {}", SMALLVAL);
    }

    let action = match Some(input.method.as_ref()) {
        Some("buy") => {
            TakerAction::Buy
        },
        Some("sell") => {
            TakerAction::Sell
        },
        _ => return ERR!("Auto buy must be called only from buy/sell RPC methods")
    };

    let ordermatch_ctx = try_s!(OrdermatchContext::from_ctx(&ctx));
    let mut my_taker_orders = try_s!(ordermatch_ctx.my_taker_orders.lock());
    let uuid = Uuid::new_v4();
    let request = TakerRequest {
        base: input.base,
        rel: input.rel,
        rel_amount: &input.volume * input.price,
        base_amount: input.volume,
        method: "request".into(),
        uuid,
        dest_pub_key: input.dest_pub_key,
        sender_pubkey: H256Json::from(unsafe { lp::G.LP_mypub25519.bytes }),
        action,
    };
    ctx.broadcast_p2p_msg(&unwrap!(json::to_string(&request)));
    let result = json!({
        "result": request
    }).to_string();
    let order = TakerOrder {
        created_at: now_ms(),
        matches: HashMap::new(),
        request,
    };
    save_my_taker_order(ctx, &order);
    my_taker_orders.insert(uuid, order);
    drop(my_taker_orders);
    Ok(result)
}

fn price_ping_sig_hash(timestamp: u32, pubsecp: &[u8], pubkey: &[u8], base: &[u8], rel: &[u8], price64: u64) -> H256 {
    let mut input = vec![];
    input.extend_from_slice(&timestamp.to_le_bytes());
    input.extend_from_slice(pubsecp);
    input.extend_from_slice(pubkey);
    input.extend_from_slice(base);
    input.extend_from_slice(rel);
    input.extend_from_slice(&price64.to_le_bytes());
    sha256(&input)
}

#[derive(Debug, Deserialize, Serialize)]
struct PricePingRequest {
    method: String,
    pubkey: String,
    base: String,
    rel: String,
    price: BigDecimal,
    price64: String,
    timestamp: u64,
    pubsecp: String,
    sig: String,
    // TODO rename, it's called "balance", but it's actual meaning is max available volume to trade
    #[serde(rename="bal")]
    balance: BigDecimal,
}

impl PricePingRequest {
    fn new(ctx: &MmArc, order: &MakerOrder) -> Result<PricePingRequest, String> {
        let base_coin = match try_s!(lp_coinfind(ctx, &order.base)) {
            Some(coin) => coin,
            None => return ERR!("Base coin {} is not found", order.base),
        };

        let _rel_coin = match try_s!(lp_coinfind(ctx, &order.rel)) {
            Some(coin) => coin,
            None => return ERR!("Rel coin {} is not found", order.rel),
        };

        let price64 = (&order.price * BigDecimal::from(100000000)).to_u64().unwrap();
        let timestamp = now_ms() / 1000;
        let sig_hash = price_ping_sig_hash(
            timestamp as u32,
            &**ctx.secp256k1_key_pair().public(),
            unsafe { &lp::G.LP_mypub25519.bytes },
            order.base.as_bytes(),
            order.rel.as_bytes(),
            price64,
        );

        let sig = try_s!(ctx.secp256k1_key_pair().private().sign(&sig_hash));

        let available_amount = order.available_amount();
        let max_volume = if available_amount > 0.into() {
            let my_balance = try_s!(base_coin.my_balance().wait());
            if available_amount <= my_balance && available_amount > 0.into() {
                available_amount
            } else {
                my_balance
            }
        } else {
            0.into()
        };

        Ok(PricePingRequest {
            method: "postprice".into(),
            pubkey: unsafe { hex::encode(&lp::G.LP_mypub25519.bytes) },
            base: order.base.clone(),
            rel: order.rel.clone(),
            price64: price64.to_string(),
            price: order.price.clone(),
            timestamp,
            pubsecp: unsafe { hex::encode(&lp::G.LP_pubsecp.to_vec()) },
            sig: hex::encode(&*sig),
            balance: max_volume,
        })
    }
}

pub fn lp_post_price_recv(ctx: &MmArc, req: Json) -> HyRes {
    let req: PricePingRequest = try_h!(json::from_value(req));
    let signature: Signature = try_h!(req.sig.parse());
    let pub_secp = try_h!(Public::from_slice(&try_h!(hex::decode(&req.pubsecp))));
    let pubkey = try_h!(hex::decode(&req.pubkey));
    let sig_hash = price_ping_sig_hash(
        req.timestamp as u32,
        &*pub_secp,
        &pubkey,
        req.base.as_bytes(),
        req.rel.as_bytes(),
        try_h!(req.price64.parse()),
    );
    let sig_check = try_h!(pub_secp.verify(&sig_hash, &signature));
    if sig_check {
        unsafe {
            let mut pubkey_bits = lp::bits256::default();
            pubkey_bits.bytes.copy_from_slice(&pubkey);
            let base = try_h!(CString::new(req.base));
            let rel = try_h!(CString::new(req.rel));
            lp::LP_pricefeedupdate(
                pubkey_bits,
                base.as_ptr() as *mut c_char,
                rel.as_ptr() as *mut c_char,
                req.price.to_f64().unwrap(),
                req.balance.to_f64().unwrap(),
                0,
            );
        }
        rpc_response(200, r#"{"result":"success"}"#)
    } else {
        rpc_err_response(400, "price ping invalid signature")
    }
}

fn lp_send_price_ping(req: &PricePingRequest, ctx: &MmArc) -> Result<(), String> {
    let req_string = try_s!(json::to_string(req));
    // TODO this is required to process the set price message on our own node, it's the easiest way now
    //      there might be a better way of doing this so we should consider refactoring
    lp_post_price_recv(ctx, try_s!(json::to_value(req)));
    ctx.broadcast_p2p_msg(&req_string);
    Ok(())
}

fn one() -> u8 { 1 }

#[derive(Deserialize)]
struct SetPriceReq {
    base: String,
    rel: String,
    price: BigDecimal,
    #[serde(default)]
    max: bool,
    #[serde(default = "one")]
    broadcast: u8,
    #[serde(default)]
    volume: BigDecimal,
}

pub fn set_price(ctx: MmArc, req: Json) -> HyRes {
    let req: SetPriceReq = try_h!(json::from_value(req));
    if req.base == req.rel {
        return rpc_err_response(500, "Base and rel must be different coins");
    }

    let base_coin: MmCoinEnum = match try_h!(lp_coinfind(&ctx, &req.base)) {
        Some(coin) => coin,
        None => return rpc_err_response(500, &format!("Base coin {} is not found", req.base)),
    };

    let rel_coin: MmCoinEnum = match try_h!(lp_coinfind(&ctx, &req.rel)) {
        Some(coin) => coin,
        None => return rpc_err_response(500, &format!("Rel coin {} is not found", req.rel)),
    };

    let volume_f = if req.max {
        // entire balance deducted by 0.1 to reserve for tx fees
        Either::A(base_coin.my_balance().map(|balance| balance - "0.1".parse::<BigDecimal>().unwrap()))
    } else {
        Either::B(futures::future::ok(req.volume.clone()))
    };

    Box::new(
        volume_f.and_then(move |volume| {
            base_coin.check_i_have_enough_to_trade(volume.clone(), true).and_then(move |_|
                rel_coin.can_i_spend_other_payment().and_then(move |_| {
                    let ordermatch_ctx = try_h!(OrdermatchContext::from_ctx(&ctx));
                    let mut my_orders = try_h!(ordermatch_ctx.my_maker_orders.lock());
                    // remove the previous order if there's one to allow multiple setprice call per pair
                    // it's common use case now as `autoprice` doesn't work with new ordermatching and
                    // MM2 users request the coins price from aggregators by their own scripts issuing
                    // repetitive setprice calls with new price
                    *my_orders = my_orders.drain().filter(|(_, order)| !(order.base == req.base && order.rel == req.rel)).collect();

                    let uuid = Uuid::new_v4();
                    let order = MakerOrder {
                        max_base_vol: volume,
                        min_base_vol: 0.into(),
                        price: req.price,
                        created_at: now_ms(),
                        base: req.base,
                        rel: req.rel,
                        matches: HashMap::new(),
                        started_swaps: Vec::new(),
                        uuid,
                    };
                    let response = json!({"result":order}).to_string();
                    save_my_maker_order(&ctx, &order);
                    my_orders.insert(uuid, order);
                    rpc_response(200, response)
                })
            )
        })
    )
}

pub fn broadcast_my_maker_orders(ctx: &MmArc) -> Result<(), String> {
    let ordermatch_ctx = try_s!(OrdermatchContext::from_ctx(ctx));
    let my_orders = try_s!(ordermatch_ctx.my_maker_orders.lock()).clone();

    for (_, order) in my_orders.iter() {
        let ping = match PricePingRequest::new(ctx, order) {
            Ok(p) => p,
            Err(e) => {
                ctx.log.log("", &[&"broadcast_my_maker_orders", &order.base, &order.rel], &format! ("ping request creation failed {}", e));
                continue;
            },
        };

        if let Err(e) = lp_send_price_ping(&ping, ctx) {
            ctx.log.log("", &[&"broadcast_my_maker_orders", &order.base, &order.rel], &format! ("ping request send failed {}", e));
            continue;
        }
    }
    // the difference of cancelled orders from maker orders that we broadcast the cancel request only once
    // cancelled record can be just dropped then
    let cancelled_orders: HashMap<_, _> = try_s!(ordermatch_ctx.cancelled_orders.lock()).drain().collect();
    for (_, order) in cancelled_orders {
        let ping = match PricePingRequest::new(ctx, &order) {
            Ok(p) => p,
            Err(e) => {
                ctx.log.log("", &[&"broadcast_cancelled_orders", &order.base, &order.rel], &format! ("ping request creation failed {}", e));
                continue;
            },
        };

        if let Err(e) = lp_send_price_ping(&ping, ctx) {
            ctx.log.log("", &[&"broadcast_cancelled_orders", &order.base, &order.rel], &format! ("ping request send failed {}", e));
            continue;
        }
    }

    Ok(())
}

/// Result of match_order_and_request function
#[derive(Debug, PartialEq)]
enum OrderMatchResult {
    /// Order and request matched, contains base and rel resulting amounts
    Matched((BigDecimal, BigDecimal)),
    /// Orders didn't match
    NotMatched,
}

/// Attempts to match the Maker's order and Taker's request
fn match_order_and_request(maker: &MakerOrder, taker: &TakerRequest) -> OrderMatchResult {
    match taker.action {
        TakerAction::Buy => {
            if maker.base == taker.base && maker.rel == taker.rel && taker.base_amount <= maker.available_amount() && taker.base_amount >= maker.min_base_vol {
                let taker_price = &taker.rel_amount / &taker.base_amount;
                if taker_price >= maker.price {
                    OrderMatchResult::Matched((taker.base_amount.clone(), &taker.base_amount * &maker.price))
                } else {
                    OrderMatchResult::NotMatched
                }
            } else {
                OrderMatchResult::NotMatched
            }
        },
        TakerAction::Sell => {
            if maker.base == taker.rel && maker.rel == taker.base && taker.rel_amount <= maker.available_amount() && taker.rel_amount >= maker.min_base_vol {
                let taker_price = &taker.base_amount / &taker.rel_amount;
                if taker_price >= maker.price {
                    OrderMatchResult::Matched((&taker.base_amount / &maker.price, taker.base_amount.clone()))
                } else {
                    OrderMatchResult::NotMatched
                }
            } else {
                OrderMatchResult::NotMatched
            }
        },
    }
}

#[derive(Deserialize)]
struct OrderStatusReq {
    uuid: Uuid,
}

pub fn order_status(ctx: MmArc, req: Json) -> HyRes {
    let req: OrderStatusReq = try_h!(json::from_value(req));

    let ordermatch_ctx = try_h!(OrdermatchContext::from_ctx(&ctx));
    let maker_orders = try_h!(ordermatch_ctx.my_maker_orders.lock());
    if let Some(order) = maker_orders.get(&req.uuid) {
        return rpc_response(200, json!({
            "type": "Maker",
            "order": order,
            "available_amount": order.available_amount(),
        }).to_string());
    }

    let taker_orders = try_h!(ordermatch_ctx.my_taker_orders.lock());
    if let Some(order) = taker_orders.get(&req.uuid) {
        return rpc_response(200, json!({
            "type": "Taker",
            "order": order,
        }).to_string());
    }

    rpc_err_response(404, &format!("Order with uuid {} is not found", req.uuid))
}

#[derive(Deserialize)]
struct CancelOrderReq {
    uuid: Uuid,
}

pub fn cancel_order(ctx: MmArc, req: Json) -> HyRes {
    let req: CancelOrderReq = try_h!(json::from_value(req));

    let ordermatch_ctx = try_h!(OrdermatchContext::from_ctx(&ctx));
    let mut maker_orders = try_h!(ordermatch_ctx.my_maker_orders.lock());
    match maker_orders.remove(&req.uuid) {
        Some(mut order) => {
            let mut cancelled_orders = try_h!(ordermatch_ctx.cancelled_orders.lock());
            // TODO cancel means setting the volume to 0 as of now, should refactor
            order.max_base_vol = 0.into();
            delete_my_maker_order(&ctx, &order);
            cancelled_orders.insert(req.uuid, order);
            return rpc_response(200, json!({
                "result": "success"
            }).to_string())
        },
        // look for taker order with provided uuid
        None => (),
    }

    let mut taker_orders = try_h!(ordermatch_ctx.my_taker_orders.lock());
    match taker_orders.remove(&req.uuid) {
        Some(order) => {
            delete_my_taker_order(&ctx, &order);
            return rpc_response(200, json!({
                "result": "success"
            }).to_string())
        },
        // error is returned
        None => (),
    }

    rpc_err_response(404, &format!("Order with uuid {} is not found", req.uuid))
}

pub fn my_orders(ctx: MmArc) -> HyRes {
    let ordermatch_ctx = try_h!(OrdermatchContext::from_ctx(&ctx));
    let maker_orders = try_h!(ordermatch_ctx.my_maker_orders.lock());
    let taker_orders = try_h!(ordermatch_ctx.my_taker_orders.lock());

    rpc_response(200, json!({
        "result": {
            "maker_orders": *maker_orders,
            "taker_orders": *taker_orders,
        }
    }).to_string())
}

fn my_maker_orders_dir(ctx: &MmArc) -> PathBuf {
    ctx.dbdir().join("ORDERS").join("MY").join("MAKER")
}

fn my_taker_orders_dir(ctx: &MmArc) -> PathBuf {
    ctx.dbdir().join("ORDERS").join("MY").join("TAKER")
}

fn my_maker_order_file_path(ctx: &MmArc, uuid: &Uuid) -> PathBuf {
    my_maker_orders_dir(ctx).join(format!("{}.json", uuid))
}

fn my_taker_order_file_path(ctx: &MmArc, uuid: &Uuid) -> PathBuf {
    my_taker_orders_dir(ctx).join(format!("{}.json", uuid))
}

fn save_my_maker_order(ctx: &MmArc, order: &MakerOrder) {
    let path = my_maker_order_file_path(ctx, &order.uuid);
    let content = unwrap!(json::to_vec(order));
    unwrap!(fs::write(path, content));
}

fn save_my_taker_order(ctx: &MmArc, order: &TakerOrder) {
    let path = my_taker_order_file_path(ctx, &order.request.uuid);
    let content = unwrap!(json::to_vec(order));
    unwrap!(fs::write(path, content));
}

fn delete_my_maker_order(ctx: &MmArc, order: &MakerOrder) {
    unwrap!(fs::remove_file(my_maker_order_file_path(ctx, &order.uuid)));
}

fn delete_my_taker_order(ctx: &MmArc, order: &TakerOrder) {
    unwrap!(fs::remove_file(my_taker_order_file_path(ctx, &order.request.uuid)));
}

pub fn orders_kick_start(ctx: &MmArc) -> Result<HashSet<String>, String> {
    let mut coins = HashSet::new();
    let ordermatch_ctx = try_s!(OrdermatchContext::from_ctx(ctx));
    let mut maker_orders = try_s!(ordermatch_ctx.my_maker_orders.lock());
    let entries: Vec<DirEntry> = try_s!(my_maker_orders_dir(&ctx).read_dir()).filter_map(|dir_entry| {
        let entry = match dir_entry {
            Ok(ent) => ent,
            Err(e) => {
                log!("Error " (e) " reading from dir " (my_maker_orders_dir(&ctx).display()));
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
        match json::from_slice::<MakerOrder>(&slurp(&entry.path())) {
            Ok(order) => {
                coins.insert(order.base.clone());
                coins.insert(order.rel.clone());
                maker_orders.insert(order.uuid, order);
            }
            Err(_) => (),
        }
    });

    let mut taker_orders = try_s!(ordermatch_ctx.my_taker_orders.lock());
    let entries: Vec<DirEntry> = try_s!(my_taker_orders_dir(&ctx).read_dir()).filter_map(|dir_entry| {
        let entry = match dir_entry {
            Ok(ent) => ent,
            Err(e) => {
                log!("Error " (e) " reading from dir " (my_taker_orders_dir(&ctx).display()));
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
        match json::from_slice::<TakerOrder>(&slurp(&entry.path())) {
            Ok(order) => {
                coins.insert(order.request.base.clone());
                coins.insert(order.request.rel.clone());
                taker_orders.insert(order.request.uuid, order);
            }
            Err(_) => (),
        }
    });
    Ok(coins)
}
