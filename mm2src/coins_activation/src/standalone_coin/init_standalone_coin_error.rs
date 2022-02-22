use crate::prelude::CoinConfWithProtocolError;
use coins::CoinProtocol;
use common::{HttpStatusCode, StatusCode};
use derive_more::Display;
use rpc_task::rpc_common::{RpcTaskStatusError, RpcTaskUserActionError};
use rpc_task::{RpcTaskError, TaskId};
use ser_error_derive::SerializeErrorType;
use serde_derive::Serialize;
use std::time::Duration;

pub type InitStandaloneCoinStatusError = RpcTaskStatusError;
pub type InitStandaloneCoinUserActionError = RpcTaskUserActionError;

#[derive(Clone, Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum InitStandaloneCoinError {
    NoSuchTask(TaskId),
    #[display(fmt = "Initialization task has timed out {:?}", duration)]
    TaskTimedOut {
        duration: Duration,
    },
    CoinIsAlreadyActivated {
        ticker: String,
    },
    #[display(fmt = "Coin {} config is not found", _0)]
    CoinConfigIsNotFound(String),
    #[display(fmt = "Coin {} protocol parsing failed: {}", ticker, error)]
    CoinProtocolParseError {
        ticker: String,
        error: String,
    },
    #[display(fmt = "Unexpected platform protocol {:?} for {}", protocol, ticker)]
    UnexpectedCoinProtocol {
        ticker: String,
        protocol: CoinProtocol,
    },
    #[display(fmt = "Error on platform coin {} creation: {}", ticker, error)]
    CoinCreationError {
        ticker: String,
        error: String,
    },
    #[display(fmt = "Private key is not allowed: {}", _0)]
    PrivKeyNotAllowed(String),
    #[display(fmt = "Derivation method is not supported: {}", _0)]
    DerivationMethodNotSupported(String),
    Transport(String),
    Internal(String),
}

impl From<CoinConfWithProtocolError> for InitStandaloneCoinError {
    fn from(e: CoinConfWithProtocolError) -> Self {
        match e {
            CoinConfWithProtocolError::ConfigIsNotFound(error) => InitStandaloneCoinError::CoinConfigIsNotFound(error),
            CoinConfWithProtocolError::CoinProtocolParseError { ticker, err } => {
                InitStandaloneCoinError::CoinProtocolParseError {
                    ticker,
                    error: err.to_string(),
                }
            },
            CoinConfWithProtocolError::UnexpectedProtocol { ticker, protocol } => {
                InitStandaloneCoinError::UnexpectedCoinProtocol { ticker, protocol }
            },
        }
    }
}

impl From<RpcTaskError> for InitStandaloneCoinError {
    fn from(e: RpcTaskError) -> Self {
        match e {
            RpcTaskError::NoSuchTask(task_id) => InitStandaloneCoinError::NoSuchTask(task_id),
            RpcTaskError::Timeout(duration) => InitStandaloneCoinError::TaskTimedOut { duration },
            rpc_internal => InitStandaloneCoinError::Internal(rpc_internal.to_string()),
        }
    }
}

impl HttpStatusCode for InitStandaloneCoinError {
    fn status_code(&self) -> StatusCode {
        match self {
            InitStandaloneCoinError::NoSuchTask(_)
            | InitStandaloneCoinError::CoinIsAlreadyActivated { .. }
            | InitStandaloneCoinError::CoinConfigIsNotFound { .. }
            | InitStandaloneCoinError::CoinProtocolParseError { .. }
            | InitStandaloneCoinError::UnexpectedCoinProtocol { .. }
            | InitStandaloneCoinError::CoinCreationError { .. }
            | InitStandaloneCoinError::PrivKeyNotAllowed(_)
            | InitStandaloneCoinError::DerivationMethodNotSupported(_) => StatusCode::BAD_REQUEST,
            InitStandaloneCoinError::TaskTimedOut { .. } => StatusCode::REQUEST_TIMEOUT,
            InitStandaloneCoinError::Transport(_) | InitStandaloneCoinError::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
        }
    }
}
