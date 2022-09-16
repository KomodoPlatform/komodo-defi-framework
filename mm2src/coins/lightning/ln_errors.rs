use crate::utxo::rpc_clients::UtxoRpcError;
use crate::CoinFindError;
use common::HttpStatusCode;
use db_common::sqlite::rusqlite::Error as SqlError;
use derive_more::Display;
use http::StatusCode;
use lightning_invoice::SignOrCreationError;
use mm2_err_handle::prelude::*;
use std::num::TryFromIntError;

pub type EnableLightningResult<T> = Result<T, MmError<EnableLightningError>>;
pub type GenerateInvoiceResult<T> = Result<T, MmError<GenerateInvoiceError>>;
pub type SendPaymentResult<T> = Result<T, MmError<SendPaymentError>>;
pub type SaveChannelClosingResult<T> = Result<T, MmError<SaveChannelClosingError>>;

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
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
            | EnableLightningError::DbError(_) => StatusCode::INTERNAL_SERVER_ERROR,
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

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GenerateInvoiceError {
    #[display(fmt = "Lightning network is not supported for {}", _0)]
    UnsupportedCoin(String),
    #[display(fmt = "No such coin {}", _0)]
    NoSuchCoin(String),
    #[display(fmt = "Invoice signing or creation error: {}", _0)]
    SignOrCreationError(String),
    #[display(fmt = "DB error {}", _0)]
    DbError(String),
}

impl HttpStatusCode for GenerateInvoiceError {
    fn status_code(&self) -> StatusCode {
        match self {
            GenerateInvoiceError::UnsupportedCoin(_) => StatusCode::BAD_REQUEST,
            GenerateInvoiceError::NoSuchCoin(_) => StatusCode::NOT_FOUND,
            GenerateInvoiceError::SignOrCreationError(_) | GenerateInvoiceError::DbError(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
        }
    }
}

impl From<CoinFindError> for GenerateInvoiceError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => GenerateInvoiceError::NoSuchCoin(coin),
        }
    }
}

impl From<SignOrCreationError> for GenerateInvoiceError {
    fn from(e: SignOrCreationError) -> Self { GenerateInvoiceError::SignOrCreationError(e.to_string()) }
}

impl From<SqlError> for GenerateInvoiceError {
    fn from(err: SqlError) -> GenerateInvoiceError { GenerateInvoiceError::DbError(err.to_string()) }
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum SendPaymentError {
    #[display(fmt = "Lightning network is not supported for {}", _0)]
    UnsupportedCoin(String),
    #[display(fmt = "No such coin {}", _0)]
    NoSuchCoin(String),
    #[display(fmt = "Couldn't parse destination pubkey: {}", _0)]
    NoRouteFound(String),
    #[display(fmt = "Payment error: {}", _0)]
    PaymentError(String),
    #[display(fmt = "Final cltv expiry delta {} is below the required minimum of {}", _0, _1)]
    CLTVExpiryError(u32, u32),
    #[display(fmt = "DB error {}", _0)]
    DbError(String),
}

impl HttpStatusCode for SendPaymentError {
    fn status_code(&self) -> StatusCode {
        match self {
            SendPaymentError::UnsupportedCoin(_) => StatusCode::BAD_REQUEST,
            SendPaymentError::NoSuchCoin(_) => StatusCode::NOT_FOUND,
            SendPaymentError::PaymentError(_)
            | SendPaymentError::NoRouteFound(_)
            | SendPaymentError::CLTVExpiryError(_, _)
            | SendPaymentError::DbError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for SendPaymentError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => SendPaymentError::NoSuchCoin(coin),
        }
    }
}

impl From<SqlError> for SendPaymentError {
    fn from(err: SqlError) -> SendPaymentError { SendPaymentError::DbError(err.to_string()) }
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
