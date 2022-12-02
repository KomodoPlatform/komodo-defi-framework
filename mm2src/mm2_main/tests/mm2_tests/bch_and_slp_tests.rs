use common::executor::Timer;
use common::{block_on, log, now_ms};
use http::StatusCode;
use itertools::Itertools;
use mm2_test_helpers::for_tests::{enable_bch_with_tokens, enable_slp, get_passphrase, my_tx_history_v2, sign_message,
                                  tbch_for_slp_conf, tbch_usdf_conf, verify_message, MarketMakerIt, Mm2TestConf,
                                  UtxoRpcMode, ETH_DEV_NODES};
use mm2_test_helpers::structs::{EnableBchWithTokensResponse, RpcV2Response, SignatureResponse, StandardHistoryV2Res,
                                UtxoFeeDetails, VerificationResponse};
use serde_json::{self as json, json, Value as Json};
use std::env;
use std::str::FromStr;
use std::thread;
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
const T_BCH_ELECTRUMS: &[&str] = &[
    "electroncash.de:50003",
    "tbch.loping.net:60001",
    "blackie.c3-soft.com:60001",
    "bch0.kister.net:51001",
    "testnet.imaginary.cash:50001",
];

#[cfg(target_arch = "wasm32")]
const T_BCH_ELECTRUMS: &[&str] = &[
    "electroncash.de:60003",
    "electroncash.de:60004",
    "blackie.c3-soft.com:60004",
];

const BIP39_PASSPHRASE: &str = "tank abandon bind salon remove wisdom net size aspect direct source fossil";

