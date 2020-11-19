use super::*;
use crate::utxo::{RequestTxHistoryResult, UtxoFeeDetails};
use crate::CoinsContext;
use crate::TxFeeDetails;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use common::jsonrpc_client::JsonRpcErrorType;
use common::lazy::LazyLocal;
use common::mm_metrics::MetricsArc;
use futures01::Future as Future01;
use itertools::Itertools;
use script_pubkey::{extract_contract_call_from_script, extract_gas_from_script, ExtractGasEnum};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::future::Future;
use std::io::Cursor;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration;
use utxo_common::{HISTORY_TOO_LARGE_ERROR, HISTORY_TOO_LARGE_ERR_CODE};

type TxTransferMap = HashMap<TxInternalId, TransactionDetails>;
type HistoryMapByHash = HashMap<H256Json, TxTransferMap>;
type TxIds = Vec<(H256Json, u64)>;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TxInternalId {
    tx_hash: H256Json,
    output_index: u64,
    log_index: u64,
}

impl TxInternalId {
    pub fn new(tx_hash: H256Json, output_index: u64, log_index: u64) -> TxInternalId {
        TxInternalId {
            tx_hash,
            output_index,
            log_index,
        }
    }

    /// TODO use parity_bitcoin::serialization instead of this custom implementation
    pub fn from_bytes(bytes: &BytesJson) -> Result<TxInternalId, String> {
        // H256(32 bytes) + output_index(8 bytes) + log_index(8 bytes)
        const EXPECTED_LEN: usize = 32 + 8 + 8;

        if bytes.len() != EXPECTED_LEN {
            return ERR!("Incorrect bytes len {}, expected {}", bytes.len(), EXPECTED_LEN);
        }

        let tx_hash: H256Json = bytes[0..32].into();

        let buf = bytes[32..].to_vec();
        let mut cursor = Cursor::new(buf);
        let output_index = cursor.read_u64::<BigEndian>().unwrap();
        let log_index = cursor.read_u64::<BigEndian>().unwrap();

        Ok(TxInternalId {
            tx_hash,
            output_index,
            log_index,
        })
    }
}

impl From<TxInternalId> for BytesJson {
    fn from(id: TxInternalId) -> Self {
        let mut bytes = id.tx_hash.0.to_vec();
        bytes
            .write_u64::<BigEndian>(id.output_index)
            .expect("Error on write_u64");
        bytes.write_u64::<BigEndian>(id.log_index).expect("Error on write_u64");
        bytes.into()
    }
}

#[derive(Debug, PartialEq)]
enum ProcessCachedTransferMapResult {
    Updated,
    UpdateIsNotNeeded,
    ReloadIsRequired,
}

impl Qrc20Coin {
    pub fn history_loop(&self, ctx: MmArc) {
        let mut my_balance: Option<BigDecimal> = None;
        let mut history_map = self.try_load_history_from_file(&ctx);

        let mut success_iteration = 0i32;
        loop {
            if ctx.is_stopping() {
                break;
            };
            {
                let coins_ctx = unwrap!(CoinsContext::from_ctx(&ctx));
                let coins = block_on(coins_ctx.coins.lock());
                if !coins.contains_key(&self.utxo.ticker) {
                    ctx.log.log("", &[&"tx_history", &self.utxo.ticker], "Loop stopped");
                    break;
                };
            }

            let actual_balance = match self.my_balance().wait() {
                Ok(b) => b,
                Err(err) => {
                    ctx.log.log(
                        "😟",
                        &[&"tx_history", &self.utxo.ticker],
                        &ERRL!("Error {:?} on getting balance", err),
                    );
                    thread::sleep(Duration::from_secs(10));
                    continue;
                },
            };

            let need_update = self.check_if_history_update_is_needed(&history_map, &my_balance, &actual_balance);
            if !need_update {
                thread::sleep(Duration::from_secs(30));
                continue;
            }

            let tx_ids = match self.request_tx_history(ctx.metrics.clone()) {
                RequestTxHistoryResult::Ok(tx_ids) => tx_ids,
                RequestTxHistoryResult::Retry { error } => {
                    ctx.log
                        .log("", &[&"tx_history", &self.utxo.ticker], &ERRL!("{}, retrying", error));
                    thread::sleep(Duration::from_secs(10));
                    continue;
                },
                RequestTxHistoryResult::HistoryTooLarge => {
                    ctx.log.log(
                        "😟",
                        &[&"tx_history", &self.utxo.ticker],
                        &ERRL!("Got `history too large`, stopping further attempts to retrieve it"),
                    );
                    *unwrap!(self.utxo.history_sync_state.lock()) = HistorySyncState::Error(json!({
                        "code": HISTORY_TOO_LARGE_ERR_CODE,
                        "message": "Got `history too large` error from Electrum server. History is not available",
                    }));
                    break;
                },
                RequestTxHistoryResult::UnknownError(e) => {
                    ctx.log.log(
                        "😟",
                        &[&"tx_history", &self.utxo.ticker],
                        &ERRL!("{}, stopping futher attempts to retreive it", e),
                    );
                    break;
                },
            };

            let updated = self.process_tx_ids(&ctx, &mut history_map, tx_ids);
            if success_iteration == 0 {
                ctx.log.log(
                    "😅",
                    &[&"tx_history", &("coin", self.utxo.ticker.clone().as_str())],
                    "history has been loaded successfully",
                );
            }

            my_balance = Some(actual_balance);
            success_iteration += 1;

            if !updated {
                continue;
            }

            // `history_map` has been updated.
            let mut to_write: Vec<&TransactionDetails> = history_map
                .iter()
                .map(|(_, value)| value)
                .flatten()
                .map(|(_tx_id, tx)| tx)
                .collect();
            to_write.sort_unstable_by(|a, b| {
                match sort_newest_to_oldest(a.block_height, b.block_height) {
                    // do not reverse `transfer` events in one transaction
                    Ordering::Equal => a.internal_id.cmp(&b.internal_id),
                    ord => ord,
                }
            });
            self.save_history_to_file(&unwrap!(json::to_vec(&to_write)), &ctx);
        }
    }

