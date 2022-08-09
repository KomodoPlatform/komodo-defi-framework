//! Helpers used in the unit and integration tests.

use gstuff::{try_s, ERR, ERRL};
use http::{HeaderMap, StatusCode};
use lazy_static::lazy_static;
use mm2_number::BigDecimal;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{self as json, json, Value as Json};
use std::collections::HashMap;
use std::net::IpAddr;
use std::num::NonZeroUsize;
use std::process::Child;
use std::sync::Mutex;
use uuid::Uuid;

use common::executor::Timer;
use common::log;
use common::mm_metrics::{MetricType, MetricsJson};
use common::{cfg_native, now_float, now_ms, PagingOptionsEnum};
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};

cfg_native! {
    use common::block_on;
    use common::log::dashboard_path;
    use mm2_io::fs::slurp;
    use mm2_net::transport::slurp_req;
    use common::wio::POOL;
    use chrono::{Local, TimeZone};
    use bytes::Bytes;
    use futures::channel::oneshot;
    use futures::task::SpawnExt;
    use gstuff::ISATTY;
    use http::Request;
    use regex::Regex;
    use std::env;
    use std::fs;
    use std::net::Ipv4Addr;
    use std::path::{Path, PathBuf};
    use std::process::Command;
}

pub const MAKER_SUCCESS_EVENTS: [&str; 11] = [
    "Started",
    "Negotiated",
    "TakerFeeValidated",
    "MakerPaymentSent",
    "TakerPaymentReceived",
    "TakerPaymentWaitConfirmStarted",
    "TakerPaymentValidatedAndConfirmed",
    "TakerPaymentSpent",
    "TakerPaymentSpendConfirmStarted",
    "TakerPaymentSpendConfirmed",
    "Finished",
];

pub const MAKER_ERROR_EVENTS: [&str; 13] = [
    "StartFailed",
    "NegotiateFailed",
    "TakerFeeValidateFailed",
    "MakerPaymentTransactionFailed",
    "MakerPaymentDataSendFailed",
    "MakerPaymentWaitConfirmFailed",
    "TakerPaymentValidateFailed",
    "TakerPaymentWaitConfirmFailed",
    "TakerPaymentSpendFailed",
    "TakerPaymentSpendConfirmFailed",
    "MakerPaymentWaitRefundStarted",
    "MakerPaymentRefunded",
    "MakerPaymentRefundFailed",
];

pub const TAKER_SUCCESS_EVENTS: [&str; 10] = [
    "Started",
    "Negotiated",
    "TakerFeeSent",
    "MakerPaymentReceived",
    "MakerPaymentWaitConfirmStarted",
    "MakerPaymentValidatedAndConfirmed",
    "TakerPaymentSent",
    "TakerPaymentSpent",
    "MakerPaymentSpent",
    "Finished",
];

pub const TAKER_ERROR_EVENTS: [&str; 13] = [
    "StartFailed",
    "NegotiateFailed",
    "TakerFeeSendFailed",
    "MakerPaymentValidateFailed",
    "MakerPaymentWaitConfirmFailed",
    "TakerPaymentTransactionFailed",
    "TakerPaymentWaitConfirmFailed",
    "TakerPaymentDataSendFailed",
    "TakerPaymentWaitForSpendFailed",
    "MakerPaymentSpendFailed",
    "TakerPaymentWaitRefundStarted",
    "TakerPaymentRefunded",
    "TakerPaymentRefundFailed",
];

pub const RICK: &str = "RICK";
pub const RICK_ELECTRUM_ADDRS: &[&str] = &[
    "electrum1.cipig.net:10017",
    "electrum2.cipig.net:10017",
    "electrum3.cipig.net:10017",
];
pub const MORTY: &str = "MORTY";
pub const MORTY_ELECTRUM_ADDRS: &[&str] = &[
    "electrum1.cipig.net:10018",
    "electrum2.cipig.net:10018",
    "electrum3.cipig.net:10018",
];
pub const ZOMBIE_TICKER: &str = "ZOMBIE";
pub const ZOMBIE_ELECTRUMS: &[&str] = &["zombie.sirseven.me:10033"];
pub const ZOMBIE_LIGHTWALLETD_URLS: &[&str] = &["http://zombie.sirseven.me:443"];
const DEFAULT_RPC_PASSWORD: &str = "pass";

pub struct Mm2TestConf {
    pub conf: Json,
    pub rpc_password: String,
    /// This doesn't seem to be really used, so we will possibly remove it soon
    pub local: Option<LocalStart>,
}

impl Mm2TestConf {
    pub fn seednode(passphrase: &str, coins: &Json) -> Self {
        Mm2TestConf {
            conf: json!({
                "gui": "nogui",
                "netid": 9998,
                "passphrase": passphrase,
                "coins": coins,
                "rpc_password": DEFAULT_RPC_PASSWORD,
                "i_am_seed": true,
            }),
            rpc_password: DEFAULT_RPC_PASSWORD.into(),
            local: None,
        }
    }

    pub fn light_node(passphrase: &str, coins: &Json, seednodes: &[&str]) -> Self {
        Mm2TestConf {
            conf: json!({
                "gui": "nogui",
                "netid": 9998,
                "passphrase": passphrase,
                "coins": coins,
                "rpc_password": DEFAULT_RPC_PASSWORD,
                "seednodes": seednodes,
            }),
            rpc_password: DEFAULT_RPC_PASSWORD.into(),
            local: None,
        }
    }

    pub fn no_login_node(coins: &Json, seednodes: &[&str]) -> Self {
        Mm2TestConf {
            conf: json!({
                "gui": "nogui",
                "netid": 9998,
                "coins": coins,
                "rpc_password": DEFAULT_RPC_PASSWORD,
                "seednodes": seednodes,
            }),
            rpc_password: DEFAULT_RPC_PASSWORD.into(),
            local: None,
        }
    }
}

pub fn zombie_conf() -> Json {
    json!({
        "coin":"ZOMBIE",
        "asset":"ZOMBIE",
        "txversion":4,
        "overwintered":1,
        "mm2":1,
        "protocol":{
            "type":"ZHTLC",
            "protocol_data": {
                "consensus_params": {
                    "overwinter_activation_height": 0,
                    "sapling_activation_height": 1,
                    "blossom_activation_height": null,
                    "heartwood_activation_height": null,
                    "canopy_activation_height": null,
                    "coin_type": 133,
                    "hrp_sapling_extended_spending_key": "secret-extended-key-main",
                    "hrp_sapling_extended_full_viewing_key": "zxviews",
                    "hrp_sapling_payment_address": "zs",
                    "b58_pubkey_address_prefix": [ 28, 184 ],
                    "b58_script_address_prefix": [ 28, 189 ]
                }
            }
        },
        "required_confirmations":0
    })
}

