use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::{MmError, MmResult};

pub(crate) mod nft_errors;
pub(crate) mod nft_structs;

use crate::WithdrawError;
use nft_errors::GetNftInfoError;
use nft_structs::{Chain, ConvertChain, Nft, NftList, NftListReq, NftMetadataReq, NftTransferHistory,
                  NftTransferHistoryWrapper, NftTransfersReq, NftWrapper, NftsTransferHistoryList,
                  TransactionNftDetails, WithdrawNftReq};

use crate::eth::{get_eth_address, withdraw_erc1155, withdraw_erc721};
use crate::nft::nft_structs::WithdrawNftType;
use common::APPLICATION_JSON;
use http::header::ACCEPT;
use mm2_number::BigDecimal;
use serde_json::Value as Json;

const MORALIS_API_ENDPOINT: &str = "/api/v2/";
/// query parameter for moralis request: The format of the token ID
const FORMAT_DECIMAL_MORALIS: &str = "format=decimal";
/// query parameter for moralis request: The transfer direction
const DIRECTION_BOTH_MORALIS: &str = "direction=both";

pub type WithdrawNftResult = Result<TransactionNftDetails, MmError<WithdrawError>>;

/// `get_nft_list` function returns list of NFTs on requested chains owned by user.
pub async fn get_nft_list(ctx: MmArc, req: NftListReq) -> MmResult<NftList, GetNftInfoError> {
    let mut res_list = Vec::new();
    for chain in req.chains {
        let (coin_str, chain_str) = chain.to_ticker_chain();
        let my_address = get_eth_address(&ctx, &coin_str).await?;
        let req_url = &req.url;
        let wallet_address = my_address.wallet_address;
        let uri_without_cursor =
            format!("{req_url}{MORALIS_API_ENDPOINT}{wallet_address}/nft?chain={chain_str}&{FORMAT_DECIMAL_MORALIS}");

        // The cursor returned in the previous response (used for getting the next page).
        let mut cursor = String::new();
        loop {
            let uri = format!("{}{}", uri_without_cursor, cursor);
            let response = send_moralis_request(uri.as_str()).await?;
            if let Some(nfts_list) = response["result"].as_array() {
                for nft_json in nfts_list {
                    let nft_wrapper: NftWrapper = serde_json::from_str(&nft_json.to_string())?;
                    let nft = Nft {
                        chain,
                        token_address: nft_wrapper.token_address,
                        token_id: nft_wrapper.token_id.0,
                        amount: nft_wrapper.amount.0,
                        owner_of: nft_wrapper.owner_of,
                        token_hash: nft_wrapper.token_hash,
                        block_number_minted: *nft_wrapper.block_number_minted,
                        block_number: *nft_wrapper.block_number,
                        contract_type: nft_wrapper.contract_type.map(|v| v.0),
                        name: nft_wrapper.name,
                        symbol: nft_wrapper.symbol,
                        token_uri: nft_wrapper.token_uri,
                        metadata: nft_wrapper.metadata,
                        last_token_uri_sync: nft_wrapper.last_token_uri_sync,
                        last_metadata_sync: nft_wrapper.last_metadata_sync,
                        minter_address: nft_wrapper.minter_address,
                        possible_spam: nft_wrapper.possible_spam,
                    };
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
    }
    drop_mutability!(res_list);
    let nft_list = NftList { nfts: res_list };
    Ok(nft_list)
}

/// `get_nft_metadata` function returns info of one specific NFT.
/// Current implementation sends request to Moralis.
/// Later, after adding caching, metadata lookup can be performed using previously obtained NFTs info without
/// sending new moralis request. The moralis request can be sent as a fallback, if the data was not found in the cache.
///
/// **Caution:** ERC-1155 token can have a total supply more than 1, which means there could be several owners
/// of the same token. `get_nft_metadata` returns NFTs info with the most recent owner.
/// **Dont** use this function to get specific info about owner address, amount etc, you will get info not related to my_address.
pub async fn get_nft_metadata(_ctx: MmArc, req: NftMetadataReq) -> MmResult<Nft, GetNftInfoError> {
    let chain_str = match req.chain {
        Chain::Avalanche => "avalanche",
        Chain::Bsc => "bsc",
        Chain::Eth => "eth",
        Chain::Fantom => "fantom",
        Chain::Polygon => "polygon",
    };
    let req_url = &req.url;
    let token_address = &req.token_address;
    let token_id = &req.token_id;
    let uri = format!(
        "{req_url}{MORALIS_API_ENDPOINT}nft/{token_address}/{token_id}?chain={chain_str}&{FORMAT_DECIMAL_MORALIS}"
    );
    let response = send_moralis_request(uri.as_str()).await?;
    let nft_wrapper: NftWrapper = serde_json::from_str(&response.to_string())?;
    let nft_metadata = Nft {
        chain: req.chain,
        token_address: nft_wrapper.token_address,
        token_id: nft_wrapper.token_id.0,
        amount: nft_wrapper.amount.0,
        owner_of: nft_wrapper.owner_of,
        token_hash: nft_wrapper.token_hash,
        block_number_minted: *nft_wrapper.block_number_minted,
        block_number: *nft_wrapper.block_number,
        contract_type: nft_wrapper.contract_type.map(|v| v.0),
        name: nft_wrapper.name,
        symbol: nft_wrapper.symbol,
        token_uri: nft_wrapper.token_uri,
        metadata: nft_wrapper.metadata,
        last_token_uri_sync: nft_wrapper.last_token_uri_sync,
        last_metadata_sync: nft_wrapper.last_metadata_sync,
        minter_address: nft_wrapper.minter_address,
        possible_spam: nft_wrapper.possible_spam,
    };
    Ok(nft_metadata)
}

/// `get_nft_transfers` function returns a transfer history of NFTs on requested chains owned by user.
/// Currently doesnt support filters.
pub async fn get_nft_transfers(ctx: MmArc, req: NftTransfersReq) -> MmResult<NftsTransferHistoryList, GetNftInfoError> {
    let mut res_list = Vec::new();

    for chain in req.chains {
        let (coin_str, chain_str) = match chain {
            Chain::Avalanche => ("AVAX", "avalanche"),
            Chain::Bsc => ("BNB", "bsc"),
            Chain::Eth => ("ETH", "eth"),
            Chain::Fantom => ("FTM", "fantom"),
            Chain::Polygon => ("MATIC", "polygon"),
        };
        let my_address = get_eth_address(&ctx, coin_str).await?;
        let req_url = &req.url;
        let wallet_address = my_address.wallet_address;
        let uri_without_cursor = format!(
            "{req_url}{MORALIS_API_ENDPOINT}{wallet_address}/nft/transfers?chain={chain_str}&{FORMAT_DECIMAL_MORALIS}&{DIRECTION_BOTH_MORALIS}",

        );

        // The cursor returned in the previous response (used for getting the next page).
        let mut cursor = String::new();
        loop {
            let uri = format!("{}{}", uri_without_cursor, cursor);
            let response = send_moralis_request(uri.as_str()).await?;
            if let Some(transfer_list) = response["result"].as_array() {
                for transfer in transfer_list {
                    let transfer_wrapper: NftTransferHistoryWrapper = serde_json::from_str(&transfer.to_string())?;
                    let transfer_history = NftTransferHistory {
                        chain,
                        block_number: *transfer_wrapper.block_number,
                        block_timestamp: transfer_wrapper.block_timestamp,
                        block_hash: transfer_wrapper.block_hash,
                        transaction_hash: transfer_wrapper.transaction_hash,
                        transaction_index: transfer_wrapper.transaction_index,
                        log_index: transfer_wrapper.log_index,
                        value: transfer_wrapper.value.0,
                        contract_type: transfer_wrapper.contract_type.0,
                        transaction_type: transfer_wrapper.transaction_type,
                        token_address: transfer_wrapper.token_address,
                        token_id: transfer_wrapper.token_id.0,
                        from_address: transfer_wrapper.from_address,
                        to_address: transfer_wrapper.to_address,
                        amount: transfer_wrapper.amount.0,
                        verified: transfer_wrapper.verified,
                        operator: transfer_wrapper.operator,
                        possible_spam: transfer_wrapper.possible_spam,
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
    }
    drop_mutability!(res_list);
    let transfer_history_list = NftsTransferHistoryList {
        transfer_history: res_list,
    };
    Ok(transfer_history_list)
}

/// `withdraw_nft` function generates, signs and returns a transaction that transfers NFT
/// from my address to recipient's address.
/// This method generates a raw transaction which should then be broadcast using `send_raw_transaction`.
pub async fn withdraw_nft(ctx: MmArc, req: WithdrawNftReq) -> WithdrawNftResult {
    match req.withdraw_type {
        WithdrawNftType::WithdrawErc1155(erc1155_withdraw) => withdraw_erc1155(ctx, erc1155_withdraw, req.url).await,
        WithdrawNftType::WithdrawErc721(erc721_withdraw) => withdraw_erc721(ctx, erc721_withdraw).await,
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn send_moralis_request(uri: &str) -> MmResult<Json, GetNftInfoError> {
    use http::header::HeaderValue;
    use mm2_net::transport::slurp_req_body;

    let request = http::Request::builder()
        .method("GET")
        .uri(uri)
        .header(ACCEPT, HeaderValue::from_static(APPLICATION_JSON))
        .body(hyper::Body::from(""))?;

    let (status, _header, body) = slurp_req_body(request).await?;
    if !status.is_success() {
        return Err(MmError::new(GetNftInfoError::Transport(format!(
            "Response !200 from {}: {}, {}",
            uri, status, body
        ))));
    }
    Ok(body)
}

#[cfg(target_arch = "wasm32")]
async fn send_moralis_request(uri: &str) -> MmResult<Json, GetNftInfoError> {
    use mm2_net::wasm_http::FetchRequest;

    macro_rules! try_or {
        ($exp:expr, $errtype:ident) => {
            match $exp {
                Ok(x) => x,
                Err(e) => return Err(MmError::new(GetNftInfoError::$errtype(ERRL!("{:?}", e)))),
            }
        };
    }

    let result = FetchRequest::get(uri)
        .cors()
        .body_utf8("".to_owned())
        .header(ACCEPT.as_str(), APPLICATION_JSON)
        .request_str()
        .await;
    let (status_code, response_str) = try_or!(result, Transport);
    if !status_code.is_success() {
        return Err(MmError::new(GetNftInfoError::Transport(ERRL!(
            "!200: {}, {}",
            status_code,
            response_str
        ))));
    }

    let response: Json = try_or!(serde_json::from_str(&response_str), InvalidResponse);
    Ok(response)
}

/// This function uses `get_nft_list` method to get the correct info about amount of specific NFT owned by my_address.
pub(crate) async fn find_wallet_amount(
    ctx: MmArc,
    nft_list: NftListReq,
    token_address_req: String,
    token_id_req: BigDecimal,
) -> MmResult<BigDecimal, GetNftInfoError> {
    let nft_list = get_nft_list(ctx, nft_list).await?.nfts;
    let nft = nft_list
        .into_iter()
        .find(|nft| nft.token_address == token_address_req && nft.token_id == token_id_req)
        .ok_or_else(|| GetNftInfoError::TokenNotFoundInWallet {
            token_address: token_address_req,
            token_id: token_id_req.to_string(),
        })?;
    Ok(nft.amount)
}
