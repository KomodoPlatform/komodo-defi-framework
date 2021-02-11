#![cfg_attr(not(feature = "native"), allow(unused_variables))]

use super::lp_main;
use bigdecimal::BigDecimal;
#[cfg(not(feature = "native"))] use common::call_back;
use common::executor::Timer;
#[cfg(feature = "native")] use common::for_tests::mm_dump;
use common::for_tests::{check_my_swap_status, check_recent_swaps, check_stats_swap_status, enable_electrum,
                        enable_native, enable_qrc20, find_metrics_in_json, from_env_file, get_passphrase, mm_spat,
                        LocalStart, MarketMakerIt, RaiiDump, MAKER_ERROR_EVENTS, MAKER_SUCCESS_EVENTS,
                        TAKER_ERROR_EVENTS, TAKER_SUCCESS_EVENTS};
use common::mm_metrics::{MetricType, MetricsJson};
use common::mm_number::Fraction;
use common::privkey::key_pair_from_seed;
use common::BigInt;
use common::{block_on, slurp};
use http::StatusCode;
#[cfg(feature = "native")]
use hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN;
use num_rational::BigRational;
use serde_json::{self as json, Value as Json};
use std::collections::HashMap;
use std::convert::identity;
use std::env::{self, var};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

#[path = "mm2_tests/structs.rs"] mod structs;
use structs::*;

// TODO: Consider and/or try moving the integration tests into separate Rust files.
// "Tests in your src files should be unit tests, and tests in tests/ should be integration-style tests."
// - https://doc.rust-lang.org/cargo/guide/tests.html

async fn enable_coins_eth_electrum(mm: &MarketMakerIt, eth_urls: Vec<&str>) -> HashMap<&'static str, Json> {
    let mut replies = HashMap::new();
    replies.insert(
        "RICK",
        enable_electrum(mm, "RICK", vec![
            "electrum1.cipig.net:10017",
            "electrum2.cipig.net:10017",
            "electrum3.cipig.net:10017",
        ])
        .await,
    );
    replies.insert(
        "MORTY",
        enable_electrum(mm, "MORTY", vec![
            "electrum1.cipig.net:10018",
            "electrum2.cipig.net:10018",
            "electrum3.cipig.net:10018",
        ])
        .await,
    );
    replies.insert("ETH", enable_native(mm, "ETH", eth_urls.clone()).await);
    replies.insert("JST", enable_native(mm, "JST", eth_urls).await);
    replies
}

fn addr_from_enable(enable_response: &Json) -> Json { enable_response["address"].clone() }

fn rmd160_from_passphrase(passphrase: &str) -> [u8; 20] {
    key_pair_from_seed(passphrase).unwrap().public().address_hash().take()
}

/*
portfolio is removed from dependencies temporary
#[test]
#[ignore]
fn test_autoprice_coingecko() {portfolio::portfolio_tests::test_autoprice_coingecko (local_start())}

#[test]
#[ignore]
fn test_autoprice_coinmarketcap() {portfolio::portfolio_tests::test_autoprice_coinmarketcap (local_start())}

#[test]
fn test_fundvalue() {portfolio::portfolio_tests::test_fundvalue (local_start())}
*/

/// Integration test for RPC server.
/// Check that MM doesn't crash in case of invalid RPC requests
#[test]
#[cfg(feature = "native")]
fn test_rpc() {
    let (_, mut mm, _dump_log, _dump_dashboard) = mm_spat(local_start(), &identity);
    unwrap!(block_on(
        mm.wait_for_log(19., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    let no_method = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "coin": "RICK",
        "ipaddr": "electrum1.cipig.net",
        "port": 10017
    }))));
    assert!(no_method.0.is_server_error());
    assert_eq!((no_method.2)[ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:4000");

    let not_json = unwrap!(mm.rpc_str("It's just a string"));
    assert!(not_json.0.is_server_error());
    assert_eq!((not_json.2)[ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:4000");

    let unknown_method = unwrap!(block_on(mm.rpc(json! ({
        "method": "unknown_method",
    }))));

    assert!(unknown_method.0.is_server_error());
    assert_eq!((unknown_method.2)[ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:4000");

    let version = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "version",
    }))));
    assert_eq!(version.0, StatusCode::OK);
    assert_eq!((version.2)[ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:4000");

    let help = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "help",
    }))));
    assert_eq!(help.0, StatusCode::OK);
    assert_eq!((help.2)[ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:4000");

    unwrap!(block_on(mm.stop()));
    // unwrap! (mm.wait_for_log (9., &|log| log.contains ("on_stop] firing shutdown_tx!")));
    // TODO (workaround libtorrent hanging in delete) // unwrap! (mm.wait_for_log (9., &|log| log.contains ("LogState] Bye!")));
}

/// This is not a separate test but a helper used by `MarketMakerIt` to run the MarketMaker from the test binary.
#[test]
fn test_mm_start() {
    if let Ok(conf) = var("_MM2_TEST_CONF") {
        log!("test_mm_start] Starting the MarketMaker...");
        let conf: Json = unwrap!(json::from_str(&conf));
        unwrap!(lp_main(conf, &|_ctx| ()))
    }
}

#[allow(unused_variables)]
fn chdir(dir: &Path) {
    #[cfg(feature = "native")]
    {
        #[cfg(not(windows))]
        {
            use std::ffi::CString;
            let dir_s = unwrap!(dir.to_str());
            let dir_c = unwrap!(CString::new(dir_s));
            let rc = unsafe { libc::chdir(dir_c.as_ptr()) };
            assert_eq!(rc, 0, "Can not chdir to {:?}", dir);
        }

        #[cfg(windows)]
        {
            use std::ffi::CString;
            use winapi::um::processenv::SetCurrentDirectoryA;
            let dir = unwrap!(dir.to_str());
            let dir = unwrap!(CString::new(dir));
            // https://docs.microsoft.com/en-us/windows/desktop/api/WinBase/nf-winbase-setcurrentdirectory
            let rc = unsafe { SetCurrentDirectoryA(dir.as_ptr()) };
            assert_ne!(rc, 0);
        }
    }
}

/// Typically used when the `LOCAL_THREAD_MM` env is set, helping debug the tested MM.  
/// NB: Accessing `lp_main` this function have to reside in the mm2 binary crate. We pass a pointer to it to subcrates.
#[cfg(feature = "native")]
fn local_start_impl(folder: PathBuf, log_path: PathBuf, mut conf: Json) {
    unwrap!(thread::Builder::new().name("MM".into()).spawn(move || {
        if conf["log"].is_null() {
            conf["log"] = unwrap!(log_path.to_str()).into();
        } else {
            let path = Path::new(unwrap!(conf["log"].as_str(), "log is not a string"));
            assert_eq!(log_path, path);
        }

        log! ({"local_start] MM in a thread, log {:?}.", log_path});

        chdir(&folder);

        unwrap!(lp_main(conf, &|_ctx| ()))
    }));
}

/// Starts the WASM version of MM.
#[cfg(not(feature = "native"))]
fn wasm_start_impl(ctx: MmArc) {
    crate::mm2::rpc::init_header_slots();

    let netid = ctx.conf["netid"].as_u64().unwrap_or(0) as u16;
    let (_, pubport, _) = unwrap!(super::lp_ports(netid));
    common::executor::spawn(async move {
        unwrap!(super::lp_init(pubport, ctx).await);
    })
}

#[cfg(feature = "native")]
fn local_start() -> LocalStart { local_start_impl }

#[cfg(not(feature = "native"))]
fn local_start() -> LocalStart { wasm_start_impl }

macro_rules! local_start {
    ($who: expr) => {
        if cfg!(feature = "native") {
            match var("LOCAL_THREAD_MM") {
                Ok(ref e) if e == $who => Some(local_start()),
                _ => None,
            }
        } else {
            Some(local_start())
        }
    };
}

/// https://github.com/artemii235/SuperNET/issues/241
#[test]
#[cfg(feature = "native")]
fn alice_can_see_the_active_order_after_connection() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"},"rpcport":80},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20", "protocol_data":{"platform":"ETH","contract_address":"0xc0eb7AeD740E1796992A08962c15661bDEB58003"}}}
    ]);

    // start bob and immediately place the order
    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        local_start!("bob")
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    // Enable coins on Bob side. Print the replies in case we need the "address".
    log!({ "enable_coins (bob): {:?}", block_on(enable_coins_eth_electrum(&mm_bob, vec!["https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b"])) });
    // issue sell request on Bob side by setting base/rel price
    log!("Issue bob sell request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "MORTY",
        "price": 0.9,
        "volume": "0.9",
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    // Bob orderbook must show the new order
    log!("Get RICK/MORTY orderbook on Bob side");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Bob orderbook "[bob_orderbook]);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert!(asks.len() > 0, "Bob RICK/MORTY asks are empty");
    assert_eq!(Json::from("0.9"), asks[0]["maxvolume"]);

    // start eve and immediately place the order
    let mut mm_eve = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "eve passphrase",
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": [fomat!((mm_bob.ip))],
        }),
        "pass".into(),
        local_start!("bob")
    ));
    let (_eve_dump_log, _eve_dump_dashboard) = mm_dump(&mm_eve.log_path);
    log!({ "Eve log path: {}", mm_eve.log_path.display() });
    unwrap!(block_on(
        mm_eve.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    // Enable coins on Eve side. Print the replies in case we need the "address".
    log!({ "enable_coins (eve): {:?}", block_on(enable_coins_eth_electrum(&mm_eve, vec!["https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b"])) });
    // issue sell request on Eve side by setting base/rel price
    log!("Issue eve sell request");
    let rc = unwrap!(block_on(mm_eve.rpc(json! ({
        "userpass": mm_eve.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "MORTY",
        "price": "1",
        "volume": "0.9",
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    // issue sell request on Eve side by setting base/rel price
    log!("Issue eve sell request");
    let rc = unwrap!(block_on(mm_eve.rpc(json! ({
        "userpass": mm_eve.userpass,
        "method": "setprice",
        "base": "MORTY",
        "rel": "RICK",
        "price": "1",
        "volume": "0.9",
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    log!("Get RICK/MORTY orderbook on Eve side");
    let rc = unwrap!(block_on(mm_eve.rpc(json! ({
        "userpass": mm_eve.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let eve_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Eve orderbook "[eve_orderbook]);
    let asks = eve_orderbook["asks"].as_array().unwrap();
    let bids = eve_orderbook["bids"].as_array().unwrap();
    assert_eq!(asks.len(), 2, "Eve RICK/MORTY orderbook must have exactly 2 asks");
    assert_eq!(bids.len(), 1, "Eve RICK/MORTY orderbook must have exactly 1 bid");

    log!("Give Bob 2 seconds to import Eve order");
    thread::sleep(Duration::from_secs(2));
    log!("Get RICK/MORTY orderbook on Bob side");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Bob orderbook "[bob_orderbook]);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    let bids = bob_orderbook["bids"].as_array().unwrap();
    assert_eq!(asks.len(), 2, "Bob RICK/MORTY orderbook must have exactly 2 asks");
    assert_eq!(bids.len(), 1, "Bob RICK/MORTY orderbook must have exactly 1 bid");

    let mut mm_alice = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": "alice passphrase",
            "coins": coins,
            "seednodes": [fomat!((mm_bob.ip))],
            "rpc_password": "pass",
        }),
        "pass".into(),
        local_start!("alice")
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
    log!({ "Alice log path: {}", mm_alice.log_path.display() });

    unwrap!(block_on(
        mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    // Enable coins on Alice side. Print the replies in case we need the "address".
    log!({ "enable_coins (alice): {:?}", block_on(enable_coins_eth_electrum(&mm_alice, vec!["https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b"])) });

    log!("Get RICK/MORTY orderbook on Alice side");
    let rc = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Alice orderbook "[alice_orderbook]);
    let asks = alice_orderbook["asks"].as_array().unwrap();
    let bids = alice_orderbook["bids"].as_array().unwrap();
    assert_eq!(asks.len(), 2, "Alice RICK/MORTY orderbook must have exactly 2 asks");
    assert_eq!(bids.len(), 1, "Alice RICK/MORTY orderbook must have exactly 1 bid");

    unwrap!(block_on(mm_bob.stop()));
    unwrap!(block_on(mm_alice.stop()));
    unwrap!(block_on(mm_eve.stop()));
}

#[test]
fn log_test_status() { common::log::tests::test_status() }

#[test]
fn log_test_printed_dashboard() { common::log::tests::test_printed_dashboard() }

#[test]
#[cfg(feature = "native")]
fn test_my_balance() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": "bob passphrase",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        local_start!("bob")
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({"log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    // Enable RICK.
    let json = block_on(enable_electrum(&mm, "RICK", vec![
        "electrum1.cipig.net:10017",
        "electrum2.cipig.net:10017",
        "electrum3.cipig.net:10017",
    ]));
    let balance_on_enable = unwrap!(json["balance"].as_str());
    assert_eq!(balance_on_enable, "7.777");

    let my_balance = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "my_balance",
        "coin": "RICK",
    }))));
    assert_eq!(
        my_balance.0,
        StatusCode::OK,
        "RPC «my_balance» failed with status «{}»",
        my_balance.0
    );
    let json: Json = unwrap!(json::from_str(&my_balance.1));
    let my_balance = unwrap!(json["balance"].as_str());
    assert_eq!(my_balance, "7.777");
    let my_address = unwrap!(json["address"].as_str());
    assert_eq!(my_address, "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD");
}

fn check_set_price_fails(mm: &MarketMakerIt, base: &str, rel: &str) {
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "setprice",
        "base": base,
        "rel": rel,
        "price": 0.9,
        "volume": 1,
    }))));
    assert!(
        rc.0.is_server_error(),
        "!setprice success but should be error: {}",
        rc.1
    );
}

fn check_buy_fails(mm: &MarketMakerIt, base: &str, rel: &str, vol: f64) {
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "buy",
        "base": base,
        "rel": rel,
        "volume": vol,
        "price": 0.9
    }))));
    assert!(rc.0.is_server_error(), "!buy success but should be error: {}", rc.1);
}

fn check_sell_fails(mm: &MarketMakerIt, base: &str, rel: &str, vol: f64) {
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "sell",
        "base": base,
        "rel": rel,
        "volume": vol,
        "price": 0.9
    }))));
    assert!(rc.0.is_server_error(), "!sell success but should be error: {}", rc.1);
}

