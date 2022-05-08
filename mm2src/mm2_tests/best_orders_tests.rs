use super::*;
use common::for_tests::best_orders_v2;

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_best_orders() {
    let bob_passphrase = get_passphrase(&".env.seed", "BOB_PASSPHRASE").unwrap();

    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"},"rpcport":80},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20", "protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    // start bob and immediately place the orders
    let mut mm_bob = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        local_start!("bob"),
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!({"Bob log path: {}", mm_bob.log_path.display()});

    // Enable coins on Bob side. Print the replies in case we need the "address".
    let bob_coins = block_on(enable_coins_eth_electrum(&mm_bob, &["http://195.201.0.6:8565"]));
    log!({ "enable_coins (bob): {:?}", bob_coins });
    // issue sell request on Bob side by setting base/rel price
    log!("Issue bob sell requests");

    let bob_orders = [
        // (base, rel, price, volume, min_volume)
        ("RICK", "MORTY", "0.9", "0.9", None),
        ("RICK", "MORTY", "0.8", "0.9", None),
        ("RICK", "MORTY", "0.7", "0.9", Some("0.9")),
        ("RICK", "ETH", "0.8", "0.9", None),
        ("MORTY", "RICK", "0.8", "0.9", None),
        ("MORTY", "RICK", "0.9", "0.9", None),
        ("ETH", "RICK", "0.8", "0.9", None),
        ("MORTY", "ETH", "0.8", "0.8", None),
        ("MORTY", "ETH", "0.7", "0.8", Some("0.8")),
    ];
    for (base, rel, price, volume, min_volume) in bob_orders.iter() {
        let rc = block_on(mm_bob.rpc(&json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": base,
            "rel": rel,
            "price": price,
            "volume": volume,
            "min_volume": min_volume.unwrap_or("0.00777"),
            "cancel_previous": false,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    }

    let mm_alice = MarketMakerIt::start(
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
        local_start!("alice"),
    )
    .unwrap();

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!({ "Alice log path: {}", mm_alice.log_path.display() });

    block_on(mm_bob.wait_for_log(22., |log| {
        log.contains("DEBUG Handling IncludedTorelaysMesh message for peer")
    }))
    .unwrap();

    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "best_orders",
        "coin": "RICK",
        "action": "buy",
        "volume": "0.1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!best_orders: {}", rc.1);
    let response: BestOrdersResponse = json::from_str(&rc.1).unwrap();
    let best_morty_orders = response.result.get("MORTY").unwrap();
    assert_eq!(1, best_morty_orders.len());
    let expected_price: BigDecimal = "0.8".parse().unwrap();
    assert_eq!(expected_price, best_morty_orders[0].price);

    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "best_orders",
        "coin": "RICK",
        "action": "buy",
        "volume": "1.7",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!best_orders: {}", rc.1);
    let response: BestOrdersResponse = json::from_str(&rc.1).unwrap();
    // MORTY
    let best_morty_orders = response.result.get("MORTY").unwrap();
    let expected_price: BigDecimal = "0.7".parse().unwrap();
    let bob_morty_addr = addr_from_enable(&bob_coins, "MORTY");
    assert_eq!(expected_price, best_morty_orders[0].price);
    assert_eq!(bob_morty_addr, best_morty_orders[0].address);
    let expected_price: BigDecimal = "0.8".parse().unwrap();
    assert_eq!(expected_price, best_morty_orders[1].price);
    assert_eq!(bob_morty_addr, best_morty_orders[1].address);
    // ETH
    let expected_price: BigDecimal = "0.8".parse().unwrap();
    let best_eth_orders = response.result.get("ETH").unwrap();
    assert_eq!(expected_price, best_eth_orders[0].price);

    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "best_orders",
        "coin": "RICK",
        "action": "sell",
        "volume": "0.1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!best_orders: {}", rc.1);
    let response: BestOrdersResponse = json::from_str(&rc.1).unwrap();

    let expected_price: BigDecimal = "1.25".parse().unwrap();

    let best_morty_orders = response.result.get("MORTY").unwrap();
    assert_eq!(expected_price, best_morty_orders[0].price);
    assert_eq!(1, best_morty_orders.len());

    let best_eth_orders = response.result.get("ETH").unwrap();
    assert_eq!(expected_price, best_eth_orders[0].price);

    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "best_orders",
        "coin": "ETH",
        "action": "sell",
        "volume": "0.1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!best_orders: {}", rc.1);
    let response: BestOrdersResponse = json::from_str(&rc.1).unwrap();

    let expected_price: BigDecimal = "1.25".parse().unwrap();

    let best_morty_orders = response.result.get("MORTY").unwrap();
    assert_eq!(expected_price, best_morty_orders[0].price);
    assert_eq!("MORTY", best_morty_orders[0].coin);
    assert_eq!(1, best_morty_orders.len());

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_best_orders_duplicates_after_update() {
    let eve_passphrase = get_passphrase(&".env.seed", "BOB_PASSPHRASE").unwrap();

    let coins = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}}
    ]);

    // start bob as a seednode
    let mut mm_bob = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": "bob",
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        local_start!("bob"),
    )
    .unwrap();

    // start eve and immediately place the order
    let mm_eve = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": eve_passphrase,
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": [fomat!((mm_bob.ip))],
        }),
        "pass".into(),
        local_start!("bob"),
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!({"Bob log path: {}", mm_bob.log_path.display()});

    // Enable coins on Eve side. Print the replies in case we need the "address".
    let eve_coins = block_on(enable_coins_rick_morty_electrum(&mm_eve));
    log!({ "enable_coins (eve): {:?}", eve_coins });
    // issue sell request on Eve side by setting base/rel price
    log!("Issue eve sell request");

    let rc = block_on(mm_eve.rpc(&json! ({
        "userpass": mm_eve.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "MORTY",
        "price": "1",
        "volume": "1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let eve_order: SetPriceResponse = json::from_str(&rc.1).unwrap();

    let mm_alice = MarketMakerIt::start(
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
        local_start!("alice"),
    )
    .unwrap();

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!({ "Alice log path: {}", mm_alice.log_path.display() });

    block_on(mm_bob.wait_for_log(22., |log| {
        log.contains("DEBUG Handling IncludedTorelaysMesh message for peer")
    }))
    .unwrap();

    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "best_orders",
        "coin": "RICK",
        "action": "buy",
        "volume": "0.1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!best_orders: {}", rc.1);
    let response: BestOrdersResponse = json::from_str(&rc.1).unwrap();
    let best_morty_orders = response.result.get("MORTY").unwrap();
    assert_eq!(1, best_morty_orders.len());
    let expected_price: BigDecimal = "1".parse().unwrap();
    assert_eq!(expected_price, best_morty_orders[0].price);

    for _ in 0..5 {
        let rc = block_on(mm_eve.rpc(&json!({
            "userpass": mm_eve.userpass,
            "method": "update_maker_order",
            "uuid": eve_order.result.uuid,
            "new_price": "1.1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
        thread::sleep(Duration::from_secs(1));
    }

    for _ in 0..5 {
        let rc = block_on(mm_eve.rpc(&json!({
            "userpass": mm_eve.userpass,
            "method": "update_maker_order",
            "uuid": eve_order.result.uuid,
            "new_price": "1.2",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
        thread::sleep(Duration::from_secs(1));
    }

    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "best_orders",
        "coin": "RICK",
        "action": "buy",
        "volume": "500",
    })))
    .unwrap();

    assert!(rc.0.is_success(), "!best_orders: {}", rc.1);
    let response: BestOrdersResponse = json::from_str(&rc.1).unwrap();
    let best_morty_orders = response.result.get("MORTY").unwrap();
    assert_eq!(1, best_morty_orders.len());
    let expected_price: BigDecimal = "1.2".parse().unwrap();
    assert_eq!(expected_price, best_morty_orders[0].price);

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
    block_on(mm_eve.stop()).unwrap();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_best_orders_filter_response() {
    let bob_passphrase = get_passphrase(&".env.seed", "BOB_PASSPHRASE").unwrap();

    let bob_coins_config = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"},"rpcport":80},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20", "protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    // alice defined MORTY as "wallet_only" in config
    let alice_coins_config = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"wallet_only": true,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"},"rpcport":80},
        {"coin":"JST","name":"jst","protocol":{"type":"ERC20", "protocol_data":{"platform":"ETH","contract_address":"0x2b294F029Fde858b2c62184e8390591755521d8E"}}}
    ]);

    // start bob and immediately place the orders
    let mut mm_bob = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": bob_passphrase,
            "coins": bob_coins_config,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        local_start!("bob"),
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!({"Bob log path: {}", mm_bob.log_path.display()});

    // Enable coins on Bob side. Print the replies in case we need the "address".
    let bob_coins = block_on(enable_coins_eth_electrum(&mm_bob, &["http://195.201.0.6:8565"]));
    log!({ "enable_coins (bob): {:?}", bob_coins });
    // issue sell request on Bob side by setting base/rel price
    log!("Issue bob sell requests");

    let bob_orders = [
        // (base, rel, price, volume, min_volume)
        ("RICK", "MORTY", "0.9", "0.9", None),
        ("RICK", "MORTY", "0.8", "0.9", None),
        ("RICK", "MORTY", "0.7", "0.9", Some("0.9")),
        ("RICK", "ETH", "0.8", "0.9", None),
        ("MORTY", "RICK", "0.8", "0.9", None),
        ("MORTY", "RICK", "0.9", "0.9", None),
        ("ETH", "RICK", "0.8", "0.9", None),
        ("MORTY", "ETH", "0.8", "0.8", None),
        ("MORTY", "ETH", "0.7", "0.8", Some("0.8")),
    ];
    for (base, rel, price, volume, min_volume) in bob_orders.iter() {
        let rc = block_on(mm_bob.rpc(&json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": base,
            "rel": rel,
            "price": price,
            "volume": volume,
            "min_volume": min_volume.unwrap_or("0.00777"),
            "cancel_previous": false,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    }

    let mm_alice = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": "alice passphrase",
            "coins": alice_coins_config,
            "seednodes": [fomat!((mm_bob.ip))],
            "rpc_password": "pass",
        }),
        "pass".into(),
        local_start!("alice"),
    )
    .unwrap();

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!({ "Alice log path: {}", mm_alice.log_path.display() });

    block_on(mm_bob.wait_for_log(22., |log| {
        log.contains("DEBUG Handling IncludedTorelaysMesh message for peer")
    }))
    .unwrap();

    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "best_orders",
        "coin": "RICK",
        "action": "buy",
        "volume": "0.1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!best_orders: {}", rc.1);
    let response: BestOrdersResponse = json::from_str(&rc.1).unwrap();
    let empty_vec = Vec::new();
    let best_morty_orders = response.result.get("MORTY").unwrap_or(&empty_vec);
    assert_eq!(0, best_morty_orders.len());
    let best_eth_orders = response.result.get("ETH").unwrap();
    assert_eq!(1, best_eth_orders.len());

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_best_orders_address_and_confirmations() {
    let bob_passphrase = get_passphrase(&".env.seed", "BOB_PASSPHRASE").unwrap();

    let bob_coins_config = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"required_confirmations":10,"requires_notarization":true,"protocol":{"type":"UTXO"}},
        {"coin":"tBTC","name":"tbitcoin","fname":"tBitcoin","rpcport":18332,"pubtype":111,"p2shtype":196,"wiftype":239,"segwit":true,"bech32_hrp":"tb","txfee":0,"estimate_fee_mode":"ECONOMICAL","mm2":1,"required_confirmations":5,"requires_notarization":false,"protocol":{"type":"UTXO"},"address_format":{"format":"segwit"}}
    ]);

    let alice_coins_config = json!([
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"tBTC","name":"tbitcoin","fname":"tBitcoin","rpcport":18332,"pubtype":111,"p2shtype":196,"wiftype":239,"segwit":true,"bech32_hrp":"tb","txfee":0,"estimate_fee_mode":"ECONOMICAL","mm2":1,"required_confirmations":0,"protocol":{"type":"UTXO"}}
    ]);

    let mut mm_bob = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": bob_passphrase,
            "coins": bob_coins_config,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        local_start!("bob"),
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!({"Bob log path: {}", mm_bob.log_path.display()});

    // Enable coins on Bob side. Print the replies in case we need the "address".
    let electrum = block_on(mm_bob.rpc(&json!({
        "userpass": "pass",
        "method": "electrum",
        "coin": "tBTC",
        "servers": [{"url":"electrum1.cipig.net:10068"},{"url":"electrum2.cipig.net:10068"},{"url":"electrum3.cipig.net:10068"}],
        "address_format":{"format":"segwit"},
        "mm2": 1,
    }))).unwrap();
    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with {} {}",
        electrum.0,
        electrum.1
    );
    log!({ "enable tBTC: {:?}", electrum });
    let enable_tbtc_res: EnableElectrumResponse = json::from_str(&electrum.1).unwrap();
    let tbtc_segwit_address = enable_tbtc_res.address;

    let electrum = block_on(mm_bob.rpc(&json!({
        "userpass": "pass",
        "method": "electrum",
        "coin": "RICK",
        "servers": [{"url":"electrum1.cipig.net:10017"},{"url":"electrum2.cipig.net:10017"},{"url":"electrum3.cipig.net:10017"}],
        "mm2": 1,
    }))).unwrap();
    assert_eq!(
        electrum.0,
        StatusCode::OK,
        "RPC «electrum» failed with {} {}",
        electrum.0,
        electrum.1
    );
    log!({ "enable RICK: {:?}", electrum });
    let enable_rick_res: EnableElectrumResponse = json::from_str(&electrum.1).unwrap();
    let rick_address = enable_rick_res.address;

    // issue sell request on Bob side by setting base/rel price
    log!("Issue bob sell requests");

    let bob_orders = [
        // (base, rel, price, volume, min_volume)
        ("tBTC", "RICK", "0.7", "0.0002", Some("0.00015")),
        ("RICK", "tBTC", "0.7", "0.0002", Some("0.00015")),
    ];
    for (base, rel, price, volume, min_volume) in bob_orders.iter() {
        let rc = block_on(mm_bob.rpc(&json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": base,
            "rel": rel,
            "price": price,
            "volume": volume,
            "min_volume": min_volume.unwrap_or("0.00777"),
            "cancel_previous": false,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    }

    let mm_alice = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": "alice passphrase",
            "coins": alice_coins_config,
            "seednodes": [fomat!((mm_bob.ip))],
            "rpc_password": "pass",
        }),
        "pass".into(),
        local_start!("alice"),
    )
    .unwrap();

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!({ "Alice log path: {}", mm_alice.log_path.display() });

    block_on(mm_bob.wait_for_log(22., |log| {
        log.contains("DEBUG Handling IncludedTorelaysMesh message for peer")
    }))
    .unwrap();

    // checking buy and sell best_orders against ("tBTC", "RICK", "0.7", "0.0002", Some("0.00015"))
    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "best_orders",
        "coin": "tBTC",
        "action": "buy",
        "volume": "0.0002",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!best_orders: {}", rc.1);
    let response: BestOrdersResponse = json::from_str(&rc.1).unwrap();
    let best_orders = response.result.get("RICK").unwrap();
    assert_eq!(1, best_orders.len());
    assert_eq!(best_orders[0].coin, "RICK");
    assert_eq!(best_orders[0].address, rick_address);
    assert_eq!(best_orders[0].base_confs, 5);
    assert_eq!(best_orders[0].base_nota, false);
    assert_eq!(best_orders[0].rel_confs, 10);
    assert_eq!(best_orders[0].rel_nota, true);

    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "best_orders",
        "coin": "RICK",
        "action": "sell",
        "volume": "0.0002",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!best_orders: {}", rc.1);
    let response: BestOrdersResponse = json::from_str(&rc.1).unwrap();
    let best_orders = response.result.get("tBTC").unwrap();
    assert_eq!(1, best_orders.len());
    assert_eq!(best_orders[0].coin, "tBTC");
    assert_eq!(best_orders[0].address, tbtc_segwit_address);
    assert_eq!(best_orders[0].base_confs, 10);
    assert_eq!(best_orders[0].base_nota, true);
    assert_eq!(best_orders[0].rel_confs, 5);
    assert_eq!(best_orders[0].rel_nota, false);

    // checking buy and sell best_orders against ("RICK", "tBTC", "0.7", "0.0002", Some("0.00015"))
    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "best_orders",
        "coin": "RICK",
        "action": "buy",
        "volume": "0.0002",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!best_orders: {}", rc.1);
    let response: BestOrdersResponse = json::from_str(&rc.1).unwrap();
    let best_orders = response.result.get("tBTC").unwrap();
    assert_eq!(1, best_orders.len());
    assert_eq!(best_orders[0].coin, "tBTC");
    assert_eq!(best_orders[0].address, tbtc_segwit_address);
    assert_eq!(best_orders[0].base_confs, 10);
    assert_eq!(best_orders[0].base_nota, true);
    assert_eq!(best_orders[0].rel_confs, 5);
    assert_eq!(best_orders[0].rel_nota, false);

    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "best_orders",
        "coin": "tBTC",
        "action": "sell",
        "volume": "0.0002",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!best_orders: {}", rc.1);
    let response: BestOrdersResponse = json::from_str(&rc.1).unwrap();
    let best_orders = response.result.get("RICK").unwrap();
    assert_eq!(1, best_orders.len());
    assert_eq!(best_orders[0].coin, "RICK");
    assert_eq!(best_orders[0].address, rick_address);
    assert_eq!(best_orders[0].base_confs, 5);
    assert_eq!(best_orders[0].base_nota, false);
    assert_eq!(best_orders[0].rel_confs, 10);
    assert_eq!(best_orders[0].rel_nota, true);

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[cfg(feature = "zhtlc-native-tests")]
#[test]
fn zhtlc_best_orders() {
    let bob_passphrase = get_passphrase!(".env.seed", "BOB_PASSPHRASE").unwrap();
    let alice_passphrase = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();

    let coins = json!([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {"coin":"ZOMBIE","asset":"ZOMBIE","fname":"ZOMBIE (TESTCOIN)","txversion":4,"overwintered":1,"mm2":1,"protocol":{"type":"ZHTLC"},"required_confirmations":0}
    ]);

    let mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "bob" => Some(local_start()),
            _ => None,
        },
    )
    .unwrap();

    let (_dump_log, _dump_dashboard) = mm_bob.mm_dump();
    log!({"Bob log path: {}", mm_bob.log_path.display()});

    let rmd = rmd160_from_passphrase(&bob_passphrase);
    let bob_zombie_cache_path = mm_bob.folder.join("DB").join(hex::encode(rmd)).join("ZOMBIE_CACHE.db");
    log!("bob_zombie_cache_path "(bob_zombie_cache_path.display()));
    std::fs::copy("./mm2src/coins/for_tests/ZOMBIE_CACHE.db", bob_zombie_cache_path).unwrap();

    block_on(enable_electrum_json(&mm_bob, "RICK", false, rick_electrums()));
    block_on(enable_z_coin(&mm_bob, "ZOMBIE"));

    let set_price_json = json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "ZOMBIE",
        "rel": "RICK",
        "price": 1,
        "volume": "1",
    });
    log!("Issue sell request on Bob side by setting base/rel price…");
    let rc = block_on(mm_bob.rpc(&set_price_json)).unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let bob_set_price_res: SetPriceResponse = json::from_str(&rc.1).unwrap();

    let mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9998,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": alice_passphrase,
            "coins": coins,
            "seednodes": [fomat!((mm_bob.ip))],
            "rpc_password": "pass",
        }),
        "pass".into(),
        match var("LOCAL_THREAD_MM") {
            Ok(ref e) if e == "alice" => Some(local_start()),
            _ => None,
        },
    )
    .unwrap();

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!({"Alice log path: {}", mm_alice.log_path.display()});

    let best_orders = block_on(best_orders_v2(&mm_alice, "RICK", "sell", "1"));
    let best_orders: RpcV2Response<BestOrdersV2Response> = json::from_value(best_orders).unwrap();
    let zombie_best_orders = best_orders.result.orders.get("ZOMBIE").unwrap();

    assert_eq!(1, zombie_best_orders.len());
    zombie_best_orders
        .iter()
        .find(|order| order.uuid == bob_set_price_res.result.uuid)
        .unwrap();

    let best_orders = block_on(best_orders_v2(&mm_alice, "ZOMBIE", "buy", "1"));
    let best_orders: RpcV2Response<BestOrdersV2Response> = json::from_value(best_orders).unwrap();
    let rick_best_orders = best_orders.result.orders.get("RICK").unwrap();

    assert_eq!(1, rick_best_orders.len());
    rick_best_orders
        .iter()
        .find(|order| order.uuid == bob_set_price_res.result.uuid)
        .unwrap();
}
