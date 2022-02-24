use crate::utxo::rpc_clients::ElectrumBlockHeader;
use crate::utxo::utxo_block_header_storage::BlockHeaderStorageError;
use crate::utxo::utxo_block_header_storage::BlockHeaderStorageOps;
use async_trait::async_trait;
use chain::BlockHeader;
use common::mm_error::MmError;
use std::collections::HashMap;

#[derive(Debug)]
pub struct IndexedDBBlockHeadersStorage {}

#[async_trait]
impl BlockHeaderStorageOps for IndexedDBBlockHeadersStorage {
    async fn init(&self, _for_coin: &str) -> Result<(), MmError<BlockHeaderStorageError>> { Ok(()) }

    async fn is_initialized_for(&self, _for_coin: &str) -> Result<bool, MmError<BlockHeaderStorageError>> { Ok(true) }

    async fn add_electrum_block_headers_to_storage(
        &self,
        _for_coin: &str,
        _headers: Vec<ElectrumBlockHeader>,
    ) -> Result<(), MmError<BlockHeaderStorageError>> {
        Ok(())
    }

    async fn add_block_headers_to_storage(
        &self,
        _for_coin: &str,
        _headers: HashMap<u64, BlockHeader>,
    ) -> Result<(), MmError<BlockHeaderStorageError>> {
        Ok(())
    }

    async fn get_block_header(
        &self,
        _for_coin: &str,
        _height: u64,
    ) -> Result<Option<BlockHeader>, MmError<BlockHeaderStorageError>> {
        Ok(None)
    }

    async fn get_block_header_raw(
        &self,
        _for_coin: &str,
        _height: u64,
    ) -> Result<Option<String>, MmError<BlockHeaderStorageError>> {
        Ok(None)
    }
}
