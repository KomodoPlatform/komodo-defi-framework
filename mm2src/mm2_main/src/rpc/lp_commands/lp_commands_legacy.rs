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
//  rpc_commands.rs
//  marketmaker
//

use coins::{lp_coinfind, lp_coininit, CoinsContext, MmCoinEnum};
use common::executor::Timer;
use common::log::error;
use common::{rpc_err_response, rpc_response, HyRes};
use futures::compat::Future01CompatExt;
use http::Response;
use mm2_core::mm_ctx::MmArc;
use mm2_metrics::MetricsOps;
use mm2_number::{construct_detailed, BigDecimal};
use serde_json::{self as json, Value as Json};
use std::borrow::Cow;
use std::iter::Extend;
use uuid::Uuid;

use crate::mm2::lp_dispatcher::{dispatch_lp_event, StopCtxEvent};
use crate::mm2::lp_network::subscribe_to_topic;
use crate::mm2::lp_ordermatch::{cancel_orders_by, get_matching_orders, CancelBy};
use crate::mm2::lp_swap::{active_swaps_using_coins, tx_helper_topic, watcher_topic};
use crate::mm2::MmVersionResult;

const INTERNAL_SERVER_ERROR_CODE: u16 = 500;
const RESPONSE_OK_STATUS_CODE: u16 = 200;

pub fn disable_coin_err(
    error: String,
    matching: &[Uuid],
    cancelled: &[Uuid],
    active_swaps: &[Uuid],
) -> Result<Response<Vec<u8>>, String> {
    let err = json!({
        "error": error,
        "orders": {
            "matching": matching,
            "cancelled": cancelled
        },
        "active_swaps": active_swaps
    });
    Response::builder()
        .status(INTERNAL_SERVER_ERROR_CODE)
        .body(json::to_vec(&err).unwrap())
        .map_err(|e| ERRL!("{}", e))
}

/// Attempts to disable the coin
pub async fn disable_coin(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or("No 'coin' field")).to_owned();
    let coin = match lp_coinfind(&ctx, &ticker).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin: {}", ticker),
        Err(err) => return ERR!("!lp_coinfind({}): ", err),
    };
    let coins_ctx = try_s!(CoinsContext::from_ctx(&ctx));

    // If a platform coin is to be disabled, we get all the enabled tokens for this platform coin first.
    let mut tokens_to_disable = coins_ctx.get_tokens_to_disable(&ticker).await;
    // We then add the platform coin to the list of the coins to be disabled.
    tokens_to_disable.insert(ticker.clone());

    // Get all matching orders and active swaps.
    let active_swaps = try_s!(active_swaps_using_coins(&ctx, &tokens_to_disable));
    let still_matching_orders = try_s!(get_matching_orders(&ctx, &tokens_to_disable).await);

    // If there're matching orders or active swaps we return an error.
    if !active_swaps.is_empty() || !still_matching_orders.is_empty() {
        let err = String::from("There're currently matching orders or active swaps for some tokens");
        return disable_coin_err(err, &still_matching_orders, &[], &active_swaps);
    }

    // Proceed with diabling the coin/tokens.
    let mut cancelled_orders = vec![];
    for ticker in &tokens_to_disable {
        log!("disabling {ticker} coin");
        let cancelled_and_matching_orders = cancel_orders_by(&ctx, CancelBy::Coin {
            ticker: ticker.to_string(),
        })
        .await;
        match cancelled_and_matching_orders {
            Ok((cancelled, _)) => {
                cancelled_orders.extend(cancelled);
            },
            Err(err) => {
                return disable_coin_err(err, &still_matching_orders, &cancelled_orders, &active_swaps);
            },
        }
    }

    coins_ctx.remove_coin(coin).await;
    // Ticker shouldn't be a part of the disabled tokens vector in the response.
    tokens_to_disable.remove(&ticker);
    drop_mutability!(tokens_to_disable);
    let res = json!({
        "result": {
            "coin": ticker,
            "cancelled_orders": cancelled_orders,
            "tokens": tokens_to_disable
        }
    });
    Response::builder()
        .body(json::to_vec(&res).unwrap())
        .map_err(|e| ERRL!("{}", e))
}

#[derive(Serialize)]
struct CoinInitResponse<'a> {
    result: &'a str,
    address: String,
    balance: BigDecimal,
    unspendable_balance: BigDecimal,
    coin: &'a str,
    required_confirmations: u64,
    requires_notarization: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    mature_confirmations: Option<u32>,
}

