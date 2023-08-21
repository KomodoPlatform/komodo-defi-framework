use common::executor::Timer;
use common::log::{info, LogLevel};
use common::{block_on, log, now_ms, now_sec, wait_until_ms};
use crypto::privkey::key_pair_from_seed;
use mm2_main::mm2::{lp_main, LpMainParams};
use mm2_number::BigDecimal;
use mm2_rpc::data::legacy::CoinInitResponse;
use mm2_test_helpers::electrums::{morty_electrums, rick_electrums};
use mm2_test_helpers::for_tests::{disable_coin, enable_native as enable_native_impl, init_utxo_electrum,
                                  init_utxo_status, init_z_coin_light, init_z_coin_status, MarketMakerIt};
use mm2_test_helpers::structs::{EnableCoinBalance, InitTaskResult, InitUtxoStatus, InitZcoinStatus, RpcV2Response,
                                UtxoStandardActivationResult, ZCoinActivationResult};
use serde_json::{self as json, Value as Json};
use std::collections::HashMap;
use std::env::var;
use std::str::FromStr;

/// This is not a separate test but a helper used by `MarketMakerIt` to run the MarketMaker from the test binary.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_mm_start() { test_mm_start_impl(); }

pub fn test_mm_start_impl() {
    if let Ok(conf) = var("_MM2_TEST_CONF") {
        if let Ok(log_var) = var("RUST_LOG") {
            if let Ok(filter) = LogLevel::from_str(&log_var) {
                log!("test_mm_start] Starting the MarketMaker...");
                let conf: Json = json::from_str(&conf).unwrap();
                let params = LpMainParams::with_conf(conf).log_filter(Some(filter));
                block_on(lp_main(params, &|_ctx| (), "TEST".into(), "TEST".into())).unwrap()
            }
        }
    }
}

/// Ideally, this function should be replaced everywhere with `enable_electrum_json`.
pub async fn enable_electrum(mm: &MarketMakerIt, coin: &str, tx_history: bool, urls: &[&str]) -> CoinInitResponse {
    use mm2_test_helpers::for_tests::enable_electrum as enable_electrum_impl;

    let value = enable_electrum_impl(mm, coin, tx_history, urls).await;
    json::from_value(value).unwrap()
}

pub async fn enable_electrum_json(
    mm: &MarketMakerIt,
    coin: &str,
    tx_history: bool,
    servers: Vec<Json>,
) -> CoinInitResponse {
    use mm2_test_helpers::for_tests::enable_electrum_json as enable_electrum_impl;

    let value = enable_electrum_impl(mm, coin, tx_history, servers).await;
    json::from_value(value).unwrap()
}

pub async fn enable_native(mm: &MarketMakerIt, coin: &str, urls: &[&str]) -> CoinInitResponse {
    let value = enable_native_impl(mm, coin, urls).await;
    json::from_value(value).unwrap()
}

pub async fn enable_coins_rick_morty_electrum(mm: &MarketMakerIt) -> HashMap<&'static str, CoinInitResponse> {
    let mut replies = HashMap::new();
    replies.insert("RICK", enable_electrum_json(mm, "RICK", false, rick_electrums()).await);
    replies.insert(
        "MORTY",
        enable_electrum_json(mm, "MORTY", false, morty_electrums()).await,
    );
    replies
}

