use super::*;
use crate::utxo::rpc_clients::{BlockHashOrHeight, ElectrumClient, EstimateFeeMethod, UtxoRpcClientEnum};
use crate::utxo::utxo_standard::UtxoStandardCoin;
use crate::{MarketCoinOps, MmCoin};
use bitcoin::blockdata::block::BlockHeader;
use bitcoin::blockdata::script::Script;
use bitcoin::blockdata::transaction::Transaction;
use bitcoin::consensus::encode;
use bitcoin::hash_types::Txid;
use bitcoin_hashes::Hash;
use common::executor::spawn;
use common::log;
use derive_more::Display;
use futures::compat::Future01CompatExt;
use keys::hash::H256;
use lightning::chain::{chaininterface::{BroadcasterInterface, ConfirmationTarget, FeeEstimator},
                       Filter, WatchedOutput};
use rpc::v1::types::H256 as H256Json;
use std::cmp;
use std::convert::TryFrom;

const MIN_ALLOWED_FEE_PER_1000_WEIGHT: u32 = 253;

impl FeeEstimator for PlatformFields {
    // Gets estimated satoshis of fee required per 1000 Weight-Units.
    fn get_est_sat_per_1000_weight(&self, confirmation_target: ConfirmationTarget) -> u32 {
        let platform_coin = &self.platform_coin;

        let default_fee = match confirmation_target {
            ConfirmationTarget::Background => self.default_fees_and_confirmations.background.default_feerate,
            ConfirmationTarget::Normal => self.default_fees_and_confirmations.normal.default_feerate,
            ConfirmationTarget::HighPriority => self.default_fees_and_confirmations.high_priority.default_feerate,
        } * 4;

        let conf = &platform_coin.as_ref().conf;
        let n_blocks = match confirmation_target {
            ConfirmationTarget::Background => self.default_fees_and_confirmations.background.n_blocks,
            ConfirmationTarget::Normal => self.default_fees_and_confirmations.normal.n_blocks,
            ConfirmationTarget::HighPriority => self.default_fees_and_confirmations.high_priority.n_blocks,
        };
        let fee_per_kb = tokio::task::block_in_place(move || {
            platform_coin
                .as_ref()
                .rpc_client
                .estimate_fee_sat(
                    platform_coin.decimals(),
                    // Todo: when implementing Native client detect_fee_method should be used for Native and
                    // EstimateFeeMethod::Standard for Electrum
                    &EstimateFeeMethod::Standard,
                    &conf.estimate_fee_mode,
                    n_blocks,
                )
                .wait()
                .unwrap_or(default_fee)
        });
        // Must be no smaller than 253 (ie 1 satoshi-per-byte rounded up to ensure later round-downs don’t put us below 1 satoshi-per-byte).
        // https://docs.rs/lightning/0.0.101/lightning/chain/chaininterface/trait.FeeEstimator.html#tymethod.get_est_sat_per_1000_weight
        cmp::max((fee_per_kb as f64 / 4.0).ceil() as u32, MIN_ALLOWED_FEE_PER_1000_WEIGHT)
    }
}

impl BroadcasterInterface for UtxoStandardCoin {
    fn broadcast_transaction(&self, tx: &Transaction) {
        let tx_hex = encode::serialize_hex(tx);
        log::debug!("Trying to broadcast transaction: {}", tx_hex);
        let tx_id = tx.txid();
        let fut = self.send_raw_tx(&tx_hex);
        spawn(async move {
            match fut.compat().await {
                Ok(id) => log::info!("Transaction broadcasted successfully: {:?} ", id),
                Err(e) => log::error!("Broadcast transaction {} failed: {}", tx_id, e),
            }
        });
    }
}

#[derive(Debug, Display)]
pub enum FindWatchedOutputSpendError {
    #[display(fmt = "Can't convert transaction: {}", _0)]
    TransactionConvertionErr(String),
    #[display(fmt = "Can't deserialize block header: {}", _0)]
    BlockHeaderDeserializeErr(String),
}

pub async fn find_watched_output_spend_with_header(
    electrum_client: &ElectrumClient,
    output: &WatchedOutput,
) -> Result<Option<(BlockHeader, usize, Transaction, u64)>, FindWatchedOutputSpendError> {
    // from_block parameter is not used in find_output_spend for electrum clients
    let utxo_client: UtxoRpcClientEnum = electrum_client.clone().into();
    let output_spend = match utxo_client
        .find_output_spend(
            H256::from(output.outpoint.txid.as_hash().into_inner()),
            output.script_pubkey.as_ref(),
            output.outpoint.index.into(),
            BlockHashOrHeight::Hash(Default::default()),
        )
        .compat()
        .await
    {
        Ok(Some(output)) => output,
        _ => return Ok(None),
    };

    if let BlockHashOrHeight::Height(height) = output_spend.spent_in_block {
        if let Ok(header) = electrum_client.blockchain_block_header(height as u64).compat().await {
            match encode::deserialize(&header) {
                Ok(h) => {
                    let spending_tx = match Transaction::try_from(output_spend.spending_tx) {
                        Ok(tx) => tx,
                        Err(e) => return Err(FindWatchedOutputSpendError::TransactionConvertionErr(e.to_string())),
                    };
                    return Ok(Some((h, output_spend.input_index, spending_tx, height as u64)));
                },
                Err(e) => return Err(FindWatchedOutputSpendError::BlockHeaderDeserializeErr(e.to_string())),
            }
        }
    }
    Ok(None)
}

impl Filter for PlatformFields {
    // Watches for this transaction on-chain
    fn register_tx(&self, txid: &Txid, script_pubkey: &Script) { self.add_tx(txid, script_pubkey); }

    // Watches for any transactions that spend this output on-chain
    fn register_output(&self, output: WatchedOutput) -> Option<(usize, Transaction)> {
        self.add_output(output.clone());

        let block_hash = match output.block_hash {
            Some(h) => H256Json::from(h.as_hash().into_inner()),
            None => return None,
        };

        let client = &self.platform_coin.as_ref().rpc_client;
        // Although this works for both native and electrum clients as the block hash is available,
        // the filter interface which includes register_output and register_tx should be used for electrum clients only,
        // this is the reason for initializing the filter as an option in the start_lightning function as it will be None
        // when implementing lightning for native clients
        let output_spend_fut = tokio::task::block_in_place(move || {
            client
                .find_output_spend(
                    H256::from(output.outpoint.txid.as_hash().into_inner()),
                    output.script_pubkey.as_ref(),
                    output.outpoint.index.into(),
                    BlockHashOrHeight::Hash(block_hash),
                )
                .wait()
        });

        match output_spend_fut {
            Ok(Some(spent_output_info)) => {
                let spending_tx = match Transaction::try_from(spent_output_info.spending_tx) {
                    Ok(tx) => tx,
                    Err(e) => {
                        log::error!("Can't convert transaction error: {}", e.to_string());
                        return None;
                    },
                };
                Some((spent_output_info.input_index, spending_tx))
            },
            Ok(None) => None,
            Err(e) => {
                log::error!("Error when calling register_output: {}", e);
                None
            },
        }
    }
}