#[test]
#[cfg(feature = "native")]
fn test_check_balance_on_order_post() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"},"rpcport":80},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20", "protocol_data":{"platform":"ETH","contract_address":"0x996a8aE0304680F6A69b8A9d7C6E37D65AB5AB56"}}}
    ]);

    // start bob and immediately place the order
    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase check balance on order post",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        local_start!("bob")
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    // Enable coins. Print the replies in case we need the "address".
    log! ({"enable_coins (bob): {:?}", block_on (enable_coins_eth_electrum (&mm, vec!["http://eth1.cipig.net:8555"]))});
    // issue sell request by setting base/rel price

    // Expect error as MORTY balance is 0
    check_set_price_fails(&mm, "MORTY", "RICK");
    // Address has enough RICK, but doesn't have ETH, so setprice call should fail because maker will not have gas to spend ETH taker payment.
    check_set_price_fails(&mm, "RICK", "ETH");
    // Address has enough RICK, but doesn't have ETH, so setprice call should fail because maker will not have gas to spend ERC20 taker payment.
    check_set_price_fails(&mm, "RICK", "JST");

    // Expect error as MORTY balance is 0
    check_buy_fails(&mm, "RICK", "MORTY", 0.1);
    // RICK balance is sufficient, but amount is too small, it will result to dust error from RPC
    check_buy_fails(&mm, "MORTY", "RICK", 0.000001);
    // Address has enough RICK, but doesn't have ETH, so buy call should fail because taker will not have gas to spend ETH maker payment.
    check_buy_fails(&mm, "ETH", "RICK", 0.1);
    // Address has enough RICK, but doesn't have ETH, so buy call should fail because taker will not have gas to spend ERC20 maker payment.
    check_buy_fails(&mm, "JST", "RICK", 0.1);

    // Expect error as MORTY balance is 0
    check_sell_fails(&mm, "MORTY", "RICK", 0.1);
    // RICK balance is sufficient, but amount is too small, the dex fee will result to dust error from RPC
    check_sell_fails(&mm, "RICK", "MORTY", 0.000001);
    // Address has enough RICK, but doesn't have ETH, so buy call should fail because taker will not have gas to spend ETH maker payment.
    check_sell_fails(&mm, "RICK", "ETH", 0.1);
    // Address has enough RICK, but doesn't have ETH, so buy call should fail because taker will not have gas to spend ERC20 maker payment.
    check_sell_fails(&mm, "RICK", "JST", 0.1);
}

#[test]
#[cfg(feature = "native")]
fn test_rpc_password_from_json() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
    ]);

    // do not allow empty password
    let mut err_mm1 = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "passphrase": "bob passphrase",
            "coins": coins,
            "rpc_password": "",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));
    unwrap!(block_on(
        err_mm1.wait_for_log(5., |log| log.contains("rpc_password must not be empty"))
    ));

    // do not allow empty password
    let mut err_mm2 = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "passphrase": "bob passphrase",
            "coins": coins,
            "rpc_password": {"key":"value"},
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));
    unwrap!(block_on(
        err_mm2.wait_for_log(5., |log| log.contains("rpc_password must be string"))
    ));

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "passphrase": "bob passphrase",
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    let electrum_invalid = unwrap! (block_on (mm.rpc (json! ({
        "userpass": "password1",
        "method": "electrum",
        "coin": "RICK",
        "servers": [{"url":"electrum1.cipig.net:10017"},{"url":"electrum2.cipig.net:10017"},{"url":"electrum3.cipig.net:10017"}],
        "mm2": 1,
    }))));

    // electrum call must fail if invalid password is provided
    assert!(
        electrum_invalid.0.is_server_error(),
        "RPC «electrum» should have failed with server error, but got «{}», response «{}»",
        electrum_invalid.0,
        electrum_invalid.1
    );

    let electrum = unwrap! (block_on (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "RICK",
        "servers": [{"url":"electrum1.cipig.net:10017"},{"url":"electrum2.cipig.net:10017"},{"url":"electrum3.cipig.net:10017"}],
        "mm2": 1,
    }))));

    // electrum call must be successful with RPC password from config
    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with status «{}», response «{}»",
        electrum.0,
        electrum.1
    );

    let electrum = unwrap! (block_on (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "MORTY",
        "servers": [{"url":"electrum1.cipig.net:10018"},{"url":"electrum2.cipig.net:10018"},{"url":"electrum3.cipig.net:10018"}],
        "mm2": 1,
    }))));

    // electrum call must be successful with RPC password from config
    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with status «{}», response «{}»",
        electrum.0,
        electrum.1
    );

    let orderbook = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));

    // orderbook call must be successful with RPC password from config
    assert_eq!(
        orderbook.0,
        StatusCode::OK,
        "RPC «orderbook» failed with status «{}», response «{}»",
        orderbook.0,
        orderbook.1
    );
}

#[test]
#[cfg(feature = "native")]
fn test_rpc_password_from_json_no_userpass() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"protocol":{"type":"UTXO"}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "passphrase": "bob passphrase",
            "coins": coins,
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    let electrum = unwrap!(block_on(mm.rpc(json! ({
        "method": "electrum",
        "coin": "RICK",
        "urls": ["electrum2.cipig.net:10017"],
    }))));

    // electrum call must return 500 status code
    assert!(
        electrum.0.is_server_error(),
        "RPC «electrum» should have failed with server error, but got «{}», response «{}»",
        electrum.0,
        electrum.1
    );
}

/// Trading test using coins with remote RPC (Electrum, ETH nodes), it needs only ENV variables to be set, coins daemons are not required.
/// Trades few pairs concurrently to speed up the process and also act like "load" test
async fn trade_base_rel_electrum(pairs: Vec<(&'static str, &'static str)>) {
    let bob_passphrase = unwrap!(get_passphrase(&".env.seed", "BOB_PASSPHRASE"));
    let alice_passphrase = unwrap!(get_passphrase(&".env.client", "ALICE_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    #[cfg(feature = "native")]
    {
        log! ({"Bob log path: {}", mm_bob.log_path.display()})
    }

    // wait until bob starts listening on the p2p port and sleep for 1 second
    wait_log_re!(mm_bob, 22., "INFO Listening on");
    Timer::sleep(1.).await;

    // Both Alice and Bob might try to bind on the "0.0.0.0:47773" DHT port in this test
    // (because the local "127.0.0.*:47773" addresses aren't that useful for DHT).
    // We want to give Bob a headstart in acquiring the port,
    // because Alice will then be able to directly reach it (thanks to "seednode").
    // Direct communication is not required in this test, but it's nice to have.
    // wait_log_re! (mm_bob, 9., "preferred port");

    let mut mm_alice = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": alice_passphrase,
            "coins": coins,
            "seednodes": [fomat!((mm_bob.ip))],
            "rpc_password": "password",
        }),
        "password".into(),
        local_start!("alice")
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    #[cfg(feature = "native")]
    {
        log! ({"Alice log path: {}", mm_alice.log_path.display()})
    }

    // wait until both nodes RPC API is active
    wait_log_re!(mm_bob, 22., ">>>>>>>>> DEX stats ");
    wait_log_re!(mm_alice, 22., ">>>>>>>>> DEX stats ");

    // Enable coins on Bob side. Print the replies in case we need the address.
    let rc = enable_coins_eth_electrum(&mm_bob, vec!["http://195.201.0.6:8565"]).await;
    log! ({"enable_coins (bob): {:?}", rc});
    // Enable coins on Alice side. Print the replies in case we need the address.
    let rc = enable_coins_eth_electrum(&mm_alice, vec!["http://195.201.0.6:8565"]).await;
    log! ({"enable_coins (alice): {:?}", rc});

    // unwrap! (mm_alice.wait_for_log (999., &|log| log.contains ("set pubkey for ")));

    let mut uuids = vec![];

    // issue sell request on Bob side by setting base/rel price
    for (base, rel) in pairs.iter() {
        log!("Issue bob " (base) "/" (rel) " sell request");
        let rc = unwrap!(
            mm_bob
                .rpc(json! ({
                    "userpass": mm_bob.userpass,
                    "method": "setprice",
                    "base": base,
                    "rel": rel,
                    "price": 1,
                    "volume": 0.1
                }))
                .await
        );
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    }

    for (base, rel) in pairs.iter() {
        common::log::info!(
            "Trigger alice subscription to {}/{} orderbook topic first and sleep for 1 second",
            base,
            rel
        );
        let rc = unwrap!(
            mm_alice
                .rpc(json! ({
                    "userpass": mm_alice.userpass,
                    "method": "orderbook",
                    "base": base,
                    "rel": rel,
                }))
                .await
        );
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);
        Timer::sleep(1.).await;
        common::log::info!("Issue alice {}/{} buy request", base, rel);
        let rc = unwrap!(
            mm_alice
                .rpc(json! ({
                    "userpass": mm_alice.userpass,
                    "method": "buy",
                    "base": base,
                    "rel": rel,
                    "volume": 0.1,
                    "price": 2
                }))
                .await
        );
        assert!(rc.0.is_success(), "!buy: {}", rc.1);
        let buy_json: Json = unwrap!(serde_json::from_str(&rc.1));
        uuids.push(unwrap!(buy_json["result"]["uuid"].as_str()).to_owned());
    }

    for (base, rel) in pairs.iter() {
        // ensure the swaps are started
        let expected_log = format!("Entering the taker_swap_loop {}/{}", base, rel);
        mm_alice
            .wait_for_log(5., |log| log.contains(&expected_log))
            .await
            .unwrap();
        let expected_log = format!("Entering the maker_swap_loop {}/{}", base, rel);
        mm_bob
            .wait_for_log(5., |log| log.contains(&expected_log))
            .await
            .unwrap()
    }

    for uuid in uuids.iter() {
        // ensure the swaps are indexed to the SQLite database
        let expected_log = format!("Inserting new swap {} to the SQLite database", uuid);
        mm_alice
            .wait_for_log(5., |log| log.contains(&expected_log))
            .await
            .unwrap();
        mm_bob
            .wait_for_log(5., |log| log.contains(&expected_log))
            .await
            .unwrap()
    }

    for uuid in uuids.iter() {
        unwrap!(
            mm_bob
                .wait_for_log(600., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))
                .await
        );
        unwrap!(
            mm_alice
                .wait_for_log(600., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))
                .await
        );

        #[cfg(not(feature = "native"))]
        {
            log!("Waiting a few second for the fresh swap status to be saved..");
            Timer::sleep(7.77).await;
        }

        log!("Checking alice/taker status..");
        check_my_swap_status(
            &mm_alice,
            &uuid,
            &TAKER_SUCCESS_EVENTS,
            &TAKER_ERROR_EVENTS,
            "0.1".parse().unwrap(),
            "0.1".parse().unwrap(),
        )
        .await;

        log!("Checking bob/maker status..");
        check_my_swap_status(
            &mm_bob,
            &uuid,
            &MAKER_SUCCESS_EVENTS,
            &MAKER_ERROR_EVENTS,
            "0.1".parse().unwrap(),
            "0.1".parse().unwrap(),
        )
        .await;
    }

    log!("Waiting 3 seconds for nodes to broadcast their swaps data..");
    Timer::sleep(3.).await;

    for uuid in uuids.iter() {
        log!("Checking alice status..");
        check_stats_swap_status(&mm_alice, &uuid, &MAKER_SUCCESS_EVENTS, &TAKER_SUCCESS_EVENTS).await;

        log!("Checking bob status..");
        check_stats_swap_status(&mm_bob, &uuid, &MAKER_SUCCESS_EVENTS, &TAKER_SUCCESS_EVENTS).await;
    }

    log!("Checking alice recent swaps..");
    check_recent_swaps(&mm_alice, uuids.len()).await;
    log!("Checking bob recent swaps..");
    check_recent_swaps(&mm_bob, uuids.len()).await;
    for (base, rel) in pairs.iter() {
        log!("Get " (base) "/" (rel) " orderbook");
        let rc = unwrap!(
            mm_bob
                .rpc(json! ({
                    "userpass": mm_bob.userpass,
                    "method": "orderbook",
                    "base": base,
                    "rel": rel,
                }))
                .await
        );
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!((base) "/" (rel) " orderbook " [bob_orderbook]);

        let bids = bob_orderbook["bids"].as_array().unwrap();
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(0, bids.len(), "{} {} bids must be empty", base, rel);
        assert_eq!(0, asks.len(), "{} {} asks must be empty", base, rel);
    }
    unwrap!(mm_bob.stop().await);
    unwrap!(mm_alice.stop().await);
}

#[cfg(feature = "native")]
#[test]
fn trade_test_electrum_and_eth_coins() { block_on(trade_base_rel_electrum(vec![("ETH", "JST")])); }

#[cfg(not(feature = "native"))]
#[no_mangle]
pub extern "C" fn trade_test_electrum_and_eth_coins(cb_id: i32) {
    use std::ptr::null;

    common::executor::spawn(async move {
        let pairs = vec![("ETH", "JST")];
        trade_base_rel_electrum(pairs).await;
        unsafe { call_back(cb_id, null(), 0) }
    })
}

#[cfg(feature = "native")]
fn withdraw_and_send(
    mm: &MarketMakerIt,
    coin: &str,
    to: &str,
    enable_res: &HashMap<&'static str, Json>,
    expected_bal_change: &str,
) {
    let addr = addr_from_enable(unwrap!(enable_res.get(coin)));

    let withdraw = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": coin,
        "to": to,
        "amount": 0.001
    }))));

    assert!(withdraw.0.is_success(), "!{} withdraw: {}", coin, withdraw.1);
    let withdraw_json: Json = unwrap!(json::from_str(&withdraw.1));
    assert_eq!(Some(&vec![Json::from(to)]), withdraw_json["to"].as_array());
    assert_eq!(Json::from(expected_bal_change), withdraw_json["my_balance_change"]);
    assert_eq!(Some(&vec![addr]), withdraw_json["from"].as_array());

    let send = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "send_raw_transaction",
        "coin": coin,
        "tx_hex": withdraw_json["tx_hex"]
    }))));
    assert!(send.0.is_success(), "!{} send: {}", coin, send.1);
    let send_json: Json = unwrap!(json::from_str(&send.1));
    assert_eq!(withdraw_json["tx_hash"], send_json["tx_hash"]);
}