pub fn rick_conf() -> Json {
    json!({
        "coin":"RICK",
        "asset":"RICK",
        "required_confirmations":0,
        "txversion":4,
        "overwintered":1,
        "protocol":{
            "type":"UTXO"
        }
    })
}

pub fn morty_conf() -> Json {
    json!({
        "coin":"MORTY",
        "asset":"MORTY",
        "required_confirmations":0,
        "txversion":4,
        "overwintered":1,
        "protocol":{
            "type":"UTXO"
        }
    })
}

#[cfg(target_arch = "wasm32")]
pub fn mm_ctx_with_custom_db() -> MmArc { MmCtxBuilder::new().with_test_db_namespace().into_mm_arc() }

#[cfg(not(target_arch = "wasm32"))]
pub fn mm_ctx_with_custom_db() -> MmArc {
    use db_common::sqlite::rusqlite::Connection;
    use std::sync::Arc;

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let connection = Connection::open_in_memory().unwrap();
    let _ = ctx.sqlite_connection.pin(Arc::new(Mutex::new(connection)));
    ctx
}

/// Automatically kill a wrapped process.
pub struct RaiiKill {
    pub handle: Child,
    running: bool,
}
impl RaiiKill {
    pub fn from_handle(handle: Child) -> RaiiKill { RaiiKill { handle, running: true } }
    pub fn running(&mut self) -> bool {
        if !self.running {
            return false;
        }
        match self.handle.try_wait() {
            Ok(None) => true,
            _ => {
                self.running = false;
                false
            },
        }
    }
}
impl Drop for RaiiKill {
    fn drop(&mut self) {
        // The cached `running` check might provide some protection against killing a wrong process under the same PID,
        // especially if the cached `running` check is also used to monitor the status of the process.
        if self.running() {
            let _ = self.handle.kill();
        }
    }
}

/// When `drop`ped, dumps the given file to the stdout.
///
/// Used in the tests, copying the MM log to the test output.
///
/// Note that because of https://github.com/rust-lang/rust/issues/42474 it's currently impossible to share the MM log interactively,
/// hence we're doing it in the `drop`.
pub struct RaiiDump {
    #[cfg(not(target_arch = "wasm32"))]
    pub log_path: PathBuf,
}
#[cfg(not(target_arch = "wasm32"))]
impl Drop for RaiiDump {
    fn drop(&mut self) {
        use crossterm::execute;
        use crossterm::style::{Color, Print, SetForegroundColor};

        // `term` bypasses the stdout capturing, we should only use it if the capturing was disabled.
        let nocapture = env::args().any(|a| a == "--nocapture");

        let log = slurp(&self.log_path).unwrap();

        // Make sure the log is Unicode.
        // We'll get the "io error when listing tests: Custom { kind: InvalidData, error: StringError("text was not valid unicode") }" otherwise.
        let log = String::from_utf8_lossy(&log);
        let log = log.trim();

        if let (true, true, mut stdout) = (nocapture, *ISATTY, std::io::stdout()) {
            execute!(
                stdout,
                SetForegroundColor(Color::DarkYellow),
                Print(format!("vvv {:?} vvv\n", self.log_path)),
                SetForegroundColor(Color::Yellow),
                Print(log),
            )
            .expect("Printing to stdout failed");
        } else {
            log!("vvv {:?} vvv\n{}", self.log_path, log);
        }
    }
}

lazy_static! {
    /// A singleton with the IPs used by the MarketMakerIt instances created in this session.
    /// The value is set to `false` when the instance is retired.
    static ref MM_IPS: Mutex<HashMap<IpAddr, bool>> = Mutex::new (HashMap::new());
}

#[cfg(not(target_arch = "wasm32"))]
pub type LocalStart = fn(PathBuf, PathBuf, Json);

#[cfg(target_arch = "wasm32")]
pub type LocalStart = fn(MmArc);

/// An instance of a MarketMaker process started by and for an integration test.
/// Given that [in CI] the tests are executed before the build, the binary of that process is the tests binary.
#[cfg(not(target_arch = "wasm32"))]
pub struct MarketMakerIt {
    /// The MarketMaker's current folder where it will try to open the database files.
    pub folder: PathBuf,
    /// Unique (to run multiple instances) IP, like "127.0.0.$x".
    pub ip: IpAddr,
    /// The file we redirected the standard output and error streams to.
    pub log_path: PathBuf,
    /// The PID of the MarketMaker process.
    pub pc: Option<RaiiKill>,
    /// RPC API key.
    pub userpass: String,
}

/// A MarketMaker instance started by and for an integration test.
#[cfg(target_arch = "wasm32")]
pub struct MarketMakerIt {
    pub ctx: mm2_core::mm_ctx::MmArc,
    /// RPC API key.
    pub userpass: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl std::fmt::Debug for MarketMakerIt {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "MarketMakerIt {{ folder: {:?}, ip: {}, log_path: {:?}, userpass: {} }}",
            self.folder, self.ip, self.log_path, self.userpass
        )
    }
}

