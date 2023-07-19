#[cfg(feature = "track-ctx-pointer")]
use common::executor::Timer;
use common::executor::{abortable_queue::{AbortableQueue, WeakSpawner},
                       graceful_shutdown, AbortSettings, AbortableSystem, SpawnAbortable, SpawnFuture};
use common::log::{self, LogLevel, LogOnError, LogState};
use common::{cfg_native, cfg_wasm32, small_rng};
use gstuff::{try_s, Constructible, ERR, ERRL};
use lazy_static::lazy_static;
use mm2_metrics::{MetricsArc, MetricsOps};
use primitives::hash::H160;
use rand::Rng;
use serde_json::{self as json, Value as Json};
use shared_ref_counter::{SharedRc, WeakRc};
use std::any::Any;
use std::collections::hash_map::{Entry, HashMap};
use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

cfg_wasm32! {
    use mm2_rpc::wasm_rpc::WasmRpcSender;
    use crate::DbNamespaceId;
}

cfg_native! {
    use db_common::sqlite::rusqlite::Connection;
    use futures_rustls::webpki::DNSNameRef;
    use mm2_metrics::prometheus;
    use mm2_metrics::MmMetricsError;
    use std::net::{IpAddr, SocketAddr, AddrParseError};
    use std::path::{Path, PathBuf};
    use std::str::FromStr;
    use std::sync::MutexGuard;
}

/// Default interval to export and record metrics to log.
const EXPORT_METRICS_INTERVAL: f64 = 5. * 60.;

/// MarketMaker state, shared between the various MarketMaker threads.
///
/// Every MarketMaker has one and only one instance of `MmCtx`.
///
/// Should fully replace `LP_globals`.
///
/// *Not* a singleton: we should be able to run multiple MarketMakers instances in a process.
///
/// Any function directly using `MmCtx` is automatically a stateful function.
/// In the future we might want to replace direct state access with traceable and replayable
/// state modifications
/// (cf. https://github.com/artemii235/SuperNET/blob/mm2-dice/mm2src/README.md#purely-functional-core).
///
/// `MmCtx` never moves in memory (and it isn't `Send`), it is created and then destroyed in place
/// (this invariant should make it a bit simpler thinking about aliasing and thread-safety,
/// particularly of the C structures during the gradual port).
/// Only the pointers (`MmArc`, `MmWeak`) can be moved around.
///
/// Threads only have the non-`mut` access to `MmCtx`, allowing us to directly share certain fields.
pub struct MmCtx {
    /// MM command-line configuration.
    pub conf: Json,
    /// Human-readable log and status dashboard.
    pub log: log::LogArc,
    /// Tools and methods and to collect and export the MM metrics.
    pub metrics: MetricsArc,
    /// Set to true after `lp_passphrase_init`, indicating that we have a usable state.
    ///
    /// Should be refactored away in the future. State should always be valid.
    /// If there are things that are loaded in background then they should be separately optional,
    /// without invalidating the entire state.
    pub initialized: Constructible<bool>,
    /// True if the RPC HTTP server was started.
    pub rpc_started: Constructible<bool>,
    /// True if the MarketMaker instance needs to stop.
    pub stop: Constructible<bool>,
    /// Unique context identifier, allowing us to more easily pass the context through the FFI boundaries.  
    /// 0 if the handler ID is allocated yet.
    pub ffi_handle: Constructible<u32>,
    /// The context belonging to the `ordermatch` mod: `OrdermatchContext`.
    pub ordermatch_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    pub rate_limit_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    pub simple_market_maker_bot_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    pub dispatcher_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    pub message_service_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    pub p2p_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    pub peer_id: Constructible<String>,
    pub account_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    /// The context belonging to the `coins` crate: `CoinsContext`.
    pub coins_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    pub coins_activation_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    pub crypto_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    /// RIPEMD160(SHA256(x)) where x is secp256k1 pubkey derived from passphrase.
    /// This hash is **unique** among Iguana and each HD accounts derived from the same passphrase.
    pub rmd160: Constructible<H160>,
    /// A shared DB identifier - RIPEMD160(SHA256(x)) where x is secp256k1 pubkey derived from (passphrase + magic salt).
    /// This hash is **the same** for Iguana and all HD accounts derived from the same passphrase.
    pub shared_db_id: Constructible<H160>,
    /// Coins that should be enabled to kick start the interrupted swaps and orders.
    pub coins_needed_for_kick_start: Mutex<HashSet<String>>,
    /// The context belonging to the `lp_swap` mod: `SwapsContext`.
    pub swaps_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    /// The context belonging to the `lp_stats` mod: `StatsContext`
    pub stats_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    /// The RPC sender forwarding requests to writing part of underlying stream.
    #[cfg(target_arch = "wasm32")]
    pub wasm_rpc: Constructible<WasmRpcSender>,
    #[cfg(not(target_arch = "wasm32"))]
    pub sqlite_connection: Constructible<Arc<Mutex<Connection>>>,
    #[cfg(not(target_arch = "wasm32"))]
    pub shared_sqlite_conn: Constructible<Arc<Mutex<Connection>>>,
    pub mm_version: String,
    pub datetime: String,
    pub mm_init_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    /// The abortable system is pinned to the `MmCtx` context.
    /// It's used to spawn futures that can be aborted immediately or after a timeout
    /// on the [`MmArc::stop`] function call.
    pub abortable_system: AbortableQueue,
    /// The abortable system is pinned to the `MmCtx` context.
    /// It's used to register listeners that will wait for graceful shutdown.
    pub graceful_shutdown_registry: graceful_shutdown::GracefulShutdownRegistry,
    #[cfg(target_arch = "wasm32")]
    pub db_namespace: DbNamespaceId,
    /// The context belonging to the `nft` mod: `NftCtx`.
    pub nft_ctx: Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
}