#[test]
#[cfg(feature = "native")]
fn test_withdraw_and_send() {
    let (alice_file_passphrase, _alice_file_userpass) = from_env_file(unwrap!(slurp(&".env.client")));

    let alice_passphrase = unwrap!(
        var("ALICE_PASSPHRASE").ok().or(alice_file_passphrase),
        "No ALICE_PASSPHRASE or .env.client/PASSPHRASE"
    );

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":8923,"txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY_SEGWIT","asset":"MORTY_SEGWIT","txversion":4,"overwintered":1,"segwit":true,"txfee":1000,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    let mut mm_alice = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8100,
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": alice_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "alice" => Some(local_start()),
            _ => None,
        }
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
    log! ({"Alice log path: {}", mm_alice.log_path.display()});

    // wait until RPC API is active
    unwrap!(block_on(
        mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    // Enable coins. Print the replies in case we need the address.
    let mut enable_res = block_on(enable_coins_eth_electrum(&mm_alice, vec!["http://195.201.0.6:8565"]));
    enable_res.insert(
        "MORTY_SEGWIT",
        block_on(enable_electrum(&mm_alice, "MORTY_SEGWIT", vec![
            "electrum1.cipig.net:10018",
            "electrum2.cipig.net:10018",
            "electrum3.cipig.net:10018",
        ])),
    );

    log!("enable_coins (alice): "[enable_res]);
    withdraw_and_send(
        &mm_alice,
        "MORTY",
        "RJTYiYeJ8eVvJ53n2YbrVmxWNNMVZjDGLh",
        &enable_res,
        "-0.00101",
    );
    // dev chain gas price is 0 so ETH expected balance change doesn't include the fee
    withdraw_and_send(
        &mm_alice,
        "ETH",
        "0x657980d55733B41c0C64c06003864e1aAD917Ca7",
        &enable_res,
        "-0.001",
    );
    withdraw_and_send(
        &mm_alice,
        "JST",
        "0x657980d55733B41c0C64c06003864e1aAD917Ca7",
        &enable_res,
        "-0.001",
    );

    // must not allow to withdraw to non-P2PKH addresses
    let withdraw = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "withdraw",
        "coin": "MORTY",
        "to": "bUN5nesdt1xsAjCtAaYUnNbQhGqUWwQT1Q",
        "amount": "0.001"
    }))));

    assert!(withdraw.0.is_server_error(), "MORTY withdraw: {}", withdraw.1);
    let withdraw_json: Json = unwrap!(json::from_str(&withdraw.1));
    assert!(unwrap!(withdraw_json["error"].as_str())
        .contains("Address bUN5nesdt1xsAjCtAaYUnNbQhGqUWwQT1Q has invalid format"));

    // but must allow to withdraw to P2SH addresses if Segwit flag is true
    let withdraw = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "withdraw",
        "coin": "MORTY_SEGWIT",
        "to": "bUN5nesdt1xsAjCtAaYUnNbQhGqUWwQT1Q",
        "amount": "0.001"
    }))));

    assert!(withdraw.0.is_success(), "MORTY_SEGWIT withdraw: {}", withdraw.1);

    // must not allow to withdraw to invalid checksum address
    let withdraw = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "withdraw",
        "coin": "ETH",
        "to": "0x657980d55733b41c0c64c06003864e1aad917ca7",
        "amount": "0.001"
    }))));

    assert!(withdraw.0.is_server_error(), "ETH withdraw: {}", withdraw.1);
    let withdraw_json: Json = unwrap!(json::from_str(&withdraw.1));
    assert!(unwrap!(withdraw_json["error"].as_str()).contains("Invalid address checksum"));
    unwrap!(block_on(mm_alice.stop()));
}

/// Ensure that swap status return the 404 status code if swap is not found
#[test]
#[cfg(feature = "native")]
fn test_swap_status() {
    let coins = json! ([{"coin":"RICK","asset":"RICK"},]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8100,
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": "some passphrase",
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "alice" => Some(local_start()),
            _ => None,
        }
    ));

    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    let my_swap = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "my_swap_status",
        "params": {
            "uuid":Uuid::new_v4(),
        }
    }))));

    assert_eq!(
        my_swap.0,
        StatusCode::NOT_FOUND,
        "!not found status code: {}",
        my_swap.1
    );

    let stats_swap = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "stats_swap_status",
        "params": {
            "uuid":Uuid::new_v4(),
        }
    }))));

    assert_eq!(
        stats_swap.0,
        StatusCode::NOT_FOUND,
        "!not found status code: {}",
        stats_swap.1
    );
}

/// Ensure that setprice/buy/sell calls deny base == rel
/// https://github.com/artemii235/SuperNET/issues/363
#[test]
#[cfg(feature = "native")]
fn test_order_errors_when_base_equal_rel() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    block_on(enable_electrum(&mm, "RICK", vec![
        "electrum3.cipig.net:10017",
        "electrum2.cipig.net:10017",
        "electrum1.cipig.net:10017",
    ]));

    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "RICK",
        "price": 0.9
    }))));
    assert!(rc.0.is_server_error(), "setprice should have failed, but got {:?}", rc);

    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "buy",
        "base": "RICK",
        "rel": "RICK",
        "price": 0.9,
        "relvolume": 0.1,
    }))));
    assert!(rc.0.is_server_error(), "buy should have failed, but got {:?}", rc);

    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "sell",
        "base": "RICK",
        "rel": "RICK",
        "price": 0.9,
        "basevolume": 0.1,
    }))));
    assert!(rc.0.is_server_error(), "sell should have failed, but got {:?}", rc);
}

fn startup_passphrase(passphrase: &str, expected_address: &str) {
    let coins = json!([
        {"coin":"KMD","rpcport":8923,"txversion":4,"protocol":{"type":"UTXO"}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": passphrase,
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_dump_log, _dump_dashboard) = mm.mm_dump();
    #[cfg(feature = "native")]
    {
        log!({"Log path: {}", mm.log_path.display()})
    }
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    let enable = block_on(enable_electrum(&mm, "KMD", vec!["electrum1.cipig.net:10001"]));
    let addr = addr_from_enable(&enable);
    assert_eq!(Json::from(expected_address), addr);
    unwrap!(block_on(mm.stop()));
}

/// MM2 should detect if passphrase is WIF or 0x-prefixed hex encoded privkey and parse it properly.
/// https://github.com/artemii235/SuperNET/issues/396
#[test]
#[cfg(feature = "native")]
fn test_startup_passphrase() {
    // seed phrase
    startup_passphrase("bob passphrase", "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD");

    // WIF
    assert!(key_pair_from_seed("UvCjJf4dKSs2vFGVtCnUTAhR5FTZGdg43DDRa9s7s5DV1sSDX14g").is_ok());
    startup_passphrase(
        "UvCjJf4dKSs2vFGVtCnUTAhR5FTZGdg43DDRa9s7s5DV1sSDX14g",
        "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD",
    );
    // WIF, Invalid network version
    assert!(key_pair_from_seed("92Qba5hnyWSn5Ffcka56yMQauaWY6ZLd91Vzxbi4a9CCetaHtYj").is_err());
    // WIF, not compressed
    assert!(key_pair_from_seed("5HpHagT65TZzG1PH3CSu63k8DbpvD8s5ip4nEB3kEsreAnchuDf").is_err());

    // 0x prefixed hex
    assert!(key_pair_from_seed("0xb8c774f071de08c7fd8f62b97f1a5726f6ce9f1bcf141b70b86689254ed6714e").is_ok());
    startup_passphrase(
        "0xb8c774f071de08c7fd8f62b97f1a5726f6ce9f1bcf141b70b86689254ed6714e",
        "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD",
    );
    // Out of range, https://en.bitcoin.it/wiki/Private_key#Range_of_valid_ECDSA_private_keys
    assert!(key_pair_from_seed("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141").is_err());
}

/// MM2 should allow to issue several buy/sell calls in a row without delays.
/// https://github.com/artemii235/SuperNET/issues/245
#[test]
#[cfg(feature = "native")]
fn test_multiple_buy_sell_no_delay() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    let (bob_file_passphrase, _bob_file_userpass) = from_env_file(unwrap!(slurp(&".env.seed")));
    let bob_passphrase = unwrap!(
        var("BOB_PASSPHRASE").ok().or(bob_file_passphrase),
        "No BOB_PASSPHRASE or .env.seed/PASSPHRASE"
    );

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    log!([block_on(enable_coins_eth_electrum(&mm, vec![
        "http://195.201.0.6:8565"
    ]))]);

    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "buy",
        "base": "RICK",
        "rel": "MORTY",
        "price": 1,
        "volume": 0.1,
    }))));
    assert!(rc.0.is_success(), "buy should have succeed, but got {:?}", rc);

    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "buy",
        "base": "RICK",
        "rel": "ETH",
        "price": 1,
        "volume": 0.1,
    }))));
    assert!(rc.0.is_success(), "buy should have succeed, but got {:?}", rc);
    thread::sleep(Duration::from_secs(40));

    log!("Get RICK/MORTY orderbook");
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("RICK/MORTY orderbook "[bob_orderbook]);
    let bids = bob_orderbook["bids"].as_array().unwrap();
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert!(bids.len() > 0, "RICK/MORTY bids are empty");
    assert_eq!(0, asks.len(), "RICK/MORTY asks are not empty");
    assert_eq!(Json::from("0.1"), bids[0]["maxvolume"]);

    log!("Get RICK/ETH orderbook");
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "ETH",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("RICK/ETH orderbook "[bob_orderbook]);
    let bids = bob_orderbook["bids"].as_array().unwrap();
    assert!(bids.len() > 0, "RICK/ETH bids are empty");
    assert_eq!(asks.len(), 0, "RICK/ETH asks are not empty");
    assert_eq!(Json::from("0.1"), bids[0]["maxvolume"]);
}

/// https://github.com/artemii235/SuperNET/issues/398
#[test]
#[cfg(feature = "native")]
fn test_cancel_order() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","rpcport":80,"protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0xc0eb7AeD740E1796992A08962c15661bDEB58003"}}}
    ]);
    let bob_passphrase = "bob passphrase";

    // start bob and immediately place the order
    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    // Enable coins on Bob side. Print the replies in case we need the "address".
    log! ({"enable_coins (bob): {:?}", block_on (enable_coins_eth_electrum (&mm_bob, vec!["https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b"]))});

    log!("Issue sell request on Bob side by setting base/rel price…");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "MORTY",
        "price": 0.9,
        "volume": "0.9",
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let setprice_json: Json = unwrap!(json::from_str(&rc.1));
    log!([setprice_json]);

    let mut mm_alice = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": "alice passphrase",
            "coins": coins,
            "seednodes": [fomat!((mm_bob.ip))],
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "alice" => Some(local_start()),
            _ => None,
        }
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
    log!({"Alice log path: {}", mm_alice.log_path.display()});

    unwrap!(block_on(
        mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    // Enable coins on Alice side. Print the replies in case we need the "address".
    log! ({"enable_coins (alice): {:?}", block_on (enable_coins_eth_electrum (&mm_alice, vec!["https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b"]))});

    log!("Get RICK/MORTY orderbook on Alice side");
    let rc = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Alice orderbook "[alice_orderbook]);
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Alice RICK/MORTY orderbook must have exactly 1 ask");

    let cancel_rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "cancel_order",
        "uuid": setprice_json["result"]["uuid"],
    }))));
    assert!(cancel_rc.0.is_success(), "!cancel_order: {}", rc.1);
    let uuid: Uuid = json::from_value(setprice_json["result"]["uuid"].clone()).unwrap();
    let order_path = mm_bob.folder.join(format!(
        "DB/{}/ORDERS/MY/MAKER/{}.json",
        hex::encode(rmd160_from_passphrase(bob_passphrase)),
        uuid
    ));
    assert!(!order_path.exists());

    let pause = 3;
    log!("Waiting (" (pause) " seconds) for Bob to cancel the order…");
    thread::sleep(Duration::from_secs(pause));

    // Bob orderbook must show no orders
    log!("Get RICK/MORTY orderbook on Bob side");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Bob orderbook "[bob_orderbook]);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 0, "Bob RICK/MORTY asks are not empty");

    // Alice orderbook must show no orders
    log!("Get RICK/MORTY orderbook on Alice side");
    let rc = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Alice orderbook "[alice_orderbook]);
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 0, "Alice RICK/MORTY asks are not empty");
}

#[test]
#[cfg(feature = "native")]
fn test_cancel_all_orders() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","rpcport":80,"protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0xc0eb7AeD740E1796992A08962c15661bDEB58003"}}}
    ]);

    let bob_passphrase = "bob passphrase";
    // start bob and immediately place the order
    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    // Enable coins on Bob side. Print the replies in case we need the "address".
    log! ({"enable_coins (bob): {:?}", block_on (enable_coins_eth_electrum (&mm_bob, vec!["https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b"]))});

    log!("Issue sell request on Bob side by setting base/rel price…");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "MORTY",
        "price": 0.9,
        "volume": "0.9",
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let setprice_json: Json = unwrap!(json::from_str(&rc.1));
    log!([setprice_json]);

    let mut mm_alice = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": "alice passphrase",
            "coins": coins,
            "seednodes": [fomat!((mm_bob.ip))],
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "alice" => Some(local_start()),
            _ => None,
        }
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
    log!({"Alice log path: {}", mm_alice.log_path.display()});

    unwrap!(block_on(
        mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    // Enable coins on Alice side. Print the replies in case we need the "address".
    log! ({"enable_coins (alice): {:?}", block_on (enable_coins_eth_electrum (&mm_alice, vec!["https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b"]))});

    log!("Give Alice 15 seconds to import the order…");
    thread::sleep(Duration::from_secs(15));

    log!("Get RICK/MORTY orderbook on Alice side");
    let rc = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Alice orderbook "[alice_orderbook]);
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Alice RICK/MORTY orderbook must have exactly 1 ask");

    let cancel_rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "cancel_all_orders",
        "cancel_by": {
            "type": "All",
        }
    }))));
    assert!(cancel_rc.0.is_success(), "!cancel_all_orders: {}", rc.1);
    let uuid: Uuid = json::from_value(setprice_json["result"]["uuid"].clone()).unwrap();
    let order_path = mm_bob.folder.join(format!(
        "DB/{}/ORDERS/MY/MAKER/{}.json",
        hex::encode(rmd160_from_passphrase(bob_passphrase)),
        uuid
    ));
    assert!(!order_path.exists());

    let pause = 3;
    log!("Waiting (" (pause) " seconds) for Bob to cancel the order…");
    thread::sleep(Duration::from_secs(pause));

    // Bob orderbook must show no orders
    log!("Get RICK/MORTY orderbook on Bob side");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Bob orderbook "[bob_orderbook]);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 0, "Bob RICK/MORTY asks are not empty");

    // Alice orderbook must show no orders
    log!("Get RICK/MORTY orderbook on Alice side");
    let rc = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Alice orderbook "[alice_orderbook]);
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 0, "Alice RICK/MORTY asks are not empty");
}

