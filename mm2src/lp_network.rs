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
//  lp_network.rs
//  marketmaker
//
#![allow(uncommon_codepoints)]

use bitcrypto::ripemd160;
use bytes::Bytes;
use common::executor::{spawn, Timer};
#[cfg(not(feature = "native"))] use common::helperᶜ;
use common::mm_ctx::MmArc;
use common::{lp_queue_command, now_ms, HyRes, P2PMessage, QueuedCommand};
use crossbeam::channel;
use futures::channel::mpsc;
use futures::compat::Future01CompatExt;
use futures::future::FutureExt;
use futures::StreamExt;
use futures01::{future, Future};
use primitives::hash::H160;
use serde_bencode::de::from_bytes as bdecode;
use serde_json::{self as json, Value as Json};
use std::collections::hash_map::{Entry, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, TcpStream};
use std::thread;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::TcpListener as AsyncTcpListener;

use crate::mm2::lp_native_dex::lp_command_process;
use crate::mm2::lp_ordermatch::lp_post_price_recv;
use crate::mm2::lp_swap::save_stats_swap_status;
use crate::mm2::rpc::lp_signatures::lp_notify_recv;

/// Result of `fn dispatcher`.
pub enum DispatcherRes {
    /// `fn dispatcher` has found a Rust handler for the RPC "method".
    Match(HyRes),
    /// No handler found by `fn dispatcher`. Returning the `Json` request in order for it to be handled elsewhere.
    NoMatch(Json),
}

/// The network module dispatcher, handles the messages received from other nodes
fn dispatcher(req: Json, ctx: MmArc) -> DispatcherRes {
    // AP: the HTTP RPC server dispatcher was previously used for this purpose which IMHO
    // breaks single responsibility principe, makes harder to maintain the codebase and possibly
    // adds security concerns. Also we might end with using different serialization formats (binary)
    // for P2P messages - JSON is excessive for such purpose while it's completely fine to use it for HTTP server.
    // See https://github.com/artemii235/SuperNET/issues/415 for more info
    // So this is a starting point of further refactoring
    //log! ("dispatcher] " (json::to_string (&req) .unwrap()));
    let method = match req["method"].clone() {
        Json::String(method) => method,
        _ => return DispatcherRes::NoMatch(req),
    };
    DispatcherRes::Match(match &method[..] {
        // Sorted alphanumerically (on the first latter) for readability.
        "notify" => lp_notify_recv(ctx, req), // Invoked usually from the `lp_command_q_loop`
        "postprice" => lp_post_price_recv(&ctx, req),
        _ => return DispatcherRes::NoMatch(req),
    })
}

#[derive(Serialize)]
struct CommandForNn {
    result: Json,
    #[serde(rename = "queueid")]
    queue_id: u32,
}

/// Sends a reply to the `cmd.response_sock` peer.
fn reply_to_peer(cmd: QueuedCommand, mut reply: Vec<u8>) -> Result<(), String> {
    if cmd.response_sock >= 0 {
        if cmd.queue_id != 0 {
            let result = try_s!(json::from_slice(&reply));
            let nn_command = CommandForNn {
                queue_id: cmd.queue_id,
                result,
            };

            reply = try_s!(json::to_vec(&nn_command))
        }

        // See also commits ce09bcd and 62f3cba: looks like we need the wired string to be zero-terminated.
        reply.push(0);
    }
    Ok(())
}

/// Run the RPC handler and send it's reply to a peer.
fn rpc_reply_to_peer(handler: HyRes, cmd: QueuedCommand) {
    let f = handler.then(move |r| -> Box<dyn Future<Item = (), Error = ()> + Send> {
        let res = match r {
            Ok(r) => r,
            Err(err) => {
                log!("rpc_reply_to_peer] handler error: "(err));
                return Box::new(future::err(()));
            },
        };
        let body = res.into_body();
        if let Err(err) = reply_to_peer(cmd, body) {
            log!("reply_to_peer error: "(err));
            return Box::new(future::err(()));
        }
        Box::new(future::ok(()))
    });
    spawn(f.compat().map(|_| ()))
}

