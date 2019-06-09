use common::identity;
use common::for_tests::{enable_electrum, from_env_file, mm_dump, mm_spat, LocalStart, MarketMakerIt};
use dirs;
use gstuff::{slurp};
use hyper::StatusCode;
use hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN;
use peers;
use serde_json::{self as json, Value as Json};
use std::borrow::Cow;
use std::collections::HashMap;
use std::env::{self, var};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

// TODO: Consider and/or try moving the integration tests into separate Rust files.
// "Tests in your src files should be unit tests, and tests in tests/ should be integration-style tests."
// - https://doc.rust-lang.org/cargo/guide/tests.html

/// Asks MM to enable the given currency in native mode.  
/// Returns the RPC reply containing the corresponding wallet address.
fn enable_native(mm: &MarketMakerIt, coin: &str, urls: Vec<&str>) -> Json {
    let native = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "enable",
        "coin": coin,
        "urls": urls,
        // Dev chain swap contract address
        "swap_contract_address": "0xa09ad3cd7e96586ebd05a2607ee56b56fb2db8fd",
        "mm2": 1,
    })));
    assert_eq! (native.0, StatusCode::OK, "'enable' failed: {}", native.1);
    unwrap!(json::from_str(&native.1))
}

/// Enables BEER, PIZZA, ETOMIC and ETH.
/// Returns the RPC replies containing the corresponding wallet addresses.
fn enable_coins(mm: &MarketMakerIt) -> Vec<(&'static str, Json)> {
    let mut replies = Vec::new();
    replies.push (("BEER", enable_native (mm, "BEER", vec![])));
    replies.push (("PIZZA", enable_native (mm, "PIZZA", vec![])));
    replies.push (("ETOMIC", enable_native(mm, "ETOMIC", vec![])));
    replies.push (("ETH", enable_native (mm, "ETH", vec!["http://195.201.0.6:8545"])));
    replies
}

fn enable_coins_eth_electrum(mm: &MarketMakerIt, eth_urls: Vec<&str>) -> HashMap<&'static str, Json> {
    let mut replies = HashMap::new();
    replies.insert ("BEER", enable_electrum (mm, "BEER", vec!["electrum1.cipig.net:10022","electrum2.cipig.net:10022","electrum3.cipig.net:10022"]));
    replies.insert ("PIZZA", enable_electrum (mm, "PIZZA", vec!["electrum1.cipig.net:10024","electrum2.cipig.net:10024","electrum3.cipig.net:10024"]));
    replies.insert ("ETOMIC", enable_electrum (mm, "ETOMIC", vec!["electrum1.cipig.net:10025","electrum2.cipig.net:10025"]));
    replies.insert ("ETH", enable_native (mm, "ETH", eth_urls.clone()));
    replies.insert ("JST", enable_native (mm, "JST", eth_urls));
    replies
}

fn addr_from_enable(enable_response: &Json) -> Json {
    enable_response["address"].clone()
}

#[test]
fn test_autoprice_coingecko() {portfolio::portfolio_tests::test_autoprice_coingecko (local_start())}

#[test]
fn test_autoprice_coinmarketcap() {portfolio::portfolio_tests::test_autoprice_coinmarketcap (local_start())}

#[test]
fn test_fundvalue() {portfolio::portfolio_tests::test_fundvalue (local_start())}

