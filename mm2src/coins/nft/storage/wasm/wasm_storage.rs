use crate::nft::nft_structs::{Chain, ContractType, Nft, NftList, NftTransferHistory, NftsTransferHistoryList,
                              TransferStatus, TxMeta};
use crate::nft::storage::wasm::nft_idb::{NftCacheIDB, NftCacheIDBLocked};
use crate::nft::storage::wasm::{WasmNftCacheError, WasmNftCacheResult};
use crate::nft::storage::{CreateNftStorageError, NftListStorageOps, NftTokenAddrId, NftTxHistoryFilters,
                          NftTxHistoryStorageOps, RemoveNftResult};
use crate::CoinsContext;
use async_trait::async_trait;
use mm2_core::mm_ctx::MmArc;
use mm2_db::indexed_db::{BeBigUint, DbUpgrader, OnUpgradeResult, SharedDb, TableSignature};
use mm2_err_handle::map_mm_error::MapMmError;
use mm2_err_handle::map_to_mm::MapToMmResult;
use mm2_err_handle::prelude::MmResult;
use mm2_number::BigDecimal;
use serde_json::Value as Json;
use std::num::NonZeroUsize;

#[derive(Clone)]
pub struct IndexedDbNftStorage {
    db: SharedDb<NftCacheIDB>,
}

impl IndexedDbNftStorage {
    pub fn new(ctx: &MmArc) -> MmResult<Self, CreateNftStorageError> {
        let coins_ctx = CoinsContext::from_ctx(ctx).map_to_mm(CreateNftStorageError::Internal)?;
        Ok(IndexedDbNftStorage {
            db: coins_ctx.nft_cache_db.clone(),
        })
    }

    #[allow(dead_code)]
    async fn lock_db(&self) -> WasmNftCacheResult<NftCacheIDBLocked<'_>> {
        self.db.get_or_initialize().await.mm_err(WasmNftCacheError::from)
    }
}

#[async_trait]
impl NftListStorageOps for IndexedDbNftStorage {
    type Error = WasmNftCacheError;

    async fn init(&self, _chain: &Chain) -> MmResult<(), Self::Error> { todo!() }

    async fn is_initialized(&self, _chain: &Chain) -> MmResult<bool, Self::Error> { todo!() }

    async fn get_nft_list(
        &self,
        _chains: Vec<Chain>,
        _max: bool,
        _limit: usize,
        _page_number: Option<NonZeroUsize>,
    ) -> MmResult<NftList, Self::Error> {
        todo!()
    }

    async fn add_nfts_to_list<I>(&self, _chain: &Chain, _nfts: I, _last_scanned_block: u32) -> MmResult<(), Self::Error>
    where
        I: IntoIterator<Item = Nft> + Send + 'static,
        I::IntoIter: Send,
    {
        todo!()
    }

    async fn get_nft(
        &self,
        _chain: &Chain,
        _token_address: String,
        _token_id: BigDecimal,
    ) -> MmResult<Option<Nft>, Self::Error> {
        todo!()
    }

    async fn remove_nft_from_list(
        &self,
        _chain: &Chain,
        _token_address: String,
        _token_id: BigDecimal,
        _scanned_block: u64,
    ) -> MmResult<RemoveNftResult, Self::Error> {
        todo!()
    }

    async fn get_nft_amount(
        &self,
        _chain: &Chain,
        _token_address: String,
        _token_id: BigDecimal,
    ) -> MmResult<Option<String>, Self::Error> {
        todo!()
    }

    async fn refresh_nft_metadata(&self, _chain: &Chain, _nft: Nft) -> MmResult<(), Self::Error> { todo!() }

    async fn get_last_block_number(&self, _chain: &Chain) -> MmResult<Option<u32>, Self::Error> { todo!() }

    async fn get_last_scanned_block(&self, _chain: &Chain) -> MmResult<Option<u32>, Self::Error> { todo!() }

