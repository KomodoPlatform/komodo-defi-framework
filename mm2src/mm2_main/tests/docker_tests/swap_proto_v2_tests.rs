use crate::{generate_utxo_coin_with_random_privkey, MYCOIN};
use bitcrypto::dhash160;
use coins::utxo::UtxoCommonOps;
use coins::{GenTakerPaymentSpendArgs, RefundPaymentArgs, SendDexFeeWithPremiumArgs, SwapOpsV2, Transaction,
            TransactionEnum, ValidateTakerPaymentArgs};
use common::{block_on, now_sec_u32, DEX_FEE_ADDR_RAW_PUBKEY};
use script::{Builder, Opcode};

#[test]
fn send_and_refund_taker_payment() {
    let (_mm_arc, coin, _privkey) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());

    let time_lock = now_sec_u32() - 1000;
    let secret_hash = &[0; 20];
    let other_pub = coin.my_public_key().unwrap();

    let send_args = SendDexFeeWithPremiumArgs {
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

    let time_lock = now_sec_u32() - 1000;
    let secret = [1; 32];
    let secret_hash = dhash160(&secret);
    let send_args = SendDexFeeWithPremiumArgs {
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

    let dex_fee_spend = block_on(maker_coin.sign_and_broadcast_taker_payment_spend(
        &preimage_with_taker_sig,
        &gen_preimage_args,
        &secret,
        &[],
    ))
    .unwrap();
    println!("dex_fee_spend hash {:02x}", dex_fee_spend.tx_hash());
}
