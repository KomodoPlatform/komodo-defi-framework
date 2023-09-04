use crate::docker_tests::docker_tests_common::generate_utxo_coin_with_privkey;
use crate::integration_tests_common::*;
use crate::{fill_address, generate_utxo_coin_with_random_privkey, random_secp256k1_secret, rmd160_from_priv,
            utxo_coin_from_privkey};
use bitcrypto::dhash160;
use chain::OutPoint;
use coins::utxo::rpc_clients::UnspentInfo;
use coins::utxo::{GetUtxoListOps, UtxoCommonOps};
use coins::{ConfirmPaymentInput, FoundSwapTxSpend, MarketCoinOps, MmCoin, RefundPaymentArgs,
            SearchForSwapTxSpendInput, SendPaymentArgs, SpendPaymentArgs, SwapOps, TransactionEnum, WithdrawRequest};
use common::{block_on, now_sec_u32, wait_until_sec};
use crypto::privkey::key_pair_from_seed;
use futures01::Future;
use mm2_number::{BigDecimal, MmNumber};
use mm2_test_helpers::for_tests::{check_my_swap_status_amounts, eth_testnet_conf, get_locked_amount, kmd_conf,
                                  max_maker_vol, mm_dump, mycoin1_conf, mycoin_conf, set_price, start_swaps,
                                  MarketMakerIt, Mm2TestConf, ETH_DEV_NODES};
use mm2_test_helpers::{get_passphrase, structs::*};
use serde_json::Value as Json;
use std::collections::HashMap;
use std::env;
use std::thread;
use std::time::Duration;

