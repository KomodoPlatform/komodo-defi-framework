use crate::eth::GetEthAddressError;
use crate::nft::GetInfoFromUriError;
use common::HttpStatusCode;
use derive_more::Display;
use enum_from::EnumFromStringify;
use http::StatusCode;
use serde::{Deserialize, Serialize};
use web3::Error;

#[derive(Clone, Debug, Deserialize, Display, EnumFromStringify, PartialEq, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetNftInfoError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Transport: {}", _0)]
    Transport(String),
    #[from_stringify("serde_json::Error")]
    #[display(fmt = "Invalid response: {}", _0)]
    InvalidResponse(String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
    GetEthAddressError(GetEthAddressError),
    #[display(
        fmt = "Token: token_address {}, token_id {} was not find in wallet",
        token_address,
        token_id
    )]
    TokenNotFoundInWallet {
        token_address: String,
        token_id: String,
    },
}

impl From<web3::Error> for GetNftInfoError {
    fn from(e: Error) -> Self {
        let error_str = e.to_string();
        match e {
            web3::Error::InvalidResponse(_) | web3::Error::Decoder(_) | web3::Error::Rpc(_) => {
                GetNftInfoError::InvalidResponse(error_str)
            },
            web3::Error::Transport(_) | web3::Error::Io(_) => GetNftInfoError::Transport(error_str),
            _ => GetNftInfoError::Internal(error_str),
        }
    }
}

impl From<GetEthAddressError> for GetNftInfoError {
    fn from(e: GetEthAddressError) -> Self { GetNftInfoError::GetEthAddressError(e) }
}

impl From<GetInfoFromUriError> for GetNftInfoError {
    fn from(e: GetInfoFromUriError) -> Self {
        match e {
            GetInfoFromUriError::InvalidRequest(e) => GetNftInfoError::InvalidRequest(e),
            GetInfoFromUriError::Transport(e) => GetNftInfoError::Transport(e),
            GetInfoFromUriError::InvalidResponse(e) => GetNftInfoError::InvalidResponse(e),
            GetInfoFromUriError::Internal(e) => GetNftInfoError::Internal(e),
        }
    }
}

impl HttpStatusCode for GetNftInfoError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetNftInfoError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            GetNftInfoError::InvalidResponse(_) => StatusCode::FAILED_DEPENDENCY,
            GetNftInfoError::Transport(_)
            | GetNftInfoError::Internal(_)
            | GetNftInfoError::GetEthAddressError(_)
            | GetNftInfoError::TokenNotFoundInWallet { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