fn t_bch_electrums_legacy_json() -> Vec<Json> { T_BCH_ELECTRUMS.into_iter().map(|url| json!({ "url": url })).collect() }

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_cashaddresses() {
    let coins = json!([
        {"coin":"BCH","pubtype":0,"p2shtype":5,"mm2":1,"fork_id": "0x40","protocol":{"type":"UTXO"},
         "address_format":{"format":"cashaddress","network":"bchtest"}},
    ]);

    let mm = MarketMakerIt::start(
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
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm.mm_dump();
    log!("log path: {}", mm.log_path.display());

    let electrum = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "BCH",
        "servers": t_bch_electrums_legacy_json(),
        "mm2": 1,
    })))
    .unwrap();

    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with {} {}",
        electrum.0,
        electrum.1
    );
    let electrum: Json = json::from_str(&electrum.1).unwrap();
    log!("{:?}", electrum);

    // make withdraw from cashaddress to cashaddress
    let withdraw = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": "BCH",
        "to": "bchtest:qr39na5d25wdeecgw3euh9fkd4ygvd4pnsury96597",
        "amount": 0.00001,
    })))
    .unwrap();

    assert!(withdraw.0.is_success(), "BCH withdraw: {}", withdraw.1);
    let withdraw_json: Json = json::from_str(&withdraw.1).unwrap();
    log!("{}", withdraw_json);

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
    let send_tx = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "send_raw_transaction",
        "coin": "BCH",
        "tx_hex": withdraw_json["tx_hex"],
    })))
    .unwrap();
    assert!(send_tx.0.is_success(), "BCH send_raw_transaction: {}", send_tx.1);
    log!("{}", send_tx.1);

    // Wait 5 seconds to avoid double spending
    thread::sleep(Duration::from_secs(5));

    // make withdraw from cashaddress to legacy
    let withdraw = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": "BCH",
        "to": "1WxswvLF2HdaDr4k77e92VjaXuPQA8Uji",
        "amount": 0.00001,
    })))
    .unwrap();

    assert!(withdraw.0.is_success(), "BCH withdraw: {}", withdraw.1);
    let withdraw_json: Json = json::from_str(&withdraw.1).unwrap();
    log!("{}", withdraw_json);

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
    assert_eq!(to, vec!["1WxswvLF2HdaDr4k77e92VjaXuPQA8Uji"]);

    // send the transaction
    let send_tx = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "send_raw_transaction",
        "coin": "BCH",
        "tx_hex": withdraw_json["tx_hex"],
    })))
    .unwrap();
    assert!(send_tx.0.is_success(), "BCH send_raw_transaction: {}", send_tx.1);
    log!("{}", send_tx.1);

    // Wait 5 seconds to avoid double spending
    thread::sleep(Duration::from_secs(5));

    //Disable BCH to enable in Legacy Mode
    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "disable_coin",
        "coin": "BCH",
    })))
    .unwrap();
    assert_eq!(rc.0, StatusCode::OK, "RPC «disable_coin» failed with status «{}»", rc.0);

    let electrum = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "BCH",
        "servers": t_bch_electrums_legacy_json(),
        "address_format":{"format":"standard"},
        "mm2": 1,
    })))
    .unwrap();

    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with {} {}",
        electrum.0,
        electrum.1
    );
    let electrum: Json = json::from_str(&electrum.1).unwrap();
    log!("{:?}", electrum);

    // make withdraw from Legacy to Cashaddress
    let withdraw = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": "BCH",
        "to": "bchtest:qr39na5d25wdeecgw3euh9fkd4ygvd4pnsury96597",
        "amount": 0.00001,
    })))
    .unwrap();

    assert!(withdraw.0.is_success(), "BCH withdraw: {}", withdraw.1);
    let withdraw_json: Json = json::from_str(&withdraw.1).unwrap();
    log!("{}", withdraw_json);

    // check "from" addresses
    let from: Vec<&str> = withdraw_json["from"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(from, vec!["12Tz6nWqA7e5tV7m6d1EzMkNs6MQVW4UMw"]);

    // check "to" addresses
    let to: Vec<&str> = withdraw_json["to"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(to, vec!["bchtest:qr39na5d25wdeecgw3euh9fkd4ygvd4pnsury96597"]);

    // send the transaction
    let send_tx = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "send_raw_transaction",
        "coin": "BCH",
        "tx_hex": withdraw_json["tx_hex"],
    })))
    .unwrap();
    assert!(send_tx.0.is_success(), "BCH send_raw_transaction: {}", send_tx.1);
    log!("{}", send_tx.1);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_to_different_cashaddress_network_should_fail() {
    let coins = json!([
        {"coin":"BCH","pubtype":0,"p2shtype":5,"mm2":1,"fork_id": "0x40","protocol":{"type":"UTXO"},
         "address_format":{"format":"cashaddress","network":"bchtest"}},
    ]);

    let mm = MarketMakerIt::start(
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
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm.mm_dump();
    log!("log path: {}", mm.log_path.display());

    let electrum = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "BCH",
        "servers": t_bch_electrums_legacy_json(),
        "mm2": 1,
    })))
    .unwrap();

    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with {} {}",
        electrum.0,
        electrum.1
    );
    let electrum: Json = json::from_str(&electrum.1).unwrap();
    log!("{:?}", electrum);

    // make withdraw to from bchtest to bitcoincash should fail
    let withdraw = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": "BCH",
        "to": "bitcoincash:qqyf96yqdrpa8f6pkf9f00ap068m5tgvly28qsfq9p",
        "amount": 0.00001,
    })))
    .unwrap();

    assert!(withdraw.0.is_server_error(), "BCH withdraw: {}", withdraw.1);
    log!("{:?}", withdraw.1);

    block_on(mm.stop()).unwrap();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_common_cashaddresses() {
    let coins = json!([
        {"coin":"BCH","pubtype":0,"p2shtype":5,"mm2":1,"protocol":{"type":"UTXO"},
         "address_format":{"format":"cashaddress","network":"bchtest"}},
    ]);

    let mm = MarketMakerIt::start(
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
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm.mm_dump();
    log!("log path: {}", mm.log_path.display());

    // Enable BCH electrum client with tx_history loop.
    // Enable RICK electrum client with tx_history loop.
    let electrum = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "BCH",
        "servers": t_bch_electrums_legacy_json(),
        "mm2": 1,
    })))
    .unwrap();

    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with {} {}",
        electrum.0,
        electrum.1
    );
    let electrum: Json = json::from_str(&electrum.1).unwrap();
    log!("{:?}", electrum);

    assert_eq!(
        electrum["address"].as_str().unwrap(),
        "bchtest:qze8g4gx3z428jjcxzpycpxl7ke7d947gca2a7n2la"
    );

    // check my_balance
    let rc = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "my_balance",
        "coin": "BCH",
    })))
    .unwrap();
    assert_eq!(rc.0, StatusCode::OK, "RPC «my_balance» failed with status «{}»", rc.0);
    let json: Json = json::from_str(&rc.1).unwrap();
    let my_balance_address = json["address"].as_str().unwrap();
    assert_eq!(my_balance_address, "bchtest:qze8g4gx3z428jjcxzpycpxl7ke7d947gca2a7n2la");

    // check get_enabled_coins
    let rc = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "get_enabled_coins",
    })))
    .unwrap();
    assert_eq!(
        rc.0,
        StatusCode::OK,
        "RPC «get_enabled_coins» failed with status «{}»",
        rc.0
    );
    let json: Json = json::from_str(&rc.1).unwrap();

    let obj = &json["result"].as_array().unwrap()[0];
    assert_eq!(obj["ticker"].as_str().unwrap(), "BCH");
    assert_eq!(
        obj["address"].as_str().unwrap(),
        "bchtest:qze8g4gx3z428jjcxzpycpxl7ke7d947gca2a7n2la"
    );
}