    pub async fn transfer_details_by_hash(&self, tx_hash: H256Json) -> Result<TxTransferMap, String> {
        let electrum = match self.utxo.rpc_client {
            UtxoRpcClientEnum::Electrum(ref rpc) => rpc,
            UtxoRpcClientEnum::Native(_) => return ERR!("Electrum client expected"),
        };
        let receipts = try_s!(electrum.blochchain_transaction_get_receipt(&tx_hash).compat().await);
        // request Qtum transaction details to get a tx_hex, timestamp, block_height and calculate a miner_fee
        let qtum_details = try_s!(utxo_common::tx_details_by_hash(self, &tx_hash.0).await);
        // Deserialize the UtxoTx to get a script pubkey
        let qtum_tx: UtxoTx = try_s!(deserialize(qtum_details.tx_hex.as_slice()).map_err(|e| ERRL!("{:?}", e)));

        let miner_fee = {
            let total_qtum_fee = match qtum_details.fee_details {
                Some(TxFeeDetails::Utxo(UtxoFeeDetails { ref amount })) => amount.clone(),
                Some(ref fee) => return ERR!("Unexpected fee details {:?}", fee),
                None => return ERR!("No Qtum fee details"),
            };
            let total_gas_used = receipts.iter().fold(0, |gas, receipt| gas + receipt.gas_used);
            let total_gas_used = big_decimal_from_sat(total_gas_used as i64, self.utxo.decimals);
            total_qtum_fee - total_gas_used
        };

        let mut details = TxTransferMap::new();
        for receipt in receipts {
            let log_details =
                try_s!(self.transfer_details_from_receipt(&qtum_tx, &qtum_details, receipt, miner_fee.clone()));
            details.extend(log_details.into_iter())
        }

        Ok(details)
    }

