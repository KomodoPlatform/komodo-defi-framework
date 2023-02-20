use crate::eth::GetEthAddressError;
use common::HttpStatusCode;
use derive_more::Display;
use http::StatusCode;
use mm2_net::transport::SlurpError;
use serde::{Deserialize, Serialize};
use web3::Error;

#[derive(Debug, Display, Serialize, SerializeErrorType, Deserialize)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetNftInfoError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Transport: {}", _0)]
    Transport(String),
    #[display(fmt = "Invalid response: {}", _0)]
    InvalidResponse(String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
    GetEthAddressError(GetEthAddressError),
    #[display(fmt = "X-API-Key is missing")]
    ApiKeyError,
}

/// `http::Error` can appear on an HTTP request [`http::Builder::build`] building.
impl From<http::Error> for GetNftInfoError {
    fn from(e: http::Error) -> Self { GetNftInfoError::InvalidRequest(e.to_string()) }
}

impl From<SlurpError> for GetNftInfoError {
    fn from(e: SlurpError) -> Self {
        let error_str = e.to_string();
        match e {
            SlurpError::ErrorDeserializing { .. } => GetNftInfoError::InvalidResponse(error_str),
            SlurpError::Transport { .. } | SlurpError::Timeout { .. } => GetNftInfoError::Transport(error_str),
            SlurpError::Internal(_) | SlurpError::InvalidRequest(_) => GetNftInfoError::Internal(error_str),
        }
    }
}

impl From<web3::Error> for GetNftInfoError {
    fn from(e: Error) -> Self {
        let error_str = e.to_string();
        match e.kind() {
            web3::ErrorKind::InvalidResponse(_)
            | web3::ErrorKind::Decoder(_)
            | web3::ErrorKind::Msg(_)
            | web3::ErrorKind::Rpc(_) => GetNftInfoError::InvalidResponse(error_str),
            web3::ErrorKind::Transport(_) | web3::ErrorKind::Io(_) => GetNftInfoError::Transport(error_str),
            _ => GetNftInfoError::Internal(error_str),
        }
    }
}

impl From<GetEthAddressError> for GetNftInfoError {
    fn from(e: GetEthAddressError) -> Self { GetNftInfoError::GetEthAddressError(e) }
}

impl From<serde_json::Error> for GetNftInfoError {
    fn from(e: serde_json::Error) -> Self { GetNftInfoError::InvalidResponse(e.to_string()) }
}

impl HttpStatusCode for GetNftInfoError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetNftInfoError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            GetNftInfoError::InvalidResponse(_) => StatusCode::FAILED_DEPENDENCY,
            GetNftInfoError::ApiKeyError => StatusCode::FORBIDDEN,
            GetNftInfoError::Transport(_) | GetNftInfoError::Internal(_) | GetNftInfoError::GetEthAddressError(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
        }
    }
}
