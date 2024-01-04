use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::{MmError, MmResult};
use url::Url;

pub(crate) mod nft_errors;
pub(crate) mod nft_structs;
pub(crate) mod storage;

#[cfg(any(test, target_arch = "wasm32"))] mod nft_tests;

use crate::{coin_conf, get_my_address, lp_coinfind_or_err, MarketCoinOps, MmCoinEnum, MyAddressReq, WithdrawError};
use nft_errors::{GetNftInfoError, UpdateNftError};
use nft_structs::{Chain, ContractType, ConvertChain, Nft, NftFromMoralis, NftList, NftListReq, NftMetadataReq,
                  NftTransferHistory, NftTransferHistoryFromMoralis, NftTransfersReq, NftsTransferHistoryList,
                  TransactionNftDetails, UpdateNftReq, WithdrawNftReq};

use crate::eth::{eth_addr_to_hex, get_eth_address, withdraw_erc1155, withdraw_erc721, EthCoin, EthCoinType,
                 EthTxFeeDetails};
use crate::nft::nft_errors::{MetaFromUrlError, ProtectFromSpamError, TransferConfirmationsError,
                             UpdateSpamPhishingError};
use crate::nft::nft_structs::{build_nft_with_empty_meta, BuildNftFields, NftCommon, NftCtx, NftTransferCommon,
                              PhishingDomainReq, PhishingDomainRes, RefreshMetadataReq, SpamContractReq,
                              SpamContractRes, TransferMeta, TransferStatus, UriMeta};
use crate::nft::storage::{NftListStorageOps, NftTransferHistoryStorageOps};
use common::parse_rfc3339_to_timestamp;
use crypto::StandardHDCoinAddress;
use ethereum_types::{Address, H256};
use futures::compat::Future01CompatExt;
use futures::future::try_join_all;
use mm2_err_handle::map_to_mm::MapToMmResult;
use mm2_net::transport::send_post_request_to_uri;
use mm2_number::{BigDecimal, BigUint};
use regex::Regex;
use serde_json::Value as Json;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use web3::types::TransactionId;

#[cfg(not(target_arch = "wasm32"))]
use mm2_net::native_http::send_request_to_uri;

#[cfg(target_arch = "wasm32")]
use mm2_net::wasm_http::send_request_to_uri;

const MORALIS_API_ENDPOINT: &str = "api/v2";
/// query parameters for moralis request: The format of the token ID
const MORALIS_FORMAT_QUERY_NAME: &str = "format";
const MORALIS_FORMAT_QUERY_VALUE: &str = "decimal";
/// The minimum block number from which to get the transfers
const MORALIS_FROM_BLOCK_QUERY_NAME: &str = "from_block";

const BLOCKLIST_ENDPOINT: &str = "api/blocklist";
const BLOCKLIST_CONTRACT: &str = "contract";
const BLOCKLIST_DOMAIN: &str = "domain";
const BLOCKLIST_SCAN: &str = "scan";

/// `WithdrawNftResult` type represents the result of an NFT withdrawal operation. On success, it provides the details
/// of the generated transaction meant for transferring the NFT. On failure, it details the encountered error.
pub type WithdrawNftResult = Result<TransactionNftDetails, MmError<WithdrawError>>;

/// Fetches a list of user-owned NFTs across specified chains.
///
/// The function aggregates NFTs based on provided chains, supports pagination, and
/// allows for result limits and filters. If the `protect_from_spam` flag is true,
/// NFTs are checked and redacted for potential spam.
///
/// # Parameters
///
/// * `ctx`: Shared context with configurations/resources.
/// * `req`: Request specifying chains, pagination, and filters.
///
/// # Returns
///
/// On success, returns a detailed `NftList` containing NFTs, total count, and skipped count.
/// # Errors
///
/// Returns `GetNftInfoError` variants for issues like invalid requests, transport failures,
/// database errors, and spam protection errors.
pub async fn get_nft_list(ctx: MmArc, req: NftListReq) -> MmResult<NftList, GetNftInfoError> {
    let nft_ctx = NftCtx::from_ctx(&ctx).map_to_mm(GetNftInfoError::Internal)?;

    let storage = nft_ctx.lock_db().await?;
    for chain in req.chains.iter() {
        if !NftListStorageOps::is_initialized(&storage, chain).await? {
            NftListStorageOps::init(&storage, chain).await?;
        }
    }
    let mut nft_list = storage
        .get_nft_list(req.chains, req.max, req.limit, req.page_number, req.filters)
        .await?;
    if req.protect_from_spam {
        for nft in &mut nft_list.nfts {
            protect_from_nft_spam_links(nft, true)?;
        }
    }
    drop_mutability!(nft_list);
    Ok(nft_list)
}

/// Retrieves detailed metadata for a specified NFT.
///
/// The function accesses the stored NFT data, based on provided token address,
/// token ID, and chain, and returns comprehensive information about the NFT.
/// It also checks and redacts potential spam if `protect_from_spam` in the request is set to true.
///
/// # Arguments
///
/// * `ctx`: Context required for handling internal operations.
/// * `req`: A request containing details about the NFT to fetch.
///
/// # Returns
///
/// On success, returns the whole info about desired Nft.
/// # Errors
///
/// Returns `GetNftInfoError` variants for issues like invalid requests, transport failures,
/// database errors, and spam protection errors.
pub async fn get_nft_metadata(ctx: MmArc, req: NftMetadataReq) -> MmResult<Nft, GetNftInfoError> {
    let nft_ctx = NftCtx::from_ctx(&ctx).map_to_mm(GetNftInfoError::Internal)?;

    let storage = nft_ctx.lock_db().await?;
    if !NftListStorageOps::is_initialized(&storage, &req.chain).await? {
        NftListStorageOps::init(&storage, &req.chain).await?;
    }
    let mut nft = storage
        .get_nft(&req.chain, format!("{:#02x}", req.token_address), req.token_id.clone())
        .await?
        .ok_or_else(|| GetNftInfoError::TokenNotFoundInWallet {
            token_address: format!("{:#02x}", req.token_address),
            token_id: req.token_id.to_string(),
        })?;
    if req.protect_from_spam {
        protect_from_nft_spam_links(&mut nft, true)?;
    }
    drop_mutability!(nft);
    Ok(nft)
}

/// Fetches the transfer history of user-owned NFTs across specified chains.
///
/// The function aggregates NFT transfers based on provided chains, offers pagination,
/// allows for result limits, and filters. If the `protect_from_spam` flag is true,
/// the returned transfers are checked and redacted for potential spam.
///
/// # Parameters
///
/// * `ctx`: Shared context with configurations/resources.
/// * `req`: Request detailing chains, pagination, and filters for the transfer history.
///
/// # Returns
///
/// On success, returns an `NftsTransferHistoryList` containing NFT transfer details,
/// the total count, and skipped count.
///
/// # Errors
///
/// Returns `GetNftInfoError` variants for issues like invalid requests, transport failures,
/// database errors, and spam protection errors.
pub async fn get_nft_transfers(ctx: MmArc, req: NftTransfersReq) -> MmResult<NftsTransferHistoryList, GetNftInfoError> {
    let nft_ctx = NftCtx::from_ctx(&ctx).map_to_mm(GetNftInfoError::Internal)?;

    let storage = nft_ctx.lock_db().await?;
    for chain in req.chains.iter() {
        if !NftTransferHistoryStorageOps::is_initialized(&storage, chain).await? {
            NftTransferHistoryStorageOps::init(&storage, chain).await?;
        }
    }
    let mut transfer_history_list = storage
        .get_transfer_history(req.chains.clone(), req.max, req.limit, req.page_number, req.filters)
        .await?;
    if req.protect_from_spam {
        for transfer in &mut transfer_history_list.transfer_history {
            protect_from_history_spam_links(transfer, true)?;
        }
    }
    process_transfers_confirmations(&ctx, req.chains, &mut transfer_history_list).await?;
    drop_mutability!(transfer_history_list);
    Ok(transfer_history_list)
}

