mod new_connection;
mod sessions;

use common::HttpStatusCode;
use derive_more::Display;
use http::StatusCode;
pub use new_connection::new_connection;
use serde::Deserialize;
pub use sessions::*;

#[derive(Deserialize)]
pub struct EmptyRpcRequest {}

#[derive(Debug, Serialize)]
pub struct EmptyRpcResponse {}

#[derive(Serialize, Display, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum WalletConnectRpcError {
    InternalError(String),
    InitializationError(String),
    SessionRequestError(String),
}

impl HttpStatusCode for WalletConnectRpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            WalletConnectRpcError::InitializationError(_) => StatusCode::BAD_REQUEST,
            WalletConnectRpcError::SessionRequestError(_) | WalletConnectRpcError::InternalError(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
        }
    }
}
