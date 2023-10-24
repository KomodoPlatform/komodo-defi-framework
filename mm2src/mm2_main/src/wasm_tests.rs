use crate::mm2::lp_init;
use common::executor::{spawn, Timer};
use common::log::info;
use common::log::wasm_log::register_wasm_log;
use common::wait_until_ms;
use crypto::StandardHDCoinAddress;
use gstuff::now_ms;
use mm2_core::mm_ctx::MmArc;
use mm2_number::BigDecimal;
use mm2_rpc::data::legacy::OrderbookResponse;
use mm2_test_helpers::electrums::{morty_electrums, rick_electrums};
use mm2_test_helpers::for_tests::{check_recent_swaps, enable_electrum_json, init_z_coin_light, init_z_coin_status,
                                  morty_conf, rick_conf, start_swaps, test_qrc20_history_impl,
                                  wait_for_swaps_finish_and_check_status, zombie_conf, MarketMakerIt,
                                  Mm2InitPrivKeyPolicy, Mm2TestConf, Mm2TestConfForSwap, MORTY, RICK,
                                  ZOMBIE_ELECTRUMS, ZOMBIE_LIGHTWALLETD_URLS, ZOMBIE_TICKER};
use mm2_test_helpers::get_passphrase;
use mm2_test_helpers::structs::{EnableCoinBalance, InitTaskResult, InitZcoinStatus, RpcV2Response,
                                ZCoinActivationResult};
use serde_json::json;
use serde_json::{self as json};
use wasm_bindgen_test::wasm_bindgen_test;

const ZOMBIE_TEST_BALANCE_SEED: &str = "zombie test seed";

/// Starts the WASM version of MM.
fn wasm_start(ctx: MmArc) {
    spawn(async move {
        lp_init(ctx, "TEST".into(), "TEST".into()).await.unwrap();
    })
}

/// This function runs Alice and Bob nodes, activates coins, starts swaps,
/// and then immediately stops the nodes to check if `MmArc` is dropped in a short period.
async fn test_mm2_stops_impl(
    pairs: &[(&'static str, &'static str)],
    maker_price: f64,
    taker_price: f64,
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
    let rc = enable_electrum_json(&mm_bob, RICK, true, rick_electrums(), None).await;
    log!("enable RICK (bob): {:?}", rc);

    let rc = enable_electrum_json(&mm_bob, MORTY, true, morty_electrums(), None).await;
    log!("enable MORTY (bob): {:?}", rc);

    // Enable coins on Alice side. Print the replies in case we need the address.
    let rc = enable_electrum_json(&mm_alice, RICK, true, rick_electrums(), None).await;
    log!("enable RICK (bob): {:?}", rc);

    let rc = enable_electrum_json(&mm_alice, MORTY, true, morty_electrums(), None).await;
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
    test_mm2_stops_impl(pairs, 1., 1., 0.0001, STOP_TIMEOUT_MS).await;
}

#[wasm_bindgen_test]
async fn test_qrc20_tx_history() { test_qrc20_history_impl(Some(wasm_start)).await }