#[test]
fn test_search_for_swap_tx_spend_native_was_refunded_taker() {
    let timeout = wait_until_sec(120); // timeout if test takes more than 120 seconds to run
    let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    let my_public_key = coin.my_public_key().unwrap();

    let time_lock = now_sec_u32() - 3600;
    let taker_payment_args = SendPaymentArgs {
        time_lock_duration: 0,
        time_lock,
        other_pubkey: my_public_key,
        secret_hash: &[0; 20],
        amount: 1u64.into(),
        swap_contract_address: &None,
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let tx = coin.send_taker_payment(taker_payment_args).wait().unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: tx.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();
    let maker_refunds_payment_args = RefundPaymentArgs {
        payment_tx: &tx.tx_hex(),
        time_lock,
        other_pubkey: my_public_key,
        secret_hash: &[0; 20],
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let refund_tx = block_on(coin.send_maker_refunds_payment(maker_refunds_payment_args)).unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: refund_tx.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: coin.my_public_key().unwrap(),
        secret_hash: &[0; 20],
        tx: &tx.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let found = block_on(coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();
    assert_eq!(FoundSwapTxSpend::Refunded(refund_tx), found);
}

#[test]
fn test_for_non_existent_tx_hex_utxo() {
    // This test shouldn't wait till timeout!
    let timeout = wait_until_sec(120);
    let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    // bad transaction hex
    let tx = hex::decode("0400008085202f8902bf17bf7d1daace52e08f732a6b8771743ca4b1cb765a187e72fd091a0aabfd52000000006a47304402203eaaa3c4da101240f80f9c5e9de716a22b1ec6d66080de6a0cca32011cd77223022040d9082b6242d6acf9a1a8e658779e1c655d708379862f235e8ba7b8ca4e69c6012102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ffffffffff023ca13c0e9e085dd13f481f193e8a3e8fd609020936e98b5587342d994f4d020000006b483045022100c0ba56adb8de923975052312467347d83238bd8d480ce66e8b709a7997373994022048507bcac921fdb2302fa5224ce86e41b7efc1a2e20ae63aa738dfa99b7be826012102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ffffffff0300e1f5050000000017a9141ee6d4c38a3c078eab87ad1a5e4b00f21259b10d87000000000000000016611400000000000000000000000000000000000000001b94d736000000001976a91405aab5342166f8594baf17a7d9bef5d56744332788ac2d08e35e000000000000000000000000000000").unwrap();
    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: tx,
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    let actual = coin.wait_for_confirmations(confirm_payment_input).wait().err().unwrap();
    assert!(actual.contains(
        "Tx d342ff9da528a2e262bddf2b6f9a27d1beb7aeb03f0fc8d9eac2987266447e44 was not found on chain after 10 tries"
    ));
}

#[test]
fn test_search_for_swap_tx_spend_native_was_refunded_maker() {
    let timeout = wait_until_sec(120); // timeout if test takes more than 120 seconds to run
    let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    let my_public_key = coin.my_public_key().unwrap();

    let time_lock = now_sec_u32() - 3600;
    let maker_payment_args = SendPaymentArgs {
        time_lock_duration: 0,
        time_lock,
        other_pubkey: my_public_key,
        secret_hash: &[0; 20],
        amount: 1u64.into(),
        swap_contract_address: &None,
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let tx = coin.send_maker_payment(maker_payment_args).wait().unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: tx.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();
    let maker_refunds_payment_args = RefundPaymentArgs {
        payment_tx: &tx.tx_hex(),
        time_lock,
        other_pubkey: my_public_key,
        secret_hash: &[0; 20],
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let refund_tx = block_on(coin.send_maker_refunds_payment(maker_refunds_payment_args)).unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: refund_tx.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: coin.my_public_key().unwrap(),
        secret_hash: &[0; 20],
        tx: &tx.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let found = block_on(coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();
    assert_eq!(FoundSwapTxSpend::Refunded(refund_tx), found);
}

#[test]
fn test_search_for_taker_swap_tx_spend_native_was_spent_by_maker() {
    let timeout = wait_until_sec(120); // timeout if test takes more than 120 seconds to run
    let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    let secret = [0; 32];
    let my_pubkey = coin.my_public_key().unwrap();

    let secret_hash = dhash160(&secret);
    let time_lock = now_sec_u32() - 3600;
    let taker_payment_args = SendPaymentArgs {
        time_lock_duration: 0,
        time_lock,
        other_pubkey: my_pubkey,
        secret_hash: secret_hash.as_slice(),
        amount: 1u64.into(),
        swap_contract_address: &None,
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let tx = coin.send_taker_payment(taker_payment_args).wait().unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: tx.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();
    let maker_spends_payment_args = SpendPaymentArgs {
        other_payment_tx: &tx.tx_hex(),
        time_lock,
        other_pubkey: my_pubkey,
        secret: &secret,
        secret_hash: secret_hash.as_slice(),
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let spend_tx = coin
        .send_maker_spends_taker_payment(maker_spends_payment_args)
        .wait()
        .unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: spend_tx.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: coin.my_public_key().unwrap(),
        secret_hash: &*dhash160(&secret),
        tx: &tx.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let found = block_on(coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();
    assert_eq!(FoundSwapTxSpend::Spent(spend_tx), found);
}

#[test]
fn test_search_for_maker_swap_tx_spend_native_was_spent_by_taker() {
    let timeout = wait_until_sec(120); // timeout if test takes more than 120 seconds to run
    let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    let secret = [0; 32];
    let my_pubkey = coin.my_public_key().unwrap();

    let time_lock = now_sec_u32() - 3600;
    let secret_hash = dhash160(&secret);
    let maker_payment_args = SendPaymentArgs {
        time_lock_duration: 0,
        time_lock,
        other_pubkey: my_pubkey,
        secret_hash: secret_hash.as_slice(),
        amount: 1u64.into(),
        swap_contract_address: &None,
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let tx = coin.send_maker_payment(maker_payment_args).wait().unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: tx.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();
    let taker_spends_payment_args = SpendPaymentArgs {
        other_payment_tx: &tx.tx_hex(),
        time_lock,
        other_pubkey: my_pubkey,
        secret: &secret,
        secret_hash: secret_hash.as_slice(),
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let spend_tx = coin
        .send_taker_spends_maker_payment(taker_spends_payment_args)
        .wait()
        .unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: spend_tx.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: coin.my_public_key().unwrap(),
        secret_hash: &*dhash160(&secret),
        tx: &tx.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let found = block_on(coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();
    assert_eq!(FoundSwapTxSpend::Spent(spend_tx), found);
}

#[test]
fn test_one_hundred_maker_payments_in_a_row_native() {
    let timeout = 30; // timeout if test takes more than 30 seconds to run
    let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
    let secret = [0; 32];
    let my_pubkey = coin.my_public_key().unwrap();

    let time_lock = now_sec_u32() - 3600;
    let mut unspents = vec![];
    let mut sent_tx = vec![];
    for i in 0..100 {
        let maker_payment_args = SendPaymentArgs {
            time_lock_duration: 0,
            time_lock: time_lock + i,
            other_pubkey: my_pubkey,
            secret_hash: &*dhash160(&secret),
            amount: 1.into(),
            swap_contract_address: &coin.swap_contract_address(),
            swap_unique_data: &[],
            payment_instructions: &None,
            watcher_reward: None,
            wait_for_confirmation_until: 0,
        };
        let tx = coin.send_maker_payment(maker_payment_args).wait().unwrap();
        if let TransactionEnum::UtxoTx(tx) = tx {
            unspents.push(UnspentInfo {
                outpoint: OutPoint {
                    hash: tx.hash(),
                    index: 2,
                },
                value: tx.outputs[2].value,
                height: None,
            });
            sent_tx.push(tx);
        }
    }

    let recently_sent = block_on(coin.as_ref().recently_spent_outpoints.lock());

    unspents = recently_sent
        .replace_spent_outputs_with_cache(unspents.into_iter().collect())
        .into_iter()
        .collect();

    let last_tx = sent_tx.last().unwrap();
    let expected_unspent = UnspentInfo {
        outpoint: OutPoint {
            hash: last_tx.hash(),
            index: 2,
        },
        value: last_tx.outputs[2].value,
        height: None,
    };
    assert_eq!(vec![expected_unspent], unspents);
}

// https://github.com/KomodoPlatform/atomicDEX-API/issues/554
#[test]
fn order_should_be_cancelled_when_entire_balance_is_withdrawn() {
    let (_ctx, _, priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);

    let mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var("BOB_TRADE_IP") .ok(),
            "rpcip": env::var("BOB_TRADE_IP") .ok(),
            "canbind": env::var("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": format!("0x{}", hex::encode(priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "volume": "999",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let json: Json = serde_json::from_str(&rc.1).unwrap();
    let bob_uuid = json["result"]["uuid"].as_str().unwrap().to_owned();

    log!("Get MYCOIN/MYCOIN1 orderbook");
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("orderbook {:?}", bob_orderbook);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

    let withdraw = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "withdraw",
        "coin": "MYCOIN",
        "max": true,
        "to": "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF",
    })))
    .unwrap();
    assert!(withdraw.0.is_success(), "!withdraw: {}", withdraw.1);

    let withdraw: Json = serde_json::from_str(&withdraw.1).unwrap();

    let send_raw = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "send_raw_transaction",
        "coin": "MYCOIN",
        "tx_hex": withdraw["tx_hex"],
    })))
    .unwrap();
    assert!(send_raw.0.is_success(), "!send_raw: {}", send_raw.1);

    thread::sleep(Duration::from_secs(32));

    log!("Get MYCOIN/MYCOIN1 orderbook");
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("orderbook {}", serde_json::to_string(&bob_orderbook).unwrap());
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 0, "MYCOIN/MYCOIN1 orderbook must have exactly 0 asks");

    log!("Get my orders");
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "my_orders",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!my_orders: {}", rc.1);
    let orders: Json = serde_json::from_str(&rc.1).unwrap();
    log!("my_orders {}", serde_json::to_string(&orders).unwrap());
    assert!(
        orders["result"]["maker_orders"].as_object().unwrap().is_empty(),
        "maker_orders must be empty"
    );

    let rmd160 = rmd160_from_priv(priv_key);
    let order_path = mm_bob.folder.join(format!(
        "DB/{}/ORDERS/MY/MAKER/{}.json",
        hex::encode(rmd160.take()),
        bob_uuid,
    ));
    log!("Order path {}", order_path.display());
    assert!(!order_path.exists());
    block_on(mm_bob.stop()).unwrap();
}

#[test]
fn order_should_be_updated_when_balance_is_decreased_alice_subscribes_after_update() {
    let (_ctx, _, priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);

    let mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var("BOB_TRADE_IP") .ok(),
            "rpcip": env::var("BOB_TRADE_IP") .ok(),
            "canbind": env::var("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": format!("0x{}", hex::encode(priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": "alice passphrase",
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));

    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "volume": "999",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    log!("Get MYCOIN/MYCOIN1 orderbook");
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("orderbook {:?}", bob_orderbook);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

    let withdraw = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "withdraw",
        "coin": "MYCOIN",
        "amount": "499.99998",
        "to": "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF",
    })))
    .unwrap();
    assert!(withdraw.0.is_success(), "!withdraw: {}", withdraw.1);

    let withdraw: Json = serde_json::from_str(&withdraw.1).unwrap();

    let send_raw = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "send_raw_transaction",
        "coin": "MYCOIN",
        "tx_hex": withdraw["tx_hex"],
    })))
    .unwrap();
    assert!(send_raw.0.is_success(), "!send_raw: {}", send_raw.1);

    thread::sleep(Duration::from_secs(32));

    log!("Get MYCOIN/MYCOIN1 orderbook on Bob side");
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("orderbook {}", serde_json::to_string(&bob_orderbook).unwrap());
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

    let order_volume = asks[0]["maxvolume"].as_str().unwrap();
    assert_eq!("500", order_volume);

    log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("orderbook {}", serde_json::to_string(&alice_orderbook).unwrap());
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Alice MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

    let order_volume = asks[0]["maxvolume"].as_str().unwrap();
    assert_eq!("500", order_volume);

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
fn order_should_be_updated_when_balance_is_decreased_alice_subscribes_before_update() {
    let (_ctx, _, priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "myipaddr": env::var("BOB_TRADE_IP") .ok(),
            "rpcip": env::var("BOB_TRADE_IP") .ok(),
            "canbind": env::var("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
            "passphrase": format!("0x{}", hex::encode(priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": "alice passphrase",
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));

    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "volume": "999",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    log!("Get MYCOIN/MYCOIN1 orderbook");
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("orderbook {:?}", bob_orderbook);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

    thread::sleep(Duration::from_secs(2));
    log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("orderbook {}", serde_json::to_string(&alice_orderbook).unwrap());
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Alice MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

    let withdraw = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "withdraw",
        "coin": "MYCOIN",
        "amount": "499.99998",
        "to": "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF",
    })))
    .unwrap();
    assert!(withdraw.0.is_success(), "!withdraw: {}", withdraw.1);

    let withdraw: Json = serde_json::from_str(&withdraw.1).unwrap();

    let send_raw = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "send_raw_transaction",
        "coin": "MYCOIN",
        "tx_hex": withdraw["tx_hex"],
    })))
    .unwrap();
    assert!(send_raw.0.is_success(), "!send_raw: {}", send_raw.1);

    thread::sleep(Duration::from_secs(32));

    log!("Get MYCOIN/MYCOIN1 orderbook on Bob side");
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("orderbook {}", serde_json::to_string(&bob_orderbook).unwrap());
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

    let order_volume = asks[0]["maxvolume"].as_str().unwrap();
    assert_eq!("500", order_volume);

    log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("orderbook {}", serde_json::to_string(&alice_orderbook).unwrap());
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Alice MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

    let order_volume = asks[0]["maxvolume"].as_str().unwrap();
    assert_eq!("500", order_volume);

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
fn test_order_should_be_updated_when_matched_partially() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 2000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mut mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mut mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));

    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "volume": "1000",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "volume": "500",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
    block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1"))).unwrap();

    log!("Get MYCOIN/MYCOIN1 orderbook on Bob side");
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("orderbook {}", serde_json::to_string(&bob_orderbook).unwrap());
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

    let order_volume = asks[0]["maxvolume"].as_str().unwrap();
    assert_eq!("500", order_volume);

    log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("orderbook {}", serde_json::to_string(&alice_orderbook).unwrap());
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Alice MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

// https://github.com/KomodoPlatform/atomicDEX-API/issues/471
#[test]
fn test_match_and_trade_setprice_max() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mut mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mut mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let json: Json = serde_json::from_str(&rc.1).unwrap();
    let bob_uuid = json["result"]["uuid"].as_str().unwrap().to_owned();

    log!("Get MYCOIN/MYCOIN1 orderbook");
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("orderbook {:?}", bob_orderbook);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");
    assert_eq!(asks[0]["maxvolume"], Json::from("999.99999"));

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "volume": "999.99999",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
    block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1"))).unwrap();

    thread::sleep(Duration::from_secs(3));

    let rmd160 = rmd160_from_priv(bob_priv_key);
    let order_path = mm_bob.folder.join(format!(
        "DB/{}/ORDERS/MY/MAKER/{}.json",
        hex::encode(rmd160.take()),
        bob_uuid,
    ));
    log!("Order path {}", order_path.display());
    assert!(!order_path.exists());
    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/888
