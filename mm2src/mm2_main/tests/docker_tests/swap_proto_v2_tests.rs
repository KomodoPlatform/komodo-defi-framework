use bitcrypto::dhash160;
use coins::utxo::UtxoCommonOps;
use coins::{GenTakerPaymentSpendArgs, RefundPaymentArgs, SendCombinedTakerPaymentArgs, SwapOpsV2, Transaction,
            TransactionEnum, ValidateTakerPaymentArgs};
use common::{block_on, now_sec, DEX_FEE_ADDR_RAW_PUBKEY};
use mm2_test_helpers::for_tests::{enable_native, mm_dump, mycoin1_conf, mycoin_conf, start_swaps, MarketMakerIt,
                                  Mm2TestConf};
use script::{Builder, Opcode};

use crate::{generate_utxo_coin_with_random_privkey, MYCOIN, MYCOIN1};

#[test]
fn send_and_refund_taker_payment() {
    let (_mm_arc, coin, _privkey) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());

    let time_lock = now_sec() - 1000;
    let secret_hash = &[0; 20];
    let other_pub = coin.my_public_key().unwrap();

    let send_args = SendCombinedTakerPaymentArgs {
        time_lock,
        secret_hash,
        other_pub,
        dex_fee_amount: "0.01".parse().unwrap(),
        premium_amount: "0.1".parse().unwrap(),
        trading_amount: 1.into(),
        swap_unique_data: &[],
    };
    let taker_payment_tx = block_on(coin.send_combined_taker_payment(send_args)).unwrap();
    println!("{:02x}", taker_payment_tx.tx_hash());
    let taker_payment_utxo_tx = match taker_payment_tx {
        TransactionEnum::UtxoTx(tx) => tx,
        unexpected => panic!("Unexpected tx {:?}", unexpected),
    };
    // tx must have 3 outputs: actual payment, OP_RETURN containing the secret hash and change
    assert_eq!(3, taker_payment_utxo_tx.outputs.len());

    // dex_fee_amount + premium_amount + trading_amount
    let expected_amount = 111000000u64;
    assert_eq!(expected_amount, taker_payment_utxo_tx.outputs[0].value);

    let expected_op_return = Builder::default()
        .push_opcode(Opcode::OP_RETURN)
        .push_data(&[0; 20])
        .into_bytes();
    assert_eq!(expected_op_return, taker_payment_utxo_tx.outputs[1].script_pubkey);

    let taker_payment_bytes = taker_payment_utxo_tx.tx_hex();

    let validate_args = ValidateTakerPaymentArgs {
        taker_tx: &taker_payment_bytes,
        time_lock,
        secret_hash,
        other_pub,
        dex_fee_amount: "0.01".parse().unwrap(),
        premium_amount: "0.1".parse().unwrap(),
        trading_amount: 1.into(),
        swap_unique_data: &[],
    };
    block_on(coin.validate_combined_taker_payment(validate_args)).unwrap();

    let refund_args = RefundPaymentArgs {
        payment_tx: &taker_payment_bytes,
        time_lock,
        other_pubkey: coin.my_public_key().unwrap(),
        secret_hash: &[0; 20],
        swap_unique_data: &[],
        swap_contract_address: &None,
        watcher_reward: false,
    };

    let refund_tx = block_on(coin.refund_combined_taker_payment(refund_args)).unwrap();
    println!("{:02x}", refund_tx.tx_hash());
}

#[test]
fn send_and_spend_taker_payment() {
    let (_, taker_coin, _) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());
    let (_, maker_coin, _) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());

    let time_lock = now_sec() - 1000;
    let secret = [1; 32];
    let secret_hash = dhash160(&secret);
    let send_args = SendCombinedTakerPaymentArgs {
        time_lock,
        secret_hash: secret_hash.as_slice(),
        other_pub: maker_coin.my_public_key().unwrap(),
        dex_fee_amount: "0.01".parse().unwrap(),
        premium_amount: "0.1".parse().unwrap(),
        trading_amount: 1.into(),
        swap_unique_data: &[],
    };
    let taker_payment_tx = block_on(taker_coin.send_combined_taker_payment(send_args)).unwrap();
    println!("taker_payment_tx hash {:02x}", taker_payment_tx.tx_hash());
    let taker_payment_utxo_tx = match taker_payment_tx {
        TransactionEnum::UtxoTx(tx) => tx,
        unexpected => panic!("Unexpected tx {:?}", unexpected),
    };

    let taker_payment_bytes = taker_payment_utxo_tx.tx_hex();
    let validate_args = ValidateTakerPaymentArgs {
        taker_tx: &taker_payment_bytes,
        time_lock,
        secret_hash: secret_hash.as_slice(),
        other_pub: taker_coin.my_public_key().unwrap(),
        dex_fee_amount: "0.01".parse().unwrap(),
        premium_amount: "0.1".parse().unwrap(),
        trading_amount: 1.into(),
        swap_unique_data: &[],
    };
    block_on(maker_coin.validate_combined_taker_payment(validate_args)).unwrap();

    let gen_preimage_args = GenTakerPaymentSpendArgs {
        taker_tx: &taker_payment_utxo_tx.tx_hex(),
        time_lock,
        secret_hash: secret_hash.as_slice(),
        maker_pub: maker_coin.my_public_key().unwrap(),
        taker_pub: taker_coin.my_public_key().unwrap(),
        dex_fee_pub: &DEX_FEE_ADDR_RAW_PUBKEY,
        dex_fee_amount: "0.01".parse().unwrap(),
        premium_amount: "0.1".parse().unwrap(),
        trading_amount: 1.into(),
    };
    let preimage_with_taker_sig =
        block_on(taker_coin.gen_taker_payment_spend_preimage(&gen_preimage_args, &[])).unwrap();

    block_on(maker_coin.validate_taker_payment_spend_preimage(&gen_preimage_args, &preimage_with_taker_sig)).unwrap();

    let taker_payment_spend = block_on(maker_coin.sign_and_broadcast_taker_payment_spend(
        &preimage_with_taker_sig,
        &gen_preimage_args,
        &secret,
        &[],
    ))
    .unwrap();
    println!("taker_payment_spend hash {:02x}", taker_payment_spend.tx_hash());
}

#[test]
fn test_v2_swap_utxo_utxo() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey(MYCOIN1, 1000.into());
    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);

    let bob_conf = Mm2TestConf::seednode_trade_v2(&format!("0x{}", hex::encode(bob_priv_key)), &coins);
    let mut mm_bob = MarketMakerIt::start(bob_conf.conf, bob_conf.rpc_password, None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let alice_conf =
        Mm2TestConf::light_node_trade_v2(&format!("0x{}", hex::encode(alice_priv_key)), &coins, &[&mm_bob
            .ip
            .to_string()]);
    let mut mm_alice = MarketMakerIt::start(alice_conf.conf, alice_conf.rpc_password, None).unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

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
        block_on(mm_alice.wait_for_log(60., |log| log.contains(&expected_msg))).unwrap();
    }
}
