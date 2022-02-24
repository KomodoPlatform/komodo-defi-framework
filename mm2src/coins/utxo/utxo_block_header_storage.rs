use crate::utxo::rpc_clients::ElectrumBlockHeader;
use async_trait::async_trait;
use chain::BlockHeader;
use common::mm_error::MmError;
use derive_more::Display;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum BlockHeaderStorageError {
    #[display(fmt = "The storage cannot be initialized for {}", _0)]
    InitializationError(String),
    #[display(fmt = "The storage is not initialized for {} - reason: {}", ticker, reason)]
    NotInitializedError { ticker: String, reason: String },
    #[display(fmt = "Can't add to the storage for {} - reason: {}", ticker, reason)]
    AddToStorageError { ticker: String, reason: String },
    #[display(fmt = "Can't add to the storage for {} - reason: {}", ticker, reason)]
    GetFromStorageError { ticker: String, reason: String },
    #[display(
        fmt = "Can't retrieve the table from the storage for {} - reason: {}",
        ticker,
        reason
    )]
    CantRetrieveTableError { ticker: String, reason: String },
    #[display(fmt = "Can't query from the storage - query: {} - reason: {}", query, reason)]
    QueryError { query: String, reason: String },
    #[display(fmt = "Can't retrieve string from row - reason: {}", reason)]
    StringRowError { reason: String },
    #[display(fmt = "Can't execute query from storage for {} - reason: {}", ticker, reason)]
    ExecutionError { ticker: String, reason: String },
    #[display(fmt = "Can't start a transaction from storage for {} - reason: {}", ticker, reason)]
    TransactionError { ticker: String, reason: String },
    #[display(fmt = "Can't commit a transaction from storage for {} - reason: {}", ticker, reason)]
    CommitError { ticker: String, reason: String },
    #[display(fmt = "Can't decode/deserialize from storage for {} - reason: {}", ticker, reason)]
    DecodeError { ticker: String, reason: String },
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
