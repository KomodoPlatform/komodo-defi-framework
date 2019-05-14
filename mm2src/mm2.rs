/******************************************************************************
 * Copyright © 2014-2018 The SuperNET Developers.                             *
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
//  mm2.rs
//  marketmaker
//
//  Copyright © 2017-2018 SuperNET. All rights reserved.
//

use common::{bitcoin_priv2wif, lp, os, BitcoinCtx, CJSON, MM_VERSION};
use common::lp::{_bits256 as bits256};
use common::mm_ctx::MmCtx;

use gstuff::{now_ms, slurp};

use libc::{c_char};

use rand::random;

use serde_json::{self as json, Value as Json};

use std::env;
use std::ffi::{CStr, CString, OsString};
use std::mem::{zeroed};
use std::process::exit;
use std::ptr::{null};
use std::str::from_utf8_unchecked;
use std::slice::from_raw_parts;
use std::str;

#[path = "crash_reports.rs"]
pub mod crash_reports;
use self::crash_reports::init_crash_reports;

#[path = "lp_native_dex.rs"]
mod lp_native_dex;
use self::lp_native_dex::{lp_init};

#[path = "lp_network.rs"]
pub mod lp_network;

#[path = "lp_ordermatch.rs"]
pub mod lp_ordermatch;
#[path = "lp_swap.rs"]
pub mod lp_swap;
#[path = "rpc.rs"]
pub mod rpc;

#[cfg(test)]
#[path = "mm2_tests.rs"]
mod mm2_tests;

fn lp_main (c_conf: CJSON, conf: Json, ctx_cb: &Fn (u32)) -> Result<(), String> {
    // Redirects the C stdout to the log.
    let c_log_path_buf: CString;
    let c_log_path = if conf["log"].is_null() {null()} else {
        let log = try_s! (conf["log"].as_str().ok_or ("log is not a string"));
        c_log_path_buf = try_s! (CString::new (log));
        c_log_path_buf.as_ptr()
    };
    unsafe {lp::unbuffered_output_support (c_log_path)};

    if !conf["rpc_password"].is_null() {
        if !conf["rpc_password"].is_string() {
            return ERR!("rpc_password must be string");
        }

        if conf["rpc_password"].as_str() == Some("") {
            return ERR!("rpc_password must not be empty");
        }
    }

    let (mut pullport, mut pubport, mut busport) = (0, 0, 0);
    if conf["passphrase"].is_string() {
        let profitmargin = conf["profitmargin"].as_f64();
        unsafe {lp::LP_profitratio += profitmargin.unwrap_or (0.)};
        let netid = conf["netid"].as_u64().unwrap_or (0) as u16;
        unsafe {lp::LP_ports (&mut pullport, &mut pubport, &mut busport, netid)};
        try_s! (lp_init (pullport, pubport, conf, c_conf, ctx_cb));
        Ok(())
    } else {ERR! ("!passphrase")}
}

fn help() {
    // Removed options:
    // "client" - In MM2 anyone can be a Maker, the "client" option is no longer applicable.

    pintln! (
        "Command-line options.\n"
        "The first command-line argument is special and designates the mode.\n"
        "\n"
        "  help                  ..  Display this message.\n"
        "  btc2kmd {WIF or BTC}  ..  Convert a BTC WIF into a KMD WIF.\n"
        "  events                ..  Listen to a feed coming from a separate MM daemon and print it to stdout.\n"
        "  vanity {substring}    ..  Tries to find an address with the given substring.\n"
        "  nxt                   ..  Query the local NXT client (port 7876) regarding the SuperNET account in NXT.\n"
        "  {JSON configuration}  ..  Run the MarketMaker daemon.\n"
        "\n"
        "Some (but not all) of the JSON configuration parameters (* - required):\n"
        "\n"
        "  canbind        ..  If > 1000 and < 65536, initializes the `LP_fixed_pairport`.\n"
        // We don't want to break the existing RPC API,
        // so the "refrel=coinmarketcap" designator will act as autoselect,
        // using the CoinGecko behind the scenes unless the "cmc_key" is given.
        // In the future, when MM2 is more widely used and thus we're working more tighly with the GUIs (BarterDEX, HyperDEX, dICO),
        // we might add the "refrel=cmc" and "refrel=coingecko" RPC options.
        "  cmc_key        ..  CoinMarketCap Professional API Key. Switches from CoinGecko to CoinMarketCap.\n"
        "                     The Key can be obtained from 'https://pro.coinmarketcap.com/account'.\n"
        "                     NB: The 'coins' command-line configuration must have the lowercased coin names in the 'name' field,\n"
      r#"                     {"coins": [{"name": "dash", "coin": "DASH", ...}, ...], ...}."# "\n"
        // cf. https://github.com/atomiclabs/hyperdex/blob/1d4ed3234b482e769124725c7e979eef5cd72d24/app/marketmaker/supported-currencies.js#L12
        "  coins          ..  Information about the currencies: their ticker symbols, names, ports, addresses, etc.\n"
        "                     If the field isn't present on the command line then we try loading it from the 'coins' file.\n"
        "  dbdir          ..  MM database path. 'DB' by default.\n"
        "  log            ..  File path. Redirect (as of now only a part of) the log there.\n"
        "  myipaddr       ..  IP address to bind to for P2P networking.\n"
        "  netid          ..  Subnetwork. Affects ports and keys.\n"
        "  passphrase *   ..  Wallet seed.\n"
        "  profitmargin   ..  Adds to `LP_profitratio`.\n"
        // cf. https://github.com/atomiclabs/hyperdex/pull/563/commits/6d17c0c994693b768e30130855c679a7849a2b27
        "  rpccors        ..  Access-Control-Allow-Origin header value to be used in all the RPC responses.\n"
        "                     Default is currently 'http://localhost:3000'\n"
        "  rpcip          ..  IP address to bind to for RPC server. Overrides the 127.0.0.1 default\n"
        "  rpc_password   ..  RPC password used to authorize non-public RPC calls\n"
        "                     MM generates password from passphrase if this field is not set\n"
        "  rpc_local_only ..  MM forbids some RPC requests from not loopback (localhost) IPs as additional security measure.\n"
        "                     Defaults to `true`, set `false` to disable. `Use with caution`.\n"
        "  rpcport        ..  If > 1000 overrides the 7783 default.\n"
        "  i_am_seed      ..  Activate the seed node mode (acting as a relay for mm2 clients).\n"
        "                     Defaults to `false`.\n"
        "  seednodes      ..  Seednode IPs that node will use. At least 1 seed IP be set if the node is not seed itself.\n"
        "  userhome       ..  System home directory of a user ('/root' by default).\n"
        "  wif            ..  `1` to add WIFs to the information we provide about a coin.\n"
        "\n"
        // Generated from https://github.com/KomodoPlatform/Documentation (PR to dev branch).
        // SHossain: "this would be the URL we would recommend and it will be maintained
        //            Please let @gcharang or me know if anything needs updating there".
        // P.S.
        // siddhartha-crypto and artemii235 worked on updating the docs here:
        // https://github.com/KomodoPlatform/developer-docs/tree/mm/docs/basic-docs/atomic-swap-dex
        "See also the online documentation at https://docs.komodoplatform.com/barterDEX/barterDEX-API.html."
    )
}

pub fn mm2_main() {
    init_crash_reports();
    unsafe {os::OS_init()};
    log!({"BarterDEX MarketMaker {}", MM_VERSION});

    // Temporarily simulate `argv[]` for the C version of the main method.
    let args: Vec<String> = env::args().map (|mut arg| {arg.push ('\0'); arg}) .collect();
    let mut args: Vec<*const c_char> = args.iter().map (|s| s.as_ptr() as *const c_char) .collect();
    args.push (null());

    let args_os: Vec<OsString> = env::args_os().collect();

    // NB: The first argument is special, being used as the mode switcher.
    // The other arguments might be used to pass the data to the various MM modes,
    // we're not checking them for the mode switches in order not to risk [untrusted] data being mistaken for a mode switch.
    let first_arg = args_os.get (1) .and_then (|arg| arg.to_str());

    if first_arg == Some ("btc2kmd") && args_os.get (2) .is_some() {
        match btc2kmd (unwrap! (args_os[2].to_str(), "Bad argument encoding")) {
            Ok (output) => log! ((output)),
            Err (err) => log! ({"btc2kmd error] {}", err})
        }
        return
    }

    let second_arg = args_os.get (2) .and_then (|arg| arg.to_str());
    if first_arg == Some ("vanity") && second_arg.is_some() {vanity (unwrap! (second_arg)); return}

    if first_arg == Some ("--help") || first_arg == Some ("-h") || first_arg == Some ("help") {help(); return}
    if cfg! (windows) && first_arg == Some ("/?") {help(); return}

    if let Err (err) = run_lp_main (first_arg, &|_|()) {
        log! ((err));
        exit (1);
    }
}

// TODO: `btc2kmd` is *pure*, it doesn't use shared state,
// though some of the underlying functions (`LP_convaddress`) do (the hash of cryptocurrencies is shared).
// Should mark it as shallowly pure.

/// Implements the "btc2kmd" command line utility.
fn btc2kmd (wif_or_btc: &str) -> Result<String, String> {
    extern "C" {
        fn LP_wifstr_valid (symbol: *const u8, wifstr: *const u8) -> i32;
        fn LP_convaddress (symbol: *const u8, address: *const u8, dest: *const u8) -> *const c_char;
        fn bitcoin_wif2priv (symbol: *const u8, wiftaddr: u8, addrtypep: *mut u8, privkeyp: *mut bits256, wifstr: *const c_char) -> i32;
    }

    let wif_or_btc_z = format! ("{}\0", wif_or_btc);
    /* (this line helps the IDE diff to match the old and new code)
    if ( strstr(argv[0],"btc2kmd") != 0 && argv[1] != 0 )
    */
    let mut privkey: bits256 = unsafe {zeroed()};
    let mut checkkey: bits256 = unsafe {zeroed()};
    let mut tmptype = 0;
    let mut kmdwif: [c_char; 64] = unsafe {zeroed()};
    if unsafe {LP_wifstr_valid (b"BTC\0".as_ptr(), wif_or_btc_z.as_ptr())} > 0 {
        let rc = unsafe {bitcoin_wif2priv (b"BTC\0".as_ptr(), 0, &mut tmptype, &mut privkey, wif_or_btc_z.as_ptr() as *const c_char)};
        if rc < 0 {return ERR! ("!bitcoin_wif2priv")}
        let rc = unsafe {bitcoin_priv2wif (b"KMD\0".as_ptr(), 0, kmdwif.as_mut_ptr(), privkey, 188)};
        if rc < 0 {return ERR! ("!bitcoin_priv2wif")}
        let rc = unsafe {bitcoin_wif2priv (b"KMD\0".as_ptr(), 0, &mut tmptype, &mut checkkey, kmdwif.as_ptr())};
        if rc < 0 {return ERR! ("!bitcoin_wif2priv")}
        let kmdwif = try_s! (unsafe {CStr::from_ptr (kmdwif.as_ptr())} .to_str());
        if privkey == checkkey {
            Ok (format! ("BTC {} -> KMD {}: privkey {}", wif_or_btc, kmdwif, privkey))
        } else {
            Err (format! ("ERROR BTC {} {} != KMD {} {}", wif_or_btc, privkey, kmdwif, checkkey))
        }
    } else {
        let retstr = unsafe {LP_convaddress(b"BTC\0".as_ptr(), wif_or_btc_z.as_ptr(), b"KMD\0".as_ptr())};
        if retstr == null() {return ERR! ("LP_convaddress")}
        Ok (unwrap! (unsafe {CStr::from_ptr (retstr)} .to_str()) .into())
    }
}

