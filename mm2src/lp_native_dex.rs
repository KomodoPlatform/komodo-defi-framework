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
//  lp_native_dex.rs
//  marketmaker
//
#![cfg_attr(not(feature = "native"), allow(dead_code))]
#![cfg_attr(not(feature = "native"), allow(unused_imports))]
#![cfg_attr(not(feature = "native"), allow(unused_variables))]

use coins::register_balance_update_handler;
use mm2_libp2p::start_gossipsub;
use rand::rngs::SmallRng;
use rand::{random, Rng, SeedableRng};
use serde_json::{self as json};
use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr};
use std::os::raw::c_char;
use std::path::Path;
use std::str;
use std::str::from_utf8;

use crate::common::executor::{spawn, spawn_boxed, Timer};
#[cfg(feature = "native")] use crate::common::lp;
use crate::common::mm_ctx::{MmArc, MmCtx};
use crate::common::privkey::key_pair_from_seed;
use crate::common::{slurp_url, MM_DATETIME, MM_VERSION};
#[cfg(not(feature = "wallet-only"))]
use crate::mm2::lp_network::{p2p_event_process_loop, P2PContext};
#[cfg(not(feature = "wallet-only"))]
use crate::mm2::lp_ordermatch::{broadcast_maker_orders_keep_alive_loop, lp_ordermatch_loop, orders_kick_start,
                                BalanceUpdateOrdermatchHandler};
#[cfg(not(feature = "wallet-only"))]
use crate::mm2::lp_swap::{running_swaps_num, swap_kick_starts};
use crate::mm2::rpc::spawn_rpc;

pub fn lp_ports(netid: u16) -> Result<(u16, u16, u16), String> {
    const LP_RPCPORT: u16 = 7783;
    let max_netid = (65535 - 40 - LP_RPCPORT) / 4;
    if netid > max_netid {
        return ERR!("Netid {} is larger than max {}", netid, max_netid);
    }

    let other_ports = if netid != 0 {
        let net_mod = netid % 10;
        let net_div = netid / 10;
        (net_div * 40) + LP_RPCPORT + net_mod
    } else {
        LP_RPCPORT
    };
    Ok((other_ports + 10, other_ports + 20, other_ports + 30))
}

/// Invokes `OS_ensure_directory`,
/// then prints an error and returns `false` if the directory is not writable.
fn ensure_dir_is_writable(dir_path: &Path) -> bool {
    #[cfg(feature = "native")]
    unsafe {
        let c_dir_path = unwrap!(dir_path.to_str());
        let c_dir_path = unwrap!(CString::new(c_dir_path));
        lp::OS_ensure_directory(c_dir_path.as_ptr() as *mut c_char)
    };

    let r: [u8; 32] = random();
    let mut check: Vec<u8> = Vec::with_capacity(r.len());
    let fname = dir_path.join("checkval");
    let mut fp = match fs::File::create(&fname) {
        Ok(fp) => fp,
        Err(_) => {
            log! ({"FATAL ERROR cant create {:?}", fname});
            return false;
        },
    };
    if fp.write_all(&r).is_err() {
        log! ({"FATAL ERROR writing {:?}", fname});
        return false;
    }
    drop(fp);
    let mut fp = match fs::File::open(&fname) {
        Ok(fp) => fp,
        Err(_) => {
            log! ({"FATAL ERROR cant open {:?}", fname});
            return false;
        },
    };
    if fp.read_to_end(&mut check).is_err() || check.len() != r.len() {
        log! ({"FATAL ERROR reading {:?}", fname});
        return false;
    }
    if check != r {
        log! ({"FATAL ERROR error comparing {:?} {:?} vs {:?}", fname, r, check});
        return false;
    }
    true
}

