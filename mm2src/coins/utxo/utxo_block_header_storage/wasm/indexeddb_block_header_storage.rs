use super::BlockHeaderStorageTable;
use async_trait::async_trait;
use chain::BlockHeader;
use mm2_core::mm_ctx::MmArc;
use mm2_db::indexed_db::cursor_prelude::CollectCursor;
use mm2_db::indexed_db::cursor_prelude::WithOnly;
use mm2_db::indexed_db::{ConstructibleDb, DbIdentifier, DbInstance, DbLocked, IndexedDb, IndexedDbBuilder,
                         InitDbResult, SharedDb};
use mm2_err_handle::prelude::*;
use primitives::hash::H256;
use serialization::Reader;
use spv_validation::storage::{BlockHeaderStorageError, BlockHeaderStorageOps};
use spv_validation::work::MAX_BITS_BTC;
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
            db: ConstructibleDb::new_shared(ctx),
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
            .map_err(|err| BlockHeaderStorageError::DBLockError(err.to_string()))?;
        let db_transaction = locked_db
            .get_inner()
            .transaction()
            .await
            .map_err(|err| BlockHeaderStorageError::TransactionError(err.to_string()))?;
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

            block_headers_db.add_item(&headers_to_store).await.map_err(|err| {
                BlockHeaderStorageError::AddToStorageError {
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
            .map_err(|err| BlockHeaderStorageError::DBLockError(err.to_string()))?;
        let db_transaction = locked_db
            .get_inner()
            .transaction()
            .await
            .map_err(|err| BlockHeaderStorageError::TransactionError(err.to_string()))?;
        let block_headers_db = db_transaction
            .table::<BlockHeaderStorageTable>()
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

        Ok(block_headers_db
            .get_item_by_unique_index("height", height)
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?
            .map(|raw| raw.1.raw_header))
    }

    async fn get_last_block_height(&self) -> Result<u64, BlockHeaderStorageError> {
        let ticker = self.ticker.clone();
        let locked_db = self
            .lock_db()
            .await
            .map_err(|err| BlockHeaderStorageError::DBLockError(err.to_string()))?;
        let db_transaction = locked_db
            .get_inner()
            .transaction()
            .await
            .map_err(|err| BlockHeaderStorageError::TransactionError(err.to_string()))?;
        let block_headers_db = db_transaction
            .table::<BlockHeaderStorageTable>()
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

        // Todo use open_cursor with direction to optimze this process.
        let mut res = block_headers_db
            .open_cursor("ticker")
            .await
            .map_err(|err| BlockHeaderStorageError::cursor_err(&ticker, err.to_string()))?
            .only("ticker", ticker.clone())
            .map_err(|err| BlockHeaderStorageError::cursor_err(&ticker, err.to_string()))?
            .collect()
            .await
            .map_err(|err| BlockHeaderStorageError::cursor_err(&ticker, err.to_string()))?
            .into_iter()
            .map(|(_item_id, item)| item.height)
            .collect::<Vec<_>>();
        res.sort_by_key(|&b| std::cmp::Reverse(b));

        if res.len() >= 1 {
            return Ok(res[0]);
        }

        Ok(0)
    }

    async fn get_last_block_header_with_non_max_bits(&self) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        let ticker = self.ticker.clone();
        let locked_db = self
            .lock_db()
            .await
            .map_err(|err| BlockHeaderStorageError::DBLockError(err.to_string()))?;
        let db_transaction = locked_db
            .get_inner()
            .transaction()
            .await
            .map_err(|err| BlockHeaderStorageError::TransactionError(err.to_string()))?;
        let block_headers_db = db_transaction
            .table::<BlockHeaderStorageTable>()
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

        // Todo use open_cursor with direction to optimze this process.
        let mut res = block_headers_db
            .open_cursor("ticker")
            .await
            .map_err(|err| BlockHeaderStorageError::cursor_err(&ticker, err.to_string()))?
            .only("ticker", ticker.clone())
            .map_err(|err| BlockHeaderStorageError::cursor_err(&ticker, err.to_string()))?
            .collect()
            .await
            .map_err(|err| BlockHeaderStorageError::cursor_err(&ticker, err.to_string()))?
            .into_iter()
            .map(|(_item_id, item)| item)
            .collect::<Vec<_>>();
        res.sort_by(|a, b| b.height.cmp(&a.height));

        for header in res {
            if header.bits != MAX_BITS_BTC {
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
        }

        Ok(None)
    }

    async fn get_block_height_by_hash(&self, hash: H256) -> Result<Option<i64>, BlockHeaderStorageError> {
        let ticker = self.ticker.clone();
        let locked_db = self
            .lock_db()
            .await
            .map_err(|err| BlockHeaderStorageError::DBLockError(err.to_string()))?;
        let db_transaction = locked_db
            .get_inner()
            .transaction()
            .await
            .map_err(|err| BlockHeaderStorageError::TransactionError(err.to_string()))?;
        let block_headers_db = db_transaction
            .table::<BlockHeaderStorageTable>()
            .await
            .map_err(|err| BlockHeaderStorageError::table_err(&ticker, err.to_string()))?;

        Ok(block_headers_db
            .get_item_by_unique_index("hash", hash.to_string())
            .await
            .map_err(|err| BlockHeaderStorageError::get_err(&ticker, err.to_string()))?
            .map(|raw| raw.1.height as i64))
    }
}

#[cfg(target_arch = "wasm32")]
mod tests {
    use super::IDBBlockHeadersStorage;
    use super::*;
    use chain::BlockHeaderBits;
    use common::log::wasm_log::register_wasm_log;
    use mm2_test_helpers::for_tests::mm_ctx_with_custom_db;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

    wasm_bindgen_test_configure!(run_in_browser);

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
        let block_header: BlockHeader = "0000002076d41d3e4b0bfd4c0d3b30aa69fdff3ed35d85829efd04000000000000000000b386498b583390959d9bac72346986e3015e83ac0b54bc7747a11a494ac35c94bb3ce65a53fb45177f7e311c".into();
        headers.insert(520481, block_header);
        let add_headers = storage.add_block_headers_to_storage(headers).await;

        assert!(add_headers.is_ok());
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
        let block_header: BlockHeader = "0000002076d41d3e4b0bfd4c0d3b30aa69fdff3ed35d85829efd04000000000000000000b386498b583390959d9bac72346986e3015e83ac0b54bc7747a11a494ac35c94bb3ce65a53fb45177f7e311c".into();
        headers.insert(520481, block_header);
        let add_headers = storage.add_block_headers_to_storage(headers).await;
        assert!(add_headers.is_ok());

        let block_header_hex = storage.get_block_header_raw(520481).await.unwrap();
        assert_eq!
        ("0000002076d41d3e4b0bfd4c0d3b30aa69fdff3ed35d85829efd04000000000000000000b386498b583390959d9bac72346986e3015e83ac0b54bc7747a11a494ac35c94bb3ce65a53fb45177f7e311c".to_string(), block_header_hex.unwrap());

        let expected_header_hash: H256 = "0000000000000000002e31d0714a5ab23100945ff87ba2d856cd566a3c9344ec".into();
        let block_header = storage.get_block_header(520481).await.unwrap().unwrap();
        assert_eq!(block_header.hash(), expected_header_hash.reversed());
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
        let add_headers = storage.add_block_headers_to_storage(headers).await;
        assert!(add_headers.is_ok());

        let raw_header = storage.get_block_header_raw(201595).await.unwrap();
        assert_eq!("02000000ea01a61a2d7420a1b23875e40eb5eb4ca18b378902c8e6384514ad0000000000c0c5a1ae80582b3fe319d8543307fa67befc2a734b8eddb84b1780dfdf11fa2b20e71353ffff001d00805fe0", raw_header.unwrap())
    }

    #[wasm_bindgen_test]
    async fn test_get_last_block_height() {
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
        // This block has max difficulty
        // https://live.blockcypher.com/btc-testnet/block/00000000961a9d117feb57e516e17217207a849bf6cdfce529f31d9a96053530/
        let block_header: BlockHeader = "02000000ea01a61a2d7420a1b23875e40eb5eb4ca18b378902c8e6384514ad0000000000c0c5a1ae80582b3fe319d8543307fa67befc2a734b8eddb84b1780dfdf11fa2b20e71353ffff001d00805fe0".into();
        headers.insert(201595, block_header);

        // https://live.blockcypher.com/btc-testnet/block/0000000000ad144538e6c80289378ba14cebb50ee47538b2a120742d1aa601ea/
        let expected_block_header: BlockHeader = "02000000cbed7fd98f1f06e85c47e13ff956533642056be45e7e6b532d4d768f00000000f2680982f333fcc9afa7f9a5e2a84dc54b7fe10605cd187362980b3aa882e9683be21353ab80011c813e1fc0".into();
        headers.insert(201594, expected_block_header.clone());

        // This block has max difficulty
        // https://live.blockcypher.com/btc-testnet/block/0000000000ad144538e6c80289378ba14cebb50ee47538b2a120742d1aa601ea/
        let block_header: BlockHeader = "020000001f38c8e30b30af912fbd4c3e781506713cfb43e73dff6250348e060000000000afa8f3eede276ccb4c4ee649ad9823fc181632f262848ca330733e7e7e541beb9be51353ffff001d00a63037".into();

        headers.insert(201593, block_header);
        let add_headers = storage.add_block_headers_to_storage(headers.clone()).await;
        assert!(add_headers.is_ok());

        let last_height = storage.get_last_block_height().await.unwrap();
        assert_eq!(201595, last_height);
    }

    #[wasm_bindgen_test]
    async fn test_get_last_block_header_with_non_max_bits() {
        let ctx = mm_ctx_with_custom_db();
        let storage = IDBBlockHeadersStorage::new(&ctx, "RICK".to_string());
        register_wasm_log();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);

        // repetitive init must not fail
        storage.init().await.unwrap();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);
        let mut headers = HashMap::with_capacity(2);

        // This block has max difficulty
        // https://live.blockcypher.com/btc-testnet/block/00000000961a9d117feb57e516e17217207a849bf6cdfce529f31d9a96053530/
        let block_header: BlockHeader = "02000000ea01a61a2d7420a1b23875e40eb5eb4ca18b378902c8e6384514ad0000000000c0c5a1ae80582b3fe319d8543307fa67befc2a734b8eddb84b1780dfdf11fa2b20e71353ffff001d00805fe0".into();
        headers.insert(201595, block_header);

        // https://live.blockcypher.com/btc-testnet/block/0000000000ad144538e6c80289378ba14cebb50ee47538b2a120742d1aa601ea/
        let expected_block_header: BlockHeader = "02000000cbed7fd98f1f06e85c47e13ff956533642056be45e7e6b532d4d768f00000000f2680982f333fcc9afa7f9a5e2a84dc54b7fe10605cd187362980b3aa882e9683be21353ab80011c813e1fc0".into();
        headers.insert(201594, expected_block_header.clone());

        // This block has max difficulty
        // https://live.blockcypher.com/btc-testnet/block/0000000000ad144538e6c80289378ba14cebb50ee47538b2a120742d1aa601ea/
        let block_header: BlockHeader = "020000001f38c8e30b30af912fbd4c3e781506713cfb43e73dff6250348e060000000000afa8f3eede276ccb4c4ee649ad9823fc181632f262848ca330733e7e7e541beb9be51353ffff001d00a63037".into();
        headers.insert(201593, block_header);

        let add_headers = storage.add_block_headers_to_storage(headers).await;
        assert!(add_headers.is_ok());

        let actual_block_header = storage
            .get_last_block_header_with_non_max_bits()
            .await
            .unwrap()
            .unwrap();
        assert_ne!(actual_block_header.bits, BlockHeaderBits::Compact(MAX_BITS_BTC.into()));
        assert_eq!(actual_block_header, expected_block_header);
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
        let add_headers = storage.add_block_headers_to_storage(headers).await;
        assert!(add_headers.is_ok());

        let raw_header = storage
            .get_block_height_by_hash("00000000961a9d117feb57e516e17217207a849bf6cdfce529f31d9a96053530".into())
            .await
            .unwrap();
        assert_eq!(201595, raw_header.unwrap())
    }
}
