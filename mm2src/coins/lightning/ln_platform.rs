use super::*;
use crate::lightning::ln_errors::{SaveChannelClosingError, SaveChannelClosingResult};
use crate::utxo::rpc_clients::{BestBlock as RpcBestBlock, BlockHashOrHeight, ElectrumBlockHeader, ElectrumClient,
                               ElectrumNonce, EstimateFeeMethod, UtxoRpcClientEnum};
use crate::utxo::spv::{ConfirmedTransactionInfo, SimplePaymentVerification};
use crate::utxo::utxo_standard::UtxoStandardCoin;
use crate::{MarketCoinOps, MmCoin};
use bitcoin::blockdata::block::BlockHeader;
use bitcoin::blockdata::script::Script;
use bitcoin::blockdata::transaction::Transaction;
use bitcoin::consensus::encode::{deserialize, serialize_hex};
use bitcoin::hash_types::{BlockHash, TxMerkleNode, Txid};
use bitcoin_hashes::{sha256d, Hash};
use common::executor::{spawn, Timer};
use common::log::{debug, error, info};
use futures::compat::Future01CompatExt;
use futures::future::join_all;
use keys::hash::H256;
use lightning::chain::{chaininterface::{BroadcasterInterface, ConfirmationTarget, FeeEstimator},
                       Confirm, Filter, WatchedOutput};
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use spv_validation::spv_proof::TRY_SPV_PROOF_INTERVAL;
use std::cmp;
use std::convert::{TryFrom, TryInto};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

const CHECK_FOR_NEW_BEST_BLOCK_INTERVAL: f64 = 60.;
const MIN_ALLOWED_FEE_PER_1000_WEIGHT: u32 = 253;
const TRY_LOOP_INTERVAL: f64 = 60.;

#[inline]
pub fn h256_json_from_txid(txid: Txid) -> H256Json { H256Json::from(txid.as_hash().into_inner()).reversed() }

#[inline]
pub fn h256_from_txid(txid: Txid) -> H256 { H256::from(txid.as_hash().into_inner()) }

pub async fn get_best_header(best_header_listener: &ElectrumClient) -> EnableLightningResult<ElectrumBlockHeader> {
    best_header_listener
        .blockchain_headers_subscribe()
        .compat()
        .await
        .map_to_mm(|e| EnableLightningError::RpcError(e.to_string()))
}

pub async fn update_best_block(
    chain_monitor: &ChainMonitor,
    channel_manager: &ChannelManager,
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
                let prev_blockhash = sha256d::Hash::from_inner(h.prev_block_hash.0);
                let merkle_root = sha256d::Hash::from_inner(h.merkle_root.0);
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
                        error!("Block header deserialization error: {}", e.to_string());
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
    db: SqliteLightningDB,
    chain_monitor: Arc<ChainMonitor>,
    channel_manager: Arc<ChannelManager>,
    best_header_listener: ElectrumClient,
    best_block: RpcBestBlock,
) {
    let mut current_best_block = best_block;
    loop {
        // Transactions confirmations check can be done at every CHECK_FOR_NEW_BEST_BLOCK_INTERVAL instead of at every new block
        // in case a transaction confirmation fails due to electrums being down. This way there will be no need to wait for a new
        // block to confirm such transaction and causing delays.
        platform
            .process_txs_confirmations(&best_header_listener, &db, &chain_monitor, &channel_manager)
            .await;
        let best_header = ok_or_continue_after_sleep!(get_best_header(&best_header_listener).await, TRY_LOOP_INTERVAL);
        if current_best_block != best_header.clone().into() {
            platform.update_best_block_height(best_header.block_height());
            platform
                .process_txs_unconfirmations(&chain_monitor, &channel_manager)
                .await;
            current_best_block = best_header.clone().into();
            update_best_block(&chain_monitor, &channel_manager, best_header).await;
        }
        Timer::sleep(CHECK_FOR_NEW_BEST_BLOCK_INTERVAL).await;
    }
}

async fn get_funding_tx_bytes_loop(rpc_client: &UtxoRpcClientEnum, tx_hash: H256Json) -> BytesJson {
    loop {
        match rpc_client.get_transaction_bytes(&tx_hash).compat().await {
            Ok(res) => break res,
            Err(e) => {
                error!("error {}", e);
                Timer::sleep(TRY_LOOP_INTERVAL).await;
                continue;
            },
        }
    }
}

