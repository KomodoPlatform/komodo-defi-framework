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
use common::executor::{SpawnFuture, Timer};
use common::log::{info, warn};
use crypto::{from_hw_error, CryptoCtx, CryptoInitError, HwError, HwProcessingError, HwRpcError, WithHwRpcError};
use derive_more::Display;
use enum_from::EnumFromTrait;
use mm2_core::mm_ctx::{MmArc, MmCtx};
use mm2_err_handle::common_errors::InternalError;
use mm2_err_handle::prelude::*;
use mm2_libp2p::{spawn_gossipsub, AdexBehaviourError, NodeType, RelayAddress, RelayAddressError, SwarmRuntime,
                 WssCerts};
use mm2_metrics::mm_gauge;
use mm2_net::p2p::P2PContext;
use rpc_task::RpcTaskError;
use serde_json::{self as json};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::str;
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
use crate::mm2::database::init_and_migrate_db;
use crate::mm2::lp_message_service::{init_message_service, InitMessageServiceError};
use crate::mm2::lp_network::{lp_network_ports, p2p_event_process_loop, NetIdError};
use crate::mm2::lp_ordermatch::{broadcast_maker_orders_keep_alive_loop, clean_memory_loop, init_ordermatch_context,
                                lp_ordermatch_loop, orders_kick_start, BalanceUpdateOrdermatchHandler,
                                OrdermatchInitError};
use crate::mm2::lp_swap::{running_swaps_num, swap_kick_starts};
use crate::mm2::rpc::spawn_rpc;

cfg_native! {
    use db_common::sqlite::rusqlite::Error as SqlError;
    use mm2_event_stream::behaviour::EventBehaviour;
    use mm2_io::fs::{ensure_dir_is_writable, ensure_file_is_writable};
    use mm2_net::ip_addr::myipaddr;
    use mm2_net::network_event::NetworkEvent;
}

#[path = "lp_init/init_context.rs"] mod init_context;
#[path = "lp_init/init_hw.rs"] pub mod init_hw;
#[cfg(target_arch = "wasm32")]
#[path = "lp_init/init_metamask.rs"]
pub mod init_metamask;

const NETID_8762_SEEDNODES: [&str; 3] = [
    "streamseed1.komodo.earth",
    "streamseed2.komodo.earth",
    "streamseed3.komodo.earth",
];

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
            error => P2PInitError::Internal(error.to_string()),
        }
    }
}

#[derive(Clone, Debug, Display, EnumFromTrait, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum MmInitError {
    Cancelled,
    #[from_trait(WithTimeout::timeout)]
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
    #[display(fmt = "The '{}' field has wrong value in the config: {}", field, error)]
    FieldWrongValueInConfig {
        field: String,
        error: String,
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
    #[display(fmt = "Passphrase cannot be an empty string")]
    EmptyPassphrase,
    #[display(fmt = "Invalid passphrase: {}", _0)]
    InvalidPassphrase(String),
    #[from_trait(WithHwRpcError::hw_rpc_error)]
    #[display(fmt = "{}", _0)]
    HwError(HwRpcError),
    #[from_trait(WithInternal::internal)]
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
            CryptoInitError::EmptyPassphrase => MmInitError::EmptyPassphrase,
            CryptoInitError::InvalidPassphrase(pass) => MmInitError::InvalidPassphrase(pass.to_string()),
            CryptoInitError::Internal(internal) => MmInitError::Internal(internal),
        }
    }
}

impl From<HwError> for MmInitError {
    fn from(e: HwError) -> Self { from_hw_error(e) }
}

