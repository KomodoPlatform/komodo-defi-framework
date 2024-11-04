use coins::eth::erc20::{get_erc20_ticker_by_contract_address, get_erc20_token_info, Erc20TokenBasicInfo};
use coins::eth::valid_addr_from_str;
use coins::{lp_coinfind_or_err, CoinFindError, CoinProtocol, MmCoinEnum};
use common::HttpStatusCode;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;

#[derive(Deserialize)]
pub struct TokenInfoRequest {
    protocol: CoinProtocol,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "info")]
pub enum TokenInfo {
    ERC20(Erc20TokenBasicInfo),
}

#[derive(Serialize)]
pub struct TokenInfoResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    config_ticker: Option<String>,
    #[serde(flatten)]
    info: TokenInfo,
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum TokenInfoError {
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(fmt = "Custom tokens are not supported for {} protocol yet!", protocol)]
    UnsupportedTokenProtocol { protocol: String },
    #[display(fmt = "Invalid request {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Error retrieving token info {}", _0)]
    RetrieveInfoError(String),
}

impl HttpStatusCode for TokenInfoError {
    fn status_code(&self) -> StatusCode {
        match self {
            TokenInfoError::NoSuchCoin { .. } => StatusCode::NOT_FOUND,
            TokenInfoError::UnsupportedTokenProtocol { .. } | TokenInfoError::InvalidRequest(_) => {
                StatusCode::BAD_REQUEST
            },
            TokenInfoError::RetrieveInfoError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for TokenInfoError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => TokenInfoError::NoSuchCoin { coin },
        }
    }
}

pub async fn get_token_info(ctx: MmArc, req: TokenInfoRequest) -> MmResult<TokenInfoResponse, TokenInfoError> {
    // Check that the protocol is a token protocol
    let platform = req.protocol.platform().ok_or(TokenInfoError::InvalidRequest(format!(
        "Protocol '{:?}' is not a token protocol",
        req.protocol
    )))?;
    // Platform coin should be activated
    let platform_coin = lp_coinfind_or_err(&ctx, platform).await?;
    match platform_coin {
        MmCoinEnum::EthCoin(eth_coin) => {
            let contract_address_str =
                req.protocol
                    .contract_address()
                    .ok_or(TokenInfoError::UnsupportedTokenProtocol {
                        protocol: platform.to_string(),
                    })?;
            let contract_address = valid_addr_from_str(contract_address_str).map_to_mm(|e| {
                let error = format!("Invalid contract address: {}", e);
                TokenInfoError::InvalidRequest(error)
            })?;

            let config_ticker = get_erc20_ticker_by_contract_address(&ctx, platform, contract_address_str);
            let token_info = get_erc20_token_info(&eth_coin, contract_address)
                .await
                .map_to_mm(TokenInfoError::RetrieveInfoError)?;
            Ok(TokenInfoResponse {
                config_ticker,
                info: TokenInfo::ERC20(token_info),
            })
        },
        _ => MmError::err(TokenInfoError::UnsupportedTokenProtocol {
            protocol: platform.to_string(),
        }),
    }
}