impl MmCtx {
    pub fn with_log_state(log: LogState) -> MmCtx {
        MmCtx {
            conf: Json::Object(json::Map::new()),
            log: log::LogArc::new(log),
            metrics: MetricsArc::new(),
            initialized: Constructible::default(),
            rpc_started: Constructible::default(),
            stop: Constructible::default(),
            ffi_handle: Constructible::default(),
            ordermatch_ctx: Mutex::new(None),
            rate_limit_ctx: Mutex::new(None),
            simple_market_maker_bot_ctx: Mutex::new(None),
            dispatcher_ctx: Mutex::new(None),
            message_service_ctx: Mutex::new(None),
            p2p_ctx: Mutex::new(None),
            peer_id: Constructible::default(),
            account_ctx: Mutex::new(None),
            coins_ctx: Mutex::new(None),
            coins_activation_ctx: Mutex::new(None),
            crypto_ctx: Mutex::new(None),
            rmd160: Constructible::default(),
            shared_db_id: Constructible::default(),
            coins_needed_for_kick_start: Mutex::new(HashSet::new()),
            swaps_ctx: Mutex::new(None),
            stats_ctx: Mutex::new(None),
            #[cfg(target_arch = "wasm32")]
            wasm_rpc: Constructible::default(),
            #[cfg(not(target_arch = "wasm32"))]
            sqlite_connection: Constructible::default(),
            #[cfg(not(target_arch = "wasm32"))]
            shared_sqlite_conn: Constructible::default(),
            mm_version: "".into(),
            datetime: "".into(),
            mm_init_ctx: Mutex::new(None),
            abortable_system: AbortableQueue::default(),
            graceful_shutdown_registry: graceful_shutdown::GracefulShutdownRegistry::default(),
            #[cfg(target_arch = "wasm32")]
            db_namespace: DbNamespaceId::Main,
            nft_ctx: Mutex::new(None),
        }
    }

    pub fn rmd160(&self) -> &H160 {
        lazy_static! {
            static ref DEFAULT: H160 = [0; 20].into();
        }
        self.rmd160.or(&|| &*DEFAULT)
    }