fn test_max_taker_vol_swap() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 50.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mut mm_bob = block_on(MarketMakerIt::start_with_envs(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
        &[("MYCOIN_FEE_DISCOUNT", "")],
    ))
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    block_on(mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))).unwrap();

    let mut mm_alice = block_on(MarketMakerIt::start_with_envs(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
        &[("MYCOIN_FEE_DISCOUNT", "")],
    ))
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
    block_on(mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))).unwrap();

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    let price = MmNumber::from((100, 1620));
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": price,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "MYCOIN1",
        "rel": "MYCOIN",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);
    log!("{}", rc.1);
    thread::sleep(Duration::from_secs(3));

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "max_taker_vol",
        "coin": "MYCOIN1",
        "trade_with": "MYCOIN",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!max_taker_vol: {}", rc.1);
    let vol: MaxTakerVolResponse = serde_json::from_str(&rc.1).unwrap();
    let expected_vol = MmNumber::from((647499741, 12965000));

    let actual_vol = MmNumber::from(vol.result.clone());
    println!("actual vol {}", actual_vol.to_decimal());
    println!("expected vol {}", expected_vol.to_decimal());

    assert_eq!(expected_vol, actual_vol);

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "sell",
        "base": "MYCOIN1",
        "rel": "MYCOIN",
        "price": "16",
        "volume": vol.result,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!sell: {}", rc.1);
    let sell_res: BuyOrSellRpcResult = serde_json::from_str(&rc.1).unwrap();

    block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
    block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1"))).unwrap();

    thread::sleep(Duration::from_secs(3));

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "my_swap_status",
        "params": {
            "uuid": sell_res.result.uuid
        }
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!my_swap_status: {}", rc.1);

    let status_response: Json = serde_json::from_str(&rc.1).unwrap();
    let events_array = status_response["result"]["events"].as_array().unwrap();
    let first_event_type = events_array[0]["event"]["type"].as_str().unwrap();
    assert_eq!("Started", first_event_type);
    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
fn test_buy_when_coins_locked_by_other_swap() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mut mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mut mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        // the result of equation x + x / 777 + 0.00002 = 1
        "volume": {
            "numer":"77698446",
            "denom":"77800000"
        },
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
    block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
    // TODO when buy call is made immediately swap might be not put into swap ctx yet so locked
    // amount returns 0
    thread::sleep(Duration::from_secs(6));

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        // it is slightly more than previous volume so it should fail
        // because the total sum of used funds will be slightly more than available 2
        "volume": {
            "numer":"77698447",
            "denom":"77800000"
        },
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "buy success, but should fail: {}", rc.1);
    assert!(rc.1.contains("Not enough MYCOIN1 for swap"), "{}", rc.1);
    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
fn test_sell_when_coins_locked_by_other_swap() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mut mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mut mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "sell",
        "base": "MYCOIN1",
        "rel": "MYCOIN",
        "price": 1,
        // the result of equation x + x / 777 + 0.00002 = 1
        "volume": {
            "numer":"77698446",
            "denom":"77800000"
        },
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!sell: {}", rc.1);

    block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
    block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
    // TODO when sell call is made immediately swap might be not put into swap ctx yet so locked
    // amount returns 0
    thread::sleep(Duration::from_secs(6));

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "sell",
        "base": "MYCOIN1",
        "rel": "MYCOIN",
        "price": 1,
        // it is slightly more than previous volume so it should fail
        // because the total sum of used funds will be slightly more than available 2
        "volume": {
            "numer":"77698447",
            "denom":"77800000"
        },
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "sell success, but should fail: {}", rc.1);
    assert!(rc.1.contains("Not enough MYCOIN1 for swap"), "{}", rc.1);
    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
fn test_buy_max() {
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 1.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        // the result of equation x + x / 777 + 0.00002 = 1
        "volume": {
            "numer":"77698446",
            "denom":"77800000"
        },
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        // it is slightly more than previous volume so it should fail
        "volume": {
            "numer":"77698447",
            "denom":"77800000"
        },
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "buy success, but should fail: {}", rc.1);
    // assert! (rc.1.contains("MYCOIN1 balance 1 is too low"));
    block_on(mm_alice.stop()).unwrap();
}

#[test]
fn test_maker_trade_preimage() {
    let priv_key = random_secp256k1_secret();

    let (_ctx, mycoin) = utxo_coin_from_privkey("MYCOIN", priv_key);
    let my_address = mycoin.my_address().expect("!my_address");
    fill_address(&mycoin, &my_address, 10.into(), 30);

    let (_ctx, mycoin1) = utxo_coin_from_privkey("MYCOIN1", priv_key);
    let my_address = mycoin1.my_address().expect("!my_address");
    fill_address(&mycoin1, &my_address, 20.into(), 30);

    let coins = json!([mycoin_conf(1000), mycoin1_conf(2000)]);
    let mm = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);

    log!("{:?}", block_on(enable_native(&mm, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm, "MYCOIN", &[], None)));

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "setprice",
            "price": 1,
            "max": true,
        },
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);
    let base_coin_fee = TradeFeeForTest::new("MYCOIN", "0.00001", false);
    let rel_coin_fee = TradeFeeForTest::new("MYCOIN1", "0.00002", true);
    let volume = MmNumber::from("9.99999");

    let my_coin_total = TotalTradeFeeForTest::new("MYCOIN", "0.00001", "0.00001");
    let my_coin1_total = TotalTradeFeeForTest::new("MYCOIN1", "0.00002", "0");

    let expected = TradePreimageResult::MakerPreimage(MakerPreimage {
        base_coin_fee,
        rel_coin_fee,
        volume: Some(volume.to_decimal()),
        volume_rat: Some(volume.to_ratio()),
        volume_fraction: Some(volume.to_fraction()),
        total_fees: vec![my_coin_total, my_coin1_total],
    });

    let mut actual: RpcSuccessResponse<TradePreimageResult> = serde_json::from_str(&rc.1).unwrap();
    actual.result.sort_total_fees();
    assert_eq!(expected, actual.result);

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN1",
            "rel": "MYCOIN",
            "swap_method": "setprice",
            "price": 1,
            "max": true,
        },
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);
    let mut actual: RpcSuccessResponse<TradePreimageResult> = serde_json::from_str(&rc.1).unwrap();
    actual.result.sort_total_fees();

    let base_coin_fee = TradeFeeForTest::new("MYCOIN1", "0.00002", false);
    let rel_coin_fee = TradeFeeForTest::new("MYCOIN", "0.00001", true);
    let volume = MmNumber::from("19.99998");

    let my_coin_total = TotalTradeFeeForTest::new("MYCOIN", "0.00001", "0");
    let my_coin1_total = TotalTradeFeeForTest::new("MYCOIN1", "0.00002", "0.00002");
    let expected = TradePreimageResult::MakerPreimage(MakerPreimage {
        base_coin_fee,
        rel_coin_fee,
        volume: Some(volume.to_decimal()),
        volume_rat: Some(volume.to_ratio()),
        volume_fraction: Some(volume.to_fraction()),
        total_fees: vec![my_coin_total, my_coin1_total],
    });

    actual.result.sort_total_fees();
    assert_eq!(expected, actual.result);

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN1",
            "rel": "MYCOIN",
            "swap_method": "setprice",
            "price": 1,
            "volume": "19.99998",
        },
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);
    let mut actual: RpcSuccessResponse<TradePreimageResult> = serde_json::from_str(&rc.1).unwrap();
    actual.result.sort_total_fees();

    let base_coin_fee = TradeFeeForTest::new("MYCOIN1", "0.00002", false);
    let rel_coin_fee = TradeFeeForTest::new("MYCOIN", "0.00001", true);

    let total_my_coin = TotalTradeFeeForTest::new("MYCOIN", "0.00001", "0");
    let total_my_coin1 = TotalTradeFeeForTest::new("MYCOIN1", "0.00002", "0.00002");

    let expected = TradePreimageResult::MakerPreimage(MakerPreimage {
        base_coin_fee,
        rel_coin_fee,
        volume: None,
        volume_rat: None,
        volume_fraction: None,
        total_fees: vec![total_my_coin, total_my_coin1],
    });

    actual.result.sort_total_fees();
    assert_eq!(expected, actual.result);
}

