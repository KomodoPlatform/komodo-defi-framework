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
 * or distributed except according to the terms contained in the              *
 * LICENSE-COPYRIGHT-NOTICE file.                                             *
 *                                                                            *
 * Removal or modification of this copyright notice is prohibited.            *
 *                                                                            *
 ******************************************************************************/
//
//  mm2.rs
//  marketmaker
//
//  Copyright © 2022 AtomicDEX. All rights reserved.
//

#![cfg_attr(target_arch = "wasm32", allow(dead_code))]
#![cfg_attr(target_arch = "wasm32", allow(unused_imports))]

#[cfg(not(target_arch = "wasm32"))] use common::block_on;
use common::crash_reports::init_crash_reports;
use common::double_panic_crash;
use common::log::LogLevel;
use common::password_policy::password_policy;
use mm2_core::mm_ctx::MmCtxBuilder;

#[cfg(feature = "custom-swap-locktime")] use common::log::warn;
#[cfg(feature = "custom-swap-locktime")]
use lp_swap::PAYMENT_LOCKTIME;
#[cfg(feature = "custom-swap-locktime")]
use std::sync::atomic::Ordering;

use gstuff::slurp;

use serde::ser::Serialize;
use serde_json::{self as json, Value as Json};

use std::env;
use std::ffi::OsString;
use std::process::exit;
use std::ptr::null;
use std::str;

#[path = "lp_native_dex.rs"] mod lp_native_dex;
pub use self::lp_native_dex::lp_init;
use coins::update_coins_config;
use mm2_err_handle::prelude::*;

#[cfg(not(target_arch = "wasm32"))]
#[path = "database.rs"]
pub mod database;

#[path = "lp_dispatcher.rs"] pub mod lp_dispatcher;
#[path = "lp_message_service.rs"] pub mod lp_message_service;
#[path = "lp_network.rs"] pub mod lp_network;
#[path = "lp_ordermatch.rs"] pub mod lp_ordermatch;
#[path = "lp_price.rs"] pub mod lp_price;
#[path = "lp_stats.rs"] pub mod lp_stats;
#[path = "lp_swap.rs"] pub mod lp_swap;
#[path = "rpc.rs"] pub mod rpc;

pub const PASSWORD_MAXIMUM_CONSECUTIVE_CHARACTERS: usize = 3;

#[cfg(feature = "custom-swap-locktime")]
const CUSTOM_PAYMENT_LOCKTIME_DEFAULT: u64 = 900;

#[derive(Serialize)]
pub struct MmVersionResult {
    result: String,
    datetime: String,
}

impl MmVersionResult {
    pub const fn new(result: String, datetime: String) -> MmVersionResult { MmVersionResult { result, datetime } }

    pub fn to_json(&self) -> Json { json::to_value(self).expect("expected valid JSON object") }
}

pub struct LpMainParams {
    conf: Json,
    filter: Option<LogLevel>,
}

impl LpMainParams {
    pub fn with_conf(conf: Json) -> LpMainParams { LpMainParams { conf, filter: None } }

    pub fn log_filter(mut self, filter: Option<LogLevel>) -> LpMainParams {
        self.filter = filter;
        self
    }
}

#[cfg(feature = "custom-swap-locktime")]
/// Reads `payment_locktime` from conf arg and assigns it into `PAYMENT_LOCKTIME` in lp_swap.
/// Assigns 900 if `payment_locktime` is invalid or not provided.
fn initialize_payment_locktime(conf: &Json) {
    match conf["payment_locktime"].as_u64() {
        Some(lt) => PAYMENT_LOCKTIME.store(lt, Ordering::Relaxed),
        None => {
            warn!(
                "payment_locktime is either invalid type or not provided in the configuration or
                MM2.json file. payment_locktime will be proceeded as {} seconds.",
                CUSTOM_PAYMENT_LOCKTIME_DEFAULT
            );
        },
    };
}

/// * `ctx_cb` - callback used to share the `MmCtx` ID with the call site.
pub async fn lp_main(
    params: LpMainParams,
    ctx_cb: &dyn Fn(u32),
    version: String,
    datetime: String,
) -> Result<(), String> {
    let log_filter = params.filter.unwrap_or_default();
    // Logger can be initialized once.
    // If `mm2` is linked as a library, and `mm2` is restarted, `init_logger` returns an error.
    init_logger(log_filter).ok();

    let conf = params.conf;
    if !conf["rpc_password"].is_null() {
        if !conf["rpc_password"].is_string() {
            return ERR!("rpc_password must be string");
        }

        let is_weak_password_accepted = conf["allow_weak_password"].as_bool() == Some(true);

        if conf["rpc_password"].as_str() == Some("") {
            return ERR!("rpc_password must not be empty");
        }

        if !is_weak_password_accepted && cfg!(not(test)) {
            match password_policy(conf["rpc_password"].as_str().unwrap()) {
                Ok(_) => {},
                Err(err) => return Err(format!("{}", err)),
            }
        }
    }

    #[cfg(feature = "custom-swap-locktime")]
    initialize_payment_locktime(&conf);

    let ctx = MmCtxBuilder::new()
        .with_conf(conf)
        .with_log_level(log_filter)
        .with_version(version.clone())
        .with_datetime(datetime.clone())
        .into_mm_arc();
    ctx_cb(try_s!(ctx.ffi_handle()));
    try_s!(lp_init(ctx, version, datetime).await);
    Ok(())
}

