//! Helpers used in the unit and integration tests.

#![cfg_attr(not(feature = "native"), allow(unused_variables))]

use bytes::Bytes;
use chrono::{Local, TimeZone};
#[cfg(feature = "native")]
use futures01::Future;
use futures::channel::oneshot::channel;
use futures::task::SpawnExt;
use gstuff::ISATTY;
use http::{HeaderMap, Request, StatusCode};
use serde_json::{self as json, Value as Json};
use term;
use rand::Rng;
use regex::Regex;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
#[cfg(not(feature = "native"))]
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Child};
#[cfg(feature = "native")]
use std::str::from_utf8;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

use crate::{now_float, slurp};
use crate::executor::Timer;
#[cfg(not(feature = "native"))]
use crate::mm_ctx::{MmArc, MmCtxBuilder};
#[cfg(not(feature = "native"))]
use crate::helperᶜ;
#[cfg(feature = "native")]
use crate::wio::{slurp_req, POOL};
use crate::log::{dashboard_path, LogState};

/// Automatically kill a wrapped process.
pub struct RaiiKill {pub handle: Child, running: bool}
impl RaiiKill {
    pub fn from_handle (handle: Child) -> RaiiKill {
        RaiiKill {handle, running: true}
    }
    pub fn running (&mut self) -> bool {
        if !self.running {return false}
        match self.handle.try_wait() {Ok (None) => true, _ => {self.running = false; false}}
    }
}
impl Drop for RaiiKill {
    fn drop (&mut self) {
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
    #[cfg(feature = "native")]
    pub log_path: PathBuf
}
#[cfg(feature = "native")]
impl Drop for RaiiDump {
    fn drop (&mut self) {
        // `term` bypasses the stdout capturing, we should only use it if the capturing was disabled.
        let nocapture = env::args().any (|a| a == "--nocapture");

        let log = unwrap! (slurp (&self.log_path));

        // Make sure the log is Unicode.
        // We'll get the "io error when listing tests: Custom { kind: InvalidData, error: StringError("text was not valid unicode") }" otherwise.
        let log = String::from_utf8_lossy (&log);
        let log = log.trim();

        if let (true, true, Some (mut t)) = (nocapture, *ISATTY, term::stdout()) {
            let _ = t.fg (term::color::BRIGHT_YELLOW);
            let _ = t.write (format! ("vvv {:?} vvv\n", self.log_path) .as_bytes());
            let _ = t.fg (term::color::YELLOW);
            let _ = t.write (log.as_bytes());
            let _ = t.write (b"\n");
            let _ = t.reset();
        } else {
            log! ({"vvv {:?} vvv\n{}", self.log_path, log});
        }
    }
}

lazy_static! {
    /// A singleton with the IPs used by the MarketMakerIt instances created in this session.  
    /// The value is set to `false` when the instance is retired.
    static ref MM_IPS: Mutex<HashMap<IpAddr, bool>> = Mutex::new (HashMap::new());
}

#[cfg(feature = "native")]
pub type LocalStart = fn (PathBuf, PathBuf, Json);

#[cfg(not(feature = "native"))]
pub type LocalStart = fn (MmArc);

/// An instance of a MarketMaker process started by and for an integration test.  
/// Given that [in CI] the tests are executed before the build, the binary of that process is the tests binary.
#[cfg(feature = "native")]
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
    pub userpass: String
}

/// A MarketMaker instance started by and for an integration test.
#[cfg(not(feature = "native"))]
pub struct MarketMakerIt {
    pub ctx: super::mm_ctx::MmArc,
    /// Unique (to run multiple instances) IP, like "127.0.0.$x".
    pub ip: IpAddr,
    /// RPC API key.
    pub userpass: String
}

#[cfg(feature = "native")]
impl std::fmt::Debug for MarketMakerIt {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "MarketMakerIt {{ folder: {:?}, ip: {}, log_path: {:?}, userpass: {} }}", self.folder, self.ip, self.log_path, self.userpass)
}   }