async fn wait_till_history_has_records(
    mm: &MarketMakerIt,
    expected_len: usize,
    for_coin: &str,
    paging: Option<common::PagingOptionsEnum<String>>,
    timeout_s: u64,
) -> StandardHistoryV2Res {
    let started_at = now_ms() / 1000;
    let wait_until = started_at + timeout_s;
    loop {
        let history_json = my_tx_history_v2(mm, for_coin, expected_len, paging.clone()).await;
        let history: RpcV2Response<StandardHistoryV2Res> = json::from_value(history_json).unwrap();
        if history.result.transactions.len() >= expected_len {
            break history.result;
        }

        let now = now_ms() / 1000;
        if wait_until < now {
            panic!(
                "Waited too long until {} for TX history loads {} transactions",
                wait_until, expected_len
            );
        }

        Timer::sleep(1.).await;
    }
}

async fn test_bch_and_slp_testnet_history_impl() {
    const PASSPHRASE: &str = "BCH SLP test";
    const TIMEOUT_S: u64 = 45;

    let coins = json!([
        {"coin":"tBCH","pubtype":0,"p2shtype":5,"mm2":1,"protocol":{"type":"BCH","protocol_data":{"slp_prefix":"slptest"}},
         "address_format":{"format":"cashaddress","network":"bchtest"}},
        {"coin":"USDF","protocol":{"type":"SLPTOKEN","protocol_data":{"decimals":4,"token_id":"bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7","platform":"tBCH","required_confirmations":1}}}
    ]);

    let conf = Mm2TestConf::seednode(PASSPHRASE, &coins);
    let mm = MarketMakerIt::start_async(conf.conf, conf.rpc_password, None)
        .await
        .unwrap();

    #[cfg(not(target_arch = "wasm32"))]
    {
        let (_dump_log, _dump_dashboard) = mm.mm_dump();
        log!("log path: {}", mm.log_path.display());
    }

    let rpc_mode = UtxoRpcMode::electrum(T_BCH_ELECTRUMS);
    let tx_history = true;
    let enable_bch = enable_bch_with_tokens(&mm, "tBCH", &[], rpc_mode, tx_history).await;
    log!("enable_bch: {:?}", enable_bch);
    let history = wait_till_history_has_records(&mm, 4, "tBCH", None, TIMEOUT_S).await;
    log!("bch history: {:?}", history);

    let expected_internal_ids = vec![
        "eefb21290909cb7f2864ef066836bd98f8963731576f65a8c0ff590c3e91d439",
        "6686ee013620d31ba645b27d581fed85437ce00f46b595a576718afac4dd5b69",
        "c07836722bbdfa2404d8fe0ea56700d02e2012cb9dc100ccaf1138f334a759ce",
        "091877294268b2b1734255067146f15c3ac5e6199e72cd4f68a8d9dec32bb0c0",
    ];

    let actual_ids: Vec<_> = history
        .transactions
        .iter()
        .map(|tx| tx.tx.internal_id.as_str())
        .collect();

    assert_eq!(expected_internal_ids, actual_ids);

    let enable_usdf = enable_slp(&mm, "USDF").await;
    log!("enable_usdf: {:?}", enable_usdf);

    let paging =
        common::PagingOptionsEnum::FromId("433b641bc89e1b59c22717918583c60ec98421805c8e85b064691705d9aeb970".into());
    let slp_history = wait_till_history_has_records(&mm, 4, "USDF", Some(paging), TIMEOUT_S).await;

    log!("slp history: {:?}", slp_history);

    let expected_slp_ids = vec![
        "babe9bd0dc1495dff0920da14a76311b744daadc9d01314f8bd4e2438c6b183b",
        "1c1e68357cf5a6dacb53881f13aa5d2048fe0d0fab24b76c9ec48f53884bed97",
        "cd6ec10b0cd9747ddc66ac5c97c2d7b493e8cea191bc2d847b3498719d4bd989",
        "b0035434a1e7be5af2ed991ee2a21a90b271c5852a684a0b7d315c5a770d1b1c",
    ];

    let actual_slp_ids: Vec<_> = slp_history
        .transactions
        .iter()
        .map(|tx| tx.tx.internal_id.as_str())
        .collect();

    assert_eq!(expected_slp_ids, actual_slp_ids);

    for tx in slp_history.transactions {
        assert_eq!("USDF", tx.tx.coin);

        let fee_details: UtxoFeeDetails = json::from_value(tx.tx.fee_details).unwrap();
        assert_eq!(fee_details.coin, Some("tBCH".to_owned()));
    }

    #[cfg(target_arch = "wasm32")]
    {
        /// 1 second.
        const STOP_TIMEOUT_MS: u64 = 1000;

        mm.stop_and_wait_for_ctx_is_dropped(STOP_TIMEOUT_MS).await.unwrap();
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_bch_and_slp_testnet_history() { block_on(test_bch_and_slp_testnet_history_impl()); }

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen_test]
async fn test_bch_and_slp_testnet_history() {
    common::log::wasm_log::register_wasm_log();
    test_bch_and_slp_testnet_history_impl().await;
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_sign_verify_message_bch() {
    let seed = "spice describe gravity federal blast come thank unfair canal monkey style afraid";

    let coins = json!([
        {"coin":"BCH","pubtype":0,"p2shtype":5,"mm2":1,"fork_id": "0x40","sign_message_prefix": "Bitcoin Signed Message:\n","protocol":{"type":"UTXO"},
         "address_format":{"format":"cashaddress","network":"bitcoincash"}},
    ]);

    let mm = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": seed.to_string(),
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm.mm_dump();
    log!("log path: {}", mm.log_path.display());

    let electrum = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "electrum",
        "coin": "BCH",
        "servers": t_bch_electrums_legacy_json(),
        "mm2": 1,
    })))
    .unwrap();

    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with {} {}",
        electrum.0,
        electrum.1
    );
    let electrum: Json = json::from_str(&electrum.1).unwrap();
    log!("{:?}", electrum);

    let response = block_on(sign_message(&mm, "BCH"));
    let response: RpcV2Response<SignatureResponse> = json::from_value(response).unwrap();
    let response = response.result;

    assert_eq!(
        response.signature,
        "HzNH58Xd+orz5jKewdH88/cGOVmsK6tTDEsJSag3pmVWMdjlw7gB6N6cNgRtWaeJIadsqQmhwv8DHWIjqGzOoE8="
    );

    let response = block_on(verify_message(
        &mm,
        "BCH",
        "HzNH58Xd+orz5jKewdH88/cGOVmsK6tTDEsJSag3pmVWMdjlw7gB6N6cNgRtWaeJIadsqQmhwv8DHWIjqGzOoE8=",
        "bitcoincash:qqz64df5y9n0sk2t4ut60kd77h2kw3pnyursltctnw",
    ));
    let response: RpcV2Response<VerificationResponse> = json::from_value(response).unwrap();
    let response = response.result;

    assert!(response.is_valid);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_sign_verify_message_slp() {
    let seed = "spice describe gravity federal blast come thank unfair canal monkey style afraid";

    let coins = json!([
        {"coin":"tBCH","pubtype":0,"p2shtype":5,"mm2":1,"sign_message_prefix": "Bitcoin Signed Message:\n","protocol":{"type":"BCH","protocol_data":{"slp_prefix":"slptest"}},
         "address_format":{"format":"cashaddress","network":"bchtest"}},
        {"coin":"USDF","protocol":{"type":"SLPTOKEN","protocol_data":{"decimals":4,"token_id":"bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7","platform":"tBCH","required_confirmations":1}}}
    ]);

    let mm = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": seed.to_string(),
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm.mm_dump();
    log!("log path: {}", mm.log_path.display());

    let rpc_mode = UtxoRpcMode::electrum(T_BCH_ELECTRUMS);
    let enable_bch = block_on(enable_bch_with_tokens(&mm, "tBCH", &[], rpc_mode, false));
    log!("enable_bch: {:?}", enable_bch);

    let enable_usdf = block_on(enable_slp(&mm, "USDF"));
    log!("enable_usdf: {:?}", enable_usdf);

    let response = block_on(sign_message(&mm, "USDF"));
    let response: RpcV2Response<SignatureResponse> = json::from_value(response).unwrap();
    let response = response.result;

    assert_eq!(
        response.signature,
        "HzNH58Xd+orz5jKewdH88/cGOVmsK6tTDEsJSag3pmVWMdjlw7gB6N6cNgRtWaeJIadsqQmhwv8DHWIjqGzOoE8="
    );

    let response = block_on(verify_message(
        &mm,
        "USDF",
        "HzNH58Xd+orz5jKewdH88/cGOVmsK6tTDEsJSag3pmVWMdjlw7gB6N6cNgRtWaeJIadsqQmhwv8DHWIjqGzOoE8=",
        "slptest:qqz64df5y9n0sk2t4ut60kd77h2kw3pnyuukuhqtx0",
    ));
    let response: RpcV2Response<VerificationResponse> = json::from_value(response).unwrap();
    let response = response.result;

    assert!(response.is_valid);
}

