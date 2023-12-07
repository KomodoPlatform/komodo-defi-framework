use crate::eth::eth_addr_to_hex;
use crate::nft::nft_structs::{Chain, ContractType, Nft, NftList, NftListFilters, NftTransferHistory,
                              NftsTransferHistoryList, TransferMeta, TransferStatus};
use crate::nft::storage::wasm::nft_idb::NftCacheIDBLocked;
use crate::nft::storage::wasm::{WasmNftCacheError, WasmNftCacheResult};
use crate::nft::storage::{get_offset_limit, NftListStorageOps, NftTokenAddrId, NftTransferHistoryFilters,
                          NftTransferHistoryStorageOps, RemoveNftResult};
use async_trait::async_trait;
use common::is_initial_upgrade;
use ethereum_types::Address;
use mm2_db::indexed_db::{BeBigUint, DbTable, DbUpgrader, MultiIndex, OnUpgradeResult, TableSignature};
use mm2_err_handle::map_to_mm::MapToMmResult;
use mm2_err_handle::prelude::MmResult;
use mm2_number::BigUint;
use num_traits::ToPrimitive;
use serde_json::{self as json, Value as Json};
use std::collections::HashSet;
use std::num::NonZeroUsize;

const CHAIN_TOKEN_ADD_TOKEN_ID_INDEX: &str = "chain_token_add_token_id_index";
const CHAIN_BLOCK_NUMBER_INDEX: &str = "chain_block_number_index";
const CHAIN_TOKEN_ADD_INDEX: &str = "chain_token_add_index";
const CHAIN_TOKEN_DOMAIN_INDEX: &str = "chain_token_domain_index";
const CHAIN_IMAGE_DOMAIN_INDEX: &str = "chain_image_domain_index";

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