/// Integration test for RPC server.
/// Check that MM doesn't crash in case of invalid RPC requests
#[test]
fn test_rpc() {
    let (_, mut mm, _dump_log, _dump_dashboard) = mm_spat (local_start(), &identity);
    unwrap! (mm.wait_for_log (19., &|log| log.contains (">>>>>>>>> DEX stats ")));

    let no_method = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "coin": "BEER",
        "ipaddr": "electrum1.cipig.net",
        "port": 10022
    })));
    assert! (no_method.0.is_server_error());
    assert_eq!((no_method.2)[ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:4000");

    let not_json = unwrap! (mm.rpc_str("It's just a string"));
    assert! (not_json.0.is_server_error());
    assert_eq!((not_json.2)[ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:4000");

    let unknown_method = unwrap! (mm.rpc (json! ({
        "method": "unknown_method",
    })));

    assert! (unknown_method.0.is_server_error());
    assert_eq!((unknown_method.2)[ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:4000");

    let mpnet = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "mpnet",
        "onoff": 1,
    })));
    assert_eq!(mpnet.0, StatusCode::OK);
    assert_eq!((mpnet.2)[ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:4000");

    unwrap! (mm.wait_for_log (1., &|log| log.contains ("MPNET onoff")));

    let version = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "version",
    })));
    assert_eq!(version.0, StatusCode::OK);
    assert_eq!((version.2)[ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:4000");

    let help = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "help",
    })));
    assert_eq!(help.0, StatusCode::OK);
    assert_eq!((help.2)[ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:4000");

    unwrap! (mm.stop());
    unwrap! (mm.wait_for_log (9., &|log| log.contains ("on_stop] firing shutdown_tx!")));
    // TODO (workaround libtorrent hanging in delete) // unwrap! (mm.wait_for_log (9., &|log| log.contains ("LogState] Bye!")));
}

use super::{btc2kmd, lp_main};
use bigdecimal::BigDecimal;

/// Integration (?) test for the "btc2kmd" command line invocation.
/// The argument is the WIF example from https://en.bitcoin.it/wiki/Wallet_import_format.
#[test]
fn test_btc2kmd() {
    let output = unwrap! (btc2kmd ("5HueCGU8rMjxEXxiPuD5BDku4MkFqeZyd4dZ1jvhTVqvbTLvyTJ"));
    assert_eq! (output, "BTC 5HueCGU8rMjxEXxiPuD5BDku4MkFqeZyd4dZ1jvhTVqvbTLvyTJ \
    -> KMD UpRBUQtkA5WqFnSztd7sCYyyhtd4aq6AggQ9sXFh2fXeSnLHtd3Z: \
    privkey 0c28fca386c7a227600b2fe50b7cae11ec86d3bf1fbe471be89827e19d72aa1d");
}

/// This is not a separate test but a helper used by `MarketMakerIt` to run the MarketMaker from the test binary.
#[test]
fn test_mm_start() {
    if let Ok (conf) = var ("_MM2_TEST_CONF") {
        log! ("test_mm_start] Starting the MarketMaker...");
        let conf: Json = unwrap! (json::from_str (&conf));
        unwrap! (lp_main (conf, &|_ctx|()))
    }
}

#[cfg(windows)]
fn chdir (dir: &Path) {
    use std::ffi::CString;
    use winapi::um::processenv::SetCurrentDirectoryA;

    let dir = unwrap! (dir.to_str());
    let dir = unwrap! (CString::new (dir));
    // https://docs.microsoft.com/en-us/windows/desktop/api/WinBase/nf-winbase-setcurrentdirectory
    let rc = unsafe {SetCurrentDirectoryA (dir.as_ptr())};
    assert_ne! (rc, 0);
}

#[cfg(not(windows))]
fn chdir (dir: &Path) {
    unwrap! (nix::unistd::chdir (dir))
}

/// Typically used when the `LOCAL_THREAD_MM` env is set, helping debug the tested MM.  
/// NB: Accessing `lp_main` this function have to reside in the mm2 binary crate. We pass a pointer to it to subcrates.
fn local_start_impl (folder: PathBuf, log_path: PathBuf, mut conf: Json) {
    unwrap! (thread::Builder::new().name ("MM".into()) .spawn (move || {
        if conf["log"].is_null() {
            conf["log"] = unwrap! (log_path.to_str()) .into();
        } else {
            let path = Path::new (unwrap! (conf["log"].as_str(), "log is not a string"));
            assert_eq! (log_path, path);
        }

        log! ({"local_start] MM in a thread, log {:?}.", log_path});

        chdir (&folder);

        unwrap! (lp_main (conf, &|_ctx|()))
    }));
}

fn local_start() -> LocalStart {local_start_impl}

/// Invokes the RPC "notify" method, adding a node to the peer-to-peer ring.
#[test]
fn test_notify() {
    let (_passphrase, mut mm, _dump_log, _dump_dashboard) = mm_spat (local_start(), &identity);
    unwrap! (mm.wait_for_log (19., &|log| log.contains (">>>>>>>>> DEX stats ")));

    let notify = unwrap! (mm.rpc (json! ({
        "method": "notify",
        "rmd160": "9562c4033b6ac1ea2378636a782ce5fdf7ee9a2d",
        "pub": "5eb48483573d44f1b24e33414273384c2f0ae15ecab7f700fb3042f904b09820",
        "pubsecp": "0342407c81e408d9d6cdec35576d7284b712ee4062cb908574b5bc6bb46406f8ad",
        "timestamp": 1541434098,
        "sig":  "1f1e2198d890eeb2fc0004d092ff1266c1be10ca16a0cbe169652c2dc1b3150e5918fd9c7fc5161a8f05f4384eb05fc92e4e9c1abb551795f447b0433954f29990",
        "isLP": "45.32.19.196",
        "session": 1540419658,
    })));
    assert_eq! (notify.0, StatusCode::OK, "notify reply: {:?}", notify);
    //unwrap! (mm.wait_for_log (9., &|log| log.contains ("lp_notify_recv] hailed by peer: 45.32.19.196")));
}

/// https://github.com/artemii235/SuperNET/issues/241
#[test]
fn alice_can_see_the_active_order_after_connection() {
    let coins = json!([
        {"coin":"BEER","asset":"BEER","rpcport":8923,"txversion":4},
        {"coin":"PIZZA","asset":"PIZZA","rpcport":11608,"txversion":4},
        {"coin":"ETOMIC","asset":"ETOMIC","rpcport":10271,"txversion":4},
        {"coin":"ETH","name":"ethereum","etomic":"0x0000000000000000000000000000000000000000","rpcport":80},
        {"coin":"JST","name":"jst","etomic":"0xc0eb7AeD740E1796992A08962c15661bDEB58003"}
    ]);

    // start bob and immediately place the order
    let mut mm_bob = unwrap! (MarketMakerIt::start (
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
        }),
        "db4be27033b636c6644c356ded97b0ad08914fcb8a1e2a1efc915b833c2cbd19".into(),
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump (&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap! (mm_bob.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));
    // Enable coins on Bob side. Print the replies in case we need the "address".
    log! ({"enable_coins (bob): {:?}", enable_coins_eth_electrum (&mm_bob, vec!["http://195.201.0.6:8545"])});
    // issue sell request on Bob side by setting base/rel price
    log!("Issue bob sell request");
    let rc = unwrap! (mm_bob.rpc (json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "BEER",
        "rel": "PIZZA",
        "price": 0.9,
        "volume": "0.9",
    })));
    assert! (rc.0.is_success(), "!setprice: {}", rc.1);

    thread::sleep(Duration::from_secs(12));

    // Bob orderbook must show the new order
    log!("Get BEER/PIZZA orderbook on Bob side");
    let rc = unwrap! (mm_bob.rpc (json! ({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "BEER",
        "rel": "PIZZA",
    })));
    assert! (rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Bob orderbook " [bob_orderbook]);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert!(asks.len() > 0, "Bob BEER/PIZZA asks are empty");
    let vol = asks[0]["maxvolume"].as_f64().unwrap();
    assert_eq!(vol, 0.9);

    let mut mm_alice = unwrap! (MarketMakerIt::start (
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": "alice passphrase",
            "coins": coins,
            "seednodes": [fomat!((mm_bob.ip))],
            "alice_contract":"0xe1d4236c5774d35dc47dcc2e5e0ccfc463a3289c",
            "bob_contract":"0x105aFE60fDC8B5c021092b09E8a042135A4A976E",
            "ethnode":"http://195.201.0.6:8545"
        }),
        "08bfc72a25d860a6399005abc27d1aea35318c137fbaad686ab8d59f5716b473".into(),
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "alice" => Some (local_start()), _ => None}
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump (&mm_alice.log_path);
    log!({"Alice log path: {}", mm_alice.log_path.display()});

    unwrap! (mm_alice.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));

    // Enable coins on Alice side. Print the replies in case we need the "address".
    log! ({"enable_coins (alice): {:?}", enable_coins_eth_electrum (&mm_alice, vec!["http://195.201.0.6:8545"])});

    // give Alice 20 seconds to recognize the Bob, MM2 nodes broadcast their pubkey data every 20 seconds
    thread::sleep(Duration::from_secs(20));

    for _ in 0..2 {
        // Alice should be able to see the order no later than 10 seconds after recognizing the bob
        thread::sleep(Duration::from_secs(10));
        log!("Get BEER/PIZZA orderbook on Alice side");
        let rc = unwrap!(mm_alice.rpc (json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "BEER",
            "rel": "PIZZA",
        })));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("Alice orderbook " [alice_orderbook]);
        let asks = alice_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Alice BEER/PIZZA orderbook must have exactly 1 ask");
        let vol = asks[0]["maxvolume"].as_f64().unwrap();
        assert_eq!(vol, 0.9);
        // orderbook must display valid Bob address
        let address = asks[0]["address"].as_str().unwrap();
        assert_eq!("RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD", address);
    }
}

#[test]
fn test_status() {common::log::tests::test_status()}

#[test]
fn peers_dht() {peers::peers_tests::peers_dht()}

#[test]
fn peers_direct_send() {peers::peers_tests::peers_direct_send()}

#[test]
fn peers_http_fallback_recv() {peers::peers_tests::peers_http_fallback_recv()}

#[test]
fn peers_http_fallback_kv() {peers::peers_tests::peers_http_fallback_kv()}

#[test]
fn test_my_balance() {
    let coins = json!([
        {"coin":"BEER","asset":"BEER","rpcport":8923,"txversion":4},
    ]);

    let mut mm = unwrap! (MarketMakerIt::start (
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": "bob passphrase",
            "coins": coins,
            "i_am_seed": true,
        }),
        "db4be27033b636c6644c356ded97b0ad08914fcb8a1e2a1efc915b833c2cbd19".into(),
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));
    let (_dump_log, _dump_dashboard) = mm_dump (&mm.log_path);
    log!({"log path: {}", mm.log_path.display()});
    unwrap! (mm.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));
    // Enable BEER.
    let json = enable_electrum(&mm, "BEER", vec!["electrum1.cipig.net:10022","electrum2.cipig.net:10022","electrum3.cipig.net:10022"]);
    let balance_on_enable = unwrap!(json["balance"].as_str());
    assert_eq!(balance_on_enable, "1");

    let my_balance = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "my_balance",
        "coin": "BEER",
    })));
    assert_eq! (my_balance.0, StatusCode::OK, "RPC «my_balance» failed with status «{}»", my_balance.0);
    let json: Json = unwrap!(json::from_str(&my_balance.1));
    let my_balance = unwrap!(json["balance"].as_str());
    assert_eq!(my_balance, "1");
    let my_address = unwrap!(json["address"].as_str());
    assert_eq!(my_address, "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD");
}