async fn process_transfers_confirmations(
    ctx: &MmArc,
    chains: Vec<Chain>,
    history_list: &mut NftsTransferHistoryList,
) -> MmResult<(), TransferConfirmationsError> {
    async fn current_block_impl<Coin: MarketCoinOps>(coin: Coin) -> MmResult<u64, TransferConfirmationsError> {
        coin.current_block()
            .compat()
            .await
            .map_to_mm(TransferConfirmationsError::GetCurrentBlockErr)
    }

    let futures = chains.into_iter().map(|chain| async move {
        let ticker = chain.to_ticker();
        let coin_enum = lp_coinfind_or_err(ctx, ticker).await?;
        match coin_enum {
            MmCoinEnum::EthCoin(eth_coin) => {
                let current_block = current_block_impl(eth_coin).await?;
                Ok((ticker, current_block))
            },
            _ => MmError::err(TransferConfirmationsError::CoinDoesntSupportNft {
                coin: coin_enum.ticker().to_owned(),
            }),
        }
    });
    let blocks_map = try_join_all(futures).await?.into_iter().collect::<HashMap<_, _>>();

    for transfer in history_list.transfer_history.iter_mut() {
        let current_block = match blocks_map.get(transfer.chain.to_ticker()) {
            Some(block) => *block,
            None => 0,
        };
        transfer.confirmations = if transfer.block_number > current_block {
            0
        } else {
            current_block + 1 - transfer.block_number
        };
    }
    Ok(())
}

/// Updates NFT transfer history and NFT list in the DB.
///
/// This function refreshes the NFT transfer history and NFT list cache based on new
/// data fetched from the provided `url`. The function ensures the local cache is in
/// sync with the latest data from the source, validates against spam contract addresses and phishing domains.
///
/// # Arguments
///
/// * `ctx`: Context required for handling internal operations.
/// * `req`: A request containing details about the NFTs to be updated and the source URL.
///
/// # Returns
///
/// * `MmResult<(), UpdateNftError>`: A result indicating success or an error.
pub async fn update_nft(ctx: MmArc, req: UpdateNftReq) -> MmResult<(), UpdateNftError> {
    let nft_ctx = NftCtx::from_ctx(&ctx).map_to_mm(GetNftInfoError::Internal)?;

    let storage = nft_ctx.lock_db().await?;
    for chain in req.chains.iter() {
        let transfer_history_initialized = NftTransferHistoryStorageOps::is_initialized(&storage, chain).await?;

        let from_block = if transfer_history_initialized {
            let last_transfer_block = NftTransferHistoryStorageOps::get_last_block_number(&storage, chain).await?;
            last_transfer_block.map(|b| b + 1)
        } else {
            NftTransferHistoryStorageOps::init(&storage, chain).await?;
            None
        };
        let coin_enum = lp_coinfind_or_err(&ctx, chain.to_ticker()).await?;
        let eth_coin = match coin_enum {
            MmCoinEnum::EthCoin(eth_coin) => eth_coin,
            _ => {
                return MmError::err(UpdateNftError::CoinDoesntSupportNft {
                    coin: coin_enum.ticker().to_owned(),
                })
            },
        };
        let nft_transfers = get_moralis_nft_transfers(&ctx, chain, from_block, &req.url, eth_coin).await?;
        storage.add_transfers_to_history(*chain, nft_transfers).await?;

        let nft_block = match NftListStorageOps::get_last_block_number(&storage, chain).await {
            Ok(Some(block)) => block,
            Ok(None) => {
                // if there are no rows in NFT LIST table we can try to get nft list from moralis.
                let nft_list = cache_nfts_from_moralis(&ctx, &storage, chain, &req.url, &req.url_antispam).await?;
                update_meta_in_transfers(&storage, chain, nft_list).await?;
                update_transfers_with_empty_meta(&storage, chain, &req.url, &req.url_antispam).await?;
                update_spam(&storage, *chain, &req.url_antispam).await?;
                update_phishing(&storage, chain, &req.url_antispam).await?;
                continue;
            },
            Err(_) => {
                // if there is an error, then NFT LIST table doesnt exist, so we need to cache nft list from moralis.
                NftListStorageOps::init(&storage, chain).await?;
                let nft_list = cache_nfts_from_moralis(&ctx, &storage, chain, &req.url, &req.url_antispam).await?;
                update_meta_in_transfers(&storage, chain, nft_list).await?;
                update_transfers_with_empty_meta(&storage, chain, &req.url, &req.url_antispam).await?;
                update_spam(&storage, *chain, &req.url_antispam).await?;
                update_phishing(&storage, chain, &req.url_antispam).await?;
                continue;
            },
        };
        let scanned_block =
            storage
                .get_last_scanned_block(chain)
                .await?
                .ok_or_else(|| UpdateNftError::LastScannedBlockNotFound {
                    last_nft_block: nft_block.to_string(),
                })?;
        // if both block numbers exist, last scanned block should be equal
        // or higher than last block number from NFT LIST table.
        if scanned_block < nft_block {
            return MmError::err(UpdateNftError::InvalidBlockOrder {
                last_scanned_block: scanned_block.to_string(),
                last_nft_block: nft_block.to_string(),
            });
        }
        update_nft_list(
            ctx.clone(),
            &storage,
            chain,
            scanned_block + 1,
            &req.url,
            &req.url_antispam,
        )
        .await?;
        update_transfers_with_empty_meta(&storage, chain, &req.url, &req.url_antispam).await?;
        update_spam(&storage, *chain, &req.url_antispam).await?;
        update_phishing(&storage, chain, &req.url_antispam).await?;
    }
    Ok(())
}

/// `update_spam` function updates spam contracts info in NFT list and NFT transfers.
async fn update_spam<T>(storage: &T, chain: Chain, url_antispam: &Url) -> MmResult<(), UpdateSpamPhishingError>
where
    T: NftListStorageOps + NftTransferHistoryStorageOps,
{
    let token_addresses = storage.get_token_addresses(chain).await?;
    if !token_addresses.is_empty() {
        let addresses = token_addresses
            .iter()
            .map(eth_addr_to_hex)
            .collect::<Vec<_>>()
            .join(",");
        let spam_res = send_spam_request(&chain, url_antispam, addresses).await?;
        for (address, is_spam) in spam_res.result.into_iter() {
            if is_spam {
                let address_hex = eth_addr_to_hex(&address);
                storage
                    .update_nft_spam_by_token_address(&chain, address_hex.clone(), is_spam)
                    .await?;
                storage
                    .update_transfer_spam_by_token_address(&chain, address_hex, is_spam)
                    .await?;
            }
        }
    }
    Ok(())
}