    fn transfer_details_from_receipt(
        &self,
        qtum_tx: &UtxoTx,
        qtum_details: &TransactionDetails,
        receipt: TxReceipt,
        miner_fee: BigDecimal,
    ) -> Result<TxTransferMap, String> {
        let tx_hash: H256Json = qtum_details.tx_hash.as_slice().into();
        if qtum_tx.outputs.len() <= (receipt.output_index as usize) {
            return ERR!(
                "Length of the transaction {:?} outputs less than output_index {}",
                tx_hash,
                receipt.output_index
            );
        }
        let script_pubkey: Script = qtum_tx.outputs[receipt.output_index as usize]
            .script_pubkey
            .clone()
            .into();
        let fee_details = {
            let gas_limit = try_s!(extract_gas_from_script(&script_pubkey, ExtractGasEnum::GasLimit));
            let gas_price = try_s!(extract_gas_from_script(&script_pubkey, ExtractGasEnum::GasPrice));

            let total_gas_fee = utxo_common::big_decimal_from_sat(receipt.gas_used as i64, self.utxo.decimals);
            Qrc20FeeDetails {
                // QRC20 fees are paid in base platform currency (particular in Qtum)
                coin: self.platform.clone(),
                miner_fee,
                gas_limit,
                gas_price,
                total_gas_fee,
            }
        };

        let mut details = TxTransferMap::new();
        for (log_index, log_entry) in receipt.log.into_iter().enumerate() {
            if log_entry.topics.len() != 3 {
                continue;
            }
            // the first topic should be ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef
            // https://github.com/qtumproject/qtum-electrum/blob/v4.0.2/electrum/wallet.py#L2101
            if log_entry.topics[0] != QRC20_TRANSFER_TOPIC {
                continue;
            }
            if try_s!(log_entry.parse_address()) != self.contract_address {
                continue;
            }

            let (total_amount, from, to) = {
                let event = try_s!(transfer_event_from_log(&log_entry));
                // https://github.com/qtumproject/qtum-electrum/blob/v4.0.2/electrum/wallet.py#L2093
                if event.contract_address != self.contract_address {
                    // contract address mismatch
                    continue;
                }
                let amount = try_s!(u256_to_big_decimal(event.amount, self.decimals()));
                let from = self.utxo_address_from_qrc20(event.sender);
                let to = self.utxo_address_from_qrc20(event.receiver);
                (amount, from, to)
            };

            // https://github.com/qtumproject/qtum-electrum/blob/v4.0.2/electrum/wallet.py#L2102
            if from != self.utxo.my_address && to != self.utxo.my_address {
                // address mismatch
                continue;
            }

            let spent_by_me = if from == self.utxo.my_address {
                total_amount.clone()
            } else {
                0.into()
            };
            let received_by_me = if to == self.utxo.my_address {
                total_amount.clone()
            } else {
                0.into()
            };

            // do not inherit the block_height from qtum_tx (usually it is None)
            let block_height = receipt.block_number;
            let my_balance_change = &received_by_me - &spent_by_me;
            let internal_id = TxInternalId::new(tx_hash.clone(), receipt.output_index, log_index as u64);

            let from = if is_sender_contract(&script_pubkey) {
                display_contract_address(from)
            } else {
                try_s!(self.display_address(&from))
            };

            let to = if is_receiver_contract(&script_pubkey) {
                display_contract_address(to)
            } else {
                try_s!(self.display_address(&to))
            };

            let tx_details = TransactionDetails {
                from: vec![from],
                to: vec![to],
                total_amount,
                spent_by_me,
                received_by_me,
                my_balance_change,
                block_height,
                fee_details: Some(fee_details.clone().into()),
                internal_id: internal_id.clone().into(),
                ..qtum_details.clone()
            };

            details.insert(internal_id, tx_details);
        }

        Ok(details)
    }

    fn request_tx_history(&self, metrics: MetricsArc) -> RequestTxHistoryResult {
        mm_counter!(metrics, "tx.history.request.count", 1,
                    "coin" => self.utxo.ticker.clone(), "client" => "electrum", "method" => "blockchain.contract.event.get_history");
        let history_res = block_on(
            HistoryBuilder::new(self.clone())
                .order(HistoryOrder::NewestToOldest)
                .build_with_rpc_error(),
        );
        let history_cont = match history_res {
            Ok(h) => h,
            Err(e) => match &e.error {
                JsonRpcErrorType::Transport(e) | JsonRpcErrorType::Parse(_, e) => {
                    return RequestTxHistoryResult::Retry {
                        error: ERRL!("Error {} on blockchain_contract_event_get_history", e),
                    };
                },
                JsonRpcErrorType::Response(_addr, err) => {
                    return if HISTORY_TOO_LARGE_ERROR.eq(err) {
                        RequestTxHistoryResult::HistoryTooLarge
                    } else {
                        RequestTxHistoryResult::Retry {
                            error: ERRL!("Error {:?} on blockchain_contract_event_get_history", e),
                        }
                    }
                },
            },
        };
        mm_counter!(metrics, "tx.history.response.count", 1,
                    "coin" => self.utxo.ticker.clone(), "client" => "electrum", "method" => "blockchain.contract.event.get_history");

        mm_counter!(metrics, "tx.history.response.total_length", history_cont.len() as u64,
                    "coin" => self.utxo.ticker.clone(), "client" => "electrum", "method" => "blockchain.contract.event.get_history");

        let tx_ids = history_cont
            .into_iter()
            // electrum can returns multiple `TxHistoryItem` with the same `TxHistoryItem::tx_hash`
            // but with the different `TxHistoryItem::log_index`
            .unique_by(|item| item.tx_hash.clone())
            .map(|item| (item.tx_hash, item.height as u64))
            .collect();
        RequestTxHistoryResult::Ok(tx_ids)
    }

