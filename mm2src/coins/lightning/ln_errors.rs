use crate::utxo::rpc_clients::UtxoRpcError;
use crate::CoinFindError;
use common::HttpStatusCode;
use db_common::sqlite::rusqlite::Error as SqlError;
use derive_more::Display;
use http::StatusCode;
use lightning_invoice::SignOrCreationError;
use mm2_err_handle::prelude::*;
use rpc::v1::types::H256 as H256Json;
use std::num::TryFromIntError;

pub type EnableLightningResult<T> = Result<T, MmError<EnableLightningError>>;
pub type UpdateChannelResult<T> = Result<T, MmError<UpdateChannelError>>;
pub type ListChannelsResult<T> = Result<T, MmError<ListChannelsError>>;
pub type GetChannelDetailsResult<T> = Result<T, MmError<GetChannelDetailsError>>;
pub type GenerateInvoiceResult<T> = Result<T, MmError<GenerateInvoiceError>>;
pub type SendPaymentResult<T> = Result<T, MmError<SendPaymentError>>;
pub type ListPaymentsResult<T> = Result<T, MmError<ListPaymentsError>>;
pub type GetPaymentDetailsResult<T> = Result<T, MmError<GetPaymentDetailsError>>;
pub type CloseChannelResult<T> = Result<T, MmError<CloseChannelError>>;
pub type ClaimableBalancesResult<T> = Result<T, MmError<ClaimableBalancesError>>;
pub type SaveChannelClosingResult<T> = Result<T, MmError<SaveChannelClosingError>>;
pub type TrustedNodeResult<T> = Result<T, MmError<TrustedNodeError>>;

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
pub enum UpdateChannelError {
    #[display(fmt = "Lightning network is not supported for {}", _0)]
    UnsupportedCoin(String),
    #[display(fmt = "No such coin {}", _0)]
    NoSuchCoin(String),
    #[display(fmt = "No such channel with rpc_channel_id {}", _0)]
    NoSuchChannel(u64),
    #[display(fmt = "Failure to channel {}: {}", _0, _1)]
    FailureToUpdateChannel(u64, String),
}

impl HttpStatusCode for UpdateChannelError {
    fn status_code(&self) -> StatusCode {
        match self {
            UpdateChannelError::UnsupportedCoin(_) => StatusCode::BAD_REQUEST,
            UpdateChannelError::NoSuchChannel(_) => StatusCode::NOT_FOUND,
            UpdateChannelError::NoSuchCoin(_) => StatusCode::NOT_FOUND,
            UpdateChannelError::FailureToUpdateChannel(_, _) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for UpdateChannelError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => UpdateChannelError::NoSuchCoin(coin),
        }
    }
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum ListChannelsError {
    #[display(fmt = "Lightning network is not supported for {}", _0)]
    UnsupportedCoin(String),
    #[display(fmt = "No such coin {}", _0)]
    NoSuchCoin(String),
    #[display(fmt = "DB error {}", _0)]
    DbError(String),
}

impl HttpStatusCode for ListChannelsError {
    fn status_code(&self) -> StatusCode {
        match self {
            ListChannelsError::UnsupportedCoin(_) => StatusCode::BAD_REQUEST,
            ListChannelsError::NoSuchCoin(_) => StatusCode::NOT_FOUND,
            ListChannelsError::DbError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for ListChannelsError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => ListChannelsError::NoSuchCoin(coin),
        }
    }
}

impl From<SqlError> for ListChannelsError {
    fn from(err: SqlError) -> ListChannelsError { ListChannelsError::DbError(err.to_string()) }
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetChannelDetailsError {
    #[display(fmt = "Lightning network is not supported for {}", _0)]
    UnsupportedCoin(String),
    #[display(fmt = "No such coin {}", _0)]
    NoSuchCoin(String),
    #[display(fmt = "Channel with rpc id: {} is not found", _0)]
    NoSuchChannel(u64),
    #[display(fmt = "DB error {}", _0)]
    DbError(String),
}

impl HttpStatusCode for GetChannelDetailsError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetChannelDetailsError::UnsupportedCoin(_) => StatusCode::BAD_REQUEST,
            GetChannelDetailsError::NoSuchCoin(_) | GetChannelDetailsError::NoSuchChannel(_) => StatusCode::NOT_FOUND,
            GetChannelDetailsError::DbError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for GetChannelDetailsError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => GetChannelDetailsError::NoSuchCoin(coin),
        }
    }
}

