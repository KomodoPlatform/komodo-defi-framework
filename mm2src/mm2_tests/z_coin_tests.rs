use super::*;
use coins::z_coin::ZcoinConsensusParams;
use common::for_tests::init_z_coin_light;

const ZOMBIE_TEST_SEED: &str = "zombie test seed";
const ZOMBIE_TICKER: &str = "ZOMBIE";
const ZOMBIE_ELECTRUMS: &[&str] = &["zombie.sirseven.me:10033"];
const ZOMBIE_LIGHTWALLETD_URLS: &[&str] = &["http://zombie.sirseven.me:443"];

async fn enable_z_coin_light(
    mm: &MarketMakerIt,
    coin: &str,
    electrums: &[&str],
    lightwalletd_urls: &[&str],
) -> ZcoinActivationResult {
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

#[test]
fn activate_z_coin_light() {
    let coins = json!([
        {"coin":"RICK","asset":"RICK","required_confirmations":0,"txversion":4,"overwintered":1,"protocol":{"type":"UTXO"}},
        {
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
        }
    ]);

    let mm = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": ZOMBIE_TEST_SEED,
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
    ));
    println!("{:?}", activation_result);
}
