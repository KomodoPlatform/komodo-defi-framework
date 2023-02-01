use super::BlockHeaderStorageTable;
use async_trait::async_trait;
use chain::BlockHeader;
use mm2_core::mm_ctx::MmArc;
use mm2_db::indexed_db::{ConstructibleDb, DbIdentifier, DbInstance, DbLocked, IndexedDb, IndexedDbBuilder,
                         InitDbResult, SharedDb};
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

        Ok(block_headers_db
            .get_item_by_unique_index("height", height)
            .await
            .map_err(|err| BlockHeaderStorageError::InitializationError {
                coin: ticker.to_string(),
                reason: err.to_string(),
            })?
            .map(|raw| raw.1.raw_header))
    }

    async fn get_last_block_height(&self) -> Result<u64, BlockHeaderStorageError> {
        Err(BlockHeaderStorageError::Internal("Not implemented".into()))
    }

    async fn get_last_block_header_with_non_max_bits(&self) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        Ok(None)
    }

    async fn get_block_height_by_hash(&self, hash: H256) -> Result<Option<i64>, BlockHeaderStorageError> {
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

        Ok(block_headers_db
            .get_item_by_unique_index("hash", hash.to_string())
            .await
            .map_err(|err| BlockHeaderStorageError::InitializationError {
                coin: ticker.to_string(),
                reason: err.to_string(),
            })?
            .map(|raw| raw.1.height as i64))
    }
}
// cargo test --package coins --lib utxo::utxo_wasm_tests::test_electrum_rpc_client

#[cfg(target_arch = "wasm32")]
mod tests {
    use super::IDBBlockHeadersStorage;
    use super::*;
    use chain::BlockHeaderBits::Compact;
    use chain::BlockHeaderNonce::U32;
    // use common::log::info;
    use common::log::wasm_log::register_wasm_log;
    use mm2_test_helpers::for_tests::mm_ctx_with_custom_db;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn hello_world() { register_wasm_log(); }

    #[wasm_bindgen_test]
    async fn test_add_block_headers() {
        let ctx = mm_ctx_with_custom_db();
        let storage = IDBBlockHeadersStorage::new(&ctx, "RICK".to_string());
        register_wasm_log();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);

        // repetitive init must not fail
        storage.init().await.unwrap();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);

        let mut headers = HashMap::with_capacity(1);
        let block_header: BlockHeader = "02000000ea01a61a2d7420a1b23875e40eb5eb4ca18b378902c8e6384514ad0000000000c0c5a1ae80582b3fe319d8543307fa67befc2a734b8eddb84b1780dfdf11fa2b20e71353ffff001d00805fe0".into();
        headers.insert(201595, block_header);
        let add_header = storage.add_block_headers_to_storage(headers).await;

        assert!(add_header.is_ok());
    }

    #[wasm_bindgen_test]
    async fn test_get_block_header() {
        let ctx = mm_ctx_with_custom_db();
        let storage = IDBBlockHeadersStorage::new(&ctx, "RICK".to_string());
        register_wasm_log();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);

        // repetitive init must not fail
        storage.init().await.unwrap();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);

        let mut headers = HashMap::with_capacity(1);
        let block_header: BlockHeader = "02000000ea01a61a2d7420a1b23875e40eb5eb4ca18b378902c8e6384514ad0000000000c0c5a1ae80582b3fe319d8543307fa67befc2a734b8eddb84b1780dfdf11fa2b20e71353ffff001d00805fe0".into();
        headers.insert(201595, block_header);
        let add_header = storage.add_block_headers_to_storage(headers).await;

        assert!(add_header.is_ok());

        let expected_header = BlockHeader {
            version: 2,
            previous_header_hash: "ea01a61a2d7420a1b23875e40eb5eb4ca18b378902c8e6384514ad0000000000".into(),
            merkle_root_hash: "c0c5a1ae80582b3fe319d8543307fa67befc2a734b8eddb84b1780dfdf11fa2b".into(),
            claim_trie_root: None,
            hash_final_sapling_root: None,
            time: 1393813280,
            bits: Compact(486604799.into()),
            nonce: U32(3764355072),
            solution: None,
            aux_pow: None,
            prog_pow: None,
            mtp_pow: None,
            is_verus: false,
            hash_state_root: None,
            hash_utxo_root: None,
            prevout_stake: None,
            vch_block_sig_dlgt: None,
            n_height: None,
            n_nonce_u64: None,
            mix_hash: None,
        };
        let block_header = storage.get_block_header(201595).await.unwrap();

        assert_eq!(expected_header, block_header.unwrap());
    }

    #[wasm_bindgen_test]
    async fn test_get_block_header_raw() {
        let ctx = mm_ctx_with_custom_db();
        let storage = IDBBlockHeadersStorage::new(&ctx, "RICK".to_string());
        register_wasm_log();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);

        // repetitive init must not fail
        storage.init().await.unwrap();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);

        let mut headers = HashMap::with_capacity(1);
        let block_header: BlockHeader = "02000000ea01a61a2d7420a1b23875e40eb5eb4ca18b378902c8e6384514ad0000000000c0c5a1ae80582b3fe319d8543307fa67befc2a734b8eddb84b1780dfdf11fa2b20e71353ffff001d00805fe0".into();
        headers.insert(201595, block_header.clone());
        let add_header = storage.add_block_headers_to_storage(headers).await;
        assert!(add_header.is_ok());

        let raw_header = storage.get_block_header_raw(201595).await.unwrap();
        assert_eq!("02000000ea01a61a2d7420a1b23875e40eb5eb4ca18b378902c8e6384514ad0000000000c0c5a1ae80582b3fe319d8543307fa67befc2a734b8eddb84b1780dfdf11fa2b20e71353ffff001d00805fe0", raw_header.unwrap())
    }

    #[wasm_bindgen_test]
    async fn test_get_block_height_by_hash() {
        let ctx = mm_ctx_with_custom_db();
        let storage = IDBBlockHeadersStorage::new(&ctx, "RICK".to_string());
        register_wasm_log();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);

        // repetitive init must not fail
        storage.init().await.unwrap();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);

        let mut headers = HashMap::with_capacity(1);
        let block_header: BlockHeader = "02000000ea01a61a2d7420a1b23875e40eb5eb4ca18b378902c8e6384514ad0000000000c0c5a1ae80582b3fe319d8543307fa67befc2a734b8eddb84b1780dfdf11fa2b20e71353ffff001d00805fe0".into();
        headers.insert(201595, block_header.clone());
        let add_header = storage.add_block_headers_to_storage(headers).await;
        assert!(add_header.is_ok());

        let raw_header = storage
            .get_block_height_by_hash("00000000961a9d117feb57e516e17217207a849bf6cdfce529f31d9a96053530".into())
            .await
            .unwrap();
        assert_eq!(201595, raw_header.unwrap())
    }
}
