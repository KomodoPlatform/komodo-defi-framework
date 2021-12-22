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

use bitcrypto::sha256;
use coins::register_balance_update_handler;
use common::executor::{spawn, spawn_boxed, Timer};
use common::log::{error, info, warn};
use common::mm_ctx::{MmArc, MmCtx};
use common::mm_error::prelude::*;
use crypto::{CryptoCtx, CryptoInitError, HwError, HwProcessingError};
use derive_more::Display;
use mm2_libp2p::{spawn_gossipsub, AdexBehaviourError, NodeType, RelayAddress, RelayAddressError, WssCerts};
use rand::random;
use rpc_task::RpcTaskError;
use serde_json::{self as json};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::str;
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
use crate::mm2::database::init_and_migrate_db;
use crate::mm2::lp_message_service::{init_message_service, InitMessageServiceError};
use crate::mm2::lp_network::{lp_network_ports, p2p_event_process_loop, NetIdError, P2PContext};
use crate::mm2::lp_ordermatch::{broadcast_maker_orders_keep_alive_loop, clean_memory_loop, init_ordermatch_context,
                                lp_ordermatch_loop, orders_kick_start, BalanceUpdateOrdermatchHandler,
                                OrdermatchInitError};
use crate::mm2::lp_swap::{running_swaps_num, swap_kick_starts};
use crate::mm2::rpc::spawn_rpc;
use crate::mm2::{MM_DATETIME, MM_VERSION};

cfg_native! {
    use common::ip_addr::myipaddr;
    use db_common::sqlite::rusqlite::Error as SqlError;
}

#[path = "lp_init/init_context.rs"] mod init_context;
#[path = "lp_init/mm_init_task.rs"] mod mm_init_task;
#[path = "lp_init/rpc_command.rs"] pub mod rpc_command;

use mm_init_task::MmInitTask;

const NETID_7777_SEEDNODES: [&str; 3] = ["seed1.defimania.live", "seed2.defimania.live", "seed3.defimania.live"];

pub type P2PResult<T> = Result<T, MmError<P2PInitError>>;
pub type MmInitResult<T> = Result<T, MmError<MmInitError>>;

#[derive(Clone, Debug, Display, Serialize)]
pub enum P2PInitError {
    #[display(
        fmt = "Invalid WSS key/cert at {:?}. The file must contain {}'",
        path,
        expected_format
    )]
    InvalidWssCert { path: PathBuf, expected_format: String },
    #[display(fmt = "Error deserializing '{}' config field: {}", field, error)]
    ErrorDeserializingConfig { field: String, error: String },
    #[display(fmt = "The '{}' field not found in the config", field)]
    FieldNotFoundInConfig { field: String },
    #[display(fmt = "Error reading WSS key/cert file {:?}: {}", path, error)]
    ErrorReadingCertFile { path: PathBuf, error: String },
    #[display(fmt = "Error getting my IP address: '{}'", _0)]
    ErrorGettingMyIpAddr(String),
    #[display(fmt = "Invalid netid: '{}'", _0)]
    InvalidNetId(NetIdError),
    #[display(fmt = "Invalid relay address: '{}'", _0)]
    InvalidRelayAddress(RelayAddressError),
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    #[display(fmt = "WASM node can be a seed if only 'p2p_in_memory' is true")]
    WasmNodeCannotBeSeed,
    #[display(fmt = "Internal error: '{}'", _0)]
    Internal(String),
}

impl From<NetIdError> for P2PInitError {
    fn from(e: NetIdError) -> Self { P2PInitError::InvalidNetId(e) }
}

impl From<AdexBehaviourError> for P2PInitError {
    fn from(e: AdexBehaviourError) -> Self {
        match e {
            AdexBehaviourError::ParsingRelayAddress(e) => P2PInitError::InvalidRelayAddress(e),
        }
    }
}