async fn update_phishing<T>(storage: &T, chain: &Chain, url_antispam: &Url) -> MmResult<(), UpdateSpamPhishingError>
where
    T: NftListStorageOps + NftTransferHistoryStorageOps,
{
    let transfer_domains = storage.get_domains(chain).await?;
    let nft_domains = storage.get_animation_external_domains(chain).await?;
    let domains: HashSet<String> = transfer_domains.union(&nft_domains).cloned().collect();
    if !domains.is_empty() {
        let domains = domains.into_iter().collect::<Vec<_>>().join(",");
        let domain_res = send_phishing_request(url_antispam, domains).await?;
        for (domain, is_phishing) in domain_res.result.into_iter() {
            if is_phishing {
                storage
                    .update_nft_phishing_by_domain(chain, domain.clone(), is_phishing)
                    .await?;
                storage
                    .update_transfer_phishing_by_domain(chain, domain, is_phishing)
                    .await?;
            }
        }
    }
    Ok(())
}

/// `send_spam_request` function sends request to antispam api to scan contract addresses for spam.
async fn send_spam_request(
    chain: &Chain,
    url_antispam: &Url,
    addresses: String,
) -> MmResult<SpamContractRes, UpdateSpamPhishingError> {
    let scan_contract_uri = prepare_uri_for_blocklist_endpoint(url_antispam, BLOCKLIST_CONTRACT, BLOCKLIST_SCAN)?;
    let req_spam = SpamContractReq {
        network: *chain,
        addresses,
    };
    let req_spam_json = serde_json::to_string(&req_spam)?;
    let scan_contract_res = send_post_request_to_uri(scan_contract_uri.as_str(), req_spam_json).await?;
    let spam_res: SpamContractRes = serde_json::from_slice(&scan_contract_res)?;
    Ok(spam_res)
}

/// `send_spam_request` function sends request to antispam api to scan domains for phishing.
async fn send_phishing_request(
    url_antispam: &Url,
    domains: String,
) -> MmResult<PhishingDomainRes, UpdateSpamPhishingError> {
    let scan_contract_uri = prepare_uri_for_blocklist_endpoint(url_antispam, BLOCKLIST_DOMAIN, BLOCKLIST_SCAN)?;
    let req_phishing = PhishingDomainReq { domains };
    let req_phishing_json = serde_json::to_string(&req_phishing)?;
    let scan_domains_res = send_post_request_to_uri(scan_contract_uri.as_str(), req_phishing_json).await?;
    let phishing_res: PhishingDomainRes = serde_json::from_slice(&scan_domains_res)?;
    Ok(phishing_res)
}

/// `prepare_uri_for_blocklist_endpoint` function constructs the URI required for the antispam API request.
/// It appends the required path segments to the given base URL and returns the completed URI.
fn prepare_uri_for_blocklist_endpoint(
    url_antispam: &Url,
    blocklist_type: &str,
    blocklist_action_or_network: &str,
) -> MmResult<Url, UpdateSpamPhishingError> {
    let mut uri = url_antispam.clone();
    uri.set_path(BLOCKLIST_ENDPOINT);
    uri.path_segments_mut()
        .map_to_mm(|_| UpdateSpamPhishingError::Internal("Invalid URI".to_string()))?
        .push(blocklist_type)
        .push(blocklist_action_or_network);
    Ok(uri)
}

/// Refreshes and updates metadata associated with a specific NFT.
///
/// The function obtains updated metadata for an NFT using its token address and token id.
/// It fetches the metadata from the provided `url` and validates it against possible spam and
/// phishing domains using the provided `url_antispam`. If the fetched metadata or its domain
/// is identified as spam or matches with any phishing domains, the NFT's `possible_spam` and/or
/// `possible_phishing` flags are set to true.
///
/// # Arguments
///
/// * `ctx`: Context required for handling internal operations.
/// * `req`: A request containing details about the NFT whose metadata needs to be refreshed.
///
/// # Returns
///
/// * `MmResult<(), UpdateNftError>`: A result indicating success or an error.
pub async fn refresh_nft_metadata(ctx: MmArc, req: RefreshMetadataReq) -> MmResult<(), UpdateNftError> {
    let nft_ctx = NftCtx::from_ctx(&ctx).map_to_mm(GetNftInfoError::Internal)?;

    let storage = nft_ctx.lock_db().await?;
    let token_address_str = eth_addr_to_hex(&req.token_address);
    let moralis_meta = match get_moralis_metadata(
        token_address_str.clone(),
        req.token_id.clone(),
        &req.chain,
        &req.url,
        &req.url_antispam,
    )
    .await
    {
        Ok(moralis_meta) => moralis_meta,
        Err(_) => {
            storage
                .update_nft_spam_by_token_address(&req.chain, token_address_str.clone(), true)
                .await?;
            storage
                .update_transfer_spam_by_token_address(&req.chain, token_address_str.clone(), true)
                .await?;
            return Ok(());
        },
    };
    let mut nft_db = storage
        .get_nft(&req.chain, token_address_str.clone(), req.token_id.clone())
        .await?
        .ok_or_else(|| GetNftInfoError::TokenNotFoundInWallet {
            token_address: token_address_str,
            token_id: req.token_id.to_string(),
        })?;
    let token_uri = check_moralis_ipfs_bafy(moralis_meta.common.token_uri.as_deref());
    let token_domain = get_domain_from_url(token_uri.as_deref());
    let uri_meta = get_uri_meta(
        token_uri.as_deref(),
        moralis_meta.common.metadata.as_deref(),
        &req.url_antispam,
    )
    .await;
    // Gather domains for phishing checks
    let domains = gather_domains(&token_domain, &uri_meta);
    nft_db.common.collection_name = moralis_meta.common.collection_name;
    nft_db.common.symbol = moralis_meta.common.symbol;
    nft_db.common.token_uri = token_uri;
    nft_db.common.token_domain = token_domain;
    nft_db.common.metadata = moralis_meta.common.metadata;
    nft_db.common.last_token_uri_sync = moralis_meta.common.last_token_uri_sync;
    nft_db.common.last_metadata_sync = moralis_meta.common.last_metadata_sync;
    nft_db.common.possible_spam = moralis_meta.common.possible_spam;
    nft_db.uri_meta = uri_meta;
    if !nft_db.common.possible_spam {
        refresh_possible_spam(&storage, &req.chain, &mut nft_db, &req.url_antispam).await?;
    };
    if !nft_db.possible_phishing {
        refresh_possible_phishing(&storage, &req.chain, domains, &mut nft_db, &req.url_antispam).await?;
    };
    storage
        .refresh_nft_metadata(&moralis_meta.chain, nft_db.clone())
        .await?;
    update_transfer_meta_using_nft(&storage, &req.chain, &mut nft_db).await?;
    Ok(())
}