fn check_set_price_fails(mm: &MarketMakerIt, base: &str, rel: &str) {
    let rc = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "setprice",
        "base": base,
        "rel": rel,
        "price": 0.9
    })));
    assert! (rc.0.is_server_error(), "!setprice success but should be error: {}", rc.1);
}

fn check_buy_fails(mm: &MarketMakerIt, base: &str, rel: &str, vol: f64) {
    let rc = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "buy",
        "base": base,
        "rel": rel,
        "relvolume": vol,
        "price": 0.9
    })));
    assert! (rc.0.is_server_error(), "!buy success but should be error: {}", rc.1);
}

fn check_sell_fails(mm: &MarketMakerIt, base: &str, rel: &str, vol: f64) {
    let rc = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "sell",
        "base": base,
        "rel": rel,
        "basevolume": vol,
        "price": 0.9
    })));
    assert! (rc.0.is_server_error(), "!sell success but should be error: {}", rc.1);
}

#[test]
fn test_check_balance_on_order_post() {
    let coins = json!([
        {"coin":"BEER","asset":"BEER","rpcport":8923,"txversion":4},
        {"coin":"PIZZA","asset":"PIZZA","rpcport":11608,"txversion":4},
        {"coin":"ETOMIC","asset":"ETOMIC","rpcport":10271,"txversion":4},
        {"coin":"ETH","name":"ethereum","etomic":"0x0000000000000000000000000000000000000000","rpcport":80},
        {"coin":"JST","name":"jst","etomic":"0x2b294F029Fde858b2c62184e8390591755521d8E"}
    ]);

    // start bob and immediately place the order
    let mut mm = unwrap! (MarketMakerIt::start (
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": "bob passphrase",
            "coins": coins,
            "i_am_seed": true,
        }),
        "db4be27033b636c6644c356ded97b0ad08914fcb8a1e2a1efc915b833c2cbd19".into(),
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));
    let (_dump_log, _dump_dashboard) = mm_dump (&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap! (mm.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));
    // Enable coins. Print the replies in case we need the "address".
    log! ({"enable_coins (bob): {:?}", enable_coins_eth_electrum (&mm, vec!["http://195.201.0.6:8565"])});
    // issue sell request by setting base/rel price

    // Expect error as PIZZA balance is 0
    check_set_price_fails(&mm, "PIZZA", "BEER");
    // Address has enough BEER, but doesn't have ETH, so setprice call should fail because maker will not have gas to spend ETH taker payment.
    check_set_price_fails(&mm, "BEER", "ETH");
    // Address has enough BEER, but doesn't have ETH, so setprice call should fail because maker will not have gas to spend ERC20 taker payment.
    check_set_price_fails(&mm, "BEER", "JST");

    // Expect error as PIZZA balance is 0
    check_buy_fails(&mm, "BEER", "PIZZA", 0.1);
    // BEER balance is sufficient, but amount is too small, the dex fee will result to dust error from RPC
    check_buy_fails(&mm, "PIZZA", "BEER", 0.000770);
    // Address has enough BEER, but doesn't have ETH, so buy call should fail because taker will not have gas to spend ETH maker payment.
    check_buy_fails(&mm, "ETH", "BEER", 0.1);
    // Address has enough BEER, but doesn't have ETH, so buy call should fail because taker will not have gas to spend ERC20 maker payment.
    check_buy_fails(&mm, "JST", "BEER", 0.1);

    // Expect error as PIZZA balance is 0
    check_sell_fails(&mm, "BEER", "PIZZA", 0.1);
    // BEER balance is sufficient, but amount is too small, the dex fee will result to dust error from RPC
    check_sell_fails(&mm, "PIZZA", "BEER", 0.000770);
    // Address has enough BEER, but doesn't have ETH, so buy call should fail because taker will not have gas to spend ETH maker payment.
    check_sell_fails(&mm, "ETH", "BEER", 0.1);
    // Address has enough BEER, but doesn't have ETH, so buy call should fail because taker will not have gas to spend ERC20 maker payment.
    check_sell_fails(&mm, "JST", "BEER", 0.1);
}

#[test]
fn test_rpc_password_from_json() {
    let coins = json!([
        {"coin":"BEER","asset":"BEER","rpcport":8923,"txversion":4},
        {"coin":"PIZZA","asset":"PIZZA","rpcport":11608,"txversion":4},
    ]);

    // do not allow empty password
    let mut err_mm1 = unwrap!(MarketMakerIt::start (
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "passphrase": "bob passphrase",
            "coins": coins,
            "rpc_password": "",
            "i_am_seed": true,
        }),
        "password".into(),
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));
    unwrap! (err_mm1.wait_for_log (5., &|log| log.contains ("rpc_password must not be empty")));

    // do not allow empty password
    let mut err_mm2 = unwrap!(MarketMakerIt::start (
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "passphrase": "bob passphrase",
            "coins": coins,
            "rpc_password": {"key":"value"},
            "i_am_seed": true,
        }),
        "password".into(),
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));
    unwrap! (err_mm2.wait_for_log (5., &|log| log.contains ("rpc_password must be string")));

    let mut mm = unwrap! (MarketMakerIt::start (
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "passphrase": "bob passphrase",
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));
    let (_dump_log, _dump_dashboard) = mm_dump (&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap! (mm.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));
    let electrum_invalid = unwrap! (mm.rpc (json! ({
        "userpass": "password1",
        "method": "electrum",
        "coin": "BEER",
        "urls": ["electrum2.cipig.net:10022"],
        "mm2": 1,
    })));

    // electrum call must fail if invalid password is provided
    assert! (electrum_invalid.0.is_server_error(),"RPC «electrum» should have failed with server error, but got «{}», response «{}»", electrum_invalid.0, electrum_invalid.1);

    let electrum = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "BEER",
        "urls": ["electrum2.cipig.net:10022"],
        "mm2": 1,
    })));

    // electrum call must be successful with RPC password from config
    assert_eq! (electrum.0, StatusCode::OK, "RPC «electrum» failed with status «{}», response «{}»", electrum.0, electrum.1);

    let electrum = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "PIZZA",
        "urls": ["electrum1.cipig.net:10024"],
        "mm2": 1,
    })));

    // electrum call must be successful with RPC password from config
    assert_eq! (electrum.0, StatusCode::OK, "RPC «electrum» failed with status «{}», response «{}»", electrum.0, electrum.1);

    let orderbook = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "orderbook",
        "base": "BEER",
        "rel": "PIZZA",
    })));

    // orderbook call must be successful with RPC password from config
    assert_eq! (orderbook.0, StatusCode::OK, "RPC «orderbook» failed with status «{}», response «{}»", orderbook.0, orderbook.1);
}

#[test]
fn test_rpc_password_from_json_no_userpass() {
    let coins = json!([
        {"coin":"BEER","asset":"BEER","rpcport":8923,"txversion":4},
    ]);

    let mut mm = unwrap! (MarketMakerIt::start (
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "passphrase": "bob passphrase",
            "coins": coins,
            "i_am_seed": true,
        }),
        "password".into(),
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));
    let (_dump_log, _dump_dashboard) = mm_dump (&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap! (mm.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));
    let electrum = unwrap! (mm.rpc (json! ({
        "method": "electrum",
        "coin": "BEER",
        "urls": ["electrum2.cipig.net:10022"],
    })));

    // electrum call must return 500 status code
    assert! (electrum.0.is_server_error(), "RPC «electrum» should have failed with server error, but got «{}», response «{}»", electrum.0, electrum.1);
}

