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
 ******************************************************************************/
//
//  rpc.rs
//
//  Copyright © 2014-2018 SuperNET. All rights reserved.
//
#![cfg_attr(not(feature = "native"), allow(unused_imports))]
#![cfg_attr(not(feature = "native"), allow(dead_code))]

use coins::{convert_address, get_enabled_coins, get_trade_fee, kmd_rewards_info, my_tx_history, send_raw_transaction,
            set_required_confirmations, set_requires_notarization, show_priv_key, validate_address, withdraw};
use common::mm_ctx::MmArc;
#[cfg(feature = "native")] use common::wio::{CORE, CPUPOOL};
use common::{err_to_rpc_json_string, err_tp_rpc_json, HyRes};
use futures::compat::Future01CompatExt;
use futures::future::{join_all, FutureExt, TryFutureExt};
use http::header::{HeaderValue, ACCESS_CONTROL_ALLOW_ORIGIN};
use http::request::Parts;
use http::{Method, Request, Response};
#[cfg(feature = "native")] use hyper::{self, Server};
use serde_json::{self as json, Value as Json};
use std::future::Future as Future03;
use std::net::SocketAddr;

#[cfg(not(feature = "wallet-only"))]
use crate::mm2::lp_ordermatch::{buy, cancel_all_orders, cancel_order, my_orders, order_status, orderbook, sell,
                                set_price};
#[cfg(not(feature = "wallet-only"))]
use crate::mm2::lp_swap::{coins_needed_for_kick_start, import_swaps, list_banned_pubkeys, max_taker_vol,
                          my_recent_swaps, my_swap_status, recover_funds_of_swap, stats_swap_status, unban_pubkeys};

#[path = "rpc/lp_commands.rs"] pub mod lp_commands;
use self::lp_commands::*;
use hyper::Body;

/// Lists the RPC method not requiring the "userpass" authentication.  
/// None is also public to skip auth and display proper error in case of method is missing
const PUBLIC_METHODS: &[Option<&str>] = &[
    // Sorted alphanumerically (on the first letter) for readability.
    Some("fundvalue"),
    Some("getprice"),
    Some("getpeers"),
    Some("getcoins"),
    Some("help"),
    Some("metrics"),
    Some("notify"), // Manually checks the peer's public key.
    Some("orderbook"),
    Some("passphrase"), // Manually checks the "passphrase".
    Some("pricearray"),
    Some("psock"),
    Some("statsdisp"),
    Some("stats_swap_status"),
    Some("tradesarray"),
    Some("ticker"),
    None,
];

#[allow(unused_macros)]
macro_rules! unwrap_or_err_response {
    ($e:expr, $($args:tt)*) => {
        match $e {
            Ok(ok) => ok,
            Err(err) => return rpc_err_response(500, &ERRL!("{}", err)),
        }
    };
}

fn auth(json: &Json, ctx: &MmArc) -> Result<(), &'static str> {
    if !PUBLIC_METHODS.contains(&json["method"].as_str()) {
        if !json["userpass"].is_string() {
            return Err("Userpass is not set!");
        }

        if json["userpass"] != ctx.conf["rpc_password"] {
            return Err("Userpass is invalid!");
        }
    }
    Ok(())
}

/// Result of `fn dispatcher`.
pub enum DispatcherRes {
    /// `fn dispatcher` has found a Rust handler for the RPC "method".
    Match(HyRes),
    /// No handler found by `fn dispatcher`. Returning the `Json` request in order for it to be handled elsewhere.
    NoMatch(Json),
}

/// Using async/await (futures 0.3) in `dispatcher`
/// will pave the way for porting the remaining system threading code to async/await green threads.
fn hyres(handler: impl Future03<Output = Result<Response<Vec<u8>>, String>> + Send + 'static) -> HyRes {
    Box::new(handler.boxed().compat())
}