fn vanity (substring: &str) {
    extern "C" {
        fn bitcoin_priv2pub (
            ctx: *mut BitcoinCtx, symbol: *const u8, pubkey33: *mut u8, coinaddr: *mut u8,
            privkey: bits256, taddr: u8, addrtype: u8);
    }
    /*
    else if ( argv[1] != 0 && strcmp(argv[1],"vanity") == 0 && argv[2] != 0 )
    */
    let mut pubkey33: [u8; 33] = unsafe {zeroed()};
    let mut coinaddr: [u8; 64] = unsafe {zeroed()};
    let mut wifstr: [c_char; 128] = unsafe {zeroed()};
    let mut privkey: bits256 = unsafe {zeroed()};
    unsafe {lp::LP_mutex_init()};
    let ctx = MmCtx::new (json! ({}), [0; 20].into());
    let timestamp = now_ms() / 1000;
    log! ({"start vanitygen ({}).{} t.{}", substring, substring.len(), timestamp});
    for i in 0..1000000000 {
        privkey.bytes = random();
        unsafe {bitcoin_priv2pub (ctx.btc_ctx(), "KMD\0".as_ptr(), pubkey33.as_mut_ptr(), coinaddr.as_mut_ptr(), privkey, 0, 60)};
        let coinaddr = unsafe {from_utf8_unchecked (from_raw_parts (coinaddr.as_ptr(), 34))};
        if &coinaddr[1 .. substring.len()] == &substring[0 .. substring.len() - 1] {  // Print on near match.
            unsafe {bitcoin_priv2wif ("KMD\0".as_ptr(), 0, wifstr.as_mut_ptr(), privkey, 188)};
            let wifstr = unwrap! (unsafe {CStr::from_ptr (wifstr.as_ptr())} .to_str());
            log! ({"i.{} {} -> {} wif.{}", i, privkey, coinaddr, wifstr});
            if coinaddr.as_bytes()[substring.len()] == substring.as_bytes()[substring.len() - 1] {break}  // Stop on full match.
        }
    }
    log! ({"done vanitygen.({}) done {} elapsed {}\n", substring, now_ms() / 1000, now_ms() / 1000 - timestamp});
}

