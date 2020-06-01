
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
//  lp_ordermatch.rs
//  marketmaker
//
#![allow(uncommon_codepoints)]
#![cfg_attr(not(feature = "native"), allow(dead_code))]

use bigdecimal::BigDecimal;
use bitcrypto::sha256;
use coins::{lp_coinfindᵃ, MmCoinEnum, TradeInfo};
use coins::utxo::{compressed_pub_key_from_priv_raw, ChecksumType};
use common::{bits256, json_dir_entries, now_ms, new_uuid,
  remove_file, rpc_response, rpc_err_response, write, HyRes};
use common::executor::{spawn, Timer};
use common::mm_ctx::{from_ctx, MmArc, MmWeak};
use common::mm_number::{from_dec_to_ratio, from_ratio_to_dec, Fraction, MmNumber};
use futures::compat::Future01CompatExt;
use gstuff::slurp;
use http::Response;
use keys::{Public, Signature};
#[cfg(test)]
use mocktopus::macros::*;
use num_rational::BigRational;
use num_traits::cast::ToPrimitive;
use num_traits::identities::Zero;
use primitives::hash::{H256};
use rpc::v1::types::{H256 as H256Json};
use serde_json::{self as json, Value as Json};
use std::collections::HashSet;
use std::collections::hash_map::{Entry, HashMap};
use std::fs::DirEntry;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::mm2::lp_swap::{dex_fee_amount, get_locked_amount, is_pubkey_banned, MakerSwap,
                          RunMakerSwapInput, RunTakerSwapInput, run_maker_swap, run_taker_swap, TakerSwap};

#[cfg(test)]
#[cfg(feature = "native")]
#[path = "ordermatch_tests.rs"]
mod ordermatch_tests;

const MIN_TRADING_VOL: &str = "0.00777";

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
    base_amount_rat: Option<BigRational>,
    rel_amount: BigDecimal,
    rel_amount_rat: Option<BigRational>,
    action: TakerAction,
    uuid: Uuid,
    method: String,
    sender_pubkey: H256Json,
    dest_pub_key: H256Json,
    #[serde(default)]
    match_by: MatchBy,
}

impl TakerRequest {
    fn get_base_amount(&self) -> MmNumber {
        match &self.base_amount_rat {
            Some(r) => r.clone().into(),
            None => self.base_amount.clone().into()
        }
    }

