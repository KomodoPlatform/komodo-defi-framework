// historical milestone, first performed RICK/ZOMBIE swap
// dex fee - https://zombie.explorer.lordofthechains.com/tx/40bec29f268c349722a3228743e5c5b461cf16d124cddcfd2fc624fe895a0bdd
// maker payment - https://rick.explorer.dexstats.info/tx/9d36e95e5147450399895f0f248ac2e2de13382401c2986e134cc3d62bda738e
// taker payment - https://zombie.explorer.lordofthechains.com/tx/b248992e064fab579774c0479b04043091cf62f3975cb1664ea7d4f857ebe6f8
// taker payment spend - https://zombie.explorer.lordofthechains.com/tx/af6bb0f99f9a5a070a0c1f53d69e4189b0e9b68f9d66e69f201a6b6d9f93897e
// maker payment spend - https://rick.explorer.dexstats.info/tx/6a2dcc866ad75cebecb780a02320073a88bcf5e57ddccbe2657494e7747d591e

use super::z_rpc::{ZOperationStatus, ZOperationTxid, ZSendManyItem};
use super::ZCoin;
use crate::utxo::rpc_clients::{UtxoRpcClientEnum, UtxoRpcError};
use crate::utxo::sat_from_big_decimal;
use crate::utxo::utxo_common::{big_decimal_from_sat_unsigned, dex_fee_script, payment_script};
use bigdecimal::BigDecimal;
use bitcrypto::dhash160;
use chain::Transaction as UtxoTx;
use common::executor::Timer;
use common::mm_error::prelude::*;
use common::now_ms;
use derive_more::Display;
use futures::compat::Future01CompatExt;
use keys::{Address, Public};
use script::Script;
use secp256k1_bindings::SecretKey;
use serialization::deserialize;
use zcash_primitives::consensus;
use zcash_primitives::legacy::Script as ZCashScript;
use zcash_primitives::transaction::builder::{Builder as ZTxBuilder, Error as ZTxBuilderError};
use zcash_primitives::transaction::components::{Amount, OutPoint as ZCashOutpoint, TxOut};

#[derive(Debug, Display)]
#[allow(clippy::large_enum_variant)]
pub enum ZSendHtlcError {
    #[display(fmt = "z operation failed with statuses {:?}", _0)]
    ZOperationFailed(Vec<ZOperationStatus<ZOperationTxid>>),
    ZOperationStatusesEmpty,
    RpcError(UtxoRpcError),
}

impl From<UtxoRpcError> for ZSendHtlcError {
    fn from(rpc: UtxoRpcError) -> ZSendHtlcError { ZSendHtlcError::RpcError(rpc) }
}

/// Sends HTLC output from the coin's z_addr
pub async fn z_send_htlc(
    coin: &ZCoin,
    time_lock: u32,
    other_pub: &Public,
    secret_hash: &[u8],
    amount: BigDecimal,
) -> Result<UtxoTx, MmError<ZSendHtlcError>> {
    let _lock = coin.z_fields.z_tx_mutex.lock().await;
    let payment_script = payment_script(time_lock, secret_hash, coin.utxo_arc.key_pair.public(), &other_pub);
    let hash = dhash160(&payment_script);
    let htlc_address = Address {
        prefix: coin.utxo_arc.conf.p2sh_addr_prefix,
        t_addr_prefix: coin.utxo_arc.conf.p2sh_t_addr_prefix,
        hash,
        checksum_type: coin.utxo_arc.conf.checksum_type,
    };

    let amount_sat = sat_from_big_decimal(&amount, coin.utxo_arc.decimals).expect("temporary code");
    let amount = big_decimal_from_sat_unsigned(amount_sat, coin.utxo_arc.decimals);
    let address = htlc_address.to_string();
    if let UtxoRpcClientEnum::Native(native) = coin.rpc_client() {
        native.import_address(&address, &address, false).compat().await.unwrap();
    }

    let send_item = ZSendManyItem {
        amount,
        op_return: Some(payment_script.to_vec().into()),
        address: htlc_address.to_string(),
    };

    let op_id = coin
        .z_rpc()
        .z_send_many(&coin.z_fields.z_addr_encoded, vec![send_item])
        .compat()
        .await?;

    loop {
        let operation_statuses = coin.z_rpc().z_get_send_many_status(&[&op_id]).compat().await?;

        match operation_statuses.first() {
            Some(ZOperationStatus::Executing { .. }) | Some(ZOperationStatus::Queued { .. }) => {
                Timer::sleep(1.).await;
                continue;
            },
            Some(ZOperationStatus::Failed { .. }) => {
                break Err(MmError::new(ZSendHtlcError::ZOperationFailed(operation_statuses)));
            },
            Some(ZOperationStatus::Success { result, .. }) => {
                let tx_bytes = coin
                    .rpc_client()
                    .get_transaction_bytes(result.txid.clone())
                    .compat()
                    .await?;
                let tx: UtxoTx = deserialize(tx_bytes.0.as_slice()).expect("rpc returns valid tx bytes");
                break Ok(tx);
            },
            None => break Err(MmError::new(ZSendHtlcError::ZOperationStatusesEmpty)),
        }
    }
}

