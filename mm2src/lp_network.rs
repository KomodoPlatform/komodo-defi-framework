
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

use bitcrypto::ripemd160;
use common::{free_c_ptr, HyRes, CJSON, CORE, QueuedCommand, COMMAND_QUEUE, lp_queue_command};
use common::mm_ctx::MmArc;
use crossbeam::channel;
use futures::{future, Future, Stream};
use hashbrown::hash_map::{HashMap, Entry};
use libc::{c_void};
use primitives::hash::H160;
use serde_json::{self as json, Value as Json};
use std::ffi::{CStr};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use crate::mm2::lp_native_dex::lp_command_process;
use crate::mm2::lp_swap::save_stats_swap_status;
use crate::mm2::rpc::{dispatcher, DispatcherRes};
use gstuff::now_ms;

pub fn nanomsg_transportname (bindflag: i32, ipaddr: &fmt::Display, port: u16) -> String {
    fomat! ("tcp://" if bindflag == 0 {(ipaddr)} else {"*"} ':' (port))
}

#[derive(Serialize)]
struct CommandForNn {
    result: Json,
    #[serde(rename="queueid")]
    queue_id: u32
}

/// Sends a reply to the `cmd.response_sock` peer.
fn reply_to_peer (cmd: QueuedCommand, mut reply: Vec<u8>) -> Result<(), String> {
    if cmd.response_sock >= 0 {
        if cmd.queue_id != 0 {
            let result = try_s! (json::from_slice (&reply));
            let nn_command = CommandForNn {
                queue_id: cmd.queue_id,
                result
            };

            reply = try_s! (json::to_vec (&nn_command))
        }

        // See also commits ce09bcd and 62f3cba: looks like we need the wired string to be zero-terminated.
        reply.push (0);
    }
    Ok(())
}

/// Run the RPC handler and send it's reply to a peer.
fn rpc_reply_to_peer (handler: HyRes, cmd: QueuedCommand) {
    let f = handler.then (move |r| -> Box<Future<Item=(), Error=()> + Send> {
        let res = match r {Ok (r) => r, Err (err) => {
            log! ("rpc_reply_to_peer] handler error: " (err));
            return Box::new (future::err(()))
        }};
        let body_f = res.into_body().concat2();
        Box::new (body_f.then (move |body| -> Result<(), ()> {
            let body = match body {Ok (r) => r, Err (err) => {
                log! ("rpc_reply_to_peer] error getting the body from the RPC handler: " (err));
                return Err(())
            }};
            if let Err (err) = reply_to_peer (cmd, body.to_vec()) {
                log! ("reply_to_peer error: " (err));
                return Err(())
            }
            Ok(())
        }))
    });
    CORE.spawn (|_| f)
}

/// The thread processing the peer-to-peer messaging bus.
pub unsafe fn lp_command_q_loop(ctx: MmArc) {
    let mut processed_messages: HashMap<H160, u64> = HashMap::new();
    loop {
        if ctx.is_stopping() { break }

        let cmd = match (*COMMAND_QUEUE).1.recv_timeout(Duration::from_millis(100)) {
            Ok(cmd) => cmd,
            Err(channel::RecvTimeoutError::Timeout) => continue,  // And check `is_stopping`.
            Err(channel::RecvTimeoutError::Disconnected) => break
        };

        let now = now_ms();
        // clean up messages older than 60 seconds
        processed_messages = processed_messages.drain().filter(|(_, timestamp)| timestamp + 60000 > now).collect();

        let msg_hash = ripemd160(cmd.msg.as_bytes());
        match processed_messages.entry(msg_hash) {
            Entry::Vacant(e) => e.insert(now),
            Entry::Occupied(_) => continue, // skip the messages that we processed previously
        };

        let json: Json = match json::from_str(&cmd.msg) {
            Ok(j) => j,
            Err(e) => {
                log!("Error " (e) " parsing JSON from msg " (cmd.msg));
                continue;
            }
        };

        let method = json["method"].as_str();
        if let Some(m) = method {
            if m == "swapstatus" {
                let handler = save_stats_swap_status(json["data"].clone());
                rpc_reply_to_peer(handler, cmd);
                continue;
            }
        }

        // rebroadcast the message if we're seednode
        // swapstatus is excluded from rebroadcast as the message is big and other nodes might just not need it
        let i_am_seed = ctx.conf["i_am_seed"].as_bool().unwrap_or(false);
        if i_am_seed {
            ctx.broadcast_p2p_msg(&cmd.msg);
        }

        let json = match dispatcher(json, None, ctx.clone()) {
            DispatcherRes::Match(handler) => {
                rpc_reply_to_peer(handler, cmd);
                continue
            },
            DispatcherRes::NoMatch(req) => req
        };

        // Invokes `lp_trade_command` and the older `stats_JSON` code.
        let c_json = unwrap!(CJSON::from_str(&cmd.msg));
        let retstr = lp_command_process(
            ctx.clone(),
            cmd.response_sock,
            json,
            c_json,
            cmd.stats_json_only,
        );

        if !retstr.is_null() {
            let retvec = CStr::from_ptr(retstr).to_bytes().to_vec();
            free_c_ptr(retstr as *mut c_void);

            if let Err(err) = reply_to_peer(cmd, retvec) {
                log!("reply_to_peer error: "(err))
            }
        }
    }
}