    pub fn shared_db_id(&self) -> &H160 {
        lazy_static! {
            static ref DEFAULT: H160 = [0; 20].into();
        }
        self.shared_db_id.or(&|| &*DEFAULT)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn rpc_ip_port(&self) -> Result<SocketAddr, String> {
        let port = self.conf["rpcport"].as_u64().unwrap_or(7783);
        if port < 1000 {
            return ERR!("rpcport < 1000");
        }
        if port > u16::MAX as u64 {
            return ERR!("rpcport > u16");
        }

        let rpcip = if !self.conf["rpcip"].is_null() {
            try_s!(self.conf["rpcip"].as_str().ok_or("rpcip is not a string"))
        } else {
            "127.0.0.1"
        }
        .to_string();
        let ip: IpAddr = try_s!(rpcip.parse());
        Ok(SocketAddr::new(ip, port as u16))
    }

    /// Whether to use HTTPS for RPC server or not.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn is_https(&self) -> bool { self.conf["https"].as_bool().unwrap_or(false) }

    /// SANs for self-signed certificate generation.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn alt_names(&self) -> Result<Vec<String>, String> {
        // Helper function to validate `alt_names` entries
        fn validate_alt_name(name: &str) -> Result<(), String> {
            // Check if it is a valid IP address
            if let Ok(ip) = IpAddr::from_str(name) {
                if ip.is_unspecified() {
                    return ERR!("IP address {} must be specified", ip);
                }
                return Ok(());
            }

            // Check if it is a valid DNS name
            if DNSNameRef::try_from_ascii_str(name).is_ok() {
                return Ok(());
            }

            ERR!(
                "`alt_names` contains {} which is neither a valid IP address nor a valid DNS name",
                name
            )
        }

        if self.conf["alt_names"].is_null() {
            // Default SANs
            return Ok(vec!["localhost".to_string(), "127.0.0.1".to_string()]);
        }

        json::from_value(self.conf["alt_names"].clone())
            .map_err(|e| format!("`alt_names` is not a valid JSON array of strings: {}", e))
            .and_then(|names: Vec<String>| {
                if names.is_empty() {
                    return ERR!("alt_names is empty");
                }
                for name in &names {
                    try_s!(validate_alt_name(name));
                }
                Ok(names)
            })
    }

    /// MM database path.  
    /// Defaults to a relative "DB".
    ///
    /// Can be changed via the "dbdir" configuration field, for example:
    ///
    ///     "dbdir": "c:/Users/mm2user/.mm2-db"
    ///
    /// No checks in this method, the paths should be checked in the `fn fix_directories` instead.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn dbdir(&self) -> PathBuf { path_to_dbdir(self.conf["dbdir"].as_str(), self.rmd160()) }

    /// MM shared database path.
    /// Defaults to a relative "DB".
    ///
    /// Can be changed via the "dbdir" configuration field, for example:
    ///
    ///     "dbdir": "c:/Users/mm2user/.mm2-db"
    ///
    /// No checks in this method, the paths should be checked in the `fn fix_directories` instead.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn shared_dbdir(&self) -> PathBuf { path_to_dbdir(self.conf["dbdir"].as_str(), self.shared_db_id()) }

    pub fn is_watcher(&self) -> bool { self.conf["is_watcher"].as_bool().unwrap_or_default() }

    pub fn use_watchers(&self) -> bool {
        std::env::var("USE_WATCHERS").is_ok()
        //self.conf["use_watchers"].as_bool().unwrap_or(true)
    }

    pub fn netid(&self) -> u16 {
        let netid = self.conf["netid"].as_u64().unwrap_or(0);
        if netid > u16::MAX.into() {
            panic!("netid {} is too big", netid)
        }
        netid as u16
    }

    pub fn p2p_in_memory(&self) -> bool { self.conf["p2p_in_memory"].as_bool().unwrap_or(false) }

    pub fn p2p_in_memory_port(&self) -> Option<u64> { self.conf["p2p_in_memory_port"].as_u64() }

    /// Returns the cloneable `MmFutSpawner`.
    pub fn spawner(&self) -> MmFutSpawner { MmFutSpawner::new(&self.abortable_system) }

    /// True if the MarketMaker instance needs to stop.
    pub fn is_stopping(&self) -> bool { self.stop.copy_or(false) }

    pub fn gui(&self) -> Option<&str> { self.conf["gui"].as_str() }

