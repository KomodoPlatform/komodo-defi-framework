use super::lp_init;
use common::executor::{spawn, Timer};
use common::log::wasm_log::register_wasm_log;
use mm2_core::mm_ctx::MmArc;
use mm2_test_helpers::electrums::{morty_electrums, rick_electrums};
use mm2_test_helpers::for_tests::{check_recent_swaps, enable_electrum_json, morty_conf, rick_conf, start_swaps,
                                  test_qrc20_history_impl, wait_for_swaps_finish_and_check_status, MarketMakerIt,
                                  Mm2TestConf, MORTY, RICK};
use mm2_test_helpers::get_passphrase;
use mm2_test_helpers::structs::OrderbookResponse;
use serde_json::json;
use std::env;
use wasm_bindgen_test::wasm_bindgen_test;

/// Starts the WASM version of MM.
fn wasm_start(ctx: MmArc) {
    spawn(async move {
        lp_init(ctx).await.unwrap();
    })
}

/// This function runs Alice and Bob nodes, activates coins, starts swaps,
/// and then immediately stops the nodes to check if `MmArc` is dropped in a short period.
async fn test_mm2_stops_impl(
    pairs: &[(&'static str, &'static str)],
    maker_price: i32,
    taker_price: i32,
    volume: f64,
    stop_timeout_ms: u64,
) {
    let coins = json!([rick_conf(), morty_conf()]);

    let bob_passphrase = get_passphrase!(".env.seed", "BOB_PASSPHRASE").unwrap();
    let alice_passphrase = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();

    let bob_conf = Mm2TestConf::seednode(&bob_passphrase, &coins);
    let mut mm_bob = MarketMakerIt::start_async(bob_conf.conf, bob_conf.rpc_password, Some(wasm_start))
        .await
        .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    Timer::sleep(1.).await;

    let alice_conf = Mm2TestConf::light_node(&alice_passphrase, &coins, &[&mm_bob.my_seed_addr()]);
    let mut mm_alice = MarketMakerIt::start_async(alice_conf.conf, alice_conf.rpc_password, Some(wasm_start))
        .await
        .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();

    // Enable coins on Bob side. Print the replies in case we need the address.
    let rc = enable_electrum_json(&mm_bob, RICK, true, rick_electrums()).await;
    log!("enable RICK (bob): {:?}", rc);

    let rc = enable_electrum_json(&mm_bob, MORTY, true, morty_electrums()).await;
    log!("enable MORTY (bob): {:?}", rc);

    // Enable coins on Alice side. Print the replies in case we need the address.
    let rc = enable_electrum_json(&mm_alice, RICK, true, rick_electrums()).await;
    log!("enable RICK (bob): {:?}", rc);

    let rc = enable_electrum_json(&mm_alice, MORTY, true, morty_electrums()).await;
    log!("enable MORTY (bob): {:?}", rc);

    start_swaps(&mut mm_bob, &mut mm_alice, pairs, maker_price, taker_price, volume).await;

    mm_alice
        .stop_and_wait_for_ctx_is_dropped(stop_timeout_ms)
        .await
        .unwrap();
    mm_bob.stop_and_wait_for_ctx_is_dropped(stop_timeout_ms).await.unwrap();
}

#[wasm_bindgen_test]
async fn test_mm2_stops_immediately() {
    const STOP_TIMEOUT_MS: u64 = 1000;

    register_wasm_log();

    let pairs: &[_] = &[("RICK", "MORTY")];
    test_mm2_stops_impl(pairs, 1, 1, 0.0001, STOP_TIMEOUT_MS).await;
}

#[wasm_bindgen_test]
async fn test_qrc20_tx_history() { test_qrc20_history_impl(Some(wasm_start)).await }

async fn trade_base_rel_electrum(
    pairs: &[(&'static str, &'static str)],
    maker_price: i32,
    taker_price: i32,
    volume: f64,
) {
    let bob_passphrase = get_passphrase!(".env.seed", "BOB_PASSPHRASE").unwrap();
    let alice_passphrase = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();

    let coins = json!([rick_conf(), morty_conf(),]);

    let mut mm_bob = MarketMakerIt::start_async(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        Some(wasm_start),
    )
    .await
    .unwrap();

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    Timer::sleep(1.).await;

    let mut mm_alice = MarketMakerIt::start_async(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": alice_passphrase,
            "coins": coins,
            "seednodes": [mm_bob.my_seed_addr()],
            "rpc_password": "password",
            "skip_startup_checks": true,
        }),
        "password".into(),
        Some(wasm_start),
    )
    .await
    .unwrap();

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();

    // Enable coins on Bob side. Print the replies in case we need the address.
    let rc = enable_electrum_json(&mm_bob, RICK, true, rick_electrums()).await;
    log!("enable RICK (bob): {:?}", rc);

    let rc = enable_electrum_json(&mm_bob, MORTY, true, morty_electrums()).await;
    log!("enable MORTY (bob): {:?}", rc);

    // Enable coins on Alice side. Print the replies in case we need the address.
    let rc = enable_electrum_json(&mm_alice, RICK, true, rick_electrums()).await;
    log!("enable RICK (bob): {:?}", rc);

    let rc = enable_electrum_json(&mm_alice, MORTY, true, morty_electrums()).await;
    log!("enable MORTY (bob): {:?}", rc);

    let uuids = start_swaps(&mut mm_bob, &mut mm_alice, pairs, maker_price, taker_price, volume).await;

    wait_for_swaps_finish_and_check_status(&mut mm_bob, &mut mm_alice, &uuids, volume).await;

    log!("Checking alice recent swaps..");
    check_recent_swaps(&mm_alice, uuids.len()).await;
    log!("Checking bob recent swaps..");
    check_recent_swaps(&mm_bob, uuids.len()).await;
    for (base, rel) in pairs.iter() {
        log!("Get {}/{} orderbook", base, rel);
        let rc = mm_bob
            .rpc(&json! ({
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

    const STOP_TIMEOUT_MS: u64 = 1000;

    mm_bob.stop_and_wait_for_ctx_is_dropped(STOP_TIMEOUT_MS).await.unwrap();
    mm_alice
        .stop_and_wait_for_ctx_is_dropped(STOP_TIMEOUT_MS)
        .await
        .unwrap();
}

#[wasm_bindgen_test]
async fn trade_test_rick_and_morty() {
    let pairs: &[_] = &[("RICK", "MORTY")];
    trade_base_rel_electrum(pairs, 1, 1, 0.0001).await;
}
