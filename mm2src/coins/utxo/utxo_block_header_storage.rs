use crate::utxo::rpc_clients::ElectrumBlockHeader;
use async_trait::async_trait;
use chain::BlockHeader;
use common::{mm_error::MmError, mm_error::NotMmError, NotSame};
use std::collections::HashMap;

pub trait BlockHeaderStorageError: std::fmt::Debug + NotMmError + NotSame + Send {}

#[async_trait]
pub trait BlockHeaderStorage: Send + Sync + 'static {
    type Error: BlockHeaderStorageError;

    /// Initializes collection/tables in storage for a specified coin
    async fn init(&self, for_coin: &str) -> Result<(), MmError<Self::Error>>;

    async fn is_initialized_for(&self, for_coin: &str) -> Result<bool, MmError<Self::Error>>;

    // Adds multiple block headers to the selected coin's header storage
    // Should store it as `TICKER_HEIGHT=hex_string`
    // use this function for headers that comes from `blockchain_headers_subscribe`
    async fn add_electrum_block_headers_to_storage(
        &self,
        for_coin: &str,
        headers: Vec<ElectrumBlockHeader>,
    ) -> Result<(), MmError<Self::Error>>;

    // Adds multiple block headers to the selected coin's header storage
    // Should store it as `TICKER_HEIGHT=hex_string`
    // use this function for headers that comes from `blockchain_block_headers`
    async fn add_block_headers_to_storage(
        &self,
        for_coin: &str,
        headers: HashMap<u64, BlockHeader>,
    ) -> Result<(), MmError<Self::Error>>;

    /// Gets the block header by height from the selected coin's storage as BlockHeader
    async fn get_block_header(&self, for_coin: &str, height: u64) -> Result<Option<BlockHeader>, MmError<Self::Error>>;

    /// Gets the block header by height from the selected coin's storage as hex
    async fn get_block_header_raw(&self, for_coin: &str, height: u64) -> Result<Option<String>, MmError<Self::Error>>;
}
