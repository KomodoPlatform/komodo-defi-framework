use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::{MmError, MmResult};
use url::Url;

pub(crate) mod nft_errors;
pub(crate) mod nft_structs;
pub(crate) mod storage;

#[cfg(any(test, target_arch = "wasm32"))] mod nft_tests;

use crate::{get_my_address, MyAddressReq, WithdrawError};
use nft_errors::{GetInfoFromUriError, GetNftInfoError, UpdateNftError};
use nft_structs::{Chain, ContractType, ConvertChain, Nft, NftFromMoralis, NftList, NftListReq, NftMetadataReq,
                  NftTransferHistory, NftTransfersReq, NftTxHistoryFromMoralis, NftsTransferHistoryList,
                  TransactionNftDetails, UpdateNftReq, WithdrawNftReq};

use crate::eth::{get_eth_address, withdraw_erc1155, withdraw_erc721};
use crate::nft::nft_errors::ProtectFromSpamError;
use crate::nft::nft_structs::{NftCommon, NftTransferCommon, RefreshMetadataReq, TransferStatus, TxMeta, UriMeta};
use crate::nft::storage::{NftListStorageOps, NftStorageBuilder, NftTxHistoryStorageOps};
use common::{parse_rfc3339_to_timestamp, APPLICATION_JSON};
use http::header::ACCEPT;
use mm2_err_handle::map_to_mm::MapToMmResult;
use mm2_number::BigDecimal;
use regex::Regex;
use serde_json::Value as Json;
use std::cmp::Ordering;

const MORALIS_API_ENDPOINT: &str = "api/v2";
/// query parameters for moralis request: The format of the token ID
const MORALIS_FORMAT_QUERY_NAME: &str = "format";
const MORALIS_FORMAT_QUERY_VALUE: &str = "decimal";
/// query parameters for moralis request: The transfer direction
const MORALIS_DIRECTION_QUERY_NAME: &str = "direction";
const MORALIS_DIRECTION_QUERY_VALUE: &str = "both";
/// The minimum block number from which to get the transfers
const MORALIS_FROM_BLOCK_QUERY_NAME: &str = "from_block";

pub type WithdrawNftResult = Result<TransactionNftDetails, MmError<WithdrawError>>;

/// `get_nft_list` function returns list of NFTs on requested chains owned by user.
pub async fn get_nft_list(ctx: MmArc, req: NftListReq) -> MmResult<NftList, GetNftInfoError> {
    let storage = NftStorageBuilder::new(&ctx).build()?;
    for chain in req.chains.iter() {
        if !NftListStorageOps::is_initialized(&storage, chain).await? {
            NftListStorageOps::init(&storage, chain).await?;
        }
    }
    let mut nft_list = storage
        .get_nft_list(req.chains, req.max, req.limit, req.page_number)
        .await?;
    if req.protect_from_spam {
        for nft in &mut nft_list.nfts {
            protect_from_nft_spam(nft)?;
        }
    }
    drop_mutability!(nft_list);
    Ok(nft_list)
}

/// `get_nft_metadata` function returns info of one specific NFT.
pub async fn get_nft_metadata(ctx: MmArc, req: NftMetadataReq) -> MmResult<Nft, GetNftInfoError> {
    let storage = NftStorageBuilder::new(&ctx).build()?;
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
        protect_from_nft_spam(&mut nft)?;
    }
    drop_mutability!(nft);
    Ok(nft)
}

/// `get_nft_transfers` function returns a transfer history of NFTs on requested chains owned by user.
pub async fn get_nft_transfers(ctx: MmArc, req: NftTransfersReq) -> MmResult<NftsTransferHistoryList, GetNftInfoError> {
    let storage = NftStorageBuilder::new(&ctx).build()?;
    for chain in req.chains.iter() {
        if !NftTxHistoryStorageOps::is_initialized(&storage, chain).await? {
            NftTxHistoryStorageOps::init(&storage, chain).await?;
        }
    }
    let mut transfer_history_list = storage
        .get_tx_history(req.chains, req.max, req.limit, req.page_number, req.filters)
        .await?;
    if req.protect_from_spam {
        for tx in &mut transfer_history_list.transfer_history {
            protect_from_history_spam(tx)?;
        }
    }
    drop_mutability!(transfer_history_list);
    Ok(transfer_history_list)
}