/// The dispatcher, with full control over the HTTP result and the way we run the `Future` producing it.
///
/// Invoked both directly from the HTTP endpoint handler below and in a delayed fashion from `lp_command_q_loop`.
///
/// Returns `None` if the requested "method" wasn't found among the ported RPC methods and has to be handled elsewhere.
pub fn dispatcher(req: Json, ctx: MmArc) -> DispatcherRes {
    //log! ("dispatcher] " (json::to_string (&req) .unwrap()));
    let method = match req["method"].clone() {
        Json::String(method) => method,
        _ => return DispatcherRes::NoMatch(req),
    };
    DispatcherRes::Match(match &method[..] {
        // Sorted alphanumerically (on the first latter) for readability.
        // "autoprice" => lp_autoprice (ctx, req),
        #[cfg(not(feature = "wallet-only"))]
        "buy" => hyres(buy(ctx, req)),
        #[cfg(not(feature = "wallet-only"))]
        "cancel_all_orders" => hyres(cancel_all_orders(ctx, req)),
        #[cfg(not(feature = "wallet-only"))]
        "cancel_order" => hyres(cancel_order(ctx, req)),
        #[cfg(not(feature = "wallet-only"))]
        "coins_needed_for_kick_start" => hyres(coins_needed_for_kick_start(ctx)),
        "convertaddress" => hyres(convert_address(ctx, req)),
        "disable_coin" => hyres(disable_coin(ctx, req)),
        "electrum" => hyres(electrum(ctx, req)),
        "enable" => hyres(enable(ctx, req)),
        "get_enabled_coins" => hyres(get_enabled_coins(ctx)),
        "get_gossip_mesh" => hyres(get_gossip_mesh(ctx)),
        "get_gossip_peer_topics" => hyres(get_gossip_peer_topics(ctx)),
        "get_gossip_topic_peers" => hyres(get_gossip_topic_peers(ctx)),
        "get_my_peer_id" => hyres(get_my_peer_id(ctx)),
        "get_peers_info" => hyres(get_peers_info(ctx)),
        "get_relay_mesh" => hyres(get_relay_mesh(ctx)),
        #[cfg(not(feature = "wallet-only"))]
        "get_trade_fee" => hyres(get_trade_fee(ctx, req)),
        // "fundvalue" => lp_fundvalue (ctx, req, false),
        "help" => help(),
        #[cfg(not(feature = "wallet-only"))]
        "import_swaps" => {
            #[cfg(feature = "native")]
            {
                Box::new(CPUPOOL.spawn_fn(move || hyres(import_swaps(ctx, req))))
            }
            #[cfg(not(feature = "native"))]
            {
                return DispatcherRes::NoMatch(req);
            }
        },
        "kmd_rewards_info" => hyres(kmd_rewards_info(ctx)),
        // "inventory" => inventory (ctx, req),
        #[cfg(not(feature = "wallet-only"))]
        "list_banned_pubkeys" => hyres(list_banned_pubkeys(ctx)),
        #[cfg(not(feature = "wallet-only"))]
        "max_taker_vol" => hyres(max_taker_vol(ctx, req)),
        "metrics" => metrics(ctx),
        "my_balance" => hyres(my_balance(ctx, req)),
        #[cfg(not(feature = "wallet-only"))]
        "my_orders" => hyres(my_orders(ctx)),
        #[cfg(not(feature = "wallet-only"))]
        "my_recent_swaps" => my_recent_swaps(ctx, req),
        #[cfg(not(feature = "wallet-only"))]
        "my_swap_status" => my_swap_status(ctx, req),
        "my_tx_history" => my_tx_history(ctx, req),
        #[cfg(not(feature = "wallet-only"))]
        "order_status" => hyres(order_status(ctx, req)),
        #[cfg(not(feature = "wallet-only"))]
        "orderbook" => hyres(orderbook(ctx, req)),
        #[cfg(not(feature = "wallet-only"))]
        "sim_panic" => hyres(sim_panic(req)),
        #[cfg(not(feature = "wallet-only"))]
        "recover_funds_of_swap" => {
            #[cfg(feature = "native")]
            {
                Box::new(CPUPOOL.spawn_fn(move || hyres(recover_funds_of_swap(ctx, req))))
            }
            #[cfg(not(feature = "native"))]
            {
                return DispatcherRes::NoMatch(req);
            }
        },
        // "passphrase" => passphrase (ctx, req),
        #[cfg(not(feature = "wallet-only"))]
        "sell" => hyres(sell(ctx, req)),
        "show_priv_key" => hyres(show_priv_key(ctx, req)),
        "send_raw_transaction" => hyres(send_raw_transaction(ctx, req)),
        #[cfg(not(feature = "wallet-only"))]
        "set_required_confirmations" => hyres(set_required_confirmations(ctx, req)),
        #[cfg(not(feature = "wallet-only"))]
        "set_requires_notarization" => hyres(set_requires_notarization(ctx, req)),
        #[cfg(not(feature = "wallet-only"))]
        "setprice" => hyres(set_price(ctx, req)),
        #[cfg(not(feature = "wallet-only"))]
        "stats_swap_status" => stats_swap_status(ctx, req),
        "stop" => stop(ctx),
        #[cfg(not(feature = "wallet-only"))]
        "unban_pubkeys" => hyres(unban_pubkeys(ctx, req)),
        "validateaddress" => hyres(validate_address(ctx, req)),
        "version" => version(),
        "withdraw" => hyres(withdraw(ctx, req)),
        _ => return DispatcherRes::NoMatch(req),
    })
}

