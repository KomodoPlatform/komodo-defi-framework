use crate::{generate_utxo_coin_with_random_privkey, MYCOIN};
use bitcrypto::ripemd160;
use chain::constants::SEQUENCE_FINAL;
use chain::{OutPoint, TransactionInput, TransactionOutput};
use coins::utxo::swap_proto_v2_scripts::dex_fee_script;
use coins::utxo::utxo_common::{send_outputs_from_my_address, P2SHSpendingTxInput, DEFAULT_SWAP_TX_SPEND_SIZE};
use coins::utxo::{output_script, ScriptType, UtxoCommonOps, UtxoTxBroadcastOps};
use coins::{FeeApproxStage, MarketCoinOps, SwapOps, TransactionEnum};
use common::{block_on, now_sec_u32};
use futures01::Future;
use keys::AddressHashEnum;
use primitives::hash::{H160, H256};
use script::{Builder, Opcode, SignerHashAlgo, TransactionInputSigner, UnsignedTransactionInput};

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
    let p2sh = ripemd160(script.as_slice());

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
    let (_mm_arc, coin, _privkey) = generate_utxo_coin_with_random_privkey(MYCOIN, 1000.into());

    let timelock = now_sec_u32() - 1000;
    let script = dex_fee_script(
        timelock,
        &[0; 20],
        coin.my_public_key().unwrap(),
        coin.my_public_key().unwrap(),
    );
    let p2sh = ripemd160(script.as_slice());

    // 0.1 of the MYCOIN
    let value = 1000000;
    let output = TransactionOutput {
        value,
        script_pubkey: Builder::build_p2sh(&AddressHashEnum::AddressHash(p2sh)).into(),
    };
    let dex_fee_tx = send_outputs_from_my_address(coin.clone(), vec![output]).wait().unwrap();
    let tx_hash: H256 = dex_fee_tx.tx_hash().as_slice().into();

    let input_to_spend = UnsignedTransactionInput {
        previous_output: OutPoint {
            hash: tx_hash.reversed(),
            index: 0,
        },
        sequence: SEQUENCE_FINAL,
        amount: value,
        witness: vec![],
    };

    let fee = block_on(coin.get_htlc_spend_fee(DEFAULT_SWAP_TX_SPEND_SIZE, &FeeApproxStage::WithoutApprox)).unwrap();
    let my_address = coin.as_ref().derivation_method.single_addr_or_err().unwrap();
    let output = TransactionOutput {
        value: value - fee,
        script_pubkey: output_script(my_address, ScriptType::P2PKH).into(),
    };

    let input_signer = TransactionInputSigner {
        version: coin.as_ref().conf.tx_version,
        n_time: None,
        overwintered: coin.as_ref().conf.overwintered,
        version_group_id: coin.as_ref().conf.version_group_id,
        consensus_branch_id: coin.as_ref().conf.consensus_branch_id,
        expiry_height: 0,
        value_balance: 0,
        inputs: vec![input_to_spend],
        outputs: vec![output],
        lock_time: timelock,
        join_splits: vec![],
        shielded_spends: vec![],
        shielded_outputs: vec![],
        zcash: coin.as_ref().conf.zcash,
        posv: false,
        str_d_zeel: None,
        hash_algo: SignerHashAlgo::SHA256,
    };
}