/// https://github.com/artemii235/SuperNET/issues/367
/// Electrum requests should success if at least 1 server successfully connected,
/// all others might end up with DNS resolution errors, TCP connection errors, etc.
#[test]
#[cfg(feature = "native")]
fn test_electrum_enable_conn_errors() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","protocol":{"type":"UTXO"}},
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    // Using working servers and few else with random ports to trigger "connection refused"
    block_on(enable_electrum(&mm_bob, "RICK", vec![
        "electrum3.cipig.net:10017",
        "electrum2.cipig.net:10017",
        "electrum1.cipig.net:10017",
        "electrum1.cipig.net:60017",
        "electrum1.cipig.net:60018",
    ]));
    // use random domain name to trigger name is not resolved
    block_on(enable_electrum(&mm_bob, "MORTY", vec![
        "electrum3.cipig.net:10018",
        "electrum2.cipig.net:10018",
        "electrum1.cipig.net:10018",
        "random-electrum-domain-name1.net:60017",
        "random-electrum-domain-name2.net:60017",
    ]));
}

#[test]
#[cfg(feature = "native")]
fn test_order_should_not_be_displayed_when_node_is_down() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","protocol":{"type":"UTXO"}},
    ]);

    // start bob and immediately place the order
    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    log!(
        "Bob enable RICK "[block_on(enable_electrum(&mm_bob, "RICK", vec![
            "electrum3.cipig.net:10017",
            "electrum2.cipig.net:10017",
            "electrum1.cipig.net:10017",
        ]))]
    );

    log!(
        "Bob enable MORTY "[block_on(enable_electrum(&mm_bob, "MORTY", vec![
            "electrum3.cipig.net:10018",
            "electrum2.cipig.net:10018",
            "electrum1.cipig.net:10018",
        ]))]
    );

    let mut mm_alice = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": "alice passphrase",
            "coins": coins,
            "seednodes": [fomat!((mm_bob.ip))],
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "alice" => Some(local_start()),
            _ => None,
        }
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
    log!({"Alice log path: {}", mm_alice.log_path.display()});

    unwrap!(block_on(
        mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    log!(
        "Alice enable RICK "[block_on(enable_electrum(&mm_alice, "RICK", vec![
            "electrum3.cipig.net:10017",
            "electrum2.cipig.net:10017",
            "electrum1.cipig.net:10017",
        ]))]
    );

    log!(
        "Alice enable MORTY "[block_on(enable_electrum(&mm_alice, "MORTY", vec![
            "electrum3.cipig.net:10018",
            "electrum2.cipig.net:10018",
            "electrum1.cipig.net:10018",
        ]))]
    );

    // issue sell request on Bob side by setting base/rel price
    log!("Issue bob sell request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "MORTY",
        "price": 0.9,
        "volume": "0.9",
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    thread::sleep(Duration::from_secs(2));

    log!("Get RICK/MORTY orderbook on Alice side");
    let rc = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Alice orderbook "[alice_orderbook]);
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Alice RICK/MORTY orderbook must have exactly 1 ask");

    unwrap!(block_on(mm_bob.stop()));
    thread::sleep(Duration::from_secs(95));

    let rc = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Alice orderbook "[alice_orderbook]);
    let asks = unwrap!(alice_orderbook["asks"].as_array());
    assert_eq!(asks.len(), 0, "Alice RICK/MORTY orderbook must have zero asks");

    unwrap!(block_on(mm_alice.stop()));
}

#[test]
#[cfg(feature = "native")]
fn test_own_orders_should_not_be_removed_from_orderbook() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","protocol":{"type":"UTXO"}},
    ]);

    // start bob and immediately place the order
    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    log!(
        "Bob enable RICK "[block_on(enable_electrum(&mm_bob, "RICK", vec![
            "electrum3.cipig.net:10017",
            "electrum2.cipig.net:10017",
            "electrum1.cipig.net:10017",
        ]))]
    );

    log!(
        "Bob enable MORTY "[block_on(enable_electrum(&mm_bob, "MORTY", vec![
            "electrum3.cipig.net:10018",
            "electrum2.cipig.net:10018",
            "electrum1.cipig.net:10018",
        ]))]
    );

    // issue sell request on Bob side by setting base/rel price
    log!("Issue bob sell request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "MORTY",
        "price": 0.9,
        "volume": "0.9",
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    thread::sleep(Duration::from_secs(95));

    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Bob orderbook "[bob_orderbook]);
    let asks = unwrap!(bob_orderbook["asks"].as_array());
    assert_eq!(asks.len(), 1, "Bob RICK/MORTY orderbook must have exactly 1 ask");

    unwrap!(block_on(mm_bob.stop()));
}

#[test]
#[cfg(feature = "native")]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/511
fn test_all_orders_per_pair_per_node_must_be_displayed_in_orderbook() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","protocol":{"type":"UTXO"}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    block_on(enable_electrum(&mm, "RICK", vec![
        "electrum3.cipig.net:10017",
        "electrum2.cipig.net:10017",
        "electrum1.cipig.net:10017",
    ]));
    block_on(enable_electrum(&mm, "MORTY", vec![
        "electrum3.cipig.net:10018",
        "electrum2.cipig.net:10018",
        "electrum1.cipig.net:10018",
    ]));

    // set 2 orders with different prices
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "MORTY",
        "price": 0.9,
        "volume": "0.9",
        "cancel_previous": false,
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "MORTY",
        "price": 1,
        "volume": "0.9",
        "cancel_previous": false,
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    thread::sleep(Duration::from_secs(12));

    log!("Get RICK/MORTY orderbook");
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("orderbook "[orderbook]);
    let asks = orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 2, "RICK/MORTY orderbook must have exactly 2 asks");
}

#[test]
#[cfg(feature = "native")]
fn orderbook_should_display_rational_amounts() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","protocol":{"type":"UTXO"}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    block_on(enable_electrum(&mm, "RICK", vec![
        "electrum3.cipig.net:10017",
        "electrum2.cipig.net:10017",
        "electrum1.cipig.net:10017",
    ]));
    block_on(enable_electrum(&mm, "MORTY", vec![
        "electrum3.cipig.net:10018",
        "electrum2.cipig.net:10018",
        "electrum1.cipig.net:10018",
    ]));

    let price = BigRational::new(9.into(), 10.into());
    let volume = BigRational::new(9.into(), 10.into());

    // create order with rational amount and price
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "MORTY",
        "price": price,
        "volume": volume,
        "cancel_previous": false,
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    thread::sleep(Duration::from_secs(12));
    log!("Get RICK/MORTY orderbook");
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("orderbook "[orderbook]);
    let asks = orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "RICK/MORTY orderbook must have exactly 1 ask");
    let price_in_orderbook: BigRational = unwrap!(json::from_value(asks[0]["price_rat"].clone()));
    let volume_in_orderbook: BigRational = unwrap!(json::from_value(asks[0]["max_volume_rat"].clone()));
    assert_eq!(price, price_in_orderbook);
    assert_eq!(volume, volume_in_orderbook);

    let nine = BigInt::from(9);
    let ten = BigInt::from(10);
    // should also display fraction
    let price_in_orderbook: Fraction = unwrap!(json::from_value(asks[0]["price_fraction"].clone()));
    let volume_in_orderbook: Fraction = unwrap!(json::from_value(asks[0]["max_volume_fraction"].clone()));
    assert_eq!(nine, *price_in_orderbook.numer());
    assert_eq!(ten, *price_in_orderbook.denom());

    assert_eq!(nine, *volume_in_orderbook.numer());
    assert_eq!(ten, *volume_in_orderbook.denom());

    log!("Get MORTY/RICK orderbook");
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "orderbook",
        "base": "MORTY",
        "rel": "RICK",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("orderbook "[orderbook]);
    let bids = orderbook["bids"].as_array().unwrap();
    assert_eq!(bids.len(), 1, "MORTY/RICK orderbook must have exactly 1 bid");
    let price_in_orderbook: BigRational = unwrap!(json::from_value(bids[0]["price_rat"].clone()));
    let volume_in_orderbook: BigRational = unwrap!(json::from_value(bids[0]["max_volume_rat"].clone()));

    let price = BigRational::new(10.into(), 9.into());
    assert_eq!(price, price_in_orderbook);
    assert_eq!(volume, volume_in_orderbook);

    // should also display fraction
    let price_in_orderbook: Fraction = unwrap!(json::from_value(bids[0]["price_fraction"].clone()));
    let volume_in_orderbook: Fraction = unwrap!(json::from_value(bids[0]["max_volume_fraction"].clone()));
    assert_eq!(ten, *price_in_orderbook.numer());
    assert_eq!(nine, *price_in_orderbook.denom());

    assert_eq!(nine, *volume_in_orderbook.numer());
    assert_eq!(ten, *volume_in_orderbook.denom());
}

fn check_priv_key(mm: &MarketMakerIt, coin: &str, expected_priv_key: &str) {
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "show_priv_key",
        "coin": coin
    }))));
    assert!(rc.0.is_success(), "!show_priv_key: {}", rc.1);
    let privkey: Json = unwrap!(json::from_str(&rc.1));
    assert_eq!(privkey["result"]["priv_key"], Json::from(expected_priv_key))
}

#[test]
#[cfg(feature = "native")]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/519#issuecomment-589149811
fn test_show_priv_key() {
    let coins = json! ([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));

    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    log! ({"enable_coins: {:?}", block_on (enable_coins_eth_electrum (&mm, vec!["http://195.201.0.6:8565"]))});

    check_priv_key(&mm, "RICK", "UvCjJf4dKSs2vFGVtCnUTAhR5FTZGdg43DDRa9s7s5DV1sSDX14g");
    check_priv_key(
        &mm,
        "ETH",
        "0xb8c774f071de08c7fd8f62b97f1a5726f6ce9f1bcf141b70b86689254ed6714e",
    );
}

#[test]
#[cfg(feature = "native")]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/586
fn electrum_and_enable_required_confirmations_and_nota() {
    let coins = json! ([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));

    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    let electrum_rick = unwrap! (block_on(mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "RICK",
        "servers": [{"url":"electrum1.cipig.net:10017"},{"url":"electrum2.cipig.net:10017"},{"url":"electrum3.cipig.net:10017"}],
        "mm2": 1,
        "required_confirmations": 10,
        "requires_notarization": true
    }))));
    assert_eq!(
        electrum_rick.0,
        StatusCode::OK,
        "RPC «electrum» failed with {} {}",
        electrum_rick.0,
        electrum_rick.1
    );
    let rick_response: Json = unwrap!(json::from_str(&electrum_rick.1));
    assert_eq!(rick_response["required_confirmations"], Json::from(10));
    assert_eq!(rick_response["requires_notarization"], Json::from(true));

    // should change requires notarization at runtime
    let requires_nota_rick = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "set_requires_notarization",
        "coin": "RICK",
        "requires_notarization": false
    }))));

    assert_eq!(
        requires_nota_rick.0,
        StatusCode::OK,
        "RPC «set_requires_notarization» failed with {} {}",
        requires_nota_rick.0,
        requires_nota_rick.1
    );
    let requires_nota_rick_response: Json = unwrap!(json::from_str(&requires_nota_rick.1));
    assert_eq!(
        requires_nota_rick_response["result"]["requires_notarization"],
        Json::from(false)
    );

    let enable_eth = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "enable",
        "coin": "ETH",
        "urls": ["http://195.201.0.6:8565"],
        "mm2": 1,
        "swap_contract_address": "0xa09ad3cd7e96586ebd05a2607ee56b56fb2db8fd",
        "required_confirmations": 10,
        "requires_notarization": true
    }))));
    assert_eq!(
        enable_eth.0,
        StatusCode::OK,
        "RPC «enable» failed with {} {}",
        enable_eth.0,
        enable_eth.1
    );
    let eth_response: Json = unwrap!(json::from_str(&enable_eth.1));
    assert_eq!(eth_response["required_confirmations"], Json::from(10));
    // requires_notarization doesn't take any effect on ETH/ERC20 coins
    assert_eq!(eth_response["requires_notarization"], Json::from(false));
}

fn check_too_low_volume_order_creation_fails(mm: &MarketMakerIt, base: &str, rel: &str) {
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "setprice",
        "base": base,
        "rel": rel,
        "price": "1",
        "volume": "0.00776",
        "cancel_previous": false,
    }))));
    assert!(!rc.0.is_success(), "setprice success, but should be error {}", rc.1);

    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "setprice",
        "base": base,
        "rel": rel,
        "price": "0.00776",
        "volume": "1",
        "cancel_previous": false,
    }))));
    assert!(!rc.0.is_success(), "setprice success, but should be error {}", rc.1);

    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "sell",
        "base": base,
        "rel": rel,
        "price": "1",
        "volume": "0.00776",
    }))));
    assert!(!rc.0.is_success(), "sell success, but should be error {}", rc.1);

    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "buy",
        "base": base,
        "rel": rel,
        "price": "1",
        "volume": "0.00776",
    }))));
    assert!(!rc.0.is_success(), "buy success, but should be error {}", rc.1);
}

