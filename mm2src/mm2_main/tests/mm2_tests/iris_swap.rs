use crate::integration_tests_common::enable_electrum;
use common::executor::Timer;
use common::{block_on, log};
use mm2_number::BigDecimal;
use mm2_test_helpers::for_tests::{check_my_swap_status, check_recent_swaps, check_stats_swap_status, enable_eth_coin,
                                  enable_tendermint, iris_nimda_testnet_conf, iris_testnet_conf, rick_conf, tbnb_conf,
                                  usdc_ibc_iris_testnet_conf, MarketMakerIt, MAKER_ERROR_EVENTS, MAKER_SUCCESS_EVENTS,
                                  RICK_ELECTRUM_ADDRS, TAKER_ERROR_EVENTS, TAKER_SUCCESS_EVENTS};
use mm2_test_helpers::structs::OrderbookResponse;
use serde_json::{json, Value as Json};
use std::convert::TryFrom;
use std::env;

// https://academy.binance.com/en/articles/connecting-metamask-to-binance-smart-chain
const TBNB_URLS: &[&str] = &["https://data-seed-prebsc-1-s3.binance.org:8545/"];
// https://testnet.bscscan.com/address/0xb1ad803ea4f57401639c123000c75f5b66e4d123
const TBNB_SWAP_CONTRACT: &str = "0xB1Ad803ea4F57401639c123000C75F5B66E4D123";

#[test]
fn start_swap_operation() {
    let pairs = [
        ("USDC-IBC-IRIS", "IRIS-NIMDA"),
        ("IRIS-NIMDA", "RICK"),
        // ("USDC-IBC-IRIS", "tBNB"), having fund problems
    ];
    block_on(trade_base_rel_iris(&pairs, 1, 2, 0.008));
}

