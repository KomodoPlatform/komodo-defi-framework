use crate::eth::eth_addr_to_hex;
use crate::nft::nft_structs::{Chain, ContractType, Nft, NftCtx, NftList, NftTransferHistory, NftsTransferHistoryList,
                              TransferStatus, TxMeta};
use crate::nft::storage::wasm::nft_idb::{NftCacheIDB, NftCacheIDBLocked};
use crate::nft::storage::wasm::{WasmNftCacheError, WasmNftCacheResult};
use crate::nft::storage::{get_offset_limit, CreateNftStorageError, NftListStorageOps, NftTokenAddrId,
                          NftTxHistoryFilters, NftTxHistoryStorageOps, RemoveNftResult};
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
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::str::FromStr;

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

    fn take_txs_according_to_paging_opts(
        mut txs: Vec<NftTransferHistory>,
        max: bool,
        limit: usize,
        page_number: Option<NonZeroUsize>,
    ) -> WasmNftCacheResult<NftsTransferHistoryList> {
        let total_count = txs.len();
        txs.sort_by(|a, b| b.block_timestamp.cmp(&a.block_timestamp));
        let (offset, limit) = get_offset_limit(max, limit, page_number, total_count);
        Ok(NftsTransferHistoryList {
            transfer_history: txs.into_iter().skip(offset).take(limit).collect(),
            skipped: offset,
            total: total_count,
        })
    }

    fn take_txs_according_to_filters<I>(
        txs: I,
        filters: Option<NftTxHistoryFilters>,
    ) -> WasmNftCacheResult<Vec<NftTransferHistory>>
    where
        I: Iterator<Item = NftTxHistoryTable>,
    {
        let mut filtered_txs = Vec::new();
        for tx_table in txs {
            let tx = tx_details_from_item(tx_table)?;
            if let Some(filters) = &filters {
                if filters.is_status_match(&tx) && filters.is_date_match(&tx) {
                    filtered_txs.push(tx);
                }
            } else {
                filtered_txs.push(tx);
            }
        }
        Ok(filtered_txs)
    }
}

impl NftTxHistoryFilters {
    fn is_status_match(&self, tx: &NftTransferHistory) -> bool {
        (!self.receive && !self.send)
            || (self.receive && tx.status == TransferStatus::Receive)
            || (self.send && tx.status == TransferStatus::Send)
    }

    fn is_date_match(&self, tx: &NftTransferHistory) -> bool {
        self.from_date.map_or(true, |from| tx.block_timestamp >= from)
            && self.to_date.map_or(true, |to| tx.block_timestamp <= to)
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
impl NftTxHistoryStorageOps for IndexedDbNftStorage {
    type Error = WasmNftCacheError;

    async fn init(&self, _chain: &Chain) -> MmResult<(), Self::Error> { Ok(()) }

    async fn is_initialized(&self, _chain: &Chain) -> MmResult<bool, Self::Error> { Ok(true) }

    async fn get_tx_history(
        &self,
        chains: Vec<Chain>,
        max: bool,
        limit: usize,
        page_number: Option<NonZeroUsize>,
        filters: Option<NftTxHistoryFilters>,
    ) -> MmResult<NftsTransferHistoryList, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTxHistoryTable>().await?;
        let mut txs = Vec::new();
        for chain in chains {
            let tx_tables = table
                .get_items("chain", chain.to_string())
                .await?
                .into_iter()
                .map(|(_item_id, tx)| tx);
            let filtered = Self::take_txs_according_to_filters(tx_tables, filters)?;
            txs.extend(filtered);
        }
        Self::take_txs_according_to_paging_opts(txs, max, limit, page_number)
    }

    async fn add_txs_to_history<I>(&self, _chain: &Chain, txs: I) -> MmResult<(), Self::Error>
    where
        I: IntoIterator<Item = NftTransferHistory> + Send + 'static,
        I::IntoIter: Send,
    {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTxHistoryTable>().await?;
        for tx in txs {
            let tx_item = NftTxHistoryTable::from_tx_history(&tx)?;
            table.add_item(&tx_item).await?;
        }
        Ok(())
    }

    async fn get_last_block_number(&self, chain: &Chain) -> MmResult<Option<u64>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTxHistoryTable>().await?;
        get_last_block_from_table(chain, table, NftTxHistoryTable::CHAIN_BLOCK_NUMBER_INDEX).await
    }

    async fn get_txs_from_block(
        &self,
        chain: &Chain,
        from_block: u64,
    ) -> MmResult<Vec<NftTransferHistory>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTxHistoryTable>().await?;
        let items = table
            .cursor_builder()
            .only("chain", chain.to_string())
            .map_err(|e| WasmNftCacheError::GetLastNftBlockError(e.to_string()))?
            .bound("block_number", BeBigUint::from(from_block), BeBigUint::from(u64::MAX))
            .open_cursor(NftTxHistoryTable::CHAIN_BLOCK_NUMBER_INDEX)
            .await
            .map_err(|e| WasmNftCacheError::GetLastNftBlockError(e.to_string()))?
            .collect()
            .await
            .map_err(|e| WasmNftCacheError::GetLastNftBlockError(e.to_string()))?;

