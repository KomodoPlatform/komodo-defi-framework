use crate::eth::erc20::{get_erc20_token_info, Erc20CustomTokenInfo};
use crate::eth::valid_addr_from_str;
use crate::{lp_coinfind_or_err, CoinFindError, MmCoinEnum};
use common::HttpStatusCode;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;

// Todo: Instead of `contract_address` we should make this a generic field or an enum
#[derive(Deserialize)]
pub struct CustomTokenInfoRequest {
    // Todo: maybe use protocol as request instead.
    platform_coin: String,
    contract_address: String,
}

// Todo: Add balance to a new struct that includes the token info
#[derive(Serialize)]
#[serde(tag = "type", content = "info")]
pub enum CustomTokenInfoResponse {
    ERC20(Erc20CustomTokenInfo),
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum CustomTokenInfoError {
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(fmt = "Unsupported protocol {}", protocol)]
    UnsupportedProtocol { protocol: String },
    #[display(fmt = "Invalid request {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Error retrieving token info {}", _0)]
    RetrieveInfoError(String),
}

impl HttpStatusCode for CustomTokenInfoError {
    fn status_code(&self) -> StatusCode {
        match self {
            CustomTokenInfoError::NoSuchCoin { .. } => StatusCode::NOT_FOUND,
            CustomTokenInfoError::UnsupportedProtocol { .. } | CustomTokenInfoError::InvalidRequest(_) => {
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
    let platform_coin = lp_coinfind_or_err(&ctx, &req.platform_coin).await?;
    match platform_coin {
        MmCoinEnum::EthCoin(eth_coin) => {
            // Todo: worth considering implementing serialize and deserialize for Address
            let contract_address = valid_addr_from_str(&req.contract_address).map_to_mm(|e| {
                let error = format!("Invalid contract address: {}", e);
                CustomTokenInfoError::InvalidRequest(error)
            })?;
            let token_info = get_erc20_token_info(&eth_coin, contract_address)
                .await
                .map_to_mm(CustomTokenInfoError::RetrieveInfoError)?;
            Ok(CustomTokenInfoResponse::ERC20(token_info))
        },
        _ => MmError::err(CustomTokenInfoError::UnsupportedProtocol {
            protocol: req.platform_coin,
        }),
    }
}