/// The thread processing the peer-to-peer messaging bus.
pub async fn lp_command_q_loop(ctx: MmArc) {
    use futures::future::{select, Either};

    let mut command_queueʳ = unwrap!(unwrap!(ctx.command_queueʳ.lock()).take().ok_or("!command_queueʳ"));

    let mut processed_messages: HashMap<H160, u64> = HashMap::new();
    let mut stoppingᶠ = Box::pin(async {
        loop {
            if ctx.is_stopping() {
                return;
            };
            Timer::sleep(0.2).await
        }
    });
    loop {
        let nextᶠ = command_queueʳ.next();
        let rc = select(nextᶠ, stoppingᶠ).await;
        let cmd = match rc {
            Either::Left((Some(cmd), s)) => {
                stoppingᶠ = s;
                cmd
            },
            Either::Left((None, _s)) => break,
            Either::Right((_n, _s)) => break,
        };

        let now = now_ms();
        // clean up messages older than 60 seconds
        processed_messages = processed_messages
            .drain()
            .filter(|(_, timestamp)| timestamp + 60000 > now)
            .collect();

        let msg_hash = ripemd160(cmd.msg.content.as_bytes());
        match processed_messages.entry(msg_hash) {
            Entry::Vacant(e) => e.insert(now),
            Entry::Occupied(_) => continue, // skip the messages that we processed previously
        };

        let json: Json = match json::from_str(&cmd.msg.content) {
            Ok(j) => j,
            Err(_) => {
                if cmd.msg.content.len() > 1 {
                    log!("Invalid JSON " (cmd.msg.content) " from " (cmd.msg.from));
                }
                continue;
            },
        };
        let method = json["method"].as_str();
        if let Some(m) = method {
            if m == "swapstatus" {
                let handler = save_stats_swap_status(&ctx, json["data"].clone());
                rpc_reply_to_peer(handler, cmd);
                continue;
            }
        }

        // rebroadcast the message if we're seednode
        // swapstatus is excluded from rebroadcast as the message is big and other nodes might just not need it
        let i_am_seed = ctx.conf["i_am_seed"].as_bool().unwrap_or(false);
        if i_am_seed {
            ctx.broadcast_p2p_msg(cmd.msg.clone());
        }

        let json = match dispatcher(json, ctx.clone()) {
            DispatcherRes::Match(handler) => {
                rpc_reply_to_peer(handler, cmd);
                continue;
            },
            DispatcherRes::NoMatch(req) => req,
        };

        // Invokes `lp_trade_command`.
        lp_command_process(ctx.clone(), json);
    }
}

/// The loop processing seednode activity as message relayer/rebroadcaster
pub fn seednode_loop(ctx: MmArc, listener: std::net::TcpListener) {
    let fut = async move {
        let mut listener = AsyncTcpListener::from_std(listener).unwrap();
        let mut incoming = listener.incoming();
        while let Some(stream) = futures_util::StreamExt::next(&mut incoming).await {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    log!("Error " (e) " on connection accept");
                    continue;
                },
            };
            let peer_addr = match stream.peer_addr() {
                Ok(a) => a,
                Err(e) => {
                    log!("Could not get peer addr from stream " [stream] ", error " (e));
                    continue;
                },
            };
            ctx.log.log(
                "😀",
                &[&"incoming_connection", &peer_addr.to_string().as_str()],
                "New connection...",
            );
            let ctx_read = ctx.clone();
            let ctx_write = ctx.clone();
            let (tx, mut rx) = mpsc::unbounded();
            ctx.seednode_p2p_channel.lock().unwrap().push(tx);
            let (read, mut write) = stream.into_split();
            let read_loop = async move {
                let mut read = tokio::io::BufReader::new(read);
                let mut buffer = String::with_capacity(1024);
                loop {
                    match read.read_line(&mut buffer).await {
                        Ok(read) => {
                            if read > 0 && !buffer.is_empty() {
                                unwrap!(lp_queue_command(&ctx_read, P2PMessage {
                                    from: peer_addr,
                                    content: buffer.clone(),
                                }));
                                buffer.clear();
                            } else if read == 0 {
                                ctx_read.log.log(
                                    "😟",
                                    &[&"incoming_connection", &peer_addr.to_string().as_str()],
                                    "Reached EOF, dropping connection",
                                );
                                break;
                            }
                        },
                        Err(e) => {
                            ctx_read.log.log(
                                "😟",
                                &[&"incoming_connection", &peer_addr.to_string().as_str()],
                                &format!("Error {} reading from socket, dropping connection", e),
                            );
                            break;
                        },
                    }
                }
            };
            let write_loop = async move {
                while let Some(mut msg) = rx.next().await {
                    if msg.from != peer_addr {
                        if !msg.content.ends_with('\n') {
                            msg.content.push('\n');
                        }
                        match write.write_all(msg.content.as_bytes()).await {
                            Ok(_) => (),
                            Err(e) => {
                                ctx_write.log.log(
                                    "😟",
                                    &[&"incoming_connection", &peer_addr.to_string().as_str()],
                                    &format!("Error {} writing to socket, dropping connection", e),
                                );
                                break;
                            },
                        };
                    }
                }
            };
            tokio::spawn(async move {
                // selecting over the read and write parts processing loops in order to
                // drop both parts and close connection in case of errors
                futures::select! {
                    read = Box::pin(read_loop).fuse() => (),
                    write = Box::pin(write_loop).fuse() => (),
                };
            });
        }
    };
    // creating separate tokio 0.2 runtime as TcpListener requires it and doesn't work with
    // shared tokio 0.1 core
    let mut runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(fut);
}