impl MarketMakerIt {
    /// Create a new temporary directory and start a new MarketMaker process there.
    /// 
    /// * `conf` - The command-line configuration passed to the MarketMaker.
    ///            Unique local IP address is injected as "myipaddr" unless this field is already present.
    /// * `userpass` - RPC API key. We should probably extract it automatically from the MM log.
    /// * `local` - Function to start the MarketMaker in a local thread, instead of spawning a process.
    /// It's required to manually add 127.0.0.* IPs aliases on Mac to make it properly work.
    /// cf. https://superuser.com/a/458877, https://superuser.com/a/635327
    pub fn start (mut conf: Json, userpass: String, local: Option<LocalStart>)
    -> Result<MarketMakerIt, String> {
        let ip: IpAddr = if conf["myipaddr"].is_null() {  // Generate an unique IP.
            let mut attempts = 0;
            let mut rng = super::small_rng();
            loop {
                let ip4 = Ipv4Addr::new (127, 0, 0, rng.gen_range (1, 255));
                if attempts > 128 {return ERR! ("Out of local IPs?")}
                let ip: IpAddr = ip4.clone().into();
                let mut mm_ips = try_s! (MM_IPS.lock());
                if mm_ips.contains_key (&ip) {attempts += 1; continue}
                mm_ips.insert (ip.clone(), true);
                conf["myipaddr"] = format! ("{}", ip) .into();
                conf["rpcip"] = format! ("{}", ip) .into();
                break ip
            }
        } else {  // Just use the IP given in the `conf`.
            let ip: IpAddr = try_s! (try_s! (conf["myipaddr"].as_str().ok_or ("myipaddr is not a string")) .parse());
            let mut mm_ips = try_s! (MM_IPS.lock());
            if mm_ips.contains_key (&ip) {log! ({"MarketMakerIt] Warning, IP {} was already used.", ip})}
            mm_ips.insert (ip.clone(), true);
            ip
        };

        let folder = new_mm2_temp_folder_path(Some(ip));
        let db_dir = match conf["dbdir"].as_str() {
            Some(path) => path.into(),
            None => {
                let dir = folder.join("DB");
                conf["dbdir"] = unwrap!(dir.to_str()).into();
                dir
            }
        };

        #[cfg(not(feature = "native"))] {
            let ctx = MmCtxBuilder::new().with_conf (conf) .into_mm_arc();
            let local = try_s! (local.ok_or ("!local"));
            local (ctx.clone());
            Ok (MarketMakerIt {ctx, ip, userpass})
        }

        #[cfg(feature = "native")] {
            try_s! (fs::create_dir (&folder));
            match fs::create_dir (db_dir) {
                Ok(_) => (),
                Err(ref ie) if ie.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return ERR!("{}", e),
            };
            let log_path = match conf["log"].as_str() {
                Some(path) => path.into(),
                None => {
                    let path = folder.join("mm2.log");
                    conf["log"] = unwrap!(path.to_str()).into();
                    path
                }
            };

            // If `local` is provided
            // then instead of spawning a process we start the MarketMaker in a local thread,
            // allowing us to easily *debug* the tested MarketMaker code.
            // Note that this should only be used while running a single test,
            // using this option while running multiple tests (or multiple MarketMaker instances) is currently UB.
            let pc = if let Some (local) = local {
                local (folder.clone(), log_path.clone(), conf);
                None
            } else {
                let executable = try_s! (env::args().next().ok_or ("No program name"));
                let executable = try_s! (Path::new (&executable) .canonicalize());
                let log = try_s! (fs::File::create (&log_path));
                let child = try_s! (Command::new (&executable) .arg ("test_mm_start") .arg ("--nocapture")
                    .current_dir (&folder)
                    .env ("_MM2_TEST_CONF", try_s! (json::to_string (&conf)))
                    .env ("MM2_UNBUFFERED_OUTPUT", "1")
                    .stdout (try_s! (log.try_clone()))
                    .stderr (log)
                    .spawn());
                Some (RaiiKill::from_handle (child))
            };