pub struct Platform {
    pub coin: UtxoStandardCoin,
    /// Main/testnet/signet/regtest Needed for lightning node to know which network to connect to
    pub network: BlockchainNetwork,
    /// The best block height.
    pub best_block_height: AtomicU64,
    /// Default fees to and confirmation targets to be used for FeeEstimator. Default fees are used when the call for
    /// estimate_fee_sat fails.
    pub default_fees_and_confirmations: PlatformCoinConfirmations,
    /// This cache stores the transactions that the LN node has interest in.
    pub registered_txs: PaMutex<HashSet<Txid>>,
    /// This cache stores the outputs that the LN node has interest in.
    pub registered_outputs: PaMutex<Vec<WatchedOutput>>,
    /// This cache stores transactions to be broadcasted once the other node accepts the channel
    pub unsigned_funding_txs: PaMutex<HashMap<u64, TransactionInputSigner>>,
}

impl Platform {
    #[inline]
    pub fn new(
        coin: UtxoStandardCoin,
        network: BlockchainNetwork,
        default_fees_and_confirmations: PlatformCoinConfirmations,
    ) -> Self {
        Platform {
            coin,
            network,
            best_block_height: AtomicU64::new(0),
            default_fees_and_confirmations,
            registered_txs: PaMutex::new(HashSet::new()),
            registered_outputs: PaMutex::new(Vec::new()),
            unsigned_funding_txs: PaMutex::new(HashMap::new()),
        }
    }

    #[inline]
    fn rpc_client(&self) -> &UtxoRpcClientEnum { &self.coin.as_ref().rpc_client }

    #[inline]
    pub fn update_best_block_height(&self, new_height: u64) {
        self.best_block_height.store(new_height, AtomicOrdering::Relaxed);
    }

    #[inline]
    pub fn best_block_height(&self) -> u64 { self.best_block_height.load(AtomicOrdering::Relaxed) }

    pub fn add_tx(&self, txid: Txid) {
        let mut registered_txs = self.registered_txs.lock();
        registered_txs.insert(txid);
    }

    pub fn add_output(&self, output: WatchedOutput) {
        let mut registered_outputs = self.registered_outputs.lock();
        registered_outputs.push(output);
    }

    async fn process_tx_for_unconfirmation<T>(&self, txid: Txid, monitor: &T)
    where
        T: Confirm,
    {
        let rpc_txid = h256_json_from_txid(txid);
        match self.rpc_client().get_tx_if_onchain(&rpc_txid).await {
            Ok(Some(_)) => {},
            Ok(None) => {
                info!(
                    "Transaction {} is not found on chain. The transaction will be re-broadcasted.",
                    txid,
                );
                monitor.transaction_unconfirmed(&txid);
                // If a transaction is unconfirmed due to a block reorganization; LDK will rebroadcast it.
                // In this case, this transaction needs to be added again to the registered transactions
                // to start watching for it on the chain again.
                self.add_tx(txid);
            },
            Err(e) => error!(
                "Error while trying to check if the transaction {} is discarded or not :{:?}",
                txid, e
            ),
        }
    }

    pub async fn process_txs_unconfirmations(&self, chain_monitor: &ChainMonitor, channel_manager: &ChannelManager) {
        // Retrieve channel manager transaction IDs to check the chain for un-confirmations
        let channel_manager_relevant_txids = channel_manager.get_relevant_txids();
        for txid in channel_manager_relevant_txids {
            self.process_tx_for_unconfirmation(txid, channel_manager).await;
        }

        // Retrieve chain monitor transaction IDs to check the chain for un-confirmations
        let chain_monitor_relevant_txids = chain_monitor.get_relevant_txids();
        for txid in chain_monitor_relevant_txids {
            self.process_tx_for_unconfirmation(txid, chain_monitor).await;
        }
    }

