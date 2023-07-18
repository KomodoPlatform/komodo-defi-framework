use crate::{generate_utxo_coin_with_random_privkey, MYCOIN};
use bitcrypto::dhash160;
use coins::utxo::UtxoCommonOps;
use coins::{GenAndSignDexFeeSpendArgs, RefundPaymentArgs, SendDexFeeWithPremiumArgs, SwapOpsV2, Transaction,
            TransactionEnum};
use common::{block_on, now_sec_u32, DEX_FEE_ADDR_RAW_PUBKEY};
use script::{Builder, Opcode};

#[test]
fn send_and_refund_dex_fee() {
    let (_mm_arc, coin, _privkey) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());

    let time_lock = now_sec_u32() - 1000;

    let send_args = SendDexFeeWithPremiumArgs {
        time_lock,
        secret_hash: &[0; 20],
        other_pub: coin.my_public_key().unwrap(),
        dex_fee_amount: "0.01".parse().unwrap(),
        premium_amount: "0.1".parse().unwrap(),
        swap_unique_data: &[],
    };
    let dex_fee_tx = block_on(coin.send_dex_fee_with_premium(send_args)).unwrap();
    println!("{:02x}", dex_fee_tx.tx_hash());
    let dex_fee_utxo_tx = match dex_fee_tx {
        TransactionEnum::UtxoTx(tx) => tx,
        unexpected => panic!("Unexpected tx {:?}", unexpected),
    };
    // tx must have 3 outputs: actual payment, OP_RETURN containing the secret hash and change
    assert_eq!(3, dex_fee_utxo_tx.outputs.len());

    // dex_fee_amount + premium_amount
    let expected_amount = 11000000u64;
    assert_eq!(expected_amount, dex_fee_utxo_tx.outputs[0].value);

    let expected_op_return = Builder::default()
        .push_opcode(Opcode::OP_RETURN)
        .push_data(&[0; 20])
        .into_bytes();
    assert_eq!(expected_op_return, dex_fee_utxo_tx.outputs[1].script_pubkey);

    let refund_args = RefundPaymentArgs {
        payment_tx: &dex_fee_utxo_tx.tx_hex(),
        time_lock,
        other_pubkey: coin.my_public_key().unwrap(),
        secret_hash: &[0; 20],
        swap_unique_data: &[],
        swap_contract_address: &None,
        watcher_reward: false,
    };

    let refund_tx = block_on(coin.refund_dex_fee_with_premium(refund_args)).unwrap();
    println!("{:02x}", refund_tx.tx_hash());
}

#[test]
fn send_and_spend_dex_fee() {
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
        swap_unique_data: &[],
    };
    let dex_fee_tx = block_on(taker_coin.send_dex_fee_with_premium(send_args)).unwrap();
    println!("{:02x}", dex_fee_tx.tx_hash());
    let dex_fee_utxo_tx = match dex_fee_tx {
        TransactionEnum::UtxoTx(tx) => tx,
        unexpected => panic!("Unexpected tx {:?}", unexpected),
    };

    let gen_preimage_args = GenAndSignDexFeeSpendArgs {
        tx: &dex_fee_utxo_tx.tx_hex(),
        time_lock,
        secret_hash: secret_hash.as_slice(),
        other_pub: &maker_coin.my_public_key().unwrap(),
        dex_fee_pub: &DEX_FEE_ADDR_RAW_PUBKEY,
        dex_fee_amount: "0.01".parse().unwrap(),
        premium_amount: "0.1".parse().unwrap(),
        swap_unique_data: &[],
    };
    let preimage_with_sig = block_on(taker_coin.gen_and_sign_dex_fee_spend_preimage(gen_preimage_args)).unwrap();
    /*
    let input_to_spend = UnsignedTransactionInput {
        previous_output: OutPoint {
            hash: dex_fee_tx.hash(),
            index: 0,
        },
        sequence: SEQUENCE_FINAL,
        amount: value,
        witness: vec![],
    };

    let fee =
        block_on(taker_coin.get_htlc_spend_fee(DEFAULT_SWAP_TX_SPEND_SIZE, &FeeApproxStage::WithoutApprox)).unwrap();
    let my_address = taker_coin.as_ref().derivation_method.single_addr_or_err().unwrap();
    let output = TransactionOutput {
        value: value - fee,
        script_pubkey: output_script(my_address, ScriptType::P2PKH).into(),
    };

    let input_signer = TransactionInputSigner {
        version: taker_coin.as_ref().conf.tx_version,
        n_time: None,
        overwintered: taker_coin.as_ref().conf.overwintered,
        version_group_id: taker_coin.as_ref().conf.version_group_id,
        consensus_branch_id: taker_coin.as_ref().conf.consensus_branch_id,
        expiry_height: 0,
        value_balance: 0,
        inputs: vec![input_to_spend.clone()],
        outputs: vec![output],
        lock_time: timelock,
        join_splits: vec![],
        shielded_spends: vec![],
        shielded_outputs: vec![],
        zcash: taker_coin.as_ref().conf.zcash,
        posv: false,
        str_d_zeel: None,
        hash_algo: taker_coin.as_ref().tx_hash_algo.into(),
    };

    let sighash = input_signer.signature_hash(
        0,
        value,
        &script,
        taker_coin.as_ref().conf.signature_version,
        1 | taker_coin.as_ref().conf.fork_id,
    );
    let taker_signature = taker_coin
        .as_ref()
        .priv_key_policy
        .key_pair_or_err()
        .unwrap()
        .private()
        .sign(&sighash)
        .unwrap();
    let mut taker_signature_with_sighash = taker_signature.to_vec();
    taker_signature_with_sighash.push(1 | taker_coin.as_ref().conf.fork_id as u8);

    let maker_signature = maker_coin
        .as_ref()
        .priv_key_policy
        .key_pair_or_err()
        .unwrap()
        .private()
        .sign(&sighash)
        .unwrap();
    let mut maker_signature_with_sighash = maker_signature.to_vec();
    maker_signature_with_sighash.push(1 | taker_coin.as_ref().conf.fork_id as u8);

    let script_sig = Builder::default()
        .push_opcode(Opcode::OP_0)
        .push_data(&taker_signature_with_sighash)
        .push_data(&maker_signature_with_sighash)
        .push_data(&secret)
        .push_opcode(Opcode::OP_0)
        .push_data(&script)
        .into_bytes();

    let input = TransactionInput {
        previous_output: input_to_spend.previous_output,
        script_sig,
        sequence: SEQUENCE_FINAL,
        script_witness: vec![],
    };
    let spend_tx = UtxoTx {
        version: input_signer.version,
        n_time: input_signer.n_time,
        overwintered: input_signer.overwintered,
        version_group_id: input_signer.version_group_id,
        inputs: vec![input],
        outputs: input_signer.outputs,
        lock_time: input_signer.lock_time,
        expiry_height: input_signer.expiry_height,
        shielded_spends: input_signer.shielded_spends,
        shielded_outputs: input_signer.shielded_outputs,
        join_splits: input_signer.join_splits,
        value_balance: input_signer.value_balance,
        join_split_pubkey: Default::default(),
        join_split_sig: Default::default(),
        binding_sig: Default::default(),
        zcash: input_signer.zcash,
        posv: input_signer.posv,
        str_d_zeel: input_signer.str_d_zeel,
        tx_hash_algo: input_signer.hash_algo.into(),
    };

    block_on(taker_coin.broadcast_tx(&spend_tx)).unwrap();
     */
}
