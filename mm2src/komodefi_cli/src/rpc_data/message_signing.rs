use derive_more::Display;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub(crate) struct SignatureRequest {
    pub(crate) coin: String,
    pub(crate) message: String,
}

#[derive(Deserialize)]
pub(crate) struct SignatureResponse {
    pub(crate) signature: String,
}

#[derive(Display, Deserialize)]
#[serde(tag = "error_type", content = "error_data")]
pub(crate) enum SignatureError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
    #[display(fmt = "Coin is not found: {}", _0)]
    CoinIsNotFound(String),
    #[display(fmt = "sign_message_prefix is not set in coin config")]
    PrefixNotFound,
}

#[derive(Serialize)]
pub(crate) struct VerificationRequest {
    pub(crate) coin: String,
    pub(crate) message: String,
    pub(crate) signature: String,
    pub(crate) address: String,
}

#[derive(Deserialize)]
pub(crate) struct VerificationResponse {
    pub(crate) is_valid: bool,
}

#[derive(Display, Deserialize)]
#[serde(tag = "error_type", content = "error_data")]
pub(crate) enum VerificationError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
    #[display(fmt = "Signature decoding error: {}", _0)]
    SignatureDecodingError(String),
    #[display(fmt = "Address decoding error: {}", _0)]
    AddressDecodingError(String),
    #[display(fmt = "Coin is not found: {}", _0)]
    CoinIsNotFound(String),
    #[display(fmt = "sign_message_prefix is not set in coin config")]
    PrefixNotFound,
}
