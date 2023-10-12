use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::str::FromStr;

use async_trait::async_trait;
use common::is_initial_upgrade;
use mm2_core::mm_ctx::MmArc;
use mm2_db::indexed_db::{BeBigUint, DbTable, DbUpgrader, MultiIndex, OnUpgradeResult, SharedDb, TableSignature};
use mm2_err_handle::map_mm_error::MapMmError;
use mm2_err_handle::map_to_mm::MapToMmResult;
use mm2_err_handle::prelude::MmResult;
use mm2_number::BigDecimal;
use num_traits::ToPrimitive;
use serde_json::{self as json, Value as Json};

use crate::eth::eth_addr_to_hex;
use crate::nft::nft_structs::{Chain, ContractType, Nft, NftCtx, NftList, NftTransferHistory, NftsTransferHistoryList,
                              TransferMeta, TransferStatus};
use crate::nft::storage::wasm::nft_idb::{NftCacheIDB, NftCacheIDBLocked};
use crate::nft::storage::wasm::{WasmNftCacheError, WasmNftCacheResult};
use crate::nft::storage::{get_offset_limit, CreateNftStorageError, NftListStorageOps, NftTokenAddrId,
                          NftTransferHistoryFilters, NftTransferHistoryStorageOps, RemoveNftResult};

#[derive(Clone)]
pub struct IndexedDbNftStorage {
    db: SharedDb<NftCacheIDB>,
}

impl IndexedDbNftStorage {
    pub fn new(ctx: &MmArc) -> MmResult<Self, CreateNftStorageError> {
        let nft_ctx = NftCtx::from_ctx(ctx).map_to_mm(CreateNftStorageError::Internal)?;
        Ok(IndexedDbNftStorage {
            db: nft_ctx.nft_cache_db.clone(),
        })
    }

    async fn lock_db(&self) -> WasmNftCacheResult<NftCacheIDBLocked<'_>> {
        self.db.get_or_initialize().await.mm_err(WasmNftCacheError::from)
    }

    fn take_nft_according_to_paging_opts(
        mut nfts: Vec<Nft>,
        max: bool,
        limit: usize,
        page_number: Option<NonZeroUsize>,
    ) -> WasmNftCacheResult<NftList> {
        let total_count = nfts.len();
        nfts.sort_by(|a, b| b.block_number.cmp(&a.block_number));
        let (offset, limit) = get_offset_limit(max, limit, page_number, total_count);
        Ok(NftList {
            nfts: nfts.into_iter().skip(offset).take(limit).collect(),
            skipped: offset,
            total: total_count,
        })
    }

    fn take_transfers_according_to_paging_opts(
        mut transfers: Vec<NftTransferHistory>,
        max: bool,
        limit: usize,
        page_number: Option<NonZeroUsize>,
    ) -> WasmNftCacheResult<NftsTransferHistoryList> {
        let total_count = transfers.len();
        transfers.sort_by(|a, b| b.block_timestamp.cmp(&a.block_timestamp));
        let (offset, limit) = get_offset_limit(max, limit, page_number, total_count);
        Ok(NftsTransferHistoryList {
            transfer_history: transfers.into_iter().skip(offset).take(limit).collect(),
            skipped: offset,
            total: total_count,
        })
    }

    fn take_transfers_according_to_filters<I>(
        transfers: I,
        filters: Option<NftTransferHistoryFilters>,
    ) -> WasmNftCacheResult<Vec<NftTransferHistory>>
    where
        I: Iterator<Item = NftTransferHistoryTable>,
    {
        let mut filtered_transfers = Vec::new();
        for transfers_table in transfers {
            let transfer = transfer_details_from_item(transfers_table)?;
            if let Some(filters) = &filters {
                if filters.is_status_match(&transfer) && filters.is_date_match(&transfer) {
                    filtered_transfers.push(transfer);
                }
            } else {
                filtered_transfers.push(transfer);
            }
        }
        Ok(filtered_transfers)
    }
}

impl NftTransferHistoryFilters {
    fn is_status_match(&self, transfer: &NftTransferHistory) -> bool {
        (!self.receive && !self.send)
            || (self.receive && transfer.status == TransferStatus::Receive)
            || (self.send && transfer.status == TransferStatus::Send)
    }