        let mut res = Vec::new();
        for (_item_id, item) in items.into_iter() {
            let tx = tx_details_from_item(item)?;
            res.push(tx);
        }
        Ok(res)
    }

    async fn get_txs_by_token_addr_id(
        &self,
        chain: &Chain,
        token_address: String,
        token_id: BigDecimal,
    ) -> MmResult<Vec<NftTransferHistory>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTxHistoryTable>().await?;

        let index_keys = MultiIndex::new(NftTxHistoryTable::CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(&token_address)?
            .with_value(token_id.to_string())?;

        table
            .get_items_by_multi_index(index_keys)
            .await?
            .into_iter()
            .map(|(_item_id, item)| tx_details_from_item(item))
            .collect()
    }

    async fn get_tx_by_tx_hash(
        &self,
        chain: &Chain,
        transaction_hash: String,
    ) -> MmResult<Option<NftTransferHistory>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTxHistoryTable>().await?;
        let index_keys = MultiIndex::new(NftTxHistoryTable::CHAIN_TX_HASH_INDEX)
            .with_value(chain.to_string())?
            .with_value(&transaction_hash)?;

        if let Some((_item_id, item)) = table.get_item_by_unique_multi_index(index_keys).await? {
            Ok(Some(tx_details_from_item(item)?))
        } else {
            Ok(None)
        }
    }

    async fn update_tx_meta_by_hash(&self, chain: &Chain, tx: NftTransferHistory) -> MmResult<(), Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTxHistoryTable>().await?;

        let index_keys = MultiIndex::new(NftTxHistoryTable::CHAIN_TX_HASH_INDEX)
            .with_value(chain.to_string())?
            .with_value(&tx.common.transaction_hash)?;

        let item = NftTxHistoryTable::from_tx_history(&tx)?;
        table.replace_item_by_unique_multi_index(index_keys, &item).await?;
        Ok(())
    }

    async fn update_txs_meta_by_token_addr_id(&self, chain: &Chain, tx_meta: TxMeta) -> MmResult<(), Self::Error> {
        let txs: Vec<NftTransferHistory> = self
            .get_txs_by_token_addr_id(chain, tx_meta.token_address, tx_meta.token_id)
            .await?;
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTxHistoryTable>().await?;
        for mut tx in txs {
            tx.token_uri = tx_meta.token_uri.clone();
            tx.collection_name = tx_meta.collection_name.clone();
            tx.image_url = tx_meta.image_url.clone();
            tx.token_name = tx_meta.token_name.clone();
            drop_mutability!(tx);

            let index_keys = MultiIndex::new(NftTxHistoryTable::CHAIN_TX_HASH_INDEX)
                .with_value(chain.to_string())?
                .with_value(&tx.common.transaction_hash)?;

            let item = NftTxHistoryTable::from_tx_history(&tx)?;
            table.replace_item_by_unique_multi_index(index_keys, &item).await?;
        }
        Ok(())
    }

    async fn get_txs_with_empty_meta(&self, chain: &Chain) -> MmResult<Vec<NftTokenAddrId>, Self::Error> {
        let locked_db = self.lock_db().await?;
        let db_transaction = locked_db.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTxHistoryTable>().await?;
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

impl BlockNumberTable for NftTxHistoryTable {
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
pub(crate) struct NftTxHistoryTable {
    transaction_hash: String,
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

impl NftTxHistoryTable {
    const CHAIN_TOKEN_ADD_TOKEN_ID_INDEX: &str = "chain_token_add_token_id_index";

    const CHAIN_TX_HASH_INDEX: &str = "chain_tx_hash_index";

    const CHAIN_BLOCK_NUMBER_INDEX: &str = "chain_block_number_index";

    fn from_tx_history(tx: &NftTransferHistory) -> WasmNftCacheResult<NftTxHistoryTable> {
        let details_json = json::to_value(tx).map_to_mm(|e| WasmNftCacheError::ErrorSerializing(e.to_string()))?;
        Ok(NftTxHistoryTable {
            transaction_hash: tx.common.transaction_hash.clone(),
            chain: tx.chain.to_string(),
            block_number: BeBigUint::from(tx.block_number),
            block_timestamp: BeBigUint::from(tx.block_timestamp),
            contract_type: tx.contract_type,
            token_address: eth_addr_to_hex(&tx.common.token_address),
            token_id: tx.common.token_id.to_string(),
            status: tx.status,
            amount: tx.common.amount.to_string(),
            token_uri: tx.token_uri.clone(),
            collection_name: tx.collection_name.clone(),
            image_url: tx.image_url.clone(),
            token_name: tx.token_name.clone(),
            details_json,
        })
    }
}

impl TableSignature for NftTxHistoryTable {
    fn table_name() -> &'static str { "nft_tx_history_cache_table" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if is_initial_upgrade(old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_multi_index(
                Self::CHAIN_TOKEN_ADD_TOKEN_ID_INDEX,
                &["chain", "token_address", "token_id"],
                false,
            )?;
            table.create_multi_index(Self::CHAIN_TX_HASH_INDEX, &["chain", "transaction_hash"], true)?;
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

fn tx_details_from_item(item: NftTxHistoryTable) -> WasmNftCacheResult<NftTransferHistory> {
    json::from_value(item.details_json).map_to_mm(|e| WasmNftCacheError::ErrorDeserializing(e.to_string()))
}
