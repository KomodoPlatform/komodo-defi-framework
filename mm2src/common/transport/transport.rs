use crate::mm_error::prelude::*;
use derive_more::Display;
use http::{HeaderMap, StatusCode};

#[cfg(not(target_arch = "wasm32"))] pub mod native_http;
#[cfg(target_arch = "wasm32")] pub mod wasm_http;
#[cfg(target_arch = "wasm32")] pub mod wasm_ws;

#[cfg(not(target_arch = "wasm32"))]
pub use native_http::{slurp_post_json, slurp_req, slurp_url};

#[cfg(target_arch = "wasm32")]
pub use wasm_http::{slurp_post_json, slurp_url};

pub type SlurpResult = Result<(StatusCode, HeaderMap, Vec<u8>), MmError<SlurpError>>;

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