    fn is_date_match(&self, transfer: &NftTransferHistory) -> bool {
        self.from_date.map_or(true, |from| transfer.block_timestamp >= from)
            && self.to_date.map_or(true, |to| transfer.block_timestamp <= to)
    }
}

#[async_trait]
impl NftListStorageOps for IndexedDbNftStorage {
    type Error = WasmNftCacheError;

    async fn init(&self, _chain: &Chain) -> MmResult<(), Self::Error> { Ok(()) }

    async fn is_initialized(&self, _chain: &Chain) -> MmResult<bool, Self::Error> { Ok(true) }

    async fn get_nft_list(
        &self,
        chains: Vec<Chain>,
        max: bool,
        limit: usize,
        page_number: Option<NonZeroUsize>,
    ) -> MmResult<NftList, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;
        let mut nfts = Vec::new();
        for chain in chains {
            let items = table.get_items("chain", chain.to_string()).await?;
            for (_item_id, item) in items.into_iter() {
                let nft_detail = nft_details_from_item(item)?;
                nfts.push(nft_detail);
            }
        }
        Self::take_nft_according_to_paging_opts(nfts, max, limit, page_number)
    }

    async fn add_nfts_to_list<I>(&self, chain: &Chain, nfts: I, last_scanned_block: u64) -> MmResult<(), Self::Error>
    where
        I: IntoIterator<Item = Nft> + Send + 'static,
        I::IntoIter: Send,
    {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let nft_table = db_transaction.table::<NftListTable>().await?;
        let last_scanned_block_table = db_transaction.table::<LastScannedBlockTable>().await?;
        for nft in nfts {
            let nft_item = NftListTable::from_nft(&nft)?;
            nft_table.add_item(&nft_item).await?;
        }
        let last_scanned_block = LastScannedBlockTable {
            chain: chain.to_string(),
            last_scanned_block: BeBigUint::from(last_scanned_block),
        };
        last_scanned_block_table
            .replace_item_by_unique_index("chain", chain.to_string(), &last_scanned_block)
            .await?;
        Ok(())
    }

    async fn get_nft(
        &self,
        chain: &Chain,
        token_address: String,
        token_id: BigDecimal,
    ) -> MmResult<Option<Nft>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;
        let index_keys = MultiIndex::new(NftListTable::CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(&token_address)?
            .with_value(token_id.to_string())?;

        if let Some((_item_id, item)) = table.get_item_by_unique_multi_index(index_keys).await? {
            Ok(Some(nft_details_from_item(item)?))
        } else {
            Ok(None)
        }
    }

    async fn remove_nft_from_list(
        &self,
        chain: &Chain,
        token_address: String,
        token_id: BigDecimal,
        scanned_block: u64,
    ) -> MmResult<RemoveNftResult, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let nft_table = db_transaction.table::<NftListTable>().await?;
        let last_scanned_block_table = db_transaction.table::<LastScannedBlockTable>().await?;

        let index_keys = MultiIndex::new(NftListTable::CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(&token_address)?
            .with_value(token_id.to_string())?;

        let last_scanned_block = LastScannedBlockTable {
            chain: chain.to_string(),
            last_scanned_block: BeBigUint::from(scanned_block),
        };

        let nft_removed = nft_table.delete_item_by_unique_multi_index(index_keys).await?.is_some();
        last_scanned_block_table
            .replace_item_by_unique_index("chain", chain.to_string(), &last_scanned_block)
            .await?;
        if nft_removed {
            Ok(RemoveNftResult::NftRemoved)
        } else {
            Ok(RemoveNftResult::NftDidNotExist)
        }
    }

    async fn get_nft_amount(
        &self,
        chain: &Chain,
        token_address: String,
        token_id: BigDecimal,
    ) -> MmResult<Option<String>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;
        let index_keys = MultiIndex::new(NftListTable::CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(&token_address)?
            .with_value(token_id.to_string())?;

        if let Some((_item_id, item)) = table.get_item_by_unique_multi_index(index_keys).await? {
            Ok(Some(nft_details_from_item(item)?.common.amount.to_string()))
        } else {
            Ok(None)
        }
    }

    async fn refresh_nft_metadata(&self, chain: &Chain, nft: Nft) -> MmResult<(), Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;
        let index_keys = MultiIndex::new(NftListTable::CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(eth_addr_to_hex(&nft.common.token_address))?
            .with_value(nft.common.token_id.to_string())?;

        let nft_item = NftListTable::from_nft(&nft)?;
        table.replace_item_by_unique_multi_index(index_keys, &nft_item).await?;
        Ok(())
    }

    async fn get_last_block_number(&self, chain: &Chain) -> MmResult<Option<u64>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;
        get_last_block_from_table(chain, table, NftListTable::CHAIN_BLOCK_NUMBER_INDEX).await
    }

    async fn get_last_scanned_block(&self, chain: &Chain) -> MmResult<Option<u64>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<LastScannedBlockTable>().await?;
        if let Some((_item_id, item)) = table.get_item_by_unique_index("chain", chain.to_string()).await? {
            let last_scanned_block = item
                .last_scanned_block
                .to_u64()
                .ok_or_else(|| WasmNftCacheError::GetLastNftBlockError("height is too large".to_string()))?;
            Ok(Some(last_scanned_block))
        } else {
            Ok(None)
        }
    }

    async fn update_nft_amount(&self, chain: &Chain, nft: Nft, scanned_block: u64) -> MmResult<(), Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let nft_table = db_transaction.table::<NftListTable>().await?;
        let last_scanned_block_table = db_transaction.table::<LastScannedBlockTable>().await?;

        let index_keys = MultiIndex::new(NftListTable::CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(eth_addr_to_hex(&nft.common.token_address))?
            .with_value(nft.common.token_id.to_string())?;

        let nft_item = NftListTable::from_nft(&nft)?;
        nft_table
            .replace_item_by_unique_multi_index(index_keys, &nft_item)
            .await?;
        let last_scanned_block = LastScannedBlockTable {
            chain: chain.to_string(),
            last_scanned_block: BeBigUint::from(scanned_block),
        };
        last_scanned_block_table
            .replace_item_by_unique_index("chain", chain.to_string(), &last_scanned_block)
            .await?;
        Ok(())
    }

    async fn update_nft_amount_and_block_number(&self, chain: &Chain, nft: Nft) -> MmResult<(), Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let nft_table = db_transaction.table::<NftListTable>().await?;
        let last_scanned_block_table = db_transaction.table::<LastScannedBlockTable>().await?;

        let index_keys = MultiIndex::new(NftListTable::CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(eth_addr_to_hex(&nft.common.token_address))?
            .with_value(nft.common.token_id.to_string())?;

        let nft_item = NftListTable::from_nft(&nft)?;
        nft_table
            .replace_item_by_unique_multi_index(index_keys, &nft_item)
            .await?;
        let last_scanned_block = LastScannedBlockTable {
            chain: chain.to_string(),
            last_scanned_block: BeBigUint::from(nft.block_number),
        };
        last_scanned_block_table
            .replace_item_by_unique_index("chain", chain.to_string(), &last_scanned_block)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl NftTransferHistoryStorageOps for IndexedDbNftStorage {
    type Error = WasmNftCacheError;

    async fn init(&self, _chain: &Chain) -> MmResult<(), Self::Error> { Ok(()) }

    async fn is_initialized(&self, _chain: &Chain) -> MmResult<bool, Self::Error> { Ok(true) }

    async fn get_transfer_history(
        &self,
        chains: Vec<Chain>,
        max: bool,
        limit: usize,
        page_number: Option<NonZeroUsize>,
        filters: Option<NftTransferHistoryFilters>,
    ) -> MmResult<NftsTransferHistoryList, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;
        let mut transfers = Vec::new();
        for chain in chains {
            let transfer_tables = table
                .get_items("chain", chain.to_string())
                .await?
                .into_iter()
                .map(|(_item_id, transfer)| transfer);
            let filtered = Self::take_transfers_according_to_filters(transfer_tables, filters)?;
            transfers.extend(filtered);
        }
        Self::take_transfers_according_to_paging_opts(transfers, max, limit, page_number)
    }

    async fn add_transfers_to_history<I>(&self, _chain: &Chain, transfers: I) -> MmResult<(), Self::Error>
    where
        I: IntoIterator<Item = NftTransferHistory> + Send + 'static,
        I::IntoIter: Send,
    {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;
        for transfer in transfers {
            let transfer_item = NftTransferHistoryTable::from_transfer_history(&transfer)?;
            table.add_item(&transfer_item).await?;
        }
        Ok(())
    }

    async fn get_last_block_number(&self, chain: &Chain) -> MmResult<Option<u64>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;
        get_last_block_from_table(chain, table, NftTransferHistoryTable::CHAIN_BLOCK_NUMBER_INDEX).await
    }

    async fn get_transfers_from_block(
        &self,
        chain: &Chain,
        from_block: u64,
    ) -> MmResult<Vec<NftTransferHistory>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;
        let items = table
            .cursor_builder()
            .only("chain", chain.to_string())
            .map_err(|e| WasmNftCacheError::GetLastNftBlockError(e.to_string()))?
            .bound("block_number", BeBigUint::from(from_block), BeBigUint::from(u64::MAX))
            .open_cursor(NftTransferHistoryTable::CHAIN_BLOCK_NUMBER_INDEX)
            .await
            .map_err(|e| WasmNftCacheError::GetLastNftBlockError(e.to_string()))?
            .collect()
            .await
            .map_err(|e| WasmNftCacheError::GetLastNftBlockError(e.to_string()))?;

        let mut res = Vec::new();
        for (_item_id, item) in items.into_iter() {
            let transfer = transfer_details_from_item(item)?;
            res.push(transfer);
        }
        Ok(res)
    }

    async fn get_transfers_by_token_addr_id(
        &self,
        chain: &Chain,
        token_address: String,
        token_id: BigDecimal,
    ) -> MmResult<Vec<NftTransferHistory>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;

        let index_keys = MultiIndex::new(NftTransferHistoryTable::CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(&token_address)?
            .with_value(token_id.to_string())?;

        table
            .get_items_by_multi_index(index_keys)
            .await?
            .into_iter()
            .map(|(_item_id, item)| transfer_details_from_item(item))
            .collect()
    }

    async fn get_transfer_by_tx_hash_and_log_index(
        &self,
        chain: &Chain,
        transaction_hash: String,
        log_index: u32,
    ) -> MmResult<Option<NftTransferHistory>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;
        let index_keys = MultiIndex::new(NftTransferHistoryTable::CHAIN_TX_HASH_LOG_INDEX_INDEX)
            .with_value(chain.to_string())?
            .with_value(&transaction_hash)?
            .with_value(log_index)?;

        if let Some((_item_id, item)) = table.get_item_by_unique_multi_index(index_keys).await? {
            Ok(Some(transfer_details_from_item(item)?))
        } else {
            Ok(None)
        }
    }

    async fn update_transfer_meta_by_hash_and_log_index(
        &self,
        chain: &Chain,
        transfer: NftTransferHistory,
    ) -> MmResult<(), Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;

        let index_keys = MultiIndex::new(NftTransferHistoryTable::CHAIN_TX_HASH_LOG_INDEX_INDEX)
            .with_value(chain.to_string())?
            .with_value(&transfer.common.transaction_hash)?
            .with_value(transfer.common.log_index)?;

        let item = NftTransferHistoryTable::from_transfer_history(&transfer)?;
        table.replace_item_by_unique_multi_index(index_keys, &item).await?;
        Ok(())
    }

    async fn update_transfers_meta_by_token_addr_id(
        &self,
        chain: &Chain,
        transfer_meta: TransferMeta,
    ) -> MmResult<(), Self::Error> {
        let transfers: Vec<NftTransferHistory> = self
            .get_transfers_by_token_addr_id(chain, transfer_meta.token_address, transfer_meta.token_id)
            .await?;
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;
        for mut transfer in transfers {
            transfer.token_uri = transfer_meta.token_uri.clone();
            transfer.collection_name = transfer_meta.collection_name.clone();
            transfer.image_url = transfer_meta.image_url.clone();
            transfer.token_name = transfer_meta.token_name.clone();
            drop_mutability!(transfer);

            let index_keys = MultiIndex::new(NftTransferHistoryTable::CHAIN_TX_HASH_LOG_INDEX_INDEX)
                .with_value(chain.to_string())?
                .with_value(&transfer.common.transaction_hash)?
                .with_value(transfer.common.log_index)?;

            let item = NftTransferHistoryTable::from_transfer_history(&transfer)?;
            table.replace_item_by_unique_multi_index(index_keys, &item).await?;
        }
        Ok(())
    }

    async fn get_transfers_with_empty_meta(&self, chain: &Chain) -> MmResult<Vec<NftTokenAddrId>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;
        let items = table
            .cursor_builder()
            .only("chain", chain.to_string())
            .map_err(|e| WasmNftCacheError::GetLastNftBlockError(e.to_string()))?
            .open_cursor("chain")
            .await
            .map_err(|e| WasmNftCacheError::GetLastNftBlockError(e.to_string()))?
            .collect()
            .await
            .map_err(|e| WasmNftCacheError::GetLastNftBlockError(e.to_string()))?;

        let mut res = HashSet::new();
        for (_item_id, item) in items.into_iter() {
            if item.token_uri.is_none()
                && item.collection_name.is_none()
                && item.image_url.is_none()
                && item.token_name.is_none()
            {
                res.insert(NftTokenAddrId {
                    token_address: item.token_address,
                    token_id: BigDecimal::from_str(&item.token_id).map_err(WasmNftCacheError::ParseBigDecimalError)?,
                });
            }
        }
        Ok(res.into_iter().collect())
    }
}