impl From<RpcTaskError> for MmInitError {
    fn from(e: RpcTaskError) -> Self {
        let error = e.to_string();
        match e {
            RpcTaskError::Cancelled => MmInitError::Cancelled,
            RpcTaskError::Timeout(timeout) => MmInitError::Timeout(timeout),
            RpcTaskError::NoSuchTask(_)
            | RpcTaskError::UnexpectedTaskStatus { .. }
            | RpcTaskError::UnexpectedUserAction { .. } => MmInitError::Internal(error),
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

impl From<InternalError> for MmInitError {
    fn from(e: InternalError) -> Self { MmInitError::Internal(e.take()) }
}

impl MmInitError {
    pub fn db_directory_is_not_writable(path: &str) -> MmInitError {
        MmInitError::DbDirectoryIsNotWritable { path: path.to_owned() }
    }
}

#[cfg(target_arch = "wasm32")]
fn default_seednodes(netid: u16) -> Vec<RelayAddress> {
    if netid == 8762 {
        NETID_8762_SEEDNODES
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
    if netid == 8762 {
        NETID_8762_SEEDNODES
            .iter()
            .filter_map(|seed| addr_to_ipv4_string(seed).ok())
            .map(RelayAddress::IPv4)
            .collect()
    } else {
        Vec::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn fix_directories(ctx: &MmCtx) -> MmInitResult<()> {
    fix_shared_dbdir(ctx)?;

    let dbdir = ctx.dbdir();
    fs::create_dir_all(&dbdir).map_to_mm(|e| MmInitError::ErrorCreatingDbDir {
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
fn fix_shared_dbdir(ctx: &MmCtx) -> MmInitResult<()> {
    let shared_db = ctx.shared_dbdir();
    fs::create_dir_all(&shared_db).map_to_mm(|e| MmInitError::ErrorCreatingDbDir {
        path: shared_db.clone(),
        error: e.to_string(),
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
    std::fs::write(&migration_num_path, current_migration.to_le_bytes())
        .map_to_mm(|e| MmInitError::ErrorDbMigrating(e.to_string()))?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn migration_1(_ctx: &MmArc) {}

#[cfg(not(target_arch = "wasm32"))]
fn init_event_streaming(ctx: &MmArc) {
    // This condition only executed if events were enabled in mm2 configuration.
    if let Some(config) = &ctx.event_stream_configuration {
        // Network event handling
        NetworkEvent::new(ctx.clone()).spawn_if_active(config);
    }
}

pub async fn lp_init_continue(ctx: MmArc) -> MmInitResult<()> {
    init_ordermatch_context(&ctx)?;
    init_p2p(ctx.clone()).await?;

    if !CryptoCtx::is_init(&ctx)? {
        return Ok(());
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        fix_directories(&ctx)?;
        ctx.init_sqlite_connection()
            .map_to_mm(MmInitError::ErrorSqliteInitializing)?;
        ctx.init_shared_sqlite_conn()
            .map_to_mm(MmInitError::ErrorSqliteInitializing)?;
        init_and_migrate_db(&ctx).await?;
        migrate_db(&ctx)?;
    }

    init_message_service(&ctx).await?;

    let balance_update_ordermatch_handler = BalanceUpdateOrdermatchHandler::new(ctx.clone());
    register_balance_update_handler(ctx.clone(), Box::new(balance_update_ordermatch_handler)).await;

    ctx.initialized.pin(true).map_to_mm(MmInitError::Internal)?;

    // launch kickstart threads before RPC is available, this will prevent the API user to place
    // an order and start new swap that might get started 2 times because of kick-start
    kick_start(ctx.clone()).await?;

    #[cfg(not(target_arch = "wasm32"))]
    init_event_streaming(&ctx);

    ctx.spawner().spawn(lp_ordermatch_loop(ctx.clone()));

    ctx.spawner().spawn(broadcast_maker_orders_keep_alive_loop(ctx.clone()));

    ctx.spawner().spawn(clean_memory_loop(ctx.weak()));

    Ok(())
}

#[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
/// * `ctx_cb` - callback used to share the `MmCtx` ID with the call site.
pub async fn lp_init(ctx: MmArc, version: String, datetime: String) -> MmInitResult<()> {
    info!("Version: {} DT {}", version, datetime);

    if !ctx.conf["passphrase"].is_null() {
        let passphrase: String =
            json::from_value(ctx.conf["passphrase"].clone()).map_to_mm(|e| MmInitError::ErrorDeserializingConfig {
                field: "passphrase".to_owned(),
                error: e.to_string(),
            })?;

        // This defaults to false to maintain backward compatibility.
        match ctx.conf["enable_hd"].as_bool().unwrap_or(false) {
            true => CryptoCtx::init_with_global_hd_account(ctx.clone(), &passphrase)?,
            false => CryptoCtx::init_with_iguana_passphrase(ctx.clone(), &passphrase)?,
        };
    }

    lp_init_continue(ctx.clone()).await?;

    let ctx_id = ctx.ffi_handle().map_to_mm(MmInitError::Internal)?;

    spawn_rpc(ctx_id);
    let ctx_c = ctx.clone();

    ctx.spawner().spawn(async move {
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

pub async fn init_p2p(ctx: MmArc) -> P2PResult<()> {
    let i_am_seed = ctx.conf["i_am_seed"].as_bool().unwrap_or(false);
    let netid = ctx.netid();

    let seednodes = seednodes(&ctx)?;

    let ctx_on_poll = ctx.clone();
    let force_p2p_key = if i_am_seed {
        let crypto_ctx = CryptoCtx::from_ctx(&ctx).mm_err(|e| P2PInitError::Internal(e.to_string()))?;
        let key = sha256(crypto_ctx.mm2_internal_privkey_slice());
        Some(key.take())
    } else {
        None
    };

    let node_type = if i_am_seed {
        relay_node_type(&ctx).await?
    } else {
        light_node_type(&ctx)?
    };

    let spawner = SwarmRuntime::new(ctx.spawner());
    let spawn_result = spawn_gossipsub(netid, force_p2p_key, spawner, seednodes, node_type, move |swarm| {
        let behaviour = swarm.behaviour();
        mm_gauge!(
            ctx_on_poll.metrics,
            "p2p.connected_relays.len",
            behaviour.connected_relays_len() as f64
        );
        mm_gauge!(
            ctx_on_poll.metrics,
            "p2p.relay_mesh.len",
            behaviour.relay_mesh_len() as f64
        );
        let (period, received_msgs) = behaviour.received_messages_in_period();
        mm_gauge!(
            ctx_on_poll.metrics,
            "p2p.received_messages.period_in_secs",
            period.as_secs() as f64
        );

        mm_gauge!(ctx_on_poll.metrics, "p2p.received_messages.count", received_msgs as f64);

        let connected_peers_count = behaviour.connected_peers_len();

        mm_gauge!(
            ctx_on_poll.metrics,
            "p2p.connected_peers.count",
            connected_peers_count as f64
        );
    })
    .await;
    let (cmd_tx, event_rx, peer_id) = spawn_result?;
    ctx.peer_id.pin(peer_id.to_string()).map_to_mm(P2PInitError::Internal)?;
    let p2p_context = P2PContext::new(cmd_tx);
    p2p_context.store_to_mm_arc(&ctx);

    let fut = p2p_event_process_loop(ctx.weak(), event_rx, i_am_seed);
    ctx.spawner().spawn(fut);

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