#[cfg(feature = "native")]
pub async fn start_seednode_loop(ctx: &MmArc, myipaddr: IpAddr, mypubport: u16) -> Result<(), String> {
    log! ("i_am_seed at " (myipaddr) ":" (mypubport));
    let to_bind = std::net::SocketAddr::new(myipaddr, mypubport);
    let listener = try_s!(std::net::TcpListener::bind(to_bind));
    try_s!(thread::Builder::new().name("seednode_loop".into()).spawn({
        let ctx = ctx.clone();
        move || seednode_loop(ctx, listener)
    }));
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
struct StartSeednodeLoopArgs {
    ctx: u32,
    myipaddr: String,
    mypubport: u16,
}

#[cfg(not(feature = "native"))]
pub async fn start_seednode_loop(ctx: &MmArc, myipaddr: IpAddr, mypubport: u16) -> Result<(), String> {
    let args = StartSeednodeLoopArgs {
        ctx: try_s!(ctx.ffi_handle()),
        myipaddr: fomat!((myipaddr)),
        mypubport,
    };
    let args = try_s!(bencode(&args));
    try_s!(helperᶜ("start_seednode_loop", args).await);
    try_s!(start_queue_tap(ctx.clone()));
    Ok(())
}

#[cfg(feature = "native")]
pub async fn start_seednode_loopʰ(req: Bytes) -> Result<Vec<u8>, String> {
    let args: StartSeednodeLoopArgs = try_s!(bdecode(&req));
    let myipaddr: IpAddr = try_s!(args.myipaddr.parse());
    let ctx = try_s!(MmArc::from_ffi_handle(args.ctx));
    {
        let mut cq = try_s!(ctx.command_queueʰ.lock());
        if cq.is_none() {
            *cq = Some(Vec::new())
        }
    }
    try_s!(start_seednode_loop(&ctx, myipaddr, args.mypubport).await);
    Ok(Vec::new())
}

struct SeedConnection {
    stream: BufReader<TcpStream>,
    addr: String,
    buf: String,
    last_msg: u64,
}

#[cfg(feature = "native")]
pub async fn start_client_p2p_loop(ctx: MmArc, addrs: Vec<String>) -> Result<(), String> {
    try_s!(thread::Builder::new()
        .name("client_p2p_loop".into())
        .spawn({ move || client_p2p_loop(ctx, addrs) }));
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct StartClientP2pLoopArgs {
    ctx: u32,
    addrs: Vec<String>,
}

/// Ask helper to fetch us the messages broadcasted from the seed nodes.
#[derive(Serialize, Deserialize)]
struct ClientP2pLoopArgs {
    /// The instance running the `client_p2p_loop` for us.
    ctx: u32,
    /// The time ID of the last message we've seen.
    since: u64,
}

#[cfg(not(feature = "native"))]
pub async fn start_client_p2p_loop(ctx: MmArc, addrs: Vec<String>) -> Result<(), String> {
    use common::helperᶜ;

    let ctx_handle = try_s!(ctx.ffi_handle());
    let args = StartClientP2pLoopArgs { ctx: ctx_handle, addrs };
    let args = try_s!(bencode(&args));
    try_s!(helperᶜ("start_client_p2p_loop", args).await);
    try_s!(start_queue_tap(ctx.clone()));
    Ok(())
}

#[cfg(not(feature = "native"))]
fn start_queue_tap(ctx: MmArc) -> Result<(), String> {
    use common::helperᶜ;
    use futures::future::{select, Either};

    let ctx_handle = try_s!(ctx.ffi_handle());

    // Get messages from the helper's `client_p2p_loop` and `seednode_loop`.
    spawn(async move {
        let mut stoppingᶠ = Box::pin(async {
            loop {
                if ctx.is_stopping() {
                    return;
                };
                Timer::sleep(0.2).await
            }
        });
        let mut last_command = 0;
        loop {
            let args = ClientP2pLoopArgs {
                ctx: ctx_handle,
                since: last_command,
            };
            let args = unwrap!(bencode(&args));
            let pollᶠ = Box::pin(helperᶜ("p2p_tap", args));
            let rc = select(pollᶠ, stoppingᶠ).await;
            let res = match rc {
                Either::Left((res, s)) => {
                    stoppingᶠ = s;
                    res
                },
                Either::Right((_r, _s)) => break,
            };

            let res = match res {
                Ok(res) => res,
                Err(err) => {
                    log!("Error invoking the client_p2p_loop helper: "(err));
                    Timer::sleep(2.2).await;
                    continue;
                },
            };
            let commands: Vec<(u64, String)> = unwrap!(bdecode(&res));
            for (ms, msg) in commands {
                //log! ("Received a broadcast command: " (msg));
                last_command = ms;
                unwrap!(lp_queue_command(&ctx, msg));
            }
        }
    });

    Ok(())
}

/*
/// Poll the native helpers for messages coming from the seed nodes.
#[cfg(feature = "native")]
pub async fn p2p_tapʰ(req: Bytes) -> Result<Vec<u8>, String> {
    let args: ClientP2pLoopArgs = try_s!(bdecode(&req));
    let ctx = try_s!(MmArc::from_ffi_handle(args.ctx));

    let start = now_float();

    let broadcasts = loop {
        let tail: Vec<(u64, String)> = {
            let mut cqˡ = try_s!(ctx.command_queueʰ.lock());
            let cq = match &mut *cqˡ {
                Some(ref mut cq) => cq,
                None => return ERR!("!command_queueʰ"),
            };
            let tail = cq.iter().filter(|(tid, _)| *tid > args.since).cloned().collect();
            // The `since` entry itself is *not* removed in order to always have a ground for monotonic increment.
            cq.retain(|(tid, _)| *tid >= args.since);
            tail
        };

        // Naive HTTP Long-polling: if there's nothing to share with the client then busy-wait for more content.
        if !tail.is_empty() || 11. < now_float() - start {
            break tail;
        }
        Timer::sleep(0.1).await
    };

    let res = try_s!(bencode(&broadcasts));
    Ok(res)
}

pub async fn broadcast_p2p_msgʰ(req: Bytes) -> Result<Vec<u8>, String> {
    let args: common::BroadcastP2pMessageArgs = try_s!(bdecode(&req));
    let ctx = try_s!(MmArc::from_ffi_handle(args.ctx));
    ctx.broadcast_p2p_msg(&args.msg);
    Ok(Vec::new())
}
*/
/// Tells the native helpers to start the client_p2p_loop, collecting messages from the seed nodes.
#[cfg(feature = "native")]
pub async fn start_client_p2p_loopʰ(req: Bytes) -> Result<Vec<u8>, String> {
    let args: StartClientP2pLoopArgs = try_s!(bdecode(&req));
    let ctx = try_s!(MmArc::from_ffi_handle(args.ctx));
    {
        let mut cq = try_s!(ctx.command_queueʰ.lock());
        if cq.is_none() {
            *cq = Some(Vec::new())
        }
    }
    try_s!(start_client_p2p_loop(ctx, args.addrs).await);
    Ok(Vec::new())
}

/// The loop processing client node activity
#[cfg(feature = "native")]
fn client_p2p_loop(ctx: MmArc, addrs: Vec<String>) {
    let mut seed_connections: Vec<SeedConnection> = vec![];
    // ip and last connection attempt timestamp
    let mut addrs: Vec<(String, u64)> = addrs.into_iter().map(|addr| (addr, 0)).collect();

    loop {
        if ctx.is_stopping() {
            break;
        }

        if seed_connections.len() < addrs.len() {
            for (addr, last_attempt) in addrs.iter_mut() {
                let is_connected = seed_connections.iter().find(|conn| &conn.addr == addr);
                if is_connected.is_none() && *last_attempt + 30000 < now_ms() {
                    ctx.log.log("…", &[&"seed_connection", &addr.as_str()], "Connecting…");
                    *last_attempt = now_ms();
                    match TcpStream::connect(&*addr) {
                        Ok(stream) => match stream.set_nonblocking(true) {
                            Ok(_) => {
                                let conn = SeedConnection {
                                    stream: BufReader::new(stream),
                                    addr: (*addr).to_string(),
                                    buf: String::new(),
                                    last_msg: now_ms(),
                                };
                                ctx.log.log("⚡", &[&"seed_connection", &addr.as_str()], "Connected");
                                seed_connections.push(conn);
                            },
                            Err(e) => ctx.log.log(
                                "😟",
                                &[&"seed_connection", &addr.as_str()],
                                &format!("Error {} setting non-blocking mode", e),
                            ),
                        },
                        Err(e) => ctx.log.log(
                            "😟",
                            &[&"seed_connection", &addr.as_str()],
                            &format!("Connection error {}", e),
                        ),
                    }
                }
            }
        }

        let mut commands = Vec::new();
        seed_connections = seed_connections
            .drain_filter(|conn| match conn.stream.read_line(&mut conn.buf) {
                Ok(_) => {
                    if !conn.buf.is_empty() {
                        let msgs = conn.buf.split('\n');
                        for msg in msgs {
                            if msg.len() > 1 {
                                commands.push(P2PMessage::from_string_with_default_addr(msg.to_owned()));
                            }
                        }
                        conn.buf.clear();
                        conn.last_msg = now_ms();
                    }
                    true
                },
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => true,
                Err(e) => {
                    ctx.log.log(
                        "😟",
                        &[&"seed_connection", &conn.addr.clone().as_str()],
                        &format!("Error {} on reading from socket, dropping connection", e),
                    );
                    false
                },
            })
            .collect();
        for msg in commands {
            unwrap!(lp_queue_command(&ctx, msg));
        }

        seed_connections = match ctx.client_p2p_channel.1.recv_timeout(Duration::from_millis(1)) {
            Ok(mut msg) => seed_connections
                .drain_filter(|conn| {
                    msg.push(b'\n');
                    match conn.stream.get_mut().write(&msg) {
                        Ok(_) => true,
                        Err(e) => {
                            ctx.log.log(
                                "😟",
                                &[&"seed_connection", &conn.addr.clone().as_str()],
                                &format!("Error {} writing to socket, dropping connection", e),
                            );
                            false
                        },
                    }
                })
                .collect(),
            Err(channel::RecvTimeoutError::Timeout) => seed_connections,
            Err(channel::RecvTimeoutError::Disconnected) => panic!("client_p2p_channel is disconnected"),
        };
        seed_connections = seed_connections
            .drain_filter(|conn| {
                if conn.last_msg + 30000 < now_ms() {
                    ctx.log.log(
                        "😟",
                        &[&"seed_connection", &conn.addr.clone().as_str()],
                        "Didn't receive any data in 30 seconds, dropping connection",
                    );
                    false
                } else {
                    true
                }
            })
            .collect();
    }
}