#[cfg(windows)]
fn get_special_folder_path() -> PathBuf {
    use libc::c_char;
    use std::ffi::CStr;
    use std::mem::zeroed;
    use std::ptr::null_mut;
    use winapi::um::shlobj::SHGetSpecialFolderPathA;
    use winapi::shared::minwindef::MAX_PATH;
    use winapi::um::shlobj::CSIDL_APPDATA;

    let mut buf: [c_char; MAX_PATH + 1] = unsafe {zeroed()};
    // https://docs.microsoft.com/en-us/windows/desktop/api/shlobj_core/nf-shlobj_core-shgetspecialfolderpatha
    let rc = unsafe {SHGetSpecialFolderPathA (null_mut(), buf.as_mut_ptr(), CSIDL_APPDATA, 1)};
    if rc != 1 {panic! ("!SHGetSpecialFolderPathA")}
    Path::new (unwrap! (unsafe {CStr::from_ptr (buf.as_ptr())} .to_str())) .to_path_buf()
}

#[cfg(not(windows))]
fn get_special_folder_path() -> PathBuf {panic!("!windows")}

/// Determines komodod conf file location, emulating komodo/util.cpp/GetConfigFile.
fn komodo_conf_path (ac_name: Option<&'static str>) -> Result<PathBuf, String> {
    let confname: Cow<str> = if let Some (ac_name) = ac_name {
        format! ("{}.conf", ac_name).into()
    } else {
        "komodo.conf".into()
    };

    // komodo/util.cpp/GetDefaultDataDir

    let mut path = match dirs::home_dir() {
        Some (hd) => hd,
        None => Path::new ("/") .to_path_buf()
    };

    if cfg! (windows) {
        // >= Vista: c:\Users\$username\AppData\Roaming
        path = get_special_folder_path();
        path.push ("Komodo");
    } else if cfg! (target_os = "macos") {
        path.push ("Library");
        path.push ("Application Support");
        path.push ("Komodo");
    } else {
        path.push (".komodo");
    }

    if let Some (ac_name) = ac_name {path.push (ac_name)}
    Ok (path.join (&confname[..]))
}

/// Helper function requesting my swap status and checking it's events
fn check_my_swap_status(
    mm: &MarketMakerIt,
    uuid: &str,
    expected_success_events: &Vec<&str>,
    expected_error_events: &Vec<&str>,
    maker_amount: BigDecimal,
    taker_amount: BigDecimal,
) {
    let response = unwrap!(mm.rpc (json! ({
            "userpass": mm.userpass,
            "method": "my_swap_status",
            "params": {
                "uuid": uuid,
            }
        })));
    assert!(response.0.is_success(), "!status of {}: {}", uuid, response.1);
    let status_response: Json = unwrap!(json::from_str(&response.1));
    let success_events: Vec<String> = unwrap!(json::from_value(status_response["result"]["success_events"].clone()));
    assert_eq!(expected_success_events, &success_events);
    let error_events: Vec<String> = unwrap!(json::from_value(status_response["result"]["error_events"].clone()));
    assert_eq!(expected_error_events, &error_events);

    let events_array = unwrap!(status_response["result"]["events"].as_array());
    let actual_maker_amount = unwrap!(json::from_value(events_array[0]["event"]["data"]["maker_amount"].clone()));
    assert_eq!(maker_amount, actual_maker_amount);
    let actual_taker_amount = unwrap!(json::from_value(events_array[0]["event"]["data"]["taker_amount"].clone()));
    assert_eq!(taker_amount, actual_taker_amount);
    let actual_events = events_array.iter().map(|item| unwrap!(item["event"]["type"].as_str()));
    let actual_events: Vec<&str> = actual_events.collect();
    assert_eq!(expected_success_events, &actual_events);
}

fn check_stats_swap_status(
    mm: &MarketMakerIt,
    uuid: &str,
    maker_expected_events: &Vec<&str>,
    taker_expected_events: &Vec<&str>,
) {
    let response = unwrap!(mm.rpc (json! ({
            "method": "stats_swap_status",
            "params": {
                "uuid": uuid,
            }
        })));
    assert!(response.0.is_success(), "!status of {}: {}", uuid, response.1);
    let status_response: Json = unwrap!(json::from_str(&response.1));
    let maker_events_array = unwrap!(status_response["result"]["maker"]["events"].as_array());
    let taker_events_array = unwrap!(status_response["result"]["taker"]["events"].as_array());
    let maker_actual_events = maker_events_array.iter().map(|item| unwrap!(item["event"]["type"].as_str()));
    let maker_actual_events: Vec<&str> = maker_actual_events.collect();
    let taker_actual_events = taker_events_array.iter().map(|item| unwrap!(item["event"]["type"].as_str()));
    let taker_actual_events: Vec<&str> = taker_actual_events.collect();
    assert_eq!(maker_expected_events, &maker_actual_events);
    assert_eq!(taker_expected_events, &taker_actual_events);
}

fn check_recent_swaps(
    mm: &MarketMakerIt,
    expected_len: usize,
) {
    let response = unwrap!(mm.rpc (json! ({
            "method": "my_recent_swaps",
            "userpass": mm.userpass,
        })));
    assert!(response.0.is_success(), "!status of my_recent_swaps {}", response.1);
    let swaps_response: Json = unwrap!(json::from_str(&response.1));
    let swaps: &Vec<Json> = unwrap!(swaps_response["result"]["swaps"].as_array());
    assert_eq!(expected_len, swaps.len());
}