/// The `update_transfer_meta_using_nft` function updates the transfer metadata associated with the given NFT.
/// If metadata info contains potential spam links, function sets `possible_spam` true.
async fn update_transfer_meta_using_nft<T>(storage: &T, chain: &Chain, nft: &mut Nft) -> MmResult<(), UpdateNftError>
where
    T: NftListStorageOps + NftTransferHistoryStorageOps,
{
    let transfer_meta = TransferMeta::from(nft.clone());
    storage
        .update_transfers_meta_by_token_addr_id(chain, transfer_meta, nft.common.possible_spam)
        .await?;
    Ok(())
}

/// Extracts domains from uri_meta and token_domain.
fn gather_domains(token_domain: &Option<String>, uri_meta: &UriMeta) -> HashSet<String> {
    let mut domains = HashSet::new();
    if let Some(domain) = token_domain {
        domains.insert(domain.clone());
    }
    if let Some(domain) = &uri_meta.image_domain {
        domains.insert(domain.clone());
    }
    if let Some(domain) = &uri_meta.animation_domain {
        domains.insert(domain.clone());
    }
    if let Some(domain) = &uri_meta.external_domain {
        domains.insert(domain.clone());
    }
    domains
}

/// Refreshes the `possible_spam` flag based on spam results.
async fn refresh_possible_spam<T>(
    storage: &T,
    chain: &Chain,
    nft_db: &mut Nft,
    url_antispam: &Url,
) -> MmResult<(), UpdateNftError>
where
    T: NftListStorageOps + NftTransferHistoryStorageOps,
{
    let address_hex = eth_addr_to_hex(&nft_db.common.token_address);
    let spam_res = send_spam_request(chain, url_antispam, address_hex.clone()).await?;
    if let Some(true) = spam_res.result.get(&nft_db.common.token_address) {
        nft_db.common.possible_spam = true;
        storage
            .update_nft_spam_by_token_address(chain, address_hex.clone(), true)
            .await?;
        storage
            .update_transfer_spam_by_token_address(chain, address_hex, true)
            .await?;
    }
    Ok(())
}

/// Refreshes the `possible_phishing` flag based on phishing results.
async fn refresh_possible_phishing<T>(
    storage: &T,
    chain: &Chain,
    domains: HashSet<String>,
    nft_db: &mut Nft,
    url_antispam: &Url,
) -> MmResult<(), UpdateNftError>
where
    T: NftListStorageOps + NftTransferHistoryStorageOps,
{
    if !domains.is_empty() {
        let domain_list = domains.into_iter().collect::<Vec<_>>().join(",");
        let domain_res = send_phishing_request(url_antispam, domain_list).await?;
        for (domain, is_phishing) in domain_res.result.into_iter() {
            if is_phishing {
                nft_db.possible_phishing = true;
                storage
                    .update_transfer_phishing_by_domain(chain, domain.clone(), is_phishing)
                    .await?;
                storage
                    .update_nft_phishing_by_domain(chain, domain, is_phishing)
                    .await?;
            }
        }
    }
    Ok(())
}

async fn get_moralis_nft_list(
    ctx: &MmArc,
    chain: &Chain,
    url: &Url,
    url_antispam: &Url,
) -> MmResult<Vec<Nft>, GetNftInfoError> {
    let mut res_list = Vec::new();
    let ticker = chain.to_ticker();
    let conf = coin_conf(ctx, ticker);
    let my_address = get_eth_address(ctx, &conf, ticker, &StandardHDCoinAddress::default()).await?;

    let mut uri_without_cursor = url.clone();
    uri_without_cursor.set_path(MORALIS_API_ENDPOINT);
    uri_without_cursor
        .path_segments_mut()
        .map_to_mm(|_| GetNftInfoError::Internal("Invalid URI".to_string()))?
        .push(&my_address.wallet_address)
        .push("nft");
    uri_without_cursor
        .query_pairs_mut()
        .append_pair("chain", &chain.to_string())
        .append_pair(MORALIS_FORMAT_QUERY_NAME, MORALIS_FORMAT_QUERY_VALUE);
    drop_mutability!(uri_without_cursor);

    // The cursor returned in the previous response (used for getting the next page).
    let mut cursor = String::new();
    loop {
        let uri = format!("{}{}", uri_without_cursor, cursor);
        let response = send_request_to_uri(uri.as_str()).await?;
        if let Some(nfts_list) = response["result"].as_array() {
            for nft_json in nfts_list {
                let nft_moralis: NftFromMoralis = serde_json::from_str(&nft_json.to_string())?;
                let contract_type = match nft_moralis.contract_type {
                    Some(contract_type) => contract_type,
                    None => continue,
                };
                let mut nft = build_nft_from_moralis(*chain, nft_moralis, contract_type, url_antispam).await;
                protect_from_nft_spam_links(&mut nft, false)?;
                // collect NFTs from the page
                res_list.push(nft);
            }
            // if cursor is not null, there are other NFTs on next page,
            // and we need to send new request with cursor to get info from the next page.
            if let Some(cursor_res) = response["cursor"].as_str() {
                cursor = format!("{}{}", "&cursor=", cursor_res);
                continue;
            } else {
                break;
            }
        }
    }
    drop_mutability!(res_list);
    Ok(res_list)
}

async fn get_moralis_nft_transfers(
    ctx: &MmArc,
    chain: &Chain,
    from_block: Option<u64>,
    url: &Url,
    eth_coin: EthCoin,
) -> MmResult<Vec<NftTransferHistory>, GetNftInfoError> {
    let mut res_list = Vec::new();
    let ticker = chain.to_ticker();
    let conf = coin_conf(ctx, ticker);
    let my_address = get_eth_address(ctx, &conf, ticker, &StandardHDCoinAddress::default()).await?;

    let mut uri_without_cursor = url.clone();
    uri_without_cursor.set_path(MORALIS_API_ENDPOINT);
    uri_without_cursor
        .path_segments_mut()
        .map_to_mm(|_| GetNftInfoError::Internal("Invalid URI".to_string()))?
        .push(&my_address.wallet_address)
        .push("nft")
        .push("transfers");
    let from_block = match from_block {
        Some(block) => block.to_string(),
        None => "1".into(),
    };
    uri_without_cursor
        .query_pairs_mut()
        .append_pair("chain", &chain.to_string())
        .append_pair(MORALIS_FORMAT_QUERY_NAME, MORALIS_FORMAT_QUERY_VALUE)
        .append_pair(MORALIS_FROM_BLOCK_QUERY_NAME, &from_block);
    drop_mutability!(uri_without_cursor);

    // The cursor returned in the previous response (used for getting the next page).
    let mut cursor = String::new();
    let wallet_address = my_address.wallet_address;
    loop {
        let uri = format!("{}{}", uri_without_cursor, cursor);
        let response = send_request_to_uri(uri.as_str()).await?;
        if let Some(transfer_list) = response["result"].as_array() {
            for transfer in transfer_list {
                let transfer_moralis: NftTransferHistoryFromMoralis = serde_json::from_str(&transfer.to_string())?;
                let contract_type = match transfer_moralis.contract_type {
                    Some(contract_type) => contract_type,
                    None => continue,
                };
                let status =
                    get_transfer_status(&wallet_address, &eth_addr_to_hex(&transfer_moralis.common.to_address));
                let block_timestamp = parse_rfc3339_to_timestamp(&transfer_moralis.block_timestamp)?;
                let fee_details = get_fee_details(&eth_coin, &transfer_moralis.common.transaction_hash).await;
                let transfer_history = NftTransferHistory {
                    common: NftTransferCommon {
                        block_hash: transfer_moralis.common.block_hash,
                        transaction_hash: transfer_moralis.common.transaction_hash,
                        transaction_index: transfer_moralis.common.transaction_index,
                        log_index: transfer_moralis.common.log_index,
                        value: transfer_moralis.common.value,
                        transaction_type: transfer_moralis.common.transaction_type,
                        token_address: transfer_moralis.common.token_address,
                        from_address: transfer_moralis.common.from_address,
                        to_address: transfer_moralis.common.to_address,
                        amount: transfer_moralis.common.amount,
                        verified: transfer_moralis.common.verified,
                        operator: transfer_moralis.common.operator,
                        possible_spam: transfer_moralis.common.possible_spam,
                    },
                    chain: *chain,
                    token_id: transfer_moralis.token_id.0,
                    block_number: *transfer_moralis.block_number,
                    block_timestamp,
                    contract_type,
                    token_uri: None,
                    token_domain: None,
                    collection_name: None,
                    image_url: None,
                    image_domain: None,
                    token_name: None,
                    status,
                    possible_phishing: false,
                    fee_details,
                    confirmations: 0,
                };
                // collect NFTs transfers from the page
                res_list.push(transfer_history);
            }
            // if the cursor is not null, there are other NFTs transfers on next page,
            // and we need to send new request with cursor to get info from the next page.
            if let Some(cursor_res) = response["cursor"].as_str() {
                cursor = format!("{}{}", "&cursor=", cursor_res);
                continue;
            } else {
                break;
            }
        }
    }
    drop_mutability!(res_list);
    Ok(res_list)
}

