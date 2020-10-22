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
//  rpc_commands.rs
//  marketmaker
//

#![cfg_attr(not(feature = "native"), allow(dead_code))]
#![cfg_attr(not(feature = "native"), allow(unused_imports))]

use coins::{disable_coin as disable_coin_impl, lp_coinfindᵃ, lp_coininit, MmCoinEnum};
use common::executor::{spawn, Timer};
use common::mm_ctx::MmArc;
use common::{rpc_err_response, rpc_response, HyRes, MM_DATETIME, MM_VERSION};
use futures::compat::Future01CompatExt;
use http::Response;
use serde_json::{self as json, Value as Json};
use std::borrow::Cow;

#[cfg(not(feature = "wallet-only"))]
use crate::mm2::lp_ordermatch::{cancel_orders_by, CancelBy};
#[cfg(not(feature = "wallet-only"))]
use crate::mm2::lp_swap::active_swaps_using_coin;

/// Attempts to disable the coin
pub async fn disable_coin(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or("No 'coin' field")).to_owned();
    let _coin = match lp_coinfindᵃ(&ctx, &ticker).await {
        // Use lp_coinfindᵃ when async.
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin: {}", ticker),
        Err(err) => return ERR!("!lp_coinfind({}): ", err),
    };
    let swaps = try_s!(active_swaps_using_coin(&ctx, &ticker));
    if !swaps.is_empty() {
        let err = json!({
            "error": fomat! ("There're active swaps using " (ticker)),
            "swaps": swaps,
        });
        return Response::builder()
            .status(500)
            .body(json::to_vec(&err).unwrap())
            .map_err(|e| ERRL!("{}", e));
    }
    let (cancelled, still_matching) = try_s!(cancel_orders_by(&ctx, CancelBy::Coin { ticker: ticker.clone() }).await);
    if !still_matching.is_empty() {
        let err = json!({
            "error": fomat! ("There're currently matching orders using " (ticker)),
            "orders": {
                "matching": still_matching,
                "cancelled": cancelled,
            }
        });
        return Response::builder()
            .status(500)
            .body(json::to_vec(&err).unwrap())
            .map_err(|e| ERRL!("{}", e));
    }

    try_s!(disable_coin_impl(&ctx, &ticker).await);
    let res = json!({
        "result": {
            "coin": ticker,
            "cancelled_orders": cancelled,
        }
    });
    Response::builder()
        .body(json::to_vec(&res).unwrap())
        .map_err(|e| ERRL!("{}", e))
}

/// Enable a coin in the Electrum mode.
pub async fn electrum(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or("No 'coin' field")).to_owned();
    let coin: MmCoinEnum = try_s!(lp_coininit(&ctx, &ticker, &req).await);
    let balance = try_s!(coin.my_balance().compat().await);

    let res = json! ({
        "result": "success",
        "address": try_s!(coin.my_address()),
        "balance": balance,
        "coin": coin.ticker(),
        "required_confirmations": coin.required_confirmations(),
        "requires_notarization": coin.requires_notarization(),
    });
    let res = try_s!(json::to_vec(&res));
    Ok(try_s!(Response::builder().body(res)))
}

/// Enable a coin in the local wallet mode.
pub async fn enable(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or("No 'coin' field")).to_owned();
    let coin: MmCoinEnum = try_s!(lp_coininit(&ctx, &ticker, &req).await);
    let balance = try_s!(coin.my_balance().compat().await);

    let res = json! ({
        "result": "success",
        "address": try_s!(coin.my_address()),
        "balance": balance,
        "coin": coin.ticker(),
        "required_confirmations": coin.required_confirmations(),
        "requires_notarization": coin.requires_notarization(),
    });
    let res = try_s!(json::to_vec(&res));
    Ok(try_s!(Response::builder().body(res)))
}

pub fn help() -> HyRes {
    rpc_response(
        200,
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
        Ok(response) => rpc_response(200, response),
        Err(err) => rpc_err_response(500, &err),
    }
}

/// Get my_balance of a coin
pub async fn my_balance(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or("No 'coin' field")).to_owned();
    let coin = match lp_coinfindᵃ(&ctx, &ticker).await {
        // Use lp_coinfindᵃ when async.
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin: {}", ticker),
        Err(err) => return ERR!("!lp_coinfind({}): {}", ticker, err),
    };
    let my_balance = try_s!(coin.my_balance().compat().await);
    let res = json!({
        "coin": ticker,
        "balance": my_balance,
        "address": try_s!(coin.my_address()),
    });
    let res = try_s!(json::to_vec(&res));
    Ok(try_s!(Response::builder().body(res)))
}

pub fn stop(ctx: MmArc) -> HyRes {
    // Should delay the shutdown a bit in order not to trip the "stop" RPC call in unit tests.
    // Stopping immediately leads to the "stop" RPC call failing with the "errno 10054" sometimes.
    spawn(async move {
        Timer::sleep(0.05).await;
        ctx.stop();
    });
    rpc_response(200, r#"{"result": "success"}"#)
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

pub fn version() -> HyRes {
    rpc_response(
        200,
        json! ({
            "result": MM_VERSION,
            "datetime": MM_DATETIME
        })
        .to_string(),
    )
}

pub async fn get_peers_info(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    use crate::mm2::lp_network::P2PContext;
    use mm2_libp2p::atomicdex_behaviour::get_peers_info;
    let ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd_tx = ctx.cmd_tx.lock().await.clone();
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
    let cmd_tx = ctx.cmd_tx.lock().await.clone();
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
    let cmd_tx = ctx.cmd_tx.lock().await.clone();
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
    let cmd_tx = ctx.cmd_tx.lock().await.clone();
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
    let cmd_tx = ctx.cmd_tx.lock().await.clone();
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

// AP: Inventory is not documented and not used as of now, commented out
/*
pub fn inventory (ctx: MmArc, req: Json) -> HyRes {
    let ticker = match req["coin"].as_str() {Some (s) => s, None => return rpc_err_response (500, "No 'coin' argument in request")};
    let coin = match lp_coinfind (&ctx, ticker) {
        Ok (Some (t)) => t,
        Ok (None) => return rpc_err_response (500, &fomat! ("No such coin: " (ticker))),
        Err (err) => return rpc_err_response (500, &fomat! ("!lp_coinfind(" (ticker) "): " (err)))
    };
    let ii = coin.iguana_info();

    unsafe {lp::LP_address (ii, (*ii).smartaddr.as_mut_ptr())};
    if unsafe {nonz (lp::G.LP_privkey.bytes)} {
        unsafe {lp::LP_privkey_init (-1, ii, lp::G.LP_privkey, lp::G.LP_mypub25519)};
    } else {
        log! ("inventory] no LP_privkey");
    }
    let retjson = json! ({
        "result": "success",
        "coin": ticker,
        "timestamp": now_ms() / 1000,
        "alice": []  // LP_inventory(coin)
        // "bob": LP_inventory(coin,1)
    });
    //LP_smartutxos_push(ptr);
    rpc_response (200, try_h! (json::to_string (&retjson)))
}
*/