            Ok (MarketMakerIt {folder, ip, log_path, pc, userpass})
        }
    }

    #[cfg(feature = "native")]
    pub fn log_as_utf8 (&self) -> Result<String, String> {
        let mm_log = try_s! (slurp (&self.log_path));
        let mm_log = unsafe {String::from_utf8_unchecked (mm_log)};
        Ok (mm_log)
    }

    /// Busy-wait on the log until the `pred` returns `true` or `timeout_sec` expires.
    #[cfg(feature = "native")]
    pub async fn wait_for_log<F> (&mut self, timeout_sec: f64, pred: F) -> Result<(), String>
    where F: Fn (&str) -> bool {
        let start = now_float();
        let ms = 50 .min ((timeout_sec * 1000.) as u64 / 20 + 10);
        loop {
            let mm_log = try_s! (self.log_as_utf8());
            if pred (&mm_log) {return Ok(())}
            if now_float() - start > timeout_sec {return ERR! ("Timeout expired waiting for a log condition")}
            if let Some (ref mut pc) = self.pc {if !pc.running() {return ERR! ("MM process terminated prematurely.")}}
            Timer::sleep (ms as f64 / 1000.) .await
        }
    }

    /// Busy-wait on the log until the `pred` returns `true` or `timeout_sec` expires.
    /// The difference from standard wait_for_log is this function keeps working
    /// after process is stopped
    #[cfg(feature = "native")]
    pub async fn wait_for_log_after_stop<F> (&mut self, timeout_sec: f64, pred: F) -> Result<(), String>
    where F: Fn (&str) -> bool {
        let start = now_float();
        let ms = 50 .min ((timeout_sec * 1000.) as u64 / 20 + 10);
        loop {
            let mm_log = try_s! (self.log_as_utf8());
            if pred (&mm_log) {return Ok(())}
            if now_float() - start > timeout_sec {return ERR! ("Timeout expired waiting for a log condition")}
            Timer::sleep (ms as f64 / 1000.) .await
        }
    }

    /// Busy-wait on the instance in-memory log until the `pred` returns `true` or `timeout_sec` expires.
    #[cfg(not(feature = "native"))]
    pub async fn wait_for_log<F> (&mut self, timeout_sec: f64, pred: F) -> Result<(), String>
    where F: Fn (&str) -> bool {
        let start = now_float();
        loop {
            let tail = unsafe {std::str::from_utf8_unchecked (&crate::PROCESS_LOG_TAIL[..])};
            if pred (tail) {return Ok(())}
            if now_float() - start > timeout_sec {return ERR! ("Timeout expired waiting for a log condition")}
            Timer::sleep (0.1) .await
    }   }

    /// Invokes the locally running MM and returns its reply.
    pub async fn rpc (&self, payload: Json) -> Result<(StatusCode, String, HeaderMap), String> {
        let uri = format! ("http://{}:7783", self.ip);
        log!("sending rpc request " (unwrap!(json::to_string(&payload))) " to " (uri));
        let payload = try_s! (json::to_vec (&payload));
        #[cfg(not(feature = "native"))] let payload = futures01::stream::once (Ok (Bytes::from (payload)));
        let request = try_s! (Request::builder().method ("POST") .uri (uri) .body (payload));
        #[cfg(feature = "native")] {
            let (status, headers, body) = try_s! (slurp_req (request) .wait());
            Ok ((status, try_s! (from_utf8 (&body)) .trim().into(), headers))
        }
        #[cfg(not(feature = "native"))] {
            let rpc_service = try_s! (crate::header::RPC_SERVICE.as_option().ok_or ("!RPC_SERVICE"));
            let (parts, body) = request.into_parts();
            let client: SocketAddr = try_s! ("127.0.0.1:1".parse());
            let f = rpc_service (self.ctx.clone(), parts, Box::new (body), client);
            let response = try_s! (f.await);
            let (parts, body) = response.into_parts();
            Ok ((parts.status, try_s! (String::from_utf8 (body)), parts.headers))
        }
    }

    /// Sends the &str payload to the locally running MM and returns it's reply.
    #[cfg(feature = "native")]
    pub fn rpc_str (&self, payload: &'static str) -> Result<(StatusCode, String, HeaderMap), String> {
        let uri = format! ("http://{}:7783", self.ip);
        let request = try_s! (Request::builder().method ("POST") .uri (uri) .body (payload.into()));
        let (status, headers, body) = try_s! (slurp_req (request) .wait());
        Ok ((status, try_s! (from_utf8 (&body)) .trim().into(), headers))
    }

    #[cfg(not(feature = "native"))]
    pub fn rpc_str (&self, _payload: &'static str) -> Result<(StatusCode, String, HeaderMap), String> {
        unimplemented!()
    }

    #[cfg(feature = "native")]
    pub fn mm_dump (&self) -> (RaiiDump, RaiiDump) {mm_dump (&self.log_path)}

    #[cfg(not(feature = "native"))]
    pub fn mm_dump (&self) -> (RaiiDump, RaiiDump) {(RaiiDump{}, RaiiDump{})}

    /// Send the "stop" request to the locally running MM.
    pub async fn stop (&self) -> Result<(), String> {
        let (status, body, _headers) = match self.rpc (json! ({"userpass": self.userpass, "method": "stop"})) .await {
            Ok (t) => t,
            Err (err) => {
                // Downgrade the known errors into log warnings,
                // in order not to spam the unit test logs with confusing panics, obscuring the real issue.
                if err.contains ("An existing connection was forcibly closed by the remote host") {
                    log! ("stop] MM already down? " (err));
                    return Ok(())
                } else {
                    return ERR! ("{}", err)
        }   }   };
        if status != StatusCode::OK {return ERR! ("MM didn't accept a stop. body: {}", body)}
        Ok(())
    }
}

