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

use coins::coin_errors::MyAddressError;
use coins::{disable_coin as disable_coin_impl, lp_coinfind, lp_coininit, BalanceError, MmCoinEnum};
use common::executor::{spawn, Timer};
use common::log::error;
use common::{rpc_err_response, rpc_response, HyRes};
use derive_more::Display;
use futures::compat::Future01CompatExt;
use http::Response;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_metrics::MetricsOps;
use mm2_number::{construct_detailed, BigDecimal};
use serde_json::{self as json, Value as Json};
use std::borrow::Cow;

use crate::mm2::lp_dispatcher::{dispatch_lp_event, StopCtxEvent};
use crate::mm2::lp_network::subscribe_to_topic;
use crate::mm2::lp_ordermatch::{cancel_orders_by, CancelBy};
use crate::mm2::lp_swap::{active_swaps_using_coin, tx_helper_topic};
use crate::mm2::MmVersionResult;

#[derive(Debug, Display)]
pub enum LpCommandsLegacyError {
    #[display(fmt = "{}", _0)]
    ActiveSwapsError(String),
    CoinAlreadyInitialized(String),
    #[display(fmt = "!lp_coinfind({}): {}", _0, _1)]
    CoinFindError(String, String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
    #[display(fmt = "InvalidResponse {}", _0)]
    InvalidResponse(String),
    #[display(fmt = "No 'coin' field")]
    NoCoinField,
    #[display(fmt = "No such coin: {}", _0)]
    NoSuchCoin(String),
    #[display(fmt = "No such mode: {}", _0)]
    NoSuchMode(String),
}

impl From<http::Error> for LpCommandsLegacyError {
    fn from(err: http::Error) -> Self { Self::InvalidResponse(err.to_string()) }
}

async fn lp_coinfind_coin_enum_and_ticker(
    ctx: &MmArc,
    req: &Json,
) -> Result<(String, MmCoinEnum), MmError<LpCommandsLegacyError>> {
    let ticker = req["coin"]
        .as_str()
        .ok_or(LpCommandsLegacyError::NoCoinField)
        .map_to_mm(|e| e)?
        .to_owned();
    let coin = match lp_coinfind(ctx, &ticker).await {
        Ok(Some(t)) => t,
        Ok(None) => return Err(MmError::new(LpCommandsLegacyError::NoSuchCoin(ticker))),
        Err(err) => return Err(MmError::new(LpCommandsLegacyError::CoinFindError(ticker, err))),
    };
    Ok((ticker, coin))
}

async fn lp_coininit_coin_enum_and_ticker(
    ctx: &MmArc,
    req: &Json,
) -> Result<(String, MmCoinEnum), MmError<LpCommandsLegacyError>> {
    let ticker = req["coin"]
        .as_str()
        .ok_or(LpCommandsLegacyError::NoCoinField)
        .map_to_mm(|e| e)?
        .to_owned();
    let coin: MmCoinEnum = lp_coininit(ctx, &ticker, req)
        .await
        .map_to_mm(LpCommandsLegacyError::CoinAlreadyInitialized)?;
    Ok((ticker, coin))
}

/// Attempts to disable the coin
pub async fn disable_coin(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, MmError<LpCommandsLegacyError>> {
    let (ticker, _coin) = lp_coinfind_coin_enum_and_ticker(&ctx, &req).await?;
    let swaps = active_swaps_using_coin(&ctx, &ticker).map_to_mm(LpCommandsLegacyError::ActiveSwapsError)?;
    if !swaps.is_empty() {
        let err = json!({
            "error": format!("There're active swaps using {}", ticker),
            "swaps": swaps,
        });
        return Ok(Response::builder().status(500).body(json::to_vec(&err).unwrap())?);
    }
    let (cancelled, still_matching) = cancel_orders_by(&ctx, CancelBy::Coin { ticker: ticker.clone() })
        .await
        .map_to_mm(LpCommandsLegacyError::Internal)?;
    if !still_matching.is_empty() {
        let err = json!({
            "error": format!("There're currently matching orders using {}", ticker),
            "orders": {
                "matching": still_matching,
                "cancelled": cancelled,
            }
        });
        return Ok(Response::builder().status(500).body(json::to_vec(&err).unwrap())?);
    }

    disable_coin_impl(&ctx, &ticker)
        .await
        .map_to_mm(LpCommandsLegacyError::Internal)?;
    let res = json!({
        "result": {
            "coin": ticker,
            "cancelled_orders": cancelled,
        }
    });
    Ok(Response::builder().body(json::to_vec(&res).unwrap())?)
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

#[derive(Debug, Display)]
pub enum CoinInitResponseError {
    #[display(fmt = "CoinFindError: {}", _0)]
    CoinFindError(String),
    BalanceError(BalanceError),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
    #[display(fmt = "InvalidResponse: {}", _0)]
    InvalidResponse(String),
    AddressError(String),
}

impl From<http::Error> for CoinInitResponseError {
    fn from(err: http::Error) -> Self { Self::InvalidResponse(err.to_string()) }
}

impl From<MyAddressError> for CoinInitResponseError {
    fn from(err: MyAddressError) -> Self { Self::AddressError(err.to_string()) }
}

/// Enable a coin in the Electrum mode.
pub async fn electrum(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, MmError<CoinInitResponseError>> {
    let (_ticker, coin) = lp_coininit_coin_enum_and_ticker(&ctx, &req)
        .await
        .mm_err(|err| CoinInitResponseError::CoinFindError(err.to_string()))?;
    let balance = coin
        .my_balance()
        .compat()
        .await
        .mm_err(CoinInitResponseError::BalanceError)?;
    let res = CoinInitResponse {
        result: "success",
        address: coin.my_address()?,
        balance: balance.spendable,
        unspendable_balance: balance.unspendable,
        coin: coin.ticker(),
        required_confirmations: coin.required_confirmations(),
        requires_notarization: coin.requires_notarization(),
        mature_confirmations: coin.mature_confirmations(),
    };
    let res = json::to_vec(&res).map_to_mm(|err| CoinInitResponseError::Internal(err.to_string()))?;
    Ok(Response::builder().body(res)?)
}

/// Enable a coin in the local wallet mode.
pub async fn enable(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, MmError<CoinInitResponseError>> {
    let (_ticker, coin) = lp_coininit_coin_enum_and_ticker(&ctx, &req)
        .await
        .mm_err(|err| CoinInitResponseError::CoinFindError(err.to_string()))?;
    let balance = coin
        .my_balance()
        .compat()
        .await
        .mm_err(CoinInitResponseError::BalanceError)?;
    let res = CoinInitResponse {
        result: "success",
        address: coin.my_address()?,
        balance: balance.spendable,
        unspendable_balance: balance.unspendable,
        coin: coin.ticker(),
        required_confirmations: coin.required_confirmations(),
        requires_notarization: coin.requires_notarization(),
        mature_confirmations: coin.mature_confirmations(),
    };
    let res = json::to_vec(&res).map_to_mm(|err| CoinInitResponseError::Internal(err.to_string()))?;
    let res = Response::builder().body(res)?;

    if coin.is_utxo_in_native_mode() {
        subscribe_to_topic(&ctx, tx_helper_topic(coin.ticker()));
    }

    Ok(res)
}

#[cfg(target_arch = "wasm32")]
pub fn help() -> HyRes {
    rpc_response(
        500,
        json!({
            "error":"'help' is only supported in native mode"
        })
        .to_string(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
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
        Err(err) => rpc_err_response(500, &err.to_string()),
    }
}

/// Get my_balance of a coin
pub async fn my_balance(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, MmError<BalanceError>> {
    let (ticker, coin) = lp_coinfind_coin_enum_and_ticker(&ctx, &req)
        .await
        .mm_err(|err| BalanceError::Internal(err.to_string()))?;
    let balance = coin.my_balance().compat().await?;
    let res = json!({
        "coin": ticker,
        "balance": balance.spendable,
        "unspendable_balance": balance.unspendable,
        "address": coin.my_address()?,
    });
    let res = json::to_vec(&res).map_to_mm(|err| BalanceError::Internal(err.to_string()))?;
    Response::builder()
        .body(res)
        .map_to_mm(|err| BalanceError::InvalidResponse(err.to_string()))
}

pub async fn stop(ctx: MmArc) -> Result<Response<Vec<u8>>, MmError<LpCommandsLegacyError>> {
    dispatch_lp_event(ctx.clone(), StopCtxEvent.into()).await;
    // Should delay the shutdown a bit in order not to trip the "stop" RPC call in unit tests.
    // Stopping immediately leads to the "stop" RPC call failing with the "errno 10054" sometimes.
    spawn(async move {
        Timer::sleep(0.05).await;
        if let Err(e) = ctx.stop() {
            error!("Error stopping MmCtx: {}", e);
        }
    });
    let res = json!({
        "result": "success"
    });
    let res = json::to_vec(&res).map_to_mm(|err| LpCommandsLegacyError::Internal(err.to_string()))?;
    Ok(Response::builder().body(res)?)
}

pub async fn sim_panic(req: Json) -> Result<Response<Vec<u8>>, MmError<LpCommandsLegacyError>> {
    #[derive(Deserialize)]
    struct Req {
        #[serde(default)]
        mode: String,
    }
    let req: Req = json::from_value(req).map_to_mm(|err| LpCommandsLegacyError::Internal(err.to_string()))?;

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
        return Err(MmError::new(LpCommandsLegacyError::NoSuchMode(req.mode)));
    }

    let js = json::to_vec(&ret).map_to_mm(|err| LpCommandsLegacyError::Internal(err.to_string()))?;
    Response::builder()
        .body(js)
        .map_to_mm(|err| LpCommandsLegacyError::InvalidResponse(err.to_string()))
}

pub fn version() -> HyRes { rpc_response(200, MmVersionResult::new().to_json().to_string()) }

#[derive(Debug, Display)]
pub enum GossipPeerError {
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
    #[display(fmt = "InvalidResponse: {}", _0)]
    InvalidResponse(String),
    #[display(fmt = "Peer ID is not initialized")]
    PeerIDNotInitialized,
}

impl From<http::Error> for GossipPeerError {
    fn from(err: http::Error) -> Self { Self::InvalidResponse(err.to_string()) }
}

pub async fn get_peers_info(ctx: MmArc) -> Result<Response<Vec<u8>>, MmError<GossipPeerError>> {
    use crate::mm2::lp_network::P2PContext;
    use mm2_libp2p::atomicdex_behaviour::get_peers_info;
    let ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd_tx = ctx.cmd_tx.lock().clone();
    let result = get_peers_info(cmd_tx).await;
    let result = json!({
        "result": result,
    });
    let res = json::to_vec(&result).map_to_mm(|err| GossipPeerError::Internal(err.to_string()))?;
    Ok(Response::builder().body(res)?)
}

pub async fn get_gossip_mesh(ctx: MmArc) -> Result<Response<Vec<u8>>, MmError<GossipPeerError>> {
    use crate::mm2::lp_network::P2PContext;
    use mm2_libp2p::atomicdex_behaviour::get_gossip_mesh;
    let ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd_tx = ctx.cmd_tx.lock().clone();
    let result = get_gossip_mesh(cmd_tx).await;
    let result = json!({
        "result": result,
    });
    let res = json::to_vec(&result).map_to_mm(|err| GossipPeerError::Internal(err.to_string()))?;
    Ok(Response::builder().body(res)?)
}

pub async fn get_gossip_peer_topics(ctx: MmArc) -> Result<Response<Vec<u8>>, MmError<GossipPeerError>> {
    use crate::mm2::lp_network::P2PContext;
    use mm2_libp2p::atomicdex_behaviour::get_gossip_peer_topics;
    let ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd_tx = ctx.cmd_tx.lock().clone();
    let result = get_gossip_peer_topics(cmd_tx).await;
    let result = json!({
        "result": result,
    });
    let res = json::to_vec(&result).map_to_mm(|err| GossipPeerError::Internal(err.to_string()))?;
    Ok(Response::builder().body(res)?)
}

pub async fn get_gossip_topic_peers(ctx: MmArc) -> Result<Response<Vec<u8>>, MmError<GossipPeerError>> {
    use crate::mm2::lp_network::P2PContext;
    use mm2_libp2p::atomicdex_behaviour::get_gossip_topic_peers;
    let ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd_tx = ctx.cmd_tx.lock().clone();
    let result = get_gossip_topic_peers(cmd_tx).await;
    let result = json!({
        "result": result,
    });
    let res = json::to_vec(&result).map_to_mm(|err| GossipPeerError::Internal(err.to_string()))?;
    Ok(Response::builder().body(res)?)
}

pub async fn get_relay_mesh(ctx: MmArc) -> Result<Response<Vec<u8>>, MmError<GossipPeerError>> {
    use crate::mm2::lp_network::P2PContext;
    use mm2_libp2p::atomicdex_behaviour::get_relay_mesh;
    let ctx = P2PContext::fetch_from_mm_arc(&ctx);
    let cmd_tx = ctx.cmd_tx.lock().clone();
    let result = get_relay_mesh(cmd_tx).await;
    let result = json!({
        "result": result,
    });
    let res = json::to_vec(&result).map_to_mm(|err| GossipPeerError::Internal(err.to_string()))?;
    Ok(Response::builder().body(res)?)
}

pub async fn get_my_peer_id(ctx: MmArc) -> Result<Response<Vec<u8>>, MmError<GossipPeerError>> {
    let peer_id = ctx
        .peer_id
        .ok_or(GossipPeerError::PeerIDNotInitialized)
        .map_to_mm(|e| e)?;
    let result = json!({
        "result": peer_id,
    });
    let res = json::to_vec(&result).map_to_mm(|err| GossipPeerError::Internal(err.to_string()))?;
    Ok(Response::builder().body(res)?)
}

construct_detailed!(DetailedMinTradingVol, min_trading_vol);

#[derive(Serialize)]
struct MinTradingVolResponse<'a> {
    coin: &'a str,
    #[serde(flatten)]
    volume: DetailedMinTradingVol,
}

#[derive(Debug, Display)]
pub enum MinTradingVolResponseError {
    #[display(fmt = "CoinFindError: {}", _0)]
    CoinFindError(String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
    #[display(fmt = "InvalidResponse: {}", _0)]
    InvalidResponse(String),
}

impl From<http::Error> for MinTradingVolResponseError {
    fn from(err: http::Error) -> Self { Self::InvalidResponse(err.to_string()) }
}

/// Get min_trading_vol of a coin
pub async fn min_trading_vol(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, MmError<MinTradingVolResponseError>> {
    let (ticker, coin) = lp_coinfind_coin_enum_and_ticker(&ctx, &req)
        .await
        .mm_err(|err| MinTradingVolResponseError::CoinFindError(err.to_string()))?;
    let min_trading_vol = coin.min_trading_vol();
    let response = MinTradingVolResponse {
        coin: &ticker,
        volume: min_trading_vol.into(),
    };
    let res = json!({
        "result": response,
    });
    let res = json::to_vec(&res).map_to_mm(|err| MinTradingVolResponseError::Internal(err.to_string()))?;
    Ok(Response::builder().body(res)?)
}
