use crate::eth::erc20::{get_erc20_ticker_by_contract_address, get_erc20_token_info, Erc20CustomTokenInfo};
use crate::eth::valid_addr_from_str;
use crate::{lp_coinfind_or_err, CoinFindError, CoinProtocol, MmCoinEnum};
use common::HttpStatusCode;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;

#[derive(Deserialize)]
pub struct CustomTokenInfoRequest {
    protocol: CoinProtocol,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "info")]
pub enum CustomTokenInfo {
    ERC20(Erc20CustomTokenInfo),
}

#[derive(Serialize)]
pub struct CustomTokenInfoResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    config_ticker: Option<String>,
    #[serde(flatten)]
    info: CustomTokenInfo,
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum CustomTokenInfoError {
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(fmt = "Custom tokens are not supported for {} protocol yet!", protocol)]
    UnsupportedTokenProtocol { protocol: String },
    #[display(fmt = "Invalid request {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Error retrieving token info {}", _0)]
    RetrieveInfoError(String),
}

impl HttpStatusCode for CustomTokenInfoError {
    fn status_code(&self) -> StatusCode {
        match self {
            CustomTokenInfoError::NoSuchCoin { .. } => StatusCode::NOT_FOUND,
            CustomTokenInfoError::UnsupportedTokenProtocol { .. } | CustomTokenInfoError::InvalidRequest(_) => {
                StatusCode::BAD_REQUEST
            },
            CustomTokenInfoError::RetrieveInfoError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for CustomTokenInfoError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => CustomTokenInfoError::NoSuchCoin { coin },
        }
    }
}

pub async fn get_custom_token_info(
    ctx: MmArc,
    req: CustomTokenInfoRequest,
) -> MmResult<CustomTokenInfoResponse, CustomTokenInfoError> {
    // Check that the protocol is a token protocol
    let platform = req
        .protocol
        .platform()
        .ok_or(CustomTokenInfoError::InvalidRequest(format!(
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
                    .ok_or(CustomTokenInfoError::UnsupportedTokenProtocol {
                        protocol: platform.to_string(),
                    })?;
            let contract_address = valid_addr_from_str(contract_address_str).map_to_mm(|e| {
                let error = format!("Invalid contract address: {}", e);
                CustomTokenInfoError::InvalidRequest(error)
            })?;

            let config_ticker = get_erc20_ticker_by_contract_address(&ctx, platform, contract_address_str);
            let token_info = get_erc20_token_info(&eth_coin, contract_address)
                .await
                .map_to_mm(CustomTokenInfoError::RetrieveInfoError)?;
            Ok(CustomTokenInfoResponse {
                config_ticker,
                info: CustomTokenInfo::ERC20(token_info),
            })
        },
        _ => MmError::err(CustomTokenInfoError::UnsupportedTokenProtocol {
            protocol: platform.to_string(),
        }),
    }
}
