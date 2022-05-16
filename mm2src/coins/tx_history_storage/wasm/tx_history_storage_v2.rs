use crate::my_tx_history_v2::{GetHistoryResult, HistoryCoinType, RemoveTxResult, TxHistoryStorage};
use crate::tx_history_storage::wasm::tx_history_db::{TxHistoryDb, TxHistoryDbLocked};
use crate::tx_history_storage::wasm::{WasmTxHistoryError, WasmTxHistoryResult};
use crate::tx_history_storage::{token_id_from_tx_type, CoinTokenId, ConfirmationStatus, CreateTxHistoryStorageError};
use crate::{CoinsContext, TransactionDetails};
use async_trait::async_trait;
use common::indexed_db::{BeBigUint, DbUpgrader, MultiIndex, OnUpgradeResult, SharedDb, TableSignature};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::PagingOptionsEnum;
use itertools::Itertools;
use rpc::v1::types::Bytes as BytesJson;
use serde_json::{self as json, Value as Json};

#[derive(Clone)]
pub struct IndexedDbTxHistoryStorage {
    db: SharedDb<TxHistoryDb>,
}

impl IndexedDbTxHistoryStorage {
    pub fn new(ctx: &MmArc) -> MmResult<Self, CreateTxHistoryStorageError>
    where
        Self: Sized,
    {
        let coins_ctx = CoinsContext::from_ctx(ctx).map_to_mm(CreateTxHistoryStorageError::Internal)?;
        Ok(IndexedDbTxHistoryStorage {
            db: coins_ctx.tx_history_db.clone(),
        })
    }
}

#[async_trait]
impl TxHistoryStorage for IndexedDbTxHistoryStorage {
    type Error = WasmTxHistoryError;

    async fn init(&self, _for_coin: &str) -> MmResult<(), Self::Error> { Ok(()) }

    async fn is_initialized_for(&self, _for_coin: &str) -> MmResult<bool, Self::Error> { Ok(true) }

    /// Adds multiple transactions to the selected coin's history
    /// Also consider adding tx_hex to the cache during this operation
    async fn add_transactions_to_history<I>(&self, for_coin: &str, transactions: I) -> MmResult<(), Self::Error>
    where
        I: IntoIterator<Item = TransactionDetails> + Send + 'static,
        I::IntoIter: Send,
    {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let history_table = db_transaction.table::<TxHistoryTableV2>().await?;
        let cache_table = db_transaction.table::<TxCacheTableV2>().await?;

        for tx in transactions {
            let history_item = TxHistoryTableV2::from_tx_details(for_coin.to_owned(), &tx)?;
            history_table.add_item(&history_item).await?;

            let cache_item = TxCacheTableV2::from_tx_details(for_coin.to_owned(), &tx);
            let index_keys = MultiIndex::new(TxCacheTableV2::COIN_TX_HASH_INDEX)
                .with_value(for_coin)?
                .with_value(&tx.tx_hash)?;
            // `TxHistoryTableV2::tx_hash` is not a unique field, but `TxCacheTableV2::tx_hash` is unique.
            // So we use `DbTable::add_item_or_ignore_by_unique_multi_index` instead of `DbTable::add_item`
            // since `transactions` may contain txs with same `tx_hash` but different `internal_id`.
            cache_table
                .add_item_or_ignore_by_unique_multi_index(index_keys, &cache_item)
                .await?;
        }
        Ok(())
    }

    async fn remove_tx_from_history(
        &self,
        for_coin: &str,
        internal_id: &BytesJson,
    ) -> MmResult<RemoveTxResult, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<TxHistoryTableV2>().await?;

        let index_keys = MultiIndex::new(TxHistoryTableV2::COIN_INTERNAL_ID_INDEX)
            .with_value(for_coin)?
            .with_value(internal_id)?;