impl From<SqlError> for GetChannelDetailsError {
    fn from(err: SqlError) -> GetChannelDetailsError { GetChannelDetailsError::DbError(err.to_string()) }
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

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum ListPaymentsError {
    #[display(fmt = "Lightning network is not supported for {}", _0)]
    UnsupportedCoin(String),
    #[display(fmt = "No such coin {}", _0)]
    NoSuchCoin(String),
    #[display(fmt = "DB error {}", _0)]
    DbError(String),
}

impl HttpStatusCode for ListPaymentsError {
    fn status_code(&self) -> StatusCode {
        match self {
            ListPaymentsError::UnsupportedCoin(_) => StatusCode::BAD_REQUEST,
            ListPaymentsError::NoSuchCoin(_) => StatusCode::NOT_FOUND,
            ListPaymentsError::DbError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for ListPaymentsError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => ListPaymentsError::NoSuchCoin(coin),
        }
    }
}

impl From<SqlError> for ListPaymentsError {
    fn from(err: SqlError) -> ListPaymentsError { ListPaymentsError::DbError(err.to_string()) }
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetPaymentDetailsError {
    #[display(fmt = "Lightning network is not supported for {}", _0)]
    UnsupportedCoin(String),
    #[display(fmt = "No such coin {}", _0)]
    NoSuchCoin(String),
    #[display(fmt = "Payment with hash: {:?} is not found", _0)]
    NoSuchPayment(H256Json),
    #[display(fmt = "DB error {}", _0)]
    DbError(String),
}

impl HttpStatusCode for GetPaymentDetailsError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetPaymentDetailsError::UnsupportedCoin(_) => StatusCode::BAD_REQUEST,
            GetPaymentDetailsError::NoSuchCoin(_) => StatusCode::NOT_FOUND,
            GetPaymentDetailsError::NoSuchPayment(_) => StatusCode::NOT_FOUND,
            GetPaymentDetailsError::DbError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for GetPaymentDetailsError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => GetPaymentDetailsError::NoSuchCoin(coin),
        }
    }
}

impl From<SqlError> for GetPaymentDetailsError {
    fn from(err: SqlError) -> GetPaymentDetailsError { GetPaymentDetailsError::DbError(err.to_string()) }
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum CloseChannelError {
    #[display(fmt = "Lightning network is not supported for {}", _0)]
    UnsupportedCoin(String),
    #[display(fmt = "No such coin {}", _0)]
    NoSuchCoin(String),
    #[display(fmt = "No such channel with rpc_channel_id {}", _0)]
    NoSuchChannel(u64),
    #[display(fmt = "Closing channel error: {}", _0)]
    CloseChannelError(String),
}

impl HttpStatusCode for CloseChannelError {
    fn status_code(&self) -> StatusCode {
        match self {
            CloseChannelError::UnsupportedCoin(_) => StatusCode::BAD_REQUEST,
            CloseChannelError::NoSuchChannel(_) | CloseChannelError::NoSuchCoin(_) => StatusCode::NOT_FOUND,
            CloseChannelError::CloseChannelError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for CloseChannelError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => CloseChannelError::NoSuchCoin(coin),
        }
    }
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum ClaimableBalancesError {
    #[display(fmt = "Lightning network is not supported for {}", _0)]
    UnsupportedCoin(String),
    #[display(fmt = "No such coin {}", _0)]
    NoSuchCoin(String),
}

impl HttpStatusCode for ClaimableBalancesError {
    fn status_code(&self) -> StatusCode {
        match self {
            ClaimableBalancesError::UnsupportedCoin(_) => StatusCode::BAD_REQUEST,
            ClaimableBalancesError::NoSuchCoin(_) => StatusCode::NOT_FOUND,
        }
    }
}

impl From<CoinFindError> for ClaimableBalancesError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => ClaimableBalancesError::NoSuchCoin(coin),
        }
    }
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

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum TrustedNodeError {
    #[display(fmt = "Lightning network is not supported for {}", _0)]
    UnsupportedCoin(String),
    #[display(fmt = "No such coin {}", _0)]
    NoSuchCoin(String),
    #[display(fmt = "I/O error {}", _0)]
    IOError(String),
}

impl HttpStatusCode for TrustedNodeError {
    fn status_code(&self) -> StatusCode {
        match self {
            TrustedNodeError::UnsupportedCoin(_) => StatusCode::BAD_REQUEST,
            TrustedNodeError::NoSuchCoin(_) => StatusCode::NOT_FOUND,
            TrustedNodeError::IOError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for TrustedNodeError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => TrustedNodeError::NoSuchCoin(coin),
        }
    }
}

impl From<std::io::Error> for TrustedNodeError {
    fn from(err: std::io::Error) -> TrustedNodeError { TrustedNodeError::IOError(err.to_string()) }
}