fn filter_nfts<I>(nfts: I, filters: Option<NftListFilters>) -> WasmNftCacheResult<Vec<Nft>>
where
    I: Iterator<Item = NftListTable>,
{
    let mut filtered_nfts = Vec::new();
    for nft_table in nfts {
        let nft = nft_details_from_item(nft_table)?;
        match filters {
            Some(filters) => {
                if filters.passes_spam_filter(&nft) && filters.passes_phishing_filter(&nft) {
                    filtered_nfts.push(nft);
                }
            },
            None => filtered_nfts.push(nft),
        }
    }
    Ok(filtered_nfts)
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

fn filter_transfers<I>(
    transfers: I,
    filters: Option<NftTransferHistoryFilters>,
) -> WasmNftCacheResult<Vec<NftTransferHistory>>
where
    I: Iterator<Item = NftTransferHistoryTable>,
{
    let mut filtered_transfers = Vec::new();
    for transfers_table in transfers {
        let transfer = transfer_details_from_item(transfers_table)?;
        match filters {
            Some(filters) => {
                if filters.is_status_match(&transfer)
                    && filters.is_date_match(&transfer)
                    && filters.passes_spam_filter(&transfer)
                    && filters.passes_phishing_filter(&transfer)
                {
                    filtered_transfers.push(transfer);
                }
            },
            None => filtered_transfers.push(transfer),
        }
    }
    Ok(filtered_transfers)
}

impl NftListFilters {
    fn passes_spam_filter(&self, nft: &Nft) -> bool { !self.exclude_spam || !nft.common.possible_spam }

    fn passes_phishing_filter(&self, nft: &Nft) -> bool { !self.exclude_phishing || !nft.possible_phishing }
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

    fn passes_spam_filter(&self, transfer: &NftTransferHistory) -> bool {
        !self.exclude_spam || !transfer.common.possible_spam
    }

    fn passes_phishing_filter(&self, transfer: &NftTransferHistory) -> bool {
        !self.exclude_phishing || !transfer.possible_phishing
    }
}

#[async_trait]
impl NftListStorageOps for NftCacheIDBLocked<'_> {
    type Error = WasmNftCacheError;

    async fn init(&self, _chain: &Chain) -> MmResult<(), Self::Error> { Ok(()) }

    async fn is_initialized(&self, _chain: &Chain) -> MmResult<bool, Self::Error> { Ok(true) }

    async fn get_nft_list(
        &self,
        chains: Vec<Chain>,
        max: bool,
        limit: usize,
        page_number: Option<NonZeroUsize>,
        filters: Option<NftListFilters>,
    ) -> MmResult<NftList, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;
        let mut nfts = Vec::new();
        for chain in chains {
            let nft_tables = table
                .get_items("chain", chain.to_string())
                .await?
                .into_iter()
                .map(|(_item_id, nft)| nft);
            let filtered = filter_nfts(nft_tables, filters)?;
            nfts.extend(filtered);
        }
        take_nft_according_to_paging_opts(nfts, max, limit, page_number)
    }

    async fn add_nfts_to_list<I>(&self, chain: Chain, nfts: I, last_scanned_block: u64) -> MmResult<(), Self::Error>
    where
        I: IntoIterator<Item = Nft> + Send + 'static,
        I::IntoIter: Send,
    {
        let db_transaction = self.get_inner().transaction().await?;
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
        token_id: BigUint,
    ) -> MmResult<Option<Nft>, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;
        let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(&token_address)?
            .with_value(BeBigUint::from(token_id))?;

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
        token_id: BigUint,
        scanned_block: u64,
    ) -> MmResult<RemoveNftResult, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let nft_table = db_transaction.table::<NftListTable>().await?;
        let last_scanned_block_table = db_transaction.table::<LastScannedBlockTable>().await?;

        let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(&token_address)?
            .with_value(BeBigUint::from(token_id))?;

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
        token_id: BigUint,
    ) -> MmResult<Option<String>, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;
        let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(&token_address)?
            .with_value(BeBigUint::from(token_id))?;

        if let Some((_item_id, item)) = table.get_item_by_unique_multi_index(index_keys).await? {
            Ok(Some(nft_details_from_item(item)?.common.amount.to_string()))
        } else {
            Ok(None)
        }
    }

    async fn refresh_nft_metadata(&self, chain: &Chain, nft: Nft) -> MmResult<(), Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;
        let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(eth_addr_to_hex(&nft.common.token_address))?
            .with_value(BeBigUint::from(nft.token_id.clone()))?;

        let nft_item = NftListTable::from_nft(&nft)?;
        table.replace_item_by_unique_multi_index(index_keys, &nft_item).await?;
        Ok(())
    }

    async fn get_last_block_number(&self, chain: &Chain) -> MmResult<Option<u64>, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;
        get_last_block_from_table(chain, table, CHAIN_BLOCK_NUMBER_INDEX).await
    }

    async fn get_last_scanned_block(&self, chain: &Chain) -> MmResult<Option<u64>, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
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
        let db_transaction = self.get_inner().transaction().await?;
        let nft_table = db_transaction.table::<NftListTable>().await?;
        let last_scanned_block_table = db_transaction.table::<LastScannedBlockTable>().await?;

        let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(eth_addr_to_hex(&nft.common.token_address))?
            .with_value(BeBigUint::from(nft.token_id.clone()))?;

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
        let db_transaction = self.get_inner().transaction().await?;
        let nft_table = db_transaction.table::<NftListTable>().await?;
        let last_scanned_block_table = db_transaction.table::<LastScannedBlockTable>().await?;

        let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(eth_addr_to_hex(&nft.common.token_address))?
            .with_value(BeBigUint::from(nft.token_id.clone()))?;

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

    async fn get_nfts_by_token_address(&self, chain: Chain, token_address: String) -> MmResult<Vec<Nft>, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;

        let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_INDEX)
            .with_value(chain.to_string())?
            .with_value(&token_address)?;

        table
            .get_items_by_multi_index(index_keys)
            .await?
            .into_iter()
            .map(|(_item_id, item)| nft_details_from_item(item))
            .collect()
    }

    async fn update_nft_spam_by_token_address(
        &self,
        chain: &Chain,
        token_address: String,
        possible_spam: bool,
    ) -> MmResult<(), Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;

        let chain_str = chain.to_string();
        let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_INDEX)
            .with_value(&chain_str)?
            .with_value(&token_address)?;

        let nfts: Result<Vec<Nft>, _> = table
            .get_items_by_multi_index(index_keys)
            .await?
            .into_iter()
            .map(|(_item_id, item)| nft_details_from_item(item))
            .collect();
        let nfts = nfts?;

        for mut nft in nfts {
            nft.common.possible_spam = possible_spam;
            drop_mutability!(nft);

            let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
                .with_value(&chain_str)?
                .with_value(eth_addr_to_hex(&nft.common.token_address))?
                .with_value(BeBigUint::from(nft.token_id.clone()))?;

            let item = NftListTable::from_nft(&nft)?;
            table.replace_item_by_unique_multi_index(index_keys, &item).await?;
        }
        Ok(())
    }

    async fn get_animation_external_domains(&self, chain: &Chain) -> MmResult<HashSet<String>, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;

        let mut domains = HashSet::new();
        let nft_tables = table.get_items("chain", chain.to_string()).await?;
        for (_item_id, nft) in nft_tables.into_iter() {
            if let Some(domain) = nft.animation_domain {
                domains.insert(domain);
            }
            if let Some(domain) = nft.external_domain {
                domains.insert(domain);
            }
        }
        Ok(domains)
    }

    async fn update_nft_phishing_by_domain(
        &self,
        chain: &Chain,
        domain: String,
        possible_phishing: bool,
    ) -> MmResult<(), Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftListTable>().await?;

        let chain_str = chain.to_string();
        update_nft_phishing_for_index(&table, &chain_str, CHAIN_TOKEN_DOMAIN_INDEX, &domain, possible_phishing).await?;
        update_nft_phishing_for_index(&table, &chain_str, CHAIN_IMAGE_DOMAIN_INDEX, &domain, possible_phishing).await?;
        let animation_index = NftListTable::CHAIN_ANIMATION_DOMAIN_INDEX;
        update_nft_phishing_for_index(&table, &chain_str, animation_index, &domain, possible_phishing).await?;
        let external_index = NftListTable::CHAIN_EXTERNAL_DOMAIN_INDEX;
        update_nft_phishing_for_index(&table, &chain_str, external_index, &domain, possible_phishing).await?;
        Ok(())
    }
}