impl MarketMakerIt {
    /// Start a new MarketMaker node without any specific environment variables.
    /// For more information see [`MarketMakerIt::start_with_envs`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start(conf: Json, userpass: String, local: Option<LocalStart>) -> Result<MarketMakerIt, String> {
        block_on(MarketMakerIt::start_with_envs(conf, userpass, local, &[]))
    }

    /// Start a new MarketMaker node asynchronously without any specific environment variables.
    /// For more information see [`MarketMakerIt::start_with_envs`].
    pub async fn start_async(conf: Json, userpass: String, local: Option<LocalStart>) -> Result<MarketMakerIt, String> {
        MarketMakerIt::start_with_envs(conf, userpass, local, &[]).await
    }

    /// Create a new temporary directory and start a new MarketMaker process there.
    ///
    /// * `conf` - The command-line configuration passed to the MarketMaker.
    ///            Unique local IP address is injected as "myipaddr" unless this field is already present.
    /// * `userpass` - RPC API key. We should probably extract it automatically from the MM log.
    /// * `local` - Function to start the MarketMaker in a local thread, instead of spawning a process.
    /// * `envs` - The enviroment variables passed to the process
    /// It's required to manually add 127.0.0.* IPs aliases on Mac to make it properly work.
    /// cf. https://superuser.com/a/458877, https://superuser.com/a/635327
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn start_with_envs(
        mut conf: Json,
        userpass: String,
        local: Option<LocalStart>,
        envs: &[(&str, &str)],
    ) -> Result<MarketMakerIt, String> {
        conf["allow_weak_password"] = true.into();
        let ip = try_s!(Self::myipaddr_from_conf(&mut conf));
        let folder = new_mm2_temp_folder_path(Some(ip));
        let db_dir = match conf["dbdir"].as_str() {
            Some(path) => path.into(),
            None => {
                let dir = folder.join("DB");
                conf["dbdir"] = dir.to_str().unwrap().into();
                dir
            },
        };

        try_s!(fs::create_dir(&folder));
        match fs::create_dir(db_dir) {
            Ok(_) => (),
            Err(ref ie) if ie.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return ERR!("{}", e),
        };
        let log_path = match conf["log"].as_str() {
            Some(path) => path.into(),
            None => {
                let path = folder.join("mm2.log");
                conf["log"] = path.to_str().unwrap().into();
                path
            },
        };

        // If `local` is provided
        // then instead of spawning a process we start the MarketMaker in a local thread,
        // allowing us to easily *debug* the tested MarketMaker code.
        // Note that this should only be used while running a single test,
        // using this option while running multiple tests (or multiple MarketMaker instances) is currently UB.
        let pc = if let Some(local) = local {
            local(folder.clone(), log_path.clone(), conf.clone());
            None
        } else {
            let executable = try_s!(env::args().next().ok_or("No program name"));
            let executable = try_s!(Path::new(&executable).canonicalize());
            let log = try_s!(fs::File::create(&log_path));
            let child = try_s!(Command::new(&executable)
                .arg("test_mm_start")
                .arg("--nocapture")
                .current_dir(&folder)
                .env("_MM2_TEST_CONF", try_s!(json::to_string(&conf)))
                .env("MM2_UNBUFFERED_OUTPUT", "1")
                .env("RUST_LOG", "debug")
                .envs(envs.to_vec())
                .stdout(try_s!(log.try_clone()))
                .stderr(log)
                .spawn());
            Some(RaiiKill::from_handle(child))
        };

        let mut mm = MarketMakerIt {
            folder,
            ip,
            log_path,
            pc,
            userpass,
        };

        try_s!(mm.startup_checks(&conf).await);
        Ok(mm)
    }

    /// Start a new MarketMaker locally.
    ///
    /// * `conf` - The command-line configuration passed to the MarketMaker.
    ///            Unique P2P in-memory port is injected as `p2p_in_memory_port` unless this field is already present.
    /// * `userpass` - RPC API key. We should probably extract it automatically from the MM log.
    /// * `local` - Function to start the MarketMaker locally. Required for nodes running in a browser.
    /// * `envs` - The enviroment variables passed to the process.
    ///            The argument is ignore for nodes running in a browser.
    #[cfg(target_arch = "wasm32")]
    pub async fn start_with_envs(
        mut conf: Json,
        userpass: String,
        local: Option<LocalStart>,
        _envs: &[(&str, &str)],
    ) -> Result<MarketMakerIt, String> {
        if conf["p2p_in_memory"].is_null() {
            conf["p2p_in_memory"] = Json::Bool(true);
        }

        let i_am_seed = conf["i_am_seed"].as_bool().unwrap_or_default();
        let p2p_in_memory_port_missed = conf["p2p_in_memory_port"].is_null();
        if i_am_seed && p2p_in_memory_port_missed {
            let mut rng = common::small_rng();
            let new_p2p_port: u64 = rng.gen();

            log!("Set 'p2p_in_memory_port' to {:?}", new_p2p_port);
            conf["p2p_in_memory_port"] = Json::Number(new_p2p_port.into());
        }

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::new()
            .with_conf(conf.clone())
            .with_test_db_namespace()
            .into_mm_arc();
        let local = try_s!(local.ok_or("!local"));
        local(ctx.clone());

        let mut mm = MarketMakerIt { ctx, userpass };
        try_s!(mm.startup_checks(&conf).await);
        Ok(mm)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn log_as_utf8(&self) -> Result<String, String> {
        let mm_log = try_s!(slurp(&self.log_path));
        let mm_log = unsafe { String::from_utf8_unchecked(mm_log) };
        Ok(mm_log)
    }

    /// Busy-wait on the log until the `pred` returns `true` or `timeout_sec` expires.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn wait_for_log<F>(&mut self, timeout_sec: f64, pred: F) -> Result<(), String>
    where
        F: Fn(&str) -> bool,
    {
        let start = now_float();
        let ms = 50.min((timeout_sec * 1000.) as u64 / 20 + 10);
        loop {
            let mm_log = try_s!(self.log_as_utf8());
            if pred(&mm_log) {
                return Ok(());
            }
            if now_float() - start > timeout_sec {
                return ERR!("Timeout expired waiting for a log condition");
            }
            if let Some(ref mut pc) = self.pc {
                if !pc.running() {
                    return ERR!("MM process terminated prematurely at: {:?}.", self.folder);
                }
            }
            Timer::sleep(ms as f64 / 1000.).await
        }
    }

    /// Busy-wait on the log until the `pred` returns `true` or `timeout_sec` expires.
    /// The difference from standard wait_for_log is this function keeps working
    /// after process is stopped
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn wait_for_log_after_stop<F>(&mut self, timeout_sec: f64, pred: F) -> Result<(), String>
    where
        F: Fn(&str) -> bool,
    {
        let start = now_float();
        let ms = 50.min((timeout_sec * 1000.) as u64 / 20 + 10);
        loop {
            let mm_log = try_s!(self.log_as_utf8());
            if pred(&mm_log) {
                return Ok(());
            }
            if now_float() - start > timeout_sec {
                return ERR!("Timeout expired waiting for a log condition");
            }
            Timer::sleep(ms as f64 / 1000.).await
        }
    }

    /// Busy-wait on the instance in-memory log until the `pred` returns `true` or `timeout_sec` expires.
    #[cfg(target_arch = "wasm32")]
    pub async fn wait_for_log<F>(&mut self, timeout_sec: f64, pred: F) -> Result<(), String>
    where
        F: Fn(&str) -> bool,
    {
        wait_for_log(&self.ctx, timeout_sec, pred).await
    }

    /// Invokes the locally running MM and returns its reply.
    #[cfg(target_arch = "wasm32")]
    pub async fn rpc(&self, payload: &Json) -> Result<(StatusCode, String, HeaderMap), String> {
        let wasm_rpc = self
            .ctx
            .wasm_rpc
            .as_option()
            .expect("'MmCtx::rpc' must be initialized already");
        match wasm_rpc.request(payload.clone()).await {
            // Please note a new type of error will be introduced soon.
            Ok(body) => {
                let status_code = if body["error"].is_null() {
                    StatusCode::OK
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                let body_str = json::to_string(&body).expect(&format!("Response {:?} is not a valid JSON", body));
                Ok((status_code, body_str, HeaderMap::new()))
            },
            Err(e) => Ok((StatusCode::INTERNAL_SERVER_ERROR, e, HeaderMap::new())),
        }
    }

    /// Invokes the locally running MM and returns its reply.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn rpc(&self, payload: &Json) -> Result<(StatusCode, String, HeaderMap), String> {
        let uri = format!("http://{}:7783", self.ip);
        log!("sending rpc request {} to {}", json::to_string(payload).unwrap(), uri);

        let payload = try_s!(json::to_vec(payload));
        let request = try_s!(Request::builder().method("POST").uri(uri).body(payload));

        let (status, headers, body) = try_s!(slurp_req(request).await);
        Ok((status, try_s!(std::str::from_utf8(&body)).trim().into(), headers))
    }

    /// Sends the &str payload to the locally running MM and returns it's reply.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn rpc_str(&self, payload: &'static str) -> Result<(StatusCode, String, HeaderMap), String> {
        let uri = format!("http://{}:7783", self.ip);
        let request = try_s!(Request::builder().method("POST").uri(uri).body(payload.into()));
        let (status, headers, body) = try_s!(block_on(slurp_req(request)));
        Ok((status, try_s!(std::str::from_utf8(&body)).trim().into(), headers))
    }

    #[cfg(target_arch = "wasm32")]
    pub fn rpc_str(&self, _payload: &'static str) -> Result<(StatusCode, String, HeaderMap), String> {
        unimplemented!()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn mm_dump(&self) -> (RaiiDump, RaiiDump) { mm_dump(&self.log_path) }

    #[cfg(target_arch = "wasm32")]
    pub fn mm_dump(&self) -> (RaiiDump, RaiiDump) { (RaiiDump {}, RaiiDump {}) }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn my_seed_addr(&self) -> String { format!("{}", self.ip) }

    /// # Panic
    ///
    /// Panic if this instance is not a seed.
    #[cfg(target_arch = "wasm32")]
    pub fn my_seed_addr(&self) -> String {
        let p2p_port = self
            .ctx
            .p2p_in_memory_port()
            .expect("This instance is not a seed, so 'p2p_in_memory_port' is None");
        format!("/memory/{}", p2p_port)
    }

    /// Send the "stop" request to the locally running MM.
    pub async fn stop(&self) -> Result<(), String> {
        let (status, body, _headers) = match self.rpc(&json! ({"userpass": self.userpass, "method": "stop"})).await {
            Ok(t) => t,
            Err(err) => {
                // Downgrade the known errors into log warnings,
                // in order not to spam the unit test logs with confusing panics, obscuring the real issue.
                if err.contains("An existing connection was forcibly closed by the remote host") {
                    log!("stop] MM already down? {}", err);
                    return Ok(());
                } else {
                    return ERR!("{}", err);
                }
            },
        };
        if status != StatusCode::OK {
            return ERR!("MM didn't accept a stop. body: {}", body);
        }
        Ok(())
    }

    /// Currently, we cannot wait for the `Completed IAmrelay handling for peer` log entry on WASM node,
    /// because the P2P module logs to a global logger and doesn't log to the dashboard.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn check_seednodes(&mut self) -> Result<(), String> {
        // wait for at least 1 node to be added to relay mesh
        self.wait_for_log(22., |log| log.contains("Completed IAmrelay handling for peer"))
            .await
            .map_err(|e| ERRL!("{}", e))
    }

    /// Wait for the node to start listening to new P2P connections.
    /// Please note the node is expected to be a seed.
    ///
    /// Currently, we cannot wait for the `INFO Listening on` log entry on WASM node,
    /// because the P2P module logs to a global logger and doesn't log to the dashboard.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn wait_for_p2p_listen(&mut self) -> Result<(), String> {
        self.wait_for_log(22., |log| log.contains("INFO Listening on"))
            .await
            .map_err(|e| ERRL!("{}", e))
    }

    /// Wait for the RPC to be up.
    pub async fn wait_for_rpc_is_up(&mut self) -> Result<(), String> {
        self.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
            .await
            .map_err(|e| ERRL!("{}", e))
    }

    async fn startup_checks(&mut self, conf: &Json) -> Result<(), String> {
        let skip_startup_checks = conf["skip_startup_checks"].as_bool().unwrap_or_default();
        if skip_startup_checks {
            return Ok(());
        }

        try_s!(self.wait_for_rpc_is_up().await);

        #[cfg(not(target_arch = "wasm32"))]
        {
            let is_seed = conf["i_am_seed"].as_bool().unwrap_or_default();
            if is_seed {
                try_s!(self.wait_for_p2p_listen().await);
            }

            let skip_seednodes_check = conf["skip_seednodes_check"].as_bool().unwrap_or_default();
            if conf["seednodes"].as_array().is_some() && !skip_seednodes_check {
                try_s!(self.check_seednodes().await);
            }
        }

        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn myipaddr_from_conf(conf: &mut Json) -> Result<IpAddr, String> {
        if conf["myipaddr"].is_null() {
            // Generate an unique IP.
            let mut attempts = 0;
            let mut rng = common::small_rng();
            loop {
                if attempts > 128 {
                    return ERR!("Out of local IPs?");
                }
                let ip4 = Ipv4Addr::new(127, 0, 0, rng.gen_range(1, 255));
                let ip = IpAddr::from(ip4);
                let mut mm_ips = try_s!(MM_IPS.lock());
                if mm_ips.contains_key(&ip) {
                    attempts += 1;
                    continue;
                }
                mm_ips.insert(ip, true);
                conf["myipaddr"] = format!("{}", ip).into();
                conf["rpcip"] = format!("{}", ip).into();
                return Ok(ip);
            }
        }

        // Just use the IP given in the `conf`.

        let ip: IpAddr = try_s!(try_s!(conf["myipaddr"].as_str().ok_or("myipaddr is not a string")).parse());
        let mut mm_ips = try_s!(MM_IPS.lock());
        if mm_ips.contains_key(&ip) {
            log!("MarketMakerIt] Warning, IP {} was already used.", ip)
        }
        mm_ips.insert(ip, true);
        Ok(ip)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for MarketMakerIt {
    fn drop(&mut self) {
        if let Ok(mut mm_ips) = MM_IPS.lock() {
            mm_ips.remove(&self.ip);
        } else {
            log!("MarketMakerIt] Can't lock MM_IPS.")
        }
    }
}

/// Busy-wait on the log until the `pred` returns `true` or `timeout_sec` expires.
pub async fn wait_for_log<F>(ctx: &MmArc, timeout_sec: f64, pred: F) -> Result<(), String>
where
    F: Fn(&str) -> bool,
{
    let start = now_float();
    let ms = 50.min((timeout_sec * 1000.) as u32 / 20 + 10);
    let mut buf = String::with_capacity(128);
    let mut found = false;
    loop {
        ctx.log.with_tail(&mut |tail| {
            for en in tail {
                if en.format(&mut buf).is_ok() && pred(&buf) {
                    found = true;
                    break;
                }
            }
        });
        if found {
            return Ok(());
        }

        ctx.log.with_gravity_tail(&mut |tail| {
            for chunk in tail {
                if pred(chunk) {
                    found = true;
                    break;
                }
            }
        });
        if found {
            return Ok(());
        }

        if now_float() - start > timeout_sec {
            return ERR!("Timeout expired waiting for a log condition");
        }
        Timer::sleep_ms(ms).await;
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct ToWaitForLogRe {
    ctx: u32,
    timeout_sec: f64,
    re_pred: String,
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn common_wait_for_log_re(req: Bytes) -> Result<Vec<u8>, String> {
    let args: ToWaitForLogRe = try_s!(json::from_slice(&req));
    let ctx = try_s!(MmArc::from_ffi_handle(args.ctx));
    let re = try_s!(Regex::new(&args.re_pred));

    // Run the blocking `wait_for_log` in the `POOL`.
    let (tx, rx) = oneshot::channel();
    try_s!(try_s!(POOL.lock()).spawn(async move {
        let res = wait_for_log(&ctx, args.timeout_sec, |line| re.is_match(line)).await;
        let _ = tx.send(res);
    }));
    try_s!(try_s!(rx.await));

    Ok(Vec::new())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn wait_for_log_re(ctx: &MmArc, timeout_sec: f64, re_pred: &str) -> Result<(), String> {
    let re = try_s!(Regex::new(re_pred));
    wait_for_log(ctx, timeout_sec, |line| re.is_match(line)).await
}

/// Create RAII variables to the effect of dumping the log and the status dashboard at the end of the scope.
#[cfg(not(target_arch = "wasm32"))]
pub fn mm_dump(log_path: &Path) -> (RaiiDump, RaiiDump) {
    (
        RaiiDump {
            log_path: log_path.to_path_buf(),
        },
        RaiiDump {
            log_path: dashboard_path(log_path).unwrap(),
        },
    )
}

/// A typical MM instance.
#[cfg(not(target_arch = "wasm32"))]
pub fn mm_spat(
    local_start: LocalStart,
    conf_mod: &dyn Fn(Json) -> Json,
) -> (&'static str, MarketMakerIt, RaiiDump, RaiiDump) {
    let passphrase = "SPATsRps3dhEtXwtnpRCKF";
    let mm = MarketMakerIt::start(
        conf_mod(json! ({
            "gui": "nogui",
            "passphrase": passphrase,
            "rpccors": "http://localhost:4000",
            "coins": [
                {"coin":"RICK","asset":"RICK","rpcport":8923},
                {"coin":"MORTY","asset":"MORTY","rpcport":11608},
            ],
            "i_am_seed": true,
            "rpc_password": "pass",
        })),
        "pass".into(),
        match common::var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "1" => Some(local_start),
            _ => None,
        },
    )
    .unwrap();
    let (dump_log, dump_dashboard) = mm_dump(&mm.log_path);
    (passphrase, mm, dump_log, dump_dashboard)
}

#[cfg(target_arch = "wasm32")]
pub fn mm_spat(
    _local_start: LocalStart,
    _conf_mod: &dyn Fn(Json) -> Json,
) -> (&'static str, MarketMakerIt, RaiiDump, RaiiDump) {
    unimplemented!()
}

/// Asks MM to enable the given currency in electrum mode
/// fresh list of servers at https://github.com/jl777/coins/blob/master/electrums/.
pub async fn enable_electrum(mm: &MarketMakerIt, coin: &str, tx_history: bool, urls: &[&str]) -> Json {
    let servers = urls.iter().map(|url| json!({ "url": url })).collect();
    enable_electrum_json(mm, coin, tx_history, servers).await
}

/// Asks MM to enable the given currency in electrum mode
/// fresh list of servers at https://github.com/jl777/coins/blob/master/electrums/.
pub async fn enable_electrum_json(mm: &MarketMakerIt, coin: &str, tx_history: bool, servers: Vec<Json>) -> Json {
    let electrum = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "electrum",
            "coin": coin,
            "servers": servers,
            "mm2": 1,
            "tx_history": tx_history,
        }))
        .await
        .unwrap();
    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with {} {}",
        electrum.0,
        electrum.1
    );
    json::from_str(&electrum.1).unwrap()
}

pub async fn enable_qrc20(mm: &MarketMakerIt, coin: &str, urls: &[&str], swap_contract_address: &str) -> Json {
    let servers: Vec<_> = urls.iter().map(|url| json!({ "url": url })).collect();
    let electrum = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "electrum",
            "coin": coin,
            "servers": servers,
            "mm2": 1,
            "swap_contract_address": swap_contract_address,
        }))
        .await
        .unwrap();
    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with {} {}",
        electrum.0,
        electrum.1
    );
    json::from_str(&electrum.1).unwrap()
}

