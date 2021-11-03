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

use coins::register_balance_update_handler;
use derive_more::Display;
use mm2_libp2p::{spawn_gossipsub, NodeType, RelayAddress, WssCerts};
use rand::random;
use serde_json::{self as json};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::str;

#[cfg(not(target_arch = "wasm32"))]
use crate::mm2::database::init_and_migrate_db;
use crate::mm2::lp_message_service::init_message_service;
use crate::mm2::lp_network::{lp_network_ports, p2p_event_process_loop, P2PContext};
use crate::mm2::lp_ordermatch::{broadcast_maker_orders_keep_alive_loop, clean_memory_loop, init_ordermatch_context,
                                lp_ordermatch_loop, orders_kick_start, BalanceUpdateOrdermatchHandler};
use crate::mm2::lp_swap::{running_swaps_num, swap_kick_starts};
use crate::mm2::rpc::spawn_rpc;
use crate::mm2::{MM_DATETIME, MM_VERSION};
use bitcrypto::sha256;
use common::executor::{spawn, spawn_boxed, Timer};
#[cfg(not(target_arch = "wasm32"))]
use common::ip_addr::myipaddr;
use common::log::{error, info, warn};
use common::mm_ctx::{MmArc, MmCtx};
use common::mm_error::prelude::*;
use common::privkey::key_pair_from_seed;

const NETID_7777_SEEDNODES: [&str; 3] = ["seed1.defimania.live", "seed2.defimania.live", "seed3.defimania.live"];

/// TODO Extend `P2PError` and use `P2PResult` as a result of the `init_p2p` function.
pub type P2PResult<T> = Result<T, MmError<P2PError>>;

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Debug, Display)]
pub enum P2PError {
    #[display(
        fmt = "Invalid WSS key/cert at {:?}. The file must contain {}'",
        path,
        expected_format
    )]
    InvalidWssCert { path: PathBuf, expected_format: String },
    #[display(fmt = "Error deserializing '{}' config field: {}", field, error)]
    ErrorDeserializingConfig { field: String, error: json::Error },
    #[display(fmt = "Error reading WSS key/cert file {:?}: {}", path, error)]
    ErrorReadingCertFile { path: PathBuf, error: io::Error },
}