/// Enable a coin in the Electrum mode.
pub async fn electrum(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or("No 'coin' field")).to_owned();
    let coin: MmCoinEnum = try_s!(lp_coininit(&ctx, &ticker, &req).await);
    let balance = try_s!(coin.my_balance().compat().await);
    let res = CoinInitResponse {
        result: "success",
        address: try_s!(coin.my_address()),
        balance: balance.spendable,
        unspendable_balance: balance.unspendable,
        coin: coin.ticker(),
        required_confirmations: coin.required_confirmations(),
        requires_notarization: coin.requires_notarization(),
        mature_confirmations: coin.mature_confirmations(),
    };
    let res = try_s!(json::to_vec(&res));
    Ok(try_s!(Response::builder().body(res)))
}

/// Enable a coin in the local wallet mode.
pub async fn enable(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or("No 'coin' field")).to_owned();
    let coin: MmCoinEnum = try_s!(lp_coininit(&ctx, &ticker, &req).await);
    let balance = try_s!(coin.my_balance().compat().await);
    let res = CoinInitResponse {
        result: "success",
        address: try_s!(coin.my_address()),
        balance: balance.spendable,
        unspendable_balance: balance.unspendable,
        coin: coin.ticker(),
        required_confirmations: coin.required_confirmations(),
        requires_notarization: coin.requires_notarization(),
        mature_confirmations: coin.mature_confirmations(),
    };
    let res = try_s!(json::to_vec(&res));
    let res = try_s!(Response::builder().body(res));

    if coin.is_utxo_in_native_mode() {
        subscribe_to_topic(&ctx, tx_helper_topic(coin.ticker()));
    }
    if ctx.is_watcher() {
        subscribe_to_topic(&ctx, watcher_topic(coin.ticker()));
    }

    Ok(res)
}

