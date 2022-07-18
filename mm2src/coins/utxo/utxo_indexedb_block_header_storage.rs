use async_trait::async_trait;
use chain::BlockHeader;
use primitives::hash::H256;
use spv_validation::storage::{BlockHeaderStorageError, BlockHeaderStorageOps};
use std::collections::HashMap;

#[derive(Debug)]
pub struct IndexedDBBlockHeadersStorage {}

#[async_trait]
impl BlockHeaderStorageOps for IndexedDBBlockHeadersStorage {
    async fn init(&self, _for_coin: &str) -> Result<(), BlockHeaderStorageError> { Ok(()) }

    async fn is_initialized_for(&self, _for_coin: &str) -> Result<bool, BlockHeaderStorageError> { Ok(true) }

    async fn add_block_headers_to_storage(
        &self,
        _for_coin: &str,
        _headers: HashMap<u64, BlockHeader>,
    ) -> Result<(), BlockHeaderStorageError> {
        Ok(())
    }

    async fn get_block_header(
        &self,
        _for_coin: &str,
        _height: u64,
    ) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        Ok(None)
    }

    async fn get_block_header_raw(
        &self,
        _for_coin: &str,
        _height: u64,
    ) -> Result<Option<String>, BlockHeaderStorageError> {
        Ok(None)
    }

    async fn get_last_block_header_with_non_max_bits(
        &self,
        _for_coin: &str,
    ) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        Ok(None)
    }

    async fn get_block_height_by_hash(
        &self,
        for_coin: &str,
        hash: H256,
    ) -> Result<Option<i64>, BlockHeaderStorageError> {
        Ok(None)
    }
}