async fn rpc_serviceʹ(ctx: MmArc, req: Parts, reqᵇ: Body, client: SocketAddr) -> Result<Response<Vec<u8>>, String> {
    if req.method != Method::POST {
        return ERR!("Only POST requests are supported!");
    }

    let reqᵇ = try_s!(hyper::body::to_bytes(reqᵇ).await);
    let reqʲ: Json = try_s!(json::from_slice(&reqᵇ));
    match reqʲ.as_array() {
        Some(requests) => {
            let mut futures = Vec::with_capacity(requests.len());
            for request in requests {
                futures.push(process_single_request(ctx.clone(), request.clone(), client));
            }
            let results = join_all(futures).await;
            let responses: Vec<_> = results
                .into_iter()
                .map(|resp| match resp {
                    Ok(r) => match json::from_slice(r.body()) {
                        Ok(j) => j,
                        Err(e) => {
                            log!("Response " [r] " is not a valid JSON, err " (e));
                            Json::Null
                        },
                    },
                    Err(e) => err_tp_rpc_json(e),
                })
                .collect();
            let res = try_s!(json::to_vec(&responses));
            Ok(try_s!(Response::builder().body(res)))
        },
        None => process_single_request(ctx, reqʲ, client).await,
    }
}

async fn process_single_request(ctx: MmArc, req: Json, client: SocketAddr) -> Result<Response<Vec<u8>>, String> {
    // https://github.com/artemii235/SuperNET/issues/368
    let local_only = ctx.conf["rpc_local_only"].as_bool().unwrap_or(true);
    if local_only && !client.ip().is_loopback() && !PUBLIC_METHODS.contains(&req["method"].as_str()) {
        return ERR!("Selected method can be called from localhost only!");
    }
    try_s!(auth(&req, &ctx));

    let handler = match dispatcher(req, ctx.clone()) {
        DispatcherRes::Match(handler) => handler,
        DispatcherRes::NoMatch(req) => return ERR!("No such method: {:?}", req["method"]),
    };
    let res = try_s!(handler.compat().await);
    Ok(res)
}