/// Trading test using coins with remote RPC (Electrum, ETH nodes), it needs only ENV variables to be set, coins daemons are not required.
/// Trades few pairs concurrently to speed up the process and also act like "load" test
fn trade_base_rel_electrum(pairs: Vec<(&str, &str)>) {
    let (bob_file_passphrase, _bob_file_userpass) = from_env_file (slurp (&".env.seed"));
    let (alice_file_passphrase, _alice_file_userpass) = from_env_file (slurp (&".env.client"));

    let bob_passphrase = unwrap! (var ("BOB_PASSPHRASE") .ok().or (bob_file_passphrase), "No BOB_PASSPHRASE or .env.seed/PASSPHRASE");
    let alice_passphrase = unwrap! (var ("ALICE_PASSPHRASE") .ok().or (alice_file_passphrase), "No ALICE_PASSPHRASE or .env.client/PASSPHRASE");

    let coins = json! ([
        {"coin":"BEER","asset":"BEER"},
        {"coin":"PIZZA","asset":"PIZZA"},
        {"coin":"ETOMIC","asset":"ETOMIC"},
        {"coin":"ETH","name":"ethereum","etomic":"0x0000000000000000000000000000000000000000"},
        {"coin":"JST","name":"jst","etomic":"0x2b294F029Fde858b2c62184e8390591755521d8E"}
    ]);

    let mut mm_bob = unwrap! (MarketMakerIt::start (
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
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump (&mm_bob.log_path);
    log! ({"Bob log path: {}", mm_bob.log_path.display()});

    // Both Alice and Bob might try to bind on the "0.0.0.0:47773" DHT port in this test
    // (because the local "127.0.0.*:47773" addresses aren't that useful for DHT).
    // We want to give Bob a headstart in acquiring the port,
    // because Alice will then be able to directly reach it (thanks to "seednode").
    // Direct communication is not required in this test, but it's nice to have.
    unwrap! (mm_bob.wait_for_log (9., &|log| log.contains ("preferred port")));

    let mut mm_alice = unwrap! (MarketMakerIt::start (
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
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "alice" => Some (local_start()), _ => None}
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump (&mm_alice.log_path);
    log! ({"Alice log path: {}", mm_alice.log_path.display()});

    // wait until both nodes RPC API is active
    unwrap! (mm_bob.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));
    unwrap! (mm_alice.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));

    // Enable coins on Bob side. Print the replies in case we need the address.
    log! ({"enable_coins (bob): {:?}", enable_coins_eth_electrum (&mm_bob, vec!["http://195.201.0.6:8565"])});
    // Enable coins on Alice side. Print the replies in case we need the address.
    log! ({"enable_coins (alice): {:?}", enable_coins_eth_electrum (&mm_alice, vec!["http://195.201.0.6:8565"])});

    // unwrap! (mm_alice.wait_for_log (999., &|log| log.contains ("set pubkey for ")));

    let mut uuids = vec![];

    // issue sell request on Bob side by setting base/rel price
    for (base, rel) in pairs.iter() {
        log!("Issue bob " (base) "/" (rel) " sell request");
            let rc = unwrap!(mm_bob.rpc (json! ({
            "userpass": mm_bob.userpass,
            "method": "sell",
            "base": base,
            "rel": rel,
            "price": 1,
            "volume": 0.1
        })));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    }
    // allow order to be converted to maker if it's not matched in 30 seconds
    thread::sleep(Duration::from_secs(32));
    for (base, rel) in pairs.iter() {
        log!("Issue alice " (base) "/" (rel) " buy request");
        let rc = unwrap!(mm_alice.rpc (json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": base,
            "rel": rel,
            "volume": 0.1,
            "price": 2
        })));
        assert!(rc.0.is_success(), "!buy: {}", rc.1);
        let buy_json: Json = unwrap!(serde_json::from_str(&rc.1));
        uuids.push(unwrap!(buy_json["result"]["uuid"].as_str()).to_owned());
    }

    for (base, rel) in pairs.iter() {
        // ensure the swaps are started
        unwrap!(mm_alice.wait_for_log (5., &|log| log.contains (&format!("Entering the taker_swap_loop {}/{}", base, rel))));
        unwrap!(mm_bob.wait_for_log (5., &|log| log.contains (&format!("Entering the maker_swap_loop {}/{}", base, rel))));
    }

    let maker_success_events = vec!["Started", "Negotiated", "TakerFeeValidated", "MakerPaymentSent",
                                    "TakerPaymentReceived", "TakerPaymentWaitConfirmStarted",
                                    "TakerPaymentValidatedAndConfirmed", "TakerPaymentSpent", "Finished"];

    let maker_error_events = vec!["StartFailed", "NegotiateFailed", "TakerFeeValidateFailed",
                                  "MakerPaymentTransactionFailed", "MakerPaymentDataSendFailed",
                                  "TakerPaymentValidateFailed", "TakerPaymentSpendFailed", "MakerPaymentRefunded",
                                  "MakerPaymentRefundFailed"];

    let taker_success_events = vec!["Started", "Negotiated", "TakerFeeSent", "MakerPaymentReceived",
                                    "MakerPaymentWaitConfirmStarted", "MakerPaymentValidatedAndConfirmed",
                                    "TakerPaymentSent", "TakerPaymentSpent", "MakerPaymentSpent", "Finished"];

    let taker_error_events = vec!["StartFailed", "NegotiateFailed", "TakerFeeSendFailed", "MakerPaymentValidateFailed",
                                  "TakerPaymentTransactionFailed", "TakerPaymentDataSendFailed", "TakerPaymentWaitForSpendFailed",
                                  "MakerPaymentSpendFailed", "TakerPaymentRefunded", "TakerPaymentRefundFailed"];

    for uuid in uuids.iter() {
        unwrap!(mm_bob.wait_for_log (600., &|log| log.contains (&format!("[swap uuid={}] Finished", uuid))));
        unwrap!(mm_alice.wait_for_log (600., &|log| log.contains (&format!("[swap uuid={}] Finished", uuid))));
        check_my_swap_status(
            &mm_alice,
            &uuid,
            &taker_success_events,
            &taker_error_events,
            "0.1".parse().unwrap(),
            "0.1".parse().unwrap(),
        );

        check_my_swap_status(
            &mm_bob,
            &uuid,
            &maker_success_events,
            &maker_error_events,
            "0.1".parse().unwrap(),
            "0.1".parse().unwrap(),
        );
    }
    // give nodes 3 seconds to broadcast their swaps data
    thread::sleep(Duration::from_secs(3));
    for uuid in uuids.iter() {
        check_stats_swap_status(
            &mm_alice,
            &uuid,
            &maker_success_events,
            &taker_success_events,
        );

        check_stats_swap_status(
            &mm_bob,
            &uuid,
            &maker_success_events,
            &taker_success_events,
        );
    }

    check_recent_swaps(&mm_alice, uuids.len());
    check_recent_swaps(&mm_bob, uuids.len());
    for (base, rel) in pairs.iter() {
        log!("Get " (base) "/" (rel) " orderbook");
        let rc = unwrap!(mm_bob.rpc (json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": base,
            "rel": rel,
        })));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!((base) "/" (rel) " orderbook " [bob_orderbook]);

        let bids = bob_orderbook["bids"].as_array().unwrap();
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(0, bids.len(), "{} {} bids must be empty", base, rel);
        assert_eq!(0, asks.len(), "{} {} asks must be empty", base, rel);
    }
    unwrap! (mm_bob.stop());
    unwrap! (mm_alice.stop());
}

#[test]
#[ignore]  // TODO: Re-enable once the PIZZA trades are fixed.
fn trade_test_electrum_and_eth_coins() {
    trade_base_rel_electrum(vec![("BEER", "ETOMIC"), ("ETH", "JST")]);
}