    async fn update_nft_amount(&self, _chain: &Chain, _nft: Nft, _scanned_block: u64) -> MmResult<(), Self::Error> {
        todo!()
    }

    async fn update_nft_amount_and_block_number(&self, _chain: &Chain, _nft: Nft) -> MmResult<(), Self::Error> {
        todo!()
    }
}

#[async_trait]
impl NftTxHistoryStorageOps for IndexedDbNftStorage {
    type Error = WasmNftCacheError;

    async fn init(&self, _chain: &Chain) -> MmResult<(), Self::Error> { todo!() }

    async fn is_initialized(&self, _chain: &Chain) -> MmResult<bool, Self::Error> { todo!() }

    async fn get_tx_history(
        &self,
        _chains: Vec<Chain>,
        _max: bool,
        _limit: usize,
        _page_number: Option<NonZeroUsize>,
        _filters: Option<NftTxHistoryFilters>,
    ) -> MmResult<NftsTransferHistoryList, Self::Error> {
        todo!()
    }

    async fn add_txs_to_history<I>(&self, _chain: &Chain, _txs: I) -> MmResult<(), Self::Error>
    where
        I: IntoIterator<Item = NftTransferHistory> + Send + 'static,
        I::IntoIter: Send,
    {
        todo!()
    }

    async fn get_last_block_number(&self, _chain: &Chain) -> MmResult<Option<u32>, Self::Error> { todo!() }

    async fn get_txs_from_block(
        &self,
        _chain: &Chain,
        _from_block: u32,
    ) -> MmResult<Vec<NftTransferHistory>, Self::Error> {
        todo!()
    }

    async fn get_txs_by_token_addr_id(
        &self,
        _chain: &Chain,
        _token_address: String,
        _token_id: BigDecimal,
    ) -> MmResult<Vec<NftTransferHistory>, Self::Error> {
        todo!()
    }

    async fn get_tx_by_tx_hash(
        &self,
        _chain: &Chain,
        _transaction_hash: String,
    ) -> MmResult<Option<NftTransferHistory>, Self::Error> {
        todo!()
    }

    async fn update_tx_meta_by_hash(&self, _chain: &Chain, _tx: NftTransferHistory) -> MmResult<(), Self::Error> {
        todo!()
    }

    async fn update_txs_meta_by_token_addr_id(&self, _chain: &Chain, _tx_meta: TxMeta) -> MmResult<(), Self::Error> {
        todo!()
    }

    async fn get_txs_with_empty_meta(&self, _chain: &Chain) -> MmResult<Vec<NftTokenAddrId>, Self::Error> { todo!() }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct NftListTable {
    token_address: String,
    token_id: String,
    chain: String,
    amount: String,
    block_number: BeBigUint,
    contract_type: ContractType,
    details_json: Json,
}

impl NftListTable {
    pub const CHAIN_TOKEN_ADD_TOKEN_ID_INDEX: &str = "chain_token_add_token_id_index";
}

impl TableSignature for NftListTable {
    fn table_name() -> &'static str { "nft_list_cache_table" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(
                Self::CHAIN_TOKEN_ADD_TOKEN_ID_INDEX,
                &["chain", "token_address", "token_id"],
                true,
            )?;
            table.create_index("chain", false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct NftTxHistoryTable {
    transaction_hash: String,
    chain: String,
    block_number: BeBigUint,
    block_timestamp: u64,
    contract_type: ContractType,
    token_address: String,
    token_id: String,
    status: TransferStatus,
    amount: String,
    collection_name: String,
    image: String,
    token_name: String,
    details_json: Json,
}

impl NftTxHistoryTable {
    pub const CHAIN_TX_HASH_INDEX: &str = "chain_tx_hash_index";
}

impl TableSignature for NftTxHistoryTable {
    fn table_name() -> &'static str { "nft_tx_history_cache_table" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(Self::CHAIN_TX_HASH_INDEX, &["chain", "transaction_hash"], true)?;
            table.create_index("chain", false)?;
        }
        Ok(())
    }
}