/// `get_last_block_from_table` function returns the highest block in the table related to certain blockchain type.
async fn get_last_block_from_table(
    chain: &Chain,
    table: DbTable<'_, impl TableSignature + BlockNumberTable>,
    cursor: &str,
) -> MmResult<Option<u64>, WasmNftCacheError> {
    let items = table
        .cursor_builder()
        .only("chain", chain.to_string())
        .map_err(|e| WasmNftCacheError::GetLastNftBlockError(e.to_string()))?
        // Sets lower and upper bounds for block_number field
        .bound("block_number", BeBigUint::from(0u64), BeBigUint::from(u64::MAX))
        // Opens a cursor by the specified index.
        // In get_last_block_from_table case it is CHAIN_BLOCK_NUMBER_INDEX, as we need to search block_number for specific chain.
        // Cursor returns values from the lowest to highest key indexes.
        .open_cursor(cursor)
        .await
        .map_err(|e| WasmNftCacheError::GetLastNftBlockError(e.to_string()))?
        .collect()
        .await
        .map_err(|e| WasmNftCacheError::GetLastNftBlockError(e.to_string()))?;

    let maybe_item = items
        .into_iter()
        .last()
        .map(|(_item_id, item)| {
            item.get_block_number()
                .to_u64()
                .ok_or_else(|| WasmNftCacheError::GetLastNftBlockError("height is too large".to_string()))
        })
        .transpose()?;
    Ok(maybe_item)
}