    fn check_if_history_update_is_needed(
        &self,
        history: &HistoryMapByHash,
        last_balance: &Option<BigDecimal>,
        actual_balance: &BigDecimal,
    ) -> bool {
        let need_update = history
            .iter()
            .map(|(_, txs)| txs)
            .flatten()
            .any(|(_, tx)| tx.should_update_timestamp() || tx.should_update_block_height());
        match last_balance {
            Some(last_balance) if last_balance == actual_balance && !need_update => {
                // my balance hasn't been changed, there is no need to reload tx_history
                false
            },
            _ => true,
        }
    }

    /// Returns true if the `history_map` has been updated.
    fn process_cached_tx_transfer_map(
        &self,
        ctx: &MmArc,
        tx_hash: &H256Json,
        tx_height: u64,
        transfer_map: &mut TxTransferMap,
    ) -> ProcessCachedTransferMapResult {
        // `qtum_details` will be initialized once on first LazyLocal::[get, get_mut] call.
        // Note if `utxo_common::tx_details_by_hash` failed for some reason then LazyLocal::get will return None.
        let mut qtum_details = LazyLocal::with_constructor(move || {
            mm_counter!(ctx.metrics, "tx.history.request.count", 1, "coin" => self.utxo.ticker.clone(), "method" => "tx_detail_by_hash");
            match block_on(utxo_common::tx_details_by_hash(self, &tx_hash.0)) {
                Ok(d) => {
                    mm_counter!(ctx.metrics, "tx.history.response.count", 1, "coin" => self.utxo.ticker.clone(), "method" => "tx_detail_by_hash");
                    Some(d)
                },
                Err(e) => {
                    ctx.log.log(
                        "😟",
                        &[&"tx_history", &self.utxo.ticker],
                        &ERRL!("Error {:?} on tx_details_by_hash for {:?} tx", e, tx_hash),
                    );
                    None
                },
            }
        });

        let mut updated = false;
        for (id, tx) in transfer_map {
            if id.tx_hash != *tx_hash {
                ctx.log.log(
                    "😟",
                    &[&"tx_history", &self.utxo.ticker],
                    &ERRL!(
                        "Warning: TxTransferMap contains entries with the different tx_hash {:?}, expected {:?}",
                        id.tx_hash,
                        tx_hash
                    ),
                );
                return ProcessCachedTransferMapResult::ReloadIsRequired;
            }

            // update block height for previously unconfirmed transaction
            if tx.should_update_block_height() && tx_height > 0 {
                tx.block_height = tx_height;
                updated = true;
            }
            if tx.should_update_timestamp() {
                if let Some(qtum_details) = qtum_details.get() {
                    tx.timestamp = qtum_details.timestamp;
                    updated = true;
                } // else `utxo_common::tx_details_by_hash` failed for some reason
            }
        }

        if updated {
            ProcessCachedTransferMapResult::Updated
        } else {
            ProcessCachedTransferMapResult::UpdateIsNotNeeded
        }
    }

    /// Returns true if the `history_map` has been updated.
    fn process_tx_ids(&self, ctx: &MmArc, history_map: &mut HistoryMapByHash, tx_ids: TxIds) -> bool {
        let mut transactions_left = if history_map.len() < tx_ids.len() {
            tx_ids.len() - history_map.len()
        } else {
            0
        };
        *unwrap!(self.utxo.history_sync_state.lock()) =
            HistorySyncState::InProgress(json!({ "transactions_left": transactions_left }));

        let mut updated = false;
        for (tx_hash, height) in tx_ids {
            // first check if the `transfer` details are initialized for the `tx_hash`
            if let Some(tx_hash_history) = history_map.get_mut(&tx_hash) {
                // we should check if the cached `transfer` details are up-to-date (timestamp and blockheight are not zeros)
                match self.process_cached_tx_transfer_map(&ctx, &tx_hash, height, tx_hash_history) {
                    ProcessCachedTransferMapResult::Updated => {
                        updated = true;
                        continue;
                    },
                    ProcessCachedTransferMapResult::UpdateIsNotNeeded => continue,
                    ProcessCachedTransferMapResult::ReloadIsRequired => (),
                }
            }

            // `transfer` details are not initialized for the `tx_hash`
            // or there is an error in cached `tx_hash_history`
            mm_counter!(ctx.metrics, "tx.history.request.count", 1, "coin" => self.utxo.ticker.clone(), "method" => "transfer_details_by_hash");
            let tx_hash_history = match block_on(self.transfer_details_by_hash(tx_hash.clone())) {
                Ok(d) => d,
                Err(e) => {
                    ctx.log.log(
                        "😟",
                        &[&"tx_history", &self.utxo.ticker],
                        &ERRL!("Error {:?} on getting the details of {:?}, skipping the tx", e, tx_hash),
                    );
                    continue;
                },
            };

            if history_map.insert(tx_hash.clone(), tx_hash_history).is_some() {
                ctx.log.log(
                    "😟",
                    &[&"tx_history", &self.utxo.ticker],
                    &format!("'transfer' details of {:?} were reloaded", tx_hash),
                );
            }

            mm_counter!(ctx.metrics, "tx.history.response.count", 1, "coin" => self.utxo.ticker.clone(), "method" => "transfer_details_by_hash");
            if transactions_left > 0 {
                transactions_left -= 1;
                *unwrap!(self.utxo.history_sync_state.lock()) =
                    HistorySyncState::InProgress(json!({ "transactions_left": transactions_left }));
            }

            updated = true;
        }

        *unwrap!(self.utxo.history_sync_state.lock()) = HistorySyncState::Finished;
        updated
    }