fn ensure_file_is_writable(file_path: &Path) -> Result<(), String> {
    if fs::File::open(file_path).is_err() {
        // try to create file if opening fails
        if let Err(e) = fs::OpenOptions::new().write(true).create_new(true).open(file_path) {
            return ERR!("{} when trying to create the file {}", e, file_path.display());
        }
    } else {
        // try to open file in write append mode
        if let Err(e) = fs::OpenOptions::new().write(true).append(true).open(file_path) {
            return ERR!(
                "{} when trying to open the file {} in write mode",
                e,
                file_path.display()
            );
        }
    }
    Ok(())
}

#[cfg(feature = "native")]
fn fix_directories(ctx: &MmCtx) -> Result<(), String> {
    let dbdir = ctx.dbdir();
    try_s!(std::fs::create_dir_all(&dbdir));

    unsafe {
        let dbdir = ctx.dbdir();
        let dbdir = try_s!(dbdir.to_str().ok_or("Bad dbdir"));
        let dbdir = try_s!(CString::new(dbdir));
        lp::OS_ensure_directory(dbdir.as_ptr() as *mut c_char)
    };

    if !ensure_dir_is_writable(&dbdir.join("SWAPS")) {
        return ERR!("SWAPS db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("SWAPS").join("MY")) {
        return ERR!("SWAPS/MY db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("SWAPS").join("STATS")) {
        return ERR!("SWAPS/STATS db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("SWAPS").join("STATS").join("MAKER")) {
        return ERR!("SWAPS/STATS/MAKER db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("SWAPS").join("STATS").join("TAKER")) {
        return ERR!("SWAPS/STATS/TAKER db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("TRANSACTIONS")) {
        return ERR!("TRANSACTIONS db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("GTC")) {
        return ERR!("GTC db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("PRICES")) {
        return ERR!("PRICES db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("UNSPENTS")) {
        return ERR!("UNSPENTS db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("ORDERS")) {
        return ERR!("ORDERS db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("ORDERS").join("MY")) {
        return ERR!("ORDERS/MY db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("ORDERS").join("MY").join("MAKER")) {
        return ERR!("ORDERS/MY/MAKER db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("ORDERS").join("MY").join("TAKER")) {
        return ERR!("ORDERS/MY/TAKER db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("TX_CACHE")) {
        return ERR!("TX_CACHE db dir is not writable");
    }
    try_s!(ensure_file_is_writable(&dbdir.join("GTC").join("orders")));
    Ok(())
}

#[cfg(not(feature = "native"))]
fn fix_directories(ctx: &MmCtx) -> Result<(), String> {
    #[cfg_attr(feature = "w-bindgen", wasm_bindgen(raw_module = "../../../js/defined-in-js.js"))]
    extern "C" {
        pub fn host_ensure_dir_is_writable(ptr: *const c_char, len: i32) -> i32;
    }
    macro_rules! writeable_dir {
        ($path: expr) => {
            let path = $path;
            let path = try_s!(path.to_str().ok_or("Non-unicode path"));
            let rc = unsafe { host_ensure_dir_is_writable(path.as_ptr() as *const c_char, path.len() as i32) };
            if rc != 0 {
                return ERR!("Dir '{}' not writeable: {}", path, rc);
            }
        };
    }

    let dbdir = ctx.dbdir();
    writeable_dir!(dbdir.join("SWAPS").join("MY"));
    writeable_dir!(dbdir.join("SWAPS").join("STATS").join("MAKER"));
    writeable_dir!(dbdir.join("SWAPS").join("STATS").join("TAKER"));
    writeable_dir!(dbdir.join("ORDERS").join("MY").join("MAKER"));
    writeable_dir!(dbdir.join("ORDERS").join("MY").join("TAKER"));
    Ok(())
}

#[cfg(all(feature = "native", not(feature = "wallet-only")))]
fn migrate_db(ctx: &MmArc) -> Result<(), String> {
    let migration_num_path = ctx.dbdir().join(".migration");
    let mut current_migration = match std::fs::read(&migration_num_path) {
        Ok(bytes) => {
            let mut num_bytes = [0; 8];
            if bytes.len() == 8 {
                num_bytes.clone_from_slice(&bytes);
                u64::from_le_bytes(num_bytes)
            } else {
                0
            }
        },
        Err(_) => 0,
    };

    if current_migration < 1 {
        try_s!(migration_1(ctx));
        current_migration = 1;
    }
    try_s!(std::fs::write(&migration_num_path, &current_migration.to_le_bytes()));
    Ok(())
}

#[cfg(all(feature = "native", not(feature = "wallet-only")))]
fn migration_1(_ctx: &MmArc) -> Result<(), String> { Ok(()) }

/// Resets the context (most of which resides currently in `lp::G` but eventually would move into `MmCtx`).
/// Restarts the peer connections.
/// Reloads the coin keys.
///
/// Besides the `passphrase` it also allows changing the `seednode` at runtime.  
/// AG: While there might be value in changing `seednode` at runtime, I'm not sure if changing `gui` is actually necessary.
///
/// AG: If possible, I think we should avoid calling this function on a working MM, using it for initialization only,
///     in order to avoid the possibility of invalid state.
/// AP: Totally agree, moreover maybe we even `must` deny calling this on a working MM as it's being refactored
#[allow(unused_unsafe)]
pub unsafe fn lp_passphrase_init(ctx: &MmArc) -> Result<(), String> {
    let passphrase = ctx.conf["passphrase"].as_str();
    let passphrase = match passphrase {
        None | Some("") => return ERR!("jeezy says we cant use the nullstring as passphrase and I agree"),
        Some(s) => s.to_string(),
    };

    let key_pair = try_s!(key_pair_from_seed(&passphrase));
    let key_pair = try_s!(ctx.secp256k1_key_pair.pin(key_pair));
    try_s!(ctx.rmd160.pin(key_pair.public().address_hash()));
    Ok(())
}

/// Tries to serve on the given IP to check if it's available.  
/// We need this check because our external IP, particularly under NAT,
/// might be outside of the set of IPs we can open and run a server on.
///
/// Returns an error if the address did not work
/// (like when the `ip` does not belong to a connected interface).
///
/// The primary concern of this function is to test the IP,
/// but this opportunity is also used to start the HTTP fallback server,
/// in order to improve the reliability of the said server (in the Lean "stop the line" manner).
///
/// If the IP has passed the communication check then a shutdown Sender is returned.
/// Dropping or using that Sender will stop the HTTP fallback server.
///
/// Also the port of the HTTP fallback server is returned.
#[cfg(feature = "native")]
fn test_ip(ctx: &MmArc, ip: IpAddr) -> Result<(), String> {
    let netid = ctx.netid();

    // Try a few pseudo-random ports.
    // `netid` is used as the seed in order for the port selection to be determenistic,
    // similar to how the port selection and probing worked before (since MM1)
    // and in order to reduce the likehood of *unexpected* port conflicts.
    let mut attempts_left = 9;
    let mut rng = SmallRng::seed_from_u64(netid as u64);
    loop {
        if attempts_left < 1 {
            break ERR!("Out of attempts");
        }
        attempts_left -= 1;
        // TODO: Avoid `mypubport`.
        let port = rng.gen_range(1111, 65535);
        log! ("test_ip] Trying to bind on " (ip) ':' (port));
        match std::net::TcpListener::bind((ip, port)) {
            Ok(_) => break Ok(()),
            Err(err) => {
                if attempts_left == 0 {
                    break ERR!("{}", err);
                }
                continue;
            },
        }
    }
}

#[cfg(not(feature = "native"))]
fn test_ip(_ctx: &MmArc, _ip: IpAddr) -> Result<(Sender<()>, u16), String> {
    // Try to return a simple okay for tests.
    let (shutdown_tx, _shutdown_rx) = futures01::sync::oneshot::channel::<()>();
    Ok((shutdown_tx, 80))
}

/// * `ctx_cb` - callback used to share the `MmCtx` ID with the call site.
pub async fn lp_init(mypubport: u16, ctx: MmArc) -> Result<(), String> {
    log! ({"lp_init] version: {} DT {}", MM_VERSION, MM_DATETIME});
    unsafe { try_s!(lp_passphrase_init(&ctx)) }

    try_s!(fix_directories(&ctx));
    #[cfg(all(feature = "native", not(feature = "wallet-only")))]
    {
        try_s!(migrate_db(&ctx));
    }

    fn simple_ip_extractor(ip: &str) -> Result<IpAddr, String> {
        let ip = ip.trim();
        Ok(match ip.parse() {
            Ok(ip) => ip,
            Err(err) => return ERR!("Error parsing IP address '{}': {}", ip, err),
        })
    }

    let i_am_seed = ctx.conf["i_am_seed"].as_bool().unwrap_or(false);

    let myipaddr: IpAddr = if Path::new("myipaddr").exists() {
        match fs::File::open("myipaddr") {
            Ok(mut f) => {
                let mut buf = String::new();
                if let Err(err) = f.read_to_string(&mut buf) {
                    return ERR!("Can't read from 'myipaddr': {}", err);
                }
                try_s!(simple_ip_extractor(&buf))
            },
            Err(err) => return ERR!("Can't read from 'myipaddr': {}", err),
        }
    } else if !ctx.conf["myipaddr"].is_null() {
        let s = try_s!(ctx.conf["myipaddr"].as_str().ok_or("'myipaddr' is not a string"));
        try_s!(simple_ip_extractor(s))
    } else {
        // Detect the real IP address.
        //
        // We're detecting the outer IP address, visible to the internet.
        // Later we'll try to *bind* on this IP address,
        // and this will break under NAT or forwarding because the internal IP address will be different.
        // Which might be a good thing, allowing us to detect the likehoodness of NAT early.

        type Extractor = fn(&str) -> Result<IpAddr, String>;
        let ip_providers: [(&'static str, Extractor); 2] = [
            ("http://checkip.amazonaws.com/", simple_ip_extractor),
            ("http://api.ipify.org", simple_ip_extractor),
        ];

        let mut ip_providers_it = ip_providers.iter();
        loop {
            let (url, extactor) = match ip_providers_it.next() {
                Some(t) => t,
                None => return ERR!("Can't fetch the real IP"),
            };
            log! ({"lp_init] Trying to fetch the real IP from '{}' ...", url});
            let (status, _headers, ip) = match slurp_url(url).await {
                Ok(t) => t,
                Err(err) => {
                    log! ({"lp_init] Failed to fetch IP from '{}': {}", url, err});
                    continue;
                },
            };
            if !status.is_success() {
                log! ({"lp_init] Failed to fetch IP from '{}': status {:?}", url, status});
                continue;
            }
            let ip = match from_utf8(&ip) {
                Ok(ip) => ip,
                Err(err) => {
                    log! ({"lp_init] Failed to fetch IP from '{}', not UTF-8: {}", url, err});
                    continue;
                },
            };
            let ip = match extactor(ip) {
                Ok(ip) => ip,
                Err(err) => {
                    log! ({"lp_init] Failed to parse IP '{}' fetched from '{}': {}", ip, url, err});
                    continue;
                },
            };

            // Try to bind on this IP.
            // If we're not behind a NAT then the bind will likely suceed.
            // If the bind fails then emit a user-visible warning and fall back to 0.0.0.0.
            match test_ip(&ctx, ip) {
                Ok(_) => {
                    ctx.log.log(
                        "🙂",
                        &[&"myipaddr"],
                        &fomat! (
                        "We've detected an external IP " (ip) " and we can bind on it"
                        ", so probably a dedicated IP."),
                    );
                    break ip;
                },
                Err(err) => log! ("IP " (ip) " doesn't check: " (err)),
            }
            let all_interfaces = Ipv4Addr::new(0, 0, 0, 0).into();
            if test_ip(&ctx, all_interfaces).is_ok() {
                ctx.log.log ("😅", &[&"myipaddr"], &fomat! (
                    "We couldn't bind on the external IP " (ip) ", so NAT is likely to be present. We'll be okay though."));
                break all_interfaces;
            }
            let locahost = Ipv4Addr::new(127, 0, 0, 1).into();
            if test_ip(&ctx, locahost).is_ok() {
                ctx.log.log(
                    "🤫",
                    &[&"myipaddr"],
                    &fomat! (
                    "We couldn't bind on " (ip) " or 0.0.0.0!"
                    " Looks like we can bind on 127.0.0.1 as a workaround, but that's not how we're supposed to work."),
                );
                break locahost;
            }
            ctx.log.log(
                "🤒",
                &[&"myipaddr"],
                &fomat! (
                "Couldn't bind on " (ip) ", 0.0.0.0 or 127.0.0.1."),
            );
            break all_interfaces; // Seems like a better default than 127.0.0.1, might still work for other ports.
        }
    };

    #[cfg(not(feature = "native"))]
    try_s!(ctx.send_to_helpers().await);
    let seednodes: Option<Vec<String>> = try_s!(json::from_value(ctx.conf["seednodes"].clone()));

    let key_pair = ctx.secp256k1_key_pair.as_option().unwrap();

    let ctx_on_poll = ctx.clone();
    let (cmd_tx, event_rx, peer_id) = start_gossipsub(
        myipaddr,
        mypubport,
        spawn_boxed,
        seednodes,
        &mut *key_pair.private().secret.clone(),
        i_am_seed,
        move |swarm| {
            mm_gauge!(
                ctx_on_poll.metrics,
                "p2p.connected_relays.len",
                swarm.connected_relays_len() as i64
            );
            mm_gauge!(ctx_on_poll.metrics, "p2p.relay_mesh.len", swarm.relay_mesh_len() as i64);
        },
    );
    try_s!(ctx.peer_id.pin(peer_id.to_string()));
    let p2p_context = P2PContext::new(cmd_tx);
    p2p_context.store_to_mm_arc(&ctx);
    spawn(p2p_event_process_loop(ctx.clone(), event_rx, i_am_seed));
    /*
    if i_am_seed {
        log!("Before relay node");
        let (tx, peer_id) = relay_node(ctx.clone(), myipaddr, mypubport, seednodes.clone());
        log!("After relay node");
        try_s!(ctx.gossip_sub_cmd_queue.pin(tx));
        try_s!(ctx.peer_id.pin(peer_id));
    }
    */

    // try_s!(lp_initpeers(&ctx, netid, seednodes).await);
    let balance_update_ordermatch_handler = BalanceUpdateOrdermatchHandler::new(ctx.clone());
    register_balance_update_handler(ctx.clone(), Box::new(balance_update_ordermatch_handler)).await;

    try_s!(ctx.initialized.pin(true));

    #[cfg(all(feature = "native", not(feature = "wallet-only")))]
    {
        // launch kickstart threads before RPC is available, this will prevent the API user to place
        // an order and start new swap that might get started 2 times because of kick-start
        let mut coins_needed_for_kick_start = swap_kick_starts(ctx.clone());
        coins_needed_for_kick_start.extend(try_s!(orders_kick_start(&ctx).await));
        *(try_s!(ctx.coins_needed_for_kick_start.lock())) = coins_needed_for_kick_start;
    }

    let ctxʹ = ctx.clone();
    spawn(lp_ordermatch_loop(ctxʹ));

    let ctxʹ = ctx.clone();
    spawn(broadcast_maker_orders_keep_alive_loop(ctxʹ));

    #[cfg(not(feature = "native"))]
    {
        if 1 == 1 {
            return Ok(());
        }
    } // TODO: Gradually move this point further down.

    let ctx_id = try_s!(ctx.ffi_handle());

    spawn_rpc(ctx_id);

    // In the mobile version we might depend on `lp_init` staying around until the context stops.
    loop {
        if ctx.is_stopping() {
            break;
        };
        Timer::sleep(0.2).await
    }

    // wait for swaps to stop
    #[cfg(not(feature = "wallet-only"))]
    loop {
        if running_swaps_num(&ctx) == 0 {
            break;
        };
        Timer::sleep(0.2).await
    }
    Ok(())
}