#[cfg(feature = "native")]
impl Drop for MarketMakerIt {
    fn drop (&mut self) {
        if let Ok (mut mm_ips) = MM_IPS.lock() {
            // The IP addresses might still be used by the libtorrent even after a context is dropped,
            // hence we're not trying to reuse them but rather just mark them as fried.
            if let Some (active) = mm_ips.get_mut (&self.ip) {
                *active = false
            }
        } else {log! ("MarketMakerIt] Can't lock MM_IPS.")}
    }
}

#[macro_export]
macro_rules! wait_log_re {
    ($mm_it: expr, $timeout_sec: expr, $re_pred: expr) => {{
        log! ("Waiting for “" ($re_pred) "”…");
        let re = unwrap! (regex::Regex::new ($re_pred));
        let rc = $mm_it.wait_for_log ($timeout_sec, |line| re.is_match (line)) .await;
        if let Err (err) = rc {panic! ("{}: {}", $re_pred, err)}
    }};
}

/// Busy-wait on the log until the `pred` returns `true` or `timeout_sec` expires.
#[cfg(feature = "native")]
pub fn wait_for_log (log: &LogState, timeout_sec: f64, pred: &dyn Fn (&str) -> bool) -> Result<(), String> {
    let start = now_float();
    let ms = 50 .min ((timeout_sec * 1000.) as u64 / 20 + 10);
    let mut buf = String::with_capacity (128);
    let mut found = false;
    loop {
        log.with_tail (&mut |tail| {
            for en in tail {
                if en.format (&mut buf) .is_ok() {
                    if pred (&buf) {found = true; break}
                }
            }
        });
        if found {return Ok(())}

        log.with_gravity_tail (&mut |tail| {
            for chunk in tail {
                if pred (chunk) {found = true; break}
            }
        });
        if found {return Ok(())}

        if now_float() - start > timeout_sec {return ERR! ("Timeout expired waiting for a log condition")}
        sleep (Duration::from_millis (ms));
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct ToWaitForLogRe {ctx: u32, timeout_sec: f64, re_pred: String}

#[cfg(feature = "native")]
pub async fn common_wait_for_log_re (req: Bytes) -> Result<Vec<u8>, String> {
    let args: ToWaitForLogRe = try_s! (json::from_slice (&req));
    let ctx = try_s! (crate::mm_ctx::MmArc::from_ffi_handle (args.ctx));
    let re = try_s! (Regex::new (&args.re_pred));

    // Run the blocking `wait_for_log` in the `POOL`.
    let (tx, rx) = channel();
    try_s! (try_s! (POOL.lock()) .spawn (async move {
        let _ = tx.send (wait_for_log (&ctx.log, args.timeout_sec, &|line| re.is_match (line)));
    }));
    try_s! (try_s! (rx.await));

    Ok (Vec::new())
}

#[cfg(feature = "native")]
pub async fn wait_for_log_re (ctx: &crate::mm_ctx::MmArc, timeout_sec: f64, re_pred: &str) -> Result<(), String> {
    let re = try_s! (Regex::new (re_pred));
    wait_for_log (&ctx.log, timeout_sec, &|line| re.is_match (line))
}

#[cfg(not(feature = "native"))]
pub async fn wait_for_log_re (ctx: &crate::mm_ctx::MmArc, timeout_sec: f64, re_pred: &str) -> Result<(), String> {
    try_s! (helperᶜ ("common_wait_for_log_re", try_s! (json::to_vec (&ToWaitForLogRe {
        ctx: try_s! (ctx.ffi_handle()),
        timeout_sec: timeout_sec,
        re_pred: re_pred.into()
    }))) .await);
    Ok(())
}

/// Create RAII variables to the effect of dumping the log and the status dashboard at the end of the scope.
#[cfg(feature = "native")]
pub fn mm_dump (log_path: &Path) -> (RaiiDump, RaiiDump) {(
    RaiiDump {log_path: log_path.to_path_buf()},
    RaiiDump {log_path: unwrap! (dashboard_path (log_path))}
)}

/// A typical MM instance.
#[cfg(feature = "native")]
pub fn mm_spat (local_start: LocalStart, conf_mod: &dyn Fn(Json)->Json) -> (&'static str, MarketMakerIt, RaiiDump, RaiiDump) {
    let passphrase = "SPATsRps3dhEtXwtnpRCKF";
    let mm = unwrap! (MarketMakerIt::start (
        conf_mod (json! ({
            "gui": "nogui",
            "passphrase": passphrase,
            "rpccors": "http://localhost:4000",
            "coins": [
                {"coin": "BEER", "asset": "BEER", "rpcport": 8923},
                {"coin": "PIZZA", "asset": "PIZZA", "rpcport": 11116}
            ],
            "i_am_seed": true,
            "rpc_password": "pass",
        })),
        "pass".into(),
        match super::var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "1" => Some (local_start), _ => None}
    ));
    let (dump_log, dump_dashboard) = mm_dump (&mm.log_path);
    (passphrase, mm, dump_log, dump_dashboard)
}

#[cfg(not(feature = "native"))]
pub fn mm_spat (_local_start: LocalStart, _conf_mod: &dyn Fn(Json)->Json) -> (&'static str, MarketMakerIt, RaiiDump, RaiiDump) {
    unimplemented!()
}

/// Asks MM to enable the given currency in electrum mode
/// fresh list of servers at https://github.com/jl777/coins/blob/master/electrums/.
pub async fn enable_electrum (mm: &MarketMakerIt, coin: &str, urls: Vec<&str>) -> Json {
    let servers: Vec<_> = urls.into_iter().map(|url| json!({"url": url})).collect();
    let electrum = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": coin,
        "servers": servers,
        "mm2": 1,
    })) .await);
    assert_eq! (electrum.0, StatusCode::OK, "RPC «electrum» failed with {} {}", electrum.0, electrum.1);
    unwrap!(json::from_str(&electrum.1))
}