        if table.delete_item_by_unique_multi_index(index_keys).await?.is_some() {
            Ok(RemoveTxResult::TxRemoved)
        } else {
            Ok(RemoveTxResult::TxDidNotExist)
        }
    }

    async fn get_tx_from_history(
        &self,
        for_coin: &str,
        internal_id: &BytesJson,
    ) -> MmResult<Option<TransactionDetails>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<TxHistoryTableV2>().await?;

        let index_keys = MultiIndex::new(TxHistoryTableV2::COIN_INTERNAL_ID_INDEX)
            .with_value(for_coin)?
            .with_value(internal_id)?;

        let details_json = match table.get_item_by_unique_multi_index(index_keys).await? {
            Some((_item_id, item)) => item.details_json,
            None => return Ok(None),
        };
        json::from_value(details_json).map_to_mm(|e| WasmTxHistoryError::ErrorDeserializing(e.to_string()))
    }

    async fn history_contains_unconfirmed_txes(&self, for_coin: &str) -> Result<bool, MmError<Self::Error>> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<TxHistoryTableV2>().await?;

        let index_keys = MultiIndex::new(TxHistoryTableV2::COIN_CONFIRMATION_STATUS_INDEX)
            .with_value(for_coin)?
            .with_value(ConfirmationStatus::Unconfirmed)?;

        let count_unconfirmed = table.count_by_multi_index(index_keys).await?;
        Ok(count_unconfirmed > 0)
    }

    /// Gets the unconfirmed transactions from the history
    async fn get_unconfirmed_txes_from_history(
        &self,
        for_coin: &str,
    ) -> MmResult<Vec<TransactionDetails>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<TxHistoryTableV2>().await?;

        let index_keys = MultiIndex::new(TxHistoryTableV2::COIN_CONFIRMATION_STATUS_INDEX)
            .with_value(for_coin)?
            .with_value(ConfirmationStatus::Unconfirmed)?;

        table
            .get_items_by_multi_index(index_keys)
            .await?
            .into_iter()
            .map(|(_item_id, item)| tx_details_from_item(item))
            // Collect `WasmTxHistoryResult<Vec<TransactionDetails>>`.
            .collect()
    }

    /// Updates transaction in the selected coin's history
    async fn update_tx_in_history(&self, for_coin: &str, tx: &TransactionDetails) -> MmResult<(), Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<TxHistoryTableV2>().await?;

        let item = TxHistoryTableV2::from_tx_details(for_coin.to_owned(), tx)?;
        let index_keys = MultiIndex::new(TxHistoryTableV2::COIN_INTERNAL_ID_INDEX)
            .with_value(for_coin)?
            .with_value(&tx.internal_id)?;
        table.replace_item_by_unique_multi_index(index_keys, &item).await?;
        Ok(())
    }

    async fn history_has_tx_hash(&self, for_coin: &str, tx_hash: &str) -> Result<bool, MmError<Self::Error>> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<TxHistoryTableV2>().await?;

        let index_keys = MultiIndex::new(TxHistoryTableV2::COIN_TX_HASH_INDEX)
            .with_value(for_coin)?
            .with_value(tx_hash)?;
        let count_txs = table.count_by_multi_index(index_keys).await?;
        Ok(count_txs > 0)
    }

    async fn unique_tx_hashes_num_in_history(&self, for_coin: &str) -> Result<usize, MmError<Self::Error>> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<TxHistoryTableV2>().await?;

        // `IndexedDb` doesn't provide an elegant way to count records applying custom filters to index properties like `tx_hash`,
        // so currently fetch all records with `coin=for_coin` and apply the `unique_by(|tx| tx.tx_hash)` to them.
        Ok(table
            .get_items(TxHistoryTableV2::COIN_INDEX, for_coin)
            .await?
            .into_iter()
            .unique_by(|(_item_id, tx)| tx.tx_hash.clone())
            .count())
    }

    async fn add_tx_to_cache(
        &self,
        for_coin: &str,
        tx_hash: &str,
        tx_hex: &BytesJson,
    ) -> Result<(), MmError<Self::Error>> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<TxCacheTableV2>().await?;

        table
            .add_item(&TxCacheTableV2 {
                coin: for_coin.to_owned(),
                tx_hash: tx_hash.to_owned(),
                tx_hex: tx_hex.clone(),
            })
            .await?;
        Ok(())
    }

    async fn tx_bytes_from_cache(&self, for_coin: &str, tx_hash: &str) -> MmResult<Option<BytesJson>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<TxCacheTableV2>().await?;

        let index_keys = MultiIndex::new(TxCacheTableV2::COIN_TX_HASH_INDEX)
            .with_value(for_coin)?
            .with_value(tx_hash)?;
        match table.get_item_by_unique_multi_index(index_keys).await? {
            Some((_item_id, item)) => Ok(Some(item.tx_hex)),
            None => Ok(None),
        }
    }

    async fn get_history(
        &self,
        coin_type: HistoryCoinType,
        paging: PagingOptionsEnum<BytesJson>,
        limit: usize,
    ) -> MmResult<GetHistoryResult, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<TxHistoryTableV2>().await?;

        let CoinTokenId { coin, token_id } = CoinTokenId::from_history_coin_type(coin_type);
        let index_keys = MultiIndex::new(TxHistoryTableV2::COIN_TOKEN_ID_INDEX)
            .with_value(coin)?
            .with_value(token_id)?;

        let transactions = table
            .get_items_by_multi_index(index_keys)
            .await?
            .into_iter()
            .map(|(_item_id, tx)| tx)
            .collect();
        Self::take_according_to_paging_opts(transactions, paging, limit)
    }
}

