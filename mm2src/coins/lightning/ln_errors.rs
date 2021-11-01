use crate::CoinFindError;
use common::mm_error::prelude::*;
use common::HttpStatusCode;
use derive_more::Display;
use http::StatusCode;

pub type EnableLightningResult<T> = Result<T, MmError<EnableLightningError>>;

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum EnableLightningError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Invalid address: {}", _0)]
    InvalidAddress(String),
    #[display(fmt = "Invalid path: {}", _0)]
    InvalidPath(String),
    #[display(fmt = "Lightning node already running")]
    AlreadyRunning,
    #[display(fmt = "{} is only supported in {} mode", _0, _1)]
    UnsupportedMode(String, String),
    #[display(fmt = "Lightning network is not supported for {}: {}", _0, _1)]
    UnsupportedCoin(String, String),
    #[display(fmt = "No such coin {}", _0)]
    NoSuchCoin(String),
    #[display(fmt = "System time error {}", _0)]
    SystemTimeError(String),
    #[display(fmt = "I/O error {}", _0)]
    IOError(String),
    #[display(fmt = "Hash error {}", _0)]
    HashError(String),
    #[display(fmt = "RPC error {}", _0)]
    RpcError(String),
}

impl HttpStatusCode for EnableLightningError {
    fn status_code(&self) -> StatusCode {
        match self {
            EnableLightningError::InvalidRequest(_)
            | EnableLightningError::RpcError(_)
            | EnableLightningError::UnsupportedCoin(_, _) => StatusCode::BAD_REQUEST,
            EnableLightningError::AlreadyRunning | EnableLightningError::UnsupportedMode(_, _) => {
                StatusCode::METHOD_NOT_ALLOWED
            },
            EnableLightningError::InvalidAddress(_)
            | EnableLightningError::InvalidPath(_)
            | EnableLightningError::SystemTimeError(_)
            | EnableLightningError::IOError(_)
            | EnableLightningError::HashError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            EnableLightningError::NoSuchCoin(_) => StatusCode::PRECONDITION_REQUIRED,
        }
    }
}

impl From<CoinFindError> for EnableLightningError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => EnableLightningError::NoSuchCoin(coin),
        }
    }
}