#[async_trait]
impl NftTransferHistoryStorageOps for NftCacheIDBLocked<'_> {
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
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;
        let mut transfers = Vec::new();
        for chain in chains {
            let transfer_tables = table
                .get_items("chain", chain.to_string())
                .await?
                .into_iter()
                .map(|(_item_id, transfer)| transfer);
            let filtered = filter_transfers(transfer_tables, filters)?;
            transfers.extend(filtered);
        }
        take_transfers_according_to_paging_opts(transfers, max, limit, page_number)
    }

    async fn add_transfers_to_history<I>(&self, _chain: Chain, transfers: I) -> MmResult<(), Self::Error>
    where
        I: IntoIterator<Item = NftTransferHistory> + Send + 'static,
        I::IntoIter: Send,
    {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;
        for transfer in transfers {
            let transfer_item = NftTransferHistoryTable::from_transfer_history(&transfer)?;
            table.add_item(&transfer_item).await?;
        }
        Ok(())
    }

    async fn get_last_block_number(&self, chain: &Chain) -> MmResult<Option<u64>, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;
        get_last_block_from_table(chain, table, CHAIN_BLOCK_NUMBER_INDEX).await
    }

    async fn get_transfers_from_block(
        &self,
        chain: Chain,
        from_block: u64,
    ) -> MmResult<Vec<NftTransferHistory>, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;
        let items = table
            .cursor_builder()
            .only("chain", chain.to_string())
            .map_err(|e| WasmNftCacheError::GetLastNftBlockError(e.to_string()))?
            .bound("block_number", BeBigUint::from(from_block), BeBigUint::from(u64::MAX))
            .open_cursor(CHAIN_BLOCK_NUMBER_INDEX)
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
        chain: Chain,
        token_address: String,
        token_id: BigUint,
    ) -> MmResult<Vec<NftTransferHistory>, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;

        let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain.to_string())?
            .with_value(&token_address)?
            .with_value(BeBigUint::from(token_id))?;

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
        let db_transaction = self.get_inner().transaction().await?;
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

    async fn update_transfers_meta_by_token_addr_id(
        &self,
        chain: &Chain,
        transfer_meta: TransferMeta,
        set_spam: bool,
    ) -> MmResult<(), Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;

        let chain_str = chain.to_string();
        let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(&chain_str)?
            .with_value(&transfer_meta.token_address)?
            .with_value(BeBigUint::from(transfer_meta.token_id))?;

        let transfers: Result<Vec<NftTransferHistory>, _> = table
            .get_items_by_multi_index(index_keys)
            .await?
            .into_iter()
            .map(|(_item_id, item)| transfer_details_from_item(item))
            .collect();
        let transfers = transfers?;

        for mut transfer in transfers {
            transfer.token_uri = transfer_meta.token_uri.clone();
            transfer.token_domain = transfer_meta.token_domain.clone();
            transfer.collection_name = transfer_meta.collection_name.clone();
            transfer.image_url = transfer_meta.image_url.clone();
            transfer.image_domain = transfer_meta.image_domain.clone();
            transfer.token_name = transfer_meta.token_name.clone();
            if set_spam {
                transfer.common.possible_spam = true;
            }
            drop_mutability!(transfer);

            let index_keys = MultiIndex::new(NftTransferHistoryTable::CHAIN_TX_HASH_LOG_INDEX_INDEX)
                .with_value(&chain_str)?
                .with_value(&transfer.common.transaction_hash)?
                .with_value(transfer.common.log_index)?;

            let item = NftTransferHistoryTable::from_transfer_history(&transfer)?;
            table.replace_item_by_unique_multi_index(index_keys, &item).await?;
        }
        Ok(())
    }

    async fn get_transfers_with_empty_meta(&self, chain: Chain) -> MmResult<Vec<NftTokenAddrId>, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
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
                && !item.possible_spam
            {
                res.insert(NftTokenAddrId {
                    token_address: item.token_address,
                    token_id: BigUint::from(item.token_id),
                });
            }
        }
        Ok(res.into_iter().collect())
    }

    async fn get_transfers_by_token_address(
        &self,
        chain: Chain,
        token_address: String,
    ) -> MmResult<Vec<NftTransferHistory>, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;

        let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_INDEX)
            .with_value(chain.to_string())?
            .with_value(&token_address)?;

        table
            .get_items_by_multi_index(index_keys)
            .await?
            .into_iter()
            .map(|(_item_id, item)| transfer_details_from_item(item))
            .collect()
    }

    async fn update_transfer_spam_by_token_address(
        &self,
        chain: &Chain,
        token_address: String,
        possible_spam: bool,
    ) -> MmResult<(), Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;

        let chain_str = chain.to_string();
        let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_INDEX)
            .with_value(&chain_str)?
            .with_value(&token_address)?;

        let transfers: Result<Vec<NftTransferHistory>, _> = table
            .get_items_by_multi_index(index_keys)
            .await?
            .into_iter()
            .map(|(_item_id, item)| transfer_details_from_item(item))
            .collect();
        let transfers = transfers?;

        for mut transfer in transfers {
            transfer.common.possible_spam = possible_spam;
            drop_mutability!(transfer);

            let index_keys = MultiIndex::new(NftTransferHistoryTable::CHAIN_TX_HASH_LOG_INDEX_INDEX)
                .with_value(&chain_str)?
                .with_value(&transfer.common.transaction_hash)?
                .with_value(transfer.common.log_index)?;

            let item = NftTransferHistoryTable::from_transfer_history(&transfer)?;
            table.replace_item_by_unique_multi_index(index_keys, &item).await?;
        }
        Ok(())
    }

    async fn get_token_addresses(&self, chain: Chain) -> MmResult<HashSet<Address>, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;

        let items = table.get_items("chain", chain.to_string()).await?;
        let mut token_addresses = HashSet::new();
        for (_item_id, item) in items.into_iter() {
            let transfer = transfer_details_from_item(item)?;
            token_addresses.insert(transfer.common.token_address);
        }
        Ok(token_addresses)
    }

    async fn get_domains(&self, chain: &Chain) -> MmResult<HashSet<String>, Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;

        let mut domains = HashSet::new();
        let transfer_tables = table.get_items("chain", chain.to_string()).await?;
        for (_item_id, transfer) in transfer_tables.into_iter() {
            if let Some(domain) = transfer.token_domain {
                domains.insert(domain);
            }
            if let Some(domain) = transfer.image_domain {
                domains.insert(domain);
            }
        }
        Ok(domains)
    }

    async fn update_transfer_phishing_by_domain(
        &self,
        chain: &Chain,
        domain: String,
        possible_phishing: bool,
    ) -> MmResult<(), Self::Error> {
        let db_transaction = self.get_inner().transaction().await?;
        let table = db_transaction.table::<NftTransferHistoryTable>().await?;
        let chain_str = chain.to_string();
        update_transfer_phishing_for_index(&table, &chain_str, CHAIN_TOKEN_DOMAIN_INDEX, &domain, possible_phishing)
            .await?;
        update_transfer_phishing_for_index(&table, &chain_str, CHAIN_IMAGE_DOMAIN_INDEX, &domain, possible_phishing)
            .await?;
        Ok(())
    }
}