#[cfg(target_arch = "wasm32")]
fn default_seednodes(netid: u16) -> Vec<RelayAddress> {
    if netid == 7777 {
        NETID_7777_SEEDNODES
            .iter()
            .map(|seed| RelayAddress::Dns(seed.to_string()))
            .collect()
    } else {
        Vec::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn default_seednodes(netid: u16) -> Vec<RelayAddress> {
    use crate::mm2::lp_network::addr_to_ipv4_string;
    if netid == 7777 {
        NETID_7777_SEEDNODES
            .iter()
            .filter_map(|seed| addr_to_ipv4_string(*seed).ok())
            .map(RelayAddress::IPv4)
            .collect()
    } else {
        Vec::new()
    }
}

/// Invokes `OS_ensure_directory`,
/// then prints an error and returns `false` if the directory is not writable.
fn ensure_dir_is_writable(dir_path: &Path) -> bool {
    if dir_path.exists() && !dir_path.is_dir() {
        error!("The {} is not a directory", dir_path.display());
        return false;
    } else if let Err(e) = std::fs::create_dir_all(dir_path) {
        error!("Could not create dir {}, error {}", dir_path.display(), e);
        return false;
    }
    let r: [u8; 32] = random();
    let mut check: Vec<u8> = Vec::with_capacity(r.len());
    let fname = dir_path.join("checkval");
    let mut fp = match fs::File::create(&fname) {
        Ok(fp) => fp,
        Err(_) => {
            error!("FATAL cannot create {:?}", fname);
            return false;
        },
    };
    if fp.write_all(&r).is_err() {
        error!("FATAL cannot write to {:?}", fname);
        return false;
    }
    drop(fp);
    let mut fp = match fs::File::open(&fname) {
        Ok(fp) => fp,
        Err(_) => {
            error!("FATAL cannot open {:?}", fname);
            return false;
        },
    };
    if fp.read_to_end(&mut check).is_err() || check.len() != r.len() {
        error!("FATAL cannot read {:?}", fname);
        return false;
    }
    if check != r {
        error!("FATAL expect the same {:?} data: {:?} != {:?}", fname, r, check);
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

#[cfg(not(target_arch = "wasm32"))]
fn fix_directories(ctx: &MmCtx) -> Result<(), String> {
    let dbdir = ctx.dbdir();
    try_s!(std::fs::create_dir_all(&dbdir));

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
    if !ensure_dir_is_writable(&dbdir.join("ORDERS").join("MY").join("HISTORY")) {
        return ERR!("ORDERS/MY/HISTORY db dir is not writable");
    }
    if !ensure_dir_is_writable(&dbdir.join("TX_CACHE")) {
        return ERR!("TX_CACHE db dir is not writable");
    }
    try_s!(ensure_file_is_writable(&dbdir.join("GTC").join("orders")));
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
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
        migration_1(ctx);
        current_migration = 1;
    }
    try_s!(std::fs::write(&migration_num_path, &current_migration.to_le_bytes()));
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn migration_1(_ctx: &MmArc) {}

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
pub fn lp_passphrase_init(ctx: &MmArc) -> Result<(), String> {
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

#[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
/// * `ctx_cb` - callback used to share the `MmCtx` ID with the call site.
pub async fn lp_init(ctx: MmArc) -> Result<(), String> {
    info!("Version: {} DT {}", MM_VERSION, MM_DATETIME);
    try_s!(lp_passphrase_init(&ctx));

    #[cfg(not(target_arch = "wasm32"))]
    {
        try_s!(fix_directories(&ctx));
        try_s!(ctx.init_sqlite_connection());
        try_s!(init_and_migrate_db(&ctx, &ctx.sqlite_connection()).await);
        try_s!(migrate_db(&ctx));
    }

    try_s!(init_ordermatch_context(&ctx));
    try_s!(init_message_service(&ctx).await);
    try_s!(init_p2p(ctx.clone()).await);

    let balance_update_ordermatch_handler = BalanceUpdateOrdermatchHandler::new(ctx.clone());
    register_balance_update_handler(ctx.clone(), Box::new(balance_update_ordermatch_handler)).await;

    try_s!(ctx.initialized.pin(true));

    // launch kickstart threads before RPC is available, this will prevent the API user to place
    // an order and start new swap that might get started 2 times because of kick-start
    try_s!(kick_start(ctx.clone()).await);

    spawn(lp_ordermatch_loop(ctx.clone()));

    spawn(broadcast_maker_orders_keep_alive_loop(ctx.clone()));

    spawn(clean_memory_loop(ctx.clone()));

    let ctx_id = try_s!(ctx.ffi_handle());

    spawn_rpc(ctx_id);
    let ctx_c = ctx.clone();
    spawn(async move {
        if let Err(err) = ctx_c.init_metrics() {
            warn!("Couldn't initialize metrics system: {}", err);
        }
    });
    // In the mobile version we might depend on `lp_init` staying around until the context stops.
    loop {
        if ctx.is_stopping() {
            break;
        };
        Timer::sleep(0.2).await
    }

    // wait for swaps to stop
    loop {
        if running_swaps_num(&ctx) == 0 {
            break;
        };
        Timer::sleep(0.2).await
    }
    Ok(())
}

async fn kick_start(ctx: MmArc) -> Result<(), String> {
    let mut coins_needed_for_kick_start = try_s!(swap_kick_starts(ctx.clone()).await);
    coins_needed_for_kick_start.extend(try_s!(orders_kick_start(&ctx).await));
    *(try_s!(ctx.coins_needed_for_kick_start.lock())) = coins_needed_for_kick_start;
    Ok(())
}

async fn init_p2p(ctx: MmArc) -> Result<(), String> {
    let i_am_seed = ctx.conf["i_am_seed"].as_bool().unwrap_or(false);
    let netid = ctx.netid();

    let seednodes = try_s!(seednodes(&ctx));

    let ctx_on_poll = ctx.clone();
    let force_p2p_key = if i_am_seed {
        let key = sha256(&*ctx.secp256k1_key_pair().private().secret);
        Some(key.take())
    } else {
        None
    };

    let node_type = if i_am_seed {
        try_s!(relay_node_type(&ctx).await)
    } else {
        try_s!(light_node_type(&ctx))
    };

    let spawn_result = spawn_gossipsub(netid, force_p2p_key, spawn_boxed, seednodes, node_type, move |swarm| {
        let behaviour = swarm.behaviour();
        mm_gauge!(
            ctx_on_poll.metrics,
            "p2p.connected_relays.len",
            behaviour.connected_relays_len() as i64
        );
        mm_gauge!(
            ctx_on_poll.metrics,
            "p2p.relay_mesh.len",
            behaviour.relay_mesh_len() as i64
        );
        let (period, received_msgs) = behaviour.received_messages_in_period();
        mm_gauge!(
            ctx_on_poll.metrics,
            "p2p.received_messages.period_in_secs",
            period.as_secs() as i64
        );

        mm_gauge!(ctx_on_poll.metrics, "p2p.received_messages.count", received_msgs as i64);

        let connected_peers_count = behaviour.connected_peers_len();

        mm_gauge!(
            ctx_on_poll.metrics,
            "p2p.connected_peers.count",
            connected_peers_count as i64
        );
    })
    .await;
    let (cmd_tx, event_rx, peer_id, p2p_abort) = try_s!(spawn_result);
    let mut p2p_abort = Some(p2p_abort);
    ctx.on_stop(Box::new(move || {
        if let Some(handle) = p2p_abort.take() {
            handle.abort();
        }
        Ok(())
    }));
    try_s!(ctx.peer_id.pin(peer_id.to_string()));
    let p2p_context = P2PContext::new(cmd_tx);
    p2p_context.store_to_mm_arc(&ctx);
    spawn(p2p_event_process_loop(ctx.weak(), event_rx, i_am_seed));

    Ok(())
}

fn seednodes(ctx: &MmArc) -> Result<Vec<RelayAddress>, String> {
    if ctx.conf["seednodes"].is_null() {
        if ctx.p2p_in_memory() {
            // If the network is in memory, there is no need to use default seednodes.
            return Ok(Vec::new());
        }
        return Ok(default_seednodes(ctx.netid()));
    }

    json::from_value(ctx.conf["seednodes"].clone()).map_err(|e| ERRL!("{}", e))
}

#[cfg(target_arch = "wasm32")]
async fn relay_node_type(ctx: &MmArc) -> Result<NodeType, String> {
    if ctx.p2p_in_memory() {
        return relay_in_memory_node_type(ctx);
    }
    ERR!("WASM node can be a seed if only 'p2p_in_memory' is true")
}

#[cfg(not(target_arch = "wasm32"))]
async fn relay_node_type(ctx: &MmArc) -> Result<NodeType, String> {
    if ctx.p2p_in_memory() {
        return relay_in_memory_node_type(ctx);
    }

    let netid = ctx.netid();
    let ip = try_s!(myipaddr(ctx.clone()).await);
    let network_ports = try_s!(lp_network_ports(netid));
    let wss_certs = try_s!(wss_certs(ctx));
    if wss_certs.is_none() {
        const WARN_MSG: &str = r#"Please note TLS private key and certificate are not specified.
To accept P2P WSS connections, please pass 'wss_certs' to the config.
Example:    "wss_certs": { "server_priv_key": "/path/to/key.pem", "certificate": "/path/to/cert.pem" }"#;
        warn!("{}", WARN_MSG);
    }

    Ok(NodeType::Relay {
        ip,
        network_ports,
        wss_certs,
    })
}

fn relay_in_memory_node_type(ctx: &MmArc) -> Result<NodeType, String> {
    Ok(NodeType::RelayInMemory {
        port: ctx
            .p2p_in_memory_port()
            .ok_or_else(|| ERRL!("'p2p_in_memory_port' not found in the config"))?,
    })
}

fn light_node_type(ctx: &MmArc) -> Result<NodeType, String> {
    if ctx.p2p_in_memory() {
        return Ok(NodeType::LightInMemory);
    }

    let netid = ctx.netid();
    let network_ports = try_s!(lp_network_ports(netid));
    Ok(NodeType::Light { network_ports })
}

/// Returns non-empty vector of keys/certs or an error.
#[cfg(not(target_arch = "wasm32"))]
fn extract_cert_from_file<T, P>(path: PathBuf, parser: P, expected_format: String) -> P2PResult<Vec<T>>
where
    P: Fn(&mut dyn io::BufRead) -> Result<Vec<T>, ()>,
{
    let certfile = fs::File::open(path.as_path()).map_to_mm(|error| P2PError::ErrorReadingCertFile {
        path: path.clone(),
        error,
    })?;
    let mut reader = io::BufReader::new(certfile);
    match parser(&mut reader) {
        Ok(certs) if certs.is_empty() => MmError::err(P2PError::InvalidWssCert { path, expected_format }),
        Ok(certs) => Ok(certs),
        Err(_) => MmError::err(P2PError::InvalidWssCert { path, expected_format }),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn wss_certs(ctx: &MmArc) -> P2PResult<Option<WssCerts>> {
    use futures_rustls::rustls;

    #[derive(Deserialize)]
    struct WssCertsInfo {
        server_priv_key: PathBuf,
        certificate: PathBuf,
    }

    if ctx.conf["wss_certs"].is_null() {
        return Ok(None);
    }
    let certs: WssCertsInfo =
        json::from_value(ctx.conf["wss_certs"].clone()).map_to_mm(|error| P2PError::ErrorDeserializingConfig {
            field: "wss_certs".to_owned(),
            error,
        })?;

    // First, try to extract the all PKCS8 private keys
    let mut server_priv_keys = extract_cert_from_file(
        certs.server_priv_key.clone(),
        rustls::internal::pemfile::pkcs8_private_keys,
        "Private key, DER-encoded ASN.1 in either PKCS#8 or PKCS#1 format".to_owned(),
    )
    // or try to extract all PKCS1 private keys
    .or_else(|_| {
        extract_cert_from_file(
            certs.server_priv_key.clone(),
            rustls::internal::pemfile::rsa_private_keys,
            "Private key, DER-encoded ASN.1 in either PKCS#8 or PKCS#1 format".to_owned(),
        )
    })?;
    // `extract_cert_from_file` returns either non-empty vector or an error.
    let server_priv_key = server_priv_keys.remove(0);

    let certs = extract_cert_from_file(
        certs.certificate,
        rustls::internal::pemfile::certs,
        "Certificate, DER-encoded X.509 format".to_owned(),
    )?;
    Ok(Some(WssCerts { server_priv_key, certs }))
}