/// Reads passphrase and userpass from .env file
pub fn from_env_file(env: Vec<u8>) -> (Option<String>, Option<String>) {
    use regex::bytes::Regex;
    let (mut passphrase, mut userpass) = (None, None);
    for cap in Regex::new(r"(?m)^(PASSPHRASE|USERPASS)=(\w[\w ]+)$")
        .unwrap()
        .captures_iter(&env)
    {
        match cap.get(1) {
            Some(name) if name.as_bytes() == b"PASSPHRASE" => {
                passphrase = cap.get(2).map(|v| String::from_utf8(v.as_bytes().into()).unwrap())
            },
            Some(name) if name.as_bytes() == b"USERPASS" => {
                userpass = cap.get(2).map(|v| String::from_utf8(v.as_bytes().into()).unwrap())
            },
            _ => (),
        }
    }
    (passphrase, userpass)
}

#[macro_export]
#[cfg(target_arch = "wasm32")]
macro_rules! get_passphrase {
    ($_env_file:literal, $env:literal) => {
        option_env!($env).ok_or_else(|| ERRL!("No such '{}' environment variable", $env))
    };
}

#[macro_export]
#[cfg(not(target_arch = "wasm32"))]
macro_rules! get_passphrase {
    ($env_file:literal, $env:literal) => {
        $crate::for_tests::get_passphrase(&$env_file, $env)
    };
}

