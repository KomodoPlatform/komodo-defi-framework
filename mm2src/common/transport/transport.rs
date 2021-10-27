use crate::mm_error::prelude::*;
use derive_more::Display;
use http::{HeaderMap, StatusCode};

#[cfg(not(target_arch = "wasm32"))] pub mod native_http;
#[cfg(target_arch = "wasm32")] pub mod wasm_http;
#[cfg(target_arch = "wasm32")] pub mod wasm_ws;

#[cfg(not(target_arch = "wasm32"))]
pub use native_http::{slurp_req, slurp_url};

#[cfg(target_arch = "wasm32")] pub use wasm_http::slurp_url;

pub type SlurpResult = Result<(StatusCode, HeaderMap, Vec<u8>), MmError<SlurpError>>;

#[derive(Debug, Display)]
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
