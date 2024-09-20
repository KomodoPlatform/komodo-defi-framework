use coins::{eth::u256_to_big_decimal, NumConversError};
use common::{HttpStatusCode, StatusCode};
use enum_derives::EnumFromStringify;
use mm2_number::BigDecimal;
use ser_error_derive::SerializeErrorType;
use serde::Serialize;
use trading_api::one_inch_api::errors::ApiClientError;

#[derive(Debug, Display, Serialize, SerializeErrorType, EnumFromStringify)]
#[serde(tag = "error_type", content = "error_data")]
pub enum ApiIntegrationRpcError {
    #[from_stringify("coins::CoinFindError")]
    NoSuchCoin(String),
    #[display(fmt = "EVM token needed")]
    CoinTypeError,
    #[display(fmt = "NFT not supported")]
    NftNotSupported,
    #[display(fmt = "Chain not supported")]
    ChainNotSupported,
    #[from_stringify("coins::UnexpectedDerivationMethod")]
    MyAddressError(String),
    InvalidParam(String),
    #[display(fmt = "allowance not enough for 1inch contract, available: {allowance}, needed: {amount}")]
    OneInchAllowanceNotEnough {
        allowance: BigDecimal,
        amount: BigDecimal,
    },
    #[display(fmt = "1inch API error: {}", _0)]
    OneInchError(ApiClientError),
    ApiDataError(String),
}

impl HttpStatusCode for ApiIntegrationRpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            ApiIntegrationRpcError::NoSuchCoin { .. } => StatusCode::NOT_FOUND,
            ApiIntegrationRpcError::CoinTypeError
            | ApiIntegrationRpcError::NftNotSupported
            | ApiIntegrationRpcError::ChainNotSupported
            | ApiIntegrationRpcError::MyAddressError(_)
            | ApiIntegrationRpcError::InvalidParam(_)
            | ApiIntegrationRpcError::OneInchAllowanceNotEnough { .. } => StatusCode::BAD_REQUEST,
            ApiIntegrationRpcError::OneInchError(_) | ApiIntegrationRpcError::ApiDataError(_) => {
                StatusCode::BAD_GATEWAY
            },
        }
    }
}

impl ApiIntegrationRpcError {
    pub(crate) fn from_api_error(error: ApiClientError, decimals: u8) -> Self {
        match error {
            ApiClientError::InvalidParam(error) => ApiIntegrationRpcError::InvalidParam(error),
            ApiClientError::HttpClientError(_)
            | ApiClientError::ParseBodyError(_)
            | ApiClientError::GeneralApiError(_) => ApiIntegrationRpcError::OneInchError(error),
            ApiClientError::AllowanceNotEnough(nested_err) => ApiIntegrationRpcError::OneInchAllowanceNotEnough {
                allowance: u256_to_big_decimal(nested_err.allowance, decimals).unwrap_or_default(),
                amount: u256_to_big_decimal(nested_err.amount, decimals).unwrap_or_default(),
            },
        }
    }
}

/// Error aggregator for errors of conversion of api returned values
#[derive(Debug, Display, Serialize)]
pub(crate) struct FromApiValueError(String);

impl From<NumConversError> for FromApiValueError {
    fn from(err: NumConversError) -> Self { Self(err.to_string()) }
}

impl From<primitive_types::Error> for FromApiValueError {
    fn from(err: primitive_types::Error) -> Self { Self(format!("{:?}", err)) }
}

impl From<hex::FromHexError> for FromApiValueError {
    fn from(err: hex::FromHexError) -> Self { Self(err.to_string()) }
}

impl From<ethereum_types::FromDecStrErr> for FromApiValueError {
    fn from(err: ethereum_types::FromDecStrErr) -> Self { Self(err.to_string()) }
}
