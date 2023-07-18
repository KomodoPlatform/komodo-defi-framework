use crate::integration_tests_common::*;
use common::{block_on, log};
use http::StatusCode;
use mm2_number::BigDecimal;
use mm2_rpc::data::legacy::CoinInitResponse;
use mm2_test_helpers::for_tests::ETH_DEV_NODES;
use mm2_test_helpers::for_tests::{best_orders_v2, best_orders_v2_by_number, eth_jst_testnet_conf, eth_testnet_conf,
                                  get_passphrase, morty_conf, rick_conf, tbtc_conf, tbtc_segwit_conf, MarketMakerIt,
                                  Mm2TestConf, RICK_ELECTRUM_ADDRS, TBTC_ELECTRUMS};
use mm2_test_helpers::structs::{BestOrdersResponse, SetPriceResponse};
use serde_json::{self as json, json};
use std::collections::BTreeSet;
use std::env::{self};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_best_orders() {
    let bob_passphrase = get_passphrase(&".env.seed", "BOB_PASSPHRASE").unwrap();
    let coins = json!([rick_conf(), morty_conf(), eth_testnet_conf(), eth_jst_testnet_conf()]);

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
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {}", mm_bob.log_path.display());

    // Enable coins on Bob side. Print the replies in case we need the "address".
    let bob_coins = block_on(enable_coins_eth_electrum(&mm_bob, ETH_DEV_NODES));
    log!("enable_coins (bob): {:?}", bob_coins);
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
            "seednodes": [mm_bob.ip.to_string()],
            "rpc_password": "pass",
        }),
        "pass".into(),
        None,
    )
    .unwrap();

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {}", mm_alice.log_path.display());

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
fn test_best_orders_v2_by_number() {
    let bob_passphrase = get_passphrase(&".env.seed", "BOB_PASSPHRASE").unwrap();

    let coins = json!([rick_conf(), morty_conf(), eth_testnet_conf(), eth_jst_testnet_conf()]);

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
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {:?}", [mm_bob.log_path.display()]);

    // Enable coins on Bob side. Print the replies in case we need the "address".
    let bob_coins = block_on(enable_coins_eth_electrum(&mm_bob, ETH_DEV_NODES));
    log!("enable_coins (bob) {:?}", [bob_coins]);
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
            "seednodes": [mm_bob.ip.to_string()],
            "rpc_password": "pass",
        }),
        "pass".into(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {:?}", [mm_alice.log_path.display()]);
    block_on(mm_bob.wait_for_log(22., |log| {
        log.contains("DEBUG Handling IncludedTorelaysMesh message for peer")
    }))
    .unwrap();

    let response = block_on(best_orders_v2_by_number(&mm_alice, "RICK", "buy", 1, false));
    log!("response {response:?}");
    let best_morty_orders = response.result.orders.get("MORTY").unwrap();
    log!("Best MORTY orders when buy RICK {:?}", [best_morty_orders]);
    assert_eq!(1, best_morty_orders.len());
    let expected_price: BigDecimal = "0.7".parse().unwrap();
    assert_eq!(expected_price, best_morty_orders[0].price.decimal);

    let response = block_on(best_orders_v2_by_number(&mm_alice, "RICK", "buy", 2, false));
    log!("response {response:?}");
    let best_morty_orders = response.result.orders.get("MORTY").unwrap();
    log!("Best MORTY orders when buy RICK {:?}", [best_morty_orders]);
    assert_eq!(2, best_morty_orders.len());
    let expected_price: BigDecimal = "0.7".parse().unwrap();
    assert_eq!(expected_price, best_morty_orders[0].price.decimal);
    let expected_price: BigDecimal = "0.8".parse().unwrap();
    assert_eq!(expected_price, best_morty_orders[1].price.decimal);

    let response = block_on(best_orders_v2_by_number(&mm_alice, "RICK", "sell", 1, false));
    log!("response {response:?}");
    let expected_price: BigDecimal = "1.25".parse().unwrap();
    let best_morty_orders = response.result.orders.get("MORTY").unwrap();
    log!("Best MORTY orders when sell RICK {:?}", [best_morty_orders]);
    assert_eq!(1, best_morty_orders.len());
    assert_eq!(expected_price, best_morty_orders[0].price.decimal);
    let best_eth_orders = response.result.orders.get("ETH").unwrap();
    log!("Best ETH orders when sell RICK {:?}", [best_eth_orders]);
    assert_eq!(1, best_eth_orders.len());
    assert_eq!(expected_price, best_eth_orders[0].price.decimal);

    let response = block_on(best_orders_v2_by_number(&mm_alice, "ETH", "sell", 1, false));
    log!("response {response:?}");
    let best_rick_orders = response.result.orders.get("RICK").unwrap();
    log!("Best RICK orders when sell ETH {:?}", [best_rick_orders]);
    assert_eq!(1, best_rick_orders.len());
    let expected_price: BigDecimal = "1.25".parse().unwrap();
    assert_eq!(expected_price, best_rick_orders[0].price.decimal);

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_best_orders_v2_by_volume() {
    let bob_passphrase = get_passphrase(&".env.seed", "BOB_PASSPHRASE").unwrap();

    let coins = json!([rick_conf(), morty_conf(), eth_testnet_conf(), eth_jst_testnet_conf()]);

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
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {:?}", [mm_bob.log_path.display()]);

    // Enable coins on Bob side. Print the replies in case we need the "address".
    let bob_coins = block_on(enable_coins_eth_electrum(&mm_bob, ETH_DEV_NODES));
    log!("enable_coins (bob): {:?}", [bob_coins]);
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
            "seednodes": [mm_bob.ip.to_string()],
            "rpc_password": "pass",
        }),
        "pass".into(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {:?}", [mm_alice.log_path.display()]);
    block_on(mm_bob.wait_for_log(22., |log| {
        log.contains("DEBUG Handling IncludedTorelaysMesh message for peer")
    }))
    .unwrap();

    let response = block_on(best_orders_v2(&mm_alice, "RICK", "buy", "1.7"));
    log!("response {response:?}");
    // MORTY
    let best_morty_orders = response.result.orders.get("MORTY").unwrap();
    log!("Best MORTY orders when buy RICK {:?}", [best_morty_orders]);
    let expected_price: BigDecimal = "0.7".parse().unwrap();
    assert_eq!(expected_price, best_morty_orders[0].price.decimal);
    let expected_price: BigDecimal = "0.8".parse().unwrap();
    assert_eq!(expected_price, best_morty_orders[1].price.decimal);
    // ETH
    let expected_price: BigDecimal = "0.8".parse().unwrap();
    let best_eth_orders = response.result.orders.get("ETH").unwrap();
    log!("Best ETH orders when buy RICK {:?}", [best_eth_orders]);
    assert_eq!(expected_price, best_eth_orders[0].price.decimal);

    let response = block_on(best_orders_v2(&mm_alice, "RICK", "sell", "0.1"));
    log!("response {response:?}");
    let expected_price: BigDecimal = "1.25".parse().unwrap();
    let best_morty_orders = response.result.orders.get("MORTY").unwrap();
    log!("Best MORTY orders when sell RICK {:?}", [best_morty_orders]);
    assert_eq!(expected_price, best_morty_orders[0].price.decimal);
    assert_eq!(1, best_morty_orders.len());
    let best_eth_orders = response.result.orders.get("ETH").unwrap();
    log!("Best ETH orders when sell RICK {:?}", [best_morty_orders]);
    assert_eq!(expected_price, best_eth_orders[0].price.decimal);

    let response = block_on(best_orders_v2(&mm_alice, "ETH", "sell", "0.1"));
    log!("response {response:?}");
    let expected_price: BigDecimal = "1.25".parse().unwrap();
    let best_morty_orders = response.result.orders.get("MORTY").unwrap();
    log!("Best MORTY orders when sell ETH {:?}", [best_morty_orders]);
    assert_eq!(expected_price, best_morty_orders[0].price.decimal);
    assert_eq!("MORTY", best_morty_orders[0].coin);
    assert_eq!(1, best_morty_orders.len());

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_best_orders_v2_exclude_mine() {
    let coins = json!([rick_conf(), morty_conf(), eth_testnet_conf(), eth_jst_testnet_conf()]);
    let bob_passphrase = get_passphrase(&".env.seed", "BOB_PASSPHRASE").unwrap();
    let mm_bob = MarketMakerIt::start(
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
        None,
    )
    .unwrap();
    thread::sleep(Duration::from_secs(2));

    let _ = block_on(enable_coins_eth_electrum(&mm_bob, ETH_DEV_NODES));
    let bob_orders = [
        ("RICK", "MORTY", "0.9", "0.9", None),
        ("RICK", "MORTY", "0.8", "0.9", None),
    ];

    let mut bob_order_ids = BTreeSet::<Uuid>::new();
    for (base, rel, price, volume, min_volume) in bob_orders.iter() {
        let (status, data, _headers) = block_on(mm_bob.rpc(&json! ({
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
        let result: SetPriceResponse = json::from_str(&data).unwrap();
        bob_order_ids.insert(result.result.uuid);
        assert!(status.is_success(), "!setprice: {}", data);
    }

    let alice_passphrase = get_passphrase(&".env.seed", "ALICE_PASSPHRASE").unwrap();
    let mm_alice = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": alice_passphrase,
            "coins": coins,
            "seednodes": [mm_bob.ip.to_string()],
            "rpc_password": "pass",
        }),
        "pass".into(),
        None,
    )
    .unwrap();
    thread::sleep(Duration::from_secs(2));

    let _ = block_on(enable_coins_eth_electrum(&mm_alice, ETH_DEV_NODES));
    let alice_orders = [("RICK", "MORTY", "0.85", "1", None)];
    let mut alice_order_ids = BTreeSet::<Uuid>::new();
    for (base, rel, price, volume, min_volume) in alice_orders.iter() {
        let (status, data, _headers) = block_on(mm_alice.rpc(&json! ({
            "userpass": mm_alice.userpass,
            "method": "setprice",
            "base": base,
            "rel": rel,
            "price": price,
            "volume": volume,
            "min_volume": min_volume.unwrap_or("0.00777"),
            "cancel_previous": false,
        })))
        .unwrap();
        let result: SetPriceResponse = json::from_str(&data).unwrap();
        alice_order_ids.insert(result.result.uuid);
        assert!(status.is_success(), "!setprice: {}", data);
    }
    thread::sleep(Duration::from_secs(2));

    let response = block_on(best_orders_v2_by_number(&mm_alice, "RICK", "buy", 100, false));
    log!("all orders response: {response:?}");
    assert_eq!(response.result.orders.get("MORTY").unwrap().len(), 3);

    let response = block_on(best_orders_v2_by_number(&mm_alice, "RICK", "buy", 100, true));
    log!("alice orders response: {response:?}");
    assert_eq!(response.result.orders.get("MORTY").unwrap().len(), 2);
    for order in response.result.orders.get("MORTY").unwrap() {
        assert!(bob_order_ids.remove(&order.uuid));
    }
    assert!(bob_order_ids.is_empty());

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_best_orders_no_duplicates_after_update() {
    let eve_passphrase = get_passphrase(&".env.seed", "BOB_PASSPHRASE").unwrap();

    let coins = json!([rick_conf(), morty_conf()]);

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
        None,
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
            "seednodes": [mm_bob.ip.to_string()],
        }),
        "pass".into(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {}", mm_bob.log_path.display());

    // Enable coins on Eve side. Print the replies in case we need the "address".
    let eve_coins = block_on(enable_coins_rick_morty_electrum(&mm_eve));
    log!("enable_coins (eve): {:?}", eve_coins);
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
            "seednodes": [mm_bob.ip.to_string()],
            "rpc_password": "pass",
        }),
        "pass".into(),
        None,
    )
    .unwrap();

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {}", mm_alice.log_path.display());

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

    let bob_coins_config = json!([rick_conf(), morty_conf(), eth_testnet_conf(), eth_jst_testnet_conf()]);

    // alice defined MORTY as "wallet_only" in config
    let alice_coins_config = json!([
        rick_conf(),
        eth_testnet_conf(),
        eth_jst_testnet_conf(),
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"wallet_only": true,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
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
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {}", mm_bob.log_path.display());

    // Enable coins on Bob side. Print the replies in case we need the "address".
    let bob_coins = block_on(enable_coins_eth_electrum(&mm_bob, ETH_DEV_NODES));
    log!("enable_coins (bob): {:?}", bob_coins);
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
            "seednodes": [mm_bob.ip.to_string()],
            "rpc_password": "pass",
        }),
        "pass".into(),
        None,
    )
    .unwrap();

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {}", mm_alice.log_path.display());

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
        rick_conf(),
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
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {}", mm_bob.log_path.display());

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
    log!("enable tBTC: {:?}", electrum);
    let enable_tbtc_res: CoinInitResponse = json::from_str(&electrum.1).unwrap();
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
    log!("enable RICK: {:?}", electrum);
    let enable_rick_res: CoinInitResponse = json::from_str(&electrum.1).unwrap();
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
            "seednodes": [mm_bob.ip.to_string()],
            "rpc_password": "pass",
        }),
        "pass".into(),
        None,
    )
    .unwrap();

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {}", mm_alice.log_path.display());

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
    assert_eq!(best_orders[0].conf_settings.as_ref().unwrap().base_confs, 5);
    assert!(!best_orders[0].conf_settings.as_ref().unwrap().base_nota);
    assert_eq!(best_orders[0].conf_settings.as_ref().unwrap().rel_confs, 10);
    assert!(best_orders[0].conf_settings.as_ref().unwrap().rel_nota);

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
    assert_eq!(best_orders[0].conf_settings.as_ref().unwrap().base_confs, 10);
    assert!(best_orders[0].conf_settings.as_ref().unwrap().base_nota);
    assert_eq!(best_orders[0].conf_settings.as_ref().unwrap().rel_confs, 5);
    assert!(!best_orders[0].conf_settings.as_ref().unwrap().rel_nota);

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
    assert_eq!(best_orders[0].conf_settings.as_ref().unwrap().base_confs, 10);
    assert!(best_orders[0].conf_settings.as_ref().unwrap().base_nota);
    assert_eq!(best_orders[0].conf_settings.as_ref().unwrap().rel_confs, 5);
    assert!(!best_orders[0].conf_settings.as_ref().unwrap().rel_nota);

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
    assert_eq!(best_orders[0].conf_settings.as_ref().unwrap().base_confs, 5);
    assert!(!best_orders[0].conf_settings.as_ref().unwrap().base_nota);
    assert_eq!(best_orders[0].conf_settings.as_ref().unwrap().rel_confs, 10);
    assert!(best_orders[0].conf_settings.as_ref().unwrap().rel_nota);

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
fn best_orders_must_return_duplicate_for_orderbook_tickers() {
    let bob_passphrase = get_passphrase(&".env.seed", "BOB_PASSPHRASE").unwrap();
    let alice_passphrase = get_passphrase(&".env.client", "ALICE_PASSPHRASE").unwrap();

    let coins = json!([rick_conf(), tbtc_conf(), tbtc_segwit_conf()]);

    let bob_conf = Mm2TestConf::seednode(&bob_passphrase, &coins);
    let mm_bob = MarketMakerIt::start(bob_conf.conf, bob_conf.rpc_password, None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {}", mm_bob.log_path.display());

    let t_btc_bob = block_on(enable_electrum(&mm_bob, "tBTC", false, TBTC_ELECTRUMS));
    log!("Bob enable tBTC: {:?}", t_btc_bob);

    let rick_bob = block_on(enable_electrum(&mm_bob, "RICK", false, RICK_ELECTRUM_ADDRS));
    log!("Bob enable RICK: {:?}", rick_bob);

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

    let alice_conf = Mm2TestConf::light_node(&alice_passphrase, &coins, &[&mm_bob.ip.to_string()]);
    let mm_alice = MarketMakerIt::start(alice_conf.conf, alice_conf.rpc_password, None).unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {}", mm_alice.log_path.display());

    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "best_orders",
        "coin": "tBTC-Segwit",
        "action": "buy",
        "volume": "0.0002",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!best_orders: {}", rc.1);
    let response: BestOrdersResponse = json::from_str(&rc.1).unwrap();
    let best_orders = response.result.get("RICK").unwrap();
    assert_eq!(best_orders.len(), 1);

    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "best_orders",
        "coin": "tBTC-Segwit",
        "action": "sell",
        "volume": "0.0002",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!best_orders: {}", rc.1);
    let response: BestOrdersResponse = json::from_str(&rc.1).unwrap();
    let best_orders = response.result.get("RICK").unwrap();
    assert_eq!(best_orders.len(), 1);

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
    assert_eq!(best_orders.len(), 1);
    let best_orders = response.result.get("tBTC-Segwit").unwrap();
    assert_eq!(best_orders.len(), 1);
    assert_eq!(best_orders[0].coin, "tBTC-Segwit");

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
    assert_eq!(best_orders.len(), 1);
    let best_orders = response.result.get("tBTC-Segwit").unwrap();
    assert_eq!(best_orders.len(), 1);
    assert_eq!(best_orders[0].coin, "tBTC-Segwit");

    let response = block_on(best_orders_v2(&mm_alice, "tBTC-Segwit", "buy", "0.0002"));
    let best_orders = response.result.orders.get("RICK").unwrap();
    assert_eq!(best_orders.len(), 1);

    let response = block_on(best_orders_v2(&mm_alice, "tBTC-Segwit", "sell", "0.0002"));
    let best_orders = response.result.orders.get("RICK").unwrap();
    assert_eq!(best_orders.len(), 1);

    let response = block_on(best_orders_v2(&mm_alice, "RICK", "buy", "0.0002"));
    let best_orders = response.result.orders.get("tBTC").unwrap();
    assert_eq!(best_orders.len(), 1);
    let best_orders = response.result.orders.get("tBTC-Segwit").unwrap();
    assert_eq!(best_orders.len(), 1);
    assert_eq!(best_orders[0].coin, "tBTC-Segwit");

    let response = block_on(best_orders_v2(&mm_alice, "RICK", "sell", "0.0002"));
    let best_orders = response.result.orders.get("tBTC").unwrap();
    assert_eq!(best_orders.len(), 1);
    let best_orders = response.result.orders.get("tBTC-Segwit").unwrap();
    assert_eq!(best_orders.len(), 1);
    assert_eq!(best_orders[0].coin, "tBTC-Segwit");
}

#[test]
#[cfg(feature = "zhtlc-native-tests")]
fn zhtlc_best_orders() {
    use super::enable_z_coin;
    use mm2_test_helpers::for_tests::zombie_conf;

    use mm2_test_helpers::electrums::rick_electrums;

    let bob_passphrase = get_passphrase(&".env.seed", "BOB_PASSPHRASE").unwrap();
    let alice_passphrase = get_passphrase(&".env.client", "ALICE_PASSPHRASE").unwrap();

    let coins = json!([rick_conf(), zombie_conf()]);

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
        None,
    )
    .unwrap();

    let (_dump_log, _dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {}", mm_bob.log_path.display());

    let rmd = rmd160_from_passphrase(&bob_passphrase);
    let bob_zombie_cache_path = mm_bob.folder.join("DB").join(hex::encode(rmd)).join("ZOMBIE_CACHE.db");
    log!("bob_zombie_cache_path {}", bob_zombie_cache_path.display());
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
            "seednodes": [mm_bob.ip.to_string()],
            "rpc_password": "pass",
        }),
        "pass".into(),
        None,
    )
    .unwrap();

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {}", mm_alice.log_path.display());

    let best_orders = block_on(best_orders_v2(&mm_alice, "RICK", "sell", "1"));
    let zombie_best_orders = best_orders.result.orders.get("ZOMBIE").unwrap();

    assert_eq!(1, zombie_best_orders.len());
    zombie_best_orders
        .iter()
        .find(|order| order.uuid == bob_set_price_res.result.uuid)
        .unwrap();

    let best_orders = block_on(best_orders_v2(&mm_alice, "ZOMBIE", "buy", "1"));
    let rick_best_orders = best_orders.result.orders.get("RICK").unwrap();

    assert_eq!(1, rick_best_orders.len());
    rick_best_orders
        .iter()
        .find(|order| order.uuid == bob_set_price_res.result.uuid)
        .unwrap();
}
