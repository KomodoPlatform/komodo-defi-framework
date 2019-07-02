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
use coins::{enable, electrum, get_enabled_coins, get_trade_fee, my_balance, send_raw_transaction, withdraw, my_tx_history};
use common::{err_to_rpc_json_string, free_c_ptr, lp, lp_queue_command_for_c,
        rpc_response, rpc_err_response, HyRes};
use common::wio::{CORE, HTTP};
use common::lift_body::LiftBody;
use common::mm_ctx::MmArc;
use futures::{self, Future};
use futures_cpupool::CpuPool;
use gstuff;
use http::{Request, Response, Method};
use http::header::{HeaderValue, ACCESS_CONTROL_ALLOW_ORIGIN};
use hyper;
use hyper::rt::Stream;
use hyper::service::Service;
use libc::{c_char, c_void};
use portfolio::lp_autoprice;
use portfolio::prices::{lp_fundvalue};
use serde_json::{self as json, Value as Json};
use std::ffi::{CStr, CString};
use std::net::{SocketAddr};
use std::ptr::null_mut;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use tokio_core::net::TcpListener;
use hex;

use crate::mm2::lp_ordermatch::{buy, cancel_all_orders, cancel_order, my_orders, order_status, sell, set_price};
use crate::mm2::lp_swap::{coins_needed_for_kick_start, my_swap_status, stats_swap_status, my_recent_swaps};
use crate::mm2::CJSON;

#[path = "rpc/lp_commands.rs"]
pub mod lp_commands;
use self::lp_commands::*;

#[path = "rpc/lp_signatures.rs"]
pub mod lp_signatures;

lazy_static! {
    /// Shared CPU pool to run intensive/sleeping requests on separate thread
    pub static ref CPUPOOL: CpuPool = CpuPool::new(8);
}

/// Lists the RPC method not requiring the "userpass" authentication.  
/// None is also public to skip auth and display proper error in case of method is missing
const PUBLIC_METHODS: &[Option<&str>] = &[  // Sorted alphanumerically (on the first letter) for readability.
    Some("balance"),
    Some("balances"),
    Some("fundvalue"),
    Some("getprice"),
    Some("getpeers"),
    Some("getcoins"),
    Some("help"),
    Some("notify"),  // Manually checks the peer's public key.
    Some("orderbook"),
    Some("passphrase"),  // Manually checks the "passphrase".
    Some("pricearray"),
    Some("psock"),
    Some("statsdisp"),
    Some("stats_swap_status"),
    Some("tradesarray"),
    Some("ticker"),
    None
];

#[allow(unused_macros)]
macro_rules! unwrap_or_err_response {
    ($e:expr, $($args:tt)*) => {
        match $e {
            Ok (ok) => ok,
            Err (err) => {return err_response (500, &ERRL! ("{}", err))}
        }
    }
}

struct RpcService {
    /// Allows us to get the `MmCtx` if it is still around.
    ctx_h: u32,
    /// The IP and port from whence the request is coming from.
    remote_addr: SocketAddr,
}

fn auth(json: &Json, ctx: &MmArc) -> Result<(), &'static str> {
    if !PUBLIC_METHODS.contains(&json["method"].as_str()) {
        if !json["userpass"].is_string() {
            return Err("Userpass is not set!");
        }

        let userpass = unsafe {unwrap! (CStr::from_ptr (lp::G.USERPASS.as_ptr()) .to_str())};
        let pass_hash = hex::encode(unsafe { lp::G.LP_passhash.bytes });

        if json["userpass"].as_str() != Some(userpass) && json["userpass"].as_str() != Some(&pass_hash) && json["userpass"] != ctx.conf["rpc_password"] {
            return Err("Userpass is invalid!");
        }
    }
    Ok(())
}