/// Reads passphrase from file or environment.
#[cfg(not(target_arch = "wasm32"))]
pub fn get_passphrase(path: &dyn AsRef<Path>, env: &str) -> Result<String, String> {
    if let (Some(file_passphrase), _file_userpass) = from_env_file(try_s!(slurp(path))) {
        return Ok(file_passphrase);
    }

    if let Ok(v) = common::var(env) {
        Ok(v)
    } else {
        ERR!("No {} or {}", env, path.as_ref().display())
    }
}

/// Asks MM to enable the given currency in native mode.
/// Returns the RPC reply containing the corresponding wallet address.
pub async fn enable_native(mm: &MarketMakerIt, coin: &str, urls: &[&str]) -> Json {
    let native = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "enable",
            "coin": coin,
            "urls": urls,
            // Dev chain swap contract address
            "swap_contract_address": "0xa09ad3cd7e96586ebd05a2607ee56b56fb2db8fd",
            "mm2": 1,
        }))
        .await
        .unwrap();
    assert_eq!(native.0, StatusCode::OK, "'enable' failed: {}", native.1);
    json::from_str(&native.1).unwrap()
}

pub async fn enable_spl(mm: &MarketMakerIt, coin: &str) -> Json {
    let req = json!({
        "userpass": mm.userpass,
        "method": "enable_spl",
        "mmrpc": "2.0",
        "params": {
            "ticker": coin,
            "activation_params": {}
        }
    });
    let enable = mm.rpc(&req).await.unwrap();
    assert_eq!(enable.0, StatusCode::OK, "'enable_spl' failed: {}", enable.1);
    json::from_str(&enable.1).unwrap()
}

