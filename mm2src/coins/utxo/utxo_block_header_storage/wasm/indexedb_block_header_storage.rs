use super::{BlockHeaderStorageTable, IDBBlockHeadersInnerLocked};
use async_trait::async_trait;
use chain::BlockHeader;
use mm2_core::mm_ctx::MmArc;
use mm2_db::indexed_db::{ConstructibleDb, DbIdentifier, DbInstance, DbLocked, IndexedDb, IndexedDbBuilder,
                         InitDbResult, SharedDb};
use mm2_err_handle::prelude::*;
use primitives::hash::H256;
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
    pub ticker: String,
    pub db: SharedDb<IDBBlockHeadersInner>,
}

impl IDBBlockHeadersStorage {
    pub fn new(ctx: &MmArc, ticker: String) -> Self {
        Self {
            db: ConstructibleDb::new_shared(ctx),
            ticker,
        }
    }

    async fn lock_db(&self) -> IDBBlockHeadersStorageRes<IDBBlockHeadersInnerLocked<'_>> {
        self.db
            .get_or_initialize()
            .await
            .mm_err(WasmBlockHeadersStorageError::from)
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
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let block_headers_db = db_transaction.table::<BlockHeaderStorageTable>().await?;

        for (height, header) in headers {
            let hash = header.hash().reversed().to_string();
            let raw_header = hex::encode(header.raw());
            let bits: u32 = header.bits.into();
            let headers_to_store = BlockHeaderStorageTable {
                ticker,
                height,
                bits,
                hash,
                raw_header,
            };

            block_headers_db.add_item(&headers_to_store);
        }
        Ok(())
    }

    async fn get_block_header(&self, _height: u64) -> Result<Option<BlockHeader>, BlockHeaderStorageError> { Ok(None) }

    async fn get_block_header_raw(&self, _height: u64) -> Result<Option<String>, BlockHeaderStorageError> { Ok(None) }

    async fn get_last_block_height(&self) -> Result<u64, BlockHeaderStorageError> {
        Err(BlockHeaderStorageError::Internal("Not implemented".into()))
    }

    async fn get_last_block_header_with_non_max_bits(&self) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        Ok(None)
    }

    async fn get_block_height_by_hash(&self, _hash: H256) -> Result<Option<i64>, BlockHeaderStorageError> { Ok(None) }
}

#[test]
fn test_wasm_block_header_storage() { println!("HELLO INDEXEDDB") }