/// Tested via [Electron-Cash-SLP](https://github.com/simpleledger/Electron-Cash-SLP).
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_bch_and_slp_with_hd_account_id() {
    const TX_HISTORY: bool = false;

    let coins = json!([tbch_for_slp_conf(), tbch_usdf_conf()]);

    // HD account 0

    let hd_account_id = 0;
    let conf_0 = Mm2TestConf::seednode_with_hd_account(BIP39_PASSPHRASE, hd_account_id, &coins);
    let mm_hd_0 = MarketMakerIt::start(conf_0.conf, conf_0.rpc_password, None).unwrap();

    let rpc_mode = UtxoRpcMode::electrum(T_BCH_ELECTRUMS);
    let activation_result = block_on(enable_bch_with_tokens(
        &mm_hd_0,
        "tBCH",
        &["USDF"],
        rpc_mode,
        TX_HISTORY,
    ));

    let activation_result: RpcV2Response<EnableBchWithTokensResponse> = json::from_value(activation_result).unwrap();
    let (bch_addr, _) = activation_result
        .result
        .bch_addresses_infos
        .into_iter()
        .exactly_one()
        .unwrap();
    assert_eq!(bch_addr, "bchtest:qpylzql7gzh6yctm7uslsz5qufl44gk2tsj8c9pjw0");

    let (slp_addr, _) = activation_result
        .result
        .slp_addresses_infos
        .into_iter()
        .exactly_one()
        .unwrap();
    assert_eq!(slp_addr, "slptest:qpylzql7gzh6yctm7uslsz5qufl44gk2tsfnl7m9uj");

    // HD account 1

    let hd_account_id = 1;
    let conf_1 = Mm2TestConf::seednode_with_hd_account(BIP39_PASSPHRASE, hd_account_id, &coins);
    let mm_hd_1 = MarketMakerIt::start(conf_1.conf, conf_1.rpc_password, None).unwrap();

    let rpc_mode = UtxoRpcMode::electrum(T_BCH_ELECTRUMS);
    let activation_result = block_on(enable_bch_with_tokens(
        &mm_hd_1,
        "tBCH",
        &["USDF"],
        rpc_mode,
        TX_HISTORY,
    ));

    let activation_result: RpcV2Response<EnableBchWithTokensResponse> = json::from_value(activation_result).unwrap();
    let (bch_addr, _) = activation_result
        .result
        .bch_addresses_infos
        .into_iter()
        .exactly_one()
        .unwrap();
    assert_eq!(bch_addr, "bchtest:qpyhwc7shd5hlul8zg0snmaptaa9q9yc4q7g9khpkj");

    let (slp_addr, _) = activation_result
        .result
        .slp_addresses_infos
        .into_iter()
        .exactly_one()
        .unwrap();
    assert_eq!(slp_addr, "slptest:qpyhwc7shd5hlul8zg0snmaptaa9q9yc4q9uzddky0");
}