#[test]
fn test_taker_trade_preimage() {
    let priv_key = random_secp256k1_secret();

    let (_ctx, mycoin) = utxo_coin_from_privkey("MYCOIN", priv_key);
    let my_address = mycoin.my_address().expect("!my_address");
    fill_address(&mycoin, &my_address, 10.into(), 30);

    let (_ctx, mycoin1) = utxo_coin_from_privkey("MYCOIN1", priv_key);
    let my_address = mycoin1.my_address().expect("!my_address");
    fill_address(&mycoin1, &my_address, 20.into(), 30);

    let coins = json!([mycoin_conf(1000), mycoin1_conf(2000)]);
    let mm = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);

    log!("{:?}", block_on(enable_native(&mm, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm, "MYCOIN", &[], None)));

    // `max` field is not supported for `buy/sell` swap methods
    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "sell",
            "max": true,
            "price": 1,
        },
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);

    let actual: RpcErrorResponse<trade_preimage_error::InvalidParam> = serde_json::from_str(&rc.1).unwrap();
    assert_eq!(actual.error_type, "InvalidParam", "Unexpected error_type: {}", rc.1);
    let expected = trade_preimage_error::InvalidParam {
        param: "max".to_owned(),
        reason: "'max' cannot be used with 'sell' or 'buy' method".to_owned(),
    };
    assert_eq!(actual.error_data, Some(expected));

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "sell",
            "volume": "7.77",
            "price": "2",
        },
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);

    let mut actual: RpcSuccessResponse<TradePreimageResult> = serde_json::from_str(&rc.1).unwrap();
    actual.result.sort_total_fees();

    let base_coin_fee = TradeFeeForTest::new("MYCOIN", "0.00001", false);
    let rel_coin_fee = TradeFeeForTest::new("MYCOIN1", "0.00002", true);
    let taker_fee = TradeFeeForTest::new("MYCOIN", "0.01", false);
    let fee_to_send_taker_fee = TradeFeeForTest::new("MYCOIN", "0.00001", false);

    let my_coin_total_fee = TotalTradeFeeForTest::new("MYCOIN", "0.01002", "0.01002");
    let my_coin1_total_fee = TotalTradeFeeForTest::new("MYCOIN1", "0.00002", "0");

    let expected = TradePreimageResult::TakerPreimage(TakerPreimage {
        base_coin_fee,
        rel_coin_fee,
        taker_fee,
        fee_to_send_taker_fee,
        total_fees: vec![my_coin_total_fee, my_coin1_total_fee],
    });
    assert_eq!(expected, actual.result);

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "buy",
            "volume": "7.77",
            "price": "2",
        },
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);
    let mut actual: RpcSuccessResponse<TradePreimageResult> = serde_json::from_str(&rc.1).unwrap();
    actual.result.sort_total_fees();

    let base_coin_fee = TradeFeeForTest::new("MYCOIN", "0.00001", true);
    let rel_coin_fee = TradeFeeForTest::new("MYCOIN1", "0.00002", false);
    let taker_fee = TradeFeeForTest::new("MYCOIN1", "0.02", false);
    let fee_to_send_taker_fee = TradeFeeForTest::new("MYCOIN1", "0.00002", false);

    let my_coin_total_fee = TotalTradeFeeForTest::new("MYCOIN", "0.00001", "0");
    let my_coin1_total_fee = TotalTradeFeeForTest::new("MYCOIN1", "0.02004", "0.02004");

    let expected = TradePreimageResult::TakerPreimage(TakerPreimage {
        base_coin_fee,
        rel_coin_fee,
        taker_fee,
        fee_to_send_taker_fee,
        total_fees: vec![my_coin_total_fee, my_coin1_total_fee],
    });
    assert_eq!(expected, actual.result);
}

#[test]
fn test_trade_preimage_not_sufficient_balance() {
    #[track_caller]
    fn expect_not_sufficient_balance(
        res: &str,
        available: BigDecimal,
        required: BigDecimal,
        locked_by_swaps: Option<BigDecimal>,
    ) {
        let actual: RpcErrorResponse<trade_preimage_error::NotSufficientBalance> = serde_json::from_str(res).unwrap();
        assert_eq!(actual.error_type, "NotSufficientBalance");
        let expected = trade_preimage_error::NotSufficientBalance {
            coin: "MYCOIN".to_owned(),
            available,
            required,
            locked_by_swaps,
        };
        assert_eq!(actual.error_data, Some(expected));
    }

    let priv_key = random_secp256k1_secret();
    let fill_balance_functor = |amount: BigDecimal| {
        let (_ctx, mycoin) = utxo_coin_from_privkey("MYCOIN", priv_key);
        let my_address = mycoin.my_address().expect("!my_address");
        fill_address(&mycoin, &my_address, amount, 30);
    };

    let coins = json!([mycoin_conf(1000), mycoin1_conf(2000)]);
    let mm = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);

    log!("{:?}", block_on(enable_native(&mm, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm, "MYCOIN", &[], None)));

    fill_balance_functor(MmNumber::from("0.000015").to_decimal());
    // Try sell the max amount with the zero balance.
    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "setprice",
            "price": 1,
            "max": true,
        },
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
    let available = MmNumber::from("0.000015").to_decimal();
    // Required at least 0.00002 MYCOIN to pay the transaction_fee(0.00001) and to send a value not less than dust(0.00001).
    let required = MmNumber::from("0.00002").to_decimal();
    expect_not_sufficient_balance(&rc.1, available, required, None);

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "setprice",
            "price": 1,
            "volume": 0.1,
        },
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
    // Required 0.00001 MYCOIN to pay the transaction fee and the specified 0.1 volume.
    let available = MmNumber::from("0.000015").to_decimal();
    let required = MmNumber::from("0.10001").to_decimal();
    expect_not_sufficient_balance(&rc.1, available, required, None);

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "setprice",
            "price": 1,
            "max": true,
        },
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
    // balance(0.000015)
    let available = MmNumber::from("0.000015").to_decimal();
    // balance(0.000015) + transaction_fee(0.00001)
    let required = MmNumber::from("0.00002").to_decimal();
    expect_not_sufficient_balance(&rc.1, available, required, None);

    fill_balance_functor(MmNumber::from("7.770085").to_decimal());
    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "sell",
            "price": 1,
            "volume": 7.77,
        },
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
    let available = MmNumber::from("7.7701").to_decimal();
    // `required = volume + fee_to_send_taker_payment + dex_fee + fee_to_send_dex_fee`,
    // where `volume = 7.77`, `fee_to_send_taker_payment = fee_to_send_dex_fee = 0.00001`, `dex_fee = 0.01`.
    // Please note `dex_fee = 7.77 / 777` with dex_fee = 0.01
    // required = 7.77 + 0.01 (dex_fee) + (0.0001 * 2) = 7.78002
    let required = MmNumber::from("7.78002");
    expect_not_sufficient_balance(&rc.1, available, required.to_decimal(), Some(BigDecimal::from(0)));
}

