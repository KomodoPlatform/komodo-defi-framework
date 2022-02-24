use crate::utxo::rpc_clients::ElectrumBlockHeader;
use crate::utxo::utxo_block_header_storage::{BlockHeaderStorageError, BlockHeaderStorageOps};
#[cfg(target_arch = "wasm32")]
use crate::utxo::utxo_indexedb_block_header_storage::IndexedDBBlockHeadersStorage;
#[cfg(not(target_arch = "wasm32"))]
use crate::utxo::utxo_sql_block_header_storage::SqliteBlockHeadersStorage;
use async_trait::async_trait;
use chain::BlockHeader;
use common::mm_ctx::MmArc;
use common::mm_error::MmError;
use std::collections::HashMap;

#[derive(Debug)]
pub struct BlockHeaderStorage {
    #[cfg(not(target_arch = "wasm32"))]
    inner: SqliteBlockHeadersStorage,
    #[cfg(target_arch = "wasm32")]
    inner: IndexedDBBlockHeadersStorage,
}

impl BlockHeaderStorage {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new_from_ctx(ctx: MmArc) -> Option<BlockHeaderStorage> {
        ctx.sqlite_connection.as_option().map(|connection| BlockHeaderStorage {
            inner: SqliteBlockHeadersStorage(connection.clone()),
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub fn new_from_ctx(_ctx: MmArc) -> Option<BlockHeaderStorage> {
        Some(BlockHeaderStorage {
            inner: IndexedDBBlockHeadersStorage {},
        })
    }
}

#[async_trait]
impl BlockHeaderStorageOps for BlockHeaderStorage {
    async fn init(&self, for_coin: &str) -> Result<(), MmError<BlockHeaderStorageError>> {
        self.inner.init(for_coin).await
    }

    async fn is_initialized_for(&self, for_coin: &str) -> Result<bool, MmError<BlockHeaderStorageError>> {
        self.inner.is_initialized_for(for_coin).await
    }

    async fn add_electrum_block_headers_to_storage(
        &self,
        for_coin: &str,
        headers: Vec<ElectrumBlockHeader>,
    ) -> Result<(), MmError<BlockHeaderStorageError>> {
        self.inner
            .add_electrum_block_headers_to_storage(for_coin, headers)
            .await
    }

    async fn add_block_headers_to_storage(
        &self,
        for_coin: &str,
        headers: HashMap<u64, BlockHeader>,
    ) -> Result<(), MmError<BlockHeaderStorageError>> {
        self.inner.add_block_headers_to_storage(for_coin, headers).await
    }

    async fn get_block_header(
        &self,
        for_coin: &str,
        height: u64,
    ) -> Result<Option<BlockHeader>, MmError<BlockHeaderStorageError>> {
        self.inner.get_block_header(for_coin, height).await
    }

    async fn get_block_header_raw(
        &self,
        for_coin: &str,
        height: u64,
    ) -> Result<Option<String>, MmError<BlockHeaderStorageError>> {
        self.inner.get_block_header_raw(for_coin, height).await
    }
}