async fn trade_base_rel_electrum(
    bob_priv_key_policy: Mm2InitPrivKeyPolicy,
    alice_priv_key_policy: Mm2InitPrivKeyPolicy,
    bob_path_to_address: Option<StandardHDCoinAddress>,
    alice_path_to_address: Option<StandardHDCoinAddress>,
    pairs: &[(&'static str, &'static str)],
    maker_price: f64,
    taker_price: f64,
    volume: f64,
) {
    let coins = json!([rick_conf(), morty_conf(),]);

    let bob_conf = Mm2TestConfForSwap::bob_conf_with_policy(&bob_priv_key_policy, &coins);
    let mut mm_bob = MarketMakerIt::start_async(bob_conf.conf, bob_conf.rpc_password, Some(wasm_start))
        .await
        .unwrap();

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    Timer::sleep(1.).await;

    let alice_conf = Mm2TestConfForSwap::alice_conf_with_policy(&alice_priv_key_policy, &coins, &mm_bob.my_seed_addr());
    let mut mm_alice = MarketMakerIt::start_async(alice_conf.conf, alice_conf.rpc_password, Some(wasm_start))
        .await
        .unwrap();

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();

    // Enable coins on Bob side. Print the replies in case we need the address.
    let rc = enable_electrum_json(&mm_bob, RICK, true, rick_electrums(), bob_path_to_address.clone()).await;
    log!("enable RICK (bob): {:?}", rc);

    let rc = enable_electrum_json(&mm_bob, MORTY, true, morty_electrums(), bob_path_to_address).await;
    log!("enable MORTY (bob): {:?}", rc);

    // Enable coins on Alice side. Print the replies in case we need the address.
    let rc = enable_electrum_json(&mm_alice, RICK, true, rick_electrums(), alice_path_to_address.clone()).await;
    log!("enable RICK (bob): {:?}", rc);

    let rc = enable_electrum_json(&mm_alice, MORTY, true, morty_electrums(), alice_path_to_address).await;
    log!("enable MORTY (bob): {:?}", rc);

    let uuids = start_swaps(&mut mm_bob, &mut mm_alice, pairs, maker_price, taker_price, volume).await;

    wait_for_swaps_finish_and_check_status(&mut mm_bob, &mut mm_alice, &uuids, volume, maker_price).await;

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
    let bob_policy = Mm2InitPrivKeyPolicy::Iguana;
    let alice_policy = Mm2InitPrivKeyPolicy::GlobalHDAccount;
    let alice_path_to_address = StandardHDCoinAddress {
        account: 0,
        is_change: false,
        address_index: 0,
    };
    let pairs: &[_] = &[("RICK", "MORTY")];
    trade_base_rel_electrum(
        bob_policy,
        alice_policy,
        None,
        Some(alice_path_to_address),
        pairs,
        1.,
        1.,
        0.0001,
    )
    .await;
}

pub async fn enable_z_coin_light(
    mm: &MarketMakerIt,
    coin: &str,
    electrums: &[&str],
    lightwalletd_urls: &[&str],
    starting_date: Option<u64>,
    account: Option<u32>,
) -> ZCoinActivationResult {
    let init = init_z_coin_light(mm, coin, electrums, lightwalletd_urls, starting_date, account).await;
    let init: RpcV2Response<InitTaskResult> = json::from_value(init).unwrap();
    info!("INIT RESULT: {init:?}");
    let timeout = wait_until_ms(60000);

    loop {
        if now_ms() > timeout {
            panic!("{} initialization timed out", coin);
        }
        let status = init_z_coin_status(mm, init.result.task_id).await;
        println!("Status {}", json::to_string(&status).unwrap());
        let status: RpcV2Response<InitZcoinStatus> = json::from_value(status).unwrap();
        match status.result {
            InitZcoinStatus::Ok(result) => break result,
            InitZcoinStatus::Error(e) => panic!("{} initialization error {:?}", coin, e),
            _ => Timer::sleep(1.).await,
        }
    }
}

#[wasm_bindgen_test]
async fn activate_z_coin_light() {
    register_wasm_log();
    let coins = json!([zombie_conf()]);

    let conf = Mm2TestConf::seednode(ZOMBIE_TEST_BALANCE_SEED, &coins);
    let mm = MarketMakerIt::start_async(conf.conf, conf.rpc_password, Some(wasm_start))
        .await
        .unwrap();

    let activation_result = enable_z_coin_light(
        &mm,
        ZOMBIE_TICKER,
        ZOMBIE_ELECTRUMS,
        ZOMBIE_LIGHTWALLETD_URLS,
        None,
        None,
    )
    .await;

    let balance = match activation_result.wallet_balance {
        EnableCoinBalance::Iguana(iguana) => iguana,
        _ => panic!("Expected EnableCoinBalance::Iguana"),
    };
    assert_eq!(balance.balance.spendable, BigDecimal::default());
}