/// The loop processing seednode activity as message relayer/rebroadcaster
/// Non-blocking mode should be enabled on listener for this to work
pub fn seednode_loop(ctx: MmArc, listener: TcpListener) {
    let mut clients = vec![];
    loop {
        if ctx.is_stopping() { break }

        match listener.accept() {
            Ok((stream, addr)) => {
                ctx.log.log("😀", &[&"incoming_connection", &addr.to_string().as_str()], "New connection...");
                match stream.set_nonblocking(true) {
                    Ok(_) => clients.push((BufReader::new(stream), addr, String::new())),
                    Err(e) => ctx.log.log("😟", &[&"incoming_connection", &addr.to_string().as_str()], &format!("Error {} setting nonblocking mode", e)),
                }
            },
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => (),
            Err(e) => panic!("encountered IO error: {}", e),
        }

        clients = clients.drain_filter(|(client, addr, buf)| {
            match client.read_line(buf) {
                Ok(_) => {
                    if buf.len() > 0 {
                        let msgs: Vec<_> = buf.split('\n').collect();
                        for msg in msgs {
                            if !msg.is_empty() {
                                lp_queue_command(msg.to_string());
                            }
                        }
                        buf.clear();
                    }
                    true
                },
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => true,
                Err(e) => {
                    ctx.log.log("😟", &[&"incoming_connection", &addr.to_string().as_str()], &format!("Error {} reading from socket, dropping connection", e));
                    false
                },
            }
        }).collect();

        clients = match ctx.seednode_p2p_channel.1.recv_timeout(Duration::from_millis(1)) {
            Ok(mut msg) => clients.drain_filter(|(client, addr, _)| {
                msg.push('\n' as u8);
                match client.get_mut().write(&msg) {
                    Ok(_) => true,
                    Err(e) => {
                        ctx.log.log("😟", &[&"incoming_connection", &addr.to_string().as_str()], &format!("Error {} writing to socket, dropping connection", e));
                        false
                    }
                }
            }).collect(),
            Err(channel::RecvTimeoutError::Timeout) => clients,
            Err(channel::RecvTimeoutError::Disconnected) => panic!("seednode_p2p_channel is disconnected"),
        };
    }
}

struct SeedConnection {
    stream: BufReader<TcpStream>,
    addr: String,
    buf: String,
}

/// The loop processing client node activity
pub fn client_p2p_loop(ctx: MmArc, addrs: Vec<String>) {
    let mut seed_connections: Vec<SeedConnection> = vec![];
    // ip and last connection attempt timestamp
    let mut addrs: Vec<(String, u64)> = addrs.into_iter().map(|addr| (addr, 0)).collect();

    loop {
        if ctx.is_stopping() { break }

        if seed_connections.len() < addrs.len() {
            for (addr, last_attempt) in addrs.iter_mut() {
                let is_connected = seed_connections.iter().find(|conn| &conn.addr == addr);
                if is_connected.is_none() && *last_attempt + 30000 < now_ms() {
                    ctx.log.log("😀", &[&"seed_connection", &addr.as_str()], "Connecting...");
                    *last_attempt = now_ms();
                    match TcpStream::connect(&*addr) {
                        Ok(stream) => {
                            match stream.set_nonblocking(true) {
                                Ok(_) => {
                                    let conn = SeedConnection {
                                        stream: BufReader::new(stream),
                                        addr: addr.to_string(),
                                        buf: String::new(),
                                    };
                                    ctx.log.log("😀", &[&"seed_connection", &addr.as_str()], "Connected...");
                                    seed_connections.push(conn);
                                },
                                Err(e) => ctx.log.log("😟", &[&"seed_connection", &addr.as_str()], &format!("Error {} setting non-blocking mode", e)),
                            }
                        },
                        Err(e) => ctx.log.log("😟", &[&"seed_connection", &addr.as_str()], &format!("Connection error {}", e)),
                    }
                }
            }
        }

        seed_connections = seed_connections.drain_filter(|conn| {
            match conn.stream.read_line(&mut conn.buf) {
                Ok(_) => {
                    if conn.buf.len() > 0 {
                        let msgs: Vec<_> = conn.buf.split('\n').collect();
                        for msg in msgs {
                            if !msg.is_empty() {
                                lp_queue_command(msg.to_string());
                            }
                        }
                        conn.buf.clear();
                    }
                    true
                },
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => true,
                Err(e) => {
                    ctx.log.log("😟", &[&"seed_connection", &conn.addr.clone().as_str()], &format!("Error {} on reading from socket, dropping connection", e));
                    false
                },
            }
        }).collect();

        seed_connections = match ctx.client_p2p_channel.1.recv_timeout(Duration::from_millis(1)) {
            Ok(mut msg) => seed_connections.drain_filter(|conn| {
                msg.push('\n' as u8);
                match conn.stream.get_mut().write(&msg) {
                    Ok(_) => true,
                    Err(e) => {
                        ctx.log.log("😟", &[&"seed_connection", &conn.addr.clone().as_str()], &format!("Error {} writing to socket, dropping connection", e));
                        false
                    }
                }
            }).collect(),
            Err(channel::RecvTimeoutError::Timeout) => seed_connections,
            Err(channel::RecvTimeoutError::Disconnected) => panic!("client_p2p_channel is disconnected"),
        };
    }
}