    async fn get_confirmed_registered_txs(&self, client: &ElectrumClient) -> Vec<ConfirmedTransactionInfo> {
        let registered_txs = self.registered_txs.lock().clone();

        let on_chain_txs_futs = registered_txs
            .into_iter()
            .map(|txid| async move {
                let rpc_txid = h256_json_from_txid(txid);
                self.rpc_client().get_tx_if_onchain(&rpc_txid).await
            })
            .collect::<Vec<_>>();
        let on_chain_txs = join_all(on_chain_txs_futs)
            .await
            .into_iter()
            .filter_map(|maybe_tx| match maybe_tx {
                Ok(maybe_tx) => maybe_tx,
                Err(e) => {
                    error!(
                        "Error while trying to figure if transaction is on-chain or not: {:?}",
                        e
                    );
                    None
                },
            });

        let confirmed_transactions_futs = on_chain_txs
            .map(|transaction| async move {
                client
                    .validate_spv_proof(&transaction, (now_ms() / 1000) + TRY_SPV_PROOF_INTERVAL)
                    .await
            })
            .collect::<Vec<_>>();
        join_all(confirmed_transactions_futs)
            .await
            .into_iter()
            .filter_map(|confirmed_transaction| match confirmed_transaction {
                Ok(confirmed_tx) => {
                    let txid = Txid::from_hash(confirmed_tx.tx.hash().reversed().to_sha256d());
                    self.registered_txs.lock().remove(&txid);
                    Some(confirmed_tx)
                },
                Err(e) => {
                    error!("Error verifying transaction: {:?}", e);
                    None
                },
            })
            .collect()
    }

    async fn append_spent_registered_output_txs(
        &self,
        transactions_to_confirm: &mut Vec<ConfirmedTransactionInfo>,
        client: &ElectrumClient,
    ) {
        let registered_outputs = self.registered_outputs.lock().clone();

        let spent_outputs_info_fut = registered_outputs
            .into_iter()
            .map(|output| async move {
                self.rpc_client()
                    .find_output_spend(
                        h256_from_txid(output.outpoint.txid),
                        output.script_pubkey.as_ref(),
                        output.outpoint.index.into(),
                        BlockHashOrHeight::Hash(Default::default()),
                    )
                    .compat()
                    .await
            })
            .collect::<Vec<_>>();
        let mut spent_outputs_info = join_all(spent_outputs_info_fut)
            .await
            .into_iter()
            .filter_map(|maybe_spent| match maybe_spent {
                Ok(maybe_spent) => maybe_spent,
                Err(e) => {
                    error!("Error while trying to figure if output is spent or not: {:?}", e);
                    None
                },
            })
            .collect::<Vec<_>>();
        spent_outputs_info.retain(|output| {
            !transactions_to_confirm
                .iter()
                .any(|info| info.tx.hash() == output.spending_tx.hash())
        });

        let confirmed_transactions_futs = spent_outputs_info
            .into_iter()
            .map(|output| async move {
                client
                    .validate_spv_proof(&output.spending_tx, (now_ms() / 1000) + TRY_SPV_PROOF_INTERVAL)
                    .await
            })
            .collect::<Vec<_>>();
        let mut confirmed_transaction_info = join_all(confirmed_transactions_futs)
            .await
            .into_iter()
            .filter_map(|confirmed_transaction| match confirmed_transaction {
                Ok(confirmed_tx) => {
                    self.registered_outputs.lock().retain(|output| {
                        !confirmed_tx
                            .tx
                            .clone()
                            .inputs
                            .into_iter()
                            .any(|txin| txin.previous_output.hash == h256_from_txid(output.outpoint.txid))
                    });
                    Some(confirmed_tx)
                },
                Err(e) => {
                    error!("Error verifying transaction: {:?}", e);
                    None
                },
            })
            .collect();

        transactions_to_confirm.append(&mut confirmed_transaction_info);
    }

    pub async fn process_txs_confirmations(
        &self,
        client: &ElectrumClient,
        db: &SqliteLightningDB,
        chain_monitor: &ChainMonitor,
        channel_manager: &ChannelManager,
    ) {
        let mut transactions_to_confirm = self.get_confirmed_registered_txs(client).await;
        self.append_spent_registered_output_txs(&mut transactions_to_confirm, client)
            .await;

        transactions_to_confirm.sort_by(|a, b| (a.height, a.index).cmp(&(b.height, b.index)));

        for confirmed_transaction_info in transactions_to_confirm {
            let best_block_height = self.best_block_height() as i64;
            if let Err(e) = db
                .update_funding_tx_block_height(
                    confirmed_transaction_info.tx.hash().reversed().to_string(),
                    best_block_height,
                )
                .await
            {
                error!("Unable to update the funding tx block height in DB: {}", e);
            }
            channel_manager.transactions_confirmed(
                &confirmed_transaction_info.header.clone().into(),
                &[(
                    confirmed_transaction_info.index as usize,
                    &confirmed_transaction_info.tx.clone().into(),
                )],
                confirmed_transaction_info.height as u32,
            );
            chain_monitor.transactions_confirmed(
                &confirmed_transaction_info.header.into(),
                &[(
                    confirmed_transaction_info.index as usize,
                    &confirmed_transaction_info.tx.into(),
                )],
                confirmed_transaction_info.height as u32,
            );
        }
    }

