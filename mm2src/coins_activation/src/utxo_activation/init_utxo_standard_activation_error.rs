use crate::standalone_coin::InitStandaloneCoinError;
use coins::utxo::utxo_builder::UtxoCoinBuildError;
use coins::RegisterCoinError;
use crypto::CryptoInitError;
use derive_more::Display;
use rpc_task::RpcTaskError;
use ser_error_derive::SerializeErrorType;
use serde_derive::Serialize;
use std::time::Duration;

#[derive(Clone, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum InitUtxoStandardError {
    #[display(fmt = "Initialization task has timed out {:?}", duration)]
    TaskTimedOut {
        duration: Duration,
    },
    CoinIsAlreadyActivated {
        ticker: String,
    },
    #[display(fmt = "Error on platform coin {} creation: {}", ticker, error)]
    CoinCreationError {
        ticker: String,
        error: String,
    },
    Internal(String),
}

impl From<RpcTaskError> for InitUtxoStandardError {
    fn from(rpc_err: RpcTaskError) -> Self {
        match rpc_err {
            RpcTaskError::Timeout(duration) => InitUtxoStandardError::TaskTimedOut { duration },
            internal_error => InitUtxoStandardError::Internal(internal_error.to_string()),
        }
    }
}

impl From<CryptoInitError> for InitUtxoStandardError {
    /// `CryptoCtx` is expected to be initialized already.
    fn from(crypto_err: CryptoInitError) -> Self { InitUtxoStandardError::Internal(crypto_err.to_string()) }
}

impl From<InitUtxoStandardError> for InitStandaloneCoinError {
    fn from(e: InitUtxoStandardError) -> Self {
        match e {
            InitUtxoStandardError::TaskTimedOut { duration } => InitStandaloneCoinError::TaskTimedOut { duration },
            InitUtxoStandardError::CoinIsAlreadyActivated { ticker } => {
                InitStandaloneCoinError::CoinIsAlreadyActivated { ticker }
            },
            InitUtxoStandardError::CoinCreationError { ticker, error } => {
                InitStandaloneCoinError::CoinCreationError { ticker, error }
            },
            InitUtxoStandardError::Internal(internal) => InitStandaloneCoinError::Internal(internal),
        }
    }
}

impl InitUtxoStandardError {
    pub fn from_register_err(reg_err: RegisterCoinError, ticker: String) -> InitUtxoStandardError {
        match reg_err {
            RegisterCoinError::CoinIsInitializedAlready { coin } => {
                InitUtxoStandardError::CoinIsAlreadyActivated { ticker: coin }
            },
            RegisterCoinError::ErrorGettingBlockCount(error) => {
                InitUtxoStandardError::CoinCreationError { ticker, error }
            },
            RegisterCoinError::Internal(internal) => InitUtxoStandardError::Internal(internal),
        }
    }

    pub fn from_build_err(build_err: UtxoCoinBuildError, ticker: String) -> Self {
        match build_err {
            UtxoCoinBuildError::Internal(internal) => InitUtxoStandardError::Internal(internal),
            build_err => InitUtxoStandardError::CoinCreationError {
                ticker,
                error: build_err.to_string(),
            },
        }
    }
}