pub async fn enable_z_coin_light(
    mm: &MarketMakerIt,
    coin: &str,
    electrums: &[&str],
    lightwalletd_urls: &[&str],
) -> ZCoinActivationResult {
    let init = init_z_coin_light(mm, coin, electrums, lightwalletd_urls, None).await;
    let init: RpcV2Response<InitTaskResult> = json::from_value(init).unwrap();
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
pub async fn enable_z_coin_light_with_changing_height(
    mm: &MarketMakerIt,
    coin: &str,
    electrums: &[&str],
    lightwalletd_urls: &[&str],
) -> ZCoinActivationResult {
    // Initial activation
    let init = init_z_coin_light(mm, coin, electrums, lightwalletd_urls, None).await;
    let init_result: RpcV2Response<InitTaskResult> = json::from_value(init).unwrap();

    // Loop until activation status is obtained or timeout
    let f_activation_result = loop_until_status_ok(mm, &init_result.result, wait_until_ms(60000)).await;
    log!("init_utxo_status: {:?}", f_activation_result);
    let balance = match f_activation_result.wallet_balance {
        EnableCoinBalance::Iguana(iguana) => iguana,
        _ => panic!("Expected EnableCoinBalance::Iguana"),
    };
    assert_eq!(balance.balance.spendable, BigDecimal::default());
    // disable coin
    disable_coin(mm, coin, true).await;

    // Perform activation with changed height
    // Calculate timestamp for 2 days ago
    let two_day_seconds = 2 * 24 * 60 * 60;
    let two_days_ago = now_sec() - two_day_seconds;
    info!(
        "Re-running enable_z_coin_light_with_changing_height with new starting date {}",
        two_days_ago
    );

    let init = init_z_coin_light(mm, coin, electrums, lightwalletd_urls, Some(two_days_ago)).await;
    let new_init_result: RpcV2Response<InitTaskResult> = json::from_value(init).unwrap();

    // Loop until new activation status is obtained or timeout
    let s_activation_result = loop_until_status_ok(mm, &new_init_result.result, wait_until_ms(60000)).await;

    // let's check to make sure first activation starting height is different from current one
    assert_ne!(
        f_activation_result.first_sync_block.as_ref().unwrap().actual,
        s_activation_result.first_sync_block.as_ref().unwrap().actual
    );
    // let's check to make sure first activation starting height is greater than current one since we used date later
    // than current date
    assert!(
        f_activation_result.first_sync_block.unwrap().actual
            > s_activation_result.first_sync_block.as_ref().unwrap().actual
    );

    s_activation_result
}

async fn loop_until_status_ok(mm: &MarketMakerIt, init_result: &InitTaskResult, timeout: u64) -> ZCoinActivationResult {
    while now_ms() <= timeout {
        let status = init_z_coin_status(mm, init_result.task_id).await;
        println!("Status {}", json::to_string(&status).unwrap());
        let status: RpcV2Response<InitZcoinStatus> = json::from_value(status.clone()).unwrap();
        match status.result {
            InitZcoinStatus::Ok(res) => return res,
            InitZcoinStatus::Error(e) => panic!("Initialization error {:?}", e),
            _ => Timer::sleep(1.).await,
        }
    }

    panic!("Initialization timed out")
}

pub async fn enable_utxo_v2_electrum(
    mm: &MarketMakerIt,
    coin: &str,
    servers: Vec<Json>,
    timeout: u64,
) -> UtxoStandardActivationResult {
    let init = init_utxo_electrum(mm, coin, servers).await;
    let init: RpcV2Response<InitTaskResult> = json::from_value(init).unwrap();
    let timeout = wait_until_ms(timeout * 1000);

    loop {
        if now_ms() > timeout {
            panic!("{} initialization timed out", coin);
        }

        let status = init_utxo_status(mm, init.result.task_id).await;
        let status: RpcV2Response<InitUtxoStatus> = json::from_value(status).unwrap();
        log!("init_utxo_status: {:?}", status);
        match status.result {
            InitUtxoStatus::Ok(result) => break result,
            InitUtxoStatus::Error(e) => panic!("{} initialization error {:?}", coin, e),
            _ => Timer::sleep(1.).await,
        }
    }
}

pub async fn enable_coins_eth_electrum(
    mm: &MarketMakerIt,
    eth_urls: &[&str],
) -> HashMap<&'static str, CoinInitResponse> {
    let mut replies = HashMap::new();
    replies.insert("RICK", enable_electrum_json(mm, "RICK", false, rick_electrums()).await);
    replies.insert(
        "MORTY",
        enable_electrum_json(mm, "MORTY", false, morty_electrums()).await,
    );
    replies.insert("ETH", enable_native(mm, "ETH", eth_urls).await);
    replies.insert("JST", enable_native(mm, "JST", eth_urls).await);
    replies
}

pub fn addr_from_enable<'a>(enable_response: &'a HashMap<&str, CoinInitResponse>, coin: &str) -> &'a str {
    &enable_response.get(coin).unwrap().address
}

pub fn rmd160_from_passphrase(passphrase: &str) -> [u8; 20] {
    key_pair_from_seed(passphrase).unwrap().public().address_hash().take()
}
