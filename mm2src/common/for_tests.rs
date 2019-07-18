//! Helpers used in the unit and integration tests.

use chrono::Local;
#[cfg(feature = "native")]
use futures::Future;
use gstuff::{now_float, slurp, ISATTY};
use http::{StatusCode, HeaderMap};
#[cfg(feature = "native")]
use http::{Request};
use serde_json::{self as json, Value as Json};
use term;
use rand::{thread_rng, Rng};
use std::collections::HashSet;
use std::env::{self, var};
use std::fs;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::{Command, Child};
#[cfg(feature = "native")]
use std::str::from_utf8;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

#[cfg(feature = "native")]
use super::wio::slurp_req;
use super::log::{dashboard_path, LogState};

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
    pub log_path: PathBuf
}
impl Drop for RaiiDump {
    fn drop (&mut self) {
        // `term` bypasses the stdout capturing, we should only use it if the capturing was disabled.
        let nocapture = env::args().any (|a| a == "--nocapture");

        let log = slurp (&self.log_path);

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
    static ref MM_IPS: Mutex<HashSet<IpAddr>> = Mutex::new (HashSet::new());
}

pub type LocalStart = fn (PathBuf, PathBuf, Json);

/// An instance of a MarketMaker process started by and for an integration test.  
/// Given that [in CI] the tests are executed before the build, the binary of that process is the tests binary.
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

impl std::fmt::Debug for MarketMakerIt {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "MarketMakerIt {{ folder: {:?}, ip: {}, log_path: {:?}, userpass: {} }}", self.folder, self.ip, self.log_path, self.userpass)
    }
}

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
        let executable = try_s! (env::args().next().ok_or ("No program name"));
        let executable = try_s! (Path::new (&executable) .canonicalize());
        let ip: IpAddr = if conf["myipaddr"].is_null() {  // Generate an unique IP.
            let mut attempts = 0;
            let mut rng = thread_rng();
            loop {
                let ip4 = Ipv4Addr::new (127, 0, 0, rng.gen_range (1, 255));
                if attempts > 128 {return ERR! ("Out of local IPs?")}
                let ip: IpAddr = ip4.clone().into();
                let mut mm_ips = try_s! (MM_IPS.lock());
                if mm_ips.contains (&ip) {attempts += 1; continue}
                mm_ips.insert (ip.clone());
                conf["myipaddr"] = format! ("{}", ip) .into();
                conf["rpcip"] = format! ("{}", ip) .into();
                break ip
            }
        } else {  // Just use the IP given in the `conf`.
            let ip: IpAddr = try_s! (try_s! (conf["myipaddr"].as_str().ok_or ("myipaddr is not a string")) .parse());
            let mut mm_ips = try_s! (MM_IPS.lock());
            if mm_ips.contains (&ip) {log! ({"MarketMakerIt] Warning, IP {} was already used.", ip})}
            mm_ips.insert (ip.clone());
            ip
        };

        // Use a separate (unique) temporary folder for each MM.
        // (We could also remove the old folders after some time in order not to spam the temporary folder.
        // Though we don't always want to remove them right away, allowing developers to check the files).
        let now = Local::now();
        let folder = format! ("mm2_{}_{}", now.format ("%Y-%m-%d_%H-%M-%S-%3f"), ip);
        let folder = env::temp_dir().join (folder);
        try_s! (fs::create_dir (&folder));
        let db_dir = folder.join ("DB");
        conf["dbdir"] = unwrap! (db_dir.to_str()) .into();

        try_s! (fs::create_dir (db_dir));
        let log_path = folder.join ("mm2.log");
        conf["log"] = unwrap! (log_path.to_str()) .into();

        // If `local` is provided
        // then instead of spawning a process we start the MarketMaker in a local thread,
        // allowing us to easily *debug* the tested MarketMaker code.
        // Note that this should only be used while running a single test,
        // using this option while running multiple tests (or multiple MarketMaker instances) is currently UB.
        let pc = if let Some (local) = local {
            local (folder.clone(), log_path.clone(), conf);
            None
        } else {
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
    pub fn log_as_utf8 (&self) -> Result<String, String> {
        let mm_log = slurp (&self.log_path);
        let mm_log = unsafe {String::from_utf8_unchecked (mm_log)};
        Ok (mm_log)
    }
    /// Busy-wait on the log until the `pred` returns `true` or `timeout_sec` expires.
    pub fn wait_for_log (&mut self, timeout_sec: f64, pred: &dyn Fn (&str) -> bool) -> Result<(), String> {
        let start = now_float();
        let ms = 50 .min ((timeout_sec * 1000.) as u64 / 20 + 10);
        loop {
            let mm_log = try_s! (self.log_as_utf8());
            if pred (&mm_log) {return Ok(())}
            if now_float() - start > timeout_sec {return ERR! ("Timeout expired waiting for a log condition")}
            if let Some (ref mut pc) = self.pc {if !pc.running() {return ERR! ("MM process terminated prematurely.")}}
            sleep (Duration::from_millis (ms));
        }
    }

    /// Invokes the locally running MM and returns its reply.
    #[cfg(feature = "native")]
    pub fn rpc (&self, payload: Json) -> Result<(StatusCode, String, HeaderMap), String> {
        let payload = try_s! (json::to_string (&payload));
        let uri = format! ("http://{}:7783", self.ip);
        let request = try_s! (Request::builder().method ("POST") .uri (uri) .body (payload.into()));
        let (status, headers, body) = try_s! (slurp_req (request) .wait());
        Ok ((status, try_s! (from_utf8 (&body)) .trim().into(), headers))
    }
    #[cfg(not(feature = "native"))]
    pub fn rpc (&self, _payload: Json) -> Result<(StatusCode, String, HeaderMap), String> {
        unimplemented!()
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

    /// Send the "stop" request to the locally running MM.
    #[cfg(feature = "native")]
    pub fn stop (&self) -> Result<(), String> {
        let (status, body, _headers) = match self.rpc (json! ({"userpass": self.userpass, "method": "stop"})) {
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
    #[cfg(not(feature = "native"))]
    pub fn stop (&self) -> Result<(), String>{
        unimplemented!()
    }
}
impl Drop for MarketMakerIt {
    fn drop (&mut self) {
        if let Ok (mut mm_ips) = MM_IPS.lock() {
            mm_ips.remove (&self.ip);
        } else {log! ("MarketMakerIt] Can't lock MM_IPS.")}
    }
}

/// Busy-wait on the log until the `pred` returns `true` or `timeout_sec` expires.
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

/// Create RAII variables to the effect of dumping the log and the status dashboard at the end of the scope.
pub fn mm_dump (log_path: &Path) -> (RaiiDump, RaiiDump) {(
    RaiiDump {log_path: log_path.to_path_buf()},
    RaiiDump {log_path: unwrap! (dashboard_path (log_path))}
)}

/// A typical MM instance.
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
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "1" => Some (local_start), _ => None}
    ));
    let (dump_log, dump_dashboard) = mm_dump (&mm.log_path);
    (passphrase, mm, dump_log, dump_dashboard)
}

/// Asks MM to enable the given currency in electrum mode
/// fresh list of servers at https://github.com/jl777/coins/blob/master/electrums/.
#[cfg(feature = "native")]
pub fn enable_electrum (mm: &MarketMakerIt, coin: &str, urls: Vec<&str>) -> Json {
    let servers: Vec<_> = urls.into_iter().map(|url| json!({"url": url})).collect();
    let electrum = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": coin,
        "servers": servers,
        "mm2": 1,
    })));
    assert_eq! (electrum.0, StatusCode::OK, "RPC «electrum» failed with {} {}", electrum.0, electrum.1);
    unwrap!(json::from_str(&electrum.1))
}
#[cfg(not(feature = "native"))]
pub fn enable_electrum (_mm: &MarketMakerIt, _coin: &str, _urls: Vec<&str>) -> Json {
    unimplemented!()
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
