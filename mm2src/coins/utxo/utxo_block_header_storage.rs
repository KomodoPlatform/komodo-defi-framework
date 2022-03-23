use crate::utxo::rpc_clients::ElectrumBlockHeader;
#[cfg(target_arch = "wasm32")]
use crate::utxo::utxo_indexedb_block_header_storage::IndexedDBBlockHeadersStorage;
#[cfg(not(target_arch = "wasm32"))]
use crate::utxo::utxo_sql_block_header_storage::SqliteBlockHeadersStorage;
use crate::utxo::UtxoBlockHeaderVerificationParams;
use async_trait::async_trait;
use chain::BlockHeader;
use common::mm_ctx::MmArc;
use common::mm_error::MmError;
use derive_more::Display;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};

#[derive(Debug, Display)]
pub enum BlockHeaderStorageError {
    #[display(fmt = "Can't add to the storage for {} - reason: {}", ticker, reason)]
    AddToStorageError { ticker: String, reason: String },
    #[display(fmt = "Can't get from the storage for {} - reason: {}", ticker, reason)]
    GetFromStorageError { ticker: String, reason: String },
    #[display(
        fmt = "Can't retrieve the table from the storage for {} - reason: {}",
        ticker,
        reason
    )]
    CantRetrieveTableError { ticker: String, reason: String },
    #[display(fmt = "Can't query from the storage - query: {} - reason: {}", query, reason)]
    QueryError { query: String, reason: String },
    #[display(fmt = "Can't init from the storage - ticker: {} - reason: {}", ticker, reason)]
    InitializationError { ticker: String, reason: String },
    #[display(fmt = "Can't decode/deserialize from storage for {} - reason: {}", ticker, reason)]
    DecodeError { ticker: String, reason: String },
}

pub struct BlockHeaderStorage {
    pub inner: Box<dyn BlockHeaderStorageOps>,
    pub params: UtxoBlockHeaderVerificationParams,
}

impl Debug for BlockHeaderStorage {
    fn fmt(&self, _f: &mut Formatter<'_>) -> std::fmt::Result { Ok(()) }
}

pub trait InitBlockHeaderStorageOps: Send + Sync + 'static {
    fn new_from_ctx(ctx: MmArc, params: UtxoBlockHeaderVerificationParams) -> Option<BlockHeaderStorage>
    where
        Self: Sized;
}

#[async_trait]
pub trait BlockHeaderStorageOps: Send + Sync + 'static {
    /// Initializes collection/tables in storage for a specified coin
    async fn init(&self, for_coin: &str) -> Result<(), MmError<BlockHeaderStorageError>>;

    async fn is_initialized_for(&self, for_coin: &str) -> Result<bool, MmError<BlockHeaderStorageError>>;

    // Adds multiple block headers to the selected coin's header storage
    // Should store it as `TICKER_HEIGHT=hex_string`
    // use this function for headers that comes from `blockchain_headers_subscribe`
    async fn add_electrum_block_headers_to_storage(
        &self,
        for_coin: &str,
        headers: Vec<ElectrumBlockHeader>,
    ) -> Result<(), MmError<BlockHeaderStorageError>>;

    // Adds multiple block headers to the selected coin's header storage
    // Should store it as `TICKER_HEIGHT=hex_string`
    // use this function for headers that comes from `blockchain_block_headers`
    async fn add_block_headers_to_storage(
        &self,
        for_coin: &str,
        headers: HashMap<u64, BlockHeader>,
    ) -> Result<(), MmError<BlockHeaderStorageError>>;

    /// Gets the block header by height from the selected coin's storage as BlockHeader
    async fn get_block_header(
        &self,
        for_coin: &str,
        height: u64,
    ) -> Result<Option<BlockHeader>, MmError<BlockHeaderStorageError>>;

    /// Gets the block header by height from the selected coin's storage as hex
    async fn get_block_header_raw(
        &self,
        for_coin: &str,
        height: u64,
    ) -> Result<Option<String>, MmError<BlockHeaderStorageError>>;
}

impl InitBlockHeaderStorageOps for BlockHeaderStorage {
    #[cfg(not(target_arch = "wasm32"))]
    fn new_from_ctx(ctx: MmArc, params: UtxoBlockHeaderVerificationParams) -> Option<BlockHeaderStorage> {
        ctx.sqlite_connection.as_option().map(|connection| BlockHeaderStorage {
            inner: Box::new(SqliteBlockHeadersStorage(connection.clone())),
            params,
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn new_from_ctx(_ctx: MmArc, params: UtxoBlockHeaderVerificationParams) -> Option<BlockHeaderStorage> {
        Some(BlockHeaderStorage {
            inner: Box::new(IndexedDBBlockHeadersStorage {}),
            params,
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
