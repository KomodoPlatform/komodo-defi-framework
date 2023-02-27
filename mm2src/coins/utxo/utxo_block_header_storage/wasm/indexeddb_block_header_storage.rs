use super::BlockHeaderStorageTable;

use async_trait::async_trait;
use chain::BlockHeader;
use mm2_core::mm_ctx::MmArc;
use mm2_db::indexed_db::cursor_prelude::{CollectCursor, WithOnly};
use mm2_db::indexed_db::{ConstructibleDb, DbIdentifier, DbInstance, DbLocked, IndexedDb, IndexedDbBuilder,
                         InitDbResult, MultiIndex, SharedDb};
use mm2_err_handle::prelude::*;
use primitives::hash::H256;
use serialization::Reader;
use spv_validation::storage::{BlockHeaderStorageError, BlockHeaderStorageOps};
use std::collections::HashMap;

const DB_NAME: &str = "block_headers_cache";
const DB_VERSION: u32 = 1;

pub type IDBBlockHeadersStorageRes<T> = MmResult<T, BlockHeaderStorageError>;
pub type IDBBlockHeadersInnerLocked<'a> = DbLocked<'a, IDBBlockHeadersInner>;

pub struct IDBBlockHeadersInner {
    pub inner: IndexedDb,
}

#[async_trait]
impl DbInstance for IDBBlockHeadersInner {
    fn db_name() -> &'static str { DB_NAME }

    async fn init(db_id: DbIdentifier) -> InitDbResult<Self> {
        let inner = IndexedDbBuilder::new(db_id)
            .with_version(DB_VERSION)
            .with_table::<BlockHeaderStorageTable>()
            .build()
            .await?;

        Ok(Self { inner })
    }
}

impl IDBBlockHeadersInner {
    pub fn get_inner(&self) -> &IndexedDb { &self.inner }
}

pub struct IDBBlockHeadersStorage {
    pub db: SharedDb<IDBBlockHeadersInner>,
    pub ticker: String,
}

impl IDBBlockHeadersStorage {
    pub fn new(ctx: &MmArc, ticker: String) -> Self {
        Self {
            db: ConstructibleDb::new(ctx).into_shared(),
            ticker,
        }
    }

    async fn lock_db(&self) -> IDBBlockHeadersStorageRes<IDBBlockHeadersInnerLocked<'_>> {
        self.db
            .get_or_initialize()
            .await
            .mm_err(|err| BlockHeaderStorageError::init_err(&self.ticker, err.to_string()))
    }
}

#[async_trait]
impl BlockHeaderStorageOps for IDBBlockHeadersStorage {
    async fn init(&self) -> Result<(), BlockHeaderStorageError> { Ok(()) }

    async fn is_initialized_for(&self) -> Result<bool, BlockHeaderStorageError> { Ok(true) }

    async fn add_block_headers_to_storage(
        &self,
        headers: HashMap<u64, BlockHeader>,
    ) -> Result<(), BlockHeaderStorageError> {
        let ticker = self.ticker.clone();
        let locked_db = self
            .lock_db()
            .await
            .map_err(|err| BlockHeaderStorageError::add_err(&ticker, err.to_string()))?;
        let db_transaction = locked_db
            .get_inner()
            .transaction()
            .await
            .map_err(|err| BlockHeaderStorageError::add_err(&ticker, err.to_string()))?;
        let block_headers_db = db_transaction
            .table::<BlockHeaderStorageTable>()
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

        for (height, header) in headers {
            let hash = header.hash().reversed().to_string();
            let raw_header = hex::encode(header.raw());
            let bits: u32 = header.bits.into();
            let headers_to_store = BlockHeaderStorageTable {
                ticker: ticker.clone(),
                height,
                bits,
                hash,
                raw_header,
            };
            let index_keys = MultiIndex::new(BlockHeaderStorageTable::HEIGHT_TICKER_INDEX)
                .with_value(&height)
                .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?
                .with_value(&ticker)
                .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

            block_headers_db
                .replace_item_by_unique_multi_index(index_keys, &headers_to_store)
                .await
                .map_err(|err| BlockHeaderStorageError::add_err(&ticker, err.to_string()))?;
        }
        Ok(())
    }

    async fn get_block_header(&self, height: u64) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        let ticker = self.ticker.clone();
        if let Some(raw_header) = self.get_block_header_raw(height).await? {
            let serialized = &hex::decode(raw_header).map_err(|e| BlockHeaderStorageError::DecodeError {
                coin: ticker.clone(),
                reason: e.to_string(),
            })?;
            let mut reader = Reader::new_with_coin_variant(serialized, ticker.as_str().into());
            let header: BlockHeader =
                reader
                    .read()
                    .map_err(|e: serialization::Error| BlockHeaderStorageError::DecodeError {
                        coin: ticker,
                        reason: e.to_string(),
                    })?;

            return Ok(Some(header));
        };

