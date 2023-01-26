use async_trait::async_trait;
use chain::BlockHeader;
use mm2_core::mm_ctx::MmArc;
use mm2_db::indexed_db::{ConstructibleDb, DbIdentifier, DbInstance, DbLocked, DbUpgrader, IndexedDb, IndexedDbBuilder,
                         InitDbResult, OnUpgradeResult, SharedDb, TableSignature};
use primitives::hash::H256;
use spv_validation::storage::{BlockHeaderStorageError, BlockHeaderStorageOps};
use std::collections::HashMap;

const DB_NAME: &str = "block_headers_cache";
const DB_VERSION: u32 = 1;

pub type IndexedDBBlockHeadersStorageInnerLocked<'a> = DbLocked<'a, IndexedDBBlockHeadersStorageInner>;

#[derive(Clone, Deserialize, Serialize)]
pub struct BlockHeaderStorageTable {
    pub block_height: u64,
    pub block_bits: u64,
    pub block_hash: String,
    pub hex: String,
}

impl TableSignature for BlockHeaderStorageTable {
    fn table_name() -> &'static str { "block_headers_cache" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        match (old_version, new_version) {
            (0, 1) => {
                let table = upgrader.create_table(Self::table_name())?;
                table.create_index("block_height", true)?;
            },
            _ => (),
        }
        Ok(())
    }
}

pub struct IndexedDBBlockHeadersStorageInner {
    pub inner: IndexedDb,
}

#[async_trait]
impl DbInstance for IndexedDBBlockHeadersStorageInner {
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

impl IndexedDBBlockHeadersStorageInner {
    pub fn get_inner(&self) -> &IndexedDb { &self.inner }
}

pub struct IndexedDBBlockHeadersStorage {
    pub ticker: String,
    pub db: SharedDb<IndexedDBBlockHeadersStorageInner>,
}

impl IndexedDBBlockHeadersStorage {
    pub fn new(ctx: &MmArc, ticker: String) -> Self {
        Self {
            db: ConstructibleDb::new_shared(ctx),
            ticker,
        }
    }
}

#[async_trait]
impl BlockHeaderStorageOps for IndexedDBBlockHeadersStorage {
    async fn init(&self) -> Result<(), BlockHeaderStorageError> { Ok(()) }

    async fn is_initialized_for(&self) -> Result<bool, BlockHeaderStorageError> { Ok(true) }

    async fn add_block_headers_to_storage(
        &self,
        _headers: HashMap<u64, BlockHeader>,
    ) -> Result<(), BlockHeaderStorageError> {
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
