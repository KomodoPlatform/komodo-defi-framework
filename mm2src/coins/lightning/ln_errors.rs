use crate::utxo::rpc_clients::UtxoRpcError;
use common::HttpStatusCode;
use db_common::sqlite::rusqlite::Error as SqlError;
use derive_more::Display;
use http::StatusCode;
use mm2_err_handle::prelude::*;
use rpc_task::RpcTaskError;
use std::num::TryFromIntError;

pub type EnableLightningResult<T> = Result<T, MmError<EnableLightningError>>;
pub type SaveChannelClosingResult<T> = Result<T, MmError<SaveChannelClosingError>>;

#[derive(Clone, Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum EnableLightningError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Invalid configuration: {}", _0)]
    InvalidConfiguration(String),
    #[display(fmt = "{} is only supported in {} mode", _0, _1)]
    UnsupportedMode(String, String),
    #[display(fmt = "I/O error {}", _0)]
    IOError(String),
    #[display(fmt = "Invalid address: {}", _0)]
    InvalidAddress(String),
    #[display(fmt = "Invalid path: {}", _0)]
    InvalidPath(String),
    #[display(fmt = "System time error {}", _0)]
    SystemTimeError(String),
    #[display(fmt = "RPC error {}", _0)]
    RpcError(String),
    #[display(fmt = "DB error {}", _0)]
    DbError(String),
    #[display(fmt = "Rpc task error: {}", _0)]
    RpcTaskError(String),
    ConnectToNodeError(String),
}

impl HttpStatusCode for EnableLightningError {
    fn status_code(&self) -> StatusCode {
        match self {
            EnableLightningError::InvalidRequest(_) | EnableLightningError::RpcError(_) => StatusCode::BAD_REQUEST,
            EnableLightningError::UnsupportedMode(_, _) => StatusCode::NOT_IMPLEMENTED,
            EnableLightningError::InvalidAddress(_)
            | EnableLightningError::InvalidPath(_)
            | EnableLightningError::SystemTimeError(_)
            | EnableLightningError::IOError(_)
            | EnableLightningError::ConnectToNodeError(_)
            | EnableLightningError::InvalidConfiguration(_)
            | EnableLightningError::DbError(_)
            | EnableLightningError::RpcTaskError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<std::io::Error> for EnableLightningError {
    fn from(err: std::io::Error) -> EnableLightningError { EnableLightningError::IOError(err.to_string()) }
}

impl From<SqlError> for EnableLightningError {
    fn from(err: SqlError) -> EnableLightningError { EnableLightningError::DbError(err.to_string()) }
}

impl From<UtxoRpcError> for EnableLightningError {
    fn from(e: UtxoRpcError) -> Self { EnableLightningError::RpcError(e.to_string()) }
}

impl From<RpcTaskError> for EnableLightningError {
    fn from(e: RpcTaskError) -> Self { EnableLightningError::RpcTaskError(e.to_string()) }
}

#[derive(Display, PartialEq)]
pub enum SaveChannelClosingError {
    #[display(fmt = "DB error: {}", _0)]
    DbError(String),
    #[display(fmt = "Channel with rpc id {} not found in DB", _0)]
    ChannelNotFound(u64),
    #[display(fmt = "funding_generated_in_block is Null in DB")]
    BlockHeightNull,
    #[display(fmt = "Funding transaction hash is Null in DB")]
    FundingTxNull,
    #[display(fmt = "Error parsing funding transaction hash: {}", _0)]
    FundingTxParseError(String),
    #[display(fmt = "Error while waiting for the funding transaction to be spent: {}", _0)]
    WaitForFundingTxSpendError(String),
    #[display(fmt = "Error while converting types: {}", _0)]
    ConversionError(TryFromIntError),
}

impl From<SqlError> for SaveChannelClosingError {
    fn from(err: SqlError) -> SaveChannelClosingError { SaveChannelClosingError::DbError(err.to_string()) }
}

impl From<TryFromIntError> for SaveChannelClosingError {
    fn from(err: TryFromIntError) -> SaveChannelClosingError { SaveChannelClosingError::ConversionError(err) }
}