/// `update_nft` function updates cache of nft transfer history and nft list.
pub async fn update_nft(ctx: MmArc, req: UpdateNftReq) -> MmResult<(), UpdateNftError> {
    let storage = NftStorageBuilder::new(&ctx).build()?;
    for chain in req.chains.iter() {
        let tx_history_initialized = NftTxHistoryStorageOps::is_initialized(&storage, chain).await?;

        let from_block = if tx_history_initialized {
            let last_tx_block = NftTxHistoryStorageOps::get_last_block_number(&storage, chain).await?;
            last_tx_block.map(|b| b + 1)
        } else {
            NftTxHistoryStorageOps::init(&storage, chain).await?;
            None
        };
        let nft_transfers = get_moralis_nft_transfers(&ctx, chain, from_block, &req.url).await?;
        storage.add_txs_to_history(chain, nft_transfers).await?;

        let nft_block = match NftListStorageOps::get_last_block_number(&storage, chain).await {
            Ok(Some(block)) => block,
            Ok(None) => {
                // if there are no rows in NFT LIST table we can try to get all info from moralis.
                let nfts = cache_nfts_from_moralis(&ctx, &storage, chain, &req.url).await?;
                update_meta_in_txs(&storage, chain, nfts).await?;
                update_txs_with_empty_meta(&storage, chain, &req.url).await?;
                continue;
            },
            Err(_) => {
                // if there is an error, then NFT LIST table doesnt exist, so we need to cache from mroalis.
                NftListStorageOps::init(&storage, chain).await?;
                let nft_list = get_moralis_nft_list(&ctx, chain, &req.url).await?;
                let last_scanned_block = NftTxHistoryStorageOps::get_last_block_number(&storage, chain)
                    .await?
                    .unwrap_or(0);
                storage
                    .add_nfts_to_list(chain, nft_list.clone(), last_scanned_block)
                    .await?;
                update_meta_in_txs(&storage, chain, nft_list).await?;
                update_txs_with_empty_meta(&storage, chain, &req.url).await?;
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
        update_nft_list(ctx.clone(), &storage, chain, scanned_block + 1, &req.url).await?;
        update_txs_with_empty_meta(&storage, chain, &req.url).await?;
    }
    Ok(())
}

pub async fn refresh_nft_metadata(ctx: MmArc, req: RefreshMetadataReq) -> MmResult<(), UpdateNftError> {
    let storage = NftStorageBuilder::new(&ctx).build()?;
    let moralis_meta = get_moralis_metadata(
        format!("{:#02x}", req.token_address),
        req.token_id.clone(),
        &req.chain,
        &req.url,
    )
    .await?;
    let req = NftMetadataReq {
        token_address: req.token_address,
        token_id: req.token_id,
        chain: req.chain,
        protect_from_spam: false,
    };
    let mut nft_db = get_nft_metadata(ctx, req).await?;
    let token_uri = check_moralis_ipfs_bafy(moralis_meta.common.token_uri.as_deref());
    let uri_meta = get_uri_meta(token_uri.as_deref(), moralis_meta.common.metadata.as_deref()).await;
    nft_db.common.collection_name = moralis_meta.common.collection_name;
    nft_db.common.symbol = moralis_meta.common.symbol;
    nft_db.common.token_uri = token_uri;
    nft_db.common.metadata = moralis_meta.common.metadata;
    nft_db.common.last_token_uri_sync = moralis_meta.common.last_token_uri_sync;
    nft_db.common.last_metadata_sync = moralis_meta.common.last_metadata_sync;
    nft_db.common.possible_spam = moralis_meta.common.possible_spam;
    nft_db.uri_meta = uri_meta;
    drop_mutability!(nft_db);
    storage
        .refresh_nft_metadata(&moralis_meta.chain, nft_db.clone())
        .await?;
    let tx_meta = TxMeta::from(nft_db.clone());
    storage.update_txs_meta_by_token_addr_id(&nft_db.chain, tx_meta).await?;
    Ok(())
}

async fn get_moralis_nft_list(ctx: &MmArc, chain: &Chain, url: &Url) -> MmResult<Vec<Nft>, GetNftInfoError> {
    let mut res_list = Vec::new();
    let my_address = get_eth_address(ctx, &chain.to_ticker()).await?;

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
                let nft = build_nft_from_moralis(chain, nft_moralis, contract_type).await;
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
) -> MmResult<Vec<NftTransferHistory>, GetNftInfoError> {
    let mut res_list = Vec::new();
    let my_address = get_eth_address(ctx, &chain.to_ticker()).await?;

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
        .append_pair(MORALIS_DIRECTION_QUERY_NAME, MORALIS_DIRECTION_QUERY_VALUE)
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
                let transfer_moralis: NftTxHistoryFromMoralis = serde_json::from_str(&transfer.to_string())?;
                let contract_type = match transfer_moralis.contract_type {
                    Some(contract_type) => contract_type,
                    None => continue,
                };
                let status = get_tx_status(&wallet_address, &transfer_moralis.common.to_address);
                let block_timestamp = parse_rfc3339_to_timestamp(&transfer_moralis.block_timestamp)?;
                let transfer_history = NftTransferHistory {
                    common: NftTransferCommon {
                        block_hash: transfer_moralis.common.block_hash,
                        transaction_hash: transfer_moralis.common.transaction_hash,
                        transaction_index: transfer_moralis.common.transaction_index,
                        log_index: transfer_moralis.common.log_index,
                        value: transfer_moralis.common.value,
                        transaction_type: transfer_moralis.common.transaction_type,
                        token_address: transfer_moralis.common.token_address,
                        token_id: transfer_moralis.common.token_id,
                        from_address: transfer_moralis.common.from_address,
                        to_address: transfer_moralis.common.to_address,
                        amount: transfer_moralis.common.amount,
                        verified: transfer_moralis.common.verified,
                        operator: transfer_moralis.common.operator,
                        possible_spam: transfer_moralis.common.possible_spam,
                    },
                    chain: *chain,
                    block_number: *transfer_moralis.block_number,
                    block_timestamp,
                    contract_type,
                    token_uri: None,
                    collection_name: None,
                    image_url: None,
                    token_name: None,
                    status,
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

/// **Caution:** ERC-1155 token can have a total supply more than 1, which means there could be several owners
/// of the same token. `get_nft_metadata` returns NFTs info with the most recent owner.
/// **Dont** use this function to get specific info about owner address, amount etc, you will get info not related to my_address.
async fn get_moralis_metadata(
    token_address: String,
    token_id: BigDecimal,
    chain: &Chain,
    url: &Url,
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
    let nft_metadata = build_nft_from_moralis(chain, nft_moralis, contract_type).await;
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

#[cfg(not(target_arch = "wasm32"))]
async fn send_request_to_uri(uri: &str) -> MmResult<Json, GetInfoFromUriError> {
    use http::header::HeaderValue;
    use mm2_net::transport::slurp_req_body;

    let request = http::Request::builder()
        .method("GET")
        .uri(uri)
        .header(ACCEPT, HeaderValue::from_static(APPLICATION_JSON))
        .body(hyper::Body::from(""))?;

    let (status, _header, body) = slurp_req_body(request).await?;
    if !status.is_success() {
        return Err(MmError::new(GetInfoFromUriError::Transport(format!(
            "Response !200 from {}: {}, {}",
            uri, status, body
        ))));
    }
    Ok(body)
}

#[cfg(target_arch = "wasm32")]
async fn send_request_to_uri(uri: &str) -> MmResult<Json, GetInfoFromUriError> {
    use mm2_net::wasm_http::FetchRequest;

    macro_rules! try_or {
        ($exp:expr, $errtype:ident) => {
            match $exp {
                Ok(x) => x,
                Err(e) => return Err(MmError::new(GetInfoFromUriError::$errtype(ERRL!("{:?}", e)))),
            }
        };
    }

    let result = FetchRequest::get(uri)
        .header(ACCEPT.as_str(), APPLICATION_JSON)
        .request_str()
        .await;
    let (status_code, response_str) = try_or!(result, Transport);
    if !status_code.is_success() {
        return Err(MmError::new(GetInfoFromUriError::Transport(ERRL!(
            "!200: {}, {}",
            status_code,
            response_str
        ))));
    }

    let response: Json = try_or!(serde_json::from_str(&response_str), InvalidResponse);
    Ok(response)
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

async fn get_uri_meta(token_uri: Option<&str>, metadata: Option<&str>) -> UriMeta {
    let mut uri_meta = UriMeta::default();
    if let Some(token_uri) = token_uri {
        if let Ok(response_meta) = send_request_to_uri(token_uri).await {
            if let Ok(token_uri_meta) = serde_json::from_value(response_meta) {
                uri_meta = token_uri_meta;
            }
        }
    }
    if let Some(metadata) = metadata {
        if let Ok(meta_from_meta) = serde_json::from_str::<UriMeta>(metadata) {
            uri_meta.try_to_fill_missing_fields_from(meta_from_meta)
        }
    }
    uri_meta.image_url = check_moralis_ipfs_bafy(uri_meta.image_url.as_deref());
    uri_meta.animation_url = check_moralis_ipfs_bafy(uri_meta.animation_url.as_deref());
    drop_mutability!(uri_meta);
    uri_meta
}

fn get_tx_status(my_wallet: &str, to_address: &str) -> TransferStatus {
    // if my_wallet == from_address && my_wallet == to_address it is incoming tx, so we can check just to_address.
    if my_wallet.to_lowercase() == to_address.to_lowercase() {
        TransferStatus::Receive
    } else {
        TransferStatus::Send
    }
}

/// `update_nft_list` function gets nft transfers from NFT HISTORY table, iterates through them
/// and updates NFT LIST table info.
async fn update_nft_list<T: NftListStorageOps + NftTxHistoryStorageOps>(
    ctx: MmArc,
    storage: &T,
    chain: &Chain,
    scan_from_block: u64,
    url: &Url,
) -> MmResult<(), UpdateNftError> {
    let txs = storage.get_txs_from_block(chain, scan_from_block).await?;
    let req = MyAddressReq {
        coin: chain.to_ticker(),
    };
    let my_address = get_my_address(ctx.clone(), req).await?.wallet_address.to_lowercase();
    for tx in txs.into_iter() {
        handle_nft_tx(storage, chain, url, tx, &my_address).await?;
    }
    Ok(())
}

async fn handle_nft_tx<T: NftListStorageOps + NftTxHistoryStorageOps>(
    storage: &T,
    chain: &Chain,
    url: &Url,
    tx: NftTransferHistory,
    my_address: &str,
) -> MmResult<(), UpdateNftError> {
    match (tx.status, tx.contract_type) {
        (TransferStatus::Send, ContractType::Erc721) => handle_send_erc721(storage, chain, tx).await,
        (TransferStatus::Receive, ContractType::Erc721) => {
            handle_receive_erc721(storage, chain, tx, url, my_address).await
        },
        (TransferStatus::Send, ContractType::Erc1155) => handle_send_erc1155(storage, chain, tx).await,
        (TransferStatus::Receive, ContractType::Erc1155) => {
            handle_receive_erc1155(storage, chain, tx, url, my_address).await
        },
    }
}

async fn handle_send_erc721<T: NftListStorageOps + NftTxHistoryStorageOps>(
    storage: &T,
    chain: &Chain,
    tx: NftTransferHistory,
) -> MmResult<(), UpdateNftError> {
    let nft_db = storage
        .get_nft(chain, tx.common.token_address.clone(), tx.common.token_id.clone())
        .await?
        .ok_or_else(|| UpdateNftError::TokenNotFoundInWallet {
            token_address: tx.common.token_address.clone(),
            token_id: tx.common.token_id.to_string(),
        })?;
    let tx_meta = TxMeta::from(nft_db);
    storage.update_txs_meta_by_token_addr_id(chain, tx_meta).await?;
    storage
        .remove_nft_from_list(chain, tx.common.token_address, tx.common.token_id, tx.block_number)
        .await?;
    Ok(())
}

async fn handle_receive_erc721<T: NftListStorageOps + NftTxHistoryStorageOps>(
    storage: &T,
    chain: &Chain,
    tx: NftTransferHistory,
    url: &Url,
    my_address: &str,
) -> MmResult<(), UpdateNftError> {
    let nft = match storage
        .get_nft(chain, tx.common.token_address.clone(), tx.common.token_id.clone())
        .await?
    {
        Some(mut nft_db) => {
            // An error is raised if user tries to receive an identical ERC-721 token they already own
            // and if owner address != from address
            if my_address != tx.common.from_address {
                return MmError::err(UpdateNftError::AttemptToReceiveAlreadyOwnedErc721 {
                    tx_hash: tx.common.transaction_hash,
                });
            }
            nft_db.block_number = tx.block_number;
            drop_mutability!(nft_db);
            storage
                .update_nft_amount_and_block_number(chain, nft_db.clone())
                .await?;
            nft_db
        },
        // If token isn't in NFT LIST table then add nft to the table.
        None => {
            let mut nft = get_moralis_metadata(tx.common.token_address, tx.common.token_id, chain, url).await?;
            // sometimes moralis updates Get All NFTs (which also affects Get Metadata) later
            // than History by Wallet update
            nft.common.owner_of = my_address.to_string();
            nft.block_number = tx.block_number;
            drop_mutability!(nft);
            storage
                .add_nfts_to_list(chain, vec![nft.clone()], tx.block_number)
                .await?;
            nft
        },
    };
    let tx_meta = TxMeta::from(nft);
    storage.update_txs_meta_by_token_addr_id(chain, tx_meta).await?;
    Ok(())
}

async fn handle_send_erc1155<T: NftListStorageOps + NftTxHistoryStorageOps>(
    storage: &T,
    chain: &Chain,
    tx: NftTransferHistory,
) -> MmResult<(), UpdateNftError> {
    let mut nft_db = storage
        .get_nft(chain, tx.common.token_address.clone(), tx.common.token_id.clone())
        .await?
        .ok_or_else(|| UpdateNftError::TokenNotFoundInWallet {
            token_address: tx.common.token_address.clone(),
            token_id: tx.common.token_id.to_string(),
        })?;
    match nft_db.common.amount.cmp(&tx.common.amount) {
        Ordering::Equal => {
            storage
                .remove_nft_from_list(chain, tx.common.token_address, tx.common.token_id, tx.block_number)
                .await?;
        },
        Ordering::Greater => {
            nft_db.common.amount -= tx.common.amount;
            storage
                .update_nft_amount(chain, nft_db.clone(), tx.block_number)
                .await?;
        },
        Ordering::Less => {
            return MmError::err(UpdateNftError::InsufficientAmountInCache {
                amount_list: nft_db.common.amount.to_string(),
                amount_history: tx.common.amount.to_string(),
            });
        },
    }
    let tx_meta = TxMeta::from(nft_db);
    storage.update_txs_meta_by_token_addr_id(chain, tx_meta).await?;
    Ok(())
}

async fn handle_receive_erc1155<T: NftListStorageOps + NftTxHistoryStorageOps>(
    storage: &T,
    chain: &Chain,
    tx: NftTransferHistory,
    url: &Url,
    my_address: &str,
) -> MmResult<(), UpdateNftError> {
    let nft = match storage
        .get_nft(chain, tx.common.token_address.clone(), tx.common.token_id.clone())
        .await?
    {
        Some(mut nft_db) => {
            // if owner address == from address, then owner sent tokens to themself,
            // which means that the amount will not change.
            if my_address != tx.common.from_address {
                nft_db.common.amount += tx.common.amount;
            }
            nft_db.block_number = tx.block_number;
            drop_mutability!(nft_db);
            storage
                .update_nft_amount_and_block_number(chain, nft_db.clone())
                .await?;
            nft_db
        },
        // If token isn't in NFT LIST table then add nft to the table.
        None => {
            let moralis_meta =
                get_moralis_metadata(tx.common.token_address, tx.common.token_id.clone(), chain, url).await?;
            let token_uri = check_moralis_ipfs_bafy(moralis_meta.common.token_uri.as_deref());
            let uri_meta = get_uri_meta(token_uri.as_deref(), moralis_meta.common.metadata.as_deref()).await;
            let nft = Nft {
                common: NftCommon {
                    token_address: moralis_meta.common.token_address,
                    token_id: moralis_meta.common.token_id,
                    amount: tx.common.amount,
                    owner_of: my_address.to_string(),
                    token_hash: moralis_meta.common.token_hash,
                    collection_name: moralis_meta.common.collection_name,
                    symbol: moralis_meta.common.symbol,
                    token_uri,
                    metadata: moralis_meta.common.metadata,
                    last_token_uri_sync: moralis_meta.common.last_token_uri_sync,
                    last_metadata_sync: moralis_meta.common.last_metadata_sync,
                    minter_address: moralis_meta.common.minter_address,
                    possible_spam: moralis_meta.common.possible_spam,
                },
                chain: *chain,
                block_number_minted: moralis_meta.block_number_minted,
                block_number: tx.block_number,
                contract_type: moralis_meta.contract_type,
                uri_meta,
            };
            storage.add_nfts_to_list(chain, [nft.clone()], tx.block_number).await?;
            nft
        },
    };
    let tx_meta = TxMeta::from(nft);
    storage.update_txs_meta_by_token_addr_id(chain, tx_meta).await?;
    Ok(())
}

/// `find_wallet_nft_amount` function returns NFT amount of cached NFT.
/// Note: in db **token_address** is kept in **lowercase**, because Moralis returns all addresses in lowercase.
pub(crate) async fn find_wallet_nft_amount(
    ctx: &MmArc,
    chain: &Chain,
    token_address: String,
    token_id: BigDecimal,
) -> MmResult<BigDecimal, GetNftInfoError> {
    let storage = NftStorageBuilder::new(ctx).build()?;
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

async fn cache_nfts_from_moralis<T: NftListStorageOps + NftTxHistoryStorageOps>(
    ctx: &MmArc,
    storage: &T,
    chain: &Chain,
    url: &Url,
) -> MmResult<Vec<Nft>, UpdateNftError> {
    let nft_list = get_moralis_nft_list(ctx, chain, url).await?;
    let last_scanned_block = NftTxHistoryStorageOps::get_last_block_number(storage, chain)
        .await?
        .unwrap_or(0);
    storage
        .add_nfts_to_list(chain, nft_list.clone(), last_scanned_block)
        .await?;
    Ok(nft_list)
}

/// `update_meta_in_txs` function updates only txs related to current nfts in wallet.
async fn update_meta_in_txs<T>(storage: &T, chain: &Chain, nfts: Vec<Nft>) -> MmResult<(), UpdateNftError>
where
    T: NftListStorageOps + NftTxHistoryStorageOps,
{
    for nft in nfts.into_iter() {
        let tx_meta = TxMeta::from(nft);
        storage.update_txs_meta_by_token_addr_id(chain, tx_meta).await?;
    }
    Ok(())
}

/// `update_txs_with_empty_meta` function updates empty metadata in transfers.
async fn update_txs_with_empty_meta<T>(storage: &T, chain: &Chain, url: &Url) -> MmResult<(), UpdateNftError>
where
    T: NftListStorageOps + NftTxHistoryStorageOps,
{
    let nft_token_addr_id = storage.get_txs_with_empty_meta(chain).await?;
    for addr_id_pair in nft_token_addr_id.into_iter() {
        let nft_meta = get_moralis_metadata(addr_id_pair.token_address, addr_id_pair.token_id, chain, url).await?;
        let tx_meta = TxMeta::from(nft_meta);
        storage.update_txs_meta_by_token_addr_id(chain, tx_meta).await?;
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

/// `check_and_redact_if_spam` checks if the text contains any links.
/// It doesn't matter if the link is valid or not, as this is a spam check.
/// If text contains some link, then it is a spam.
fn check_and_redact_if_spam(text: &mut Option<String>) -> Result<bool, regex::Error> {
    match text {
        Some(s) if contains_disallowed_url(s)? => {
            *text = Some("URL redacted for user protection".to_string());
            Ok(true)
        },
        _ => Ok(false),
    }
}

/// `protect_from_history_spam` function checks and redact spam in `NftTransferHistory`.
///
/// `collection_name` and `token_name` in `NftTransferHistory` shouldn't contain any links,
/// they must be just an arbitrary text, which represents NFT names.
fn protect_from_history_spam(tx: &mut NftTransferHistory) -> MmResult<(), ProtectFromSpamError> {
    let collection_name_spam = check_and_redact_if_spam(&mut tx.collection_name)?;
    let token_name_spam = check_and_redact_if_spam(&mut tx.token_name)?;

    if collection_name_spam || token_name_spam {
        tx.common.possible_spam = true;
    }
    Ok(())
}

/// `protect_from_nft_spam` function checks and redact spam in `Nft`.
///
/// `collection_name` and `token_name` in `Nft` shouldn't contain any links,
/// they must be just an arbitrary text, which represents NFT names.
/// `symbol` also must be a text or sign that represents a symbol.
fn protect_from_nft_spam(nft: &mut Nft) -> MmResult<(), ProtectFromSpamError> {
    let collection_name_spam = check_and_redact_if_spam(&mut nft.common.collection_name)?;
    let symbol_spam = check_and_redact_if_spam(&mut nft.common.symbol)?;
    let token_name_spam = check_and_redact_if_spam(&mut nft.uri_meta.token_name)?;
    let meta_spam = check_nft_metadata_for_spam(nft)?;

    if collection_name_spam || symbol_spam || token_name_spam || meta_spam {
        nft.common.possible_spam = true;
    }
    Ok(())
}
/// `check_nft_metadata_for_spam` function checks and redact spam in `metadata` field from `Nft`.
///
/// **note:** `token_name` is usually called `name` in `metadata`.
fn check_nft_metadata_for_spam(nft: &mut Nft) -> MmResult<bool, ProtectFromSpamError> {
    if let Some(Ok(mut metadata)) = nft
        .common
        .metadata
        .as_ref()
        .map(|t| serde_json::from_str::<serde_json::Map<String, Json>>(t))
    {
        if check_spam_and_redact_metadata_field(&mut metadata, "name")? {
            nft.common.metadata = Some(serde_json::to_string(&metadata)?);
            return Ok(true);
        }
    }
    Ok(false)
}

/// The `check_spam_and_redact_metadata_field` function scans a specified field in a JSON metadata object for potential spam.
///
/// This function checks the provided `metadata` map for a field matching the `field` parameter.
/// If this field is found and its value contains some link, it's considered to contain spam.
/// To protect users, function redacts field containing spam link.
/// The function returns `true` if it detected spam link, or `false` otherwise.
fn check_spam_and_redact_metadata_field(
    metadata: &mut serde_json::Map<String, Json>,
    field: &str,
) -> MmResult<bool, ProtectFromSpamError> {
    match metadata.get(field).and_then(|v| v.as_str()) {
        Some(text) if contains_disallowed_url(text)? => {
            metadata.insert(
                field.to_string(),
                serde_json::Value::String("URL redacted for user protection".to_string()),
            );
            Ok(true)
        },
        _ => Ok(false),
    }
}

async fn build_nft_from_moralis(chain: &Chain, nft_moralis: NftFromMoralis, contract_type: ContractType) -> Nft {
    let token_uri = check_moralis_ipfs_bafy(nft_moralis.common.token_uri.as_deref());
    let uri_meta = get_uri_meta(token_uri.as_deref(), nft_moralis.common.metadata.as_deref()).await;
    Nft {
        common: NftCommon {
            token_address: nft_moralis.common.token_address,
            token_id: nft_moralis.common.token_id,
            amount: nft_moralis.common.amount,
            owner_of: nft_moralis.common.owner_of,
            token_hash: nft_moralis.common.token_hash,
            collection_name: nft_moralis.common.collection_name,
            symbol: nft_moralis.common.symbol,
            token_uri,
            metadata: nft_moralis.common.metadata,
            last_token_uri_sync: nft_moralis.common.last_token_uri_sync,
            last_metadata_sync: nft_moralis.common.last_metadata_sync,
            minter_address: nft_moralis.common.minter_address,
            possible_spam: nft_moralis.common.possible_spam,
        },
        chain: *chain,
        block_number_minted: nft_moralis.block_number_minted.map(|v| v.0),
        block_number: *nft_moralis.block_number,
        contract_type,
        uri_meta,
    }
}