/// Parses the `first_arg` as JSON and runs LP_main.
/// Attempts to load the config from `MM2.json` file if `first_arg` is None
/// 
/// * `ctx_cb` - Invoked with the MM context handle,
///              allowing the `run_lp_main` caller to communicate with MM.
pub fn run_lp_main (first_arg: Option<&str>, ctx_cb: &Fn (u32)) -> Result<(), String> {
    let conf_from_file = slurp(&"MM2.json");
    let conf = match first_arg {
        Some(s) => s,
        None => {
            if conf_from_file.is_empty() {
                return ERR!("Config is not set from command line arg and MM2.json file doesn't exist.");
            }
            try_s!(std::str::from_utf8(&conf_from_file))
        }
    };

    let c_conf = match CJSON::from_str (conf) {
        Ok (json) => json,
        Err (err) => return ERR! ("couldnt parse.({}).{}", conf, err)
    };
    let mut conf: Json = match json::from_str(conf) {
        Ok (json) => json,
        Err (err) => return ERR! ("couldnt parse.({}).{}", conf, err)
    };

    if conf["coins"].is_null() {
        let coins_from_file = slurp(&std::path::Path::new("coins"));
        if coins_from_file.is_empty() {
            return ERR!("No coins are set in JSON config and 'coins' file doesn't exist");
        }
        conf["coins"] = try_s!(json::from_slice(&coins_from_file));
    }

    if conf["docker"] == 1 {
        unsafe {lp::DOCKERFLAG = 1}
    } else if conf["docker"].is_string() {
        let ip_port = unwrap! (CString::new (unwrap! (conf["docker"].as_str())));
        unsafe {lp::DOCKERFLAG = os::calc_ipbits (ip_port.as_ptr() as *mut c_char) as u32}
    }

    try_s! (lp_main (c_conf, conf, ctx_cb));
    Ok(())
}
