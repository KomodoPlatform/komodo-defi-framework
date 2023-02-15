#[cfg(target_arch = "wasm32")] mod indexedb_block_header_storage;
#[cfg(target_arch = "wasm32")]
pub use indexedb_block_header_storage::IndexedDBBlockHeadersStorage;

#[cfg(not(target_arch = "wasm32"))] mod sql_block_header_storage;
#[cfg(not(target_arch = "wasm32"))]
pub use sql_block_header_storage::SqliteBlockHeadersStorage;

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
    pub(crate) fn new_from_ctx(_ctx: MmArc, _ticker: String) -> Result<Self, BlockHeaderStorageError> {
        Ok(BlockHeaderStorage {
            inner: Box::new(IndexedDBBlockHeadersStorage {}),
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