    pub fn mm_version(&self) -> &str { &self.mm_version }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn init_sqlite_connection(&self) -> Result<(), String> {
        let sqlite_file_path = self.dbdir().join("MM2.db");
        log::debug!("Trying to open SQLite database file {}", sqlite_file_path.display());
        let connection = try_s!(Connection::open(sqlite_file_path));
        try_s!(self.sqlite_connection.pin(Arc::new(Mutex::new(connection))));
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn init_shared_sqlite_conn(&self) -> Result<(), String> {
        let sqlite_file_path = self.shared_dbdir().join("MM2-shared.db");
        log::debug!("Trying to open SQLite database file {}", sqlite_file_path.display());
        let connection = try_s!(Connection::open(sqlite_file_path));
        try_s!(self.shared_sqlite_conn.pin(Arc::new(Mutex::new(connection))));
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn sqlite_conn_opt(&self) -> Option<MutexGuard<Connection>> {
        self.sqlite_connection.as_option().map(|conn| conn.lock().unwrap())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn sqlite_connection(&self) -> MutexGuard<Connection> {
        self.sqlite_connection
            .or(&|| panic!("sqlite_connection is not initialized"))
            .lock()
            .unwrap()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn shared_sqlite_conn(&self) -> MutexGuard<Connection> {
        self.shared_sqlite_conn
            .or(&|| panic!("shared_sqlite_conn is not initialized"))
            .lock()
            .unwrap()
    }
}

impl Default for MmCtx {
    fn default() -> Self { Self::with_log_state(LogState::in_memory()) }
}

impl Drop for MmCtx {
    fn drop(&mut self) {
        let ffi_handle = self
            .ffi_handle
            .as_option()
            .map(|handle| handle.to_string())
            .unwrap_or_else(|| "UNKNOWN".to_owned());
        log::info!("MmCtx ({}) has been dropped", ffi_handle)
    }
}

/// This function can be used later by an FFI function to open a GUI storage.
#[cfg(not(target_arch = "wasm32"))]
pub fn path_to_dbdir(db_root: Option<&str>, db_id: &H160) -> PathBuf {
    const DEFAULT_ROOT: &str = "DB";

    let path = match db_root {
        Some(dbdir) if !dbdir.is_empty() => Path::new(dbdir),
        _ => Path::new(DEFAULT_ROOT),
    };

    path.join(hex::encode(db_id.as_slice()))
}

// We don't want to send `MmCtx` across threads, it will only obstruct the normal use case
// (and might result in undefined behaviour if there's a C struct or value in the context that is aliased from the various MM threads).
// Only the `MmArc` is `Send`.
// Also, `MmCtx` not being `Send` allows us to easily keep various C pointers on the context,
// which will likely come useful during the gradual port.
//not-implemented-on-stable// impl !Send for MmCtx {}

pub struct MmArc(pub SharedRc<MmCtx>);

// NB: Explicit `Send` and `Sync` marks here should become unnecessary later,
// after we finish the initial port and replace the C values with the corresponding Rust alternatives.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for MmArc {}
unsafe impl Sync for MmArc {}

impl Clone for MmArc {
    #[track_caller]
    fn clone(&self) -> MmArc { MmArc(self.0.clone()) }
}

impl Deref for MmArc {
    type Target = MmCtx;
    fn deref(&self) -> &MmCtx { &self.0 }
}

#[derive(Clone, Default)]
pub struct MmWeak(WeakRc<MmCtx>);

// Same as `MmArc`.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for MmWeak {}
unsafe impl Sync for MmWeak {}

impl MmWeak {
    /// Create a default MmWeak without allocating any memory.
    pub fn new() -> MmWeak { MmWeak::default() }

    pub fn dropped(&self) -> bool { self.0.strong_count() == 0 }
}

impl fmt::Debug for MmWeak {
    fn fmt(&self, ft: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match MmArc::from_weak(self) {
            Some(ctx) => match ctx.ffi_handle() {
                Ok(ffi_handle) => write!(ft, "MmWeak({})", ffi_handle),
                Err(err) => write!(ft, "MmWeak(ERROR({}))", err),
            },
            None => write!(ft, "MmWeak(-)"),
        }
    }
}

lazy_static! {
    /// A map from a unique context ID to the corresponding MM context, facilitating context access across the FFI boundaries.
    /// NB: The entries are not removed in order to keep the FFI handlers unique.
    pub static ref MM_CTX_FFI: Mutex<HashMap<u32, MmWeak>> = Mutex::new (HashMap::default());
}

impl MmArc {
    pub fn new(ctx: MmCtx) -> MmArc { MmArc(SharedRc::new(ctx)) }

    pub fn stop(&self) -> Result<(), String> {
        try_s!(self.stop.pin(true));

        // Notify shutdown listeners.
        self.graceful_shutdown_registry.abort_all().warn_log();
        // Abort spawned futures.
        self.abortable_system.abort_all().warn_log();

        #[cfg(feature = "track-ctx-pointer")]
        self.track_ctx_pointer();

        Ok(())
    }

    #[cfg(feature = "track-ctx-pointer")]
    fn track_ctx_pointer(&self) {
        let ctx_weak = self.weak();
        let fut = async move {
            let level = log::log_crate::Level::Info;
            loop {
                Timer::sleep(5.).await;
                match MmArc::from_weak(&ctx_weak) {
                    Some(ctx) => ctx.log_existing_pointers(level),
                    None => {
                        log::info!("MmCtx was dropped. Stop the loop");
                        break;
                    },
                }
            }
        };
        self.spawner().spawn(fut);
    }

    #[cfg(feature = "track-ctx-pointer")]
    pub fn log_existing_pointers(&self, level: log::log_crate::Level) { self.0.log_existing_pointers(level, "MmArc") }

    /// Unique context identifier, allowing us to more easily pass the context through the FFI boundaries.
    pub fn ffi_handle(&self) -> Result<u32, String> {
        let mut mm_ctx_ffi = try_s!(MM_CTX_FFI.lock());
        if let Some(have) = self.ffi_handle.as_option() {
            return Ok(*have);
        }
        let mut tries = 0;
        let mut rng = small_rng();
        loop {
            if tries > 999 {
                panic!("MmArc] out of RIDs")
            } else {
                tries += 1
            }
            let rid: u32 = rng.gen();
            if rid == 0 {
                continue;
            }
            match mm_ctx_ffi.entry(rid) {
                Entry::Occupied(_) => continue, // Try another ID.
                Entry::Vacant(ve) => {
                    ve.insert(self.weak());
                    try_s!(self.ffi_handle.pin(rid));
                    return Ok(rid);
                },
            }
        }
    }

    /// Tries getting access to the MM context.  
    /// Fails if an invalid MM context handler is passed (no such context or dropped context).
    #[track_caller]
    pub fn from_ffi_handle(ffi_handle: u32) -> Result<MmArc, String> {
        if ffi_handle == 0 {
            return ERR!("MmArc] Zeroed ffi_handle");
        }
        let mm_ctx_ffi = try_s!(MM_CTX_FFI.lock());
        match mm_ctx_ffi.get(&ffi_handle) {
            Some(weak) => match MmArc::from_weak(weak) {
                Some(ctx) => Ok(ctx),
                None => ERR!("MmArc] ffi_handle {} is dead", ffi_handle),
            },
            None => ERR!("MmArc] ffi_handle {} does not exists", ffi_handle),
        }
    }

    /// Generates a weak pointer, to track the allocated data without prolonging its life.
    pub fn weak(&self) -> MmWeak { MmWeak(SharedRc::downgrade(&self.0)) }

    /// Tries to obtain the MM context from the weak pointer.
    #[track_caller]
    pub fn from_weak(weak: &MmWeak) -> Option<MmArc> { weak.0.upgrade().map(MmArc) }

    /// Init metrics with dashboard.
    pub fn init_metrics(&self) -> Result<(), String> {
        let interval = self.conf["metrics_interval"]
            .as_f64()
            .unwrap_or(EXPORT_METRICS_INTERVAL);

        if interval == 0.0 {
            self.metrics.init();
        } else {
            try_s!(self
                .metrics
                .init_with_dashboard(&self.spawner(), self.log.weak(), interval));
        }

        #[cfg(not(target_arch = "wasm32"))]
        try_s!(self.spawn_prometheus_exporter());

        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn spawn_prometheus_exporter(&self) -> Result<(), MmMetricsError> {
        let prometheusport = match self.conf["prometheusport"].as_u64() {
            Some(port) => port,
            _ => return Ok(()),
        };

        let address: SocketAddr = format!("127.0.0.1:{}", prometheusport)
            .parse()
            .map_err(|e: AddrParseError| MmMetricsError::PrometheusServerError(e.to_string()))?;

        let credentials =
            self.conf["prometheus_credentials"]
                .as_str()
                .map(|userpass| prometheus::PrometheusCredentials {
                    userpass: userpass.into(),
                });

        let shutdown_detector = self
            .graceful_shutdown_registry
            .register_listener()
            .map_err(|e| MmMetricsError::Internal(e.to_string()))?;
        prometheus::spawn_prometheus_exporter(self.metrics.weak(), address, shutdown_detector, credentials)
    }
}

/// The futures spawner pinned to the `MmCtx` context.
/// It's used to spawn futures that can be aborted immediately or after a timeout
/// on the [`MmArc::stop`] function call.
///
/// # Note
///
/// `MmFutSpawner` doesn't prevent the spawned futures from being aborted.
#[derive(Clone)]
pub struct MmFutSpawner {
    inner: WeakSpawner,
}

impl MmFutSpawner {
    pub fn new(system: &AbortableQueue) -> MmFutSpawner {
        MmFutSpawner {
            inner: system.weak_spawner(),
        }
    }
}

impl SpawnFuture for MmFutSpawner {
    fn spawn<F>(&self, f: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.inner.spawn(f)
    }
}

impl SpawnAbortable for MmFutSpawner {
    fn spawn_with_settings<F>(&self, fut: F, settings: AbortSettings)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.inner.spawn_with_settings(fut, settings)
    }
}

/// Helps getting a crate context from a corresponding `MmCtx` field.
///
/// * `ctx_field` - A dedicated crate context field in `MmCtx`, such as the `MmCtx::portfolio_ctx`.
/// * `constructor` - Generates the initial crate context.
pub fn from_ctx<T, C>(
    ctx_field: &Mutex<Option<Arc<dyn Any + 'static + Send + Sync>>>,
    constructor: C,
) -> Result<Arc<T>, String>
where
    C: FnOnce() -> Result<T, String>,
    T: 'static + Send + Sync,
{
    let mut ctx_field = try_s!(ctx_field.lock());
    if let Some(ref ctx) = *ctx_field {
        let ctx: Arc<T> = match ctx.clone().downcast() {
            Ok(p) => p,
            Err(_) => return ERR!("Error casting the context field"),
        };
        return Ok(ctx);
    }
    let arc = Arc::new(try_s!(constructor()));
    *ctx_field = Some(arc.clone());
    Ok(arc)
}

#[derive(Default)]
pub struct MmCtxBuilder {
    conf: Option<Json>,
    log_level: LogLevel,
    version: String,
    datetime: String,
    #[cfg(target_arch = "wasm32")]
    db_namespace: DbNamespaceId,
}

impl MmCtxBuilder {
    pub fn new() -> Self { MmCtxBuilder::default() }

    pub fn with_conf(mut self, conf: Json) -> Self {
        self.conf = Some(conf);
        self
    }

    pub fn with_log_level(mut self, level: LogLevel) -> Self {
        self.log_level = level;
        self
    }

    pub fn with_version(mut self, version: String) -> Self {
        self.version = version;
        self
    }

    pub fn with_datetime(mut self, datetime: String) -> Self {
        self.datetime = datetime;
        self
    }

    #[cfg(target_arch = "wasm32")]
    pub fn with_test_db_namespace(mut self) -> Self {
        self.db_namespace = DbNamespaceId::for_test();
        self
    }

    pub fn into_mm_arc(self) -> MmArc {
        // NB: We avoid recreating LogState
        // in order not to interfere with the integration tests checking LogState drop on shutdown.
        let mut log = if let Some(ref conf) = self.conf {
            LogState::mm(conf)
        } else {
            LogState::in_memory()
        };
        log.set_level(self.log_level);
        let mut ctx = MmCtx::with_log_state(log);
        ctx.mm_version = self.version;
        ctx.datetime = self.datetime;
        if let Some(conf) = self.conf {
            ctx.conf = conf
        }

        #[cfg(target_arch = "wasm32")]
        {
            ctx.db_namespace = self.db_namespace;
        }

        MmArc::new(ctx)
    }
}