async fn get_fee_details(eth_coin: &EthCoin, transaction_hash: &str) -> Option<EthTxFeeDetails> {
    let hash = H256::from_str(transaction_hash).ok()?;
    let receipt = eth_coin.web3.eth().transaction_receipt(hash).await.ok()?;
    let fee_coin = match eth_coin.coin_type {
        EthCoinType::Eth => eth_coin.ticker(),
        EthCoinType::Erc20 { .. } => return None,
    };

    match receipt {
        Some(r) => {
            let gas_used = r.gas_used.unwrap_or_default();
            match r.effective_gas_price {
                Some(gas_price) => EthTxFeeDetails::new(gas_used, gas_price, fee_coin).ok(),
                None => {
                    let web3_tx = eth_coin
                        .web3
                        .eth()
                        .transaction(TransactionId::Hash(hash))
                        .await
                        .ok()??;
                    let gas_price = web3_tx.gas_price.unwrap_or_default();
                    EthTxFeeDetails::new(gas_used, gas_price, fee_coin).ok()
                },
            }
        },
        None => None,
    }
}

/// Implements request to the Moralis "Get NFT metadata" endpoint.
///
/// [Moralis Documentation Link](https://docs.moralis.io/web3-data-api/evm/reference/get-nft-metadata)
///
/// **Caution:**
///
/// ERC-1155 token can have a total supply more than 1, which means there could be several owners
/// of the same token. `get_nft_metadata` returns NFTs info with the most recent owner.
/// **Dont** use this function to get specific info about owner address, amount etc, you will get info not related to my_address.
async fn get_moralis_metadata(
    token_address: String,
    token_id: BigUint,
    chain: &Chain,
    url: &Url,
    url_antispam: &Url,
) -> MmResult<Nft, GetNftInfoError> {
    let mut uri = url.clone();
    uri.set_path(MORALIS_API_ENDPOINT);
    uri.path_segments_mut()
        .map_to_mm(|_| GetNftInfoError::Internal("Invalid URI".to_string()))?
        .push("nft")
        .push(&token_address)
        .push(&token_id.to_string());
    uri.query_pairs_mut()
        .append_pair("chain", &chain.to_string())
        .append_pair(MORALIS_FORMAT_QUERY_NAME, MORALIS_FORMAT_QUERY_VALUE);
    drop_mutability!(uri);

    let response = send_request_to_uri(uri.as_str()).await?;
    let nft_moralis: NftFromMoralis = serde_json::from_str(&response.to_string())?;
    let contract_type = match nft_moralis.contract_type {
        Some(contract_type) => contract_type,
        None => return MmError::err(GetNftInfoError::ContractTypeIsNull),
    };
    let mut nft_metadata = build_nft_from_moralis(*chain, nft_moralis, contract_type, url_antispam).await;
    protect_from_nft_spam_links(&mut nft_metadata, false)?;
    Ok(nft_metadata)
}

/// `withdraw_nft` function generates, signs and returns a transaction that transfers NFT
/// from my address to recipient's address.
/// This method generates a raw transaction which should then be broadcast using `send_raw_transaction`.
pub async fn withdraw_nft(ctx: MmArc, req: WithdrawNftReq) -> WithdrawNftResult {
    match req {
        WithdrawNftReq::WithdrawErc1155(erc1155_withdraw) => withdraw_erc1155(ctx, erc1155_withdraw).await,
        WithdrawNftReq::WithdrawErc721(erc721_withdraw) => withdraw_erc721(ctx, erc721_withdraw).await,
    }
}

/// `check_moralis_ipfs_bafy` inspects a given token URI and modifies it if certain conditions are met.
///
/// It checks if the URI points to the Moralis IPFS domain `"ipfs.moralis.io"` and starts with a specific path prefix `"/ipfs/bafy"`.
/// If these conditions are satisfied, it modifies the URI to point to the `"ipfs.io"` domain.
/// This is due to certain "bafy"-prefixed hashes being banned on Moralis IPFS gateway due to abuse.
///
/// If the URI does not meet these conditions or cannot be parsed, it is returned unchanged.
fn check_moralis_ipfs_bafy(token_uri: Option<&str>) -> Option<String> {
    token_uri.map(|uri| {
        if let Ok(parsed_url) = Url::parse(uri) {
            if parsed_url.host_str() == Some("ipfs.moralis.io") && parsed_url.path().starts_with("/ipfs/bafy") {
                let parts: Vec<&str> = parsed_url.path().splitn(2, "/ipfs/").collect();
                format!("https://ipfs.io/ipfs/{}", parts[1])
            } else {
                uri.to_string()
            }
        } else {
            uri.to_string()
        }
    })
}

async fn get_uri_meta(token_uri: Option<&str>, metadata: Option<&str>, url_antispam: &Url) -> UriMeta {
    let mut uri_meta = UriMeta::default();
    // Fetching data from the URL if token_uri is provided
    if let Some(token_uri) = token_uri {
        if let Some(url) = construct_camo_url_with_token(token_uri, url_antispam) {
            uri_meta = fetch_meta_from_url(url).await.unwrap_or_default();
        }
    }
    // Filling fields from metadata if provided
    if let Some(metadata) = metadata {
        if let Ok(meta_from_meta) = serde_json::from_str::<UriMeta>(metadata) {
            uri_meta.try_to_fill_missing_fields_from(meta_from_meta);
        }
    }
    update_uri_moralis_ipfs_fields(&mut uri_meta);
    drop_mutability!(uri_meta);
    uri_meta
}

