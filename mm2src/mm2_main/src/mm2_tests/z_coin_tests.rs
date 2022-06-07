use super::*;
use common::now_ms;
use mm2_test_helpers::for_tests::{init_withdraw, rick_conf, send_raw_transaction, withdraw_status, zombie_conf, RICK,
                                  ZOMBIE_ELECTRUMS, ZOMBIE_LIGHTWALLETD_URLS, ZOMBIE_TICKER};

const ZOMBIE_TEST_BALANCE_SEED: &str = "zombie test seed";
const ZOMBIE_TEST_WITHDRAW_SEED: &str = "zombie withdraw test seed";

async fn withdraw(mm: &MarketMakerIt, coin: &str, to: &str, amount: &str) -> TransactionDetails {
    let init = init_withdraw(mm, coin, to, amount).await;
    let init: RpcV2Response<InitTaskResult> = json::from_value(init).unwrap();
    let timeout = now_ms() + 150000;

    loop {
        if now_ms() > timeout {
            panic!("{} init_withdraw timed out", coin);
        }

        let status = withdraw_status(mm, init.result.task_id).await;
        println!("Withdraw status {}", json::to_string(&status).unwrap());
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

// ignored because it requires a long-running Zcoin initialization process
#[test]
#[ignore]
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

// ignored because it requires a long-running Zcoin initialization process
#[test]
#[ignore]
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

    let withdraw_res = block_on(withdraw(
        &mm,
        ZOMBIE_TICKER,
        "zs1hs0p406y5tntz6wlp7sc3qe4g6ycnnd46leeyt6nyxr42dfvf0dwjkhmjdveukem0x72kkx0tup",
        "0.1",
    ));
    println!("{:?}", withdraw_res);

    // withdrawing to myself, balance change is the fee
    assert_eq!(
        withdraw_res.my_balance_change,
        BigDecimal::from_str("-0.00001").unwrap()
    );

    let send_raw_tx = block_on(send_raw_transaction(&mm, ZOMBIE_TICKER, &withdraw_res.tx_hex));
    println!("{:?}", send_raw_tx);
}

// ignored because it requires a long-running Zcoin initialization process
#[test]
#[ignore]
fn trade_rick_zombie_light() {
    let coins = json!([zombie_conf(), rick_conf()]);
    let bob_passphrase = "RICK ZOMBIE BOB";
    let alice_passphrase = "RICK ZOMBIE ALICE";

    let mut mm_bob = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": bob_passphrase,
            "coins": coins,
            "rpc_password": "password",
            "i_am_seed": true,
        }),
        "password".into(),
        local_start!("bob"),
    )
    .unwrap();

    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!({"Bob log path: {}", mm_bob.log_path.display()});

    let zombie_activation = block_on(enable_z_coin_light(
        &mm_bob,
        ZOMBIE_TICKER,
        ZOMBIE_ELECTRUMS,
        ZOMBIE_LIGHTWALLETD_URLS,
        &blocks_cache_path(&mm_bob, bob_passphrase, ZOMBIE_TICKER),
    ));

    println!("Bob ZOMBIE activation {:?}", zombie_activation);

    let rick_activation = block_on(enable_electrum_json(&mm_bob, RICK, false, rick_electrums()));

    println!("Bob RICK activation {:?}", rick_activation);

    let rc = block_on(mm_bob.rpc(&json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": RICK,
        "rel": ZOMBIE_TICKER,
        "price": 1,
        "volume": "0.1"
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let mut mm_alice = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 8999,
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": alice_passphrase,
            "coins": coins,
            "seednodes": [mm_bob.my_seed_addr()],
            "rpc_password": "password",
            "skip_startup_checks": true,
        }),
        "password".into(),
        local_start!("alice"),
    )
    .unwrap();

    thread::sleep(Duration::from_secs(1));

    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!({"Alice log path: {}", mm_alice.log_path.display()});

    let zombie_activation = block_on(enable_z_coin_light(
        &mm_alice,
        ZOMBIE_TICKER,
        ZOMBIE_ELECTRUMS,
        ZOMBIE_LIGHTWALLETD_URLS,
        &blocks_cache_path(&mm_alice, alice_passphrase, ZOMBIE_TICKER),
    ));

    println!("Alice ZOMBIE activation {:?}", zombie_activation);

    let rick_activation = block_on(enable_electrum_json(&mm_alice, RICK, false, rick_electrums()));

    println!("Alice RICK activation {:?}", rick_activation);

    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": RICK,
        "rel": ZOMBIE_TICKER,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    thread::sleep(Duration::from_secs(1));

    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": RICK,
        "rel": ZOMBIE_TICKER,
        "volume": "0.1",
        "price": 1
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    let buy_json: Json = serde_json::from_str(&rc.1).unwrap();
    let uuid = buy_json["result"]["uuid"].as_str().unwrap().to_owned();

    block_on(mm_alice.wait_for_log(5., |log| log.contains("Entering the taker_swap_loop RICK/ZOMBIE"))).unwrap();

    block_on(mm_bob.wait_for_log(5., |log| log.contains("Entering the maker_swap_loop RICK/ZOMBIE"))).unwrap();

    block_on(mm_bob.wait_for_log(900., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))).unwrap();

    block_on(mm_alice.wait_for_log(900., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))).unwrap();
}
