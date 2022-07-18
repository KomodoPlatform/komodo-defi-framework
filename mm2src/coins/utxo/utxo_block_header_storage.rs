#[cfg(target_arch = "wasm32")]
use crate::utxo::utxo_indexedb_block_header_storage::IndexedDBBlockHeadersStorage;
#[cfg(not(target_arch = "wasm32"))]
use crate::utxo::utxo_sql_block_header_storage::SqliteBlockHeadersStorage;
use async_trait::async_trait;
use chain::BlockHeader;
use mm2_core::mm_ctx::MmArc;
use primitives::hash::H256;
use spv_validation::storage::{BlockHeaderStorageError, BlockHeaderStorageOps};
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::num::NonZeroU64;

/// SPV headers verification parameters
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BlockHeaderVerificationParams {
    pub difficulty_check: bool,
    pub constant_difficulty: bool,
    // This should to be equal to or greater than the number of blocks needed before the chain is safe from reorganization (e.g. 6 blocks for BTC)
    pub blocks_limit_to_check: NonZeroU64,
    pub check_every: f64,
}

pub struct BlockHeaderStorage {
    pub inner: Box<dyn BlockHeaderStorageOps>,
    pub params: BlockHeaderVerificationParams,
}

impl Debug for BlockHeaderStorage {
    fn fmt(&self, _f: &mut Formatter<'_>) -> std::fmt::Result { Ok(()) }
}

pub trait InitBlockHeaderStorageOps: Send + Sync + 'static {
    fn new_from_ctx(
        ctx: MmArc,
        params: BlockHeaderVerificationParams,
    ) -> Result<BlockHeaderStorage, BlockHeaderStorageError>
    where
        Self: Sized;
}

impl InitBlockHeaderStorageOps for BlockHeaderStorage {
    #[cfg(not(target_arch = "wasm32"))]
    fn new_from_ctx(ctx: MmArc, params: BlockHeaderVerificationParams) -> Result<Self, BlockHeaderStorageError> {
        let sqlite_connection = ctx.sqlite_connection.ok_or(BlockHeaderStorageError::Internal(
            "sqlite_connection is not initialized".to_owned(),
        ))?;
        Ok(BlockHeaderStorage {
            inner: Box::new(SqliteBlockHeadersStorage(sqlite_connection.clone())),
            params,
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn new_from_ctx(_ctx: MmArc, params: BlockHeaderVerificationParams) -> Result<Self, BlockHeaderStorageError> {
        Ok(BlockHeaderStorage {
            inner: Box::new(IndexedDBBlockHeadersStorage {}),
            params,
        })
    }
}

#[async_trait]
impl BlockHeaderStorageOps for BlockHeaderStorage {
    async fn init(&self, for_coin: &str) -> Result<(), BlockHeaderStorageError> { self.inner.init(for_coin).await }

    async fn is_initialized_for(&self, for_coin: &str) -> Result<bool, BlockHeaderStorageError> {
        self.inner.is_initialized_for(for_coin).await
    }

    async fn add_block_headers_to_storage(
        &self,
        for_coin: &str,
        headers: HashMap<u64, BlockHeader>,
    ) -> Result<(), BlockHeaderStorageError> {
        self.inner.add_block_headers_to_storage(for_coin, headers).await
    }

    async fn get_block_header(
        &self,
        for_coin: &str,
        height: u64,
    ) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        self.inner.get_block_header(for_coin, height).await
    }

    async fn get_block_header_raw(
        &self,
        for_coin: &str,
        height: u64,
    ) -> Result<Option<String>, BlockHeaderStorageError> {
        self.inner.get_block_header_raw(for_coin, height).await
    }

    async fn get_last_block_header_with_non_max_bits(
        &self,
        for_coin: &str,
    ) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        self.inner.get_last_block_header_with_non_max_bits(for_coin).await
    }

    async fn get_block_height_by_hash(
        &self,
        for_coin: &str,
        hash: H256,
    ) -> Result<Option<i64>, BlockHeaderStorageError> {
        self.inner.get_block_height_by_hash(for_coin, hash).await
    }
}