fn construct_camo_url_with_token(token_uri: &str, url_antispam: &Url) -> Option<Url> {
    let mut url = url_antispam.clone();
    url.set_path("url/decode");
    url.path_segments_mut().ok()?.push(hex::encode(token_uri).as_str());
    Some(url)
}

async fn fetch_meta_from_url(url: Url) -> MmResult<UriMeta, MetaFromUrlError> {
    let response_meta = send_request_to_uri(url.as_str()).await?;
    serde_json::from_value(response_meta).map_err(|e| e.into())
}

fn update_uri_moralis_ipfs_fields(uri_meta: &mut UriMeta) {
    uri_meta.image_url = check_moralis_ipfs_bafy(uri_meta.image_url.as_deref());
    uri_meta.image_domain = get_domain_from_url(uri_meta.image_url.as_deref());
    uri_meta.animation_url = check_moralis_ipfs_bafy(uri_meta.animation_url.as_deref());
    uri_meta.animation_domain = get_domain_from_url(uri_meta.animation_url.as_deref());
    uri_meta.external_url = check_moralis_ipfs_bafy(uri_meta.external_url.as_deref());
    uri_meta.external_domain = get_domain_from_url(uri_meta.external_url.as_deref());
}

fn get_transfer_status(my_wallet: &str, to_address: &str) -> TransferStatus {
    // if my_wallet == from_address && my_wallet == to_address it is incoming transfer, so we can check just to_address.
    if my_wallet.to_lowercase() == to_address.to_lowercase() {
        TransferStatus::Receive
    } else {
        TransferStatus::Send
    }
}

/// `update_nft_list` function gets nft transfers from NFT HISTORY table, iterates through them
/// and updates NFT LIST table info.
async fn update_nft_list<T: NftListStorageOps + NftTransferHistoryStorageOps>(
    ctx: MmArc,
    storage: &T,
    chain: &Chain,
    scan_from_block: u64,
    url: &Url,
    url_antispam: &Url,
) -> MmResult<(), UpdateNftError> {
    let transfers = storage.get_transfers_from_block(*chain, scan_from_block).await?;
    let req = MyAddressReq {
        coin: chain.to_ticker().to_string(),
        path_to_address: StandardHDCoinAddress::default(),
    };
    let my_address = get_my_address(ctx.clone(), req).await?.wallet_address.to_lowercase();
    for transfer in transfers.into_iter() {
        handle_nft_transfer(storage, chain, url, url_antispam, transfer, &my_address).await?;
    }
    Ok(())
}

async fn handle_nft_transfer<T: NftListStorageOps + NftTransferHistoryStorageOps>(
    storage: &T,
    chain: &Chain,
    url: &Url,
    url_antispam: &Url,
    transfer: NftTransferHistory,
    my_address: &str,
) -> MmResult<(), UpdateNftError> {
    match (transfer.status, transfer.contract_type) {
        (TransferStatus::Send, ContractType::Erc721) => handle_send_erc721(storage, chain, transfer).await,
        (TransferStatus::Receive, ContractType::Erc721) => {
            handle_receive_erc721(storage, chain, transfer, url, url_antispam, my_address).await
        },
        (TransferStatus::Send, ContractType::Erc1155) => handle_send_erc1155(storage, chain, transfer).await,
        (TransferStatus::Receive, ContractType::Erc1155) => {
            handle_receive_erc1155(storage, chain, transfer, url, url_antispam, my_address).await
        },
    }
}

async fn handle_send_erc721<T: NftListStorageOps + NftTransferHistoryStorageOps>(
    storage: &T,
    chain: &Chain,
    transfer: NftTransferHistory,
) -> MmResult<(), UpdateNftError> {
    storage
        .get_nft(
            chain,
            eth_addr_to_hex(&transfer.common.token_address),
            transfer.token_id.clone(),
        )
        .await?
        .ok_or_else(|| UpdateNftError::TokenNotFoundInWallet {
            token_address: eth_addr_to_hex(&transfer.common.token_address),
            token_id: transfer.token_id.to_string(),
        })?;
    storage
        .remove_nft_from_list(
            chain,
            eth_addr_to_hex(&transfer.common.token_address),
            transfer.token_id,
            transfer.block_number,
        )
        .await?;
    Ok(())
}

async fn handle_receive_erc721<T: NftListStorageOps + NftTransferHistoryStorageOps>(
    storage: &T,
    chain: &Chain,
    transfer: NftTransferHistory,
    url: &Url,
    url_antispam: &Url,
    my_address: &str,
) -> MmResult<(), UpdateNftError> {
    let token_address_str = eth_addr_to_hex(&transfer.common.token_address);
    match storage
        .get_nft(chain, token_address_str.clone(), transfer.token_id.clone())
        .await?
    {
        Some(mut nft_db) => {
            // An error is raised if user tries to receive an identical ERC-721 token they already own
            // and if owner address != from address
            if my_address != eth_addr_to_hex(&transfer.common.from_address) {
                return MmError::err(UpdateNftError::AttemptToReceiveAlreadyOwnedErc721 {
                    tx_hash: transfer.common.transaction_hash,
                });
            }
            nft_db.block_number = transfer.block_number;
            storage
                .update_nft_amount_and_block_number(chain, nft_db.clone())
                .await?;
            update_transfer_meta_using_nft(storage, chain, &mut nft_db).await?;
        },
        None => {
            let mut nft = match get_moralis_metadata(
                token_address_str.clone(),
                transfer.token_id.clone(),
                chain,
                url,
                url_antispam,
            )
            .await
            {
                Ok(mut moralis_meta) => {
                    // sometimes moralis updates Get All NFTs (which also affects Get Metadata) later
                    // than History by Wallet update
                    moralis_meta.common.owner_of =
                        Address::from_str(my_address).map_to_mm(|e| UpdateNftError::InvalidHexString(e.to_string()))?;
                    moralis_meta.block_number = transfer.block_number;
                    moralis_meta
                },
                Err(_) => {
                    mark_as_spam_and_build_empty_meta(storage, chain, token_address_str, &transfer, my_address).await?
                },
            };
            storage
                .add_nfts_to_list(*chain, vec![nft.clone()], transfer.block_number)
                .await?;
            update_transfer_meta_using_nft(storage, chain, &mut nft).await?;
        },
    }
    Ok(())
}

async fn handle_send_erc1155<T: NftListStorageOps + NftTransferHistoryStorageOps>(
    storage: &T,
    chain: &Chain,
    transfer: NftTransferHistory,
) -> MmResult<(), UpdateNftError> {
    let token_address_str = eth_addr_to_hex(&transfer.common.token_address);
    let mut nft_db = storage
        .get_nft(chain, token_address_str.clone(), transfer.token_id.clone())
        .await?
        .ok_or_else(|| UpdateNftError::TokenNotFoundInWallet {
            token_address: token_address_str.clone(),
            token_id: transfer.token_id.to_string(),
        })?;
    match nft_db.common.amount.cmp(&transfer.common.amount) {
        Ordering::Equal => {
            storage
                .remove_nft_from_list(chain, token_address_str, transfer.token_id, transfer.block_number)
                .await?;
        },
        Ordering::Greater => {
            nft_db.common.amount -= transfer.common.amount;
            storage
                .update_nft_amount(chain, nft_db.clone(), transfer.block_number)
                .await?;
        },
        Ordering::Less => {
            return MmError::err(UpdateNftError::InsufficientAmountInCache {
                amount_list: nft_db.common.amount.to_string(),
                amount_history: transfer.common.amount.to_string(),
            });
        },
    }
    Ok(())
}