fn help() {
    const HELP_MSG: &str = r#"Command-line options.
The first command-line argument is special and designates the mode.

  help                       ..  Display this message.
  btc2kmd {WIF or BTC}       ..  Convert a BTC WIF into a KMD WIF.
  events                     ..  Listen to a feed coming from a separate MM daemon and print it to stdout.
  vanity {substring}         ..  Tries to find an address with the given substring.
  update_config {SRC} {DST}  ..  Update the configuration of coins from the SRC config and save it to DST file.
  {JSON configuration}       ..  Run the MarketMaker daemon.

Some (but not all) of the JSON configuration parameters (* - required):

                     NB: The 'coins' command-line configuration must have the lowercased coin names in the 'name' field,
                     {"coins": [{"name": "dash", "coin": "DASH", ...}, ...], ...}.
  coins          ..  Information about the currencies: their ticker symbols, names, ports, addresses, etc.
                     If the field isn't present on the command line then we try loading it from the 'coins' file.
  crash          ..  Simulate a crash to check how the crash handling works.
  dbdir          ..  MM database path. 'DB' by default.
  gui            ..  The information about GUI app using MM2 instance. Included in swap statuses shared with network.
                 ..  It's recommended to put essential info to this field (application name, OS, version, etc).
                 ..  e.g. AtomicDEX iOS 1.0.1000.
  myipaddr       ..  IP address to bind to for P2P networking.
  netid          ..  Subnetwork. Affects ports and keys.
  passphrase *   ..  Wallet seed.
                     Compressed WIFs and hexadecimal ECDSA keys (prefixed with 0x) are also accepted.
  panic          ..  Simulate a panic to see if backtrace works.
  rpccors        ..  Access-Control-Allow-Origin header value to be used in all the RPC responses.
                     Default is currently 'http://localhost:3000'
  rpcip          ..  IP address to bind to for RPC server. Overrides the 127.0.0.1 default
  rpc_password   ..  RPC password used to authorize non-public RPC calls
                     MM generates password from passphrase if this field is not set
  rpc_local_only ..  MM forbids some RPC requests from not loopback (localhost) IPs as additional security measure.
                     Defaults to `true`, set `false` to disable. `Use with caution`.
  rpcport        ..  If > 1000 overrides the 7783 default.
  i_am_seed      ..  Activate the seed node mode (acting as a relay for mm2 clients).
                     Defaults to `false`.
  seednodes      ..  Seednode IPs that node will use.
                     At least one seed IP must be present if the node is not a seed itself.
  stderr         ..  Print a message to stderr and exit.
  wif            ..  `1` to add WIFs to the information we provide about a coin.

Environment variables:

  MM_CONF_PATH   ..  File path. MM2 will try to load the JSON configuration from this file.
                     File must contain valid json with structure mentioned above.
                     Defaults to `MM2.json`
  MM_COINS_PATH  ..  File path. MM2 will try to load coins data from this file.
                     File must contain valid json.
                     Recommended: https://github.com/jl777/coins/blob/master/coins.
                     Defaults to `coins`.
  MM_LOG         ..  File path. Must end with '.log'. MM will log to this file.

See also the online documentation at
https://developers.atomicdex.io
"#;

    println!("{}", HELP_MSG);
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)] // Not used by mm2_lib.
pub fn mm2_main(version: String, datetime: String) {
    use libc::c_char;

    init_crash_reports();

    // Temporarily simulate `argv[]` for the C version of the main method.
    let args: Vec<String> = env::args()
        .map(|mut arg| {
            arg.push('\0');
            arg
        })
        .collect();
    let mut args: Vec<*const c_char> = args.iter().map(|s| s.as_ptr() as *const c_char).collect();
    args.push(null());

    let args_os: Vec<OsString> = env::args_os().collect();

    // NB: The first argument is special, being used as the mode switcher.
    // The other arguments might be used to pass the data to the various MM modes,
    // we're not checking them for the mode switches in order not to risk [untrusted] data being mistaken for a mode switch.
    let first_arg = args_os.get(1).and_then(|arg| arg.to_str());

    if first_arg == Some("panic") {
        panic!("panic message")
    }
    if first_arg == Some("crash") {
        double_panic_crash()
    }
    if first_arg == Some("stderr") {
        eprintln!("This goes to stderr");
        return;
    }
    if first_arg == Some("update_config") {
        match on_update_config(&args_os) {
            Ok(_) => println!("Success"),
            Err(e) => eprintln!("{}", e),
        }
        return;
    }

    if first_arg == Some("--version") || first_arg == Some("-v") || first_arg == Some("version") {
        println!("AtomicDEX API: {version}");
        return;
    }

    if first_arg == Some("--help") || first_arg == Some("-h") || first_arg == Some("help") {
        help();
        return;
    }

    if cfg!(windows) && first_arg == Some("/?") {
        help();
        return;
    }

    log!("AtomicDEX API {} DT {}", version, datetime);

    if let Err(err) = run_lp_main(first_arg, &|_| (), version, datetime) {
        log!("{}", err);
        exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Parses and returns the `first_arg` as JSON.
/// Attempts to load the config from `MM2.json` file if `first_arg` is None
pub fn get_mm2config(first_arg: Option<&str>) -> Result<Json, String> {
    let conf_path = env::var("MM_CONF_PATH").unwrap_or_else(|_| "MM2.json".into());
    let conf_from_file = slurp(&conf_path);
    let conf = match first_arg {
        Some(s) => s,
        None => {
            if conf_from_file.is_empty() {
                return ERR!(
                    "Config is not set from command line arg and {} file doesn't exist.",
                    conf_path
                );
            }
            try_s!(std::str::from_utf8(&conf_from_file))
        },
    };

    let mut conf: Json = match json::from_str(conf) {
        Ok(json) => json,
        Err(err) => return ERR!("Couldn't parse.({}).{}", conf, err),
    };

    if conf["coins"].is_null() {
        let coins_path = env::var("MM_COINS_PATH").unwrap_or_else(|_| "coins".into());
        let coins_from_file = slurp(&coins_path);
        if coins_from_file.is_empty() {
            return ERR!(
                "No coins are set in JSON config and '{}' file doesn't exist",
                coins_path
            );
        }
        conf["coins"] = match json::from_slice(&coins_from_file) {
            Ok(j) => j,
            Err(e) => {
                return ERR!(
                    "Error {} parsing the coins file, please ensure it contains valid json",
                    e
                )
            },
        }
    }

    Ok(conf)
}

/// Runs LP_main with result of `get_mm2config()`.
///
/// * `ctx_cb` - Invoked with the MM context handle,
///              allowing the `run_lp_main` caller to communicate with MM.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_lp_main(
    first_arg: Option<&str>,
    ctx_cb: &dyn Fn(u32),
    version: String,
    datetime: String,
) -> Result<(), String> {
    fn setup_env_logger() {
        const MM2_LOG_ENV_KEY: &str = "RUST_LOG";

        if env::var_os(MM2_LOG_ENV_KEY).is_none() {
            env::set_var(MM2_LOG_ENV_KEY, "info");
        };

        if let Err(e) = env_logger::try_init() {
            common::log::debug!("env_logger is already initialized. {}", e);
        };
    }

    setup_env_logger();

    let conf = get_mm2config(first_arg)?;

    let log_filter = LogLevel::from_env();

    let params = LpMainParams::with_conf(conf).log_filter(log_filter);
    try_s!(block_on(lp_main(params, ctx_cb, version, datetime)));
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn on_update_config(args: &[OsString]) -> Result<(), String> {
    use mm2_io::fs::safe_slurp;

    let src_path = args.get(2).ok_or(ERRL!("Expect path to the source coins config."))?;
    let dst_path = args.get(3).ok_or(ERRL!("Expect destination path."))?;

    let config = try_s!(safe_slurp(src_path));
    let mut config: Json = try_s!(json::from_slice(&config));

    let result = if config.is_array() {
        try_s!(update_coins_config(config))
    } else {
        // try to get config["coins"] as array
        let conf_obj = config.as_object_mut().ok_or(ERRL!("Expected coin list"))?;
        let coins = conf_obj.remove("coins").ok_or(ERRL!("Expected coin list"))?;
        let updated_coins = try_s!(update_coins_config(coins));
        conf_obj.insert("coins".into(), updated_coins);
        config
    };

    let buf = Vec::new();
    let formatter = json::ser::PrettyFormatter::with_indent(b"\t");
    let mut ser = json::Serializer::with_formatter(buf, formatter);
    try_s!(result.serialize(&mut ser));
    try_s!(std::fs::write(dst_path, ser.into_inner()));
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn init_logger(level: LogLevel) -> Result<(), String> {
    use common::log::UnifiedLoggerBuilder;

    UnifiedLoggerBuilder::default()
        .level_filter(level)
        .console(false)
        .mm_log(true)
        .try_init()
}

#[cfg(target_arch = "wasm32")]
fn init_logger(level: LogLevel) -> Result<(), String> {
    common::log::WasmLoggerBuilder::default().level_filter(level).try_init()
}