#[cfg(target_arch = "wasm32")]
pub fn help() -> HyRes {
    rpc_response(
        INTERNAL_SERVER_ERROR_CODE,
        json!({
            "error":"'help' is only supported in native mode"
        })
        .to_string(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub fn help() -> HyRes {
    rpc_response(
        RESPONSE_OK_STATUS_CODE,
        "
        buy(base, rel, price, relvolume, timeout=10, duration=3600)
        electrum(coin, urls)
        enable(coin, urls, swap_contract_address)
        myprice(base, rel)
        my_balance(coin)
        my_swap_status(params/uuid)
        orderbook(base, rel, duration=3600)
        sell(base, rel, price, basevolume, timeout=10, duration=3600)
        send_raw_transaction(coin, tx_hex)
        setprice(base, rel, price, broadcast=1)
        stop()
        version
        withdraw(coin, amount, to)
    ",
    )
}

/// Get MarketMaker session metrics
pub fn metrics(ctx: MmArc) -> HyRes {
    match ctx.metrics.collect_json().map(|value| value.to_string()) {
        Ok(response) => rpc_response(RESPONSE_OK_STATUS_CODE, response),
        Err(err) => rpc_err_response(INTERNAL_SERVER_ERROR_CODE, &err.to_string()),
    }
}

/// Get my_balance of a coin
pub async fn my_balance(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or("No 'coin' field")).to_owned();
    let coin = match lp_coinfind(&ctx, &ticker).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin: {}", ticker),
        Err(err) => return ERR!("!lp_coinfind({}): {}", ticker, err),
    };
    let my_balance = try_s!(coin.my_balance().compat().await);
    let res = json!({
        "coin": ticker,
        "balance": my_balance.spendable,
        "unspendable_balance": my_balance.unspendable,
        "address": try_s!(coin.my_address()),
    });
    let res = try_s!(json::to_vec(&res));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn stop(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    dispatch_lp_event(ctx.clone(), StopCtxEvent.into()).await;
    // Should delay the shutdown a bit in order not to trip the "stop" RPC call in unit tests.
    // Stopping immediately leads to the "stop" RPC call failing with the "errno 10054" sometimes.
    let fut = async move {
        Timer::sleep(0.05).await;
        if let Err(e) = ctx.stop() {
            error!("Error stopping MmCtx: {}", e);
        }
    };

    // Please note we shouldn't use `MmCtx::spawner` to spawn this future,
    // as all spawned futures will be dropped on `MmArc::stop`, so this future will be dropped as well,
    // and it may lead to an unexpected behaviour.
    common::executor::spawn(fut);

    let res = json!({
        "result": "success"
    });
    let res = try_s!(json::to_vec(&res));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn sim_panic(req: Json) -> Result<Response<Vec<u8>>, String> {
    #[derive(Deserialize)]
    struct Req {
        #[serde(default)]
        mode: String,
    }
    let req: Req = try_s!(json::from_value(req));

    #[derive(Serialize)]
    struct Ret<'a> {
        /// Supported panic modes.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        modes: Vec<Cow<'a, str>>,
    }
    let ret: Ret;

    if req.mode.is_empty() {
        ret = Ret {
            modes: vec!["simple".into()],
        }
    } else if req.mode == "simple" {
        panic!("sim_panic: simple")
    } else {
        return ERR!("No such mode: {}", req.mode);
    }

    let js = try_s!(json::to_vec(&ret));
    Ok(try_s!(Response::builder().body(js)))
}

pub fn version(ctx: MmArc) -> HyRes {
    rpc_response(
        RESPONSE_OK_STATUS_CODE,
        MmVersionResult::new(ctx.mm_version.clone(), ctx.datetime.clone())
            .to_json()
            .to_string(),
    )
}

pub async fn get_peers_info(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    use crate::mm2::lp_network::P2PContext;
    use mm2_libp2p::atomicdex_behaviour::get_peers_info;
    let ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd_tx = ctx.cmd_tx.lock().clone();
    let result = get_peers_info(cmd_tx).await;
    let result = json!({
        "result": result,
    });
    let res = try_s!(json::to_vec(&result));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn get_gossip_mesh(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    use crate::mm2::lp_network::P2PContext;
    use mm2_libp2p::atomicdex_behaviour::get_gossip_mesh;
    let ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd_tx = ctx.cmd_tx.lock().clone();
    let result = get_gossip_mesh(cmd_tx).await;
    let result = json!({
        "result": result,
    });
    let res = try_s!(json::to_vec(&result));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn get_gossip_peer_topics(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    use crate::mm2::lp_network::P2PContext;
    use mm2_libp2p::atomicdex_behaviour::get_gossip_peer_topics;
    let ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd_tx = ctx.cmd_tx.lock().clone();
    let result = get_gossip_peer_topics(cmd_tx).await;
    let result = json!({
        "result": result,
    });
    let res = try_s!(json::to_vec(&result));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn get_gossip_topic_peers(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    use crate::mm2::lp_network::P2PContext;
    use mm2_libp2p::atomicdex_behaviour::get_gossip_topic_peers;
    let ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd_tx = ctx.cmd_tx.lock().clone();
    let result = get_gossip_topic_peers(cmd_tx).await;
    let result = json!({
        "result": result,
    });
    let res = try_s!(json::to_vec(&result));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn get_relay_mesh(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    use crate::mm2::lp_network::P2PContext;
    use mm2_libp2p::atomicdex_behaviour::get_relay_mesh;
    let ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd_tx = ctx.cmd_tx.lock().clone();
    let result = get_relay_mesh(cmd_tx).await;
    let result = json!({
        "result": result,
    });
    let res = try_s!(json::to_vec(&result));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn get_my_peer_id(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    let peer_id = try_s!(ctx.peer_id.ok_or("Peer ID is not initialized"));
    let result = json!({
        "result": peer_id,
    });
    let res = try_s!(json::to_vec(&result));
    Ok(try_s!(Response::builder().body(res)))
}

construct_detailed!(DetailedMinTradingVol, min_trading_vol);

#[derive(Serialize)]
struct MinTradingVolResponse<'a> {
    coin: &'a str,
    #[serde(flatten)]
    volume: DetailedMinTradingVol,
}

/// Get min_trading_vol of a coin
pub async fn min_trading_vol(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or("No 'coin' field")).to_owned();
    let coin = match lp_coinfind(&ctx, &ticker).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin: {}", ticker),
        Err(err) => return ERR!("!lp_coinfind({}): {}", ticker, err),
    };
    let min_trading_vol = coin.min_trading_vol();
    let response = MinTradingVolResponse {
        coin: &ticker,
        volume: min_trading_vol.into(),
    };
    let res = json!({
        "result": response,
    });
    let res = try_s!(json::to_vec(&res));
    Ok(try_s!(Response::builder().body(res)))
}
