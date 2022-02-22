use crate::utxo::rpc_clients::ElectrumBlockHeader;
use crate::utxo::utxo_block_header_storage::BlockHeaderStorage;
use crate::utxo::utxo_block_header_storage::BlockHeaderStorageError;
use async_trait::async_trait;
use chain::BlockHeader;
use common::indexed_db::DbTransactionError;
use common::mm_error::MmError;
use std::collections::HashMap;

impl BlockHeaderStorageError for DbTransactionError {}

pub struct IndexedDBBlockHeadersStorage {}

#[async_trait]
impl BlockHeaderStorage for IndexedDBBlockHeadersStorage {
    type Error = DbTransactionError;

    async fn init(&self, _for_coin: &str) -> Result<(), MmError<Self::Error>> { Ok(()) }

    async fn is_initialized_for(&self, _for_coin: &str) -> Result<bool, MmError<Self::Error>> { Ok(true) }

    async fn add_electrum_block_headers_to_storage(
        &self,
        _for_coin: &str,
        _headers: Vec<ElectrumBlockHeader>,
    ) -> Result<(), MmError<Self::Error>> {
        Ok(())
    }

    async fn add_block_headers_to_storage(
        &self,
        _for_coin: &str,
        _headers: HashMap<u64, BlockHeader>,
    ) -> Result<(), MmError<Self::Error>> {
        Ok(())
    }

    async fn get_block_header(
        &self,
        _for_coin: &str,
        _height: u64,
    ) -> Result<Option<BlockHeader>, MmError<Self::Error>> {
        Ok(None)
    }

    async fn get_block_header_raw(
        &self,
        _for_coin: &str,
        _height: u64,
    ) -> Result<Option<String>, MmError<Self::Error>> {
        Ok(None)
    }
}