#[test]
#[cfg(feature = "native")]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/481
fn setprice_buy_sell_too_low_volume() {
    let bob_passphrase = unwrap!(get_passphrase(&".env.seed", "BOB_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));

    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    log!([block_on(enable_coins_eth_electrum(&mm, vec![
        "http://195.201.0.6:8565"
    ]))]);

    check_too_low_volume_order_creation_fails(&mm, "MORTY", "ETH");
    check_too_low_volume_order_creation_fails(&mm, "ETH", "MORTY");
    check_too_low_volume_order_creation_fails(&mm, "JST", "MORTY");
}

#[test]
#[cfg(feature = "native")]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/473
fn setprice_min_volume_should_be_displayed_in_orderbook() {
    let bob_passphrase = unwrap!(get_passphrase(&".env.seed", "BOB_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));

    let (_dump_log, _dump_dashboard) = mm_dump(&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    let mut mm_alice = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": "alice passphrase",
            "coins": coins,
            "seednodes": [fomat!((mm_bob.ip))],
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "alice" => Some(local_start()),
            _ => None,
        }
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
    log!({"Alice log path: {}", mm_alice.log_path.display()});

    unwrap!(block_on(
        mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    log! ({"enable_coins (bob): {:?}", block_on (enable_coins_eth_electrum (&mm_bob, vec!["http://195.201.0.6:8565"]))});
    log! ({"enable_coins (alice): {:?}", block_on (enable_coins_eth_electrum (&mm_alice, vec!["http://195.201.0.6:8565"]))});

    // issue orderbook call on Alice side to trigger subscription to a topic
    unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "ETH",
        "rel": "JST",
    }))));

    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "ETH",
        "rel": "JST",
        "price": "1",
        "volume": "10",
        "min_volume": "1",
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    thread::sleep(Duration::from_secs(2));
    log!("Get ETH/JST orderbook on Bob side");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "ETH",
        "rel": "JST",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("orderbook "[orderbook]);
    let asks = orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Bob ETH/JST orderbook must have exactly 1 ask");

    let min_volume = asks[0]["min_volume"].as_str().unwrap();
    assert_eq!(min_volume, "1", "Bob ETH/JST ask must display correct min_volume");

    log!("Get ETH/JST orderbook on Alice side");
    let rc = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "ETH",
        "rel": "JST",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("orderbook "[orderbook]);
    let asks = orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Alice ETH/JST orderbook must have exactly 1 ask");

    let min_volume = asks[0]["min_volume"].as_str().unwrap();
    assert_eq!(min_volume, "1", "Alice ETH/JST ask must display correct min_volume");
}

#[test]
#[cfg(feature = "native")]
fn test_fill_or_kill_taker_order_should_not_transform_to_maker() {
    let bob_passphrase = unwrap!(get_passphrase(&".env.client", "BOB_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log! ({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    log!([block_on(enable_coins_eth_electrum(&mm_bob, vec![
        "http://195.201.0.6:8565"
    ]))]);

    log!("Issue bob ETH/JST sell request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "sell",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
        "order_type": {
            "type": "FillOrKill"
        }
    }))));
    assert!(rc.0.is_success(), "!sell: {}", rc.1);
    let sell_json: Json = json::from_str(&rc.1).unwrap();
    let order_type = sell_json["result"]["order_type"]["type"].as_str();
    assert_eq!(order_type, Some("FillOrKill"));

    log!("Wait for 40 seconds for Bob order to be cancelled");
    thread::sleep(Duration::from_secs(40));

    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "my_orders",
    }))));
    assert!(rc.0.is_success(), "!my_orders: {}", rc.1);
    let my_orders: Json = unwrap!(json::from_str(&rc.1));
    let my_maker_orders: HashMap<String, Json> = unwrap!(json::from_value(my_orders["result"]["maker_orders"].clone()));
    let my_taker_orders: HashMap<String, Json> = unwrap!(json::from_value(my_orders["result"]["taker_orders"].clone()));
    assert!(my_maker_orders.is_empty(), "maker_orders must be empty");
    assert!(my_taker_orders.is_empty(), "taker_orders must be empty");
}

#[test]
#[cfg(feature = "native")]
fn test_gtc_taker_order_should_transform_to_maker() {
    let bob_passphrase = unwrap!(get_passphrase(&".env.client", "BOB_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log! ({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    log!([block_on(enable_coins_eth_electrum(&mm_bob, vec![
        "http://195.201.0.6:8565"
    ]))]);

    log!("Issue bob ETH/JST sell request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "sell",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
        "order_type": {
            "type": "GoodTillCancelled"
        }
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let rc_json: Json = json::from_str(&rc.1).unwrap();
    let uuid: Uuid = json::from_value(rc_json["result"]["uuid"].clone()).unwrap();

    log!("Wait for 40 seconds for Bob order to be converted to maker");
    thread::sleep(Duration::from_secs(40));

    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "my_orders",
    }))));
    assert!(rc.0.is_success(), "!my_orders: {}", rc.1);
    let my_orders: Json = unwrap!(json::from_str(&rc.1));
    let my_maker_orders: HashMap<String, Json> = unwrap!(json::from_value(my_orders["result"]["maker_orders"].clone()));
    let my_taker_orders: HashMap<String, Json> = unwrap!(json::from_value(my_orders["result"]["taker_orders"].clone()));
    assert_eq!(1, my_maker_orders.len(), "maker_orders must have exactly 1 order");
    assert!(my_taker_orders.is_empty(), "taker_orders must be empty");
    let order_path = mm_bob.folder.join(format!(
        "DB/{}/ORDERS/MY/MAKER/{}.json",
        hex::encode(rmd160_from_passphrase(&bob_passphrase)),
        uuid
    ));
    log!("Order path "(order_path.display()));
    assert!(order_path.exists());
}

#[test]
#[cfg(feature = "native")]
fn test_set_price_response_format() {
    let bob_passphrase = unwrap!(get_passphrase(&".env.client", "BOB_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log! ({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    log!([block_on(enable_coins_eth_electrum(&mm_bob, vec![
        "http://195.201.0.6:8565"
    ]))]);

    log!("Issue bob ETH/JST sell request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let rc_json: Json = json::from_str(&rc.1).unwrap();
    let _: BigDecimal = json::from_value(rc_json["result"]["max_base_vol"].clone()).unwrap();
    let _: BigDecimal = json::from_value(rc_json["result"]["min_base_vol"].clone()).unwrap();
    let _: BigDecimal = json::from_value(rc_json["result"]["price"].clone()).unwrap();

    let _: BigRational = json::from_value(rc_json["result"]["max_base_vol_rat"].clone()).unwrap();
    let _: BigRational = json::from_value(rc_json["result"]["min_base_vol_rat"].clone()).unwrap();
    let _: BigRational = json::from_value(rc_json["result"]["price_rat"].clone()).unwrap();
}

#[test]
#[cfg(feature = "native")]
fn test_set_price_must_save_order_to_db() {
    let bob_passphrase = unwrap!(get_passphrase(&".env.client", "BOB_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log! ({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    log!([block_on(enable_coins_eth_electrum(&mm_bob, vec![
        "http://195.201.0.6:8565"
    ]))]);

    log!("Issue bob ETH/JST sell request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let rc_json: Json = json::from_str(&rc.1).unwrap();
    let uuid: Uuid = json::from_value(rc_json["result"]["uuid"].clone()).unwrap();
    let order_path = mm_bob.folder.join(format!(
        "DB/05aab5342166f8594baf17a7d9bef5d567443327/ORDERS/MY/MAKER/{}.json",
        uuid
    ));
    assert!(order_path.exists());
}

#[test]
#[cfg(feature = "native")]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/635
fn set_price_with_cancel_previous_should_broadcast_cancelled_message() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"},"rpcport":80},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0xc0eb7AeD740E1796992A08962c15661bDEB58003"}}}
    ]);

    // start bob and immediately place the order
    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    // Enable coins on Bob side. Print the replies in case we need the "address".
    log! ({"enable_coins (bob): {:?}", block_on (enable_coins_eth_electrum (&mm_bob, vec!["https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b"]))});

    let set_price_json = json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "MORTY",
        "price": 0.9,
        "volume": "0.9",
    });
    log!("Issue sell request on Bob side by setting base/rel price…");
    let rc = unwrap!(block_on(mm_bob.rpc(set_price_json.clone())));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let mut mm_alice = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": "alice passphrase",
            "coins": coins,
            "seednodes": [fomat!((mm_bob.ip))],
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "alice" => Some(local_start()),
            _ => None,
        }
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
    log!({"Alice log path: {}", mm_alice.log_path.display()});

    unwrap!(block_on(
        mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    // Enable coins on Alice side. Print the replies in case we need the "address".
    log! ({"enable_coins (alice): {:?}", block_on (enable_coins_eth_electrum (&mm_alice, vec!["https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b"]))});

    log!("Get RICK/MORTY orderbook on Alice side");
    let rc = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Alice orderbook "[alice_orderbook]);
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Alice RICK/MORTY orderbook must have exactly 1 ask");

    log!("Issue sell request again on Bob side by setting base/rel price…");
    let rc = unwrap!(block_on(mm_bob.rpc(set_price_json.clone())));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let pause = 11;
    log!("Waiting (" (pause) " seconds) for Bob to broadcast messages…");
    thread::sleep(Duration::from_secs(pause));

    // Bob orderbook must show 1 order
    log!("Get RICK/MORTY orderbook on Bob side");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Bob orderbook "[bob_orderbook]);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Bob RICK/MORTY orderbook must have exactly 1 ask");

    // Alice orderbook must have 1 order
    log!("Get RICK/MORTY orderbook on Alice side");
    let rc = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Alice orderbook "[alice_orderbook]);
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Alice RICK/MORTY orderbook must have exactly 1 ask");
}

#[test]
fn test_batch_requests() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"},"rpcport":80},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0xc0eb7AeD740E1796992A08962c15661bDEB58003"}}}
    ]);

    // start bob and immediately place the order
    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    let batch_json = json!([
        {
            "userpass": mm_bob.userpass,
            "method": "electrum",
            "coin": "RICK",
            "servers": [{"url":"electrum1.cipig.net:10017"},{"url":"electrum2.cipig.net:10017"},{"url":"electrum3.cipig.net:10017"}],
            "mm2": 1,
        },
        {
            "userpass": mm_bob.userpass,
            "method": "electrum",
            "coin": "MORTY",
            "servers": [{"url":"electrum1.cipig.net:10018"},{"url":"electrum2.cipig.net:10018"},{"url":"electrum3.cipig.net:10018"}],
            "mm2": 1,
        },
        {
            "userpass": "error",
            "method": "electrum",
            "coin": "MORTY",
            "servers": [{"url":"electrum1.cipig.net:10018"},{"url":"electrum2.cipig.net:10018"},{"url":"electrum3.cipig.net:10018"}],
            "mm2": 1,
        },
    ]);

    let rc = unwrap!(block_on(mm_bob.rpc(batch_json)));
    assert!(rc.0.is_success(), "!batch: {}", rc.1);
    log!((rc.1));
    let responses = json::from_str::<Vec<Json>>(&rc.1).unwrap();
    assert_eq!(responses[0]["coin"], Json::from("RICK"));
    assert_eq!(
        responses[0]["address"],
        Json::from("RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD")
    );

    assert_eq!(responses[1]["coin"], Json::from("MORTY"));
    assert_eq!(
        responses[1]["address"],
        Json::from("RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD")
    );

    assert!(responses[2]["error"].as_str().unwrap().contains("Userpass is invalid!"));
}

fn request_metrics(mm: &MarketMakerIt) -> MetricsJson {
    let (status, metrics, _headers) = unwrap!(block_on(mm.rpc(json!({ "method": "metrics"}))));
    assert_eq!(status, StatusCode::OK, "RPC «metrics» failed with status «{}»", status);
    unwrap!(json::from_str(&metrics))
}

#[test]
#[cfg(feature = "native")]
fn test_metrics_method() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": "face pin block number add byte put seek mime test note password sin tab multiple",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        local_start!("bob")
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({ "log path: {}", mm.log_path.display() });
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    let _electrum = block_on(enable_electrum(&mm, "RICK", vec![
        "electrum1.cipig.net:10017",
        "electrum2.cipig.net:10017",
        "electrum3.cipig.net:10017",
    ]));

    let metrics = request_metrics(&mm);
    assert!(!metrics.metrics.is_empty());

    log!("Received metrics:");
    log!([metrics]);

    find_metrics_in_json(metrics, "rpc_client.traffic.out", &[("coin", "RICK")])
        .expect(r#"Couldn't find a metric with key = "traffic.out" and label: coin = "RICK" in received json"#);
}

#[test]
#[ignore]
#[cfg(feature = "native")]
fn test_electrum_tx_history() {
    fn get_tx_history_request_count(mm: &MarketMakerIt) -> u64 {
        let metrics = request_metrics(&mm);
        match find_metrics_in_json(metrics, "tx.history.request.count", &[
            ("coin", "RICK"),
            ("method", "blockchain.scripthash.get_history"),
        ])
        .unwrap()
        {
            MetricType::Counter { value, .. } => value,
            _ => panic!("tx.history.request.count should be a counter"),
        }
    }

    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": "face pin block number add byte put seek mime test note password sin tab multiple",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
            "metrics_interval": 30.
        }),
        "pass".into(),
        local_start!("bob")
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({ "log path: {}", mm.log_path.display() });
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    // Enable RICK electrum client with tx_history loop.
    let electrum = unwrap!(block_on(mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "RICK",
        "servers": [{"url":"electrum1.cipig.net:10017"},{"url":"electrum2.cipig.net:10017"},{"url":"electrum3.cipig.net:10017"}],
        "mm2": 1,
        "tx_history": true
    }))));

    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with {} {}",
        electrum.0,
        electrum.1
    );
    let electrum: Json = unwrap!(json::from_str(&electrum.1));

    // Wait till tx_history will not be loaded
    unwrap!(block_on(mm.wait_for_log(500., |log| {
        log.contains("history has been loaded successfully")
    })));

    // tx_history is requested every 30 seconds, wait another iteration
    thread::sleep(Duration::from_secs(31));

    // Balance is not changed, therefore tx_history shouldn't be reloaded.
    // Request metrics and check if the MarketMaker has requested tx_history only once
    assert_eq!(get_tx_history_request_count(&mm), 1);

    // make a transaction to change balance
    let mut enable_res: HashMap<&str, Json> = HashMap::new();
    enable_res.insert("RICK", electrum);
    log!("enable_coins: "[enable_res]);
    withdraw_and_send(
        &mm,
        "RICK",
        "RRYmiZSDo3UdHHqj1rLKf8cbJroyv9NxXw",
        &enable_res,
        "-0.00001",
    );

    // Wait another iteration
    thread::sleep(Duration::from_secs(31));

    // tx_history should be reloaded on next loop iteration
    assert_eq!(get_tx_history_request_count(&mm), 2);
}

#[allow(dead_code)]
fn spin_n_nodes(seednodes: &[&str], coins: &Json, n: usize) -> Vec<(MarketMakerIt, RaiiDump, RaiiDump)> {
    let mut mm_nodes = Vec::with_capacity(n);
    for i in 0..n {
        let mut mm = unwrap!(MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9998,
                "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
                "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
                "passphrase": format!("alice passphrase {}", i),
                "coins": coins,
                "seednodes": seednodes,
                "rpc_password": "pass",
            }),
            "pass".into(),
            local_start!("alice")
        ));

        let (alice_dump_log, alice_dump_dashboard) = mm_dump(&mm.log_path);
        log!({ "Alice {} log path: {}", i, mm.log_path.display() });
        for seednode in seednodes.iter() {
            unwrap!(block_on(
                mm.wait_for_log(22., |log| log.contains(&format!("Dialed {}", seednode)))
            ));
        }
        mm_nodes.push((mm, alice_dump_log, alice_dump_dashboard));
    }
    mm_nodes
}

