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
use std::io::Cursor;
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
        let receipts = try_s!(self.utxo.rpc_client.get_transaction_receipts(&tx_hash).compat().await);
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
                let from = self.utxo_addr_from_contract_addr(event.sender);
                let to = self.utxo_addr_from_contract_addr(event.receiver);
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

            let from = if is_transferred_from_contract(&script_pubkey) {
                qtum::display_as_contract_address(from)
            } else {
                try_s!(self.display_address(&from))
            };

            let to = if is_transferred_to_contract(&script_pubkey) {
                qtum::display_as_contract_address(to)
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
        let history_res = block_on(TransferHistoryBuilder::new(self.clone()).build_tx_idents());
        let history = match history_res {
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

        mm_counter!(metrics, "tx.history.response.total_length", history.len() as u64,
                    "coin" => self.utxo.ticker.clone(), "client" => "electrum", "method" => "blockchain.contract.event.get_history");

        RequestTxHistoryResult::Ok(history)
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

pub struct TransferHistoryBuilder {
    coin: Qrc20Coin,
    params: TransferHistoryParams,
}

struct TransferHistoryParams {
    from_block: u64,
    address: H160,
    token_address: H160,
}

impl TransferHistoryBuilder {
    pub fn new(coin: Qrc20Coin) -> TransferHistoryBuilder {
        let address = qtum::contract_addr_from_utxo_addr(coin.utxo.my_address.clone());
        let token_address = coin.contract_address;
        let params = TransferHistoryParams {
            from_block: 0,
            address,
            token_address,
        };
        TransferHistoryBuilder { coin, params }
    }

    #[allow(clippy::wrong_self_convention)]
    pub fn from_block(mut self, from_block: u64) -> TransferHistoryBuilder {
        self.params.from_block = from_block;
        self
    }

    pub fn address(mut self, address: H160) -> TransferHistoryBuilder {
        self.params.address = address;
        self
    }

    #[allow(dead_code)]
    pub fn token_address(mut self, token_address: H160) -> TransferHistoryBuilder {
        self.params.token_address = token_address;
        self
    }

    pub async fn build(self) -> Result<Vec<TxReceipt>, JsonRpcError> {
        self.coin.utxo.rpc_client.build(self.params).await
    }

    pub async fn build_tx_idents(self) -> Result<Vec<(H256Json, u64)>, JsonRpcError> {
        self.coin.utxo.rpc_client.build_tx_idents(self.params).await
    }
}

#[async_trait]
trait BuildTransferHistory {
    async fn build(&self, params: TransferHistoryParams) -> Result<Vec<TxReceipt>, JsonRpcError>;

    async fn build_tx_idents(&self, params: TransferHistoryParams) -> Result<Vec<(H256Json, u64)>, JsonRpcError>;
}

#[async_trait]
impl BuildTransferHistory for UtxoRpcClientEnum {
    async fn build(&self, params: TransferHistoryParams) -> Result<Vec<TxReceipt>, JsonRpcError> {
        match self {
            UtxoRpcClientEnum::Native(native) => native.build(params).await,
            UtxoRpcClientEnum::Electrum(electrum) => electrum.build(params).await,
        }
    }

    async fn build_tx_idents(&self, params: TransferHistoryParams) -> Result<Vec<(H256Json, u64)>, JsonRpcError> {
        match self {
            UtxoRpcClientEnum::Native(native) => native.build_tx_idents(params).await,
            UtxoRpcClientEnum::Electrum(electrum) => electrum.build_tx_idents(params).await,
        }
    }
}

#[async_trait]
impl BuildTransferHistory for ElectrumClient {
    async fn build(&self, params: TransferHistoryParams) -> Result<Vec<TxReceipt>, JsonRpcError> {
        let tx_idents = self.build_tx_idents(params).await?;

        let mut receipts = Vec::new();
        for (tx_hash, _height) in tx_idents {
            let mut tx_receipts = self.blochchain_transaction_get_receipt(&tx_hash).compat().await?;
            // remove receipts of contract calls didn't emit at least one `Transfer` event
            tx_receipts.retain(|receipt| receipt.log.iter().any(is_transfer_event_log));
            receipts.extend(tx_receipts.into_iter());
        }

        Ok(receipts)
    }

    async fn build_tx_idents(&self, params: TransferHistoryParams) -> Result<Vec<(H256Json, u64)>, JsonRpcError> {
        let address = contract_addr_into_rpc_format(&params.address);
        let token_address = contract_addr_into_rpc_format(&params.token_address);
        let history = self
            .blockchain_contract_event_get_history(&address, &token_address, QRC20_TRANSFER_TOPIC)
            .compat()
            .await?;

        Ok(history
            .into_iter()
            .filter(|item| params.from_block <= item.height)
            .map(|tx| (tx.tx_hash, tx.height))
            .unique()
            .collect())
    }
}

#[async_trait]
impl BuildTransferHistory for NativeClient {
    async fn build(&self, params: TransferHistoryParams) -> Result<Vec<TxReceipt>, JsonRpcError> {
        const SEARCH_LOGS_STEP: u64 = 100;

        let token_address = contract_addr_into_rpc_format(&params.token_address);
        let address_topic = address_to_log_topic(&params.address);

        // «Skip the log if none of the topics are matched»
        // https://github.com/qtumproject/qtum-enterprise/blob/qtumx_beta_0.16.0/src/rpc/blockchain.cpp#L1590
        //
        // It means disjunction of topics (binary `OR`).
        // So we cannot specify `Transfer` event signature in the first topic,
        // but we can specify either `sender` or `receiver` in `Transfer` event.
        let topics = vec![
            TopicFilter::Skip,                         // event signature
            TopicFilter::Match(address_topic.clone()), // `sender` address in `Transfer` event
            TopicFilter::Match(address_topic.clone()), // `receiver` address in `Transfer` event
        ];

        let block_count = self.get_block_count().compat().await?;

        let mut result = Vec::new();
        let mut from_block = params.from_block;
        while from_block <= block_count {
            let to_block = from_block + SEARCH_LOGS_STEP - 1;
            let mut receipts = self
                .search_logs(from_block, Some(to_block), vec![token_address.clone()], topics.clone())
                .compat()
                .await?;

            // remove receipts of transaction that didn't emit at least one `Transfer` event
            receipts.retain(|receipt| receipt.log.iter().any(is_transfer_event_log));

            result.extend(receipts.into_iter());
            from_block += SEARCH_LOGS_STEP;
        }
        Ok(result)
    }

    async fn build_tx_idents(&self, params: TransferHistoryParams) -> Result<Vec<(H256Json, u64)>, JsonRpcError> {
        let receipts = self.build(params).await?;
        Ok(receipts
            .into_iter()
            .map(|receipt| (receipt.transaction_hash, receipt.block_number))
            .unique()
            .collect())
    }
}

fn is_transferred_from_contract(script_pubkey: &Script) -> bool {
    let contract_call_bytes = match extract_contract_call_from_script(&script_pubkey) {
        Ok(bytes) => bytes,
        Err(e) => {
            error!("{}", e);
            return false;
        },
    };
    let call_type = match MutContractCallType::from_script_pubkey(&contract_call_bytes) {
        Ok(Some(t)) => t,
        Ok(None) => return false,
        Err(e) => {
            error!("{}", e);
            return false;
        },
    };
    match call_type {
        MutContractCallType::Transfer => false,
        MutContractCallType::Erc20Payment => false,
        MutContractCallType::ReceiverSpend => true,
        MutContractCallType::SenderRefund => true,
    }
}

fn is_transferred_to_contract(script_pubkey: &Script) -> bool {
    let contract_call_bytes = match extract_contract_call_from_script(&script_pubkey) {
        Ok(bytes) => bytes,
        Err(e) => {
            error!("{}", e);
            return false;
        },
    };
    let call_type = match MutContractCallType::from_script_pubkey(&contract_call_bytes) {
        Ok(Some(t)) => t,
        Ok(None) => return false,
        Err(e) => {
            error!("{}", e);
            return false;
        },
    };
    match call_type {
        MutContractCallType::Transfer => false,
        MutContractCallType::Erc20Payment => true,
        MutContractCallType::ReceiverSpend => false,
        MutContractCallType::SenderRefund => false,
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

fn is_transfer_event_log(log: &LogEntry) -> bool {
    match log.topics.first() {
        Some(first_topic) => first_topic == QRC20_TRANSFER_TOPIC,
        None => false,
    }
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
