
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
//  rpc_commands.rs
//  marketmaker
//

#![cfg_attr(not(feature = "native"), allow(dead_code))]
#![cfg_attr(not(feature = "native"), allow(unused_imports))]

use coins::{disable_coin as disable_coin_impl, lp_coinfind, lp_coininit, MmCoinEnum};
use common::{rpc_err_response, rpc_response, HyRes, MM_VERSION};
use common::executor::{spawn, Timer};
use common::mm_ctx::MmArc;
use futures01::Future;
use futures::compat::Future01CompatExt;
use http::Response;
use serde_json::{self as json, Value as Json};

use crate::mm2::lp_ordermatch::{CancelBy, cancel_orders_by};
use crate::mm2::lp_swap::{get_locked_amount, active_swaps_using_coin};

/// Attempts to disables the coin
pub fn disable_coin (ctx: MmArc, req: Json) -> HyRes {
    let ticker = try_h!(req["coin"].as_str().ok_or ("No 'coin' field")).to_owned();
    let _coin = match lp_coinfind (&ctx, &ticker) {
        Ok (Some (t)) => t,
        Ok (None) => return rpc_err_response (500, &fomat! ("No such coin: " (ticker))),
        Err (err) => return rpc_err_response (500, &fomat! ("!lp_coinfind(" (ticker) "): " (err)))
    };
    let swaps = try_h!(active_swaps_using_coin(&ctx, &ticker));
    if !swaps.is_empty() {
        return rpc_response (500, json!({
            "error": fomat! ("There're active swaps using " (ticker)),
            "swaps": swaps,
        }).to_string());
    }
    let (cancelled, still_matching) = try_h!(cancel_orders_by(&ctx, CancelBy::Coin{ ticker: ticker.clone() }));
    if !still_matching.is_empty() {
        return rpc_response (500, json!({
            "error": fomat! ("There're currently matching orders using " (ticker)),
            "orders": {
                "matching": still_matching,
                "cancelled": cancelled,
            }
        }).to_string());
    }

    try_h!(disable_coin_impl(&ctx, &ticker));
    rpc_response(200, json!({
        "result": {
            "coin": ticker,
            "cancelled_orders": cancelled,
        }
    }).to_string())
}

/// Enable a coin in the Electrum mode.
pub async fn electrum (ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s! (req["coin"].as_str().ok_or ("No 'coin' field")).to_owned();
    let coin: MmCoinEnum = try_s! (lp_coininit (&ctx, &ticker, &req) .await);
    let balance = try_s! (coin.my_balance().compat().await);
    let res = json! ({
        "result": "success",
        "address": coin.my_address(),
        "balance": balance,
        "locked_by_swaps": get_locked_amount (&ctx, &ticker),
        "coin": coin.ticker(),
        "required_confirmations": coin.required_confirmations(),
    });
    let res = try_s! (json::to_vec (&res));
    Ok (try_s! (Response::builder().body (res)))
}

/// Enable a coin in the local wallet mode.
pub async fn enable (ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s! (req["coin"].as_str().ok_or ("No 'coin' field")).to_owned();
    let coin: MmCoinEnum = try_s! (lp_coininit (&ctx, &ticker, &req) .await);
    let balance = try_s! (coin.my_balance().compat().await);
    let res = json! ({
        "result": "success",
        "address": coin.my_address(),
        "balance": balance,
        "locked_by_swaps": get_locked_amount (&ctx, &ticker),
        "coin": coin.ticker(),
        "required_confirmations": coin.required_confirmations(),
    });
    let res = try_s! (json::to_vec (&res));
    Ok (try_s! (Response::builder().body (res)))
}