#[test]
fn test_withdraw_cashaddresses() {
    let coins = json!([
        {"coin":"BCH","pubtype":0,"p2shtype":5,"mm2":1,"protocol":{"type":"UTXO"},
         "address_format":{"format":"cashaddress","network":"bchtest"}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": "face pin lock number add byte put seek mime test note password sin tab multiple",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        local_start!("bob")
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({ "log path: {}", mm.log_path.display() });
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    let electrum = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "BCH",
        "servers": [{"url":"blackie.c3-soft.com:60001"},{"url":"testnet.imaginary.cash:50001"}],
        "mm2": 1,
    }))));

    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with {} {}",
        electrum.0,
        electrum.1
    );
    let electrum: Json = unwrap!(json::from_str(&electrum.1));
    log!([electrum]);

    // make withdraw
    let withdraw = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": "BCH",
        "to": "bchtest:qr39na5d25wdeecgw3euh9fkd4ygvd4pnsury96597",
        "amount": 0.00001,
    }))));

    assert!(withdraw.0.is_success(), "BCH withdraw: {}", withdraw.1);
    let withdraw_json: Json = unwrap!(json::from_str(&withdraw.1));
    log!((withdraw_json));

    // check "from" addresses
    let from: Vec<&str> = withdraw_json["from"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(from, vec!["bchtest:qqgp9xh3435xamv7ghct8emer2s2erzj8gx3gnhwkq"]);

    // check "to" addresses
    let to: Vec<&str> = withdraw_json["to"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(to, vec!["bchtest:qr39na5d25wdeecgw3euh9fkd4ygvd4pnsury96597"]);

    // send the transaction
    let send_tx = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "send_raw_transaction",
        "coin": "BCH",
        "tx_hex": withdraw_json["tx_hex"],
    }))));
    assert!(send_tx.0.is_success(), "BCH send_raw_transaction: {}", send_tx.1);
    log!((send_tx.1));
}

#[test]
fn test_common_cashaddresses() {
    let coins = json!([
        {"coin":"BCH","pubtype":0,"p2shtype":5,"mm2":1,"protocol":{"type":"UTXO"},
         "address_format":{"format":"cashaddress","network":"bchtest"}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": "face pin block number add byte put seek mime test note password sin tab multiple",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        local_start!("bob")
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({ "log path: {}", mm.log_path.display() });
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    // Enable BCH electrum client with tx_history loop.
    // Enable RICK electrum client with tx_history loop.
    let electrum = unwrap!(block_on(mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "BCH",
        "servers": [{"url":"blackie.c3-soft.com:60001"}, {"url":"bch0.kister.net:51001"}, {"url":"testnet.imaginary.cash:50001"}],
        "mm2": 1,
    }))));

    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with {} {}",
        electrum.0,
        electrum.1
    );
    let electrum: Json = unwrap!(json::from_str(&electrum.1));
    log!([electrum]);

    assert_eq!(
        unwrap!(electrum["address"].as_str()),
        "bchtest:qze8g4gx3z428jjcxzpycpxl7ke7d947gca2a7n2la"
    );

    // check my_balance
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "my_balance",
        "coin": "BCH",
    }))));
    assert_eq!(rc.0, StatusCode::OK, "RPC «my_balance» failed with status «{}»", rc.0);
    let json: Json = unwrap!(json::from_str(&rc.1));
    let my_balance_address = unwrap!(json["address"].as_str());
    assert_eq!(my_balance_address, "bchtest:qze8g4gx3z428jjcxzpycpxl7ke7d947gca2a7n2la");

    // check get_enabled_coins
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "get_enabled_coins",
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «get_enabled_coins» failed with status «{}»",
        rc.0
    );
    let json: Json = unwrap!(json::from_str(&rc.1));

    let obj = &json["result"].as_array().unwrap()[0];
    assert_eq!(obj["ticker"].as_str().unwrap(), "BCH");
    assert_eq!(
        obj["address"].as_str().unwrap(),
        "bchtest:qze8g4gx3z428jjcxzpycpxl7ke7d947gca2a7n2la"
    );
}

#[test]
fn test_convert_utxo_address() {
    let coins = json!([
        {"coin":"BCH","pubtype":0,"p2shtype":5,"mm2":1,"protocol":{"type":"UTXO"}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": "face pin block number add byte put seek mime test note password sin tab multiple",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        local_start!("bob")
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({ "log path: {}", mm.log_path.display() });
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    let _electrum = block_on(enable_electrum(&mm, "BCH", vec![
        "electrum1.cipig.net:10017",
        "electrum2.cipig.net:10017",
        "electrum3.cipig.net:10017",
    ]));

    // test standard to cashaddress
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "convertaddress",
        "coin": "BCH",
        "from": "1DmFp16U73RrVZtYUbo2Ectt8mAnYScpqM",
        "to_address_format":{"format":"cashaddress","network":"bitcoincash"},
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «convertaddress» failed with status «{}»",
        rc.0
    );
    let actual: Json = unwrap!(json::from_str(&rc.1));

    let expected = json!({
        "result": {
            "address": "bitcoincash:qzxqqt9lh4feptf0mplnk58gnajfepzwcq9f2rxk55",
        },
    });
    assert_eq!(actual, expected);

    // test cashaddress to standard
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "convertaddress",
        "coin": "BCH",
        "from": "bitcoincash:qzxqqt9lh4feptf0mplnk58gnajfepzwcq9f2rxk55",
        "to_address_format":{"format":"standard"},
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «convertaddress» failed with status «{}»",
        rc.0
    );
    let actual: Json = unwrap!(json::from_str(&rc.1));

    let expected = json!({
        "result": {
            "address": "1DmFp16U73RrVZtYUbo2Ectt8mAnYScpqM",
        },
    });
    assert_eq!(actual, expected);

    // test standard to standard
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "convertaddress",
        "coin": "BCH",
        "from": "1DmFp16U73RrVZtYUbo2Ectt8mAnYScpqM",
        "to_address_format":{"format":"standard"},
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «convertaddress» failed with status «{}»",
        rc.0
    );
    let actual: Json = unwrap!(json::from_str(&rc.1));

    let expected = json!({
        "result": {
            "address": "1DmFp16U73RrVZtYUbo2Ectt8mAnYScpqM",
        },
    });
    assert_eq!(actual, expected);

    // test invalid address
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "convertaddress",
        "coin": "BCH",
        "from": "0000000000000000000000000000000000",
        "to_address_format":{"format":"standard"},
    }))));
    assert!(
        rc.0.is_server_error(),
        "!convertaddress success but should be error: {}",
        rc.1
    );
}

#[test]
fn test_convert_eth_address() {
    let coins = json!([
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
    ]);

    // start mm and immediately place the order
    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({ "log path: {}", mm.log_path.display() });
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    block_on(enable_native(&mm, "ETH", vec!["http://195.201.0.6:8565"]));

    // test single-case to mixed-case
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "convertaddress",
        "coin": "ETH",
        "from": "0xfb6916095ca1df60bb79ce92ce3ea74c37c5d359",
        "to_address_format":{"format":"mixedcase"},
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «convertaddress» failed with status «{}»",
        rc.0
    );
    let actual: Json = unwrap!(json::from_str(&rc.1));

    let expected = json!({
        "result": {
            "address": "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
        },
    });
    assert_eq!(actual, expected);

    // test mixed-case to mixed-case (expect error)
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "convertaddress",
        "coin": "ETH",
        "from": "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
        "to_address_format":{"format":"mixedcase"},
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «convertaddress» failed with status «{}»",
        rc.0
    );
    let actual: Json = unwrap!(json::from_str(&rc.1));

    let expected = json!({
        "result": {
            "address": "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
        },
    });
    assert_eq!(actual, expected);

    // test invalid address
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "convertaddress",
        "coin": "ETH",
        "from": "fB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
        "to_address_format":{"format":"mixedcase"},
    }))));
    assert!(
        rc.0.is_server_error(),
        "!convertaddress success but should be error: {}",
        rc.1
    );
    assert!(rc.1.contains("Address must be prefixed with 0x"));
}

#[test]
fn test_convert_qrc20_address() {
    let passphrase = "cV463HpebE2djP9ugJry5wZ9st5cc6AbkHXGryZVPXMH1XJK8cVU";
    let coins = json! ([
        {"coin":"QRC20","required_confirmations":0,"pubtype": 120,"p2shtype": 50,"wiftype": 128,"segwit": true,"txfee": 0,"mm2": 1,"mature_confirmations":500,
         "protocol":{"type":"QRC20","protocol_data":{"platform":"QTUM","contract_address":"0xd362e096e873eb7907e205fadc6175c6fec7bc44"}}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm.mm_dump();
    log! ({"Bob log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    let _electrum = block_on(enable_qrc20(
        &mm,
        "QRC20",
        &["95.217.83.126:10001"],
        "0xba8b71f3544b93e2f681f996da519a98ace0107a",
    ));

    // test wallet to contract
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "convertaddress",
        "coin": "QRC20",
        "from": "qKVvtDqpnFGDxsDzck5jmLwdnD2jRH6aM8",
        "to_address_format":{"format":"contract"},
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «convertaddress» failed with status «{}»",
        rc.0
    );
    let actual: Json = unwrap!(json::from_str(&rc.1));

    let expected = json!({
        "result": {
            "address": "0x1549128bbfb33b997949b4105b6a6371c998e212",
        },
    });
    assert_eq!(actual, expected);

    // test contract to wallet
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "convertaddress",
        "coin": "QRC20",
        "from": "0x1549128bbfb33b997949b4105b6a6371c998e212",
        "to_address_format":{"format":"wallet"},
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «convertaddress» failed with status «{}»",
        rc.0
    );
    let actual: Json = unwrap!(json::from_str(&rc.1));

    let expected = json!({
        "result": {
            "address": "qKVvtDqpnFGDxsDzck5jmLwdnD2jRH6aM8",
        },
    });
    assert_eq!(actual, expected);

    // test wallet to wallet
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "convertaddress",
        "coin": "QRC20",
        "from": "qKVvtDqpnFGDxsDzck5jmLwdnD2jRH6aM8",
        "to_address_format":{"format":"wallet"},
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «convertaddress» failed with status «{}»",
        rc.0
    );
    let actual: Json = unwrap!(json::from_str(&rc.1));

    let expected = json!({
        "result": {
            "address": "qKVvtDqpnFGDxsDzck5jmLwdnD2jRH6aM8",
        },
    });
    assert_eq!(actual, expected);

    // test invalid address (invalid prefixes)
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "convertaddress",
        "coin": "QRC20",
        "from": "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD",
        "to_address_format":{"format":"contract"},
    }))));
    assert!(
        rc.0.is_server_error(),
        "!convertaddress success but should be error: {}",
        rc.1
    );
    log!((rc.1));
    assert!(rc.1.contains("Address has invalid prefixes"));

    // test invalid address
    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "convertaddress",
        "coin": "QRC20",
        "from": "0000000000000000000000000000000000",
        "to_address_format":{"format":"wallet"},
    }))));
    assert!(
        rc.0.is_server_error(),
        "!convertaddress success but should be error: {}",
        rc.1
    );
}

#[test]
fn test_validateaddress() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    let (bob_file_passphrase, _bob_file_userpass) = from_env_file(unwrap!(slurp(&".env.seed")));
    let bob_passphrase = unwrap!(
        var("BOB_PASSPHRASE").ok().or(bob_file_passphrase),
        "No BOB_PASSPHRASE or .env.seed/PASSPHRASE"
    );

    let mut mm = unwrap!(MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var("BOB_TRADE_IP").ok(),
            "rpcip": env::var("BOB_TRADE_IP").ok(),
            "canbind": env::var("BOB_TRADE_PORT").ok().map(|s| unwrap!(s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    log!([block_on(enable_coins_eth_electrum(&mm, vec![
        "http://195.201.0.6:8565"
    ]))]);

    // test valid RICK address

    let rc = unwrap!(block_on(mm.rpc(json!({
        "userpass": mm.userpass,
        "method": "validateaddress",
        "coin": "RICK",
        "address": "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD",
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «validateaddress» failed with status «{}»",
        rc.0
    );
    let actual: Json = unwrap!(json::from_str(&rc.1));

    let expected = json!({
        "result": {
            "is_valid": true,
        },
    });
    assert_eq!(actual, expected);

    // test valid ETH address

    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "validateaddress",
        "coin": "ETH",
        "address": "0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94",
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «validateaddress» failed with status «{}»",
        rc.0
    );
    let actual: Json = unwrap!(json::from_str(&rc.1));

    let expected = json!({
        "result": {
            "is_valid": true,
        },
    });
    assert_eq!(actual, expected);

    // test invalid RICK address (legacy address format activated)

    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "validateaddress",
        "coin": "RICK",
        "address": "bchtest:qr39na5d25wdeecgw3euh9fkd4ygvd4pnsury96597",
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «validateaddress» failed with status «{}»",
        rc.0
    );
    let json: Json = unwrap!(json::from_str(&rc.1));
    let result = &json["result"];

    assert!(!result["is_valid"].as_bool().unwrap());
    let reason = result["reason"].as_str().unwrap();
    log!((reason));
    assert!(reason.contains("Legacy address format activated for RICK, but cashaddress format used instead"));

    // test invalid RICK address (invalid prefixes)

    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "validateaddress",
        "coin": "RICK",
        "address": "1DmFp16U73RrVZtYUbo2Ectt8mAnYScpqM",
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «validateaddress» failed with status «{}»",
        rc.0
    );

    let json: Json = unwrap!(json::from_str(&rc.1));
    let result = &json["result"];

    assert!(!result["is_valid"].as_bool().unwrap());
    let reason = result["reason"].as_str().unwrap();
    log!((reason));
    assert!(reason.contains("Address 1DmFp16U73RrVZtYUbo2Ectt8mAnYScpqM has invalid"));

    // test invalid ETH address

    let rc = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "validateaddress",
        "coin": "ETH",
        "address": "7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94",
    }))));
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «validateaddress» failed with status «{}»",
        rc.0
    );
    let json: Json = unwrap!(json::from_str(&rc.1));
    let result = &json["result"];

    assert!(!result["is_valid"].as_bool().unwrap());
    let reason = result["reason"].as_str().unwrap();
    log!((reason));
    assert!(reason.contains("Address must be prefixed with 0x"));
}