impl IndexedDbTxHistoryStorage {
    pub(super) fn take_according_to_paging_opts(
        txs: Vec<TxHistoryTableV2>,
        paging: PagingOptionsEnum<BytesJson>,
        limit: usize,
    ) -> WasmTxHistoryResult<GetHistoryResult> {
        let total_count = txs.len();

        let skip = match paging {
            // `page_number` is ignored if from_uuid is set
            PagingOptionsEnum::FromId(from_internal_id) => {
                txs.iter()
                    .position(|tx| tx.internal_id == from_internal_id)
                    .or_mm_err(|| WasmTxHistoryError::FromIdNotFound {
                        internal_id: from_internal_id,
                    })?
                    + 1
            },
            PagingOptionsEnum::PageNumber(page_number) => (page_number.get() - 1) * limit,
        };

        let transactions = txs
            .into_iter()
            .skip(skip)
            .take(limit)
            .map(tx_details_from_item)
            // Collect `WasmTxHistoryResult<TransactionDetails>` items into `WasmTxHistoryResult<Vec<TransactionDetails>>`
            .collect::<WasmTxHistoryResult<Vec<_>>>()?;
        Ok(GetHistoryResult {
            transactions,
            skipped: skip,
            total: total_count,
        })
    }

    async fn lock_db(&self) -> WasmTxHistoryResult<TxHistoryDbLocked<'_>> {
        self.db.get_or_initialize().await.mm_err(WasmTxHistoryError::from)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct TxHistoryTableV2 {
    coin: String,
    tx_hash: String,
    internal_id: BytesJson,
    block_height: BeBigUint,
    confirmation_status: ConfirmationStatus,
    token_id: String,
    details_json: Json,
}

impl TxHistoryTableV2 {
    /// An index that consists of the only one `coin` property.
    const COIN_INDEX: &'static str = "coin";
    /// A **unique** index that consists of the following properties:
    /// * coin - coin ticker
    /// * internal_id - transaction internal ID
    const COIN_INTERNAL_ID_INDEX: &'static str = "coin_internal_id";
    /// An index that consists of the following properties:
    /// * coin - coin ticker
    /// * tx_hash - transaction hash
    const COIN_TX_HASH_INDEX: &'static str = "coin_tx_hash";
    /// An index that consists of the following properties:
    /// * coin - coin ticker
    /// * confirmation_status - whether transaction is confirmed or unconfirmed
    const COIN_CONFIRMATION_STATUS_INDEX: &'static str = "coin_confirmation_status";
    /// An index that consists of the following properties:
    /// * coin - coin ticker
    /// * token_id - token ID (can be an empty string)
    const COIN_TOKEN_ID_INDEX: &'static str = "coin_token_id";

    fn from_tx_details(coin: String, tx: &TransactionDetails) -> WasmTxHistoryResult<TxHistoryTableV2> {
        let details_json = json::to_value(tx).map_to_mm(|e| WasmTxHistoryError::ErrorSerializing(e.to_string()))?;
        Ok(TxHistoryTableV2 {
            coin,
            tx_hash: tx.tx_hash.clone(),
            internal_id: tx.internal_id.clone(),
            block_height: BeBigUint::from(tx.block_height),
            confirmation_status: ConfirmationStatus::from_block_height(tx.block_height),
            token_id: token_id_from_tx_type(&tx.transaction_type),
            details_json,
        })
    }
}

impl TableSignature for TxHistoryTableV2 {
    fn table_name() -> &'static str { "tx_history_v2" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        match (old_version, new_version) {
            (0, 1) => {
                let table = upgrader.create_table(Self::table_name())?;
                table.create_index(TxHistoryTableV2::COIN_INDEX, false)?;
                table.create_multi_index(TxHistoryTableV2::COIN_INTERNAL_ID_INDEX, &["coin", "internal_id"], true)?;
                table.create_multi_index(TxHistoryTableV2::COIN_TX_HASH_INDEX, &["coin", "tx_hash"], false)?;
                table.create_multi_index(
                    TxHistoryTableV2::COIN_CONFIRMATION_STATUS_INDEX,
                    &["coin", "confirmation_status"],
                    false,
                )?;
                table.create_multi_index(TxHistoryTableV2::COIN_TOKEN_ID_INDEX, &["coin", "token_id"], false)?;
            },
            _ => (),
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct TxCacheTableV2 {
    coin: String,
    tx_hash: String,
    tx_hex: BytesJson,
}

impl TxCacheTableV2 {
    /// A **unique** index that consists of the following properties:
    /// * coin - coin ticker
    /// * tx_hash - transaction hash
    const COIN_TX_HASH_INDEX: &'static str = "coin_tx_hash";

    fn from_tx_details(coin: String, tx: &TransactionDetails) -> TxCacheTableV2 {
        TxCacheTableV2 {
            coin,
            tx_hash: tx.tx_hash.clone(),
            tx_hex: tx.tx_hex.clone(),
        }
    }
}

impl TableSignature for TxCacheTableV2 {
    fn table_name() -> &'static str { "tx_cache_v2" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        match (old_version, new_version) {
            (0, 1) => {
                let table = upgrader.create_table(Self::table_name())?;
                table.create_multi_index(TxCacheTableV2::COIN_TX_HASH_INDEX, &["coin", "tx_hash"], true)?;
            },
            _ => (),
        }
        Ok(())
    }
}

fn tx_details_from_item(item: TxHistoryTableV2) -> WasmTxHistoryResult<TransactionDetails> {
    json::from_value(item.details_json).map_to_mm(|e| WasmTxHistoryError::ErrorDeserializing(e.to_string()))
}