fn trade_base_rel_native(base: &str, rel: &str) {
    let beer_cfp = unwrap! (komodo_conf_path (Some ("BEER")));
    let pizza_cfp = unwrap! (komodo_conf_path (Some ("PIZZA")));
    let etomic_cfp = unwrap! (komodo_conf_path (Some ("ETOMIC")));
    assert! (beer_cfp.exists(), "BEER config {:?} is not found", beer_cfp);
    assert! (pizza_cfp.exists(), "PIZZA config {:?} is not found", pizza_cfp);
    assert! (etomic_cfp.exists(), "ETOMIC config {:?} is not found", etomic_cfp);

    let (bob_file_passphrase, bob_file_userpass) = from_env_file (slurp (&".env.seed"));
    let (alice_file_passphrase, alice_file_userpass) = from_env_file (slurp (&".env.client"));

    let bob_passphrase = unwrap! (var ("BOB_PASSPHRASE") .ok().or (bob_file_passphrase), "No BOB_PASSPHRASE or .env.seed/PASSPHRASE");
    let bob_userpass = unwrap! (var ("BOB_USERPASS") .ok().or (bob_file_userpass), "No BOB_USERPASS or .env.seed/USERPASS");
    let alice_passphrase = unwrap! (var ("ALICE_PASSPHRASE") .ok().or (alice_file_passphrase), "No ALICE_PASSPHRASE or .env.client/PASSPHRASE");
    let alice_userpass = unwrap! (var ("ALICE_USERPASS") .ok().or (alice_file_userpass), "No ALICE_USERPASS or .env.client/USERPASS");

    let coins = json! ([
        {"coin":"BEER","asset":"BEER","confpath":unwrap!(beer_cfp.to_str())},
        {"coin":"PIZZA","asset":"PIZZA","confpath":unwrap!(pizza_cfp.to_str())},
        {"coin":"ETOMIC","asset":"ETOMIC","confpath":unwrap!(etomic_cfp.to_str())},
        {"coin":"ETH","name":"ethereum","etomic":"0x0000000000000000000000000000000000000000","rpcport":80}
    ]);

    let mut mm_bob = unwrap! (MarketMakerIt::start (
        json! ({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
            "passphrase": bob_passphrase,
            "coins": coins,
        }),
        bob_userpass,
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));

    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump (&mm_bob.log_path);
    log! ({"Bob log path: {}", mm_bob.log_path.display()});

    // Both Alice and Bob might try to bind on the "0.0.0.0:47773" DHT port in this test
    // (because the local "127.0.0.*:47773" addresses aren't that useful for DHT).
    // We want to give Bob a headstart in acquiring the port,
    // because Alice will then be able to directly reach it (thanks to "seednode").
    // Direct communication is not required in this test, but it's nice to have.
    // The port differs for another netid, should be 43804 for 9000
    unwrap! (mm_bob.wait_for_log (9., &|log| log.contains ("preferred port 43804 drill true")));

    let mut mm_alice = unwrap! (MarketMakerIt::start (
        json! ({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": alice_passphrase,
            "coins": coins,
            // We're using the open (non-NAT) netid 9000 seed instead, 195.201.42.102 // "seednode": fomat!((mm_bob.ip))
        }),
        alice_userpass,
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "alice" => Some (local_start()), _ => None}
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump (&mm_alice.log_path);
    log! ({"Alice log path: {}", mm_alice.log_path.display()});

    // wait until both nodes RPC API is active
    unwrap! (mm_bob.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));
    unwrap! (mm_alice.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));

    // Enable coins on Bob side. Print the replies in case we need the "smartaddress".
    log! ({"enable_coins (bob): {:?}", enable_coins (&mm_bob)});
    // Enable coins on Alice side. Print the replies in case we need the "smartaddress".
    log! ({"enable_coins (alice): {:?}", enable_coins (&mm_alice)});

    // Both the Taker and the Maker should connect to the netid 9000 open (non-NAT) seed node.
    // NB: Long wayt as there might be delays in the seed node from us reusing the 127.0.0.* IPs with different keys.
    unwrap! (mm_bob.wait_for_log (999., &|log| log.contains ("set pubkey for ")));
    unwrap! (mm_alice.wait_for_log (999., &|log| log.contains ("set pubkey for ")));

    // issue sell request on Bob side by setting base/rel price
    log! ("Issue bob sell request");
    let rc = unwrap! (mm_bob.rpc (json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": base,
        "rel": rel,
        "price": 0.9
    })));
    assert! (rc.0.is_success(), "!setprice: {}", rc.1);

    // issue base/rel buy request from Alice side
    thread::sleep (Duration::from_secs (2));
    log! ("Issue alice buy request");
    let rc = unwrap! (mm_alice.rpc (json! ({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": base,
        "rel": rel,
        "relvolume": 0.1,
        "price": 1
    })));
    assert! (rc.0.is_success(), "!buy: {}", rc.1);

    // ensure the swap started
    unwrap! (mm_alice.wait_for_log (99., &|log| log.contains ("Entering the taker_swap_loop")));
    unwrap! (mm_bob.wait_for_log (20., &|log| log.contains ("Entering the maker_swap_loop")));

    // wait for swap to complete on both sides
    unwrap! (mm_alice.wait_for_log (600., &|log| log.contains ("Swap finished successfully")));
    unwrap! (mm_bob.wait_for_log (600., &|log| log.contains ("Swap finished successfully")));

    unwrap! (mm_bob.stop());
    unwrap! (mm_alice.stop());
}

/// Integration test for PIZZA/BEER and BEER/PIZZA trade
/// This test is ignored because as of now it requires additional environment setup:
/// PIZZA and ETOMIC daemons must be running and fully synced for swaps to be successful
/// The trades can't be executed concurrently now for 2 reasons:
/// 1. Bob node starts listening 47772 port on all interfaces so no more Bobs can be started at once
/// 2. Current UTXO handling algo might result to conflicts between concurrently running nodes
/// 
/// Steps that are currently necessary to run this test:
/// 
/// Obtain the wallet binaries (komodod, komodo-cli) from the [Agama wallet](https://github.com/KomodoPlatform/Agama/releases/).
/// (Or use the Docker image artempikulin/komodod-etomic).
/// (Or compile them from [source](https://github.com/jl777/komodo/tree/dev))
/// 
/// Obtain ~/.zcash-params (c:/Users/$username/AppData/Roaming/ZcashParams on Windows).
/// 
/// Start the wallets
/// 
///     komodod -ac_name=PIZZA -ac_supply=100000000 -addnode=24.54.206.138 -addnode=78.47.196.146
/// 
/// and
/// 
///     komodod -ac_name=ETOMIC -ac_supply=100000000 -addnode=78.47.196.146
/// 
/// and (if you want to test BEER coin):
///
///     komodod -ac_name=BEER -ac_supply=100000000 -addnode=78.47.196.146 -addnode=43.245.162.106 -addnode=88.99.153.2 -addnode=94.130.173.120 -addnode=195.201.12.150 -addnode=23.152.0.28
///
/// Get rpcuser and rpcpassword from ETOMIC/ETOMIC.conf
/// (c:/Users/$username/AppData/Roaming/Komodo/ETOMIC/ETOMIC.conf on Windows)
/// and run
/// 
///     komodo-cli -ac_name=ETOMIC importaddress RKGn1jkeS7VNLfwY74esW7a8JFfLNj1Yoo
/// 
/// Share the wallet information with the test. On Windows:
/// 
///     set BOB_PASSPHRASE=...
///     set BOB_USERPASS=...
///     set ALICE_PASSPHRASE=...
///     set ALICE_USERPASS=...
/// 
/// And run the test:
/// 
///     cargo test trade_etomic_pizza -- --nocapture --ignored
#[test]
#[ignore]
fn trade_pizza_eth() {
    trade_base_rel_native("PIZZA", "ETH");
}

#[test]
#[ignore]
fn trade_eth_pizza() {
    trade_base_rel_native("ETH", "PIZZA");
}

#[test]
#[ignore]
fn trade_beer_eth() {
    trade_base_rel_native("BEER", "ETH");
}

#[test]
#[ignore]
fn trade_eth_beer() {
    trade_base_rel_native("ETH", "BEER");
}

#[test]
#[ignore]
fn trade_pizza_beer() {
    trade_base_rel_native("PIZZA", "BEER");
}

#[test]
#[ignore]
fn trade_beer_pizza() {
    trade_base_rel_native("BEER", "PIZZA");
}

#[test]
#[ignore]
fn trade_pizza_etomic() {
    trade_base_rel_native("PIZZA", "ETOMIC");
}

#[test]
#[ignore]
fn trade_etomic_pizza() {
    trade_base_rel_native("ETOMIC", "PIZZA");
}

