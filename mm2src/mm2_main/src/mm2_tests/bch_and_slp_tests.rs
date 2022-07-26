use super::*;
use mm2_test_helpers::for_tests::{enable_bch_with_tokens, enable_slp, my_tx_history_v2, sign_message, verify_message,
                                  UtxoRpcMode};

const T_BCH_ELECTRUMS: &[&str] = &[
    "electroncash.de:50003",
    "tbch.loping.net:60001",
    "blackie.c3-soft.com:60001",
    "bch0.kister.net:51001",
    "testnet.imaginary.cash:50001",
];

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
        local_start!("bob"),
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
        local_start!("bob"),
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
        local_start!("bob"),
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
) -> StandardHistoryV2Res {
    loop {
        let history_json = my_tx_history_v2(mm, for_coin, expected_len, paging.clone()).await;
        let history: RpcV2Response<StandardHistoryV2Res> = json::from_value(history_json).unwrap();
        if history.result.transactions.len() >= expected_len {
            break history.result;
        }
        Timer::sleep(1.).await;
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_bch_and_slp_testnet_history() {
    let coins = json!([
        {"coin":"tBCH","pubtype":0,"p2shtype":5,"mm2":1,"protocol":{"type":"BCH","protocol_data":{"slp_prefix":"slptest"}},
         "address_format":{"format":"cashaddress","network":"bchtest"}},
        {"coin":"USDF","protocol":{"type":"SLPTOKEN","protocol_data":{"decimals":4,"token_id":"bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7","platform":"tBCH","required_confirmations":1}}}
    ]);

    let mm = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": "BCH SLP test",
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        local_start!("bob"),
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm.mm_dump();
    log!("log path: {}", mm.log_path.display());

    let rpc_mode = UtxoRpcMode::electrum(T_BCH_ELECTRUMS);
    let tx_history = true;
    let enable_bch = block_on(enable_bch_with_tokens(&mm, "tBCH", &[], rpc_mode, tx_history));
    log!("enable_bch: {:?}", enable_bch);
    let history = block_on(wait_till_history_has_records(&mm, 4, "tBCH", None));
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

    let enable_usdf = block_on(enable_slp(&mm, "USDF"));
    log!("enable_usdf: {:?}", enable_usdf);

    let paging =
        common::PagingOptionsEnum::FromId("433b641bc89e1b59c22717918583c60ec98421805c8e85b064691705d9aeb970".into());
    let slp_history = block_on(wait_till_history_has_records(&mm, 4, "USDF", Some(paging)));

    log!("slp history: {:?}", slp_history);

    let expected_slp_ids = vec![
        "cd6ec10b0cd9747ddc66ac5c97c2d7b493e8cea191bc2d847b3498719d4bd989",
        "1c1e68357cf5a6dacb53881f13aa5d2048fe0d0fab24b76c9ec48f53884bed97",
        "c4304b5ef4f1b88ed4939534a8ca9eca79f592939233174ae08002e8454e3f06",
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
        local_start!("bob"),
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
        local_start!("bob"),
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