/// This test ensures that `trade_preimage` will not succeed on input that will fail on `buy/sell/setprice`.
/// https://github.com/KomodoPlatform/atomicDEX-API/issues/902
#[test]
fn test_trade_preimage_additional_validation() {
    let priv_key = random_secp256k1_secret();

    let (_ctx, mycoin1) = utxo_coin_from_privkey("MYCOIN1", priv_key);
    let my_address = mycoin1.my_address().expect("!my_address");
    fill_address(&mycoin1, &my_address, 20.into(), 30);

    let (_ctx, mycoin) = utxo_coin_from_privkey("MYCOIN", priv_key);
    let my_address = mycoin.my_address().expect("!my_address");
    fill_address(&mycoin, &my_address, 10.into(), 30);

    let coins = json!([mycoin_conf(1000), mycoin1_conf(2000)]);

    let mm = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);

    log!("{:?}", block_on(enable_native(&mm, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm, "MYCOIN", &[], None)));

    // Price is too low
    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "setprice",
            "price": 0,
            "volume": 0.1,
        },
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
    let actual: RpcErrorResponse<trade_preimage_error::PriceTooLow> = serde_json::from_str(&rc.1).unwrap();
    assert_eq!(actual.error_type, "PriceTooLow");
    // currently the minimum price is 0.00000001
    let price_threshold = BigDecimal::from(1) / BigDecimal::from(100_000_000);
    let expected = trade_preimage_error::PriceTooLow {
        price: BigDecimal::from(0),
        threshold: price_threshold,
    };
    assert_eq!(actual.error_data, Some(expected));

    // volume 0.00001 is too low, min trading volume 0.0001
    let low_volume = BigDecimal::from(1) / BigDecimal::from(100_000);

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "setprice",
            "price": 1,
            "volume": low_volume,
        },
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
    let actual: RpcErrorResponse<trade_preimage_error::VolumeTooLow> = serde_json::from_str(&rc.1).unwrap();
    assert_eq!(actual.error_type, "VolumeTooLow");
    // Min MYCOIN trading volume is 0.0001.
    let volume_threshold = BigDecimal::from(1) / BigDecimal::from(10_000);
    let expected = trade_preimage_error::VolumeTooLow {
        coin: "MYCOIN".to_owned(),
        volume: low_volume.clone(),
        threshold: volume_threshold,
    };
    assert_eq!(actual.error_data, Some(expected));

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "sell",
            "price": 1,
            "volume": low_volume,
        },
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
    let actual: RpcErrorResponse<trade_preimage_error::VolumeTooLow> = serde_json::from_str(&rc.1).unwrap();
    assert_eq!(actual.error_type, "VolumeTooLow");
    // Min MYCOIN trading volume is 0.0001.
    let volume_threshold = BigDecimal::from(1) / BigDecimal::from(10_000);
    let expected = trade_preimage_error::VolumeTooLow {
        coin: "MYCOIN".to_owned(),
        volume: low_volume,
        threshold: volume_threshold,
    };
    assert_eq!(actual.error_data, Some(expected));

    // rel volume is too low
    // Min MYCOIN trading volume is 0.0001.
    let volume = BigDecimal::from(1) / BigDecimal::from(10_000);
    let low_price = BigDecimal::from(1) / BigDecimal::from(10);
    // Min MYCOIN1 trading volume is 0.0001, but the actual volume is 0.00001
    let low_rel_volume = &volume * &low_price;
    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "mmrpc": "2.0",
        "method": "trade_preimage",
        "params": {
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "sell",
            "price": low_price,
            "volume": volume,
        },
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
    let actual: RpcErrorResponse<trade_preimage_error::VolumeTooLow> = serde_json::from_str(&rc.1).unwrap();
    assert_eq!(actual.error_type, "VolumeTooLow");
    // Min MYCOIN1 trading volume is 0.0001.
    let volume_threshold = BigDecimal::from(1) / BigDecimal::from(10_000);
    let expected = trade_preimage_error::VolumeTooLow {
        coin: "MYCOIN1".to_owned(),
        volume: low_rel_volume,
        threshold: volume_threshold,
    };
    assert_eq!(actual.error_data, Some(expected));
}

#[test]
fn test_trade_preimage_legacy() {
    let priv_key = random_secp256k1_secret();
    let (_ctx, mycoin) = utxo_coin_from_privkey("MYCOIN", priv_key);
    let my_address = mycoin.my_address().expect("!my_address");
    fill_address(&mycoin, &my_address, 10.into(), 30);
    let (_ctx, mycoin1) = utxo_coin_from_privkey("MYCOIN1", priv_key);
    let my_address = mycoin1.my_address().expect("!my_address");
    fill_address(&mycoin1, &my_address, 20.into(), 30);

    let coins = json!([mycoin_conf(1000), mycoin1_conf(2000)]);
    let mm = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);

    log!("{:?}", block_on(enable_native(&mm, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm, "MYCOIN", &[], None)));

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "trade_preimage",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "swap_method": "setprice",
        "max": true,
        "price": "1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);
    let _: TradePreimageResponse = serde_json::from_str(&rc.1).unwrap();

    // vvv test a taker method vvv

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "trade_preimage",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "swap_method": "sell",
        "volume": "7.77",
        "price": "2",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);
    let _: TradePreimageResponse = serde_json::from_str(&rc.1).unwrap();

    // vvv test the error response vvv

    // `max` field is not supported for `buy/sell` swap methods
    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "trade_preimage",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "swap_method": "sell",
        "max": true,
        "price": "1",
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
    assert!(rc
        .1
        .contains("Incorrect use of the 'max' parameter: 'max' cannot be used with 'sell' or 'buy' method"));
}

#[test]
fn test_get_max_taker_vol() {
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 1.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "max_taker_vol",
        "coin": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!max_taker_vol: {}", rc.1);
    let json: MaxTakerVolResponse = serde_json::from_str(&rc.1).unwrap();
    // the result of equation `max_vol + max_vol / 777 + 0.00002 = 1`
    // derived from `max_vol = balance - locked - trade_fee - fee_to_send_taker_fee - dex_fee(max_vol)`
    // where balance = 1, locked = 0, trade_fee = fee_to_send_taker_fee = 0.00001, dex_fee = max_vol / 777
    let expected = MmNumber::from((38849223, 38900000)).to_fraction();
    assert_eq!(json.result, expected);
    assert_eq!(json.coin, "MYCOIN1");

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "sell",
        "base": "MYCOIN1",
        "rel": "MYCOIN",
        "price": 1,
        "volume": json.result,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!sell: {}", rc.1);

    block_on(mm_alice.stop()).unwrap();
}

// https://github.com/KomodoPlatform/atomicDEX-API/issues/733
#[test]
fn test_get_max_taker_vol_dex_fee_threshold() {
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", "0.05328455".parse().unwrap());
    let coins = json!([mycoin_conf(10000), mycoin1_conf(10000)]);
    let mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "max_taker_vol",
        "coin": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!max_taker_vol: {}", rc.1);
    let json: Json = serde_json::from_str(&rc.1).unwrap();
    // the result of equation x + 0.0001 (dex fee) + 0.0002 (miner fee * 2) = 0.05328455
    assert_eq!(json["result"]["numer"], Json::from("1059691"));
    assert_eq!(json["result"]["denom"], Json::from("20000000"));

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "sell",
        "base": "MYCOIN1",
        "rel": "MYCOIN",
        "price": 1,
        "volume": {
            "numer": json["result"]["numer"],
            "denom": json["result"]["denom"],
        }
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!sell: {}", rc.1);

    block_on(mm_alice.stop()).unwrap();
}

/// Test if the `max_taker_vol` cannot return a volume less than the coin's dust.
/// The minimum required balance for trading can be obtained by solving the equation:
/// `volume + taker_fee + trade_fee + fee_to_send_taker_fee = x`.
/// Let `dust = 0.000728` like for Qtum, `trade_fee = 0.0001`, `fee_to_send_taker_fee = 0.0001` and `taker_fee` is the `0.000728` threshold,
/// therefore to find a minimum required balance, we should pass the `dust` as the `volume` into the equation above:
/// `2 * 0.000728 + 0.0002 = x`, so `x = 0.001656`
#[test]
fn test_get_max_taker_vol_dust_threshold() {
    // first, try to test with the balance slightly less than required
    let (_ctx, coin, priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", "0.001656".parse().unwrap());
    let coins = json!([
    mycoin_conf(10000),
    {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":10000,"protocol":{"type":"UTXO"},"dust":72800}
    ]);
    let mm = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm.log_path);

    log!("{:?}", block_on(enable_native(&mm, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm, "MYCOIN", &[], None)));

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "max_taker_vol",
        "coin": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!max_taker_vol {}", rc.1);
    let json: Json = serde_json::from_str(&rc.1).unwrap();
    let result: MmNumber = serde_json::from_value(json["result"].clone()).unwrap();
    assert!(result.is_zero());

    fill_address(&coin, &coin.my_address().unwrap(), "0.00001".parse().unwrap(), 30);

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "max_taker_vol",
        "coin": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!max_taker_vol: {}", rc.1);
    let json: Json = serde_json::from_str(&rc.1).unwrap();
    // the result of equation x + 0.000728 (dex fee) + 0.0002 (miner fee * 2) = 0.001666
    assert_eq!(json["result"]["numer"], Json::from("369"));
    assert_eq!(json["result"]["denom"], Json::from("500000"));

    block_on(mm.stop()).unwrap();
}

#[test]
fn test_get_max_taker_vol_with_kmd() {
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 1.into());
    let coins = json!([mycoin_conf(10000), mycoin1_conf(10000), kmd_conf(10000)]);
    let mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    let electrum = block_on(enable_electrum(
        &mm_alice,
        "KMD",
        false,
        &[
            "electrum1.cipig.net:10001",
            "electrum2.cipig.net:10001",
            "electrum3.cipig.net:10001",
        ],
        None,
    ));
    log!("{:?}", electrum);
    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "max_taker_vol",
        "coin": "MYCOIN1",
        "trade_with": "KMD",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!max_taker_vol: {}", rc.1);
    let json: Json = serde_json::from_str(&rc.1).unwrap();
    // the result of equation x + x * 9 / 7770 + 0.0002 = 1
    assert_eq!(json["result"]["numer"], Json::from("1294741"));
    assert_eq!(json["result"]["denom"], Json::from("1296500"));

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "sell",
        "base": "MYCOIN1",
        "rel": "KMD",
        "price": 1,
        "volume": {
            "numer": json["result"]["numer"],
            "denom": json["result"]["denom"],
        }
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!sell: {}", rc.1);

    block_on(mm_alice.stop()).unwrap();
}