pub fn help() -> HyRes {
    rpc_response(200, "
        buy(base, rel, price, relvolume, timeout=10, duration=3600)
        electrum(coin, urls)
        enable(coin, urls, swap_contract_address)
        myprice(base, rel)
        my_balance(coin)
        my_swap_status(params/uuid)
        orderbook(base, rel, duration=3600)
        sell(base, rel, price, basevolume, timeout=10, duration=3600)
        send_raw_transaction(coin, tx_hex)
        setprice(base, rel, price, broadcast=1)
        stop()
        version
        withdraw(coin, amount, to)
    ")
}

/// Get my_balance of a coin
pub fn my_balance (ctx: MmArc, req: Json) -> HyRes {
    let ticker = try_h! (req["coin"].as_str().ok_or ("No 'coin' field")).to_owned();
    let coin = match lp_coinfind (&ctx, &ticker) {
        Ok (Some (t)) => t,
        Ok (None) => return rpc_err_response (500, &fomat! ("No such coin: " (ticker))),
        Err (err) => return rpc_err_response (500, &fomat! ("!lp_coinfind(" (ticker) "): " (err)))
    };
    Box::new(coin.my_balance().and_then(move |balance| rpc_response(200, json!({
        "coin": ticker,
        "balance": balance,
        "locked_by_swaps": get_locked_amount(&ctx, &ticker),
        "address": coin.my_address(),
    }).to_string())))
}

/*
AP: Passphrase call is not documented and not used as of now, commented out

/// JSON structure passed to the "passphrase" RPC call.  
/// cf. https://docs.komodoplatform.com/barterDEX/barterDEX-API.html#passphrase
#[derive(Clone, Deserialize, Debug)]
struct PassphraseReq {
    passphrase: String,
    /// Optional because we're checking the `passphrase` hash first.
    userpass: Option<String>,
    /// Defaults to "cli" (in `lp_passphrase_init`).
    gui: Option<String>,
    seednodes: Option<Vec<String>>
}

pub fn passphrase (ctx: MmArc, req: Json) -> HyRes {
    let matching_userpass = super::auth (&req, &ctx) .is_ok();
    let req: PassphraseReq = try_h! (json::from_value (req));

    let mut passhash: bits256 = unsafe {zeroed()};
    unsafe {lp::vcalc_sha256 (null_mut(), passhash.bytes.as_mut_ptr(), req.passphrase.as_ptr() as *mut u8, req.passphrase.len() as i32)};
    let matching_passphrase = unsafe {passhash.bytes == lp::G.LP_passhash.bytes};
    if !matching_passphrase {
        log! ({"passphrase] passhash {} != G {}", passhash, unsafe {bits256::from (lp::G.LP_passhash)}});
        if !matching_userpass {return rpc_err_response (500, "authentication error")}
    }

    unsafe {lp::G.USERPASS_COUNTER = 1}

    unsafe {try_h! (lp_passphrase_init (Some (&req.passphrase), req.gui.as_ref().map (|s| &s[..])))};

    let mut coins = Vec::new();
    try_h! (unsafe {coins_iter (&mut |coin| {
        let coin_json = lp::LP_coinjson (coin, lp::LP_showwif);
        let cjs = lp::jprint (coin_json, 1);
        let cjs_copy = Vec::from (CStr::from_ptr (cjs) .to_bytes());
        free (cjs as *mut c_void);
        lp::free_json (coin_json);
        let rcjs: Json = try_s! (json::from_slice (&cjs_copy));
        coins.push (rcjs);
        Ok(())
    })});

    let retjson = json! ({
        "result": "success",
        "userpass": try_h! (unsafe {CStr::from_ptr (lp::G.USERPASS.as_ptr())} .to_str()),
        "mypubkey": fomat! ((unsafe {bits256::from (lp::G.LP_mypub25519.bytes)})),
        "pubsecp": hex::encode (unsafe {&lp::G.LP_pubsecp[..]}),
        "KMD": try_h! (bitcoin_address ("KMD", 60, unsafe {lp::G.LP_myrmd160})),
        "BTC": try_h! (bitcoin_address ("BTC", 0, unsafe {lp::G.LP_myrmd160})),
        "NXT": try_h! (unsafe {CStr::from_ptr (lp::G.LP_NXTaddr.as_ptr())} .to_str()),
        "coins": coins
    });

    rpc_response (200, try_h! (json::to_string (&retjson)))
}
*/
pub fn stop (ctx: MmArc) -> HyRes {
    // Should delay the shutdown a bit in order not to trip the "stop" RPC call in unit tests.
    // Stopping immediately leads to the "stop" RPC call failing with the "errno 10054" sometimes.
    spawn (async move {
        Timer::sleep (0.05) .await;
        ctx.stop();
    });
    rpc_response (200, r#"{"result": "success"}"#)
}

pub fn version() -> HyRes { rpc_response(200, json!({"result": MM_VERSION}).to_string()) }

// AP: Inventory is not documented and not used as of now, commented out
/*
pub fn inventory (ctx: MmArc, req: Json) -> HyRes {
    let ticker = match req["coin"].as_str() {Some (s) => s, None => return rpc_err_response (500, "No 'coin' argument in request")};
    let coin = match lp_coinfind (&ctx, ticker) {
        Ok (Some (t)) => t,
        Ok (None) => return rpc_err_response (500, &fomat! ("No such coin: " (ticker))),
        Err (err) => return rpc_err_response (500, &fomat! ("!lp_coinfind(" (ticker) "): " (err)))
    };
    let ii = coin.iguana_info();

    unsafe {lp::LP_address (ii, (*ii).smartaddr.as_mut_ptr())};
    if unsafe {nonz (lp::G.LP_privkey.bytes)} {
        unsafe {lp::LP_privkey_init (-1, ii, lp::G.LP_privkey, lp::G.LP_mypub25519)};
    } else {
        log! ("inventory] no LP_privkey");
    }
    let retjson = json! ({
        "result": "success",
        "coin": ticker,
        "timestamp": now_ms() / 1000,
        "alice": []  // LP_inventory(coin)
        // "bob": LP_inventory(coin,1)
    });
    //LP_smartutxos_push(ptr);
    rpc_response (200, try_h! (json::to_string (&retjson)))
}
*/
