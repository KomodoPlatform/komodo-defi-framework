use crate::utxo::rpc_clients::UtxoRpcError;
use crate::PrivKeyPolicyNotAllowed;
use common::executor::AbortedError;
use common::HttpStatusCode;
use db_common::sqlite::rusqlite::Error as SqlError;
use derive_more::Display;
use enum_derives::EnumFromStringify;
use http::StatusCode;
use mm2_err_handle::prelude::*;
use rpc_task::RpcTaskError;
use std::num::TryFromIntError;
use uuid::Uuid;

pub type EnableLightningResult<T> = Result<T, MmError<EnableLightningError>>;
pub type SaveChannelClosingResult<T> = Result<T, MmError<SaveChannelClosingError>>;

#[derive(Clone, Debug, Display, Serialize, SerializeErrorType, EnumFromStringify)]
#[serde(tag = "error_type", content = "error_data")]
pub enum EnableLightningError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Invalid configuration: {}", _0)]
    InvalidConfiguration(String),
    #[display(fmt = "{} is only supported in {} mode", _0, _1)]
    UnsupportedMode(String, String),
    #[display(fmt = "I/O error {}", _0)]
    #[from_stringify("std::io::Error")]
    IOError(String),
    #[display(fmt = "Invalid address: {}", _0)]
    InvalidAddress(String),
    #[display(fmt = "Invalid path: {}", _0)]
    InvalidPath(String),
    #[display(fmt = "Private key policy is not allowed: {}", _0)]
    #[from_stringify("PrivKeyPolicyNotAllowed")]
    PrivKeyPolicyNotAllowed(PrivKeyPolicyNotAllowed),
    #[display(fmt = "System time error {}", _0)]
    SystemTimeError(String),
    #[display(fmt = "RPC error {}", _0)]
    #[from_stringify("UtxoRpcError")]
    RpcError(String),
    #[display(fmt = "DB error {}", _0)]
    #[from_stringify("SqlError")]
    DbError(String),
    #[display(fmt = "Rpc task error: {}", _0)]
    #[from_stringify("RpcTaskError")]
    RpcTaskError(String),
    ConnectToNodeError(String),
    #[display(fmt = "Internal error: {}", _0)]
    #[from_stringify("AbortedError")]
    Internal(String),
}

impl HttpStatusCode for EnableLightningError {
    fn status_code(&self) -> StatusCode {
        match self {
            EnableLightningError::InvalidRequest(_)
            | EnableLightningError::RpcError(_)
            | EnableLightningError::PrivKeyPolicyNotAllowed(_) => StatusCode::BAD_REQUEST,
            EnableLightningError::UnsupportedMode(_, _) => StatusCode::NOT_IMPLEMENTED,
            EnableLightningError::InvalidAddress(_)
            | EnableLightningError::InvalidPath(_)
            | EnableLightningError::SystemTimeError(_)
            | EnableLightningError::IOError(_)
            | EnableLightningError::ConnectToNodeError(_)
            | EnableLightningError::InvalidConfiguration(_)
            | EnableLightningError::DbError(_)
            | EnableLightningError::RpcTaskError(_)
            | EnableLightningError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Display, PartialEq, EnumFromStringify)]
pub enum SaveChannelClosingError {
    #[display(fmt = "DB error: {}", _0)]
    #[from_stringify("SqlError")]
    DbError(String),
    #[display(fmt = "Channel with uuid {} not found in DB", _0)]
    ChannelNotFound(Uuid),
    #[display(fmt = "Funding transaction hash is Null in DB")]
    FundingTxNull,
    #[display(fmt = "Error parsing funding transaction hash: {}", _0)]
    FundingTxParseError(String),
    #[display(fmt = "Error while waiting for the funding transaction to be spent: {}", _0)]
    WaitForFundingTxSpendError(String),
    #[display(fmt = "Error while converting types: {}", _0)]
    #[from_stringify("TryFromIntError")]
    ConversionError(TryFromIntError),
}
