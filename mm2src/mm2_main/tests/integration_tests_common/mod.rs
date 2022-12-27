use common::executor::Timer;
use common::log::LogLevel;
use common::{block_on, log, now_ms};
use crypto::privkey::key_pair_from_seed;
use mm2_main::mm2::{lp_main, LpMainParams};
use mm2_test_helpers::electrums::{morty_electrums, rick_electrums};
use mm2_test_helpers::for_tests::{enable_native as enable_native_impl, init_utxo_electrum, init_utxo_status,
                                  init_z_coin_light, init_z_coin_status, MarketMakerIt};
use mm2_test_helpers::structs::{CoinActivationResult, EnableElectrumResponse, InitTaskResult, InitUtxoStatus,
                                InitZcoinStatus, RpcV2Response, UtxoStandardActivationResult};
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
pub async fn enable_electrum(
    mm: &MarketMakerIt,
    coin: &str,
    tx_history: bool,
    urls: &[&str],
) -> EnableElectrumResponse {
    use mm2_test_helpers::for_tests::enable_electrum as enable_electrum_impl;

    let value = enable_electrum_impl(mm, coin, tx_history, urls).await;
    json::from_value(value).unwrap()
}

pub async fn enable_electrum_json(
    mm: &MarketMakerIt,
    coin: &str,
    tx_history: bool,
    servers: Vec<Json>,
) -> EnableElectrumResponse {
    use mm2_test_helpers::for_tests::enable_electrum_json as enable_electrum_impl;

    let value = enable_electrum_impl(mm, coin, tx_history, servers).await;
    json::from_value(value).unwrap()
}

pub async fn enable_native(mm: &MarketMakerIt, coin: &str, urls: &[&str]) -> EnableElectrumResponse {
    let value = enable_native_impl(mm, coin, urls).await;
    json::from_value(value).unwrap()
}

pub async fn enable_coins_rick_morty_electrum(mm: &MarketMakerIt) -> HashMap<&'static str, EnableElectrumResponse> {
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
) -> CoinActivationResult {
    let init = init_z_coin_light(mm, coin, electrums, lightwalletd_urls).await;
    let init: RpcV2Response<InitTaskResult> = json::from_value(init).unwrap();
    let timeout = now_ms() + 12000000;

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

pub async fn enable_utxo_v2_electrum(
    mm: &MarketMakerIt,
    coin: &str,
    servers: Vec<Json>,
    timeout: u64,
) -> UtxoStandardActivationResult {
    let init = init_utxo_electrum(mm, coin, servers).await;
    let init: RpcV2Response<InitTaskResult> = json::from_value(init).unwrap();
    let timeout = now_ms() + (timeout * 1000);

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
) -> HashMap<&'static str, EnableElectrumResponse> {
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

pub fn addr_from_enable<'a>(enable_response: &'a HashMap<&str, EnableElectrumResponse>, coin: &str) -> &'a str {
    &enable_response.get(coin).unwrap().address
}

pub fn rmd160_from_passphrase(passphrase: &str) -> [u8; 20] {
    key_pair_from_seed(passphrase).unwrap().public().address_hash().take()
}