#[derive(Clone, Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum MmInitError {
    Canceled,
    #[display(fmt = "Initialization timeout {:?}", _0)]
    Timeout(Duration),
    #[display(fmt = "Error deserializing '{}' config field: {}", field, error)]
    ErrorDeserializingConfig {
        field: String,
        error: String,
    },
    #[display(fmt = "The '{}' field not found in the config", field)]
    FieldNotFoundInConfig {
        field: String,
    },
    #[display(fmt = "P2P initializing error: '{}'", _0)]
    P2PError(P2PInitError),
    #[display(fmt = "Error creating DB director '{:?}': {}", path, error)]
    ErrorCreatingDbDir {
        path: PathBuf,
        error: String,
    },
    #[display(fmt = "{} db dir is not writable", path)]
    DbDirectoryIsNotWritable {
        path: String,
    },
    #[display(fmt = "{} db file is not writable", path)]
    DbFileIsNotWritable {
        path: String,
    },
    #[display(fmt = "sqlite initializing error: {}", _0)]
    ErrorSqliteInitializing(String),
    #[display(fmt = "DB migrating error: {}", _0)]
    ErrorDbMigrating(String),
    #[display(fmt = "Swap kick start error: {}", _0)]
    SwapsKickStartError(String),
    #[display(fmt = "Order kick start error: {}", _0)]
    OrdersKickStartError(String),
    NullStringPassphrase,
    #[display(fmt = "Invalid passphrase: {}", _0)]
    InvalidPassphrase(String),
    #[display(fmt = "No Trezor device available")]
    NoTrezorDeviceAvailable,
    #[display(fmt = "Hardware Wallet error: {}", _0)]
    HardwareWalletError(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl From<P2PInitError> for MmInitError {
    fn from(e: P2PInitError) -> Self {
        match e {
            P2PInitError::ErrorDeserializingConfig { field, error } => {
                MmInitError::ErrorDeserializingConfig { field, error }
            },
            P2PInitError::FieldNotFoundInConfig { field } => MmInitError::FieldNotFoundInConfig { field },
            P2PInitError::Internal(e) => MmInitError::Internal(e),
            other => MmInitError::P2PError(other),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqlError> for MmInitError {
    fn from(e: SqlError) -> Self { MmInitError::ErrorSqliteInitializing(e.to_string()) }
}

impl From<OrdermatchInitError> for MmInitError {
    fn from(e: OrdermatchInitError) -> Self {
        match e {
            OrdermatchInitError::ErrorDeserializingConfig { field, error } => {
                MmInitError::ErrorDeserializingConfig { field, error }
            },
            OrdermatchInitError::Internal(internal) => MmInitError::Internal(internal),
        }
    }
}

impl From<InitMessageServiceError> for MmInitError {
    fn from(e: InitMessageServiceError) -> Self {
        match e {
            InitMessageServiceError::ErrorDeserializingConfig { field, error } => {
                MmInitError::ErrorDeserializingConfig { field, error }
            },
        }
    }
}

impl From<CryptoInitError> for MmInitError {
    fn from(e: CryptoInitError) -> Self {
        match e {
            e @ CryptoInitError::InitializedAlready | e @ CryptoInitError::NotInitialized => {
                MmInitError::Internal(e.to_string())
            },
            CryptoInitError::NullStringPassphrase => MmInitError::NullStringPassphrase,
            CryptoInitError::InvalidPassphrase(pass) => MmInitError::InvalidPassphrase(pass.to_string()),
            CryptoInitError::Internal(internal) => MmInitError::Internal(internal),
        }
    }
}

impl From<HwError> for MmInitError {
    fn from(e: HwError) -> Self {
        match e {
            HwError::NoTrezorDeviceAvailable => MmInitError::NoTrezorDeviceAvailable,
            HwError::Internal(internal) => MmInitError::Internal(internal),
            hw => MmInitError::HardwareWalletError(hw.to_string()),
        }
    }
}

impl From<RpcTaskError> for MmInitError {
    fn from(e: RpcTaskError) -> Self {
        let error = e.to_string();
        match e {
            RpcTaskError::Canceled => MmInitError::Canceled,
            RpcTaskError::Timeout(timeout) => MmInitError::Timeout(timeout),
            RpcTaskError::NoSuchTask(_) | RpcTaskError::UnexpectedTaskStatus { .. } => MmInitError::Internal(error),
            RpcTaskError::Internal(internal) => MmInitError::Internal(internal),
        }
    }
}

impl From<HwProcessingError<RpcTaskError>> for MmInitError {
    fn from(e: HwProcessingError<RpcTaskError>) -> Self {
        match e {
            HwProcessingError::HwError(hw) => MmInitError::from(hw),
            HwProcessingError::ProcessorError(rpc_task) => MmInitError::from(rpc_task),
        }
    }
}

impl MmInitError {
    pub fn db_directory_is_not_writable(path: &str) -> MmInitError {
        MmInitError::DbDirectoryIsNotWritable { path: path.to_owned() }
    }
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
fn fix_directories(ctx: &MmCtx) -> MmInitResult<()> {
    let dbdir = ctx.dbdir();
    std::fs::create_dir_all(&dbdir).map_to_mm(|e| MmInitError::ErrorCreatingDbDir {
        path: dbdir.clone(),
        error: e.to_string(),
    })?;

    if !ensure_dir_is_writable(&dbdir.join("SWAPS")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("SWAPS"));
    }
    if !ensure_dir_is_writable(&dbdir.join("SWAPS").join("MY")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("SWAPS/MY"));
    }
    if !ensure_dir_is_writable(&dbdir.join("SWAPS").join("STATS")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("SWAPS/STATS"));
    }
    if !ensure_dir_is_writable(&dbdir.join("SWAPS").join("STATS").join("MAKER")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("SWAPS/STATS/MAKER"));
    }
    if !ensure_dir_is_writable(&dbdir.join("SWAPS").join("STATS").join("TAKER")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("SWAPS/STATS/TAKER"));
    }
    if !ensure_dir_is_writable(&dbdir.join("TRANSACTIONS")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("TRANSACTIONS"));
    }
    if !ensure_dir_is_writable(&dbdir.join("GTC")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("GTC"));
    }
    if !ensure_dir_is_writable(&dbdir.join("PRICES")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("PRICES"));
    }
    if !ensure_dir_is_writable(&dbdir.join("UNSPENTS")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("UNSPENTS"));
    }
    if !ensure_dir_is_writable(&dbdir.join("ORDERS")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("ORDERS"));
    }
    if !ensure_dir_is_writable(&dbdir.join("ORDERS").join("MY")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("ORDERS/MY"));
    }
    if !ensure_dir_is_writable(&dbdir.join("ORDERS").join("MY").join("MAKER")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("ORDERS/MY/MAKER"));
    }
    if !ensure_dir_is_writable(&dbdir.join("ORDERS").join("MY").join("TAKER")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("ORDERS/MY/TAKER"));
    }
    if !ensure_dir_is_writable(&dbdir.join("ORDERS").join("MY").join("HISTORY")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("ORDERS/MY/HISTORY"));
    }
    if !ensure_dir_is_writable(&dbdir.join("TX_CACHE")) {
        return MmError::err(MmInitError::db_directory_is_not_writable("TX_CACHE"));
    }
    ensure_file_is_writable(&dbdir.join("GTC").join("orders")).map_to_mm(|_| MmInitError::DbFileIsNotWritable {
        path: "GTC/orders".to_owned(),
    })?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn migrate_db(ctx: &MmArc) -> MmInitResult<()> {
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
    std::fs::write(&migration_num_path, &current_migration.to_le_bytes())
        .map_to_mm(|e| MmInitError::ErrorDbMigrating(e.to_string()))?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn migration_1(_ctx: &MmArc) {}

pub async fn lp_init_continue(ctx: MmArc) -> MmInitResult<()> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        fix_directories(&ctx)?;
        ctx.init_sqlite_connection()
            .map_to_mm(MmInitError::ErrorSqliteInitializing)?;
        init_and_migrate_db(&ctx).await?;
        migrate_db(&ctx)?;
    }

    init_ordermatch_context(&ctx)?;
    init_message_service(&ctx).await?;
    init_p2p(ctx.clone()).await?;

    let balance_update_ordermatch_handler = BalanceUpdateOrdermatchHandler::new(ctx.clone());
    register_balance_update_handler(ctx.clone(), Box::new(balance_update_ordermatch_handler)).await;

    ctx.initialized.pin(true).map_to_mm(MmInitError::Internal)?;

    // launch kickstart threads before RPC is available, this will prevent the API user to place
    // an order and start new swap that might get started 2 times because of kick-start
    kick_start(ctx.clone()).await?;

    spawn(lp_ordermatch_loop(ctx.clone()));

    spawn(broadcast_maker_orders_keep_alive_loop(ctx.clone()));

    spawn(clean_memory_loop(ctx.weak()));
    Ok(())
}

