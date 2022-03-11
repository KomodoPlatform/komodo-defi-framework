use super::*;
use crate::utxo::rpc_clients::{electrum_script_hash, BestBlock as RpcBestBlock, BlockHashOrHeight,
                               ElectrumBlockHeader, ElectrumClient, ElectrumNonce, EstimateFeeMethod,
                               UtxoRpcClientEnum, UtxoRpcError};
use crate::utxo::utxo_standard::UtxoStandardCoin;
use crate::{MarketCoinOps, MmCoin};
use bitcoin::blockdata::block::BlockHeader;
use bitcoin::blockdata::script::Script;
use bitcoin::blockdata::transaction::Transaction;
use bitcoin::consensus::encode::{deserialize, serialize_hex};
use bitcoin::hash_types::{BlockHash, TxMerkleNode, Txid};
use bitcoin_hashes::{sha256d, Hash};
use common::executor::{spawn, Timer};
use common::jsonrpc_client::JsonRpcErrorType;
use common::log;
use derive_more::Display;
use futures::compat::Future01CompatExt;
use keys::hash::H256;
use lightning::chain::{chaininterface::{BroadcasterInterface, ConfirmationTarget, FeeEstimator},
                       Confirm, Filter, WatchedOutput};
use rpc::v1::types::H256 as H256Json;
use std::cmp;
use std::convert::{TryFrom, TryInto};

const CHECK_FOR_NEW_BEST_BLOCK_INTERVAL: u64 = 60;
const MIN_ALLOWED_FEE_PER_1000_WEIGHT: u32 = 253;

#[derive(Debug, Display)]
pub enum FindWatchedOutputSpendError {
    #[display(fmt = "Can't convert transaction: {}", _0)]
    TransactionConvertionErr(String),
    #[display(fmt = "Can't deserialize block header: {}", _0)]
    BlockHeaderDeserializeErr(String),
}