#[test]
fn test_get_max_maker_vol() {
    let (_ctx, _, priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 1.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let conf = Mm2TestConf::seednode(&format!("0x{}", hex::encode(priv_key)), &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);

    log!("{:?}", block_on(enable_native(&mm, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm, "MYCOIN1", &[], None)));

    // 1 - tx_fee
    let expected_volume = MmNumber::from("0.99999");
    let expected = MaxMakerVolResponse {
        coin: "MYCOIN1".to_string(),
        volume: MmNumberMultiRepr::from(expected_volume.clone()),
        balance: MmNumberMultiRepr::from(1),
        locked_by_swaps: MmNumberMultiRepr::from(0),
    };
    let actual = block_on(max_maker_vol(&mm, "MYCOIN1")).unwrap::<MaxMakerVolResponse>();
    assert_eq!(actual, expected);

    let res = block_on(set_price(&mm, "MYCOIN1", "MYCOIN", "1", "0", true));
    assert_eq!(res.result.max_base_vol, expected_volume.to_decimal());
}

#[test]
fn test_get_max_maker_vol_error() {
    let priv_key = random_secp256k1_secret();
    let coins = json!([mycoin_conf(1000)]);
    let conf = Mm2TestConf::seednode(&format!("0x{}", hex::encode(priv_key)), &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();
    let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);

    log!("{:?}", block_on(enable_native(&mm, "MYCOIN", &[], None)));

    let actual_error = block_on(max_maker_vol(&mm, "MYCOIN")).unwrap_err::<max_maker_vol_error::NotSufficientBalance>();
    let expected_error = max_maker_vol_error::NotSufficientBalance {
        coin: "MYCOIN".to_owned(),
        available: 0.into(),
        // tx_fee
        required: BigDecimal::from(1000) / BigDecimal::from(100_000_000),
        locked_by_swaps: None,
    };
    assert_eq!(actual_error.error_type, "NotSufficientBalance");
    assert_eq!(actual_error.error_data, Some(expected_error));
}

#[test]
fn test_set_price_max() {
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        // the result of equation x + 0.00001 = 1
        "volume": {
            "numer":"99999",
            "denom":"100000"
        },
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        // it is slightly more than previous volume so it should fail
        "volume": {
            "numer":"100000",
            "denom":"100000"
        },
    })))
    .unwrap();
    assert!(!rc.0.is_success(), "setprice success, but should fail: {}", rc.1);
    block_on(mm_alice.stop()).unwrap();
}

