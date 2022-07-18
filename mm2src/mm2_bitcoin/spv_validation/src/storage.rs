use async_trait::async_trait;
use chain::BlockHeader;
use derive_more::Display;
use primitives::hash::H256;
use std::collections::HashMap;

#[derive(Debug, Display)]
pub enum BlockHeaderStorageError {
    #[display(fmt = "Can't add to the storage for {} - reason: {}", coin, reason)]
    AddToStorageError {
        coin: String,
        reason: String,
    },
    #[display(fmt = "Can't get from the storage for {} - reason: {}", coin, reason)]
    GetFromStorageError {
        coin: String,
        reason: String,
    },
    #[display(fmt = "Can't retrieve the table from the storage for {} - reason: {}", coin, reason)]
    CantRetrieveTableError {
        coin: String,
        reason: String,
    },
    #[display(fmt = "Can't query from the storage - query: {} - reason: {}", query, reason)]
    QueryError {
        query: String,
        reason: String,
    },
    #[display(fmt = "Can't init from the storage - coin: {} - reason: {}", coin, reason)]
    InitializationError {
        coin: String,
        reason: String,
    },
    #[display(fmt = "Can't decode/deserialize from storage for {} - reason: {}", coin, reason)]
    DecodeError {
        coin: String,
        reason: String,
    },
    Internal(String),
}

#[async_trait]
pub trait BlockHeaderStorageOps: Send + Sync + 'static {
    /// Initializes collection/tables in storage for a specified coin
    async fn init(&self, for_coin: &str) -> Result<(), BlockHeaderStorageError>;

    async fn is_initialized_for(&self, for_coin: &str) -> Result<bool, BlockHeaderStorageError>;

    // Adds multiple block headers to the selected coin's header storage
    // Should store it as `COIN_HEIGHT=hex_string`
    // use this function for headers that comes from `blockchain_block_headers`
    async fn add_block_headers_to_storage(
        &self,
        for_coin: &str,
        headers: HashMap<u64, BlockHeader>,
    ) -> Result<(), BlockHeaderStorageError>;

    /// Gets the block header by height from the selected coin's storage as BlockHeader
    async fn get_block_header(
        &self,
        for_coin: &str,
        height: u64,
    ) -> Result<Option<BlockHeader>, BlockHeaderStorageError>;

    /// Gets the block header by height from the selected coin's storage as hex
    async fn get_block_header_raw(
        &self,
        for_coin: &str,
        height: u64,
    ) -> Result<Option<String>, BlockHeaderStorageError>;

    async fn get_last_block_header_with_non_max_bits(
        &self,
        for_coin: &str,
    ) -> Result<Option<BlockHeader>, BlockHeaderStorageError>;

    async fn get_block_height_by_hash(
        &self,
        for_coin: &str,
        hash: H256,
    ) -> Result<Option<i64>, BlockHeaderStorageError>;
}
