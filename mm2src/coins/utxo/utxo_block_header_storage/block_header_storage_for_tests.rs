use async_trait::async_trait;
use chain::BlockHeader;
use futures::lock::Mutex as AsyncMutex;
use primitives::hash::H256;
use spv_validation::storage::{BlockHeaderStorageError, BlockHeaderStorageOps};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[derive(Clone)]
pub struct BlockHeaderStorageForTests {
    pub inner: Arc<AsyncMutex<BlockHeaderStorageForTestsImpl>>,
}

impl BlockHeaderStorageForTests {
    pub fn new(ticker: String) -> BlockHeaderStorageForTests {
        BlockHeaderStorageForTests {
            inner: Arc::new(AsyncMutex::new(BlockHeaderStorageForTestsImpl {
                ticker,
                block_headers: BTreeMap::new(),
                block_headers_by_hash: HashMap::new(),
            })),
        }
    }
}

#[derive(Clone)]
pub struct BlockHeaderStorageForTestsImpl {
    pub ticker: String,
    // The block headers should be ordered to be able to return the last block header.
    pub block_headers: BTreeMap<u64, BlockHeader>,
    // This can be used on [`BlockHeaderStorageOps::get_block_height_by_hash`].
    pub block_headers_by_hash: HashMap<H256, u64>,
}

#[async_trait]
impl BlockHeaderStorageOps for BlockHeaderStorageForTests {
    async fn init(&self) -> Result<(), BlockHeaderStorageError> { Ok(()) }

    async fn is_initialized_for(&self) -> Result<bool, BlockHeaderStorageError> { Ok(true) }

    async fn add_block_headers_to_storage(
        &self,
        headers: HashMap<u64, BlockHeader>,
    ) -> Result<(), BlockHeaderStorageError> {
        let mut inner = self.inner.lock().await;
        inner
            .block_headers_by_hash
            .extend(headers.iter().map(|(height, header)| (header.hash(), *height)));
        inner.block_headers.extend(headers.into_iter());
        Ok(())
    }

    async fn get_block_header(&self, height: u64) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        Ok(self.inner.lock().await.block_headers.get(&height).cloned())
    }

    async fn get_block_header_raw(&self, height: u64) -> Result<Option<String>, BlockHeaderStorageError> {
        Ok(self
            .get_block_header(height)
            .await?
            .map(|header| hex::encode(header.raw())))
    }

    async fn get_last_block_height(&self) -> Result<u64, BlockHeaderStorageError> {
        Ok(self
            .inner
            .lock()
            .await
            .block_headers
            .iter()
            .last()
            .map(|(height, _header)| *height)
            .unwrap_or(0))
    }

    async fn get_last_block_header_with_non_max_bits(&self) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        todo!()
    }

    async fn get_block_height_by_hash(&self, hash: H256) -> Result<Option<i64>, BlockHeaderStorageError> {
        Ok(self
            .inner
            .lock()
            .await
            .block_headers_by_hash
            .get(&hash)
            .map(|height| *height as i64))
    }
}