#[test]
fn swaps_should_stop_on_stop_rpc() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mut mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mut mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let mut uuids = Vec::with_capacity(3);

    for _ in 0..3 {
        let rc = block_on(mm_alice.rpc(&json!({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);
        let buy: Json = serde_json::from_str(&rc.1).unwrap();
        uuids.push(buy["result"]["uuid"].as_str().unwrap().to_owned());
    }
    for uuid in uuids.iter() {
        block_on(mm_bob.wait_for_log(22., |log| {
            log.contains(&format!(
                "Entering the maker_swap_loop MYCOIN/MYCOIN1 with uuid: {}",
                uuid
            ))
        }))
        .unwrap();
        block_on(mm_alice.wait_for_log(22., |log| {
            log.contains(&format!(
                "Entering the taker_swap_loop MYCOIN/MYCOIN1 with uuid: {}",
                uuid
            ))
        }))
        .unwrap();
    }
    thread::sleep(Duration::from_secs(3));
    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
    for uuid in uuids {
        block_on(mm_bob.wait_for_log_after_stop(22., |log| log.contains(&format!("swap {} stopped", uuid)))).unwrap();
        block_on(mm_alice.wait_for_log_after_stop(22., |log| log.contains(&format!("swap {} stopped", uuid)))).unwrap();
    }
}

#[test]
fn test_maker_order_should_kick_start_and_appear_in_orderbook_on_restart() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mut bob_conf = json!({
        "gui": "nogui",
        "netid": 9000,
        "dht": "on",  // Enable DHT without delay.
        "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
        "coins": coins,
        "rpc_password": "pass",
        "i_am_seed": true,
    });
    let mm_bob = MarketMakerIt::start(bob_conf.clone(), "pass".to_string(), None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    // mm_bob using same DB dir that should kick start the order
    bob_conf["dbdir"] = mm_bob.folder.join("DB").to_str().unwrap().into();
    bob_conf["log"] = mm_bob.folder.join("mm2_dup.log").to_str().unwrap().into();
    block_on(mm_bob.stop()).unwrap();

    let mm_bob_dup = MarketMakerIt::start(bob_conf, "pass".to_string(), None).unwrap();
    let (_bob_dup_dump_log, _bob_dup_dump_dashboard) = mm_dump(&mm_bob_dup.log_path);
    log!("{:?}", block_on(enable_native(&mm_bob_dup, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob_dup, "MYCOIN1", &[], None)));

    thread::sleep(Duration::from_secs(2));

    log!("Get RICK/MORTY orderbook on Bob side");
    let rc = block_on(mm_bob_dup.rpc(&json!({
        "userpass": mm_bob_dup.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("Bob orderbook {:?}", bob_orderbook);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 asks");
}

#[test]
fn test_maker_order_should_not_kick_start_and_appear_in_orderbook_if_balance_is_withdrawn() {
    let (_ctx, coin, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mut bob_conf = json!({
        "gui": "nogui",
        "netid": 9000,
        "dht": "on",  // Enable DHT without delay.
        "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
        "coins": coins,
        "rpc_password": "pass",
        "i_am_seed": true,
    });
    let mm_bob = MarketMakerIt::start(bob_conf.clone(), "pass".to_string(), None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let res: SetPriceResponse = serde_json::from_str(&rc.1).unwrap();
    let uuid = res.result.uuid;

    // mm_bob using same DB dir that should kick start the order
    bob_conf["dbdir"] = mm_bob.folder.join("DB").to_str().unwrap().into();
    bob_conf["log"] = mm_bob.folder.join("mm2_dup.log").to_str().unwrap().into();
    block_on(mm_bob.stop()).unwrap();

    let withdraw = coin
        .withdraw(WithdrawRequest::new_max(
            "MYCOIN".to_string(),
            "RRYmiZSDo3UdHHqj1rLKf8cbJroyv9NxXw".to_string(),
        ))
        .wait()
        .unwrap();
    coin.send_raw_tx(&hex::encode(&withdraw.tx_hex.0)).wait().unwrap();
    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: withdraw.tx_hex.0,
        confirmations: 1,
        requires_nota: false,
        wait_until: wait_until_sec(10),
        check_every: 1,
    };
    coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let mm_bob_dup = MarketMakerIt::start(bob_conf, "pass".to_string(), None).unwrap();
    let (_bob_dup_dump_log, _bob_dup_dump_dashboard) = mm_dump(&mm_bob_dup.log_path);
    log!("{:?}", block_on(enable_native(&mm_bob_dup, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob_dup, "MYCOIN1", &[], None)));

    thread::sleep(Duration::from_secs(2));

    log!("Get RICK/MORTY orderbook on Bob side");
    let rc = block_on(mm_bob_dup.rpc(&json!({
        "userpass": mm_bob_dup.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("Bob orderbook {:?}", bob_orderbook);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert!(asks.is_empty(), "Bob MYCOIN/MYCOIN1 orderbook must not have asks");

    let rc = block_on(mm_bob_dup.rpc(&json!({
        "userpass": mm_bob_dup.userpass,
        "method": "my_orders",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!my_orders: {}", rc.1);

    let res: MyOrdersRpcResult = serde_json::from_str(&rc.1).unwrap();
    assert!(res.result.maker_orders.is_empty(), "Bob maker orders must be empty");

    let order_path = mm_bob.folder.join(format!(
        "DB/{}/ORDERS/MY/MAKER/{}.json",
        hex::encode(rmd160_from_priv(bob_priv_key).take()),
        uuid
    ));

    println!("Order path {}", order_path.display());
    assert!(!order_path.exists());
}

#[test]
fn test_maker_order_kick_start_should_trigger_subscription_and_match() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 1000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);

    let relay_conf = json!({
        "gui": "nogui",
        "netid": 9000,
        "dht": "on",  // Enable DHT without delay.
        "passphrase": "relay",
        "coins": coins,
        "rpc_password": "pass",
        "i_am_seed": true,
    });
    let relay = MarketMakerIt::start(relay_conf, "pass".to_string(), None).unwrap();
    let (_relay_dump_log, _relay_dump_dashboard) = mm_dump(&relay.log_path);

    let mut bob_conf = json!({
        "gui": "nogui",
        "netid": 9000,
        "dht": "on",  // Enable DHT without delay.
        "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
        "coins": coins,
        "rpc_password": "pass",
        "seednodes": vec![format!("{}", relay.ip)],
        "i_am_seed": false,
    });
    let mm_bob = MarketMakerIt::start(bob_conf.clone(), "pass".to_string(), None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mut mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", relay.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));

    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    // mm_bob using same DB dir that should kick start the order
    bob_conf["dbdir"] = mm_bob.folder.join("DB").to_str().unwrap().into();
    bob_conf["log"] = mm_bob.folder.join("mm2_dup.log").to_str().unwrap().into();
    block_on(mm_bob.stop()).unwrap();

    let mut mm_bob_dup = MarketMakerIt::start(bob_conf, "pass".to_string(), None).unwrap();
    let (_bob_dup_dump_log, _bob_dup_dump_dashboard) = mm_dump(&mm_bob_dup.log_path);
    log!("{:?}", block_on(enable_native(&mm_bob_dup, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob_dup, "MYCOIN1", &[], None)));

    log!("Give restarted Bob 2 seconds to kickstart the order");
    thread::sleep(Duration::from_secs(2));

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "volume": 1,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    block_on(mm_bob_dup.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
    block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
}

#[test]
fn test_orders_should_match_on_both_nodes_with_same_priv() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mut mm_alice_1 = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_1_dump_log, _alice_1_dump_dashboard) = mm_dump(&mm_alice_1.log_path);

    let mut mm_alice_2 = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_2_dump_log, _alice_2_dump_dashboard) = mm_dump(&mm_alice_2.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice_1, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice_1, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice_2, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice_2, "MYCOIN1", &[], None)));

    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let rc = block_on(mm_alice_1.rpc(&json!({
        "userpass": mm_alice_1.userpass,
        "method": "buy",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "volume": "1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    block_on(mm_alice_1.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1"))).unwrap();

    let rc = block_on(mm_alice_2.rpc(&json!({
        "userpass": mm_alice_2.userpass,
        "method": "buy",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "volume": "1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    block_on(mm_alice_2.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1"))).unwrap();

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice_1.stop()).unwrap();
    block_on(mm_alice_2.stop()).unwrap();
}

#[test]
fn test_maker_and_taker_order_created_with_same_priv_should_not_match() {
    let (_ctx, coin, priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let (_ctx, coin1, _) = generate_utxo_coin_with_random_privkey("MYCOIN1", 1000.into());
    fill_address(&coin1, &coin.my_address().unwrap(), 1000.into(), 30);
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mut mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mut mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_1_dump_log, _alice_1_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));

    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "volume": "1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    block_on(mm_bob.wait_for_log(5., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap_err();
    block_on(mm_alice.wait_for_log(5., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1"))).unwrap_err();

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
fn test_taker_order_converted_to_maker_should_cancel_properly_when_matched() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mut mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mut mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "sell",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "volume": 1,
        "timeout": 2,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!sell: {}", rc.1);

    log!("Give Bob 4 seconds to convert order to maker");
    thread::sleep(Duration::from_secs(4));

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "volume": 1,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
    block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1"))).unwrap();

    log!("Give Bob 2 seconds to cancel the order");
    thread::sleep(Duration::from_secs(2));
    log!("Get my_orders on Bob side");
    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "my_orders",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!my_orders: {}", rc.1);
    let my_orders_json: Json = serde_json::from_str(&rc.1).unwrap();
    let maker_orders: HashMap<String, Json> =
        serde_json::from_value(my_orders_json["result"]["maker_orders"].clone()).unwrap();
    assert!(maker_orders.is_empty());

    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let bob_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("Bob orderbook {:?}", bob_orderbook);
    let asks = bob_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 0, "Bob MYCOIN/MYCOIN1 orderbook must be empty");

    log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

    let alice_orderbook: Json = serde_json::from_str(&rc.1).unwrap();
    log!("Alice orderbook {:?}", alice_orderbook);
    let asks = alice_orderbook["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 0, "Alice MYCOIN/MYCOIN1 orderbook must be empty");

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
fn test_utxo_merge() {
    let timeout = 30; // timeout if test takes more than 30 seconds to run
    let (_ctx, coin, privkey) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    // fill several times to have more UTXOs on address
    fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
    fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
    fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
    fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);

    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mut mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(privkey)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let native = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "enable",
        "coin": "MYCOIN",
        "mm2": 1,
        "utxo_merge_params": {
            "merge_at": 2,
            "check_every": 1,
        }
    })))
    .unwrap();
    assert!(native.0.is_success(), "'enable' failed: {}", native.1);
    log!("Enable result {}", native.1);

    block_on(mm_bob.wait_for_log(4., |log| log.contains("Starting UTXO merge loop for coin MYCOIN"))).unwrap();

    block_on(mm_bob.wait_for_log(4., |log| log.contains("Trying to merge 5 UTXOs of coin MYCOIN"))).unwrap();

    block_on(mm_bob.wait_for_log(4., |log| log.contains("UTXO merge successful for coin MYCOIN, tx_hash"))).unwrap();

    thread::sleep(Duration::from_secs(2));
    let (unspents, _) =
        block_on(coin.get_unspent_ordered_list(coin.as_ref().derivation_method.unwrap_single_addr())).unwrap();
    assert_eq!(unspents.len(), 1);
}

#[test]
fn test_utxo_merge_max_merge_at_once() {
    let timeout = 30; // timeout if test takes more than 30 seconds to run
    let (_ctx, coin, privkey) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    // fill several times to have more UTXOs on address
    fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
    fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
    fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
    fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);

    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mut mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(privkey)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let native = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "enable",
        "coin": "MYCOIN",
        "mm2": 1,
        "utxo_merge_params": {
            "merge_at": 3,
            "check_every": 1,
            "max_merge_at_once": 4,
        }
    })))
    .unwrap();
    assert!(native.0.is_success(), "'enable' failed: {}", native.1);
    log!("Enable result {}", native.1);

    block_on(mm_bob.wait_for_log(4., |log| log.contains("Starting UTXO merge loop for coin MYCOIN"))).unwrap();

    block_on(mm_bob.wait_for_log(4., |log| log.contains("Trying to merge 4 UTXOs of coin MYCOIN"))).unwrap();

    block_on(mm_bob.wait_for_log(4., |log| log.contains("UTXO merge successful for coin MYCOIN, tx_hash"))).unwrap();

    thread::sleep(Duration::from_secs(2));
    let (unspents, _) =
        block_on(coin.get_unspent_ordered_list(coin.as_ref().derivation_method.unwrap_single_addr())).unwrap();
    // 4 utxos are merged of 5 so the resulting unspents len must be 2
    assert_eq!(unspents.len(), 2);
}

#[test]
fn test_withdraw_not_sufficient_balance() {
    let privkey = random_secp256k1_secret();
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mm = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(privkey)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm.log_path);
    log!("{:?}", block_on(enable_native(&mm, "MYCOIN", &[], None)));

    // balance = 0, but amount = 1
    let amount = BigDecimal::from(1);
    let withdraw = block_on(mm.rpc(&json!({
        "mmrpc": "2.0",
        "userpass": mm.userpass,
        "method": "withdraw",
        "params": {
            "coin": "MYCOIN",
            "to": "RJTYiYeJ8eVvJ53n2YbrVmxWNNMVZjDGLh",
            "amount": amount,
        },
        "id": 0,
    })))
    .unwrap();

    assert!(withdraw.0.is_client_error(), "RICK withdraw: {}", withdraw.1);
    log!("error: {:?}", withdraw.1);
    let error: RpcErrorResponse<withdraw_error::NotSufficientBalance> =
        serde_json::from_str(&withdraw.1).expect("Expected 'RpcErrorResponse<NotSufficientBalance>'");
    let expected_error = withdraw_error::NotSufficientBalance {
        coin: "MYCOIN".to_owned(),
        available: 0.into(),
        required: amount,
    };
    assert_eq!(error.error_type, "NotSufficientBalance");
    assert_eq!(error.error_data, Some(expected_error));

    // fill the MYCOIN balance
    let balance = BigDecimal::from(1) / BigDecimal::from(2);
    let (_ctx, coin) = utxo_coin_from_privkey("MYCOIN", privkey);
    fill_address(&coin, &coin.my_address().unwrap(), balance.clone(), 30);

    // txfee = 0.00001, amount = 0.5 => required = 0.50001
    // but balance = 0.5
    let txfee = BigDecimal::from(1) / BigDecimal::from(100000);
    let withdraw = block_on(mm.rpc(&json!({
        "mmrpc": "2.0",
        "userpass": mm.userpass,
        "method": "withdraw",
        "params": {
            "coin": "MYCOIN",
            "to": "RJTYiYeJ8eVvJ53n2YbrVmxWNNMVZjDGLh",
            "amount": balance,
        },
        "id": 0,
    })))
    .unwrap();

    assert!(withdraw.0.is_client_error(), "RICK withdraw: {}", withdraw.1);
    log!("error: {:?}", withdraw.1);
    let error: RpcErrorResponse<withdraw_error::NotSufficientBalance> =
        serde_json::from_str(&withdraw.1).expect("Expected 'RpcErrorResponse<NotSufficientBalance>'");
    let expected_error = withdraw_error::NotSufficientBalance {
        coin: "MYCOIN".to_owned(),
        available: balance.clone(),
        required: balance + txfee,
    };
    assert_eq!(error.error_type, "NotSufficientBalance");
    assert_eq!(error.error_data, Some(expected_error));
}

// https://github.com/KomodoPlatform/atomicDEX-API/issues/1053
#[test]
fn test_taker_should_match_with_best_price_buy() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 2000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 4000.into());
    let (_ctx, _, eve_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 2000.into());

    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mut mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    let mut mm_eve = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(eve_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_eve_dump_log, _eve_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_eve, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_eve, "MYCOIN1", &[], None)));

    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 2,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let rc = block_on(mm_eve.rpc(&json!({
        "userpass": mm_eve.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    // subscribe alice to the orderbook topic to not miss eve's message
    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!alice orderbook: {}", rc.1);
    log!("alice orderbook {}", rc.1);

    thread::sleep(Duration::from_secs(1));

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 3,
        "volume": "1000",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);
    let alice_buy: BuyOrSellRpcResult = serde_json::from_str(&rc.1).unwrap();

    block_on(mm_eve.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
    block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1"))).unwrap();

    thread::sleep(Duration::from_secs(2));

    block_on(check_my_swap_status_amounts(
        &mm_alice,
        alice_buy.result.uuid,
        1000.into(),
        1000.into(),
    ));
    block_on(check_my_swap_status_amounts(
        &mm_eve,
        alice_buy.result.uuid,
        1000.into(),
        1000.into(),
    ));

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
    block_on(mm_eve.stop()).unwrap();
}

// https://github.com/KomodoPlatform/atomicDEX-API/issues/1053
#[test]
fn test_taker_should_match_with_best_price_sell() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 2000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 4000.into());
    let (_ctx, _, eve_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 2000.into());

    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mut mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    let mut mm_eve = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(eve_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_eve_dump_log, _eve_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_eve, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_eve, "MYCOIN1", &[], None)));

    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 2,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let rc = block_on(mm_eve.rpc(&json!({
        "userpass": mm_eve.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
        "price": 1,
        "max": true,
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    // subscribe alice to the orderbook topic to not miss eve's message
    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "orderbook",
        "base": "MYCOIN",
        "rel": "MYCOIN1",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!alice orderbook: {}", rc.1);
    log!("alice orderbook {}", rc.1);

    thread::sleep(Duration::from_secs(1));

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "sell",
        "base": "MYCOIN1",
        "rel": "MYCOIN",
        "price": "0.1",
        "volume": "1000",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);
    let alice_buy: BuyOrSellRpcResult = serde_json::from_str(&rc.1).unwrap();

    block_on(mm_eve.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
    block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1"))).unwrap();

    thread::sleep(Duration::from_secs(2));

    block_on(check_my_swap_status_amounts(
        &mm_alice,
        alice_buy.result.uuid,
        1000.into(),
        1000.into(),
    ));
    block_on(check_my_swap_status_amounts(
        &mm_eve,
        alice_buy.result.uuid,
        1000.into(),
        1000.into(),
    ));

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
    block_on(mm_eve.stop()).unwrap();
}

#[test]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/1074
fn test_match_utxo_with_eth_taker_sell() {
    let alice_passphrase = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();
    let bob_passphrase = get_passphrase!(".env.seed", "BOB_PASSPHRASE").unwrap();
    let alice_priv_key = key_pair_from_seed(&alice_passphrase).unwrap().private().secret;
    let bob_priv_key = key_pair_from_seed(&bob_passphrase).unwrap().private().secret;

    generate_utxo_coin_with_privkey("MYCOIN", 1000.into(), alice_priv_key);
    generate_utxo_coin_with_privkey("MYCOIN", 1000.into(), bob_priv_key);

    let coins = json!([mycoin_conf(1000), eth_testnet_conf()]);

    let mut mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mut mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    block_on(enable_native(&mm_bob, "ETH", ETH_DEV_NODES, None));
    block_on(enable_native(&mm_alice, "ETH", ETH_DEV_NODES, None));

    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "ETH",
        "price": 1,
        "volume": "0.0001",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);
    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "sell",
        "base": "ETH",
        "rel": "MYCOIN",
        "price": 1,
        "volume": "0.0001",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!sell: {}", rc.1);

    block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/ETH"))).unwrap();
    block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/ETH"))).unwrap();

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/1074
fn test_match_utxo_with_eth_taker_buy() {
    let alice_passphrase = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();
    let bob_passphrase = get_passphrase!(".env.seed", "BOB_PASSPHRASE").unwrap();
    let alice_priv_key = key_pair_from_seed(&alice_passphrase).unwrap().private().secret;
    let bob_priv_key = key_pair_from_seed(&bob_passphrase).unwrap().private().secret;

    generate_utxo_coin_with_privkey("MYCOIN", 1000.into(), alice_priv_key);
    generate_utxo_coin_with_privkey("MYCOIN", 1000.into(), bob_priv_key);
    let coins = json!([mycoin_conf(1000), eth_testnet_conf()]);
    let mut mm_bob = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let mut mm_alice = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", mm_bob.ip)],
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    block_on(enable_native(&mm_bob, "ETH", ETH_DEV_NODES, None));

    block_on(enable_native(&mm_alice, "ETH", ETH_DEV_NODES, None));

    let rc = block_on(mm_bob.rpc(&json!({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": "MYCOIN",
        "rel": "ETH",
        "price": 1,
        "volume": "0.0001",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    let rc = block_on(mm_alice.rpc(&json!({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": "MYCOIN",
        "rel": "ETH",
        "price": 1,
        "volume": "0.0001",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);

    block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/ETH"))).unwrap();
    block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/ETH"))).unwrap();

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

#[test]
fn test_locked_amount() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 1000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);
    let bob_conf = Mm2TestConf::seednode(&format!("0x{}", hex::encode(bob_priv_key)), &coins);
    let mut mm_bob = MarketMakerIt::start(bob_conf.conf, bob_conf.rpc_password, None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let alice_conf = Mm2TestConf::light_node(&format!("0x{}", hex::encode(alice_priv_key)), &coins, &[&mm_bob
        .ip
        .to_string()]);
    let mut mm_alice = MarketMakerIt::start(alice_conf.conf, alice_conf.rpc_password, None).unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));

    block_on(start_swaps(
        &mut mm_bob,
        &mut mm_alice,
        &[("MYCOIN", "MYCOIN1")],
        1.,
        1.,
        777.,
    ));

    let locked_bob = block_on(get_locked_amount(&mm_bob, "MYCOIN"));
    assert_eq!(locked_bob.coin, "MYCOIN");

    let expected_result: MmNumberMultiRepr = MmNumber::from("777.00001").into();
    assert_eq!(expected_result, locked_bob.locked_amount);

    let locked_alice = block_on(get_locked_amount(&mm_alice, "MYCOIN1"));
    assert_eq!(locked_alice.coin, "MYCOIN1");

    let expected_result: MmNumberMultiRepr = MmNumber::from("778.00002").into();
    assert_eq!(expected_result, locked_alice.locked_amount);
}
