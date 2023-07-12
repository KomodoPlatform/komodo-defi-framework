use crate::{generate_utxo_coin_with_random_privkey, MYCOIN};
use bitcrypto::dhash160;
use chain::constants::SEQUENCE_FINAL;
use chain::{OutPoint, TransactionInput, TransactionOutput};
use coins::utxo::swap_proto_v2_scripts::dex_fee_script;
use coins::utxo::utxo_common::{send_outputs_from_my_address, P2SHSpendingTxInput, DEFAULT_SWAP_TX_SPEND_SIZE};
use coins::utxo::{output_script, ScriptType, UtxoCommonOps, UtxoTx, UtxoTxBroadcastOps};
use coins::{FeeApproxStage, SwapOps, TransactionEnum};
use common::{block_on, now_sec_u32};
use futures01::Future;
use keys::AddressHashEnum;
use script::{Builder, Opcode, TransactionInputSigner, UnsignedTransactionInput};

#[test]
fn send_and_refund_dex_fee() {
    let (_mm_arc, coin, _privkey) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());

    let timelock = now_sec_u32() - 1000;
    let script = dex_fee_script(
        timelock,
        &[0; 20],
        coin.my_public_key().unwrap(),
        coin.my_public_key().unwrap(),
    );
    let p2sh = dhash160(script.as_slice());

    // 0.1 of the MYCOIN
    let value = 1000000;
    let output = TransactionOutput {
        value,
        script_pubkey: Builder::build_p2sh(&AddressHashEnum::AddressHash(p2sh)).into(),
    };
    let dex_fee_tx = match send_outputs_from_my_address(coin.clone(), vec![output]).wait().unwrap() {
        TransactionEnum::UtxoTx(tx) => tx,
        _ => panic!("Got unexpected tx"),
    };

    let fee = block_on(coin.get_htlc_spend_fee(DEFAULT_SWAP_TX_SPEND_SIZE, &FeeApproxStage::WithoutApprox)).unwrap();
    let my_address = coin.as_ref().derivation_method.single_addr_or_err().unwrap();
    let output = TransactionOutput {
        value: value - fee,
        script_pubkey: output_script(my_address, ScriptType::P2PKH).into(),
    };

    let script_data = Builder::default().push_opcode(Opcode::OP_1).into_script();
    let input = P2SHSpendingTxInput {
        prev_transaction: dex_fee_tx,
        redeem_script: script.into(),
        outputs: vec![output],
        script_data,
        sequence: SEQUENCE_FINAL - 1,
        lock_time: timelock,
        keypair: &coin.derive_htlc_key_pair(&[]),
    };
    let refund_tx = block_on(coin.p2sh_spending_tx(input)).unwrap();
    block_on(coin.broadcast_tx(&refund_tx)).unwrap();
}

#[test]
fn send_and_spend_dex_fee() {
    let (_, taker_coin, _) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());
    let (_, maker_coin, _) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());

    let timelock = now_sec_u32() - 1000;
    let secret = [1; 32];
    let secret_hash = dhash160(&secret);
    let script = dex_fee_script(
        timelock,
        secret_hash.as_slice(),
        taker_coin.my_public_key().unwrap(),
        maker_coin.my_public_key().unwrap(),
    );
    let p2sh = dhash160(script.as_slice());

    // 0.1 of the MYCOIN
    let value = 1000000;
    let output = TransactionOutput {
        value,
        script_pubkey: Builder::build_p2sh(&AddressHashEnum::AddressHash(p2sh)).into(),
    };
    let dex_fee_tx = match send_outputs_from_my_address(taker_coin.clone(), vec![output])
        .wait()
        .unwrap()
    {
        TransactionEnum::UtxoTx(tx) => tx,
        _ => panic!("Got unexpected tx"),
    };

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
}
