mod delete_connection;
mod get_chain_id;
mod new_connection;
mod ping;
mod sessions;

use common::HttpStatusCode;
pub use delete_connection::delete_connection;
use derive_more::Display;
pub use get_chain_id::get_chain_id;
use http::StatusCode;
pub use new_connection::new_connection;
pub use ping::ping_session;
use serde::Deserialize;
pub use sessions::*;

#[derive(Deserialize)]
pub struct EmptyRpcRequst {}

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