fn rpc_process_json(ctx: MmArc, remote_addr: SocketAddr, json: Json, c_json: CJSON) -> HyRes {
    // NB: `queueid` of `0` should be ignored, cf. https://github.com/atomiclabs/hyperdex/pull/563#issuecomment-434959074.
    let queueid = match json["queueid"] {
        Json::Number (ref n) => match n.as_u64() {
            Some (n) => n,
            None => return rpc_err_response (500, "The 'queueid' must be unsigned integer")
        },
        Json::Null => 0,
        _ => return rpc_err_response (500, "The 'queueid' must be unsigned integer")
    };

    if queueid > 0 {
        if unsafe { lp::IPC_ENDPOINT == -1 } {
            return rpc_err_response (500, "Can't queue the command when the WebSocket endpoint is disabled")
        } else if !remote_addr.ip().is_loopback() {
            return rpc_err_response (500, &format! ("IP {} is not local. Only the local HTTP clients can use the queue", remote_addr.ip()))
        } else {
            let json_str = json.to_string();
            let c_json_ptr = try_h! (CString::new (json_str));
            unsafe {
                lp_queue_command_for_c(null_mut(),
                                    c_json_ptr.as_ptr() as *mut c_char,
                                    lp::IPC_ENDPOINT,
                                    1,
                                    json["queueid"].as_u64().unwrap() as u32
                );
            }
            return rpc_response (200, r#"{"result": "success", "status": "queued"}"#)
        }
    }

    let rpc_ip_port = try_h! (ctx.rpc_ip_port());
    let my_ip_ptr = try_h! (CString::new (fomat! ((rpc_ip_port.ip()))));
    let remote_ip_ptr = try_h! (CString::new (fomat! ((remote_addr.ip()))));

    let stats_result = unsafe {
        lp::stats_JSON(
            ctx.btc_ctx() as *mut c_void,
            my_ip_ptr.as_ptr() as *mut c_char,
            lp::LP_mypubsock,
            c_json.0,
            remote_ip_ptr.as_ptr() as *mut c_char,
            rpc_ip_port.port(),
            1,
            ctx.conf["rpc_local_only"].as_bool().unwrap_or(true) as u8,
        )
    };

    if !stats_result.is_null() {
        let res_str = try_h! (unsafe {CStr::from_ptr(stats_result)} .to_str()) .to_string();
        free_c_ptr(stats_result as *mut c_void);

        // #220, See if `stats_JSON` returned an error and reflect this in the HTTP status.
        #[derive(Deserialize)] struct MaybeError<'a> {error: Option<&'a str>}
        let status = if let Ok (maybe_error) = json::from_str::<MaybeError> (&res_str) {
            if maybe_error.error.is_some() {500} else {200}
        } else {200};
        rpc_response (status, res_str)
    } else {
        rpc_err_response (500, "Empty result from stats_JSON")
    }
}

lazy_static! {
    /// Emulating the single-threaded execution for the older C code.
    pub static ref SINGLE_THREADED_C_LOCK: Mutex<()> = Mutex::new(());
}

/// Result of `fn dispatcher`.
pub enum DispatcherRes {
    /// `fn dispatcher` has found a Rust handler for the RPC "method".
    Match (HyRes),
    /// No handler found by `fn dispatcher`. Returning the `Json` request in order for it to be handled elsewhere.
    NoMatch (Json)
}

/// The dispatcher, with full control over the HTTP result and the way we run the `Future` producing it.
/// 
/// Invoked both directly from the HTTP endpoint handler below and in a delayed fashion from `lp_command_q_loop`.
/// 
/// Returns `None` if the requested "method" wasn't found among the ported RPC methods and has to be handled elsewhere.
pub fn dispatcher (req: Json, _remote_addr: Option<SocketAddr>, ctx: MmArc) -> DispatcherRes {
    //log! ("dispatcher] " (json::to_string (&req) .unwrap()));
    let method = match req["method"].clone() {
        Json::String (method) => method,
        _ => return DispatcherRes::NoMatch (req)
    };
    DispatcherRes::Match (match &method[..] {  // Sorted alphanumerically (on the first latter) for readability.
        "autoprice" => lp_autoprice (ctx, req),
        "buy" => buy (ctx, req),
        "cancel_all_orders" => cancel_all_orders (ctx, req),
        "cancel_order" => cancel_order (ctx, req),
        "coins_needed_for_kick_start" => coins_needed_for_kick_start(ctx),
        // TODO coin initialization performs blocking IO, i.e request.wait(), have to run it on CPUPOOL to avoid blocking shared CORE.
        //      at least until we refactor the functions like `utxo_coin_from_iguana_info` to async versions.
        "enable" => Box::new(CPUPOOL.spawn_fn(move || { enable (ctx, req) })),
        "electrum" => Box::new(CPUPOOL.spawn_fn(move || { electrum (ctx, req) })),
        "get_enabled_coins" => get_enabled_coins (ctx),
        "get_trade_fee" => get_trade_fee (ctx, req),
        "fundvalue" => lp_fundvalue (ctx, req, false),
        "help" => help(),
        "inventory" => inventory (ctx, req),
        "mpnet" => mpnet (&req),
        "my_orders" => my_orders (ctx),
        "my_balance" => my_balance (ctx, req),
        "my_tx_history" => my_tx_history(ctx, req),
        "notify" => lp_signatures::lp_notify_recv (ctx, req),  // Invoked usually from the `lp_command_q_loop`
        "order_status" => order_status (ctx, req),
        "passphrase" => passphrase (ctx, req),
        "sell" => sell (ctx, req),
        "send_raw_transaction" => send_raw_transaction (ctx, req),
        "setprice" => set_price (ctx, req),
        "stop" => stop (ctx),
        "my_recent_swaps" => my_recent_swaps(ctx, req),
        "my_swap_status" => my_swap_status(ctx, req),
        "stats_swap_status" => stats_swap_status(ctx, req),
        "version" => version(),
        "withdraw" => withdraw(ctx, req),
        _ => return DispatcherRes::NoMatch (req)
    })
}

type RpcRes = Box<dyn Future<Item=Response<LiftBody<Vec<u8>>>, Error=String> + Send>;

fn hyres_into_rpcres<HR> (hyres: HR) -> impl Future<Item=Response<LiftBody<Vec<u8>>>, Error=String> + Send
where HR: Future<Item=Response<Vec<u8>>, Error=String> + Send {
    hyres.map (move |r| {
        let (parts, body) = r.into_parts();
        Response::from_parts (parts, LiftBody::from (body))
    })
}

impl Service for RpcService {
    type ReqBody = hyper::Body;
    type ResBody = LiftBody<Vec<u8>>;
    type Error = String;
    type Future = RpcRes;

    fn call(&mut self, req: Request<hyper::Body>) -> Self::Future {
        macro_rules! try_sf {($value: expr) => {match $value {Ok (ok) => ok, Err (err) => {
            log! ({"RPC error response: {}", err});
            let rep = rpc_response (500, err_to_rpc_json_string (&ERRL! ("{}", err)));
            return Box::new (hyres_into_rpcres (rep))
        }}}}

        let ctx = try_sf! (MmArc::from_ffi_handle (self.ctx_h));

        // https://github.com/artemii235/SuperNET/issues/219
        let rpc_cors = match ctx.conf["rpccors"].as_str() {
            Some(s) => try_sf!(HeaderValue::from_str(s)),
            None => HeaderValue::from_static("http://localhost:3000"),
        };

        if req.method() != Method::POST {
            return Box::new (hyres_into_rpcres (rpc_err_response (400, "Only POST requests are supported!")))
        }

        let (_parts, body) = req.into_parts();
        let body_f = body.concat2();

        let remote_addr = self.remote_addr.clone();

        let f = body_f.then (move |chunk| -> HyRes {

            let vec = try_h! (chunk) .to_vec();
            let req: Json = try_h! (json::from_slice (&vec));

            let method = req["method"].as_str();
            // https://github.com/artemii235/SuperNET/issues/368
            let local_only = ctx.conf["rpc_local_only"].as_bool().unwrap_or(true);
            if local_only && !remote_addr.ip().is_loopback() && !PUBLIC_METHODS.contains (&method) {
                return rpc_err_response (400, &ERRL!("Selected method can be called from localhost only!"))
            }
            try_h! (auth (&req, &ctx));

            match dispatcher (req, Some (remote_addr), ctx.clone()) {
                DispatcherRes::Match (handler) => handler,
                DispatcherRes::NoMatch (req) => {
                    // Evoke the older C dispatcher (stats_JSON), RPC methods we haven't ported yet are handled there.
                    let c_json = try_h! (CJSON::from_str (&req.to_string()));
                    let cpu_pool_fut = CPUPOOL.spawn_fn(move || {
                        // Emulates the single-threaded execution of the old C code.
                        let _lock = SINGLE_THREADED_C_LOCK.lock();
                        rpc_process_json (ctx, remote_addr, req, c_json)
                    });
                    Box::new (cpu_pool_fut)
                }
            }
        });

        let f = hyres_into_rpcres (f);

        let f = f.map (|mut res| {
            res.headers_mut().insert(
                ACCESS_CONTROL_ALLOW_ORIGIN,
                rpc_cors
            );
            res
        });

        let f = f.then (|res| -> Self::Future {
            // even if future returns error we need to map it to JSON response and send to client
            let res = try_sf! (res);
            Box::new (futures::future::ok (res))
        });

        Box::new (f)
    }
}

pub extern fn spawn_rpc(ctx_h: u32) {
    // NB: We need to manually handle the incoming connections in order to get the remote IP address,
    // cf. https://github.com/hyperium/hyper/issues/1410#issuecomment-419510220.
    // Although if the ability to access the remote IP address is solved by the Hyper in the future
    // then we might want to refactor into starting it ideomatically in order to benefit from a more graceful shutdown,
    // cf. https://github.com/hyperium/hyper/pull/1640.

    let ctx = unwrap! (MmArc::from_ffi_handle (ctx_h), "No context");

    let rpc_ip_port = unwrap! (ctx.rpc_ip_port());
    let listener = unwrap! (TcpListener::bind2 (&rpc_ip_port), "Can't bind on {}", rpc_ip_port);

    let server = listener
        .incoming()
        .for_each(move |(socket, _my_sock)| {
            let remote_addr = match socket.peer_addr() {
                Ok (addr) => addr,
                Err (err) => {
                    log! ({"spawn_rpc] No peer_addr: {}", err});
                    return Ok(())
                }
            };

            CORE.spawn(move |_|
                HTTP.serve_connection(
                    socket,
                    RpcService {
                        ctx_h,
                        remote_addr
                    },
                )
                .map(|_| ())
                .map_err (|err| log! ({"spawn_rpc] HTTP error: {}", err}))
            );
            Ok(())
        })
        .map_err (|err| log! ({"spawn_rpc] accept error: {}", err}));

    // Finish the server `Future` when `shutdown_rx` fires.

    let (shutdown_tx, shutdown_rx) = futures::sync::oneshot::channel::<()>();
    let server = server.select2 (shutdown_rx) .then (|_| Ok(()));
    let mut shutdown_tx = Some (shutdown_tx);
    ctx.on_stop (Box::new (move || {
        if let Some (shutdown_tx) = shutdown_tx.take() {
            log! ("on_stop] firing shutdown_tx!");
            if let Err (_) = shutdown_tx.send(()) {log! ("on_stop] Warning, shutdown_tx already closed")}
            Ok(())
        } else {ERR! ("on_stop callback called twice!")}
    }));

    let rpc_ip_port = unwrap! (ctx.rpc_ip_port());
    CORE.spawn (move |_| {
        log!(">>>>>>>>>> DEX stats " (rpc_ip_port.ip())":"(rpc_ip_port.port()) " \
                DEX stats API enabled at unixtime." (gstuff::now_ms() / 1000) " <<<<<<<<<");
        ctx.rpc_started.store (true, Ordering::Relaxed);
        server
    });
}
