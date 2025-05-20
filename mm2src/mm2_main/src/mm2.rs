/******************************************************************************
 * Copyright © 2023 Pampex LTD and TillyHK LTD                                *
 *                                                                            *
 * See the CONTRIBUTOR-LICENSE-AGREEMENT, COPYING, LICENSE-COPYRIGHT-NOTICE   *
 * and DEVELOPER-CERTIFICATE-OF-ORIGIN files in the LEGAL directory in        *
 * the top-level directory of this distribution for the individual copyright  *
 * holder information and the developer policies on copyright and licensing.  *
 *                                                                            *
 * Unless otherwise agreed in a custom licensing agreement, no part of the    *
 * Komodo DeFi Framework software, including this file may be copied, modified, propagated*
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
//  Copyright © 2023 Pampex LTD and TillyHK LTD. All rights reserved.
//

#![feature(hash_raw_entry)]
// `mockable` implementation uses these
#![allow(
    forgetting_references,
    forgetting_copy_types,
    clippy::swap_ptr_to_ref,
    clippy::forget_non_drop,
    clippy::let_unit_value,
    clippy::doc_lazy_continuation
)]
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]
#![cfg_attr(target_arch = "wasm32", allow(unused_imports))]

#[macro_use] extern crate common;
#[macro_use] extern crate gstuff;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate ser_error_derive;
#[cfg(test)] extern crate mm2_test_helpers;

#[cfg(not(target_arch = "wasm32"))] use common::block_on;
use common::crash_reports::init_crash_reports;
use common::executor::Timer;
use common::log;
use common::log::LogLevel;
use common::password_policy::password_policy;
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};

#[cfg(any(feature = "custom-swap-locktime", test, feature = "run-docker-tests"))]
use common::log::warn;
#[cfg(any(feature = "custom-swap-locktime", test, feature = "run-docker-tests"))]
use lp_swap::PAYMENT_LOCKTIME;
#[cfg(any(feature = "custom-swap-locktime", test, feature = "run-docker-tests"))]
use std::sync::atomic::Ordering;

use gstuff::slurp;
use serde_json::{self as json, Value as Json};

use std::env;
use std::ffi::OsString;
use std::process::exit;
use std::ptr::null;
use std::str;

pub use self::lp_native_dex::init_hw;
pub use self::lp_native_dex::lp_init;
use mm2_err_handle::prelude::*;

#[cfg(not(target_arch = "wasm32"))] pub mod database;

pub mod heartbeat_event;
pub mod lp_dispatcher;
pub mod lp_healthcheck;
pub mod lp_message_service;
mod lp_native_dex;
pub mod lp_network;
pub mod lp_ordermatch;
pub mod lp_stats;
pub mod lp_swap;
pub mod lp_wallet;
pub mod rpc;
mod swap_versioning;
#[cfg(all(target_arch = "wasm32", test))] mod wasm_tests;

pub const PASSWORD_MAXIMUM_CONSECUTIVE_CHARACTERS: usize = 3;

#[cfg(any(feature = "custom-swap-locktime", test, feature = "run-docker-tests"))]
const CUSTOM_PAYMENT_LOCKTIME_DEFAULT: u64 = 900;

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

#[cfg(any(feature = "custom-swap-locktime", test, feature = "run-docker-tests"))]
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
) -> Result<MmArc, String> {
    let log_filter = params.filter.unwrap_or_default();
    // Logger can be initialized once.
    // If `kdf` is linked as a library, and `kdf` is restarted, `init_logger` returns an error.
    init_logger(log_filter, params.conf["silent_console"].as_bool().unwrap_or_default()).ok();

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

    #[cfg(any(feature = "custom-swap-locktime", test, feature = "run-docker-tests"))]
    initialize_payment_locktime(&conf);

    let ctx = MmCtxBuilder::new()
        .with_conf(conf)
        .with_log_level(log_filter)
        .with_version(version.clone())
        .with_datetime(datetime.clone())
        .into_mm_arc();
    ctx_cb(try_s!(ctx.ffi_handle()));

    #[cfg(not(target_arch = "wasm32"))]
    spawn_ctrl_c_handler(ctx.clone());

    try_s!(lp_init(ctx.clone(), version, datetime).await);
    Ok(ctx)
}

pub async fn lp_run(ctx: MmArc) {
    // In the mobile version we might depend on `lp_init` staying around until the context stops.
    loop {
        if ctx.is_stopping() {
            break;
        };
        Timer::sleep(0.2).await
    }

    // Clearing up the running swaps removes any circular references that might prevent the context from being dropped.
    lp_swap::clear_running_swaps(&ctx);
}

/// Handles CTRL-C signals and shutdowns the KDF runtime gracefully.
///
/// It's important to spawn this task as soon as `Ctx` is in the correct state.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_ctrl_c_handler(ctx: MmArc) {
    use crate::lp_dispatcher::{dispatch_lp_event, StopCtxEvent};

    common::executor::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("Couldn't listen for the CTRL-C signal.");

        log::info!("Wrapping things up and shutting down...");

        dispatch_lp_event(ctx.clone(), StopCtxEvent.into()).await;
        ctx.stop().await.expect("Couldn't stop the KDF runtime.");
    });
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
  dbdir          ..  MM database path. 'DB' by default.
  gui            ..  The information about GUI app using KDF instance. Included in swap statuses shared with network.
                 ..  It's recommended to put essential info to this field (application name, OS, version, etc).
                 ..  e.g. AtomicDEX iOS 1.0.1000.
  myipaddr       ..  IP address to bind to for P2P networking.
  netid          ..  Subnetwork. Affects ports and keys.
  passphrase *   ..  Wallet seed.
                     Compressed WIFs and hexadecimal ECDSA keys (prefixed with 0x) are also accepted.
  rpccors        ..  Access-Control-Allow-Origin header value to be used in all the RPC responses.
                     Default is currently 'http://localhost:3000'
  rpcip          ..  IP address to bind to for RPC server. Overrides the 127.0.0.1 default
  rpc_password   ..  RPC password used to authorize non-public RPC calls
                     MM generates password from passphrase if this field is not set
  rpc_local_only ..  MM forbids some RPC requests from not loopback (localhost) IPs as additional security measure.
                     Defaults to `true`, set `false` to disable. `Use with caution`.
  rpcport        ..  If > 1000 overrides the 7783 default.
  i_am_seed      ..  Activate the seed node mode (acting as a relay for kdf clients).
                     Defaults to `false`.
  seednodes      ..  Seednode IPs that node will use.
                     At least one seed IP must be present if the node is not a seed itself.
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

    if first_arg == Some("--version") || first_arg == Some("-v") || first_arg == Some("version") {
        println!("Komodo DeFi Framework: {version}");
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

    log!("Komodo DeFi Framework {} DT {}", version, datetime);

    if let Err(err) = run_lp_main(first_arg, &|_| (), version, datetime) {
        log!("{}", err);
        exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Parses and returns the `first_arg` as JSON.
/// Attempts to load the config from `MM2.json` file if `first_arg` is None
pub fn get_mm2config(first_arg: Option<&str>) -> Result<Json, String> {
    let conf = match first_arg {
        Some(s) => s.to_owned(),
        None => {
            let conf_path = common::kdf_config_file().map_err(|e| e.to_string())?;
            let conf_from_file = slurp(&conf_path);

            if conf_from_file.is_empty() {
                return ERR!(
                    "Config is not set from command line arg and {} file doesn't exist.",
                    conf_path.display()
                );
            }
            try_s!(String::from_utf8(conf_from_file))
        },
    };

    let mut conf: Json = match json::from_str(&conf) {
        Ok(json) => json,
        // Syntax or io errors may include the conf string in the error message so we don't want to take risks and show these errors internals in the log.
        // If new variants are added to the Error enum, there can be a risk of exposing the conf string in the error message when updating serde_json so
        // I think it's better to not include the serde_json::error::Error at all in the returned error message rather than selectively excluding certain variants.
        Err(_) => return ERR!("Couldn't parse mm2 config to JSON format!"),
    };

    if conf["coins"].is_null() {
        let coins_path = common::kdf_coins_file().map_err(|e| e.to_string())?;

        let coins_from_file = slurp(&coins_path);
        if coins_from_file.is_empty() {
            return ERR!(
                "No coins are set in JSON config and '{}' file doesn't exist",
                coins_path.display()
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
    let conf = get_mm2config(first_arg)?;

    let log_filter = LogLevel::from_env();

    let params = LpMainParams::with_conf(conf).log_filter(log_filter);
    let ctx = try_s!(block_on(lp_main(params, ctx_cb, version, datetime)));
    block_on(lp_run(ctx));
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn init_logger(_level: LogLevel, silent_console: bool) -> Result<(), String> {
    common::log::UnifiedLoggerBuilder::default()
        .silent_console(silent_console)
        .init();

    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn init_logger(level: LogLevel, _silent_console: bool) -> Result<(), String> {
    common::log::WasmLoggerBuilder::default().level_filter(level).try_init()
}
