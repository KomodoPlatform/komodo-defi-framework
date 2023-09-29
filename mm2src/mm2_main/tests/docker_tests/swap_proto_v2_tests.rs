use crate::{generate_utxo_coin_with_random_privkey, MYCOIN, MYCOIN1};
use bitcrypto::dhash160;
use coins::utxo::UtxoCommonOps;
use coins::{GenTakerFundingSpendArgs, RefundFundingSecretArgs, RefundPaymentArgs, SendTakerFundingArgs, SwapOpsV2,
            Transaction, ValidateTakerFundingArgs};
use common::{block_on, now_sec};
use mm2_test_helpers::for_tests::{enable_native, mm_dump, my_swap_status, mycoin1_conf, mycoin_conf, start_swaps,
                                  MarketMakerIt, Mm2TestConf};
use script::{Builder, Opcode};
use serialization::serialize;

#[test]
fn send_and_refund_taker_funding_timelock() {
    let (_mm_arc, coin, _privkey) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());

    let time_lock = now_sec() - 1000;
    let taker_secret_hash = &[0; 20];
    let maker_pub = coin.my_public_key().unwrap();

    let send_args = SendTakerFundingArgs {
        time_lock,
        taker_secret_hash,
        maker_pub,
        dex_fee_amount: "0.01".parse().unwrap(),
        premium_amount: "0.1".parse().unwrap(),
        trading_amount: 1.into(),
        swap_unique_data: &[],
    };
    let taker_funding_utxo_tx = block_on(coin.send_taker_funding(send_args)).unwrap();
    println!("{:02x}", taker_funding_utxo_tx.tx_hash());
    // tx must have 3 outputs: actual funding, OP_RETURN containing the secret hash and change
    assert_eq!(3, taker_funding_utxo_tx.outputs.len());

    // dex_fee_amount + premium_amount + trading_amount
    let expected_amount = 111000000u64;
    assert_eq!(expected_amount, taker_funding_utxo_tx.outputs[0].value);

    let expected_op_return = Builder::default()
        .push_opcode(Opcode::OP_RETURN)
        .push_data(&[0; 20])
        .into_bytes();
    assert_eq!(expected_op_return, taker_funding_utxo_tx.outputs[1].script_pubkey);

    let validate_args = ValidateTakerFundingArgs {
        funding_tx: &taker_funding_utxo_tx,
        time_lock,
        taker_secret_hash,
        other_pub: maker_pub,
        dex_fee_amount: "0.01".parse().unwrap(),
        premium_amount: "0.1".parse().unwrap(),
        trading_amount: 1.into(),
        swap_unique_data: &[],
    };
    block_on(coin.validate_taker_funding(validate_args)).unwrap();

    let refund_args = RefundPaymentArgs {
        payment_tx: &serialize(&taker_funding_utxo_tx).take(),
        time_lock,
        other_pubkey: coin.my_public_key().unwrap(),
        secret_hash: &[0; 20],
        swap_unique_data: &[],
        swap_contract_address: &None,
        watcher_reward: false,
    };

    let refund_tx = block_on(coin.refund_taker_funding_timelock(refund_args)).unwrap();
    println!("{:02x}", refund_tx.tx_hash());
}

#[test]
fn send_and_refund_taker_funding_secret() {
    let (_mm_arc, coin, _privkey) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());

    let time_lock = now_sec() - 1000;
    let taker_secret = [0; 32];
    let taker_secret_hash = dhash160(&taker_secret);
    let maker_pub = coin.my_public_key().unwrap();

    let send_args = SendTakerFundingArgs {
        time_lock,
        taker_secret_hash: taker_secret_hash.as_slice(),
        maker_pub,
        dex_fee_amount: "0.01".parse().unwrap(),
        premium_amount: "0.1".parse().unwrap(),
        trading_amount: 1.into(),
        swap_unique_data: &[],
    };
    let taker_funding_utxo_tx = block_on(coin.send_taker_funding(send_args)).unwrap();
    println!("{:02x}", taker_funding_utxo_tx.tx_hash());
    // tx must have 3 outputs: actual funding, OP_RETURN containing the secret hash and change
    assert_eq!(3, taker_funding_utxo_tx.outputs.len());

    // dex_fee_amount + premium_amount + trading_amount
    let expected_amount = 111000000u64;
    assert_eq!(expected_amount, taker_funding_utxo_tx.outputs[0].value);

    let expected_op_return = Builder::default()
        .push_opcode(Opcode::OP_RETURN)
        .push_data(taker_secret_hash.as_slice())
        .into_bytes();
    assert_eq!(expected_op_return, taker_funding_utxo_tx.outputs[1].script_pubkey);

    let validate_args = ValidateTakerFundingArgs {
        funding_tx: &taker_funding_utxo_tx,
        time_lock,
        taker_secret_hash: taker_secret_hash.as_slice(),
        other_pub: maker_pub,
        dex_fee_amount: "0.01".parse().unwrap(),
        premium_amount: "0.1".parse().unwrap(),
        trading_amount: 1.into(),
        swap_unique_data: &[],
    };
    block_on(coin.validate_taker_funding(validate_args)).unwrap();

    let refund_args = RefundFundingSecretArgs {
        funding_tx: &taker_funding_utxo_tx,
        time_lock,
        maker_pubkey: maker_pub,
        taker_secret: &taker_secret,
        taker_secret_hash: taker_secret_hash.as_slice(),
        swap_unique_data: &[],
        swap_contract_address: &None,
        watcher_reward: false,
    };

    let refund_tx = block_on(coin.refund_taker_funding_secret(refund_args)).unwrap();
    println!("{:02x}", refund_tx.tx_hash());
}