    fn try_load_history_from_file(&self, ctx: &MmArc) -> HistoryMapByHash {
        let history = self.load_history_from_file(&ctx);
        let mut history_map: HistoryMapByHash = HashMap::default();

        for tx in history {
            let id = match TxInternalId::from_bytes(&tx.internal_id) {
                Ok(i) => i,
                Err(e) => {
                    ctx.log.log(
                        "😟",
                        &[&"tx_history", &self.utxo.ticker],
                        &ERRL!("Error {:?} on load history from file", e),
                    );
                    return HistoryMapByHash::default();
                },
            };
            let tx_hash_history = history_map.entry(id.tx_hash.clone()).or_insert_with(HashMap::default);
            if tx_hash_history.insert(id, tx).is_some() {
                ctx.log.log(
                    "😟",
                    &[&"tx_history", &self.utxo.ticker],
                    &ERRL!("History file contains entries with the same 'internal_id'"),
                );
                return HistoryMapByHash::default();
            }
        }

        history_map
    }
}

pub struct HistoryBuilder {
    coin: Qrc20Coin,
    from_block: u64,
    topic: String,
    address: H160,
    token_address: H160,
    order: Option<HistoryOrder>,
}

pub struct HistoryCont<T> {
    history: Vec<T>,
}

/// Future for the [`HistoryBuilder::build_utxo_lazy()`].
/// Loads `UtxoTx` from `tx_hash`.
pub struct UtxoFromHashFuture {
    future: Option<Box<dyn Future<Output = Result<UtxoTx, String>> + Unpin + 'static>>,
}

pub enum HistoryOrder {
    NewestToOldest,
    OldestToNewest,
}

impl HistoryBuilder {
    pub fn new(coin: Qrc20Coin) -> HistoryBuilder {
        let address = qrc20_addr_from_utxo_addr(coin.utxo.my_address.clone());
        let token_address = coin.contract_address;
        HistoryBuilder {
            coin,
            from_block: 0,
            topic: QRC20_TRANSFER_TOPIC.to_string(),
            address,
            token_address,
            order: None,
        }
    }

    #[allow(clippy::wrong_self_convention)]
    pub fn from_block(mut self, from_block: u64) -> HistoryBuilder {
        self.from_block = from_block;
        self
    }

    #[allow(dead_code)]
    pub fn topic(mut self, topic: &str) -> HistoryBuilder {
        self.topic = topic.to_string();
        self
    }

    pub fn address(mut self, address: H160) -> HistoryBuilder {
        self.address = address;
        self
    }

    #[allow(dead_code)]
    pub fn token_address(mut self, token_address: H160) -> HistoryBuilder {
        self.token_address = token_address;
        self
    }

    pub fn order(mut self, order: HistoryOrder) -> HistoryBuilder {
        self.order = Some(order);
        self
    }

    pub async fn build(self) -> Result<HistoryCont<TxHistoryItem>, String> {
        self.build_with_rpc_error().await.map_err(|e| ERRL!("{}", e))
    }