pub async fn enable_slp(mm: &MarketMakerIt, coin: &str) -> Json {
    let enable = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "enable_slp",
            "mmrpc": "2.0",
            "params": {
                "ticker": coin,
                "activation_params": {}
            }
        }))
        .await
        .unwrap();
    assert_eq!(enable.0, StatusCode::OK, "'enable_slp' failed: {}", enable.1);
    json::from_str(&enable.1).unwrap()
}

#[derive(Serialize)]
pub struct ElectrumRpcRequest {
    pub url: String,
}

#[derive(Serialize)]
#[serde(tag = "rpc", content = "rpc_data")]
pub enum UtxoRpcMode {
    Native,
    Electrum { servers: Vec<ElectrumRpcRequest> },
}

fn electrum_servers_rpc(servers: &[&str]) -> Vec<ElectrumRpcRequest> {
    servers
        .iter()
        .map(|url| ElectrumRpcRequest { url: url.to_string() })
        .collect()
}

impl UtxoRpcMode {
    pub fn electrum(servers: &[&str]) -> Self {
        UtxoRpcMode::Electrum {
            servers: electrum_servers_rpc(servers),
        }
    }
}

pub async fn enable_bch_with_tokens(
    mm: &MarketMakerIt,
    platform_coin: &str,
    tokens: &[&str],
    mode: UtxoRpcMode,
    tx_history: bool,
) -> Json {
    let slp_requests: Vec<_> = tokens.iter().map(|ticker| json!({ "ticker": ticker })).collect();

    let enable = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "enable_bch_with_tokens",
            "mmrpc": "2.0",
            "params": {
                "ticker": platform_coin,
                "allow_slp_unsafe_conf": true,
                "bchd_urls": [],
                "mode": mode,
                "tx_history": tx_history,
                "slp_tokens_requests": slp_requests,
            }
        }))
        .await
        .unwrap();
    assert_eq!(
        enable.0,
        StatusCode::OK,
        "'enable_bch_with_tokens' failed: {}",
        enable.1
    );
    json::from_str(&enable.1).unwrap()
}

pub async fn enable_solana_with_tokens(
    mm: &MarketMakerIt,
    platform_coin: &str,
    tokens: &[&str],
    solana_client_url: &str,
    tx_history: bool,
) -> Json {
    let spl_requests: Vec<_> = tokens.iter().map(|ticker| json!({ "ticker": ticker })).collect();
    let req = json! ({
        "userpass": mm.userpass,
        "method": "enable_solana_with_tokens",
        "mmrpc": "2.0",
        "params": {
            "ticker": platform_coin,
            "confirmation_commitment": "finalized",
            "allow_slp_unsafe_conf": true,
            "client_url": solana_client_url,
            "tx_history": tx_history,
            "spl_tokens_requests": spl_requests,
        }
    });

    let enable = mm.rpc(&req).await.unwrap();
    assert_eq!(
        enable.0,
        StatusCode::OK,
        "'enable_bch_with_tokens' failed: {}",
        enable.1
    );
    json::from_str(&enable.1).unwrap()
}

pub async fn my_tx_history_v2(
    mm: &MarketMakerIt,
    coin: &str,
    limit: usize,
    paging: Option<PagingOptionsEnum<String>>,
) -> Json {
    let paging = paging.unwrap_or(PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap()));
    let request = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "my_tx_history",
            "mmrpc": "2.0",
            "params": {
                "coin": coin,
                "limit": limit,
                "paging_options": paging,
            }
        }))
        .await
        .unwrap();
    assert_eq!(request.0, StatusCode::OK, "'my_tx_history' failed: {}", request.1);
    json::from_str(&request.1).unwrap()
}