async fn handle_receive_erc1155<T: NftListStorageOps + NftTransferHistoryStorageOps>(
    storage: &T,
    chain: &Chain,
    transfer: NftTransferHistory,
    url: &Url,
    url_antispam: &Url,
    my_address: &str,
) -> MmResult<(), UpdateNftError> {
    let token_address_str = eth_addr_to_hex(&transfer.common.token_address);
    let mut nft = match storage
        .get_nft(chain, token_address_str.clone(), transfer.token_id.clone())
        .await?
    {
        Some(mut nft_db) => {
            // if owner address == from address, then owner sent tokens to themself,
            // which means that the amount will not change.
            if my_address != eth_addr_to_hex(&transfer.common.from_address) {
                nft_db.common.amount += transfer.common.amount;
            }
            nft_db.block_number = transfer.block_number;
            drop_mutability!(nft_db);
            storage
                .update_nft_amount_and_block_number(chain, nft_db.clone())
                .await?;
            nft_db
        },
        // If token isn't in NFT LIST table then add nft to the table.
        None => {
            let nft = match get_moralis_metadata(
                token_address_str.clone(),
                transfer.token_id.clone(),
                chain,
                url,
                url_antispam,
            )
            .await
            {
                Ok(moralis_meta) => {
                    create_nft_from_moralis_metadata(moralis_meta, &transfer, my_address, chain, url_antispam).await?
                },
                Err(_) => {
                    mark_as_spam_and_build_empty_meta(storage, chain, token_address_str, &transfer, my_address).await?
                },
            };
            storage
                .add_nfts_to_list(*chain, [nft.clone()], transfer.block_number)
                .await?;
            nft
        },
    };
    update_transfer_meta_using_nft(storage, chain, &mut nft).await?;
    Ok(())
}

async fn create_nft_from_moralis_metadata(
    moralis_meta: Nft,
    transfer: &NftTransferHistory,
    my_address: &str,
    chain: &Chain,
    url_antispam: &Url,
) -> MmResult<Nft, UpdateNftError> {
    let token_uri = check_moralis_ipfs_bafy(moralis_meta.common.token_uri.as_deref());
    let token_domain = get_domain_from_url(token_uri.as_deref());
    let uri_meta = get_uri_meta(
        token_uri.as_deref(),
        moralis_meta.common.metadata.as_deref(),
        url_antispam,
    )
    .await;
    let nft = Nft {
        common: NftCommon {
            token_address: moralis_meta.common.token_address,
            amount: transfer.common.amount.clone(),
            owner_of: Address::from_str(my_address).map_to_mm(|e| UpdateNftError::InvalidHexString(e.to_string()))?,
            token_hash: moralis_meta.common.token_hash,
            collection_name: moralis_meta.common.collection_name,
            symbol: moralis_meta.common.symbol,
            token_uri,
            token_domain,
            metadata: moralis_meta.common.metadata,
            last_token_uri_sync: moralis_meta.common.last_token_uri_sync,
            last_metadata_sync: moralis_meta.common.last_metadata_sync,
            minter_address: moralis_meta.common.minter_address,
            possible_spam: moralis_meta.common.possible_spam,
        },
        chain: *chain,
        token_id: moralis_meta.token_id,
        block_number_minted: moralis_meta.block_number_minted,
        block_number: transfer.block_number,
        contract_type: moralis_meta.contract_type,
        possible_phishing: false,
        uri_meta,
    };
    Ok(nft)
}

async fn mark_as_spam_and_build_empty_meta<T: NftListStorageOps + NftTransferHistoryStorageOps>(
    storage: &T,
    chain: &Chain,
    token_address_str: String,
    transfer: &NftTransferHistory,
    my_address: &str,
) -> MmResult<Nft, UpdateNftError> {
    storage
        .update_nft_spam_by_token_address(chain, token_address_str.clone(), true)
        .await?;
    storage
        .update_transfer_spam_by_token_address(chain, token_address_str, true)
        .await?;

    Ok(build_nft_with_empty_meta(BuildNftFields {
        token_address: transfer.common.token_address,
        token_id: transfer.token_id.clone(),
        amount: transfer.common.amount.clone(),
        owner_of: Address::from_str(my_address).map_to_mm(|e| UpdateNftError::InvalidHexString(e.to_string()))?,
        contract_type: transfer.contract_type,
        possible_spam: true,
        chain: transfer.chain,
        block_number: transfer.block_number,
    }))
}

/// `find_wallet_nft_amount` function returns NFT amount of cached NFT.
/// Note: in db **token_address** is kept in **lowercase**, because Moralis returns all addresses in lowercase.
pub(crate) async fn find_wallet_nft_amount(
    ctx: &MmArc,
    chain: &Chain,
    token_address: String,
    token_id: BigUint,
) -> MmResult<BigDecimal, GetNftInfoError> {
    let nft_ctx = NftCtx::from_ctx(ctx).map_to_mm(GetNftInfoError::Internal)?;

    let storage = nft_ctx.lock_db().await?;
    if !NftListStorageOps::is_initialized(&storage, chain).await? {
        NftListStorageOps::init(&storage, chain).await?;
    }
    let nft_meta = storage
        .get_nft(chain, token_address.to_lowercase(), token_id.clone())
        .await?
        .ok_or_else(|| GetNftInfoError::TokenNotFoundInWallet {
            token_address,
            token_id: token_id.to_string(),
        })?;
    Ok(nft_meta.common.amount)
}

async fn cache_nfts_from_moralis<T: NftListStorageOps + NftTransferHistoryStorageOps>(
    ctx: &MmArc,
    storage: &T,
    chain: &Chain,
    url: &Url,
    url_antispam: &Url,
) -> MmResult<Vec<Nft>, UpdateNftError> {
    let nft_list = get_moralis_nft_list(ctx, chain, url, url_antispam).await?;
    let last_scanned_block = NftTransferHistoryStorageOps::get_last_block_number(storage, chain)
        .await?
        .unwrap_or(0);
    storage
        .add_nfts_to_list(*chain, nft_list.clone(), last_scanned_block)
        .await?;
    Ok(nft_list)
}

/// `update_meta_in_transfers` function updates only transfers related to current nfts in wallet.
async fn update_meta_in_transfers<T>(storage: &T, chain: &Chain, nfts: Vec<Nft>) -> MmResult<(), UpdateNftError>
where
    T: NftListStorageOps + NftTransferHistoryStorageOps,
{
    for mut nft in nfts.into_iter() {
        update_transfer_meta_using_nft(storage, chain, &mut nft).await?;
    }
    Ok(())
}