#[test]
fn qrc20_activate_electrum() {
    let passphrase = "cV463HpebE2djP9ugJry5wZ9st5cc6AbkHXGryZVPXMH1XJK8cVU";
    let coins = json! ([
        {"coin":"QRC20","required_confirmations":0,"pubtype": 120,"p2shtype": 50,"wiftype": 128,"segwit": true,"txfee": 0,"mm2": 1,"mature_confirmations":500,
         "protocol":{"type":"QRC20","protocol_data":{"platform":"QTUM","contract_address":"0xd362e096e873eb7907e205fadc6175c6fec7bc44"}}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm.mm_dump();
    log! ({"Bob log path: {}", mm.log_path.display()});
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    let electrum_json = block_on(enable_qrc20(
        &mm,
        "QRC20",
        &["95.217.83.126:10001"],
        "0xba8b71f3544b93e2f681f996da519a98ace0107a",
    ));
    assert_eq!(
        electrum_json["address"].as_str(),
        Some("qKEDGuogDhtH9zBnc71QtqT1KDamaR1KJ3")
    );
    assert_eq!(electrum_json["balance"].as_str(), Some("139"));
}

#[test]
fn test_qrc20_withdraw() {
    // corresponding private key: [3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72, 172, 110, 180, 13, 123, 179, 10, 49]
    let passphrase = "cMhHM3PMpMrChygR4bLF7QsTdenhWpFrrmf2UezBG3eeFsz41rtL";
    let coins = json!([
        {"coin":"QRC20","required_confirmations":0,"pubtype": 120,"p2shtype": 50,"wiftype": 128,"segwit": true,"txfee": 0,"mm2": 1,"mature_confirmations":500,
         "protocol":{"type":"QRC20","protocol_data":{"platform":"QTUM","contract_address":"0xd362e096e873eb7907e205fadc6175c6fec7bc44"}}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm.mm_dump();
    log!({ "Bob log path: {}", mm.log_path.display() });
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    let electrum_json = block_on(enable_qrc20(
        &mm,
        "QRC20",
        &["95.217.83.126:10001"],
        "0xba8b71f3544b93e2f681f996da519a98ace0107a",
    ));
    assert_eq!(
        electrum_json["address"].as_str(),
        Some("qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG")
    );
    log!("electrum_json: "[electrum_json]);
    let balance: f64 = electrum_json["balance"].as_str().unwrap().parse().unwrap();
    log!("Balance "(balance));

    let amount = 10;

    let withdraw = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": "QRC20",
        "to": "qHmJ3KA6ZAjR9wGjpFASn4gtUSeFAqdZgs",
        "amount": amount,
        "fee": {
            "type": "Qrc20Gas",
            "gas_limit": 2_500_000,
            "gas_price": 40,
        }
    }))));

    let withdraw_json: Json = unwrap!(json::from_str(&withdraw.1));
    assert!(withdraw.0.is_success(), "QRC20 withdraw: {}", withdraw.1);

    log!((withdraw_json));
    assert!(withdraw_json["tx_hex"].as_str().unwrap().contains("5403a02526012844a9059cbb0000000000000000000000000240b898276ad2cc0d2fe6f527e8e31104e7fde3000000000000000000000000000000000000000000000000000000003b9aca0014d362e096e873eb7907e205fadc6175c6fec7bc44c2"));

    let send_tx = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "send_raw_transaction",
        "coin": "QRC20",
        "tx_hex": withdraw_json["tx_hex"],
    }))));
    assert!(send_tx.0.is_success(), "QRC20 send_raw_transaction: {}", send_tx.1);
    log!((send_tx.1));
}

#[test]
fn test_qrc20_withdraw_error() {
    let passphrase = "album hollow help heart use bird response large lounge fat elbow coral";
    let coins = json!([
        {"coin":"QRC20","required_confirmations":0,"pubtype": 120,"p2shtype": 50,"wiftype": 128,"segwit": true,"txfee": 0,"mm2": 1,"mature_confirmations":500,
         "protocol":{"type":"QRC20","protocol_data":{"platform":"QTUM","contract_address":"0xd362e096e873eb7907e205fadc6175c6fec7bc44"}}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({ "log path: {}", mm.log_path.display() });
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    let electrum_json = block_on(enable_qrc20(
        &mm,
        "QRC20",
        &["95.217.83.126:10001"],
        "0xba8b71f3544b93e2f681f996da519a98ace0107a",
    ));
    let balance = electrum_json["balance"].as_str().unwrap();
    assert_eq!(balance, "10");

    // try to transfer too low amount
    let withdraw = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": "QRC20",
        "to": "qHmJ3KA6ZAjR9wGjpFASn4gtUSeFAqdZgs",
        "amount": 0,
    }))));
    assert!(
        withdraw.0.is_server_error(),
        "withdraw should have failed, but got {:?}",
        withdraw
    );
    log!([withdraw.1]);
    assert!(withdraw.1.contains("The amount 0 is too small"));

    // try to transfer amount with more than 8 decimals
    let withdraw = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": "QRC20",
        "to": "qHmJ3KA6ZAjR9wGjpFASn4gtUSeFAqdZgs",
        "amount": "0.0000000001",
    }))));
    assert!(
        withdraw.0.is_server_error(),
        "withdraw should have failed, but got {:?}",
        withdraw
    );
    log!([withdraw.1]);
    assert!(withdraw.1.contains("The amount 0.0000000001 is too small"));

    // try to transfer more than balance
    let withdraw = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": "QRC20",
        "to": "qHmJ3KA6ZAjR9wGjpFASn4gtUSeFAqdZgs",
        "amount": "11",
    }))));
    assert!(
        withdraw.0.is_server_error(),
        "withdraw should have failed, but got {:?}",
        withdraw
    );
    log!([withdraw.1]);
    assert!(withdraw
        .1
        .contains("The amount 11 to withdraw is larger than balance 10"));

    // try to transfer with zero QTUM balance
    let withdraw = unwrap!(block_on(mm.rpc(json! ({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": "QRC20",
        "to": "qHmJ3KA6ZAjR9wGjpFASn4gtUSeFAqdZgs",
        "amount": "2",
    }))));
    assert!(
        withdraw.0.is_server_error(),
        "withdraw should have failed, but got {:?}",
        withdraw
    );
    log!([withdraw.1]);
    assert!(withdraw
        .1
        .contains("Not enough QTUM to Pay Fee: Couldn't generate tx from empty UTXOs set"));
}

#[test]
#[cfg(feature = "native")]
fn test_qrc20_tx_history() {
    let passphrase = "daring blind measure rebuild grab boost fix favorite nurse stereo april rookie";
    let coins = json!([
        {"coin":"QRC20","required_confirmations":0,"pubtype": 120,"p2shtype": 50,"wiftype": 128,"segwit": true,"txfee": 0,"mm2": 1,"mature_confirmations":500,
         "protocol":{"type":"QRC20","protocol_data":{"platform":"QTUM","contract_address":"0xd362e096e873eb7907e205fadc6175c6fec7bc44"}}},
    ]);

    let mut mm = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": passphrase,
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
            "metrics_interval": 30.,
        }),
        "pass".into(),
        local_start!("bob")
    ));
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);
    log!({ "log path: {}", mm.log_path.display() });
    unwrap!(block_on(
        mm.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    let electrum = unwrap!(block_on(mm.rpc(json!({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "QRC20",
        "servers": [{"url":"95.217.83.126:10001"}],
        "mm2": 1,
        "tx_history": true,
        "swap_contract_address": "0xd362e096e873eb7907e205fadc6175c6fec7bc44",
    }))));
    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with status «{}», response «{}»",
        electrum.0,
        electrum.1
    );
    let electrum_json: Json = json::from_str(&electrum.1).unwrap();
    assert_eq!(
        electrum_json["address"].as_str(),
        Some("qfkXE2cNFEwPFQqvBcqs8m9KrkNa9KV4xi")
    );

    // Wait till tx_history will not be loaded
    unwrap!(block_on(mm.wait_for_log(22., |log| {
        log.contains("history has been loaded successfully")
    })));

    let tx_history = unwrap!(block_on(mm.rpc(json!({
        "userpass": mm.userpass,
        "method": "my_tx_history",
        "coin": "QRC20",
        "limit": 100,
    }))));
    assert_eq!(
        tx_history.0,
        StatusCode::OK,
        "RPC «my_tx_history» failed with status «{}», response «{}»",
        tx_history.0,
        tx_history.1
    );
    log!([tx_history.1]);
    let tx_history_json: Json = json::from_str(&tx_history.1).unwrap();
    let tx_history_result = &tx_history_json["result"];

    let mut expected = vec![
        // https://testnet.qtum.info/tx/45d722e615feb853d608033ffc20fd51c9ee86e2321cfa814ba5961190fb57d2
        "45d722e615feb853d608033ffc20fd51c9ee86e2321cfa814ba5961190fb57d200000000000000020000000000000000",
        // https://testnet.qtum.info/tx/45d722e615feb853d608033ffc20fd51c9ee86e2321cfa814ba5961190fb57d2
        "45d722e615feb853d608033ffc20fd51c9ee86e2321cfa814ba5961190fb57d200000000000000020000000000000001",
        // https://testnet.qtum.info/tx/abcb51963e720fdfed7b889cea79947ba3cabd7b8b384f6b5adb41a3f4b5d61b
        "abcb51963e720fdfed7b889cea79947ba3cabd7b8b384f6b5adb41a3f4b5d61b00000000000000020000000000000000",
        // https://testnet.qtum.info/tx/4ea5392d03a9c35126d2d5a8294c3c3102cfc6d65235897c92ca04c5515f6be5
        "4ea5392d03a9c35126d2d5a8294c3c3102cfc6d65235897c92ca04c5515f6be500000000000000020000000000000000",
        // https://testnet.qtum.info/tx/9156f5f1d3652c27dca0216c63177da38de5c9e9f03a5cfa278bf82882d2d3d8
        "9156f5f1d3652c27dca0216c63177da38de5c9e9f03a5cfa278bf82882d2d3d800000000000000020000000000000000",
        // https://testnet.qtum.info/tx/35e03bc529528a853ee75dde28f27eec8ed7b152b6af7ab6dfa5d55ea46f25ac
        "35e03bc529528a853ee75dde28f27eec8ed7b152b6af7ab6dfa5d55ea46f25ac00000000000000010000000000000000",
        // https://testnet.qtum.info/tx/39104d29d77ba83c5c6c63ab7a0f096301c443b4538dc6b30140453a40caa80a
        "39104d29d77ba83c5c6c63ab7a0f096301c443b4538dc6b30140453a40caa80a00000000000000000000000000000000",
        // https://testnet.qtum.info/tx/d9965e3496a8a4af2d462424b989694b3146d78c61654b99bbadba64464f75cb
        "d9965e3496a8a4af2d462424b989694b3146d78c61654b99bbadba64464f75cb00000000000000000000000000000000",
        // https://testnet.qtum.info/tx/c2f346d3d2aadc35f5343d0d493a139b2579175496d685ec30734d161e62f7a1
        "c2f346d3d2aadc35f5343d0d493a139b2579175496d685ec30734d161e62f7a100000000000000000000000000000000",
    ];

    assert_eq!(tx_history_result["total"].as_u64().unwrap(), expected.len() as u64);
    for tx in tx_history_result["transactions"].as_array().unwrap() {
        // pop front item
        let expected_tx = expected.remove(0);
        assert_eq!(tx["internal_id"].as_str().unwrap(), expected_tx);
    }
}

#[test]
fn test_buy_conf_settings() {
    let bob_passphrase = unwrap!(get_passphrase(&".env.client", "BOB_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}},"required_confirmations":2}
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log! ({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    log!([block_on(enable_coins_eth_electrum(&mm_bob, vec![
        "http://195.201.0.6:8565"
    ]))]);

    log!("Issue bob buy request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "buy",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
        "base_confs": 5,
        "base_nota": true,
        "rel_confs": 4,
        "rel_nota": false,
    }))));
    assert!(rc.0.is_success(), "!buy: {}", rc.1);
    let json: Json = json::from_str(&rc.1).unwrap();
    assert_eq!(json["result"]["conf_settings"]["base_confs"], Json::from(5));
    assert_eq!(json["result"]["conf_settings"]["base_nota"], Json::from(true));
    assert_eq!(json["result"]["conf_settings"]["rel_confs"], Json::from(4));
    assert_eq!(json["result"]["conf_settings"]["rel_nota"], Json::from(false));

    // must use coin config as defaults if not set in request
    log!("Issue bob buy request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "buy",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
    }))));
    assert!(rc.0.is_success(), "!buy: {}", rc.1);
    let json: Json = json::from_str(&rc.1).unwrap();
    assert_eq!(json["result"]["conf_settings"]["base_confs"], Json::from(1));
    assert_eq!(json["result"]["conf_settings"]["base_nota"], Json::from(false));
    assert_eq!(json["result"]["conf_settings"]["rel_confs"], Json::from(2));
    assert_eq!(json["result"]["conf_settings"]["rel_nota"], Json::from(false));
}

#[test]
fn test_buy_response_format() {
    let bob_passphrase = unwrap!(get_passphrase(&".env.client", "BOB_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}},"required_confirmations":2}
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log! ({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    log!([block_on(enable_coins_eth_electrum(&mm_bob, vec![
        "http://195.201.0.6:8565"
    ]))]);

    log!("Issue bob buy request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "buy",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
        "base_confs": 5,
        "base_nota": true,
        "rel_confs": 4,
        "rel_nota": false,
    }))));
    assert!(rc.0.is_success(), "!buy: {}", rc.1);
    let json: Json = json::from_str(&rc.1).unwrap();
    let _: BuyOrSellRpcResult = json::from_value(json["result"].clone()).unwrap();
}

#[test]
fn test_sell_response_format() {
    let bob_passphrase = unwrap!(get_passphrase(&".env.client", "BOB_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}},"required_confirmations":2}
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log! ({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    log!([block_on(enable_coins_eth_electrum(&mm_bob, vec![
        "http://195.201.0.6:8565"
    ]))]);

    log!("Issue bob sell request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "sell",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
        "base_confs": 5,
        "base_nota": true,
        "rel_confs": 4,
        "rel_nota": false,
    }))));
    assert!(rc.0.is_success(), "!sell: {}", rc.1);
    let json: Json = json::from_str(&rc.1).unwrap();
    let _: BuyOrSellRpcResult = json::from_value(json["result"].clone()).unwrap();
}