    pub async fn build_with_rpc_error(self) -> Result<HistoryCont<TxHistoryItem>, JsonRpcError> {
        let electrum = match self.coin.utxo.rpc_client {
            UtxoRpcClientEnum::Electrum(ref rpc_cln) => rpc_cln,
            UtxoRpcClientEnum::Native(_) => panic!("Native mode doesn't support"),
        };

        let address = qrc20_addr_into_rpc_format(&self.address);
        let token_address = qrc20_addr_into_rpc_format(&self.token_address);
        let mut history = electrum
            .blockchain_contract_event_get_history(&address, &token_address, &self.topic)
            .compat()
            .await?;

        if self.from_block != 0 {
            history = history
                .into_iter()
                .filter(|item| self.from_block <= item.height)
                .collect();
        }

        match self.order {
            Some(HistoryOrder::NewestToOldest) => {
                history.sort_unstable_by(|a, b| sort_newest_to_oldest(a.height, b.height))
            },
            Some(HistoryOrder::OldestToNewest) => {
                history.sort_unstable_by(|a, b| sort_oldest_to_newest(a.height, b.height))
            },
            None => (),
        }

        Ok(HistoryCont { history })
    }

    /// Request history by `tx_hash` and wrap it into list of UtxoLazyFuture.
    /// This method is used when there is no reason to load all `UtxoTx`,
    /// only the necessary ones.
    ///
    /// In particular this is used to load `UtxoTx` until the wanted tx is found.
    /// See [`FindLazy::find_lazy()`] and [`FindMapLazy::find_map_lazy()`].
    pub async fn build_utxo_lazy(self) -> Result<HistoryCont<UtxoFromHashFuture>, String> {
        let coin = self.coin.clone();
        let cont = try_s!(self.build().await);

        let history = cont
            .into_iter()
            .unique_by(|tx| tx.tx_hash.clone())
            .map(|item| UtxoFromHashFuture::new(coin.clone(), item.tx_hash))
            .collect();

        Ok(HistoryCont { history })
    }
}

impl<T> HistoryCont<T> {
    #[allow(dead_code)]
    pub fn into_vec(self) -> Vec<T> { self.history }

    pub fn into_iter(self) -> impl Iterator<Item = T> { self.history.into_iter() }

    pub fn len(&self) -> usize { self.history.len() }
}

impl UtxoFromHashFuture {
    /// Create the future.
    fn new(coin: Qrc20Coin, tx_hash: H256Json) -> UtxoFromHashFuture {
        let fut = async move {
            let electrum = match coin.utxo.rpc_client {
                UtxoRpcClientEnum::Electrum(ref rpc_cln) => rpc_cln,
                UtxoRpcClientEnum::Native(_) => panic!("Native mode doesn't support"),
            };

            let verbose_tx = try_s!(electrum.get_verbose_transaction(tx_hash).compat().await);
            let utxo_tx: UtxoTx = deserialize(verbose_tx.hex.as_slice()).map_err(|e| ERRL!("{:?}", e))?;
            Ok(utxo_tx)
        };
        UtxoFromHashFuture {
            future: Some(Box::new(fut.boxed())),
        }
    }
}

unsafe impl Send for UtxoFromHashFuture {}

impl Future for UtxoFromHashFuture {
    type Output = Result<UtxoTx, String>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut fut = self.future.take().expect("cannot poll UtxoLazyFuture twice");
        if let Poll::Ready(result) = fut.poll_unpin(cx) {
            return Poll::Ready(result);
        }
        self.future = Some(fut);
        Poll::Pending
    }
}

fn is_sender_contract(script_pubkey: &Script) -> bool {
    let contract_call_bytes = match extract_contract_call_from_script(&script_pubkey) {
        Ok(bytes) => bytes,
        Err(e) => {
            log!((e));
            return false;
        },
    };
    let call_type = match ContractCallType::from_script_pubkey(&contract_call_bytes) {
        Ok(Some(t)) => t,
        Ok(None) => return false,
        Err(e) => {
            log!((e));
            return false;
        },
    };
    match call_type {
        ContractCallType::Transfer => false,
        ContractCallType::Erc20Payment => false,
        ContractCallType::ReceiverSpend => true,
        ContractCallType::SenderRefund => true,
    }
}

fn is_receiver_contract(script_pubkey: &Script) -> bool {
    let contract_call_bytes = match extract_contract_call_from_script(&script_pubkey) {
        Ok(bytes) => bytes,
        Err(e) => {
            log!((e));
            return false;
        },
    };
    let call_type = match ContractCallType::from_script_pubkey(&contract_call_bytes) {
        Ok(Some(t)) => t,
        Ok(None) => return false,
        Err(e) => {
            log!((e));
            return false;
        },
    };
    match call_type {
        ContractCallType::Transfer => false,
        ContractCallType::Erc20Payment => true,
        ContractCallType::ReceiverSpend => false,
        ContractCallType::SenderRefund => false,
    }
}