#[cfg(feature = "native")]
async fn rpc_service(req: Request<Body>, ctx_h: u32, client: SocketAddr) -> Response<Body> {
    macro_rules! try_sf {
        ($value: expr) => {
            match $value {
                Ok(ok) => ok,
                Err(err) => {
                    log!("RPC error response: "(err));
                    let ebody = err_to_rpc_json_string(&fomat!((err)));
                    return unwrap!(Response::builder().status(500).body(Body::from(ebody)));
                },
            }
        };
    }

    let ctx = try_sf!(MmArc::from_ffi_handle(ctx_h));
    // https://github.com/artemii235/SuperNET/issues/219
    let rpc_cors = match ctx.conf["rpccors"].as_str() {
        Some(s) => try_sf!(HeaderValue::from_str(s)),
        None => HeaderValue::from_static("http://localhost:3000"),
    };

    // Convert the native Hyper stream into a portable stream of `Bytes`.
    let (req, reqᵇ) = req.into_parts();
    let (mut parts, body) = match rpc_serviceʹ(ctx, req, reqᵇ, client).await {
        Ok(r) => r.into_parts(),
        Err(err) => {
            log!("RPC error response: "(err));
            let ebody = err_to_rpc_json_string(&err);
            return unwrap!(Response::builder()
                .status(500)
                .header(ACCESS_CONTROL_ALLOW_ORIGIN, rpc_cors)
                .body(Body::from(ebody)));
        },
    };
    parts.headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, rpc_cors);
    Response::from_parts(parts, Body::from(body))
}

#[cfg(feature = "native")]
pub extern "C" fn spawn_rpc(ctx_h: u32) {
    use hyper::server::conn::AddrStream;
    use hyper::service::{make_service_fn, service_fn};
    use std::convert::Infallible;

    // NB: We need to manually handle the incoming connections in order to get the remote IP address,
    // cf. https://github.com/hyperium/hyper/issues/1410#issuecomment-419510220.
    // Although if the ability to access the remote IP address is solved by the Hyper in the future
    // then we might want to refactor into starting it ideomatically in order to benefit from a more graceful shutdown,
    // cf. https://github.com/hyperium/hyper/pull/1640.

    let ctx = unwrap!(MmArc::from_ffi_handle(ctx_h), "No context");

    let rpc_ip_port = unwrap!(ctx.rpc_ip_port());
    CORE.0.enter(|| {
        let server = unwrap!(Server::try_bind(&rpc_ip_port), "Can't bind on {}", rpc_ip_port);
        let make_svc = make_service_fn(move |socket: &AddrStream| {
            let remote_addr = socket.remote_addr();
            async move {
                Ok::<_, Infallible>(service_fn(move |req: Request<Body>| async move {
                    let res = rpc_service(req, ctx_h, remote_addr).await;
                    Ok::<_, Infallible>(res)
                }))
            }
        });

        let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel::<()>();
        let mut shutdown_tx = Some(shutdown_tx);
        ctx.on_stop(Box::new(move || {
            if let Some(shutdown_tx) = shutdown_tx.take() {
                log!("on_stop] firing shutdown_tx!");
                if shutdown_tx.send(()).is_err() {
                    log!("on_stop] Warning, shutdown_tx already closed")
                }
                Ok(())
            } else {
                ERR!("on_stop callback called twice!")
            }
        }));

        let server = server
            .http1_half_close(false)
            .serve(make_svc)
            .with_graceful_shutdown(shutdown_rx.then(|_| futures::future::ready(())));

        let server = server.then(|r| {
            if let Err(err) = r {
                log!((err));
            };
            futures::future::ready(())
        });

        let rpc_ip_port = unwrap!(ctx.rpc_ip_port());
        CORE.0.spawn({
            log!(">>>>>>>>>> DEX stats " (rpc_ip_port.ip())":"(rpc_ip_port.port()) " \
                DEX stats API enabled at unixtime." (gstuff::now_ms() / 1000) " <<<<<<<<<");
            let _ = ctx.rpc_started.pin(true);
            server
        });
    });
}

#[cfg(not(feature = "native"))]
pub extern "C" fn spawn_rpc(_ctx_h: u32) { unimplemented!() }

#[cfg(not(feature = "native"))]
pub fn init_header_slots() {
    use common::header::RPC_SERVICE;
    use std::pin::Pin;

    fn rpc_service_fn(
        ctx: MmArc,
        req: Parts,
        reqᵇ: Box<dyn Stream<Item = Bytes, Error = String> + Send>,
        client: SocketAddr,
    ) -> Pin<Box<dyn Future03<Output = Result<Response<Vec<u8>>, String>> + Send>> {
        Box::pin(rpc_serviceʹ(ctx, req, reqᵇ, client))
    }
    let _ = RPC_SERVICE.pin(rpc_service_fn);
}