#[test]
fn send_and_spend_taker_funding() {
    let (_mm_arc, taker_coin, _privkey) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());
    let (_mm_arc, maker_coin, _privkey) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());

    let funding_time_lock = now_sec() - 1000;
    let taker_secret_hash = &[0; 20];

    let taker_pub = taker_coin.my_public_key().unwrap();
    let maker_pub = maker_coin.my_public_key().unwrap();

    let send_args = SendTakerFundingArgs {
        time_lock: funding_time_lock,
        taker_secret_hash,
        maker_pub,
        dex_fee_amount: "0.01".parse().unwrap(),
        premium_amount: "0.1".parse().unwrap(),
        trading_amount: 1.into(),
        swap_unique_data: &[],
    };
    let taker_funding_utxo_tx = block_on(taker_coin.send_taker_funding(send_args)).unwrap();
    println!("Funding tx {:02x}", taker_funding_utxo_tx.tx_hash());
    // tx must have 3 outputs: actual funding, OP_RETURN containing the secret hash and change
    assert_eq!(3, taker_funding_utxo_tx.outputs.len());

    // dex_fee_amount + premium_amount + trading_amount
    let expected_amount = 111000000u64;
    assert_eq!(expected_amount, taker_funding_utxo_tx.outputs[0].value);

    let expected_op_return = Builder::default()
        .push_opcode(Opcode::OP_RETURN)
        .push_data(&[0; 20])
        .into_bytes();
    assert_eq!(expected_op_return, taker_funding_utxo_tx.outputs[1].script_pubkey);

    let validate_args = ValidateTakerFundingArgs {
        funding_tx: &taker_funding_utxo_tx,
        time_lock: funding_time_lock,
        taker_secret_hash,
        other_pub: taker_pub,
        dex_fee_amount: "0.01".parse().unwrap(),
        premium_amount: "0.1".parse().unwrap(),
        trading_amount: 1.into(),
        swap_unique_data: &[],
    };
    block_on(maker_coin.validate_taker_funding(validate_args)).unwrap();

    let preimage_args = GenTakerFundingSpendArgs {
        funding_tx: &taker_funding_utxo_tx,
        maker_pub,
        taker_pub,
        funding_time_lock,
        taker_secret_hash,
        taker_payment_time_lock: 0,
        maker_secret_hash: &[0; 20],
    };
    let preimage = block_on(maker_coin.gen_taker_funding_spend_preimage(&preimage_args, &[])).unwrap();

    let payment_tx = block_on(taker_coin.sign_and_send_taker_funding_spend(&preimage, &preimage_args, &[])).unwrap();
    println!("Taker payment tx {:02x}", payment_tx.tx_hash());
}

#[test]
fn test_v2_swap_utxo_utxo() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey(MYCOIN1, 1000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);

    let bob_conf = Mm2TestConf::seednode_trade_v2(&format!("0x{}", hex::encode(bob_priv_key)), &coins);
    let mut mm_bob = MarketMakerIt::start(bob_conf.conf, bob_conf.rpc_password, None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
    log!("Bob log path: {}", mm_bob.log_path.display());

    let alice_conf =
        Mm2TestConf::light_node_trade_v2(&format!("0x{}", hex::encode(alice_priv_key)), &coins, &[&mm_bob
            .ip
            .to_string()]);
    let mut mm_alice = MarketMakerIt::start(alice_conf.conf, alice_conf.rpc_password, None).unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
    log!("Alice log path: {}", mm_alice.log_path.display());

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[], None)));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[], None)));

    let uuids = block_on(start_swaps(
        &mut mm_bob,
        &mut mm_alice,
        &[(MYCOIN, MYCOIN1)],
        1.0,
        1.0,
        100.,
    ));
    println!("{:?}", uuids);

    for uuid in uuids {
        let expected_msg = format!("Swap {} has been completed", uuid);
        block_on(mm_bob.wait_for_log(60., |log| log.contains(&expected_msg))).unwrap();
        block_on(mm_alice.wait_for_log(30., |log| log.contains(&expected_msg))).unwrap();

        let maker_swap_status = block_on(my_swap_status(&mm_bob, &uuid));
        println!("{:?}", maker_swap_status);

        let taker_swap_status = block_on(my_swap_status(&mm_alice, &uuid));
        println!("{:?}", taker_swap_status);
    }
}