/// Sends HTLC output from the coin's z_addr
pub async fn z_send_dex_fee(
    coin: &ZCoin,
    time_lock: u32,
    watcher_pub: &Public,
    amount: BigDecimal,
) -> Result<(UtxoTx, Script), MmError<ZSendHtlcError>> {
    let _lock = coin.z_fields.z_tx_mutex.lock().await;
    let payment_script = dex_fee_script([0; 16], time_lock, watcher_pub, coin.utxo_arc.key_pair.public());
    let hash = dhash160(&payment_script);
    let htlc_address = Address {
        prefix: coin.utxo_arc.conf.p2sh_addr_prefix,
        t_addr_prefix: coin.utxo_arc.conf.p2sh_t_addr_prefix,
        hash,
        checksum_type: coin.utxo_arc.conf.checksum_type,
    };

    let amount_sat = sat_from_big_decimal(&amount, coin.utxo_arc.decimals).expect("temporary code");
    let amount = big_decimal_from_sat_unsigned(amount_sat, coin.utxo_arc.decimals);

    let address = htlc_address.to_string();
    if let UtxoRpcClientEnum::Native(native) = coin.rpc_client() {
        native.import_address(&address, &address, false).compat().await.unwrap();
    }

    let send_item = ZSendManyItem {
        amount,
        op_return: Some(payment_script.to_vec().into()),
        address,
    };

    let op_id = coin
        .z_rpc()
        .z_send_many(&coin.z_fields.z_addr_encoded, vec![send_item])
        .compat()
        .await?;

    loop {
        let operation_statuses = coin.z_rpc().z_get_send_many_status(&[&op_id]).compat().await?;

        match operation_statuses.first() {
            Some(ZOperationStatus::Executing { .. }) | Some(ZOperationStatus::Queued { .. }) => {
                Timer::sleep(1.).await;
                continue;
            },
            Some(ZOperationStatus::Failed { .. }) => {
                break Err(MmError::new(ZSendHtlcError::ZOperationFailed(operation_statuses)));
            },
            Some(ZOperationStatus::Success { result, .. }) => {
                let tx_bytes = coin
                    .rpc_client()
                    .get_transaction_bytes(result.txid.clone())
                    .compat()
                    .await?;
                let tx: UtxoTx = deserialize(tx_bytes.0.as_slice()).expect("rpc returns valid tx bytes");

                coin.rpc_client()
                    .wait_for_confirmations(&tx, 1, false, now_ms() / 1000 + 120, 1)
                    .compat()
                    .await
                    .unwrap();
                break Ok((tx, payment_script));
            },
            None => break Err(MmError::new(ZSendHtlcError::ZOperationStatusesEmpty)),
        }
    }
}

#[derive(Debug, Display)]
#[allow(clippy::large_enum_variant)]
pub enum ZP2SHSpendError {
    ZTxBuilderError(ZTxBuilderError),
    Rpc(UtxoRpcError),
}

impl From<ZTxBuilderError> for ZP2SHSpendError {
    fn from(tx_builder: ZTxBuilderError) -> ZP2SHSpendError { ZP2SHSpendError::ZTxBuilderError(tx_builder) }
}

impl From<UtxoRpcError> for ZP2SHSpendError {
    fn from(rpc: UtxoRpcError) -> ZP2SHSpendError { ZP2SHSpendError::Rpc(rpc) }
}

/// Spends P2SH output 0 to the coin's z_addr
pub async fn z_p2sh_spend(
    coin: &ZCoin,
    p2sh_tx: UtxoTx,
    tx_locktime: u32,
    input_sequence: u32,
    redeem_script: Script,
    script_data: Script,
) -> Result<UtxoTx, MmError<ZP2SHSpendError>> {
    let current_block = coin.utxo_arc.rpc_client.get_block_count().compat().await? as u32;
    let mut tx_builder = ZTxBuilder::new(consensus::MAIN_NETWORK, current_block.into());
    tx_builder.set_lock_time(tx_locktime);

    let secp_secret =
        SecretKey::from_slice(&*coin.utxo_arc.key_pair.private().secret).expect("Keypair contains a valid secret key");

    let outpoint = ZCashOutpoint::new(p2sh_tx.hash().into(), 0);
    let tx_out = TxOut {
        value: Amount::from_u64(p2sh_tx.outputs[0].value).expect("p2sh_tx transaction always contains valid amount"),
        script_pubkey: ZCashScript(redeem_script.to_vec()),
    };
    tx_builder
        .add_transparent_input(
            secp_secret,
            outpoint,
            input_sequence,
            ZCashScript(script_data.to_vec()),
            tx_out,
        )
        .map_to_mm(ZP2SHSpendError::from)?;
    tx_builder
        .add_sapling_output(
            None,
            coin.z_fields.z_addr.clone(),
            Amount::from_u64(p2sh_tx.outputs[0].value - 1000).unwrap(),
            None,
        )
        .map_to_mm(ZP2SHSpendError::from)?;

    let (zcash_tx, _) = tx_builder
        .build(consensus::BranchId::Sapling, &coin.z_fields.z_tx_prover)
        .map_to_mm(ZP2SHSpendError::from)?;

    let mut tx_buffer = Vec::with_capacity(1024);
    zcash_tx.write(&mut tx_buffer).unwrap();
    let refund_tx: UtxoTx = deserialize(tx_buffer.as_slice()).expect("librustzcash should produce a valid tx");

    coin.rpc_client()
        .send_raw_transaction(tx_buffer.into())
        .compat()
        .await?;

    Ok(refund_tx)
}
