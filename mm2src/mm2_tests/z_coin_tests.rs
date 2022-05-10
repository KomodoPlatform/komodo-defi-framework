use super::*;
use coins::z_coin::ZcoinConsensusParams;
use common::for_tests::{init_withdraw, init_z_coin_light, send_raw_transaction, withdraw_status};

const ZOMBIE_TEST_BALANCE_SEED: &str = "zombie test seed";
const ZOMBIE_TEST_WITHDRAW_SEED: &str = "zombie withdraw test seed";
const ZOMBIE_TICKER: &str = "ZOMBIE";
const ZOMBIE_ELECTRUMS: &[&str] = &["zombie.sirseven.me:10033"];
const ZOMBIE_LIGHTWALLETD_URLS: &[&str] = &["http://zombie.sirseven.me:443"];

fn zombie_conf() -> Json {
    json!({
        "coin":"ZOMBIE",
        "asset":"ZOMBIE",
        "txversion":4,
        "overwintered":1,
        "mm2":1,
        "protocol":{
            "type":"ZHTLC",
            "protocol_data": {
                "consensus_params": ZcoinConsensusParams::for_zombie()
            }
        },
        "required_confirmations":0
    })
}

fn blocks_cache_path(mm: &MarketMakerIt, seed: &str, coin: &str) -> PathBuf {
    let rmd = rmd160_from_passphrase(seed);
    let db_name = format!("{}_light_cache.db", coin);
    mm.folder.join("DB").join(hex::encode(rmd)).join(db_name)
}

async fn enable_z_coin_light(
    mm: &MarketMakerIt,
    coin: &str,
    electrums: &[&str],
    lightwalletd_urls: &[&str],
    blocks_cache_path: &dyn AsRef<Path>,
) -> ZcoinActivationResult {
    std::fs::copy("./mm2src/coins/test_cache_zombie.db", blocks_cache_path).unwrap();

    let init = init_z_coin_light(mm, coin, electrums, lightwalletd_urls).await;
    let init: RpcV2Response<InitTaskResult> = json::from_value(init).unwrap();
    let timeout = now_ms() + 120000;

    loop {
        if now_ms() > timeout {
            panic!("{} initialization timed out", coin);
        }

        let status = init_z_coin_status(mm, init.result.task_id).await;
        let status: RpcV2Response<InitZcoinStatus> = json::from_value(status).unwrap();
        if let InitZcoinStatus::Ready(rpc_result) = status.result {
            match rpc_result {
                MmRpcResult::Ok { result } => break result,
                MmRpcResult::Err(e) => panic!("{} initialization error {:?}", coin, e),
            }
        }
        Timer::sleep(1.).await;
    }
}

async fn withdraw(mm: &MarketMakerIt, coin: &str, to: &str, amount: &str) -> TransactionDetails {
    let init = init_withdraw(mm, coin, to, amount).await;
    let init: RpcV2Response<InitTaskResult> = json::from_value(init).unwrap();
    let timeout = now_ms() + 120000;

    loop {
        if now_ms() > timeout {
            panic!("{} init_withdraw timed out", coin);
        }

        let status = withdraw_status(mm, init.result.task_id).await;
        let status: RpcV2Response<WithdrawStatus> = json::from_value(status).unwrap();
        if let WithdrawStatus::Ready(rpc_result) = status.result {
            match rpc_result {
                MmRpcResult::Ok { result } => break result,
                MmRpcResult::Err(e) => panic!("{} withdraw error {:?}", coin, e),
            }
        }
        Timer::sleep(1.).await;
    }
}

#[test]
fn activate_z_coin_light() {
    let coins = json!([zombie_conf()]);

    let mm = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": ZOMBIE_TEST_BALANCE_SEED,
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

    let activation_result = block_on(enable_z_coin_light(
        &mm,
        ZOMBIE_TICKER,
        ZOMBIE_ELECTRUMS,
        ZOMBIE_LIGHTWALLETD_URLS,
        &blocks_cache_path(&mm, ZOMBIE_TEST_BALANCE_SEED, ZOMBIE_TICKER),
    ));

    let balance = match activation_result.wallet_balance {
        EnableCoinBalance::Iguana(iguana) => iguana,
        _ => panic!("Expected EnableCoinBalance::Iguana"),
    };
    assert_eq!(balance.balance.spendable, BigDecimal::from(1));
}

#[test]
fn withdraw_z_coin_light() {
    let coins = json!([zombie_conf()]);

    let mm = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": ZOMBIE_TEST_WITHDRAW_SEED,
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

    let activation_result = block_on(enable_z_coin_light(
        &mm,
        ZOMBIE_TICKER,
        ZOMBIE_ELECTRUMS,
        ZOMBIE_LIGHTWALLETD_URLS,
        &blocks_cache_path(&mm, ZOMBIE_TEST_WITHDRAW_SEED, ZOMBIE_TICKER),
    ));

    println!("{:?}", activation_result);

    let withdraw = block_on(withdraw(
        &mm,
        ZOMBIE_TICKER,
        "zs1hs0p406y5tntz6wlp7sc3qe4g6ycnnd46leeyt6nyxr42dfvf0dwjkhmjdveukem0x72kkx0tup",
        "0.1",
    ));
    println!("{:?}", withdraw);

    let send_raw_tx = block_on(send_raw_transaction(&mm, ZOMBIE_TICKER, &withdraw.tx_hex));
    println!("{:?}", send_raw_tx);
}