    pub async fn get_channel_closing_tx(&self, channel_details: DBChannelDetails) -> SaveChannelClosingResult<String> {
        let from_block = channel_details
            .funding_generated_in_block
            .ok_or_else(|| MmError::new(SaveChannelClosingError::BlockHeightNull))?;

        let tx_id = channel_details
            .funding_tx
            .ok_or_else(|| MmError::new(SaveChannelClosingError::FundingTxNull))?;

        let tx_hash =
            H256Json::from_str(&tx_id).map_to_mm(|e| SaveChannelClosingError::FundingTxParseError(e.to_string()))?;

        let funding_tx_bytes = get_funding_tx_bytes_loop(self.rpc_client(), tx_hash).await;

        let closing_tx = self
            .coin
            .wait_for_tx_spend(
                &funding_tx_bytes.into_vec(),
                (now_ms() / 1000) + 3600,
                from_block.try_into()?,
                &None,
            )
            .compat()
            .await
            .map_to_mm(|e| SaveChannelClosingError::WaitForFundingTxSpendError(e.get_plain_text_format()))?;

        let closing_tx_hash = format!("{:02x}", closing_tx.tx_hash());

        Ok(closing_tx_hash)
    }
}

impl FeeEstimator for Platform {
    // Gets estimated satoshis of fee required per 1000 Weight-Units.
    fn get_est_sat_per_1000_weight(&self, confirmation_target: ConfirmationTarget) -> u32 {
        let platform_coin = &self.coin;

        let default_fee = match confirmation_target {
            ConfirmationTarget::Background => self.default_fees_and_confirmations.background.default_fee_per_kb,
            ConfirmationTarget::Normal => self.default_fees_and_confirmations.normal.default_fee_per_kb,
            ConfirmationTarget::HighPriority => self.default_fees_and_confirmations.high_priority.default_fee_per_kb,
        };

        let conf = &platform_coin.as_ref().conf;
        let n_blocks = match confirmation_target {
            ConfirmationTarget::Background => self.default_fees_and_confirmations.background.n_blocks,
            ConfirmationTarget::Normal => self.default_fees_and_confirmations.normal.n_blocks,
            ConfirmationTarget::HighPriority => self.default_fees_and_confirmations.high_priority.n_blocks,
        };
        let fee_per_kb = tokio::task::block_in_place(move || {
            self.rpc_client()
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
        let txid = tx.txid();
        let tx_hex = serialize_hex(tx);
        debug!("Trying to broadcast transaction: {}", tx_hex);
        let fut = self.coin.send_raw_tx(&tx_hex);
        spawn(async move {
            match fut.compat().await {
                Ok(id) => info!("Transaction broadcasted successfully: {:?} ", id),
                // TODO: broadcast transaction through p2p network in case of error
                Err(e) => error!("Broadcast transaction {} failed: {}", txid, e),
            }
        });
    }
}

impl Filter for Platform {
    // Watches for this transaction on-chain
    #[inline]
    fn register_tx(&self, txid: &Txid, _script_pubkey: &Script) { self.add_tx(*txid); }

    // Watches for any transactions that spend this output on-chain
    fn register_output(&self, output: WatchedOutput) -> Option<(usize, Transaction)> {
        self.add_output(output.clone());
        let block_hash = match output.block_hash {
            Some(h) => H256Json::from(h.as_hash().into_inner()),
            None => return None,
        };

        // Although this works for both native and electrum clients as the block hash is available,
        // the filter interface which includes register_output and register_tx should be used for electrum clients only,
        // this is the reason for initializing the filter as an option in the start_lightning function as it will be None
        // when implementing lightning for native clients
        let output_spend_fut = self.rpc_client().find_output_spend(
            h256_from_txid(output.outpoint.txid),
            output.script_pubkey.as_ref(),
            output.outpoint.index.into(),
            BlockHashOrHeight::Hash(block_hash),
        );
        let maybe_output_spend_res =
            tokio::task::block_in_place(move || output_spend_fut.wait()).error_log_passthrough();

        if let Ok(Some(spent_output_info)) = maybe_output_spend_res {
            match Transaction::try_from(spent_output_info.spending_tx) {
                Ok(spending_tx) => return Some((spent_output_info.input_index, spending_tx)),
                Err(e) => error!("Can't convert transaction error: {}", e.to_string()),
            }
        }

        None
    }
}