pub async fn z_coin_tx_history(
    mm: &MarketMakerIt,
    coin: &str,
    limit: usize,
    paging: Option<PagingOptionsEnum<i64>>,
) -> Json {
    let paging = paging.unwrap_or_default();
    let request = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "z_coin_tx_history",
            "mmrpc": "2.0",
            "params": {
                "coin": coin,
                "limit": limit,
                "paging_options": paging,
            }
        }))
        .await
        .unwrap();
    assert_eq!(request.0, StatusCode::OK, "'z_coin_tx_history' failed: {}", request.1);
    json::from_str(&request.1).unwrap()
}

pub async fn enable_native_bch(mm: &MarketMakerIt, coin: &str, bchd_urls: &[&str]) -> Json {
    let native = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "enable",
            "coin": coin,
            "bchd_urls": bchd_urls,
            "allow_slp_unsafe_conf": true,
            "mm2": 1,
        }))
        .await
        .unwrap();
    assert_eq!(native.0, StatusCode::OK, "'enable' failed: {}", native.1);
    json::from_str(&native.1).unwrap()
}

pub async fn enable_lightning(mm: &MarketMakerIt, coin: &str) -> Json {
    let enable = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "enable_lightning",
            "mmrpc": "2.0",
            "params": {
                "ticker": coin,
                "activation_params": {
                    "name": "test-node"
                }
            }
        }))
        .await
        .unwrap();
    assert_eq!(enable.0, StatusCode::OK, "'enable_lightning' failed: {}", enable.1);
    json::from_str(&enable.1).unwrap()
}

/// Use a separate (unique) temporary folder for each MM.
/// We could also remove the old folders after some time in order not to spam the temporary folder.
/// Though we don't always want to remove them right away, allowing developers to check the files).
/// Appends IpAddr if it is pre-known
#[cfg(not(target_arch = "wasm32"))]
pub fn new_mm2_temp_folder_path(ip: Option<IpAddr>) -> PathBuf {
    let now = common::now_ms();
    let now = Local.timestamp((now / 1000) as i64, (now % 1000) as u32 * 1_000_000);
    let folder = match ip {
        Some(ip) => format!("mm2_{}_{}", now.format("%Y-%m-%d_%H-%M-%S-%3f"), ip),
        None => format!("mm2_{}", now.format("%Y-%m-%d_%H-%M-%S-%3f")),
    };
    common::temp_dir().join(folder)
}

pub fn find_metrics_in_json(
    metrics: MetricsJson,
    search_key: &str,
    search_labels: &[(&str, &str)],
) -> Option<MetricType> {
    metrics.metrics.into_iter().find(|metric| {
        let (key, labels) = match metric {
            MetricType::Counter { key, labels, .. } => (key, labels),
            _ => return false,
        };

        if key != search_key {
            return false;
        }

        for (s_label_key, s_label_value) in search_labels.iter() {
            let label_value = match labels.get(&(*s_label_key).to_string()) {
                Some(x) => x,
                _ => return false,
            };

            if s_label_value != label_value {
                return false;
            }
        }

        true
    })
}

/// Helper function requesting my swap status and checking it's events
pub async fn check_my_swap_status(
    mm: &MarketMakerIt,
    uuid: &str,
    expected_success_events: &[&str],
    expected_error_events: &[&str],
    maker_amount: BigDecimal,
    taker_amount: BigDecimal,
) {
    let response = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "my_swap_status",
            "params": {
                "uuid": uuid,
            }
        }))
        .await
        .unwrap();
    assert!(response.0.is_success(), "!status of {}: {}", uuid, response.1);
    let status_response: Json = json::from_str(&response.1).unwrap();
    let success_events: Vec<String> = json::from_value(status_response["result"]["success_events"].clone()).unwrap();
    assert_eq!(expected_success_events, success_events.as_slice());
    let error_events: Vec<String> = json::from_value(status_response["result"]["error_events"].clone()).unwrap();
    assert_eq!(expected_error_events, error_events.as_slice());

    let events_array = status_response["result"]["events"].as_array().unwrap();
    let actual_maker_amount = json::from_value(events_array[0]["event"]["data"]["maker_amount"].clone()).unwrap();
    assert_eq!(maker_amount, actual_maker_amount);
    let actual_taker_amount = json::from_value(events_array[0]["event"]["data"]["taker_amount"].clone()).unwrap();
    assert_eq!(taker_amount, actual_taker_amount);
    let actual_events = events_array.iter().map(|item| item["event"]["type"].as_str().unwrap());
    let actual_events: Vec<&str> = actual_events.collect();
    assert_eq!(expected_success_events, actual_events.as_slice());
}

pub async fn check_my_swap_status_amounts(
    mm: &MarketMakerIt,
    uuid: Uuid,
    maker_amount: BigDecimal,
    taker_amount: BigDecimal,
) {
    let response = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "my_swap_status",
            "params": {
                "uuid": uuid,
            }
        }))
        .await
        .unwrap();
    assert!(response.0.is_success(), "!status of {}: {}", uuid, response.1);
    let status_response: Json = json::from_str(&response.1).unwrap();

    let events_array = status_response["result"]["events"].as_array().unwrap();
    let actual_maker_amount = json::from_value(events_array[0]["event"]["data"]["maker_amount"].clone()).unwrap();
    assert_eq!(maker_amount, actual_maker_amount);
    let actual_taker_amount = json::from_value(events_array[0]["event"]["data"]["taker_amount"].clone()).unwrap();
    assert_eq!(taker_amount, actual_taker_amount);
}

pub async fn check_stats_swap_status(
    mm: &MarketMakerIt,
    uuid: &str,
    maker_expected_events: &[&str],
    taker_expected_events: &[&str],
) {
    let response = mm
        .rpc(&json! ({
            "method": "stats_swap_status",
            "params": {
                "uuid": uuid,
            }
        }))
        .await
        .unwrap();
    assert!(response.0.is_success(), "!status of {}: {}", uuid, response.1);
    let status_response: Json = json::from_str(&response.1).unwrap();
    let maker_events_array = status_response["result"]["maker"]["events"].as_array().unwrap();
    let taker_events_array = status_response["result"]["taker"]["events"].as_array().unwrap();
    let maker_actual_events = maker_events_array
        .iter()
        .map(|item| item["event"]["type"].as_str().unwrap());
    let maker_actual_events: Vec<&str> = maker_actual_events.collect();
    let taker_actual_events = taker_events_array
        .iter()
        .map(|item| item["event"]["type"].as_str().unwrap());
    let taker_actual_events: Vec<&str> = taker_actual_events.collect();
    assert_eq!(maker_expected_events, maker_actual_events.as_slice());
    assert_eq!(taker_expected_events, taker_actual_events.as_slice());
}

