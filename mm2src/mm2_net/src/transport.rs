use common::jsonrpc_client::JsonRpcErrorType;
use derive_more::Display;
use ethkey::Secret;
use http::{HeaderMap, StatusCode};
use mm2_err_handle::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Error, Value as Json};

#[cfg(not(target_arch = "wasm32"))]
pub use crate::native_http::{slurp_post_json, slurp_req, slurp_req_body, slurp_url, slurp_url_with_headers};
#[cfg(target_arch = "wasm32")]
pub use crate::wasm_http::{slurp_post_json, slurp_url, slurp_url_with_headers};

pub type SlurpResult = Result<(StatusCode, HeaderMap, Vec<u8>), MmError<SlurpError>>;

pub type SlurpResultJson = Result<(StatusCode, HeaderMap, Json), MmError<SlurpError>>;

#[derive(Debug, Deserialize, Display, Serialize)]
pub enum SlurpError {
    #[display(fmt = "Error deserializing '{}' response: {}", uri, error)]
    ErrorDeserializing { uri: String, error: String },
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Request '{}' timeout: {}", uri, error)]
    Timeout { uri: String, error: String },
    #[display(fmt = "Transport '{}' error: {}", uri, error)]
    Transport { uri: String, error: String },
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl From<serde_json::Error> for SlurpError {
    fn from(e: Error) -> Self { SlurpError::Internal(e.to_string()) }
}

impl From<SlurpError> for JsonRpcErrorType {
    fn from(err: SlurpError) -> Self {
        match err {
            SlurpError::InvalidRequest(err) => Self::InvalidRequest(err),
            SlurpError::Transport { .. } | SlurpError::Timeout { .. } => Self::Transport(err.to_string()),
            SlurpError::ErrorDeserializing { uri, error } => Self::Parse(uri.into(), error),
            SlurpError::Internal(_) => Self::Internal(err.to_string()),
        }
    }
}

/// Send POST JSON HTTPS request and parse response
pub async fn post_json<T>(url: &str, json: String) -> Result<T, MmError<SlurpError>>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    let result = slurp_post_json(url, json).await?;
    serde_json::from_slice(&result.2).map_to_mm(|e| SlurpError::ErrorDeserializing {
        uri: url.to_owned(),
        error: e.to_string(),
    })
}

/// Fetch URL by HTTPS and parse JSON response
pub async fn fetch_json<T>(url: &str) -> Result<T, MmError<SlurpError>>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    let result = slurp_url(url).await?;
    serde_json::from_slice(&result.2).map_to_mm(|e| SlurpError::ErrorDeserializing {
        uri: url.to_owned(),
        error: e.to_string(),
    })
}

#[derive(Clone, Debug)]
pub struct GuiAuthValidationGenerator {
    pub coin_ticker: String,
    pub secret: Secret,
    pub address: String,
}

/// gui-auth specific data-type that needed in order to perform gui-auth calls
#[derive(Serialize, Clone)]
pub struct GuiAuthValidation {
    pub coin_ticker: String,
    pub address: String,
    pub timestamp_message: i64,
    pub signature: String,
}