fn withdraw_and_send(mm: &MarketMakerIt, coin: &str, to: &str, enable_res: &HashMap<&'static str, Json>, expected_bal_change: f64) {
    let addr = addr_from_enable(unwrap!(enable_res.get(coin)));

    let withdraw = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": coin,
        "to": to,
        "amount": 0.001
    })));

    assert! (withdraw.0.is_success(), "!{} withdraw: {}", coin, withdraw.1);
    let withdraw_json: Json = unwrap!(json::from_str(&withdraw.1));
    assert_eq!(Some(&vec![Json::from(to)]), withdraw_json["to"].as_array());
    assert_eq!(Some(expected_bal_change), withdraw_json["my_balance_change"].as_f64());
    assert_eq!(Some(&vec![addr]), withdraw_json["from"].as_array());

    let send = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "send_raw_transaction",
        "coin": coin,
        "tx_hex": withdraw_json["tx_hex"]
    })));
    assert! (send.0.is_success(), "!{} send: {}", coin, send.1);
    let send_json: Json = unwrap!(json::from_str(&send.1));
    assert_eq! (withdraw_json["tx_hash"], send_json["tx_hash"]);
}

#[test]
fn test_withdraw_and_send() {
    let (alice_file_passphrase, _alice_file_userpass) = from_env_file (slurp (&".env.client"));

    let alice_passphrase = unwrap! (var ("ALICE_PASSPHRASE") .ok().or (alice_file_passphrase), "No ALICE_PASSPHRASE or .env.client/PASSPHRASE");

    let coins = json! ([
        {"coin":"BEER","asset":"BEER"},
        {"coin":"PIZZA","asset":"PIZZA"},
        {"coin":"ETOMIC","asset":"ETOMIC"},
        {"coin":"ETH","name":"ethereum","etomic":"0x0000000000000000000000000000000000000000"},
        {"coin":"JST","name":"jst","etomic":"0x2b294F029Fde858b2c62184e8390591755521d8E"}
    ]);

    let mut mm_alice = unwrap! (MarketMakerIt::start (
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
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "alice" => Some (local_start()), _ => None}
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump (&mm_alice.log_path);
    log! ({"Alice log path: {}", mm_alice.log_path.display()});

    // wait until RPC API is active
    unwrap! (mm_alice.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));

    // Enable coins. Print the replies in case we need the address.
    let enable_res = enable_coins_eth_electrum (&mm_alice, vec!["http://195.201.0.6:8565"]);
    log! ("enable_coins (alice): " [enable_res]);
    withdraw_and_send(&mm_alice, "PIZZA", "RJTYiYeJ8eVvJ53n2YbrVmxWNNMVZjDGLh", &enable_res, -0.00101);
    // dev chain gas price is 0 so ETH expected balance change doesn't include the fee
    withdraw_and_send(&mm_alice, "ETH", "0x657980d55733B41c0C64c06003864e1aAD917Ca7", &enable_res, -0.001);
    withdraw_and_send(&mm_alice, "JST", "0x657980d55733B41c0C64c06003864e1aAD917Ca7", &enable_res, -0.001);
    unwrap!(mm_alice.stop());
}

/// Ensure that swap status return the 404 status code if swap is not found
#[test]
fn test_swap_status() {
    let coins = json! ([{"coin":"BEER","asset":"BEER"},]);

    let mut mm = unwrap! (MarketMakerIt::start (
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
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "alice" => Some (local_start()), _ => None}
    ));

    unwrap! (mm.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));

    let my_swap = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "my_swap_status",
        "params": {
            "uuid":"random",
        }
    })));

    assert_eq! (my_swap.0, StatusCode::NOT_FOUND, "!not found status code: {}", my_swap.1);

    let stats_swap = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "stats_swap_status",
        "params": {
            "uuid":"random",
        }
    })));

    assert_eq! (stats_swap.0, StatusCode::NOT_FOUND, "!not found status code: {}", stats_swap.1);
}

/// Ensure that setprice/buy/sell calls deny base == rel
/// https://github.com/artemii235/SuperNET/issues/363
#[test]
fn test_order_errors_when_base_equal_rel() {
    let coins = json!([
        {"coin":"BEER","asset":"BEER","rpcport":8923,"txversion":4},
    ]);

    let mut mm = unwrap! (MarketMakerIt::start (
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
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));
    let (_dump_log, _dump_dashboard) = mm_dump (&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap! (mm.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));
    enable_electrum (&mm, "BEER", vec!["electrum2.cipig.net:10022"]);

    let rc = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "setprice",
        "base": "BEER",
        "rel": "BEER",
        "price": 0.9
    })));
    assert! (rc.0.is_server_error(), "setprice should have failed, but got {:?}", rc);

    let rc = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "buy",
        "base": "BEER",
        "rel": "BEER",
        "price": 0.9,
        "relvolume": 0.1,
    })));
    assert! (rc.0.is_server_error(), "buy should have failed, but got {:?}", rc);

    let rc = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "sell",
        "base": "BEER",
        "rel": "BEER",
        "price": 0.9,
        "basevolume": 0.1,
    })));
    assert! (rc.0.is_server_error(), "sell should have failed, but got {:?}", rc);
}

fn startup_passphrase(passphrase: &str, expected_address: &str) {
    let coins = json!([
        {"coin":"KMD","rpcport":8923,"txversion":4},
    ]);

    let mut mm = unwrap! (MarketMakerIt::start (
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
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));
    let (_dump_log, _dump_dashboard) = mm_dump (&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap! (mm.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));
    let enable = enable_electrum (&mm, "KMD", vec!["electrum1.cipig.net:10001"]);
    let addr = addr_from_enable(&enable);
    assert_eq!(Json::from(expected_address), addr);
    unwrap!(mm.stop());
}

/// MM2 should detect if passphrase is WIF or 0x-prefixed hex encoded privkey and parse it properly.
/// https://github.com/artemii235/SuperNET/issues/396
#[test]
fn test_startup_passphrase() {
    // seed phrase
    startup_passphrase("bob passphrase", "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD");
    // WIF
    startup_passphrase("UvCjJf4dKSs2vFGVtCnUTAhR5FTZGdg43DDRa9s7s5DV1sSDX14g", "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD");
    // 0x prefixed hex
    startup_passphrase("0xb8c774f071de08c7fd8f62b97f1a5726f6ce9f1bcf141b70b86689254ed6714e", "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD");
}