pub async fn check_recent_swaps(mm: &MarketMakerIt, expected_len: usize) {
    let response = mm
        .rpc(&json! ({
            "method": "my_recent_swaps",
            "userpass": mm.userpass,
        }))
        .await
        .unwrap();
    assert!(response.0.is_success(), "!status of my_recent_swaps {}", response.1);
    let swaps_response: Json = json::from_str(&response.1).unwrap();
    let swaps: &Vec<Json> = swaps_response["result"]["swaps"].as_array().unwrap();
    assert_eq!(expected_len, swaps.len());
}

pub async fn wait_till_history_has_records(mm: &MarketMakerIt, coin: &str, expected_len: usize) {
    // give 2 second max to fetch a single transaction
    let to_wait = expected_len as u64 * 2;
    let wait_until = now_ms() + to_wait * 1000;
    loop {
        let tx_history = mm
            .rpc(&json!({
                "userpass": mm.userpass,
                "method": "my_tx_history",
                "coin": coin,
                "limit": 100,
            }))
            .await
            .unwrap();
        assert_eq!(
            tx_history.0,
            StatusCode::OK,
            "RPC «my_tx_history» failed with status «{}», response «{}»",
            tx_history.0,
            tx_history.1
        );
        log!("{:?}", tx_history.1);
        let tx_history_json: Json = json::from_str(&tx_history.1).unwrap();
        if tx_history_json["result"]["transactions"].as_array().unwrap().len() >= expected_len {
            break;
        }

        Timer::sleep(1.).await;
        assert!(now_ms() <= wait_until, "wait_till_history_has_records timed out");
    }
}

pub async fn orderbook_v2(mm: &MarketMakerIt, base: &str, rel: &str) -> Json {
    let request = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "orderbook",
            "mmrpc": "2.0",
            "params": {
                "base": base,
                "rel": rel,
            }
        }))
        .await
        .unwrap();
    assert_eq!(request.0, StatusCode::OK, "'orderbook' failed: {}", request.1);
    json::from_str(&request.1).unwrap()
}

pub async fn best_orders_v2(mm: &MarketMakerIt, coin: &str, action: &str, volume: &str) -> Json {
    let request = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "best_orders",
            "mmrpc": "2.0",
            "params": {
                "coin": coin,
                "action": action,
                "volume": volume,
            }
        }))
        .await
        .unwrap();
    assert_eq!(request.0, StatusCode::OK, "'best_orders' failed: {}", request.1);
    json::from_str(&request.1).unwrap()
}

pub async fn init_withdraw(mm: &MarketMakerIt, coin: &str, to: &str, amount: &str) -> Json {
    let request = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "init_withdraw",
            "mmrpc": "2.0",
            "params": {
                "coin": coin,
                "to": to,
                "amount": amount,
            }
        }))
        .await
        .unwrap();
    assert_eq!(request.0, StatusCode::OK, "'init_withdraw' failed: {}", request.1);
    json::from_str(&request.1).unwrap()
}

pub async fn withdraw_status(mm: &MarketMakerIt, task_id: u64) -> Json {
    let request = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "withdraw_status",
            "mmrpc": "2.0",
            "params": {
                "task_id": task_id,
            }
        }))
        .await
        .unwrap();
    assert_eq!(request.0, StatusCode::OK, "'withdraw_status' failed: {}", request.1);
    json::from_str(&request.1).unwrap()
}

pub async fn init_z_coin_native(mm: &MarketMakerIt, coin: &str) -> Json {
    let request = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "init_z_coin",
            "mmrpc": "2.0",
            "params": {
                "ticker": coin,
                "activation_params": {
                    "mode": {
                        "rpc": "Native",
                    }
                },
            }
        }))
        .await
        .unwrap();
    assert_eq!(request.0, StatusCode::OK, "'init_z_coin' failed: {}", request.1);
    json::from_str(&request.1).unwrap()
}

pub async fn init_z_coin_light(mm: &MarketMakerIt, coin: &str, electrums: &[&str], lightwalletd_urls: &[&str]) -> Json {
    let request = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "init_z_coin",
            "mmrpc": "2.0",
            "params": {
                "ticker": coin,
                "activation_params": {
                    "mode": {
                        "rpc": "Light",
                        "rpc_data": {
                            "electrum_servers": electrum_servers_rpc(electrums),
                            "light_wallet_d_servers": lightwalletd_urls,
                        },
                    }
                },
            }
        }))
        .await
        .unwrap();
    assert_eq!(request.0, StatusCode::OK, "'init_z_coin' failed: {}", request.1);
    json::from_str(&request.1).unwrap()
}

pub async fn init_z_coin_status(mm: &MarketMakerIt, task_id: u64) -> Json {
    let request = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "init_z_coin_status",
            "mmrpc": "2.0",
            "params": {
                "task_id": task_id,
            }
        }))
        .await
        .unwrap();
    assert_eq!(request.0, StatusCode::OK, "'init_z_coin_status' failed: {}", request.1);
    json::from_str(&request.1).unwrap()
}

pub async fn sign_message(mm: &MarketMakerIt, coin: &str) -> Json {
    let request = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method":"sign_message",
            "mmrpc":"2.0",
            "id": 0,
            "params":{
              "coin": coin,
              "message":"test"
            }
        }))
        .await
        .unwrap();
    assert_eq!(request.0, StatusCode::OK, "'sign_message' failed: {}", request.1);
    json::from_str(&request.1).unwrap()
}

pub async fn verify_message(mm: &MarketMakerIt, coin: &str, signature: &str, address: &str) -> Json {
    let request = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method":"verify_message",
            "mmrpc":"2.0",
            "id": 0,
            "params":{
              "coin": coin,
              "message":"test",
              "signature": signature,
              "address": address

            }
        }))
        .await
        .unwrap();
    assert_eq!(request.0, StatusCode::OK, "'verify_message' failed: {}", request.1);
    json::from_str(&request.1).unwrap()
}

pub async fn send_raw_transaction(mm: &MarketMakerIt, coin: &str, tx: &str) -> Json {
    let request = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "send_raw_transaction",
            "coin": coin,
            "tx_hex": tx,
        }))
        .await
        .unwrap();
    assert_eq!(
        request.0,
        StatusCode::OK,
        "'send_raw_transaction' failed: {}",
        request.1
    );
    json::from_str(&request.1).unwrap()
}