async fn update_transfer_phishing_for_index(
    table: &DbTable<'_, NftTransferHistoryTable>,
    chain: &str,
    index: &str,
    domain: &str,
    possible_phishing: bool,
) -> MmResult<(), WasmNftCacheError> {
    let index_keys = MultiIndex::new(index).with_value(chain)?.with_value(domain)?;
    let transfers_table = table.get_items_by_multi_index(index_keys).await?;
    for (_item_id, item) in transfers_table.into_iter() {
        let mut transfer = transfer_details_from_item(item)?;
        transfer.possible_phishing = possible_phishing;
        drop_mutability!(transfer);
        let transfer_item = NftTransferHistoryTable::from_transfer_history(&transfer)?;
        let index_keys = MultiIndex::new(NftTransferHistoryTable::CHAIN_TX_HASH_LOG_INDEX_INDEX)
            .with_value(chain)?
            .with_value(&transfer.common.transaction_hash)?
            .with_value(transfer.common.log_index)?;
        table
            .replace_item_by_unique_multi_index(index_keys, &transfer_item)
            .await?;
    }
    Ok(())
}

async fn update_nft_phishing_for_index(
    table: &DbTable<'_, NftListTable>,
    chain: &str,
    index: &str,
    domain: &str,
    possible_phishing: bool,
) -> MmResult<(), WasmNftCacheError> {
    let index_keys = MultiIndex::new(index).with_value(chain)?.with_value(domain)?;
    let nfts_table = table.get_items_by_multi_index(index_keys).await?;
    for (_item_id, item) in nfts_table.into_iter() {
        let mut nft = nft_details_from_item(item)?;
        nft.possible_phishing = possible_phishing;
        drop_mutability!(nft);
        let nft_item = NftListTable::from_nft(&nft)?;
        let index_keys = MultiIndex::new(CHAIN_TOKEN_ADD_TOKEN_ID_INDEX)
            .with_value(chain)?
            .with_value(eth_addr_to_hex(&nft.common.token_address))?
            .with_value(BeBigUint::from(nft.token_id))?;
        table.replace_item_by_unique_multi_index(index_keys, &nft_item).await?;
    }
    Ok(())
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
    token_id: BeBigUint,
    chain: String,
    amount: String,
    block_number: BeBigUint,
    contract_type: ContractType,
    possible_spam: bool,
    possible_phishing: bool,
    token_domain: Option<String>,
    image_domain: Option<String>,
    animation_domain: Option<String>,
    external_domain: Option<String>,
    details_json: Json,
}