        Ok(None)
    }

    async fn get_block_header_raw(&self, height: u64) -> Result<Option<String>, BlockHeaderStorageError> {
        let ticker = self.ticker.clone();
        let locked_db = self
            .lock_db()
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?;
        let db_transaction = locked_db
            .get_inner()
            .transaction()
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?;
        let block_headers_db = db_transaction
            .table::<BlockHeaderStorageTable>()
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;
        let index_keys = MultiIndex::new(BlockHeaderStorageTable::HEIGHT_TICKER_INDEX)
            .with_value(&height)
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?
            .with_value(&ticker)
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

        Ok(block_headers_db
            .get_item_by_unique_multi_index(index_keys)
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?
            .map(|raw| raw.1.raw_header))
    }

    async fn get_last_block_height(&self) -> Result<Option<u64>, BlockHeaderStorageError> {
        let ticker = self.ticker.clone();
        let locked_db = self
            .lock_db()
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?;
        let db_transaction = locked_db
            .get_inner()
            .transaction()
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?;
        let block_headers_db = db_transaction
            .table::<BlockHeaderStorageTable>()
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

        // Todo: use open_cursor with direction to optimze this process.
        let res = block_headers_db
            .open_cursor("ticker")
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?
            .only("ticker", ticker.clone())
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?
            .collect()
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?
            .into_iter()
            .map(|(_item_id, item)| item.height)
            .collect::<Vec<_>>();

        Ok(res.into_iter().max())
    }

    async fn get_last_block_header_with_non_max_bits(
        &self,
        max_bits: u32,
    ) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        let ticker = self.ticker.clone();
        let locked_db = self
            .lock_db()
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?;
        let db_transaction = locked_db
            .get_inner()
            .transaction()
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?;
        let block_headers_db = db_transaction
            .table::<BlockHeaderStorageTable>()
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

        // Todo: use open_cursor with direction to optimze this process.
        let res = block_headers_db
            .open_cursor("ticker")
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?
            .only("ticker", ticker.clone())
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?
            .collect()
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?
            .into_iter()
            .map(|(_item_id, item)| item)
            .collect::<Vec<_>>();
        let res = res
            .into_iter()
            .filter_map(|e| if e.bits != max_bits { Some(e) } else { None })
            .collect::<Vec<_>>();

        for header in res {
            let serialized = &hex::decode(header.raw_header).map_err(|e| BlockHeaderStorageError::DecodeError {
                coin: ticker.clone(),
                reason: e.to_string(),
            })?;
            let mut reader = Reader::new_with_coin_variant(serialized, ticker.as_str().into());
            let header: BlockHeader =
                reader
                    .read()
                    .map_err(|e: serialization::Error| BlockHeaderStorageError::DecodeError {
                        coin: ticker,
                        reason: e.to_string(),
                    })?;

            return Ok(Some(header));
        }

        Ok(None)
    }

    async fn get_block_height_by_hash(&self, hash: H256) -> Result<Option<i64>, BlockHeaderStorageError> {
        let ticker = self.ticker.clone();
        let locked_db = self
            .lock_db()
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?;
        let db_transaction = locked_db
            .get_inner()
            .transaction()
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?;
        let block_headers_db = db_transaction
            .table::<BlockHeaderStorageTable>()
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;
        let index_keys = MultiIndex::new(BlockHeaderStorageTable::HASH_TICKER_INDEX)
            .with_value(&hash.to_string())
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?
            .with_value(&ticker)
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

        Ok(block_headers_db
            .get_item_by_unique_multi_index(index_keys)
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?
            .map(|raw| raw.1.height as i64))
    }

    async fn remove_headers_up_to_height(&self, to_height: u64) -> Result<(), BlockHeaderStorageError> {
        let ticker = self.ticker.clone();
        let locked_db = self
            .lock_db()
            .await
            .map_err(|err| BlockHeaderStorageError::delete_err(&ticker, err.to_string(), to_height))?;
        let db_transaction = locked_db
            .get_inner()
            .transaction()
            .await
            .map_err(|err| BlockHeaderStorageError::delete_err(&ticker, err.to_string(), to_height))?;
        let block_headers_db = db_transaction
            .table::<BlockHeaderStorageTable>()
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

        for height in 0..=to_height {
            let index_keys = MultiIndex::new(BlockHeaderStorageTable::HEIGHT_TICKER_INDEX)
                .with_value(&height)
                .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?
                .with_value(&ticker)
                .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

            block_headers_db
                .delete_item_by_unique_multi_index(index_keys)
                .await
                .map_err(|err| BlockHeaderStorageError::delete_err(&ticker, err.to_string(), to_height))?;
        }

        Ok(())
    }

    async fn is_table_empty(&self) -> Result<(), BlockHeaderStorageError> {
        let ticker = self.ticker.clone();
        let locked_db = self
            .lock_db()
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;
        let db_transaction = locked_db
            .get_inner()
            .transaction()
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;
        let block_headers_db = db_transaction
            .table::<BlockHeaderStorageTable>()
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

        let items = block_headers_db
            .get_items("ticker", ticker.clone())
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

        if !items.is_empty() {
            return Err(BlockHeaderStorageError::table_err(
                &ticker,
                "Table is not empty".to_string(),
            ));
        };

        Ok(())
    }
}