#[test]
fn test_my_orders_response_format() {
    let bob_passphrase = unwrap!(get_passphrase(&".env.client", "BOB_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}},"required_confirmations":2}
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log! ({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    log!([block_on(enable_coins_eth_electrum(&mm_bob, vec![
        "http://195.201.0.6:8565"
    ]))]);

    log!("Issue bob buy request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "buy",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
        "base_confs": 5,
        "base_nota": true,
        "rel_confs": 4,
        "rel_nota": false,
    }))));
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    log!("Issue bob setprice request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
        "base_confs": 5,
        "base_nota": true,
        "rel_confs": 4,
        "rel_nota": false,
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    log!("Issue bob my_orders request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "my_orders",
    }))));
    assert!(rc.0.is_success(), "!my_orders: {}", rc.1);

    let json: Json = json::from_str(&rc.1).unwrap();
    let _: MyOrdersRpcResult = json::from_value(json["result"].clone()).unwrap();
}

#[test]
fn test_my_orders_after_matched() {
    let bob_passphrase = unwrap!(get_passphrase(&".env.seed", "BOB_PASSPHRASE"));
    let alice_passphrase = unwrap!(get_passphrase(&".env.client", "ALICE_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    let mut mm_alice = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": alice_passphrase,
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    ));
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
    unwrap!(block_on(
        mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    // Enable coins on Bob side. Print the replies in case we need the address.
    let rc = block_on(enable_coins_eth_electrum(&mm_bob, vec!["http://195.201.0.6:8565"]));
    log! ({"enable_coins (bob): {:?}", rc});
    // Enable coins on Alice side. Print the replies in case we need the address.
    let rc = block_on(enable_coins_eth_electrum(&mm_alice, vec!["http://195.201.0.6:8565"]));
    log! ({"enable_coins (alice): {:?}", rc});

    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 2,
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let rc = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 1,
    }))));
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    unwrap!(block_on(mm_bob.wait_for_log(22., |log| {
        log.contains("Entering the maker_swap_loop ETH/JST")
    })));
    unwrap!(block_on(mm_alice.wait_for_log(22., |log| {
        log.contains("Entering the taker_swap_loop ETH/JST")
    })));

    log!("Issue bob my_orders request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "my_orders",
    }))));
    assert!(rc.0.is_success(), "!my_orders: {}", rc.1);

    let json: Json = json::from_str(&rc.1).unwrap();
    let _: MyOrdersRpcResult = json::from_value(json["result"].clone()).unwrap();
    unwrap!(block_on(mm_bob.stop()));
    unwrap!(block_on(mm_alice.stop()));
}

#[test]
fn test_sell_conf_settings() {
    let bob_passphrase = unwrap!(get_passphrase(&".env.client", "BOB_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}},"required_confirmations":2}
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log! ({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    log!([block_on(enable_coins_eth_electrum(&mm_bob, vec![
        "http://195.201.0.6:8565"
    ]))]);

    log!("Issue bob sell request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "sell",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
        "base_confs": 5,
        "base_nota": true,
        "rel_confs": 4,
        "rel_nota": false,
    }))));
    assert!(rc.0.is_success(), "!sell: {}", rc.1);
    let json: Json = json::from_str(&rc.1).unwrap();
    assert_eq!(json["result"]["conf_settings"]["base_confs"], Json::from(5));
    assert_eq!(json["result"]["conf_settings"]["base_nota"], Json::from(true));
    assert_eq!(json["result"]["conf_settings"]["rel_confs"], Json::from(4));
    assert_eq!(json["result"]["conf_settings"]["rel_nota"], Json::from(false));

    // must use coin config as defaults if not set in request
    log!("Issue bob sell request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "sell",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
    }))));
    assert!(rc.0.is_success(), "!sell: {}", rc.1);
    let json: Json = json::from_str(&rc.1).unwrap();
    assert_eq!(json["result"]["conf_settings"]["base_confs"], Json::from(1));
    assert_eq!(json["result"]["conf_settings"]["base_nota"], Json::from(false));
    assert_eq!(json["result"]["conf_settings"]["rel_confs"], Json::from(2));
    assert_eq!(json["result"]["conf_settings"]["rel_nota"], Json::from(false));
}

#[test]
fn test_set_price_conf_settings() {
    let bob_passphrase = unwrap!(get_passphrase(&".env.client", "BOB_PASSPHRASE"));

    let coins = json! ([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}},"required_confirmations":2}
    ]);

    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob")
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log! ({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    log!([block_on(enable_coins_eth_electrum(&mm_bob, vec![
        "http://195.201.0.6:8565"
    ]))]);

    log!("Issue bob sell request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
        "base_confs": 5,
        "base_nota": true,
        "rel_confs": 4,
        "rel_nota": false,
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let json: Json = json::from_str(&rc.1).unwrap();
    assert_eq!(json["result"]["conf_settings"]["base_confs"], Json::from(5));
    assert_eq!(json["result"]["conf_settings"]["base_nota"], Json::from(true));
    assert_eq!(json["result"]["conf_settings"]["rel_confs"], Json::from(4));
    assert_eq!(json["result"]["conf_settings"]["rel_nota"], Json::from(false));

    // must use coin config as defaults if not set in request
    log!("Issue bob sell request");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let json: Json = json::from_str(&rc.1).unwrap();
    assert_eq!(json["result"]["conf_settings"]["base_confs"], Json::from(1));
    assert_eq!(json["result"]["conf_settings"]["base_nota"], Json::from(false));
    assert_eq!(json["result"]["conf_settings"]["rel_confs"], Json::from(2));
    assert_eq!(json["result"]["conf_settings"]["rel_nota"], Json::from(false));
}

#[test]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/683
// trade fee should return numbers in all 3 available formats and
// "amount" must be always in decimal representation for backwards compatibility
fn test_trade_fee_returns_numbers_in_various_formats() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"},"rpcport":80},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0xc0eb7AeD740E1796992A08962c15661bDEB58003"}}}
    ]);

    // start bob and immediately place the order
    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    block_on(enable_coins_eth_electrum(&mm_bob, vec![
        "https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b",
    ]));

    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "get_trade_fee",
        "coin": "RICK",
    }))));
    assert!(rc.0.is_success(), "!get_trade_fee: {}", rc.1);
    let trade_fee_json: Json = json::from_str(&rc.1).unwrap();
    let _amount_dec: BigDecimal = json::from_value(trade_fee_json["result"]["amount"].clone()).unwrap();
    let _amount_rat: BigRational = json::from_value(trade_fee_json["result"]["amount_rat"].clone()).unwrap();
    let _amount_fraction: Fraction = json::from_value(trade_fee_json["result"]["amount_fraction"].clone()).unwrap();
}

#[test]
fn test_orderbook_is_mine_orders() {
    let coins = json!([{"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"},"rpcport":80},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0xc0eb7AeD740E1796992A08962c15661bDEB58003"}}}
    ]);

    // start bob and immediately place the order
    let mut mm_bob = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        }
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap!(block_on(
        mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));
    // Enable coins on Bob side. Print the replies in case we need the "address".
    log! ({"enable_coins (bob): {:?}", block_on (enable_coins_eth_electrum (&mm_bob, vec!["https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b"]))});

    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "MORTY",
        "price": 0.9,
        "volume": "0.9",
    }))));
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let _bob_setprice: Json = unwrap!(json::from_str(&rc.1));

    let mut mm_alice = unwrap!(MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": "alice passphrase",
            "coins": coins,
            "seednodes": [fomat!((mm_bob.ip))],
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "alice" => Some(local_start()),
            _ => None,
        }
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
    log!({"Alice log path: {}", mm_alice.log_path.display()});

    unwrap!(block_on(
        mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
    ));

    // Enable coins on Alice side. Print the replies in case we need the "address".
    log! ({"enable_coins (alice): {:?}", block_on (enable_coins_eth_electrum (&mm_alice, vec!["https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b"]))});

    log!("Give Alice 15 seconds to import the order…");
    thread::sleep(Duration::from_secs(15));

    // Bob orderbook must show 1 mine order
    log!("Get RICK/MORTY orderbook on Bob side");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Bob orderbook "[bob_orderbook]);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Bob RICK/MORTY orderbook must have exactly 1 ask");
    let is_mine = asks[0]["is_mine"].as_bool().unwrap();
    assert_eq!(is_mine, true);

    // Alice orderbook must show 1 not-mine order
    log!("Get RICK/MORTY orderbook on Alice side");
    let rc = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Alice orderbook "[alice_orderbook]);
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Alice RICK/MORTY orderbook must have exactly 1 ask");
    let is_mine = asks[0]["is_mine"].as_bool().unwrap();
    assert_eq!(is_mine, false);

    // make another order by Alice
    let rc = unwrap!(block_on(mm_alice.rpc(json! ({
        "userpass": mm_alice.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "MORTY",
        "price": 1,
        "volume": 0.1,
    }))));
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    log!("Give Bob 15 seconds to import the order…");
    thread::sleep(Duration::from_secs(15));

    // Bob orderbook must show 1 mine and 1 non-mine orders.
    // Request orderbook with reverse base and rel coins to check bids instead of asks
    log!("Get RICK/MORTY orderbook on Bob side");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "MORTY",
        "rel": "RICK",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Bob orderbook "[bob_orderbook]);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    let bids = bob_orderbook["bids"].as_array().unwrap();
    assert!(asks.is_empty(), "Bob MORTY/RICK orderbook must contain an empty asks");
    assert_eq!(bids.len(), 2, "Bob MORTY/RICK orderbook must have exactly 2 bids");
    let mine_orders = bids.iter().filter(|bid| bid["is_mine"].as_bool().unwrap()).count();
    assert_eq!(mine_orders, 1, "Bob RICK/MORTY orderbook must have exactly 1 mine bid");

    // Alice orderbook must show 1 mine and 1 non-mine orders
    log!("Get RICK/MORTY orderbook on Alice side");
    let rc = unwrap!(block_on(mm_bob.rpc(json! ({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "MORTY",
    }))));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Alice orderbook "[alice_orderbook]);
    let asks = alice_orderbook["asks"].as_array().unwrap();
    let bids = alice_orderbook["bids"].as_array().unwrap();
    assert!(bids.is_empty(), "Alice MORTY/RICK orderbook must contain an empty bids");
    assert_eq!(asks.len(), 2, "Alice MORTY/RICK orderbook must have exactly 2 asks");
    let mine_orders = asks.iter().filter(|ask| ask["is_mine"].as_bool().unwrap()).count();
    assert_eq!(
        mine_orders, 1,
        "Alice RICK/MORTY orderbook must have exactly 1 mine bid"
    );
}

// HOWTO
// 1. Install Firefox.
// 2. Install forked version of wasm-bindgen-cli: cargo install wasm-bindgen-cli --git https://github.com/artemii235/wasm-bindgen.git
// 3. Download Gecko driver for your OS: https://github.com/mozilla/geckodriver/releases
// 4. Run HEADLESS_TIMEOUT=120 GECKODRIVER=PATH_TO_GECKO_DRIVER_BIN cargo test --target wasm32-unknown-unknown --features w-bindgen
#[cfg(feature = "w-bindgen")]
mod wasm_bindgen_tests {
    use super::*;
    use futures01::Future;
    use js_sys::Promise;
    use lazy_static::lazy_static;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use wasm_bindgen_test::*;
    use web_sys::console;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen]
    extern "C" {
        fn setInterval(closure: &Closure<FnMut()>, millis: u32) -> f64;
        fn cancelInterval(token: f64);
    }

    pub struct Interval {
        closure: Closure<FnMut()>,
    }

    impl Interval {
        fn new() -> Interval {
            let closure = Closure::new({ common::executor::run });
            Interval { closure }
        }
    }

    unsafe impl Send for Interval {}
    unsafe impl Sync for Interval {}

    lazy_static! {
        static ref EXECUTOR_INTERVAL: Interval = Interval::new();
    }

    #[wasm_bindgen(raw_module = "./js/defined-in-js.js")]
    extern "C" {
        fn sleep(ms: u32) -> Promise;
    }

    #[wasm_bindgen_test]
    async fn test_swap() {
        use crate::mm2::lp_swap::{run_maker_swap, run_taker_swap, MakerSwap, TakerSwap};
        use coins::lp_coininit;
        use common::mm_ctx::MmCtxBuilder;
        use futures::future::join;
        use futures::{Future, TryFutureExt};

        setInterval(&EXECUTOR_INTERVAL.closure, 200);
        let key_pair_taker =
            key_pair_from_seed("spice describe gravity federal blast come thank unfair canal monkey style afraid")
                .unwrap();
        let key_pair_maker =
            key_pair_from_seed("also shoot benefit prefer juice shell elder veteran woman mimic image kidney").unwrap();
        let conf = json!({
            "coins":[{
                {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"},"rpcport":80,"mm2":1},
                {"coin":"JST","name":"jst",,"rpcport":80,"mm2":1,"protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
            }]
        });
        let ctx_taker = MmCtxBuilder::new()
            .with_conf(conf.clone())
            .with_secp256k1_key_pair(key_pair_taker)
            .into_mm_arc();
        let ctx_maker = MmCtxBuilder::new()
            .with_conf(conf)
            .with_secp256k1_key_pair(key_pair_maker)
            .into_mm_arc();

        let req = json!({
            "urls":["http://195.201.0.6:8565"],
            "swap_contract_address":"0xa09ad3cd7e96586ebd05a2607ee56b56fb2db8fd"
        });
        let eth_taker = lp_coininit(&ctx_taker, "ETH", &req).await.unwrap();
        let jst_taker = lp_coininit(&ctx_taker, "JST", &req).await.unwrap();
        let eth_maker = lp_coininit(&ctx_maker, "ETH", &req).await.unwrap();
        let jst_maker = lp_coininit(&ctx_maker, "JST", &req).await.unwrap();
        let taker_swap = TakerSwap::new(
            ctx_taker.clone(),
            [0; 32].into(),
            eth_taker.clone(),
            jst_taker.clone(),
            1.into(),
            1.into(),
            (**ctx_taker.secp256k1_key_pair().public()).into(),
            "7c9319b2-866d-412f-bb82-a311b675fc52".to_owned(),
        );

        let maker_swap = MakerSwap::new(
            ctx_maker.clone(),
            [0; 32].into(),
            eth_maker.clone(),
            jst_maker.clone(),
            1.into(),
            1.into(),
            (**ctx_maker.secp256k1_key_pair().public()).into(),
            "7c9319b2-866d-412f-bb82-a311b675fc52".to_owned(),
        );
        join(run_taker_swap(taker_swap, None), run_maker_swap(maker_swap, None)).await;
    }
}