fn sort_newest_to_oldest(x_height: u64, y_height: u64) -> Ordering {
    // the transactions with block_height == 0 are the most recent
    if x_height == 0 {
        Ordering::Less
    } else if y_height == 0 {
        Ordering::Greater
    } else {
        y_height.cmp(&x_height)
    }
}

fn sort_oldest_to_newest(x_height: u64, y_height: u64) -> Ordering {
    sort_newest_to_oldest(x_height, y_height).reverse()
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::for_tests::find_metrics_in_json;
    use common::mm_metrics::{MetricType, MetricsJson};
    use qrc20_tests::qrc20_coin_for_test;

    #[test]
    fn test_tx_internal_id() {
        let tx_hash = hex::decode("39104d29d77ba83c5c6c63ab7a0f096301c443b4538dc6b30140453a40caa80a").unwrap();
        let expected_id = TxInternalId::new(tx_hash.as_slice().into(), 13, 257);
        let actual_bytes: BytesJson = expected_id.clone().into();

        let mut expected_bytes = tx_hash.clone();
        expected_bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 13]);
        expected_bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 1]);
        assert_eq!(actual_bytes, expected_bytes.into());

        let actual_id = TxInternalId::from_bytes(&actual_bytes).unwrap();
        assert_eq!(actual_id, expected_id);
    }

    #[test]
    fn test_process_cached_tx_transfer_map_update_is_not_needed() {
        // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
        let priv_key = [
            3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144,
            72, 172, 110, 180, 13, 123, 179, 10, 49,
        ];
        let (ctx, coin) = qrc20_coin_for_test(&priv_key);
        ctx.metrics.init().unwrap();

        let tx_hash: H256Json = hex::decode("85ede12ccc12fb1709c4d9e403e96c0c394b0916f2f6098d41d8dfa00013fcdb")
            .unwrap()
            .as_slice()
            .into();
        let tx_height = 699545;
        let transfer_map_expected = block_on(coin.transfer_details_by_hash(tx_hash.clone())).unwrap();

        let mut transfer_map = transfer_map_expected.clone();
        assert_eq!(
            ProcessCachedTransferMapResult::UpdateIsNotNeeded,
            coin.process_cached_tx_transfer_map(&ctx, &tx_hash, tx_height, &mut transfer_map)
        );
        assert_eq!(transfer_map, transfer_map_expected);

        let value: MetricsJson = json::from_value(ctx.metrics.collect_json().unwrap()).unwrap();
        let found = find_metrics_in_json(value, "tx.history.request.count", &[(
            "method",
            "transfer_details_by_hash",
        )]);
        assert_eq!(found, None);
    }

    #[test]
    fn test_process_cached_tx_transfer_map_updated() {
        // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
        let priv_key = [
            3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144,
            72, 172, 110, 180, 13, 123, 179, 10, 49,
        ];
        let (ctx, coin) = qrc20_coin_for_test(&priv_key);
        ctx.metrics.init().unwrap();

        let tx_hash: H256Json = hex::decode("85ede12ccc12fb1709c4d9e403e96c0c394b0916f2f6098d41d8dfa00013fcdb")
            .unwrap()
            .as_slice()
            .into();
        let tx_height = 699545;
        let transfer_map_expected = block_on(coin.transfer_details_by_hash(tx_hash.clone())).unwrap();

        let mut transfer_map_zero_timestamp = transfer_map_expected
            .clone()
            .into_iter()
            .map(|(id, mut tx)| {
                tx.timestamp = 0;
                (id, tx)
            })
            .collect();
        assert_eq!(
            ProcessCachedTransferMapResult::Updated,
            coin.process_cached_tx_transfer_map(&ctx, &tx_hash, tx_height, &mut transfer_map_zero_timestamp)
        );
        assert_eq!(transfer_map_zero_timestamp, transfer_map_expected);

        let value: MetricsJson = json::from_value(ctx.metrics.collect_json().unwrap()).unwrap();
        let found = find_metrics_in_json(value, "tx.history.request.count", &[("method", "tx_detail_by_hash")]);
        match found {
            Some(MetricType::Counter { key, value, .. }) if key == "tx.history.request.count" && value == 1 => (),
            found => panic!("Found metric type: {:?}", found),
        }
    }

    #[test]
    fn test_process_cached_tx_transfer_map_reload_is_required() {
        // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
        let priv_key = [
            3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144,
            72, 172, 110, 180, 13, 123, 179, 10, 49,
        ];
        let (ctx, coin) = qrc20_coin_for_test(&priv_key);
        ctx.metrics.init().unwrap();

        let tx_hash: H256Json = hex::decode("85ede12ccc12fb1709c4d9e403e96c0c394b0916f2f6098d41d8dfa00013fcdb")
            .unwrap()
            .as_slice()
            .into();
        let tx_height = 699545;
        let transfer_map_expected = block_on(coin.transfer_details_by_hash(tx_hash.clone())).unwrap();

        let mut transfer_map_unexpected_tx_id = transfer_map_expected
            .clone()
            .into_iter()
            .map(|(mut id, tx)| {
                // just another tx_hash
                id.tx_hash = hex::decode("8a7270110ab7b56142b3bac89999276beb70320a7fe7666f460a05aa615eb0a0")
                    .unwrap()
                    .as_slice()
                    .into();
                (id, tx)
            })
            .collect();
        assert_eq!(
            ProcessCachedTransferMapResult::ReloadIsRequired,
            coin.process_cached_tx_transfer_map(&ctx, &tx_hash, tx_height, &mut transfer_map_unexpected_tx_id)
        );

        let value: MetricsJson = json::from_value(ctx.metrics.collect_json().unwrap()).unwrap();
        let found = find_metrics_in_json(value, "tx.history.request.count", &[("method", "tx_detail_by_hash")]);
        assert_eq!(found, None);
    }

    #[test]
    fn test_process_tx_ids_updated() {
        // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
        let priv_key = [
            3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144,
            72, 172, 110, 180, 13, 123, 179, 10, 49,
        ];
        let (ctx, coin) = qrc20_coin_for_test(&priv_key);

        let tx_hash: H256Json = hex::decode("35e03bc529528a853ee75dde28f27eec8ed7b152b6af7ab6dfa5d55ea46f25ac")
            .unwrap()
            .as_slice()
            .into();
        let tx_height = 681443;
        let transfer_map_expected = block_on(coin.transfer_details_by_hash(tx_hash.clone())).unwrap();
        let mut history_map_expected = HistoryMapByHash::new();
        history_map_expected.insert(tx_hash.clone(), transfer_map_expected);

        let tx_ids = vec![(tx_hash, tx_height)];
        let mut history_map = HistoryMapByHash::new();
        assert!(coin.process_tx_ids(&ctx, &mut history_map, tx_ids));
        assert_eq!(history_map, history_map_expected);
    }

    #[test]
    fn test_process_tx_ids_not_updated() {
        // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
        let priv_key = [
            3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144,
            72, 172, 110, 180, 13, 123, 179, 10, 49,
        ];
        let (ctx, coin) = qrc20_coin_for_test(&priv_key);

        let tx_hash: H256Json = hex::decode("85ede12ccc12fb1709c4d9e403e96c0c394b0916f2f6098d41d8dfa00013fcdb")
            .unwrap()
            .as_slice()
            .into();
        let tx_height = 699545;
        let transfer_map_expected = block_on(coin.transfer_details_by_hash(tx_hash.clone())).unwrap();
        let mut history_map_expected = HistoryMapByHash::new();
        history_map_expected.insert(tx_hash.clone(), transfer_map_expected);

        let tx_ids = vec![(tx_hash, tx_height)];
        let mut history_map = history_map_expected.clone();
        assert!(!coin.process_tx_ids(&ctx, &mut history_map, tx_ids));
        assert_eq!(history_map, history_map_expected);
    }

    #[test]
    fn test_process_tx_ids_error_on_details() {
        // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
        let priv_key = [
            3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144,
            72, 172, 110, 180, 13, 123, 179, 10, 49,
        ];
        let (ctx, coin) = qrc20_coin_for_test(&priv_key);

        let metrics = MetricsArc::new();
        metrics.init().unwrap();

        let tx_hash_invalid: H256Json = hex::decode("0000000000000000000000000000000000000000000000000000000000000000")
            .unwrap()
            .as_slice()
            .into();
        let tx_hash: H256Json = hex::decode("85ede12ccc12fb1709c4d9e403e96c0c394b0916f2f6098d41d8dfa00013fcdb")
            .unwrap()
            .as_slice()
            .into();
        let tx_height = 699545;
        let transfer_map_expected = block_on(coin.transfer_details_by_hash(tx_hash.clone())).unwrap();
        let mut history_map_expected = HistoryMapByHash::new();
        // should contain only valid tx
        history_map_expected.insert(tx_hash.clone(), transfer_map_expected);

        let tx_ids = vec![(tx_hash, tx_height), (tx_hash_invalid, tx_height)];
        let mut history_map = HistoryMapByHash::default();
        assert!(coin.process_tx_ids(&ctx, &mut history_map, tx_ids));
        assert_eq!(history_map, history_map_expected);
    }
}