trait BlockNumberTable {
    fn get_block_number(&self) -> &BeBigUint;
}

impl BlockNumberTable for NftListTable {
    fn get_block_number(&self) -> &BeBigUint { &self.block_number }
}

impl BlockNumberTable for NftTransferHistoryTable {
    fn get_block_number(&self) -> &BeBigUint { &self.block_number }
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
    const CHAIN_TOKEN_ADD_TOKEN_ID_INDEX: &str = "chain_token_add_token_id_index";

    const CHAIN_BLOCK_NUMBER_INDEX: &str = "chain_block_number_index";

    fn from_nft(nft: &Nft) -> WasmNftCacheResult<NftListTable> {
        let details_json = json::to_value(nft).map_to_mm(|e| WasmNftCacheError::ErrorSerializing(e.to_string()))?;
        Ok(NftListTable {
            token_address: eth_addr_to_hex(&nft.common.token_address),
            token_id: nft.common.token_id.to_string(),
            chain: nft.chain.to_string(),
            amount: nft.common.amount.to_string(),
            block_number: BeBigUint::from(nft.block_number),
            contract_type: nft.contract_type,
            details_json,
        })
    }
}

impl TableSignature for NftListTable {
    fn table_name() -> &'static str { "nft_list_cache_table" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if is_initial_upgrade(old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(
                Self::CHAIN_TOKEN_ADD_TOKEN_ID_INDEX,
                &["chain", "token_address", "token_id"],
                true,
            )?;
            table.create_multi_index(Self::CHAIN_BLOCK_NUMBER_INDEX, &["chain", "block_number"], false)?;
            table.create_index("chain", false)?;
            table.create_index("block_number", false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct NftTransferHistoryTable {
    transaction_hash: String,
    log_index: u32,
    chain: String,
    block_number: BeBigUint,
    block_timestamp: BeBigUint,
    contract_type: ContractType,
    token_address: String,
    token_id: String,
    status: TransferStatus,
    amount: String,
    token_uri: Option<String>,
    collection_name: Option<String>,
    image_url: Option<String>,
    token_name: Option<String>,
    details_json: Json,
}

impl NftTransferHistoryTable {
    const CHAIN_TOKEN_ADD_TOKEN_ID_INDEX: &str = "chain_token_add_token_id_index";

    const CHAIN_TX_HASH_LOG_INDEX_INDEX: &str = "chain_tx_hash_log_index_index";

    const CHAIN_BLOCK_NUMBER_INDEX: &str = "chain_block_number_index";

    fn from_transfer_history(transfer: &NftTransferHistory) -> WasmNftCacheResult<NftTransferHistoryTable> {
        let details_json =
            json::to_value(transfer).map_to_mm(|e| WasmNftCacheError::ErrorSerializing(e.to_string()))?;
        Ok(NftTransferHistoryTable {
            transaction_hash: transfer.common.transaction_hash.clone(),
            log_index: transfer.common.log_index,
            chain: transfer.chain.to_string(),
            block_number: BeBigUint::from(transfer.block_number),
            block_timestamp: BeBigUint::from(transfer.block_timestamp),
            contract_type: transfer.contract_type,
            token_address: eth_addr_to_hex(&transfer.common.token_address),
            token_id: transfer.common.token_id.to_string(),
            status: transfer.status,
            amount: transfer.common.amount.to_string(),
            token_uri: transfer.token_uri.clone(),
            collection_name: transfer.collection_name.clone(),
            image_url: transfer.image_url.clone(),
            token_name: transfer.token_name.clone(),
            details_json,
        })
    }
}

impl TableSignature for NftTransferHistoryTable {
    fn table_name() -> &'static str { "nft_transfer_history_cache_table" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if is_initial_upgrade(old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(
                Self::CHAIN_TOKEN_ADD_TOKEN_ID_INDEX,
                &["chain", "token_address", "token_id"],
                false,
            )?;
            table.create_multi_index(
                Self::CHAIN_TX_HASH_LOG_INDEX_INDEX,
                &["chain", "transaction_hash", "log_index"],
                true,
            )?;
            table.create_multi_index(Self::CHAIN_BLOCK_NUMBER_INDEX, &["chain", "block_number"], false)?;
            table.create_index("block_number", false)?;
            table.create_index("chain", false)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct LastScannedBlockTable {
    chain: String,
    last_scanned_block: BeBigUint,
}

impl TableSignature for LastScannedBlockTable {
    fn table_name() -> &'static str { "last_scanned_block_table" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if is_initial_upgrade(old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_index("chain", true)?;
        }
        Ok(())
    }
}

fn nft_details_from_item(item: NftListTable) -> WasmNftCacheResult<Nft> {
    json::from_value(item.details_json).map_to_mm(|e| WasmNftCacheError::ErrorDeserializing(e.to_string()))
}

fn transfer_details_from_item(item: NftTransferHistoryTable) -> WasmNftCacheResult<NftTransferHistory> {
    json::from_value(item.details_json).map_to_mm(|e| WasmNftCacheError::ErrorDeserializing(e.to_string()))
}
