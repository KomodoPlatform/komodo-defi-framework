use crate::context::CoinsActivationContext;
use crate::init_token::{InitTokenActivationOps, InitTokenActivationResult, InitTokenAwaitingStatus, InitTokenError,
                        InitTokenInProgressStatus, InitTokenTaskHandleShared, InitTokenTaskManagerShared,
                        InitTokenUserAction};
use async_trait::async_trait;
use coins::coin_balance::EnableCoinBalanceOps;
use coins::eth::v2_activation::{Erc20Protocol, EthTokenActivationError, InitErc20TokenActivationRequest};
use coins::eth::EthCoin;
use coins::hd_wallet::RpcTaskXPubExtractor;
use coins::{MarketCoinOps, RegisterCoinError};
use common::Future01CompatExt;
use crypto::hw_rpc_task::HwConnectStatuses;
use crypto::HwRpcError;
use derive_more::Display;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::mm_error::MmError;
use mm2_err_handle::prelude::*;
use ser_error_derive::SerializeErrorType;
use serde_derive::Serialize;
use std::time::Duration;

pub type Erc20TokenTaskManagerShared = InitTokenTaskManagerShared<EthCoin>;

#[derive(Clone, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum InitErc20Error {
    #[display(fmt = "{}", _0)]
    HwError(HwRpcError),
    #[display(fmt = "Initialization task has timed out {:?}", duration)]
    TaskTimedOut { duration: Duration },
    #[display(fmt = "Token {} is activated already", ticker)]
    TokenIsAlreadyActivated { ticker: String },
    #[display(fmt = "Error on  token {} creation: {}", ticker, error)]
    TokenCreationError { ticker: String, error: String },
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

// Todo: maybe move this
impl From<InitErc20Error> for InitTokenError {
    fn from(e: InitErc20Error) -> Self {
        match e {
            InitErc20Error::HwError(hw) => InitTokenError::HwError(hw),
            InitErc20Error::TaskTimedOut { duration } => InitTokenError::TaskTimedOut { duration },
            InitErc20Error::TokenIsAlreadyActivated { ticker } => InitTokenError::TokenIsAlreadyActivated { ticker },
            InitErc20Error::TokenCreationError { ticker, error } => {
                InitTokenError::TokenCreationError { ticker, error }
            },
            InitErc20Error::Transport(transport) => InitTokenError::Transport(transport),
            InitErc20Error::Internal(internal) => InitTokenError::Internal(internal),
        }
    }
}

impl From<EthTokenActivationError> for InitErc20Error {
    fn from(e: EthTokenActivationError) -> Self {
        match e {
            EthTokenActivationError::InternalError(_) | EthTokenActivationError::UnexpectedDerivationMethod(_) => {
                InitErc20Error::Internal(e.to_string())
            },
            EthTokenActivationError::ClientConnectionFailed(_)
            | EthTokenActivationError::CouldNotFetchBalance(_)
            | EthTokenActivationError::InvalidPayload(_)
            | EthTokenActivationError::Transport(_) => InitErc20Error::Transport(e.to_string()),
        }
    }
}

impl From<RegisterCoinError> for InitErc20Error {
    fn from(err: RegisterCoinError) -> InitErc20Error {
        match err {
            RegisterCoinError::CoinIsInitializedAlready { coin } => {
                InitErc20Error::TokenIsAlreadyActivated { ticker: coin }
            },
            RegisterCoinError::Internal(e) => InitErc20Error::Internal(e),
        }
    }
}

#[async_trait]
impl InitTokenActivationOps for EthCoin {
    type ActivationRequest = InitErc20TokenActivationRequest;
    type ProtocolInfo = Erc20Protocol;
    type ActivationResult = InitTokenActivationResult;
    type ActivationError = InitErc20Error;
    type InProgressStatus = InitTokenInProgressStatus;
    type AwaitingStatus = InitTokenAwaitingStatus;
    type UserAction = InitTokenUserAction;

    fn rpc_task_manager(activation_ctx: &CoinsActivationContext) -> &Erc20TokenTaskManagerShared {
        &activation_ctx.init_erc20_token_task_manager
    }

    async fn init_token(
        ticker: String,
        platform_coin: Self::PlatformCoin,
        activation_request: &Self::ActivationRequest,
        protocol_conf: Self::ProtocolInfo,
        // Todo: use task handle and look for other init methods for inspiration
        _task_handle: InitTokenTaskHandleShared<Self>,
    ) -> Result<Self, MmError<Self::ActivationError>> {
        let token = platform_coin
            .initialize_erc20_token(activation_request.clone().into(), protocol_conf, ticker)
            .await?;

        Ok(token)
    }

    // Todo: similar to utxo_activation a common method for getting activation result can be made
    async fn get_activation_result(
        &self,
        ctx: MmArc,
        protocol_conf: Self::ProtocolInfo,
        task_handle: InitTokenTaskHandleShared<Self>,
        activation_request: &Self::ActivationRequest,
    ) -> Result<Self::ActivationResult, MmError<Self::ActivationError>> {
        let ticker = self.ticker().to_owned();
        let current_block = self
            .current_block()
            .compat()
            .await
            .map_to_mm(EthTokenActivationError::Transport)?;

        let xpub_extractor = if self.is_trezor() {
            Some(
                RpcTaskXPubExtractor::new_trezor_extractor(
                    &ctx,
                    task_handle,
                    erc20_xpub_extractor_rpc_statuses(),
                    protocol_conf.into(),
                )
                .mm_err(|_| InitErc20Error::HwError(HwRpcError::NotInitialized))?,
            )
        } else {
            None
        };

        let wallet_balance = self
            .enable_coin_balance(
                xpub_extractor,
                activation_request.enable_params.clone(),
                &activation_request.path_to_address,
            )
            .await
            .mm_err(|e| EthTokenActivationError::InternalError(e.to_string()))?;

        // Todo: this should be included in the result probably
        // let token_contract_address = token.erc20_token_address().ok_or_else(|| {
        //     EthTokenActivationError::InternalError("Token contract address is missing".to_string())
        // })?;

        Ok(InitTokenActivationResult {
            ticker,
            current_block,
            wallet_balance,
        })
    }
}

pub(crate) fn erc20_xpub_extractor_rpc_statuses(
) -> HwConnectStatuses<InitTokenInProgressStatus, InitTokenAwaitingStatus> {
    HwConnectStatuses {
        on_connect: InitTokenInProgressStatus::WaitingForTrezorToConnect,
        on_connected: InitTokenInProgressStatus::ActivatingCoin,
        on_connection_failed: InitTokenInProgressStatus::Finishing,
        on_button_request: InitTokenInProgressStatus::FollowHwDeviceInstructions,
        on_pin_request: InitTokenAwaitingStatus::EnterTrezorPin,
        on_passphrase_request: InitTokenAwaitingStatus::EnterTrezorPassphrase,
        on_ready: InitTokenInProgressStatus::ActivatingCoin,
    }
}
