use crate::utxo::rpc_clients::{electrum_script_hash, ElectrumClient, UtxoRpcClientOps, UtxoRpcError};
use bitcoin::blockdata::script::Script;
use bitcoin::blockdata::transaction::Transaction;
use bitcoin::consensus::encode;
use bitcoin::hash_types::Txid;
use common::mm_error::prelude::MapToMmFutureExt;
use futures::compat::Future01CompatExt;
use lightning::chain::{chaininterface::{BroadcasterInterface, ConfirmationTarget, FeeEstimator},
                       Filter, WatchedOutput};
use rpc::v1::types::Bytes as BytesJson;

#[cfg_attr(test, mockable)]
impl FeeEstimator for ElectrumClient {
    // Gets estimated satoshis of fee required per 1000 Weight-Units.
    // TODO: use fn estimate_fee instead of fixed number when starting work on opening channels
    fn get_est_sat_per_1000_weight(&self, confirmation_target: ConfirmationTarget) -> u32 {
        match confirmation_target {
            // fetch background feerate
            ConfirmationTarget::Background => 253,
            // fetch normal feerate (~6 blocks)
            ConfirmationTarget::Normal => 2000,
            // fetch high priority feerate
            ConfirmationTarget::HighPriority => 5000,
        }
    }
}

#[cfg_attr(test, mockable)]
impl BroadcasterInterface for ElectrumClient {
    fn broadcast_transaction(&self, tx: &Transaction) {
        let tx_bytes = BytesJson::from(encode::serialize_hex(tx).as_bytes());
        let _ = Box::new(
            self.blockchain_transaction_broadcast(tx_bytes)
                .map_to_mm_fut(UtxoRpcError::from),
        );
    }
}

#[cfg_attr(test, mockable)]
impl Filter for ElectrumClient {
    // Watches for this transaction on-chain
    fn register_tx(&self, _txid: &Txid, _script_pubkey: &Script) { unimplemented!() }

    // Watches for any transactions that spend this output on-chain
    // Todo: find a way to remove block_on
    fn register_output(&self, output: WatchedOutput) -> Option<(usize, Transaction)> {
        use common::block_on;
        let selfi = self.clone();
        let script_hash = hex::encode(electrum_script_hash(output.script_pubkey.as_ref()));
        let history = block_on(selfi.scripthash_get_history(&script_hash).compat()).unwrap_or_default();

        if history.len() < 2 {
            return None;
        }

        for item in history.iter() {
            let transaction = match block_on(selfi.get_transaction_bytes(item.tx_hash.clone()).compat()) {
                Ok(tx) => tx,
                Err(_) => continue,
            };

            let maybe_spend_tx: Transaction = match encode::deserialize(transaction.as_slice()) {
                Ok(tx) => tx,
                Err(_) => continue,
            };

            for (index, input) in maybe_spend_tx.input.iter().enumerate() {
                if input.previous_output.txid == output.outpoint.txid
                    && input.previous_output.vout == output.outpoint.index as u32
                {
                    return Some((index, maybe_spend_tx));
                }
            }
        }
        None
    }
}