    fn get_rel_amount(&self) -> MmNumber {
        match &self.rel_amount_rat {
            Some(r) => r.clone().into(),
            None => self.rel_amount.clone().into()
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", content = "data")]
enum MatchBy {
    Any,
    Orders(HashSet<Uuid>),
    Pubkeys(HashSet<H256Json>)
}

impl Default for MatchBy {
    fn default() -> Self { MatchBy::Any }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "data")]
enum OrderType {
    FillOrKill,
    GoodTillCancelled,
}

impl Default for OrderType {
    fn default() -> Self { OrderType::GoodTillCancelled }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TakerOrder {
    created_at: u64,
    request: TakerRequest,
    matches: HashMap<Uuid, TakerMatch>,
    order_type: OrderType,
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
    fn is_cancellable(&self) -> bool {
        self.matches.is_empty()
    }

    fn match_reserved(&self, reserved: &MakerReserved) -> MatchReservedResult {
        match &self.request.match_by {
            MatchBy::Any => (),
            MatchBy::Orders(uuids) => if !uuids.contains(&reserved.maker_order_uuid) {
                return MatchReservedResult::NotMatched;
            },
            MatchBy::Pubkeys(pubkeys) => if !pubkeys.contains(&reserved.sender_pubkey) {
                return MatchReservedResult::NotMatched;
            },
        }

        let my_base_amount: MmNumber = self.request.get_base_amount();
        let my_rel_amount: MmNumber = self.request.get_rel_amount();
        let other_base_amount: MmNumber = reserved.get_base_amount();
        let other_rel_amount: MmNumber = reserved.get_rel_amount();

        match self.request.action {
            TakerAction::Buy => if self.request.base == reserved.base && self.request.rel == reserved.rel
                && my_base_amount == other_base_amount && other_rel_amount <= my_rel_amount {
                MatchReservedResult::Matched
            } else {
                MatchReservedResult::NotMatched
            },
            TakerAction::Sell => if self.request.base == reserved.rel && self.request.rel == reserved.base
                && my_base_amount == other_rel_amount && my_rel_amount <= other_base_amount {
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
    #[serde(default = "zero_rat")]
    pub max_base_vol_rat: BigRational,
    pub min_base_vol: BigDecimal,
    #[serde(default = "zero_rat")]
    pub min_base_vol_rat: BigRational,
    pub price: BigDecimal,
    #[serde(default = "zero_rat")]
    pub price_rat: BigRational,
    pub created_at: u64,
    pub base: String,
    pub rel: String,
    matches: HashMap<Uuid, MakerMatch>,
    started_swaps: Vec<Uuid>,
    uuid: Uuid,
}

fn zero_rat() -> BigRational { BigRational::zero() }

impl MakerOrder {
    fn available_amount(&self) -> MmNumber {
        let reserved: MmNumber = self.matches.iter().fold(
            MmNumber::from(BigRational::from_integer(0.into())),
            |reserved, (_, order_match)| reserved + order_match.reserved.get_base_amount()
        );
        MmNumber::from(self.max_base_vol_rat.clone()) - reserved
    }

    fn is_cancellable(&self) -> bool {
        !self.has_ongoing_matches()
    }

    fn has_ongoing_matches(&self) -> bool {
        for (_, order_match) in self.matches.iter() {
            // if there's at least 1 ongoing match the order is not cancellable
            if order_match.connected.is_none() && order_match.connect.is_none() {
                return true;
            }
        }
        return false;
    }
}

impl Into<MakerOrder> for TakerOrder {
    fn into(self) -> MakerOrder {
        let order = match self.request.action {
            TakerAction::Sell => MakerOrder {
                price: &self.request.rel_amount / &self.request.base_amount,
                price_rat: (self.request.get_rel_amount() / self.request.get_base_amount()).into(),
                max_base_vol_rat: self.request.get_base_amount().into(),
                max_base_vol: self.request.base_amount,
                min_base_vol_rat: BigRational::from_integer(0.into()),
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
                price_rat: (self.request.get_base_amount() / self.request.get_rel_amount()).into(),
                max_base_vol_rat: self.request.get_rel_amount().into(),
                max_base_vol: self.request.rel_amount,
                min_base_vol: 0.into(),
                min_base_vol_rat: BigRational::from_integer(0.into()),
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
    base_amount_rat: Option<BigRational>,
    rel_amount: BigDecimal,
    rel_amount_rat: Option<BigRational>,
    taker_order_uuid: Uuid,
    maker_order_uuid: Uuid,
    method: String,
    sender_pubkey: H256Json,
    dest_pub_key: H256Json,
}

impl MakerReserved {
    fn get_base_amount(&self) -> MmNumber {
        match &self.base_amount_rat {
            Some(r) => r.clone().into(),
            None => self.base_amount.clone().into()
        }
    }

    fn get_rel_amount(&self) -> MmNumber {
        match &self.rel_amount_rat {
            Some(r) => r.clone().into(),
            None => self.rel_amount.clone().into()
        }
    }
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
    pub my_cancelled_orders: Mutex<HashMap<Uuid, MakerOrder>>,
    /// A map from (base, rel)
    pub orderbook: Mutex<HashMap<(String, String), HashMap<Uuid, PricePingRequest>>>,
}

impl OrdermatchContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    fn from_ctx (ctx: &MmArc) -> Result<Arc<OrdermatchContext>, String> {
        Ok (try_s! (from_ctx (&ctx.ordermatch_ctx, move || {
            Ok (OrdermatchContext {
                my_taker_orders: Mutex::new (HashMap::default()),
                my_maker_orders: Mutex::new (HashMap::default()),
                my_cancelled_orders: Mutex::new (HashMap::default()),
                orderbook: Mutex::new (HashMap::default()),
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

#[cfg_attr(test, mockable)]
fn lp_connect_start_bob(ctx: MmArc, maker_match: MakerMatch) {
    spawn(async move {  // aka "maker_loop"
        let taker_coin = match lp_coinfindᵃ(&ctx, &maker_match.reserved.rel).await {
            Ok(Some(c)) => c,
            Ok(None) => {log!("Coin " (maker_match.reserved.rel) " is not found/enabled"); return},
            Err(e) => {log!("!lp_coinfind(" (maker_match.reserved.rel) "): " (e)); return}
        };

        let maker_coin = match lp_coinfindᵃ(&ctx, &maker_match.reserved.base).await {
            Ok(Some(c)) => c,
            Ok(None) => {log!("Coin " (maker_match.reserved.base) " is not found/enabled"); return},
            Err(e) => {log!("!lp_coinfind(" (maker_match.reserved.base) "): " (e)); return}
        };
        let mut alice = bits256::default();
        alice.bytes = maker_match.request.sender_pubkey.0;
        let maker_amount = maker_match.reserved.get_base_amount().into();
        let taker_amount = maker_match.reserved.get_rel_amount().into();
        let privkey = &ctx.secp256k1_key_pair().private().secret;
        let my_persistent_pub = unwrap!(compressed_pub_key_from_priv_raw(&privkey[..], ChecksumType::DSHA256));
        let uuid = maker_match.request.uuid.to_string();

        log!("Entering the maker_swap_loop " (maker_coin.ticker()) "/" (taker_coin.ticker()) " with uuid: " (uuid));
        let maker_swap = MakerSwap::new(
            ctx.clone(),
            alice.into(),
            maker_coin,
            taker_coin,
            maker_amount,
            taker_amount,
            my_persistent_pub,
            uuid,
        );
        run_maker_swap(RunMakerSwapInput::StartNew(maker_swap), ctx).await;
    });
}

fn lp_connected_alice(ctx: MmArc, taker_match: TakerMatch) {
    spawn (async move {  // aka "taker_loop"
        let mut maker = bits256::default();
        maker.bytes = taker_match.reserved.sender_pubkey.0;
        let taker_coin = match lp_coinfindᵃ (&ctx, &taker_match.reserved.rel) .await {
            Ok(Some(c)) => c,
            Ok(None) => {
                log!("Coin " (taker_match.reserved.rel) " is not found/enabled");
                return;
            }
            Err(e) => {
                log!("!lp_coinfind(" (taker_match.reserved.rel) "): " (e));
                return;
            }
        };

        let maker_coin = match lp_coinfindᵃ (&ctx, &taker_match.reserved.base) .await {
            Ok(Some(c)) => c,
            Ok(None) => {
                log!("Coin " (taker_match.reserved.base) " is not found/enabled");
                return;
            }
            Err(e) => {
                log!("!lp_coinfind(" (taker_match.reserved.base) "): " (e));
                return;
            }
        };

        let privkey = &ctx.secp256k1_key_pair().private().secret;
        let my_persistent_pub = unwrap!(compressed_pub_key_from_priv_raw(&privkey[..], ChecksumType::DSHA256));
        let maker_amount = taker_match.reserved.get_base_amount().into();
        let taker_amount = taker_match.reserved.get_rel_amount().into();
        let uuid = taker_match.reserved.taker_order_uuid.to_string();

        log!("Entering the taker_swap_loop " (maker_coin.ticker()) "/" (taker_coin.ticker())  " with uuid: " (uuid));
        let taker_swap = TakerSwap::new(
            ctx.clone(),
            maker.into(),
            maker_coin,
            taker_coin,
            maker_amount,
            taker_amount,
            my_persistent_pub,
            uuid,
        );
        run_taker_swap(RunTakerSwapInput::StartNew(taker_swap), ctx).await
    });
}

pub async fn lp_ordermatch_loop(ctx: MmArc) {
    const ORDERMATCH_TIMEOUT: u64 = 30000;
    let mut last_price_broadcast = 0;

    loop {
        if ctx.is_stopping() { break }
        let ordermatch_ctx = unwrap!(OrdermatchContext::from_ctx(&ctx));
        {
            let mut my_taker_orders = unwrap!(ordermatch_ctx.my_taker_orders.lock());
            let mut my_maker_orders = unwrap!(ordermatch_ctx.my_maker_orders.lock());
            let mut my_cancelled_orders = unwrap!(ordermatch_ctx.my_cancelled_orders.lock());
            // transform the timed out and unmatched GTC taker orders to maker
            *my_taker_orders = my_taker_orders.drain().filter_map(|(uuid, order)| if order.created_at + ORDERMATCH_TIMEOUT < now_ms() {
                delete_my_taker_order(&ctx, &order);
                if order.matches.is_empty() && order.order_type == OrderType::GoodTillCancelled {
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
            *my_maker_orders = my_maker_orders.drain().filter_map(|(uuid, order)| {
                let min_amount: BigDecimal = MIN_TRADING_VOL.parse().unwrap();
                let min_amount: MmNumber = min_amount.into();
                if order.available_amount() <= min_amount && !order.has_ongoing_matches() {
                    delete_my_maker_order(&ctx, &order);
                    my_cancelled_orders.insert(uuid, order);
                    None
                } else {
                    Some((uuid, order))
                }
            }).collect();
        }

        if now_ms() > last_price_broadcast + 10000 {
            if let Err(e) = broadcast_my_maker_orders(&ctx).await {
                ctx.log.log("", &[&"broadcast_my_maker_orders"], &format!("error {}", e));
            }
            last_price_broadcast = now_ms();
        }

        {
            // remove "timed out" orders from orderbook
            // ones that didn't receive an update for 30 seconds or more
            let mut orderbook = unwrap!(ordermatch_ctx.orderbook.lock());
            *orderbook = orderbook.drain().filter_map(|((base, rel), mut pair_orderbook)| {
                pair_orderbook = pair_orderbook.drain().filter_map(|(pubkey, order)| if now_ms() / 1000 > order.timestamp + 30 {
                    None
                } else {
                    Some((pubkey, order))
                }).collect();
                if pair_orderbook.is_empty() {
                    None
                } else {
                    Some(((base, rel), pair_orderbook))
                }
            }).collect();
        }

        Timer::sleep(0.777).await;
    }
}

pub fn lp_trade_command(
    ctx: MmArc,
    json: Json,
) -> i32 {
    let method = json["method"].as_str();
    let ordermatch_ctx = unwrap!(OrdermatchContext::from_ctx(&ctx));
    let our_public_id = unwrap!(ctx.public_id());
    if method == Some("reserved") {
        let reserved_msg: MakerReserved = match json::from_value(json.clone()) {
            Ok(r) => r,
            Err(_) => return 1,
        };
        if is_pubkey_banned(&ctx, &reserved_msg.sender_pubkey.clone().into()) {
            log!("Sender pubkey " [reserved_msg.sender_pubkey] " is banned");
            return 1;
        }
        if H256Json::from(our_public_id.bytes) != reserved_msg.dest_pub_key {
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
                sender_pubkey: H256Json::from(our_public_id.bytes),
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
        if H256Json::from(our_public_id.bytes) == connected.dest_pub_key && H256Json::from(our_public_id.bytes) != connected.sender_pubkey {
            let mut my_taker_orders = unwrap!(ordermatch_ctx.my_taker_orders.lock());
            let my_order_entry = match my_taker_orders.entry(connected.taker_order_uuid) {
                Entry::Occupied(e) => e,
                Entry::Vacant(_) => {
                    log!("Our node doesn't have the order with uuid "(connected.taker_order_uuid));
                    return 1;
                },
            };
            let order_match = match my_order_entry.get().matches.get(&connected.maker_order_uuid) {
                Some(o) => o,
                None => {
                    log!("Our node doesn't have the match with uuid "(connected.maker_order_uuid));
                    return 1;
                }
            };
            // alice
            lp_connected_alice(ctx.clone(), order_match.clone());
            // remove the matched order immediately
            delete_my_taker_order(&ctx, &my_order_entry.get());
            my_order_entry.remove();
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
        if is_pubkey_banned(&ctx, &taker_request.sender_pubkey.clone().into()) {
            log!("Sender pubkey " [taker_request.sender_pubkey] " is banned");
            return 1;
        }
        if our_public_id.bytes == taker_request.dest_pub_key.0 {
            log!("Skip the request originating from our pubkey");
            return 1;
        }
        let ordermatch_ctx = unwrap!(OrdermatchContext::from_ctx(&ctx));
        let mut my_orders = unwrap!(ordermatch_ctx.my_maker_orders.lock());

        for (uuid, order) in my_orders.iter_mut() {
            if let OrderMatchResult::Matched((base_amount, rel_amount)) = match_order_and_request(order, &taker_request) {
                if !order.matches.contains_key(&taker_request.uuid) {
                    let reserved = MakerReserved {
                        dest_pub_key: taker_request.sender_pubkey.clone(),
                        sender_pubkey: our_public_id.bytes.into(),
                        base: order.base.clone(),
                        base_amount: base_amount.clone().into(),
                        base_amount_rat: Some(base_amount.into()),
                        rel_amount: rel_amount.clone().into(),
                        rel_amount_rat: Some(rel_amount.into()),
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
                }
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
        if our_public_id.bytes == connect_msg.dest_pub_key.0 && our_public_id.bytes != connect_msg.sender_pubkey.0 {
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

            if order_match.connected.is_none() && order_match.connect.is_none() {
                let connected = MakerConnected {
                    sender_pubkey: our_public_id.bytes.into(),
                    dest_pub_key: connect_msg.sender_pubkey.clone(),
                    taker_order_uuid: connect_msg.taker_order_uuid,
                    maker_order_uuid: connect_msg.maker_order_uuid,
                    method: "connected".into(),
                };
                ctx.broadcast_p2p_msg(&unwrap!(json::to_string(&connected)));
                order_match.connect = Some(connect_msg);
                order_match.connected = Some(connected);
                my_order.started_swaps.push(order_match.request.uuid);
                lp_connect_start_bob(ctx.clone(), order_match.clone());
                save_my_maker_order(&ctx, &my_order);
            }
        }
        return 1;
    }
    -1
}

async fn check_locked_coins(ctx: &MmArc, amount: &MmNumber, balance: &BigDecimal, ticker: &str) -> Result<(), String> {
    let locked = get_locked_amount(ctx, ticker);
    let available = balance - &locked;
    if amount > &available {
        ERR!("The {} amount {} is larger than available {:.8}, balance: {}, locked by swaps: {:.8}", ticker, amount, available, balance, locked)
    } else {
        Ok(())
    }
}

#[derive(Deserialize, Debug)]
pub struct AutoBuyInput {
    base: String,
    rel: String,
    price: MmNumber,
    volume: MmNumber,
    timeout: Option<u32>,
    /// Not used. Deprecated.
    duration: Option<u32>,
    // TODO: remove this field on API refactoring, method should be separated from params
    method: String,
    gui: Option<String>,
    #[serde(rename="destpubkey")]
    #[serde(default)]
    dest_pub_key: H256Json,
    #[serde(default)]
    match_by: MatchBy,
    #[serde(default)]
    order_type: OrderType,
}

pub async fn buy(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let input: AutoBuyInput = try_s!(json::from_value(req));
    if input.base == input.rel {return ERR!("Base and rel must be different coins")}
    let rel_coin = try_s!(lp_coinfindᵃ(&ctx, &input.rel).await);
    let rel_coin = try_s!(rel_coin.ok_or("Rel coin is not found or inactive"));
    let base_coin = try_s!(lp_coinfindᵃ(&ctx, &input.base).await);
    let base_coin: MmCoinEnum = try_s!(base_coin.ok_or("Base coin is not found or inactive"));
    let my_amount = &input.volume * &input.price;
    let my_balance = try_s!(rel_coin.my_balance().compat().await);
    try_s!(check_locked_coins(&ctx, &my_amount, &my_balance, rel_coin.ticker()).await);
    let dex_fee = dex_fee_amount(base_coin.ticker(), rel_coin.ticker(), &my_amount.clone().into());
    let trade_info = TradeInfo::Taker(dex_fee);
    try_s!(rel_coin.check_i_have_enough_to_trade(&my_amount.clone().into(), &my_balance.clone().into(), trade_info).compat().await);
    try_s!(base_coin.can_i_spend_other_payment().compat().await);
    let res = try_s!(lp_auto_buy(&ctx, input)).into_bytes();
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn sell(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let input: AutoBuyInput = try_s!(json::from_value(req));
    if input.base == input.rel {return ERR!("Base and rel must be different coins")}
    let base_coin = try_s!(lp_coinfindᵃ(&ctx, &input.base).await);
    let base_coin = try_s!(base_coin.ok_or("Base coin is not found or inactive"));
    let rel_coin = try_s!(lp_coinfindᵃ(&ctx, &input.rel).await);
    let rel_coin = try_s!(rel_coin.ok_or("Rel coin is not found or inactive"));
    let my_balance = try_s!(base_coin.my_balance().compat().await);
    try_s!(check_locked_coins(&ctx, &input.volume, &my_balance, base_coin.ticker()).await);
    let dex_fee = dex_fee_amount(base_coin.ticker(), rel_coin.ticker(), &input.volume.clone().into());
    let trade_info = TradeInfo::Taker(dex_fee);
    try_s!(base_coin.check_i_have_enough_to_trade(&input.volume.clone().into(), &my_balance.clone().into(), trade_info).compat().await);
    try_s!(rel_coin.can_i_spend_other_payment().compat().await);
    let res = try_s!(lp_auto_buy(&ctx, input)).into_bytes();
    Ok(try_s!(Response::builder().body(res)))
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
#[derive(Clone, Debug, Deserialize, Serialize)]
struct TakerMatch {
    reserved: MakerReserved,
    connect: TakerConnect,
    connected: Option<MakerConnected>,
    last_updated: u64,
}

pub fn lp_auto_buy(ctx: &MmArc, input: AutoBuyInput) -> Result<String, String> {
    if input.price < MmNumber::from(BigRational::new(1.into(), 100000000.into())) {
        return ERR!("Price is too low, minimum is 0.00000001");
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
    let uuid = new_uuid();
    let our_public_id = try_s!(ctx.public_id());
    let rel_volume = &input.volume * &input.price;
    if input.volume < MmNumber::from(unwrap!(MIN_TRADING_VOL.parse::<BigDecimal>())) {
        return ERR!("Base volume {} is too low, required at least {}", input.volume, MIN_TRADING_VOL);
    }

    if rel_volume < MmNumber::from(unwrap!(MIN_TRADING_VOL.parse::<BigDecimal>())) {
        return ERR!("Rel volume {} is too low, required at least {}", rel_volume, MIN_TRADING_VOL);
    }

    let request = TakerRequest {
        base: input.base,
        rel: input.rel,
        rel_amount: rel_volume.clone().into(),
        rel_amount_rat: Some(rel_volume.into()),
        base_amount: input.volume.clone().into(),
        base_amount_rat: Some(BigRational::from(input.volume)),
        method: "request".into(),
        uuid,
        dest_pub_key: input.dest_pub_key,
        sender_pubkey: H256Json::from(our_public_id.bytes),
        action,
        match_by: input.match_by,
    };
    ctx.broadcast_p2p_msg(&unwrap!(json::to_string(&request)));
    let result = json!({
        "result": request
    }).to_string();
    let order = TakerOrder {
        created_at: now_ms(),
        matches: HashMap::new(),
        request,
        order_type: input.order_type,
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
    price_rat: Option<MmNumber>,
    price64: String,
    timestamp: u64,
    pubsecp: String,
    sig: String,
    // TODO rename, it's called "balance", but it's actual meaning is max available volume to trade
    #[serde(rename="bal")]
    balance: BigDecimal,
    balance_rat: Option<MmNumber>,
    uuid: Option<Uuid>,
}

impl PricePingRequest {
    fn new(ctx: &MmArc, order: &MakerOrder, balance: BigDecimal) -> Result<PricePingRequest, String> {
        let public_id = try_s!(ctx.public_id());

        let price64 = (&order.price * BigDecimal::from(100000000)).to_u64().unwrap();
        let timestamp = now_ms() / 1000;
        let sig_hash = price_ping_sig_hash(
            timestamp as u32,
            &**ctx.secp256k1_key_pair().public(),
            &public_id.bytes,
            order.base.as_bytes(),
            order.rel.as_bytes(),
            price64,
        );

        let sig = try_s!(ctx.secp256k1_key_pair().private().sign(&sig_hash));

        let available_amount: BigRational = order.available_amount().into();
        let min_amount = BigRational::new(777.into(), 100000.into());
        let max_volume = if available_amount > min_amount {
            let my_balance = from_dec_to_ratio(balance);
            if available_amount <= my_balance && available_amount > BigRational::from_integer(0.into()) {
                available_amount
            } else {
                my_balance
            }
        } else {
            BigRational::from_integer(0.into())
        };

        Ok(PricePingRequest {
            method: "postprice".into(),
            pubkey: hex::encode(&public_id.bytes),
            base: order.base.clone(),
            rel: order.rel.clone(),
            price64: price64.to_string(),
            price: order.price.clone(),
            price_rat: Some(order.price_rat.clone().into()),
            timestamp,
            pubsecp: hex::encode(&**ctx.secp256k1_key_pair().public()),
            sig: hex::encode(&*sig),
            balance: from_ratio_to_dec(&max_volume),
            balance_rat: Some(max_volume.into()),
            uuid: Some(order.uuid),
        })
    }
}

pub fn lp_post_price_recv(ctx: &MmArc, req: Json) -> HyRes {
    let req: PricePingRequest = try_h!(json::from_value(req));
    let signature: Signature = try_h!(req.sig.parse());
    let pubkey_bytes = try_h!(hex::decode(&req.pubsecp));
    // return success response to avoid excessive logging of
    // RPC error response: lp_ordermatch:852] sender pubkey 03eb26aab2e22fd2507042d1c472b3f973d629d295d391faf7b68ac5b85197ec80 is banned""
    // messages
    if is_pubkey_banned(ctx, &H256Json::from(&pubkey_bytes[1..])) {
        return rpc_response(200, ERRL!("sender pubkey {} is banned", req.pubsecp));
    }
    let pub_secp = try_h!(Public::from_slice(&pubkey_bytes));
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
        // identify the order by first 16 bytes of node pubkey to keep backwards-compatibility
        // TODO remove this when all nodes are updated
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&pubkey[..16]);
        let uuid = req.uuid.unwrap_or(Uuid::from_bytes(bytes));
        let ordermatch_ctx: Arc<OrdermatchContext> = try_h!(OrdermatchContext::from_ctx(ctx));
        let mut orderbook = try_h!(ordermatch_ctx.orderbook.lock());
        match orderbook.entry((req.base.clone(), req.rel.clone())) {
            Entry::Vacant(pair_orders) => if req.balance > 0.into() && req.price > 0.into() {
                let mut orders = HashMap::new();
                orders.insert(uuid, req);
                pair_orders.insert(orders);
            },
            Entry::Occupied(mut pair_orders) => {
                match pair_orders.get_mut().entry(uuid) {
                    Entry::Vacant(order) => if req.balance > 0.into() && req.price > 0.into() {
                        order.insert(req);
                    },
                    Entry::Occupied(mut order) => if req.balance > 0.into() {
                        order.insert(req);
                    } else {
                        order.remove();
                    },
                }
            }
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
    let req_value = try_s!(json::to_value(req));
    let ctxʹ = ctx.clone();
    spawn(async move {
        let rc = lp_post_price_recv(&ctxʹ, req_value).compat().await;
        if let Err(err) = rc {log!("!lp_post_price_recv: "(err))}
    });

    ctx.broadcast_p2p_msg(&req_string);
    Ok(())
}

fn one() -> u8 { 1 }

fn get_true() -> bool { true }

#[derive(Deserialize)]
struct SetPriceReq {
    base: String,
    rel: String,
    price: MmNumber,
    #[serde(default)]
    max: bool,
    #[allow(dead_code)]
    #[serde(default = "one")]
    broadcast: u8,
    #[serde(default)]
    volume: MmNumber,
    #[serde(default = "get_true")]
    cancel_previous: bool,
}

pub async fn set_price(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: SetPriceReq = try_s!(json::from_value(req));
    if req.price < MmNumber::from(BigRational::new(1.into(), 100000000.into())) {
        return ERR!("Price is too low, minimum is 0.00000001");
    }
    if req.base == req.rel {
        return ERR!("Base and rel must be different coins");
    }

    let base_coin: MmCoinEnum = match try_s!(lp_coinfindᵃ(&ctx, &req.base).await) {
        Some(coin) => coin,
        None => return ERR!("Base coin {} is not found", req.base),
    };

    let rel_coin: MmCoinEnum = match try_s!(lp_coinfindᵃ(&ctx, &req.rel).await) {
        Some(coin) => coin,
        None => return ERR!("Rel coin {} is not found", req.rel),
    };

    let my_balance = try_s!(base_coin.my_balance().compat().await);
    let volume = if req.max {
        // use entire balance deducting the locked amount and trade fee if it's paid with base coin,
        // skipping "check_i_have_enough"
        let trade_fee = try_s!(base_coin.get_trade_fee().compat().await);
        let mut vol = my_balance - get_locked_amount(&ctx, base_coin.ticker());
        if trade_fee.coin == base_coin.ticker() {
            vol -= trade_fee.amount;
        }
        MmNumber::from(vol)
    } else {
        try_s!(check_locked_coins(&ctx, &req.volume, &my_balance, base_coin.ticker()).await);
        try_s!(base_coin.check_i_have_enough_to_trade(&req.volume, &my_balance.clone().into(), TradeInfo::Maker).compat().await);
        req.volume.clone()
    };
    if volume < MmNumber::from(unwrap!(MIN_TRADING_VOL.parse::<BigDecimal>())) {
        return ERR!("Base volume {} is too low, required at least {}", volume, MIN_TRADING_VOL);
    }
    let rel_volume = &volume * &req.price;
    if rel_volume < MmNumber::from(unwrap!(MIN_TRADING_VOL.parse::<BigDecimal>())) {
        return ERR!("Rel volume {} is too low, required at least {}", rel_volume, MIN_TRADING_VOL);
    }
    try_s!(rel_coin.can_i_spend_other_payment().compat().await);

    let ordermatch_ctx = try_s!(OrdermatchContext::from_ctx(&ctx));
    let mut my_orders = try_s!(ordermatch_ctx.my_maker_orders.lock());
    if req.cancel_previous {
        let mut cancelled_orders = try_s!(ordermatch_ctx.my_cancelled_orders.lock());
        // remove the previous orders if there're some to allow multiple setprice call per pair
        // it's common use case now as `autoprice` doesn't work with new ordermatching and
        // MM2 users request the coins price from aggregators by their own scripts issuing
        // repetitive setprice calls with new price
        *my_orders = my_orders.drain().filter_map(|(uuid, order)| {
            let to_delete = order.base == req.base && order.rel == req.rel;
            if to_delete {
                delete_my_maker_order(&ctx, &order);
                cancelled_orders.insert(uuid, order);
                None
            } else {
                Some((uuid, order))
            }
        }).collect();
    }

    let uuid = new_uuid();
    let order = MakerOrder {
        max_base_vol: volume.clone().into(),
        max_base_vol_rat: volume.into(),
        min_base_vol: 0.into(),
        min_base_vol_rat: BigRational::from_integer(0.into()),
        price: req.price.clone().into(),
        price_rat: req.price.clone().into(),
        created_at: now_ms(),
        base: req.base,
        rel: req.rel,
        matches: HashMap::new(),
        started_swaps: Vec::new(),
        uuid,
    };
    save_my_maker_order(&ctx, &order);
    let res = try_s!(json::to_vec(&json!({"result":order})));
    my_orders.insert(uuid, order);
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn broadcast_my_maker_orders(ctx: &MmArc) -> Result<(), String> {
    let ordermatch_ctx = try_s!(OrdermatchContext::from_ctx(ctx));
    let my_orders = try_s!(ordermatch_ctx.my_maker_orders.lock()).clone();
    for (_, order) in my_orders {
        let base_coin = match try_s!(lp_coinfindᵃ(ctx, &order.base).await) {
            Some(coin) => coin,
            None => {
                ctx.log.log("", &[&"broadcast_my_maker_orders", &order.base, &order.rel], "base coin is not active yet");
                continue
            },
        };

        let _rel_coin = match try_s!(lp_coinfindᵃ(ctx, &order.rel).await) {
            Some(coin) => coin,
            None => {
                ctx.log.log("", &[&"broadcast_my_maker_orders", &order.base, &order.rel], "rel coin is not active yet");
                continue
            },
        };

        let balance = match base_coin.my_balance().compat().await {
            Ok(b) => b,
            Err(e) => {
                ctx.log.log("", &[&"broadcast_my_maker_orders", &order.base, &order.rel], &format! ("failed to get balance of base coin: {}", e));
                continue;
            }
        };

        if balance >= MIN_TRADING_VOL.parse().unwrap() {
            let ping = match PricePingRequest::new(ctx, &order, balance) {
                Ok(p) => p,
                Err(e) => {
                    ctx.log.log("", &[&"broadcast_my_maker_orders", &order.base, &order.rel], &format!("ping request creation failed {}", e));
                    continue;
                },
            };

            if let Err(e) = lp_send_price_ping(&ping, ctx) {
                ctx.log.log("", &[&"broadcast_my_maker_orders", &order.base, &order.rel], &format!("ping request send failed {}", e));
                continue;
            }
        } else {
            // cancel the order if available balance is lower than MIN_TRADING_VOL
            try_s!(ordermatch_ctx.my_maker_orders.lock()).remove(&order.uuid);
            delete_my_maker_order(ctx, &order);
            try_s!(ordermatch_ctx.my_cancelled_orders.lock()).insert(order.uuid, order);
        }
    }
    // the difference of cancelled orders from maker orders that we broadcast the cancel request only once
    // cancelled record can be just dropped then
    let cancelled_orders: HashMap<_, _> = try_s!(ordermatch_ctx.my_cancelled_orders.lock()).drain().collect();
    for (_, mut order) in cancelled_orders {
        // TODO cancel means setting the volume to 0 as of now, should refactor
        order.max_base_vol = 0.into();
        order.max_base_vol_rat = BigRational::from_integer(0.into());
        let ping = match PricePingRequest::new(ctx, &order, 0.into()) {
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
    Matched((MmNumber, MmNumber)),
    /// Orders didn't match
    NotMatched,
}

/// Attempts to match the Maker's order and Taker's request
fn match_order_and_request(maker: &MakerOrder, taker: &TakerRequest) -> OrderMatchResult {
    let taker_base_amount: MmNumber = taker.get_base_amount();
    let taker_rel_amount: MmNumber = taker.get_rel_amount();
    let maker_price: MmNumber = maker.price_rat.clone().into();
    let maker_min_vol: MmNumber = maker.min_base_vol_rat.clone().into();

    match taker.action {
        TakerAction::Buy => {
            if maker.base == taker.base && maker.rel == taker.rel && taker_base_amount <= maker.available_amount() && taker_base_amount >= maker_min_vol {
                let taker_price = &taker_rel_amount / &taker_base_amount;
                if taker_price >= maker_price {
                    OrderMatchResult::Matched((taker_base_amount.clone(), taker_base_amount * maker_price))
                } else {
                    OrderMatchResult::NotMatched
                }
            } else {
                OrderMatchResult::NotMatched
            }
        },
        TakerAction::Sell => {
            if maker.base == taker.rel && maker.rel == taker.base && taker_rel_amount <= maker.available_amount() && taker_rel_amount >= maker_min_vol {
                let taker_price = &taker_base_amount / &taker_rel_amount;
                if taker_price >= maker_price {
                    OrderMatchResult::Matched((&taker_base_amount / &maker_price, taker_base_amount))
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
            "order": MakerOrderForRpc::from(order),
        }).to_string());
    }

    let taker_orders = try_h!(ordermatch_ctx.my_taker_orders.lock());
    if let Some(order) = taker_orders.get(&req.uuid) {
        return rpc_response(200, json!({
            "type": "Taker",
            "order": TakerOrderForRpc::from(order),
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
    match maker_orders.entry(req.uuid) {
        Entry::Occupied(order) => {
            if !order.get().is_cancellable() {
                return rpc_err_response(500, &format!("Order {} is being matched now, can't cancel", req.uuid));
            }
            let mut cancelled_orders = try_h!(ordermatch_ctx.my_cancelled_orders.lock());
            let order = order.remove();
            delete_my_maker_order(&ctx, &order);
            cancelled_orders.insert(req.uuid, order);
            return rpc_response(200, json!({
                "result": "success"
            }).to_string())
        },
        // look for taker order with provided uuid
        Entry::Vacant(_) => (),
    }

    let mut taker_orders = try_h!(ordermatch_ctx.my_taker_orders.lock());
    match taker_orders.entry(req.uuid) {
        Entry::Occupied(order) => {
            if !order.get().is_cancellable() {
                return rpc_err_response(500, &format!("Order {} is being matched now, can't cancel", req.uuid));
            }
            let order = order.remove();
            delete_my_taker_order(&ctx, &order);
            return rpc_response(200, json!({
                "result": "success"
            }).to_string())
        },
        // error is returned
        Entry::Vacant(_) => (),
    }

    rpc_err_response(404, &format!("Order with uuid {} is not found", req.uuid))
}

#[derive(Serialize)]
struct MakerOrderForRpc<'a> {
    #[serde(flatten)]
    order: &'a MakerOrder,
    cancellable: bool,
    available_amount: BigDecimal,
}

impl<'a> From<&'a MakerOrder> for MakerOrderForRpc<'a> {
    fn from(order: &'a MakerOrder) -> MakerOrderForRpc {
        MakerOrderForRpc {
            order,
            cancellable: order.is_cancellable(),
            available_amount: order.available_amount().into(),
        }
    }
}

#[derive(Serialize)]
struct TakerOrderForRpc<'a> {
    #[serde(flatten)]
    order: &'a TakerOrder,
    cancellable: bool
}

impl<'a> From<&'a TakerOrder> for TakerOrderForRpc<'a> {
    fn from(order: &'a TakerOrder) -> TakerOrderForRpc {
        TakerOrderForRpc {
            order,
            cancellable: order.is_cancellable(),
        }
    }
}

pub fn my_orders(ctx: MmArc) -> HyRes {
    let ordermatch_ctx = try_h!(OrdermatchContext::from_ctx(&ctx));
    let maker_orders = try_h!(ordermatch_ctx.my_maker_orders.lock());
    let taker_orders = try_h!(ordermatch_ctx.my_taker_orders.lock());
    let maker_orders_for_rpc: HashMap<_, _> = maker_orders.iter().map(|(uuid, order)| (uuid, MakerOrderForRpc::from(order))).collect();
    let taker_orders_for_rpc: HashMap<_, _> = taker_orders.iter().map(|(uuid, order)| (uuid, TakerOrderForRpc::from(order))).collect();
    rpc_response(200, json!({
        "result": {
            "maker_orders": maker_orders_for_rpc,
            "taker_orders": taker_orders_for_rpc,
        }
    }).to_string())
}

pub fn my_maker_orders_dir(ctx: &MmArc) -> PathBuf {
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
    unwrap!(write(&path, &content));
}

fn save_my_taker_order(ctx: &MmArc, order: &TakerOrder) {
    let path = my_taker_order_file_path(ctx, &order.request.uuid);
    let content = unwrap!(json::to_vec(order));
    unwrap!(write(&path, &content));
}

#[cfg_attr(test, mockable)]
fn delete_my_maker_order(ctx: &MmArc, order: &MakerOrder) {
    let path = my_maker_order_file_path(ctx, &order.uuid);
    match remove_file(&path) {
        Ok(_) => (),
        Err(e) => log!("Warning, could not remove order file " (path.display()) ", error " (e)),
    }
}

#[cfg_attr(test, mockable)]
fn delete_my_taker_order(ctx: &MmArc, order: &TakerOrder) {
    let path = my_taker_order_file_path(ctx, &order.request.uuid);
    match remove_file(&path) {
        Ok(_) => (),
        Err(e) => log!("Warning, could not remove order file " (path.display()) ", error " (e)),
    }
}

pub fn orders_kick_start(ctx: &MmArc) -> Result<HashSet<String>, String> {
    let mut coins = HashSet::new();
    let ordermatch_ctx = try_s!(OrdermatchContext::from_ctx(ctx));
    let mut maker_orders = try_s!(ordermatch_ctx.my_maker_orders.lock());
    let maker_entries = try_s!(json_dir_entries(&my_maker_orders_dir(&ctx)));

    maker_entries.iter().for_each(|entry| {
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
    let taker_entries: Vec<DirEntry> = try_s!(json_dir_entries(&my_taker_orders_dir(&ctx)));

    taker_entries.iter().for_each(|entry| {
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

#[derive(Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum CancelBy {
    /// All orders of current node
    All,
    /// All orders of specific pair
    Pair { base: String, rel: String },
    /// All orders using the coin ticker as base or rel
    Coin { ticker: String },
}

pub fn cancel_orders_by(ctx: &MmArc, cancel_by: CancelBy) -> Result<(Vec<Uuid>, Vec<Uuid>), String> {
    let mut cancelled = vec![];
    let mut currently_matching = vec![];

    let ordermatch_ctx = try_s!(OrdermatchContext::from_ctx(ctx));
    let mut maker_orders = try_s!(ordermatch_ctx.my_maker_orders.lock());
    let mut taker_orders = try_s!(ordermatch_ctx.my_taker_orders.lock());
    let mut my_cancelled_orders = try_s!(ordermatch_ctx.my_cancelled_orders.lock());

    macro_rules! cancel_maker_if_true {
        ($e: expr, $uuid: ident, $order: ident) => {
            if $e {
                if $order.is_cancellable() {
                    delete_my_maker_order(&ctx, &$order);
                    my_cancelled_orders.insert($uuid, $order);
                    cancelled.push($uuid);
                    None
                } else {
                    currently_matching.push($uuid);
                    Some(($uuid, $order))
                }
            } else {
                Some(($uuid, $order))
            }
        };
    }

    macro_rules! cancel_taker_if_true {
        ($e: expr, $uuid: ident, $order: ident) => {
            if $e {
                if $order.is_cancellable() {
                    delete_my_taker_order(&ctx, &$order);
                    cancelled.push($uuid);
                    None
                } else {
                    currently_matching.push($uuid);
                    Some(($uuid, $order))
                }
            } else {
                Some(($uuid, $order))
            }
        };
    }

    match cancel_by {
        CancelBy::All => {
            *maker_orders = maker_orders.drain().filter_map(|(uuid, order)| {
                cancel_maker_if_true!(true, uuid, order)
            }).collect();
            *taker_orders = taker_orders.drain().filter_map(|(uuid, order)| {
                cancel_taker_if_true!(true, uuid, order)
            }).collect();
        },
        CancelBy::Pair{base, rel} => {
            *maker_orders = maker_orders.drain().filter_map(|(uuid, order)| {
                cancel_maker_if_true!(order.base == base && order.rel == rel, uuid, order)
            }).collect();
            *taker_orders = taker_orders.drain().filter_map(|(uuid, order)| {
                cancel_taker_if_true!(order.request.base == base && order.request.rel == rel, uuid, order)
            }).collect();
        },
        CancelBy::Coin{ticker} => {
            *maker_orders = maker_orders.drain().filter_map(|(uuid, order)| {
                cancel_maker_if_true!(order.base == ticker || order.rel == ticker, uuid, order)
            }).collect();
            *taker_orders = taker_orders.drain().filter_map(|(uuid, order)| {
                cancel_taker_if_true!(order.request.base == ticker || order.request.rel == ticker, uuid, order)
            }).collect();
        },
    };

    Ok((cancelled, currently_matching))
}

pub fn cancel_all_orders(ctx: MmArc, req: Json) -> HyRes {
    let cancel_by: CancelBy = try_h!(json::from_value(req["cancel_by"].clone()));

    let (cancelled, currently_matching) = try_h!(cancel_orders_by(&ctx, cancel_by));

    rpc_response(200, json!({
        "result": {
            "cancelled": cancelled,
            "currently_matching": currently_matching,
        }
    }).to_string())
}

#[derive(Serialize)]
pub struct OrderbookEntry {
    coin: String,
    address: String,
    price: BigDecimal,
    price_rat: BigRational,
    price_fraction: Fraction,
    #[serde(rename="maxvolume")]
    max_volume: BigDecimal,
    max_volume_rat: BigRational,
    max_volume_fraction: Fraction,
    pubkey: String,
    age: i64,
    zcredits: u64,
    uuid: Uuid,
}

#[derive(Serialize)]
pub struct OrderbookResponse {
    #[serde(rename="askdepth")]
    ask_depth: u32,
    asks: Vec<OrderbookEntry>,
    base: String,
    #[serde(rename="biddepth")]
    bid_depth: u32,
    bids: Vec<OrderbookEntry>,
    netid: u16,
    #[serde(rename="numasks")]
    num_asks: usize,
    #[serde(rename="numbids")]
    num_bids: usize,
    rel: String,
    timestamp: u64,
}

#[derive(Deserialize)]
struct OrderbookReq {
    base: String,
    rel: String,
}

pub async fn orderbook(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: OrderbookReq = try_s!(json::from_value(req));
    if req.base == req.rel {return ERR!("Base and rel must be different coins")}
    let rel_coin = try_s!(lp_coinfindᵃ(&ctx, &req.rel).await);
    let rel_coin = try_s!(rel_coin.ok_or("Rel coin is not found or inactive"));
    let base_coin = try_s!(lp_coinfindᵃ(&ctx, &req.base).await);
    let base_coin: MmCoinEnum = try_s!(base_coin.ok_or("Base coin is not found or inactive"));
    let ordermatch_ctx: Arc<OrdermatchContext> = try_s!(OrdermatchContext::from_ctx(&ctx));
    let orderbook = try_s!(ordermatch_ctx.orderbook.lock());
    let asks = match orderbook.get(&(req.base.clone(), req.rel.clone())) {
        Some(asks) => {
            let mut orderbook_entries = vec![];
            for (uuid, ask) in asks.iter() {
                orderbook_entries.push(OrderbookEntry {
                    coin: req.base.clone(),
                    address: try_s!(base_coin.address_from_pubkey_str(&ask.pubsecp)),
                    price: ask.price.clone(),
                    price_rat: ask.price_rat.as_ref().map(|p| p.to_ratio()).unwrap_or(from_dec_to_ratio(ask.price.clone())),
                    price_fraction: ask.price_rat.as_ref().map(|p| p.to_fraction()).unwrap_or(ask.price.clone().into()),
                    max_volume: ask.balance.clone(),
                    max_volume_rat: ask.balance_rat.as_ref().map(|p| p.to_ratio()).unwrap_or(from_dec_to_ratio(ask.balance.clone())),
                    max_volume_fraction: ask.balance_rat.as_ref().map(|p| p.to_fraction()).unwrap_or(ask.balance.clone().into()),
                    pubkey: ask.pubkey.clone(),
                    age: (now_ms() as i64 / 1000) - ask.timestamp as i64,
                    zcredits: 0,
                    uuid: *uuid,
                })
            }
            orderbook_entries
        },
        None => vec![],
    };
    let bids = match orderbook.get(&(req.rel.clone(), req.base.clone())) {
        Some(asks) => {
            let mut orderbook_entries = vec![];
            for (uuid, ask) in asks.iter() {
                let price_mm = MmNumber::from(1i32) / ask.price_rat.as_ref().map(|p| p.clone()).unwrap_or(from_dec_to_ratio(ask.price.clone()).into());
                orderbook_entries.push(OrderbookEntry {
                    coin: req.rel.clone(),
                    address: try_s!(rel_coin.address_from_pubkey_str(&ask.pubsecp)),
                    // NB: 1/x can not be represented as a decimal and introduces a rounding error
                    // cf. https://github.com/KomodoPlatform/atomicDEX-API/issues/495#issuecomment-516365682
                    price: BigDecimal::from (1) / &ask.price,
                    price_rat: price_mm.to_ratio(),
                    price_fraction: price_mm.to_fraction(),
                    max_volume: ask.balance.clone(),
                    max_volume_rat: ask.balance_rat.as_ref().map(|p| p.to_ratio()).unwrap_or(from_dec_to_ratio(ask.balance.clone())),
                    max_volume_fraction: ask.balance_rat.as_ref().map(|p| p.to_fraction()).unwrap_or(from_dec_to_ratio(ask.balance.clone()).into()),
                    pubkey: ask.pubkey.clone(),
                    age: (now_ms() as i64 / 1000) - ask.timestamp as i64,
                    zcredits: 0,
                    uuid: *uuid,
                })
            }
            orderbook_entries
        },
        None => vec![],
    };
    let response = OrderbookResponse {
        num_asks: asks.len(),
        num_bids: bids.len(),
        ask_depth: 0,
        asks,
        base: req.base,
        bid_depth: 0,
        bids,
        netid: ctx.netid(),
        rel: req.rel,
        timestamp: now_ms() / 1000,
    };
    let responseʲ = try_s!(json::to_vec(&response));
    Ok(try_s!(Response::builder().body(responseʲ)))
}

pub fn migrate_saved_orders(ctx: &MmArc) -> Result<(), String> {
    let maker_entries = try_s!(json_dir_entries(&my_maker_orders_dir(&ctx)));
    maker_entries.iter().for_each(|entry| {
        match json::from_slice::<MakerOrder>(&slurp(&entry.path())) {
            Ok(mut order) => {
                if order.max_base_vol_rat == BigRational::zero() {
                    order.max_base_vol_rat = from_dec_to_ratio(order.max_base_vol.clone());
                }
                if order.min_base_vol_rat == BigRational::zero() {
                    order.min_base_vol_rat = from_dec_to_ratio(order.min_base_vol.clone());
                }
                if order.price_rat == BigRational::zero() {
                    order.price_rat = from_dec_to_ratio(order.price.clone());
                }
                save_my_maker_order(ctx, &order)
            }
            Err(_) => (),
        }
    });

    let taker_entries: Vec<DirEntry> = try_s!(json_dir_entries(&my_taker_orders_dir(&ctx)));
    taker_entries.iter().for_each(|entry| {
        match json::from_slice::<TakerOrder>(&slurp(&entry.path())) {
            Ok(mut order) => {
                if order.request.base_amount_rat.is_none() {
                    order.request.base_amount_rat = Some(from_dec_to_ratio(order.request.base_amount.clone()));
                }
                if order.request.rel_amount_rat.is_none() {
                    order.request.rel_amount_rat = Some(from_dec_to_ratio(order.request.rel_amount.clone()));
                }
                save_my_taker_order(ctx, &order)
            },
            Err(_) => (),
        }
    });
    Ok(())
}