async fn enable_eth_with_tokens(mm: &MarketMakerIt, platform_coin: &str, tokens: &[&str], nodes: &[&str]) -> Json {
    let erc20_tokens_requests: Vec<_> = tokens.iter().map(|ticker| json!({ "ticker": ticker })).collect();
    let nodes: Vec<_> = nodes.iter().map(|url| json!({ "url": url })).collect();

    let enable = mm
        .rpc(&json!({
        "userpass": mm.userpass,
        "method": "enable_eth_with_tokens",
        "mmrpc": "2.0",
        "params": {
        "ticker": platform_coin,
              "gas_station_url":"https://ethgasstation.info/json/ethgasAPI.json",
              "swap_contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E",
              "fallback_swap_contract":"0x8500AFc0bc5214728082163326C2FF0C73f4a871",
              "nodes": nodes,
              "tx_history": true,
              "erc20_tokens_requests": erc20_tokens_requests,
          }}))
        .await
        .unwrap();
    assert_eq!(
        enable.0,
        StatusCode::OK,
        "'enable_eth_with_tokens' failed: {}",
        enable.1
    );
    json::from_str(&enable.1).unwrap()
}

#[test]
fn test_disable_eth_coin_with_token() {
    let passphrase = get_passphrase(&".env.client", "BOB_PASSPHRASE").unwrap();
    let coins = json! ([
       {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"},"rpcport":80,"mm2":1},
           {"coin":"JST","name":"jst","rpcport":80,"mm2":1,"protocol":{"type":"ERC20","protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);
    let conf = Mm2TestConf::seednode(&passphrase, &coins);
    let mm = block_on(MarketMakerIt::start_async(conf.conf, conf.rpc_password, None)).unwrap();
    block_on(enable_eth_with_tokens(&mm, "ETH", &["JST"], ETH_DEV_NODES));

    // Create setprice order
    let req = json!({
        "userpass": mm.userpass,
        "method": "buy",
        "base": "ETH",
        "rel": "JST",
        "price": 1,
        "volume": 0.1,
        "base_confs": 5,
        "base_nota": true,
        "rel_confs": 4,
        "rel_nota": false,
    });
    let make_test_order = block_on(mm.rpc(&req)).unwrap();
    assert_eq!(make_test_order.0, StatusCode::OK);

    let order_uuid = Json::from_str(&*make_test_order.1).unwrap();
    let order_uuid = order_uuid.get("result").unwrap().get("uuid").unwrap().as_str().unwrap();

    // Disable platform coin ETH
    let disable = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "disable_coin",
        "coin": "ETH",
    })))
    .unwrap();
    assert_eq!(disable.0, StatusCode::OK);

    // We expected make_test_order to be cancelled
    let cancelled_order_uuid = Json::from_str(&*disable.1).unwrap();
    let cancelled_order_uuid = cancelled_order_uuid
        .get("result")
        .unwrap()
        .get("cancelled_orders")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .as_str()
        .unwrap();
    assert_eq!(cancelled_order_uuid, order_uuid);

    // We also expected token, "JST" to be deactivated
    let my_balance = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "my_balance",
        "coin": "JST",
    })))
    .unwrap();
    assert_eq!(my_balance.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(my_balance.1.contains("No such coin: JST"));
}