/// `update_transfers_with_empty_meta` function updates empty metadata in transfers.
async fn update_transfers_with_empty_meta<T>(
    storage: &T,
    chain: &Chain,
    url: &Url,
    url_antispam: &Url,
) -> MmResult<(), UpdateNftError>
where
    T: NftListStorageOps + NftTransferHistoryStorageOps,
{
    let nft_token_addr_id = storage.get_transfers_with_empty_meta(*chain).await?;
    for addr_id_pair in nft_token_addr_id.into_iter() {
        let mut nft_meta = match get_moralis_metadata(
            addr_id_pair.token_address.clone(),
            addr_id_pair.token_id,
            chain,
            url,
            url_antispam,
        )
        .await
        {
            Ok(nft_meta) => nft_meta,
            Err(_) => {
                storage
                    .update_nft_spam_by_token_address(chain, addr_id_pair.token_address.clone(), true)
                    .await?;
                storage
                    .update_transfer_spam_by_token_address(chain, addr_id_pair.token_address, true)
                    .await?;
                continue;
            },
        };
        update_transfer_meta_using_nft(storage, chain, &mut nft_meta).await?;
    }
    Ok(())
}

/// `contains_disallowed_scheme` function checks if the text contains some link.
fn contains_disallowed_url(text: &str) -> Result<bool, regex::Error> {
    let url_regex = Regex::new(
        r"(?:(?:https?|ftp|file|[^:\s]+:)/?|[^:\s]+:/|\b(?:[a-z\d]+\.))(?:(?:[^\s()<>]+|\((?:[^\s()<>]+|(?:\([^\s()<>]+\)))?\))+(?:\((?:[^\s()<>]+|(?:\(?:[^\s()<>]+\)))?\)|[^\s`!()\[\]{};:'.,<>?«»“”‘’]))?",
    )?;
    Ok(url_regex.is_match(text))
}

/// `process_text_for_spam_link` checks if the text contains any links and optionally redacts it.
/// It doesn't matter if the link is valid or not, as this is a spam check.
/// If text contains some link, then function returns `true`.
fn process_text_for_spam_link(text: &mut Option<String>, redact: bool) -> Result<bool, regex::Error> {
    match text {
        Some(s) if contains_disallowed_url(s)? => {
            if redact {
                *text = Some("URL redacted for user protection".to_string());
            }
            Ok(true)
        },
        _ => Ok(false),
    }
}

/// `protect_from_history_spam_links` function checks and redact spam in `NftTransferHistory`.
///
/// `collection_name` and `token_name` in `NftTransferHistory` shouldn't contain any links,
/// they must be just an arbitrary text, which represents NFT names.
fn protect_from_history_spam_links(
    transfer: &mut NftTransferHistory,
    redact: bool,
) -> MmResult<(), ProtectFromSpamError> {
    let collection_name_spam = process_text_for_spam_link(&mut transfer.collection_name, redact)?;
    let token_name_spam = process_text_for_spam_link(&mut transfer.token_name, redact)?;

    if collection_name_spam || token_name_spam {
        transfer.common.possible_spam = true;
    }
    Ok(())
}

/// `protect_from_nft_spam_links` function checks and optionally redacts spam links in `Nft`.
///
/// `collection_name` and `token_name` in `Nft` shouldn't contain any links,
/// they must be just an arbitrary text, which represents NFT names.
/// `symbol` also must be a text or sign that represents a symbol.
/// This function also checks `metadata` field for spam.
fn protect_from_nft_spam_links(nft: &mut Nft, redact: bool) -> MmResult<(), ProtectFromSpamError> {
    let collection_name_spam = process_text_for_spam_link(&mut nft.common.collection_name, redact)?;
    let symbol_spam = process_text_for_spam_link(&mut nft.common.symbol, redact)?;
    let token_name_spam = process_text_for_spam_link(&mut nft.uri_meta.token_name, redact)?;
    let meta_spam = process_metadata_for_spam_link(nft, redact)?;

    if collection_name_spam || symbol_spam || token_name_spam || meta_spam {
        nft.common.possible_spam = true;
    }
    Ok(())
}

/// The `process_metadata_for_spam_link` function checks and optionally redacts spam link in the `metadata` field of `Nft`.
///
/// **note:** `token_name` is usually called `name` in `metadata`.
fn process_metadata_for_spam_link(nft: &mut Nft, redact: bool) -> MmResult<bool, ProtectFromSpamError> {
    if let Some(Ok(mut metadata)) = nft
        .common
        .metadata
        .as_ref()
        .map(|t| serde_json::from_str::<serde_json::Map<String, Json>>(t))
    {
        let spam_detected = process_metadata_field(&mut metadata, "name", redact)?;
        if redact && spam_detected {
            nft.common.metadata = Some(serde_json::to_string(&metadata)?);
        }
        return Ok(spam_detected);
    }
    Ok(false)
}

/// The `process_metadata_field` function scans a specified field in a JSON metadata object for potential spam.
///
/// This function checks the provided `metadata` map for a field matching the `field` parameter.
/// If this field is found and its value contains some link, it's considered to contain spam.
/// Depending on the `redact` flag, it will either redact the spam link or leave it as it is.
/// The function returns `true` if it detected a spam link, or `false` otherwise.
fn process_metadata_field(
    metadata: &mut serde_json::Map<String, Json>,
    field: &str,
    redact: bool,
) -> MmResult<bool, ProtectFromSpamError> {
    match metadata.get(field).and_then(|v| v.as_str()) {
        Some(text) if contains_disallowed_url(text)? => {
            if redact {
                metadata.insert(
                    field.to_string(),
                    serde_json::Value::String("URL redacted for user protection".to_string()),
                );
            }
            Ok(true)
        },
        _ => Ok(false),
    }
}

async fn build_nft_from_moralis(
    chain: Chain,
    nft_moralis: NftFromMoralis,
    contract_type: ContractType,
    url_antispam: &Url,
) -> Nft {
    let token_uri = check_moralis_ipfs_bafy(nft_moralis.common.token_uri.as_deref());
    let uri_meta = get_uri_meta(
        token_uri.as_deref(),
        nft_moralis.common.metadata.as_deref(),
        url_antispam,
    )
    .await;
    let token_domain = get_domain_from_url(token_uri.as_deref());
    Nft {
        common: NftCommon {
            token_address: nft_moralis.common.token_address,
            amount: nft_moralis.common.amount,
            owner_of: nft_moralis.common.owner_of,
            token_hash: nft_moralis.common.token_hash,
            collection_name: nft_moralis.common.collection_name,
            symbol: nft_moralis.common.symbol,
            token_uri,
            token_domain,
            metadata: nft_moralis.common.metadata,
            last_token_uri_sync: nft_moralis.common.last_token_uri_sync,
            last_metadata_sync: nft_moralis.common.last_metadata_sync,
            minter_address: nft_moralis.common.minter_address,
            possible_spam: nft_moralis.common.possible_spam,
        },
        chain,
        token_id: nft_moralis.token_id.0,
        block_number_minted: nft_moralis.block_number_minted.map(|v| v.0),
        block_number: *nft_moralis.block_number,
        contract_type,
        possible_phishing: false,
        uri_meta,
    }
}

#[inline(always)]
pub(crate) fn get_domain_from_url(url: Option<&str>) -> Option<String> {
    url.and_then(|uri| Url::parse(uri).ok())
        .and_then(|url| url.domain().map(String::from))
}