async fn find_watched_output_spend_with_header(
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
            match deserialize(&header) {
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

pub async fn get_best_header(best_header_listener: &ElectrumClient) -> EnableLightningResult<ElectrumBlockHeader> {
    best_header_listener
        .blockchain_headers_subscribe()
        .compat()
        .await
        .map_to_mm(|e| EnableLightningError::RpcError(e.to_string()))
}

pub async fn update_best_block(
    chain_monitor: Arc<ChainMonitor>,
    channel_manager: Arc<ChannelManager>,
    best_header: ElectrumBlockHeader,
) {
    {
        let (new_best_header, new_best_height) = match best_header {
            ElectrumBlockHeader::V12(h) => {
                let nonce = match h.nonce {
                    ElectrumNonce::Number(n) => n as u32,
                    ElectrumNonce::Hash(_) => {
                        return;
                    },
                };
                let prev_blockhash = match sha256d::Hash::from_slice(&h.prev_block_hash.0) {
                    Ok(h) => h,
                    Err(e) => {
                        log::error!("Error while parsing previous block hash for lightning node: {}", e);
                        return;
                    },
                };
                let merkle_root = match sha256d::Hash::from_slice(&h.merkle_root.0) {
                    Ok(h) => h,
                    Err(e) => {
                        log::error!("Error while parsing merkle root for lightning node: {}", e);
                        return;
                    },
                };
                (
                    BlockHeader {
                        version: h.version as i32,
                        prev_blockhash: BlockHash::from_hash(prev_blockhash),
                        merkle_root: TxMerkleNode::from_hash(merkle_root),
                        time: h.timestamp as u32,
                        bits: h.bits as u32,
                        nonce,
                    },
                    h.block_height as u32,
                )
            },
            ElectrumBlockHeader::V14(h) => {
                let block_header = match deserialize(&h.hex.into_vec()) {
                    Ok(header) => header,
                    Err(e) => {
                        log::error!("Block header deserialization error: {}", e.to_string());
                        return;
                    },
                };
                (block_header, h.height as u32)
            },
        };
        channel_manager.best_block_updated(&new_best_header, new_best_height);
        chain_monitor.best_block_updated(&new_best_header, new_best_height);
    }
}

pub async fn ln_best_block_update_loop(
    platform: Arc<Platform>,
    chain_monitor: Arc<ChainMonitor>,
    channel_manager: Arc<ChannelManager>,
    best_header_listener: ElectrumClient,
    best_block: RpcBestBlock,
) {
    let mut current_best_block = best_block;
    loop {
        let best_header = match get_best_header(&best_header_listener).await {
            Ok(h) => h,
            Err(e) => {
                log::error!("Error while requesting best header for lightning node: {}", e);
                Timer::sleep(CHECK_FOR_NEW_BEST_BLOCK_INTERVAL as f64).await;
                continue;
            },
        };
        if current_best_block != best_header.clone().into() {
            platform
                .process_txs_unconfirmations(chain_monitor.clone(), channel_manager.clone())
                .await;
            platform
                .process_txs_confirmations(
                    best_header_listener.clone(),
                    chain_monitor.clone(),
                    channel_manager.clone(),
                    best_header.block_height(),
                )
                .await;
            current_best_block = best_header.clone().into();
            update_best_block(chain_monitor.clone(), channel_manager.clone(), best_header).await;
        }
        Timer::sleep(CHECK_FOR_NEW_BEST_BLOCK_INTERVAL as f64).await;
    }
}

struct ConfirmedTransactionInfo {
    txid: Txid,
    header: BlockHeader,
    index: usize,
    transaction: Transaction,
    height: u32,
}

impl ConfirmedTransactionInfo {
    fn new(txid: Txid, header: BlockHeader, index: usize, transaction: Transaction, height: u32) -> Self {
        ConfirmedTransactionInfo {
            txid,
            header,
            index,
            transaction,
            height,
        }
    }
}

pub struct Platform {
    pub coin: UtxoStandardCoin,
    /// Main/testnet/signet/regtest Needed for lightning node to know which network to connect to
    pub network: BlockchainNetwork,
    // Default fees to and confirmation targets to be used for FeeEstimator. Default fees are used when the call for
    // estimate_fee_sat fails.
    pub default_fees_and_confirmations: PlatformCoinConfirmations,
    // This cache stores the transactions that the LN node has interest in.
    pub registered_txs: PaMutex<HashMap<Txid, HashSet<Script>>>,
    // This cache stores the outputs that the LN node has interest in.
    pub registered_outputs: PaMutex<Vec<WatchedOutput>>,
    // This cache stores transactions to be broadcasted once the other node accepts the channel
    pub unsigned_funding_txs: PaMutex<HashMap<u64, TransactionInputSigner>>,
}

impl Platform {
    pub fn new(
        coin: UtxoStandardCoin,
        network: BlockchainNetwork,
        default_fees_and_confirmations: PlatformCoinConfirmations,
    ) -> Self {
        Platform {
            coin,
            network,
            default_fees_and_confirmations,
            registered_txs: PaMutex::new(HashMap::new()),
            registered_outputs: PaMutex::new(Vec::new()),
            unsigned_funding_txs: PaMutex::new(HashMap::new()),
        }
    }

    pub fn add_tx(&self, txid: &Txid, script_pubkey: &Script) {
        let mut registered_txs = self.registered_txs.lock();
        match registered_txs.get_mut(txid) {
            Some(h) => {
                h.insert(script_pubkey.clone());
            },
            None => {
                let mut script_pubkeys = HashSet::new();
                script_pubkeys.insert(script_pubkey.clone());
                registered_txs.insert(*txid, script_pubkeys);
            },
        }
    }

    pub fn add_output(&self, output: WatchedOutput) {
        let mut registered_outputs = self.registered_outputs.lock();
        registered_outputs.push(output);
    }

    async fn process_tx_for_unconfirmation<T>(&self, txid: Txid, monitor: Arc<T>)
    where
        T: Confirm,
    {
        if let Err(err) = self
            .coin
            .as_ref()
            .rpc_client
            .get_transaction_bytes(&H256Json::from(txid.as_hash().into_inner()).reversed())
            .compat()
            .await
            .map_err(|e| e.into_inner())
        {
            if let UtxoRpcError::ResponseParseError(ref json_err) = err {
                if let JsonRpcErrorType::Response(_, json) = &json_err.error {
                    if let Some(message) = json["message"].as_str() {
                        if message.contains("'code': -5") {
                            log::info!(
                                "Transaction {} is not found on chain :{}. The transaction will be re-broadcasted.",
                                txid,
                                err
                            );
                            monitor.transaction_unconfirmed(&txid);
                        }
                    }
                }
            }
            log::error!(
                "Error while trying to check if the transaction {} is discarded or not :{}",
                txid,
                err
            );
        }
    }

    pub async fn process_txs_unconfirmations(
        &self,
        chain_monitor: Arc<ChainMonitor>,
        channel_manager: Arc<ChannelManager>,
    ) {
        // Retrieve channel manager transaction IDs to check the chain for un-confirmations
        let channel_manager_relevant_txids = channel_manager.get_relevant_txids();
        for txid in channel_manager_relevant_txids {
            self.process_tx_for_unconfirmation(txid, channel_manager.clone()).await;
        }

        // Retrieve chain monitor transaction IDs to check the chain for un-confirmations
        let chain_monitor_relevant_txids = chain_monitor.get_relevant_txids();
        for txid in chain_monitor_relevant_txids {
            self.process_tx_for_unconfirmation(txid, chain_monitor.clone()).await;
        }
    }

    async fn get_confirmed_registered_txs(
        &self,
        client: &ElectrumClient,
        current_height: u64,
    ) -> Vec<ConfirmedTransactionInfo> {
        let registered_txs = self.registered_txs.lock().clone();
        let mut confirmed_registered_txs = Vec::new();
        for (txid, scripts) in registered_txs {
            let rpc_txid = H256Json::from(txid.as_hash().into_inner()).reversed();
            match self
                .coin
                .as_ref()
                .rpc_client
                .get_transaction_bytes(&rpc_txid)
                .compat()
                .await
            {
                Ok(bytes) => {
                    let transaction: Transaction = match deserialize(&bytes.into_vec()) {
                        Ok(tx) => tx,
                        Err(e) => {
                            log::error!("Transaction deserialization error: {}", e.to_string());
                            continue;
                        },
                    };
                    for (_, vout) in transaction.output.iter().enumerate() {
                        if scripts.contains(&vout.script_pubkey) {
                            let script_hash = hex::encode(electrum_script_hash(vout.script_pubkey.as_ref()));
                            let history = client
                                .scripthash_get_history(&script_hash)
                                .compat()
                                .await
                                .unwrap_or_default();
                            for item in history {
                                if item.tx_hash == rpc_txid {
                                    // If a new block mined the transaction while running process_txs_confirmations it will be confirmed later in ln_best_block_update_loop
                                    if item.height > 0 && item.height <= current_height as i64 {
                                        let height: u64 = match item.height.try_into() {
                                            Ok(h) => h,
                                            Err(e) => {
                                                log::error!("Block height convertion to u64 error: {}", e.to_string());
                                                continue;
                                            },
                                        };
                                        let header = match client.blockchain_block_header(height).compat().await {
                                            Ok(block_header) => match deserialize(&block_header) {
                                                Ok(h) => h,
                                                Err(e) => {
                                                    log::error!(
                                                        "Block header deserialization error: {}",
                                                        e.to_string()
                                                    );
                                                    continue;
                                                },
                                            },
                                            Err(_) => continue,
                                        };
                                        let index = match client
                                            .blockchain_transaction_get_merkle(rpc_txid, height)
                                            .compat()
                                            .await
                                        {
                                            Ok(merkle_branch) => merkle_branch.pos,
                                            Err(e) => {
                                                log::error!(
                                                    "Error getting transaction position in the block: {}",
                                                    e.to_string()
                                                );
                                                continue;
                                            },
                                        };
                                        let confirmed_transaction_info = ConfirmedTransactionInfo::new(
                                            txid,
                                            header,
                                            index,
                                            transaction.clone(),
                                            height as u32,
                                        );
                                        confirmed_registered_txs.push(confirmed_transaction_info);
                                        self.registered_txs.lock().remove(&txid);
                                    }
                                }
                            }
                        }
                    }
                },
                Err(e) => {
                    log::error!("Error getting transaction {} from chain: {}", txid, e);
                    continue;
                },
            };
        }
        confirmed_registered_txs
    }

    async fn append_spent_registered_output_txs(
        &self,
        transactions_to_confirm: &mut Vec<ConfirmedTransactionInfo>,
        client: &ElectrumClient,
    ) {
        let mut outputs_to_remove = Vec::new();
        let registered_outputs = self.registered_outputs.lock().clone();
        for output in registered_outputs {
            let result = match find_watched_output_spend_with_header(client, &output).await {
                Ok(res) => res,
                Err(e) => {
                    log::error!(
                        "Error while trying to find if the registered output {:?} is spent: {}",
                        output.outpoint,
                        e
                    );
                    continue;
                },
            };
            if let Some((header, _, tx, height)) = result {
                if !transactions_to_confirm.iter().any(|info| info.txid == tx.txid()) {
                    let rpc_txid = H256Json::from(tx.txid().as_hash().into_inner()).reversed();
                    let index = match client
                        .blockchain_transaction_get_merkle(rpc_txid, height)
                        .compat()
                        .await
                    {
                        Ok(merkle_branch) => merkle_branch.pos,
                        Err(e) => {
                            log::error!("Error getting transaction position in the block: {}", e.to_string());
                            continue;
                        },
                    };
                    let confirmed_transaction_info =
                        ConfirmedTransactionInfo::new(tx.txid(), header, index, tx, height as u32);
                    transactions_to_confirm.push(confirmed_transaction_info);
                }
                outputs_to_remove.push(output);
            }
        }
        self.registered_outputs
            .lock()
            .retain(|output| !outputs_to_remove.contains(output));
    }

    pub async fn process_txs_confirmations(
        &self,
        client: ElectrumClient,
        chain_monitor: Arc<ChainMonitor>,
        channel_manager: Arc<ChannelManager>,
        current_height: u64,
    ) {
        let mut transactions_to_confirm = self.get_confirmed_registered_txs(&client, current_height).await;
        self.append_spent_registered_output_txs(&mut transactions_to_confirm, &client)
            .await;

        transactions_to_confirm.sort_by(|a, b| {
            let block_order = a.height.cmp(&b.height);
            match block_order {
                cmp::Ordering::Equal => a.index.cmp(&b.index),
                _ => block_order,
            }
        });

        for confirmed_transaction_info in transactions_to_confirm {
            channel_manager.transactions_confirmed(
                &confirmed_transaction_info.header,
                &[(
                    confirmed_transaction_info.index,
                    &confirmed_transaction_info.transaction,
                )],
                confirmed_transaction_info.height,
            );
            chain_monitor.transactions_confirmed(
                &confirmed_transaction_info.header,
                &[(
                    confirmed_transaction_info.index,
                    &confirmed_transaction_info.transaction,
                )],
                confirmed_transaction_info.height,
            );
        }
    }
}

impl FeeEstimator for Platform {
    // Gets estimated satoshis of fee required per 1000 Weight-Units.
    fn get_est_sat_per_1000_weight(&self, confirmation_target: ConfirmationTarget) -> u32 {
        let platform_coin = &self.coin;

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

impl BroadcasterInterface for Platform {
    fn broadcast_transaction(&self, tx: &Transaction) {
        let tx_hex = serialize_hex(tx);
        log::debug!("Trying to broadcast transaction: {}", tx_hex);
        let tx_id = tx.txid();
        let fut = self.coin.send_raw_tx(&tx_hex);
        spawn(async move {
            match fut.compat().await {
                Ok(id) => log::info!("Transaction broadcasted successfully: {:?} ", id),
                Err(e) => log::error!("Broadcast transaction {} failed: {}", tx_id, e),
            }
        });
    }
}

impl Filter for Platform {
    // Watches for this transaction on-chain
    fn register_tx(&self, txid: &Txid, script_pubkey: &Script) { self.add_tx(txid, script_pubkey); }

    // Watches for any transactions that spend this output on-chain
    fn register_output(&self, output: WatchedOutput) -> Option<(usize, Transaction)> {
        self.add_output(output.clone());

        let block_hash = match output.block_hash {
            Some(h) => H256Json::from(h.as_hash().into_inner()),
            None => return None,
        };

        let client = &self.coin.as_ref().rpc_client;
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