pub async fn trade_base_rel_iris(
    pairs: &[(&'static str, &'static str)],
    maker_price: i32,
    taker_price: i32,
    volume: f64,
) {
    let bob_passphrase = String::from("iris test seed");
    let alice_passphrase = String::from("iris test2 seed");

    let coins = json!([
        usdc_ibc_iris_testnet_conf(),
        iris_nimda_testnet_conf(),
        iris_testnet_conf(),
        rick_conf(),
        tbnb_conf(),
    ]);

    println!("coins config {}", serde_json::to_string(&coins).unwrap());

    let mut mm_bob = MarketMakerIt::start_async(
        json!({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",
            "myipaddr": env::var("BOB_TRADE_IP") .ok(),
            "rpcip": env::var("BOB_TRADE_IP") .ok(),
            "canbind": env::var("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        None,
    )
    .await
    .unwrap();

    Timer::sleep(1.).await;

    let mut mm_alice = MarketMakerIt::start_async(
        json!({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",
            "myipaddr": env::var("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var("ALICE_TRADE_IP") .ok(),
            "passphrase": alice_passphrase,
            "coins": coins,
            "seednodes": [mm_bob.my_seed_addr()],
            "rpc_password": "password",
            "skip_startup_checks": true,
        }),
        "password".into(),
        None,
    )
    .await
    .unwrap();

    dbg!(
        enable_tendermint(
            &mm_bob,
            "IRIS-TEST",
            &["IRIS-NIMDA", "USDC-IBC-IRIS"],
            &["http://34.80.202.172:26657"],
            false
        )
        .await
    );
    dbg!(enable_electrum(&mm_bob, "RICK", false, RICK_ELECTRUM_ADDRS).await);

    dbg!(
        enable_tendermint(
            &mm_alice,
            "IRIS-TEST",
            &["IRIS-NIMDA", "USDC-IBC-IRIS"],
            &["http://34.80.202.172:26657"],
            false
        )
        .await
    );
    dbg!(enable_electrum(&mm_alice, "RICK", false, RICK_ELECTRUM_ADDRS).await);
    dbg!(enable_eth_coin(&mm_bob, "tBNB", TBNB_URLS, TBNB_SWAP_CONTRACT, None, false).await);
    dbg!(enable_eth_coin(&mm_alice, "tBNB", TBNB_URLS, TBNB_SWAP_CONTRACT, None, false).await);

    for (base, rel) in pairs.iter() {
        log!("Issue bob {}/{} sell request", base, rel);
        let rc = mm_bob
            .rpc(&json!({
                "userpass": mm_bob.userpass,
                "method": "setprice",
                "base": base,
                "rel": rel,
                "price": maker_price,
                "volume": volume
            }))
            .await
            .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    }

    let mut uuids = vec![];

    for (base, rel) in pairs.iter() {
        common::log::info!(
            "Trigger alice subscription to {}/{} orderbook topic first and sleep for 1 second",
            base,
            rel
        );
        let rc = mm_alice
            .rpc(&json!({
                "userpass": mm_alice.userpass,
                "method": "orderbook",
                "base": base,
                "rel": rel,
            }))
            .await
            .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);
        Timer::sleep(1.).await;
        common::log::info!("Issue alice {}/{} buy request", base, rel);
        let rc = mm_alice
            .rpc(&json!({
                "userpass": mm_alice.userpass,
                "method": "buy",
                "base": base,
                "rel": rel,
                "volume": volume,
                "price": taker_price
            }))
            .await
            .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);
        let buy_json: Json = serde_json::from_str(&rc.1).unwrap();
        uuids.push(buy_json["result"]["uuid"].as_str().unwrap().to_owned());
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
        match mm_bob
            .wait_for_log(900., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))
            .await
        {
            Ok(_) => (),
            Err(_) => {
                println!("{}", mm_bob.log_as_utf8().unwrap());
            },
        }

        match mm_alice
            .wait_for_log(900., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))
            .await
        {
            Ok(_) => (),
            Err(_) => {
                println!("{}", mm_alice.log_as_utf8().unwrap());
            },
        }

        log!("Waiting a few second for the fresh swap status to be saved..");
        Timer::sleep(5.).await;

        println!("{}", mm_alice.log_as_utf8().unwrap());
        log!("Checking alice/taker status..");
        check_my_swap_status(
            &mm_alice,
            uuid,
            &TAKER_SUCCESS_EVENTS,
            &TAKER_ERROR_EVENTS,
            BigDecimal::try_from(volume).unwrap(),
            BigDecimal::try_from(volume).unwrap(),
        )
        .await;

        println!("{}", mm_bob.log_as_utf8().unwrap());
        log!("Checking bob/maker status..");
        check_my_swap_status(
            &mm_bob,
            uuid,
            &MAKER_SUCCESS_EVENTS,
            &MAKER_ERROR_EVENTS,
            BigDecimal::try_from(volume).unwrap(),
            BigDecimal::try_from(volume).unwrap(),
        )
        .await;
    }

    log!("Waiting 3 seconds for nodes to broadcast their swaps data..");
    Timer::sleep(3.).await;

    for uuid in uuids.iter() {
        log!("Checking alice status..");
        check_stats_swap_status(&mm_alice, uuid, &MAKER_SUCCESS_EVENTS, &TAKER_SUCCESS_EVENTS).await;

        log!("Checking bob status..");
        check_stats_swap_status(&mm_bob, uuid, &MAKER_SUCCESS_EVENTS, &TAKER_SUCCESS_EVENTS).await;
    }

    log!("Checking alice recent swaps..");
    check_recent_swaps(&mm_alice, uuids.len()).await;
    log!("Checking bob recent swaps..");
    check_recent_swaps(&mm_bob, uuids.len()).await;
    for (base, rel) in pairs.iter() {
        log!("Get {}/{} orderbook", base, rel);
        let rc = mm_bob
            .rpc(&json!({
                "userpass": mm_bob.userpass,
                "method": "orderbook",
                "base": base,
                "rel": rel,
            }))
            .await
            .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: OrderbookResponse = serde_json::from_str(&rc.1).unwrap();
        log!("{}/{} orderbook {:?}", base, rel, bob_orderbook);

        assert_eq!(0, bob_orderbook.bids.len(), "{} {} bids must be empty", base, rel);
        assert_eq!(0, bob_orderbook.asks.len(), "{} {} asks must be empty", base, rel);
    }
    mm_bob.stop().await.unwrap();
    mm_alice.stop().await.unwrap();
}