#[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
/// * `ctx_cb` - callback used to share the `MmCtx` ID with the call site.
pub async fn lp_init(ctx: MmArc) -> MmInitResult<()> {
    info!("Version: {} DT {}", MM_VERSION, MM_DATETIME);

    if ctx.conf["passphrase"].is_null() && ctx.conf["hw_wallet"].is_null() {
        return MmError::err(MmInitError::FieldNotFoundInConfig {
            field: "passphrase".to_owned(),
        });
    }

    if ctx.conf["passphrase"].is_null() {
        // TODO
        // Currently, `MmInitTask` initializes `CryptoCtx` with Hardware Wallet only.
        // Later I'm going to change it.
        // The main blocker is that if an error occurs while MarketMaker initializing,
        // we should exit with an error since our users are used to this behaviour.
        // But `MmInitTask` inserts the error into [`MmCtx::rpc_task_manager`] and doesn't stop anything.
        let task = MmInitTask::new(ctx.clone());
        task.spawn()?;
    } else {
        let passphrase: String =
            json::from_value(ctx.conf["passphrase"].clone()).map_to_mm(|e| MmInitError::ErrorDeserializingConfig {
                field: "passphrase".to_owned(),
                error: e.to_string(),
            })?;
        CryptoCtx::init_with_passphrase(ctx.clone(), &passphrase)?;
        lp_init_continue(ctx.clone()).await?;
    }

    let ctx_id = ctx.ffi_handle().map_to_mm(MmInitError::Internal)?;

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

async fn kick_start(ctx: MmArc) -> MmInitResult<()> {
    let mut coins_needed_for_kick_start = swap_kick_starts(ctx.clone())
        .await
        .map_to_mm(MmInitError::SwapsKickStartError)?;
    coins_needed_for_kick_start.extend(
        orders_kick_start(&ctx)
            .await
            .map_to_mm(MmInitError::OrdersKickStartError)?,
    );
    let mut lock = ctx
        .coins_needed_for_kick_start
        .lock()
        .map_to_mm(|poison| MmInitError::Internal(poison.to_string()))?;
    *lock = coins_needed_for_kick_start;
    Ok(())
}

async fn init_p2p(ctx: MmArc) -> P2PResult<()> {
    let i_am_seed = ctx.conf["i_am_seed"].as_bool().unwrap_or(false);
    let netid = ctx.netid();

    let seednodes = seednodes(&ctx)?;

    let ctx_on_poll = ctx.clone();
    let force_p2p_key = if i_am_seed {
        let key = sha256(&*ctx.secp256k1_key_pair().private().secret);
        Some(key.take())
    } else {
        None
    };

    let node_type = if i_am_seed {
        relay_node_type(&ctx).await?
    } else {
        light_node_type(&ctx)?
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
    let (cmd_tx, event_rx, peer_id, p2p_abort) = spawn_result?;
    let mut p2p_abort = Some(p2p_abort);
    ctx.on_stop(Box::new(move || {
        if let Some(handle) = p2p_abort.take() {
            handle.abort();
        }
        Ok(())
    }));
    ctx.peer_id.pin(peer_id.to_string()).map_to_mm(P2PInitError::Internal)?;
    let p2p_context = P2PContext::new(cmd_tx);
    p2p_context.store_to_mm_arc(&ctx);
    spawn(p2p_event_process_loop(ctx.weak(), event_rx, i_am_seed));

    Ok(())
}

fn seednodes(ctx: &MmArc) -> P2PResult<Vec<RelayAddress>> {
    if ctx.conf["seednodes"].is_null() {
        if ctx.p2p_in_memory() {
            // If the network is in memory, there is no need to use default seednodes.
            return Ok(Vec::new());
        }
        return Ok(default_seednodes(ctx.netid()));
    }

    json::from_value(ctx.conf["seednodes"].clone()).map_to_mm(|e| P2PInitError::ErrorDeserializingConfig {
        field: "seednodes".to_owned(),
        error: e.to_string(),
    })
}

#[cfg(target_arch = "wasm32")]
async fn relay_node_type(ctx: &MmArc) -> P2PResult<NodeType> {
    if ctx.p2p_in_memory() {
        return relay_in_memory_node_type(ctx);
    }
    MmError::err(P2PInitError::WasmNodeCannotBeSeed)
}

#[cfg(not(target_arch = "wasm32"))]
async fn relay_node_type(ctx: &MmArc) -> P2PResult<NodeType> {
    if ctx.p2p_in_memory() {
        return relay_in_memory_node_type(ctx);
    }

    let netid = ctx.netid();
    let ip = myipaddr(ctx.clone())
        .await
        .map_to_mm(P2PInitError::ErrorGettingMyIpAddr)?;
    let network_ports = lp_network_ports(netid)?;
    let wss_certs = wss_certs(ctx)?;
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

fn relay_in_memory_node_type(ctx: &MmArc) -> P2PResult<NodeType> {
    let port = ctx
        .p2p_in_memory_port()
        .or_mm_err(|| P2PInitError::FieldNotFoundInConfig {
            field: "p2p_in_memory_port".to_owned(),
        })?;
    Ok(NodeType::RelayInMemory { port })
}

fn light_node_type(ctx: &MmArc) -> P2PResult<NodeType> {
    if ctx.p2p_in_memory() {
        return Ok(NodeType::LightInMemory);
    }

    let netid = ctx.netid();
    let network_ports = lp_network_ports(netid)?;
    Ok(NodeType::Light { network_ports })
}

/// Returns non-empty vector of keys/certs or an error.
#[cfg(not(target_arch = "wasm32"))]
fn extract_cert_from_file<T, P>(path: PathBuf, parser: P, expected_format: String) -> P2PResult<Vec<T>>
where
    P: Fn(&mut dyn io::BufRead) -> Result<Vec<T>, ()>,
{
    let certfile = fs::File::open(path.as_path()).map_to_mm(|e| P2PInitError::ErrorReadingCertFile {
        path: path.clone(),
        error: e.to_string(),
    })?;
    let mut reader = io::BufReader::new(certfile);
    match parser(&mut reader) {
        Ok(certs) if certs.is_empty() => MmError::err(P2PInitError::InvalidWssCert { path, expected_format }),
        Ok(certs) => Ok(certs),
        Err(_) => MmError::err(P2PInitError::InvalidWssCert { path, expected_format }),
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
        json::from_value(ctx.conf["wss_certs"].clone()).map_to_mm(|e| P2PInitError::ErrorDeserializingConfig {
            field: "wss_certs".to_owned(),
            error: e.to_string(),
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
