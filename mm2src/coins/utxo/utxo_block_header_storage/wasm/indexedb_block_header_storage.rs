use super::BlockHeaderStorageTable;
use async_trait::async_trait;
use chain::BlockHeader;
use common::log::info;
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
        println!("INTIALIZING IDDBLOCK HEADER STORAGE");
        Self {
            db: ConstructibleDb::new_shared(ctx),
            ticker,
        }
    }

    async fn lock_db(&self) -> IDBBlockHeadersStorageRes<IDBBlockHeadersInnerLocked<'_>> {
        self.db
            .get_or_initialize()
            .await
            .mm_err(|err| BlockHeaderStorageError::InitializationError {
                coin: self.ticker.to_string(),
                reason: err.to_string(),
            })
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
            .map_err(|err| BlockHeaderStorageError::InitializationError {
                coin: ticker.to_string(),
                reason: err.to_string(),
            })?;
        let db_transaction =
            locked_db
                .get_inner()
                .transaction()
                .await
                .map_err(|err| BlockHeaderStorageError::InitializationError {
                    coin: ticker.to_string(),
                    reason: err.to_string(),
                })?;
        let block_headers_db = db_transaction.table::<BlockHeaderStorageTable>().await.map_err(|err| {
            BlockHeaderStorageError::InitializationError {
                coin: ticker.to_string(),
                reason: err.to_string(),
            }
        })?;

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

            block_headers_db.add_item(&headers_to_store).await.map_err(|err| {
                BlockHeaderStorageError::InitializationError {
                    coin: ticker.clone(),
                    reason: err.to_string(),
                }
            })?;
        }
        Ok(())
    }

    async fn get_block_header(&self, _height: u64) -> Result<Option<BlockHeader>, BlockHeaderStorageError> { Ok(None) }

    async fn get_block_header_raw(&self, height: u64) -> Result<Option<String>, BlockHeaderStorageError> {
        let ticker = self.ticker.clone();
        let locked_db = self
            .lock_db()
            .await
            .map_err(|err| BlockHeaderStorageError::InitializationError {
                coin: ticker.to_string(),
                reason: err.to_string(),
            })?;
        let db_transaction =
            locked_db
                .get_inner()
                .transaction()
                .await
                .map_err(|err| BlockHeaderStorageError::InitializationError {
                    coin: ticker.to_string(),
                    reason: err.to_string(),
                })?;
        let block_headers_db = db_transaction.table::<BlockHeaderStorageTable>().await.map_err(|err| {
            BlockHeaderStorageError::InitializationError {
                coin: ticker.to_string(),
                reason: err.to_string(),
            }
        })?;

        let raw = block_headers_db
            .get_item_by_unique_index("height", height)
            .await
            .map_err(|err| BlockHeaderStorageError::InitializationError {
                coin: ticker.to_string(),
                reason: err.to_string(),
            })?;
        info!("BLOCKS {raw:?}");

        Ok(None)
    }

    async fn get_last_block_height(&self) -> Result<u64, BlockHeaderStorageError> {
        Err(BlockHeaderStorageError::Internal("Not implemented".into()))
    }

    async fn get_last_block_header_with_non_max_bits(&self) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        Ok(None)
    }

    async fn get_block_height_by_hash(&self, _hash: H256) -> Result<Option<i64>, BlockHeaderStorageError> { Ok(None) }
}

#[cfg(target_arch = "wasm32")]
mod tests {
    use super::IDBBlockHeadersStorage;
    use super::*;
    use common::log::info;
    use common::log::wasm_log::register_wasm_log;
    use mm2_test_helpers::for_tests::mm_ctx_with_custom_db;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn test_add_block_headers() {
        let ctx = mm_ctx_with_custom_db();
        let storage = IDBBlockHeadersStorage::new(&ctx, "RICK".to_string());
        register_wasm_log();
        // Please note this is the `IndexedDbTxHistoryStorage` specific:
        // [`IndexedDbTxHistoryStorage::is_initialized_for`] always returns `true`.
        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);

        // repetitive init must not fail
        storage.init().await.unwrap();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);

        let mut headers = HashMap::with_capacity(1);
        let block_header: BlockHeader = "0000002076d41d3e4b0bfd4c0d3b30aa69fdff3ed35d85829efd04000000000000000000b386498b583390959d9bac72346986e3015e83ac0b54bc7747a11a494ac35c94bb3ce65a53fb45177f7e311c".into();
        headers.insert(520481, block_header);
        let add_header = storage.add_block_headers_to_storage(headers).await;
        assert!(add_header.is_ok());

        let header = storage.get_block_header_raw(520481).await.unwrap();
        info!("BLOCKS {:?}", header);
        //        assert_eq!(rick_block_headers.len(), 3)
    }
}