/// Reads passphrase and userpass from .env file
pub fn from_env_file (env: Vec<u8>) -> (Option<String>, Option<String>) {
    use regex::bytes::Regex;
    let (mut passphrase, mut userpass) = (None, None);
    for cap in unwrap! (Regex::new (r"(?m)^(PASSPHRASE|USERPASS)=(\w[\w ]+)$")) .captures_iter (&env) {
        match cap.get (1) {
            Some (name) if name.as_bytes() == b"PASSPHRASE" =>
                passphrase = cap.get (2) .map (|v| unwrap! (String::from_utf8 (v.as_bytes().into()))),
            Some (name) if name.as_bytes() == b"USERPASS" =>
                userpass = cap.get (2) .map (|v| unwrap! (String::from_utf8 (v.as_bytes().into()))),
            _ => ()
        }
    }
    (passphrase, userpass)
}

#[cfg(not(feature = "native"))]
use std::os::raw::c_char;

/// Reads passphrase from file or environment.
pub fn get_passphrase (path: &dyn AsRef<Path>, env: &str) -> Result<String, String> {
    if let (Some (file_passphrase), _file_userpass) = from_env_file (try_s! (slurp (path))) {
        return Ok (file_passphrase)
    }

    if let Ok (v) = super::var (env) {
        Ok (v)
    } else {
        ERR! ("No {} or {}", env, path.as_ref().display())
    }
}

/// Asks MM to enable the given currency in native mode.
/// Returns the RPC reply containing the corresponding wallet address.
pub async fn enable_native(mm: &MarketMakerIt, coin: &str, urls: Vec<&str>) -> Json {
    let native = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "enable",
        "coin": coin,
        "urls": urls,
        // Dev chain swap contract address
        "swap_contract_address": "0xa09ad3cd7e96586ebd05a2607ee56b56fb2db8fd",
        "mm2": 1,
    })) .await);
    assert_eq! (native.0, StatusCode::OK, "'enable' failed: {}", native.1);
    unwrap!(json::from_str(&native.1))
}

/// Use a separate (unique) temporary folder for each MM.
/// We could also remove the old folders after some time in order not to spam the temporary folder.
/// Though we don't always want to remove them right away, allowing developers to check the files).
/// Appends IpAddr if it is pre-known
pub fn new_mm2_temp_folder_path(ip: Option<IpAddr>) -> PathBuf {
    let now = super::now_ms();
    let now = Local.timestamp ((now / 1000) as i64, (now % 1000) as u32 * 1000000);
    let folder = match ip {
        Some(ip) => format! ("mm2_{}_{}", now.format ("%Y-%m-%d_%H-%M-%S-%3f"), ip),
        None => format! ("mm2_{}", now.format ("%Y-%m-%d_%H-%M-%S-%3f")),
    };
    super::temp_dir().join (folder)
}