/// MM2 should allow to issue several buy/sell calls in a row without delays.
/// https://github.com/artemii235/SuperNET/issues/245
#[test]
fn test_multiple_buy_sell_no_delay() {
    let coins = json!([
        {"coin":"BEER","asset":"BEER","txversion":4},
        {"coin":"PIZZA","asset":"PIZZA","txversion":4},
        {"coin":"ETOMIC","asset":"ETOMIC","txversion":4},
    ]);

    let (bob_file_passphrase, _bob_file_userpass) = from_env_file (slurp (&".env.seed"));
    let bob_passphrase = unwrap! (var ("BOB_PASSPHRASE") .ok().or (bob_file_passphrase), "No BOB_PASSPHRASE or .env.seed/PASSPHRASE");

    let mut mm = unwrap! (MarketMakerIt::start (
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
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));
    let (_dump_log, _dump_dashboard) = mm_dump (&mm.log_path);
    log!({"Log path: {}", mm.log_path.display()});
    unwrap! (mm.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));
    enable_electrum (&mm, "BEER", vec!["electrum1.cipig.net:10022", "electrum2.cipig.net:10022", "electrum3.cipig.net:10022"]);
    enable_electrum (&mm, "PIZZA", vec!["electrum1.cipig.net:10024", "electrum2.cipig.net:10024", "electrum3.cipig.net:10024"]);
    enable_electrum (&mm, "ETOMIC", vec!["electrum1.cipig.net:10025", "electrum2.cipig.net:10025", "electrum3.cipig.net:10025"]);

    let rc = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "buy",
        "base": "BEER",
        "rel": "PIZZA",
        "price": 1,
        "volume": 0.1,
    })));
    assert! (rc.0.is_success(), "buy should have succeed, but got {:?}", rc);

    let rc = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "buy",
        "base": "BEER",
        "rel": "ETOMIC",
        "price": 1,
        "volume": 0.1,
    })));
    assert! (rc.0.is_success(), "buy should have succeed, but got {:?}", rc);
    thread::sleep(Duration::from_secs(40));

    log!("Get BEER/PIZZA orderbook");
    let rc = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "orderbook",
        "base": "BEER",
        "rel": "PIZZA",
    })));
    assert! (rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("BEER/PIZZA orderbook " [bob_orderbook]);
    let bids = bob_orderbook["bids"].as_array().unwrap();
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert!(bids.len() > 0, "BEER/PIZZA bids are empty");
    assert_eq!(0, asks.len(), "BEER/PIZZA asks are not empty");
    let vol = bids[0]["maxvolume"].as_f64().unwrap();
    assert_eq!(0.1, vol);

    log!("Get BEER/ETOMIC orderbook");
    let rc = unwrap! (mm.rpc (json! ({
        "userpass": mm.userpass,
        "method": "orderbook",
        "base": "BEER",
        "rel": "ETOMIC",
    })));
    assert! (rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("BEER/ETOMIC orderbook " [bob_orderbook]);
    let bids = bob_orderbook["bids"].as_array().unwrap();
    assert!(bids.len() > 0, "BEER/ETOMIC bids are empty");
    assert_eq!(asks.len(), 0, "BEER/ETOMIC asks are not empty");
    let vol = bids[0]["maxvolume"].as_f64().unwrap();
    assert_eq!(vol, 0.1);
}

/// https://github.com/artemii235/SuperNET/issues/398
#[test]
fn test_cancel_order() {
    let coins = json!([
        {"coin":"BEER","asset":"BEER","rpcport":8923,"txversion":4},
        {"coin":"PIZZA","asset":"PIZZA","rpcport":11608,"txversion":4},
        {"coin":"ETOMIC","asset":"ETOMIC","rpcport":10271,"txversion":4},
        {"coin":"ETH","name":"ethereum","etomic":"0x0000000000000000000000000000000000000000","rpcport":80},
        {"coin":"JST","name":"jst","etomic":"0xc0eb7AeD740E1796992A08962c15661bDEB58003"}
    ]);

    // start bob and immediately place the order
    let mut mm_bob = unwrap! (MarketMakerIt::start (
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
        }),
        "db4be27033b636c6644c356ded97b0ad08914fcb8a1e2a1efc915b833c2cbd19".into(),
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump (&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap! (mm_bob.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));
    // Enable coins on Bob side. Print the replies in case we need the "address".
    log! ({"enable_coins (bob): {:?}", enable_coins_eth_electrum (&mm_bob, vec!["http://195.201.0.6:8545"])});
    // issue sell request on Bob side by setting base/rel price
    log!("Issue bob sell request");
    let rc = unwrap! (mm_bob.rpc (json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "BEER",
        "rel": "PIZZA",
        "price": 0.9,
        "volume": "0.9",
    })));
    assert! (rc.0.is_success(), "!setprice: {}", rc.1);
    let setprice_json: Json = unwrap!(json::from_str(&rc.1));
    log!([setprice_json]);

    let mut mm_alice = unwrap! (MarketMakerIt::start (
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": "alice passphrase",
            "coins": coins,
            "seednodes": [fomat!((mm_bob.ip))],
            "alice_contract":"0xe1d4236c5774d35dc47dcc2e5e0ccfc463a3289c",
            "bob_contract":"0x105aFE60fDC8B5c021092b09E8a042135A4A976E",
            "ethnode":"http://195.201.0.6:8545"
        }),
        "08bfc72a25d860a6399005abc27d1aea35318c137fbaad686ab8d59f5716b473".into(),
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "alice" => Some (local_start()), _ => None}
    ));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump (&mm_alice.log_path);
    log!({"Alice log path: {}", mm_alice.log_path.display()});

    unwrap! (mm_alice.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));

    // Enable coins on Alice side. Print the replies in case we need the "address".
    log! ({"enable_coins (alice): {:?}", enable_coins_eth_electrum (&mm_alice, vec!["http://195.201.0.6:8545"])});

    // give Alice 40 seconds to recognize the Bob and import the order
    thread::sleep(Duration::from_secs(40));

    log!("Get BEER/PIZZA orderbook on Alice side");
    let rc = unwrap!(mm_alice.rpc (json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "BEER",
        "rel": "PIZZA",
    })));
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Alice orderbook " [alice_orderbook]);
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Alice BEER/PIZZA orderbook must have exactly 1 ask");

    let cancel_rc = unwrap! (mm_bob.rpc (json! ({
        "userpass": mm_bob.userpass,
        "method": "cancel_order",
        "uuid": setprice_json["result"]["uuid"],
    })));
    assert!(cancel_rc.0.is_success(), "!cancel_order: {}", rc.1);

    thread::sleep(Duration::from_secs(11));

    // Bob orderbook must show no orders
    log!("Get BEER/PIZZA orderbook on Bob side");
    let rc = unwrap! (mm_bob.rpc (json! ({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "BEER",
        "rel": "PIZZA",
    })));
    assert! (rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Bob orderbook " [bob_orderbook]);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 0, "Bob BEER/PIZZA asks are not empty");

    // Alice orderbook must show no orders
    log!("Get BEER/PIZZA orderbook on Alice side");
    let rc = unwrap! (mm_alice.rpc (json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "BEER",
        "rel": "PIZZA",
    })));
    assert! (rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
    log!("Alice orderbook " [alice_orderbook]);
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 0, "Alice BEER/PIZZA asks are not empty");
}

/// https://github.com/artemii235/SuperNET/issues/367
/// Electrum requests should success if at least 1 server successfully connected,
/// all others might end up with DNS resolution errors, TCP connection errors, etc.
#[test]
fn test_electrum_enable_conn_errors() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK"},
        {"coin":"MORTY","asset":"MORTY"},
    ]);

    // start bob and immediately place the order
    let mut mm_bob = unwrap! (MarketMakerIt::start (
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
        }),
        "db4be27033b636c6644c356ded97b0ad08914fcb8a1e2a1efc915b833c2cbd19".into(),
        match var ("LOCAL_THREAD_MM") {Ok (ref e) if e == "bob" => Some (local_start()), _ => None}
    ));
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump (&mm_bob.log_path);
    log!({"Bob log path: {}", mm_bob.log_path.display()});
    unwrap! (mm_bob.wait_for_log (22., &|log| log.contains (">>>>>>>>> DEX stats ")));
    // Using working servers and few else with random ports to trigger "connection refused"
    enable_electrum(&mm_bob, "RICK", vec![
        "electrum3.cipig.net:10017",
        "electrum2.cipig.net:10017",
        "electrum1.cipig.net:10017",
        "electrum1.cipig.net:60017",
        "electrum1.cipig.net:60018",
    ]);
    // use random domain name to trigger name is not resolved
    enable_electrum(&mm_bob, "MORTY", vec![
        "electrum3.cipig.net:10018",
        "electrum2.cipig.net:10018",
        "electrum1.cipig.net:10018",
        "random-electrum-domain-name1.net:60017",
        "random-electrum-domain-name2.net:60017",
    ]);
}
