#[cfg(not(target_arch = "wasm32"))] mod sql_block_header_storage;
#[cfg(not(target_arch = "wasm32"))]
pub use sql_block_header_storage::SqliteBlockHeadersStorage;

#[cfg(target_arch = "wasm32")] mod wasm;
#[cfg(target_arch = "wasm32")]
pub use wasm::IDBBlockHeadersStorage;

use async_trait::async_trait;
use chain::BlockHeader;
use mm2_core::mm_ctx::MmArc;
#[cfg(all(test, not(target_arch = "wasm32")))]
use mocktopus::macros::*;
use primitives::hash::H256;
use spv_validation::storage::{BlockHeaderStorageError, BlockHeaderStorageOps};
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};

pub struct BlockHeaderStorage {
    pub inner: Box<dyn BlockHeaderStorageOps>,
}

impl Debug for BlockHeaderStorage {
    fn fmt(&self, _f: &mut Formatter<'_>) -> std::fmt::Result { Ok(()) }
}

impl BlockHeaderStorage {
    #[cfg(all(not(test), not(target_arch = "wasm32")))]
    pub(crate) fn new_from_ctx(ctx: MmArc, ticker: String) -> Result<Self, BlockHeaderStorageError> {
        let sqlite_connection = ctx.sqlite_connection.ok_or(BlockHeaderStorageError::Internal(
            "sqlite_connection is not initialized".to_owned(),
        ))?;
        Ok(BlockHeaderStorage {
            inner: Box::new(SqliteBlockHeadersStorage {
                ticker,
                conn: sqlite_connection.clone(),
            }),
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new_from_ctx(ctx: MmArc, ticker: String) -> Result<Self, BlockHeaderStorageError> {
        Ok(BlockHeaderStorage {
            inner: Box::new(IDBBlockHeadersStorage::new(&ctx, ticker)),
        })
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn new_from_ctx(ctx: MmArc, ticker: String) -> Result<Self, BlockHeaderStorageError> {
        use db_common::sqlite::rusqlite::Connection;
        use std::sync::{Arc, Mutex};

        let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let conn = ctx.sqlite_connection.clone_or(conn);

        Ok(BlockHeaderStorage {
            inner: Box::new(SqliteBlockHeadersStorage { ticker, conn }),
        })
    }
}

#[async_trait]
#[cfg_attr(all(test, not(target_arch = "wasm32")), mockable)]
impl BlockHeaderStorageOps for BlockHeaderStorage {
    async fn init(&self) -> Result<(), BlockHeaderStorageError> { self.inner.init().await }

    async fn is_initialized_for(&self) -> Result<bool, BlockHeaderStorageError> {
        self.inner.is_initialized_for().await
    }

    async fn add_block_headers_to_storage(
        &self,
        headers: HashMap<u64, BlockHeader>,
    ) -> Result<(), BlockHeaderStorageError> {
        self.inner.add_block_headers_to_storage(headers).await
    }

    async fn get_block_header(&self, height: u64) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        self.inner.get_block_header(height).await
    }

    async fn get_block_header_raw(&self, height: u64) -> Result<Option<String>, BlockHeaderStorageError> {
        self.inner.get_block_header_raw(height).await
    }

    async fn get_last_block_height(&self) -> Result<Option<u64>, BlockHeaderStorageError> {
        self.inner.get_last_block_height().await
    }

    async fn get_last_block_header_with_non_max_bits(
        &self,
        max_bits: u32,
    ) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        self.inner.get_last_block_header_with_non_max_bits(max_bits).await
    }

    async fn get_block_height_by_hash(&self, hash: H256) -> Result<Option<i64>, BlockHeaderStorageError> {
        self.inner.get_block_height_by_hash(hash).await
    }

    async fn remove_headers_up_to_height(&self, to_height: u64) -> Result<(), BlockHeaderStorageError> {
        self.inner.remove_headers_up_to_height(to_height).await
    }
}

#[cfg(test)]
mod block_headers_storage_tests {
    cfg_wasm32! {
        use super::*;
        use super::IDBBlockHeadersStorage;
        use chain::BlockHeaderBits;
        use common::log::wasm_log::register_wasm_log;
        use mm2_test_helpers::for_tests::mm_ctx_with_custom_db;
        use wasm_bindgen_test::*;
        use spv_validation::work::MAX_BITS_BTC;

        wasm_bindgen_test_configure!(run_in_browser);
    }

    cfg_native! {
        use super::*;
        use spv_validation::work::MAX_BITS_BTC;
        use chain::BlockHeaderBits;
        use mm2_test_helpers::for_tests::mm_ctx_with_custom_db;

        use crate::utxo::utxo_block_header_storage::sql_block_header_storage::block_headers_cache_table;
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) async fn block_headers_storage_for_test(_for_coin: &str) -> (MmArc, IDBBlockHeadersStorage, String) {
        let ctx = mm_ctx_with_custom_db();
        let storage = IDBBlockHeadersStorage::new(&ctx, "RICK".to_string());
        register_wasm_log();

        (ctx, storage, "".to_string())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn block_headers_storage_for_test(for_coin: &str) -> (MmArc, SqliteBlockHeadersStorage, String) {
        let ctx = mm_ctx_with_custom_db();
        let storage = SqliteBlockHeadersStorage::in_memory(for_coin.into());
        let table = block_headers_cache_table(for_coin);
        storage.init().await.unwrap();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);

        (ctx, storage, table)
    }

    pub(crate) async fn test_add_block_headers_impl() {
        let for_coin = "insert";
        let (_, storage, table) = block_headers_storage_for_test(for_coin).await;

        let mut headers = HashMap::with_capacity(1);
        let block_header: BlockHeader = "0000002076d41d3e4b0bfd4c0d3b30aa69fdff3ed35d85829efd04000000000000000000b386498b583390959d9bac72346986e3015e83ac0b54bc7747a11a494ac35c94bb3ce65a53fb45177f7e311c".into();
        headers.insert(520481, block_header);
        storage.add_block_headers_to_storage(headers).await.unwrap();
        assert!(!storage.is_table_empty(&table).await);
    }

    pub(crate) async fn test_get_block_header_impl() {
        let for_coin = "get";
        let (_, storage, table) = block_headers_storage_for_test(for_coin).await;

        let mut headers = HashMap::with_capacity(1);
        let block_header: BlockHeader = "0000002076d41d3e4b0bfd4c0d3b30aa69fdff3ed35d85829efd04000000000000000000b386498b583390959d9bac72346986e3015e83ac0b54bc7747a11a494ac35c94bb3ce65a53fb45177f7e311c".into();
        headers.insert(520481, block_header);

        storage.add_block_headers_to_storage(headers).await.unwrap();
        assert!(!storage.is_table_empty(&table).await);

        let hex = storage.get_block_header_raw(520481).await.unwrap().unwrap();
        assert_eq!(hex, "0000002076d41d3e4b0bfd4c0d3b30aa69fdff3ed35d85829efd04000000000000000000b386498b583390959d9bac72346986e3015e83ac0b54bc7747a11a494ac35c94bb3ce65a53fb45177f7e311c".to_string());

        let block_header = storage.get_block_header(520481).await.unwrap().unwrap();
        let block_hash: H256 = "0000000000000000002e31d0714a5ab23100945ff87ba2d856cd566a3c9344ec".into();
        assert_eq!(block_header.hash(), block_hash.reversed());

        let height = storage.get_block_height_by_hash(block_hash).await.unwrap().unwrap();
        assert_eq!(height, 520481);
    }

    pub(crate) async fn test_get_last_block_header_with_non_max_bits_impl() {
        let for_coin = "get";
        let (_, storage, table) = block_headers_storage_for_test(for_coin).await;

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

        storage.add_block_headers_to_storage(headers).await.unwrap();
        assert!(!storage.is_table_empty(&table).await);

        let actual_block_header = storage
            .get_last_block_header_with_non_max_bits(MAX_BITS_BTC)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(actual_block_header.bits, BlockHeaderBits::Compact(MAX_BITS_BTC.into()));
        assert_eq!(actual_block_header, expected_block_header);
    }

    pub(crate) async fn test_get_last_block_height_impl() {
        let for_coin = "get";
        let (_, storage, table) = block_headers_storage_for_test(for_coin).await;

        let mut headers = HashMap::with_capacity(2);

        // https://live.blockcypher.com/btc-testnet/block/00000000961a9d117feb57e516e17217207a849bf6cdfce529f31d9a96053530/
        let block_header: BlockHeader = "02000000ea01a61a2d7420a1b23875e40eb5eb4ca18b378902c8e6384514ad0000000000c0c5a1ae80582b3fe319d8543307fa67befc2a734b8eddb84b1780dfdf11fa2b20e71353ffff001d00805fe0".into();
        headers.insert(201595, block_header);

        // https://live.blockcypher.com/btc-testnet/block/0000000000ad144538e6c80289378ba14cebb50ee47538b2a120742d1aa601ea/
        let block_header: BlockHeader = "02000000cbed7fd98f1f06e85c47e13ff956533642056be45e7e6b532d4d768f00000000f2680982f333fcc9afa7f9a5e2a84dc54b7fe10605cd187362980b3aa882e9683be21353ab80011c813e1fc0".into();
        headers.insert(201594, block_header);

        // https://live.blockcypher.com/btc-testnet/block/0000000000ad144538e6c80289378ba14cebb50ee47538b2a120742d1aa601ea/
        let block_header: BlockHeader = "020000001f38c8e30b30af912fbd4c3e781506713cfb43e73dff6250348e060000000000afa8f3eede276ccb4c4ee649ad9823fc181632f262848ca330733e7e7e541beb9be51353ffff001d00a63037".into();
        headers.insert(201593, block_header);

        storage.add_block_headers_to_storage(headers).await.unwrap();
        assert!(!storage.is_table_empty(&table).await);

        let last_block_height = storage.get_last_block_height().await.unwrap();
        assert_eq!(last_block_height.unwrap(), 201595);
    }

    pub(crate) async fn test_remove_headers_up_to_height_impl() {
        let for_coin = "get";
        let (_, storage, table) = block_headers_storage_for_test(for_coin).await;

        let mut headers = HashMap::with_capacity(2);

        // https://live.blockcypher.com/btc-testnet/block/00000000961a9d117feb57e516e17217207a849bf6cdfce529f31d9a96053530/
        let block_header: BlockHeader = "02000000ea01a61a2d7420a1b23875e40eb5eb4ca18b378902c8e6384514ad0000000000c0c5a1ae80582b3fe319d8543307fa67befc2a734b8eddb84b1780dfdf11fa2b20e71353ffff001d00805fe0".into();
        headers.insert(201595, block_header);

        // https://live.blockcypher.com/btc-testnet/block/0000000000ad144538e6c80289378ba14cebb50ee47538b2a120742d1aa601ea/
        let block_header: BlockHeader = "02000000cbed7fd98f1f06e85c47e13ff956533642056be45e7e6b532d4d768f00000000f2680982f333fcc9afa7f9a5e2a84dc54b7fe10605cd187362980b3aa882e9683be21353ab80011c813e1fc0".into();
        headers.insert(201594, block_header);

        // https://live.blockcypher.com/btc-testnet/block/0000000000ad144538e6c80289378ba14cebb50ee47538b2a120742d1aa601ea/
        let block_header: BlockHeader = "020000001f38c8e30b30af912fbd4c3e781506713cfb43e73dff6250348e060000000000afa8f3eede276ccb4c4ee649ad9823fc181632f262848ca330733e7e7e541beb9be51353ffff001d00a63037".into();
        headers.insert(201593, block_header);

        storage.add_block_headers_to_storage(headers).await.unwrap();
        assert!(!storage.is_table_empty(&table).await);

        // Remove 2 headers from storage.
        storage.remove_headers_up_to_height(201594).await.unwrap();

        // Validate that blockers 201593..201594 are removed from storage.
        for h in 201593..201594 {
            let block_header = storage.get_block_header(h).await.unwrap();
            assert!(block_header.is_none());
        }

        // Last height should be 201595
        let last_block_height = storage.get_last_block_height().await.unwrap();
        assert_eq!(last_block_height.unwrap(), 201595);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_tests {
    use super::*;
    use crate::utxo::utxo_block_header_storage::block_headers_storage_tests::*;
    use crate::utxo::utxo_block_header_storage::SqliteBlockHeadersStorage;
    use common::block_on;

    #[test]
    fn test_init_collection() {
        let for_coin = "init_collection";
        let storage = SqliteBlockHeadersStorage::in_memory(for_coin.into());
        let initialized = block_on(storage.is_initialized_for()).unwrap();
        assert!(!initialized);

        block_on(storage.init()).unwrap();
        // repetitive init must not fail
        block_on(storage.init()).unwrap();

        let initialized = block_on(storage.is_initialized_for()).unwrap();
        assert!(initialized);
    }

    #[test]
    fn test_add_block_headers() { block_on(test_add_block_headers_impl()) }

    #[test]
    fn test_test_get_block_header() { block_on(test_get_block_header_impl()) }

    #[test]
    fn test_get_last_block_header_with_non_max_bits() { block_on(test_get_last_block_header_with_non_max_bits_impl()) }

    #[test]
    fn test_get_last_block_height() { block_on(test_get_last_block_height_impl()) }

    #[test]
    fn test_remove_headers_up_to_height() { block_on(test_remove_headers_up_to_height_impl()) }
}

#[cfg(target_arch = "wasm32")]
mod wasm_test {
    use super::*;
    use crate::utxo::utxo_block_header_storage::block_headers_storage_tests::*;
    use common::log::wasm_log::register_wasm_log;
    use mm2_test_helpers::for_tests::mm_ctx_with_custom_db;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn test_storage_init() {
        let ctx = mm_ctx_with_custom_db();
        let storage = IDBBlockHeadersStorage::new(&ctx, "RICK".to_string());

        register_wasm_log();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);

        // repetitive init must not fail
        storage.init().await.unwrap();

        let initialized = storage.is_initialized_for().await.unwrap();
        assert!(initialized);
    }

    #[wasm_bindgen_test]
    async fn test_add_block_headers() { test_add_block_headers_impl().await }

    #[wasm_bindgen_test]
    async fn test_test_get_block_header() { test_get_block_header_impl().await }

    #[wasm_bindgen_test]
    async fn test_get_last_block_header_with_non_max_bits() {
        test_get_last_block_header_with_non_max_bits_impl().await
    }

    #[wasm_bindgen_test]
    async fn test_get_last_block_height() { test_get_last_block_height_impl().await }

    #[wasm_bindgen_test]
    async fn test_remove_headers_up_to_height() { test_remove_headers_up_to_height_impl().await }
}