impl NftListTable {
    const CHAIN_ANIMATION_DOMAIN_INDEX: &str = "chain_animation_domain_index";
    const CHAIN_EXTERNAL_DOMAIN_INDEX: &str = "chain_external_domain_index";

    fn from_nft(nft: &Nft) -> WasmNftCacheResult<NftListTable> {
        let details_json = json::to_value(nft).map_to_mm(|e| WasmNftCacheError::ErrorSerializing(e.to_string()))?;
        Ok(NftListTable {
            token_address: eth_addr_to_hex(&nft.common.token_address),
            token_id: BeBigUint::from(nft.token_id.clone()),
            chain: nft.chain.to_string(),
            amount: nft.common.amount.to_string(),
            block_number: BeBigUint::from(nft.block_number),
            contract_type: nft.contract_type,
            possible_spam: nft.common.possible_spam,
            possible_phishing: nft.possible_phishing,
            token_domain: nft.common.token_domain.clone(),
            image_domain: nft.uri_meta.image_domain.clone(),
            animation_domain: nft.uri_meta.animation_domain.clone(),
            external_domain: nft.uri_meta.external_domain.clone(),
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
                CHAIN_TOKEN_ADD_TOKEN_ID_INDEX,
                &["chain", "token_address", "token_id"],
                true,
            )?;
            table.create_multi_index(CHAIN_BLOCK_NUMBER_INDEX, &["chain", "block_number"], false)?;
            table.create_multi_index(CHAIN_TOKEN_ADD_INDEX, &["chain", "token_address"], false)?;
            table.create_multi_index(CHAIN_TOKEN_DOMAIN_INDEX, &["chain", "token_domain"], false)?;
            table.create_multi_index(CHAIN_IMAGE_DOMAIN_INDEX, &["chain", "image_domain"], false)?;
            table.create_multi_index(
                Self::CHAIN_ANIMATION_DOMAIN_INDEX,
                &["chain", "animation_domain"],
                false,
            )?;
            table.create_multi_index(Self::CHAIN_EXTERNAL_DOMAIN_INDEX, &["chain", "external_domain"], false)?;
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
    token_id: BeBigUint,
    status: TransferStatus,
    amount: String,
    token_uri: Option<String>,
    token_domain: Option<String>,
    collection_name: Option<String>,
    image_url: Option<String>,
    image_domain: Option<String>,
    token_name: Option<String>,
    possible_spam: bool,
    possible_phishing: bool,
    details_json: Json,
}

impl NftTransferHistoryTable {
    const CHAIN_TX_HASH_LOG_INDEX_INDEX: &str = "chain_tx_hash_log_index_index";

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
            token_id: BeBigUint::from(transfer.token_id.clone()),
            status: transfer.status,
            amount: transfer.common.amount.to_string(),
            token_uri: transfer.token_uri.clone(),
            token_domain: transfer.token_domain.clone(),
            collection_name: transfer.collection_name.clone(),
            image_url: transfer.image_url.clone(),
            image_domain: transfer.image_domain.clone(),
            token_name: transfer.token_name.clone(),
            possible_spam: transfer.common.possible_spam,
            possible_phishing: transfer.possible_phishing,
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
                CHAIN_TOKEN_ADD_TOKEN_ID_INDEX,
                &["chain", "token_address", "token_id"],
                false,
            )?;
            table.create_multi_index(
                Self::CHAIN_TX_HASH_LOG_INDEX_INDEX,
                &["chain", "transaction_hash", "log_index"],
                true,
            )?;
            table.create_multi_index(CHAIN_BLOCK_NUMBER_INDEX, &["chain", "block_number"], false)?;
            table.create_multi_index(CHAIN_TOKEN_ADD_INDEX, &["chain", "token_address"], false)?;
            table.create_multi_index(CHAIN_TOKEN_DOMAIN_INDEX, &["chain", "token_domain"], false)?;
            table.create_multi_index(CHAIN_IMAGE_DOMAIN_INDEX, &["chain", "image_domain"], false)?;
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
