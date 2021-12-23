use crate::context::CoinsActivationContext;
use async_trait::async_trait;
use coins::utxo::qtum::QtumCoin;
use coins::utxo::rpc_clients::ElectrumRpcRequest;
use coins::utxo::utxo_standard::UtxoStandardCoin;
use coins::utxo::{utxo_common, PrivKeyBuildPolicy, UtxoActivationParams, UtxoCoinBuildError, UtxoConfError};
use coins::{coin_conf, lp_coinfind, lp_register_coin, CoinProtocol, MmCoinEnum, RegisterCoinError, RegisterCoinParams};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::{HttpStatusCode, StatusCode};
use crypto::trezor::TrezorPinMatrix3x3Response;
use crypto::{CryptoCtx, CryptoInitError};
use derive_more::Display;
use rpc_task::{RpcTask, RpcTaskError, RpcTaskHandle, RpcTaskManager, RpcTaskManagerShared, RpcTaskStatus, TaskId};
use ser_error_derive::SerializeErrorType;
use serde::{Deserialize, Serialize};
use serde_json::{self as json};
use std::time::Duration;

pub type InitUtxoResult<T> = Result<T, MmError<InitUtxoError>>;
pub type UtxoInitTaskManager = RpcTaskManager<
    InitUtxoResponse,
    InitUtxoError,
    InitUtxoInProgressStatus,
    InitUtxoAwaitingStatus,
    InitUtxoUserAction,
>;
pub type UtxoInitTaskManagerShared = RpcTaskManagerShared<
    InitUtxoResponse,
    InitUtxoError,
    InitUtxoInProgressStatus,
    InitUtxoAwaitingStatus,
    InitUtxoUserAction,
>;
type UtxoInitTaskHandle = RpcTaskHandle<
    InitUtxoResponse,
    InitUtxoError,
    InitUtxoInProgressStatus,
    InitUtxoAwaitingStatus,
    InitUtxoUserAction,
>;
type UtxoInitStatus = RpcTaskStatus<InitUtxoResponse, InitUtxoError, InitUtxoInProgressStatus, InitUtxoAwaitingStatus>;

/// A combination of `UtxoCoinBuildError`, `UtxoConfError`, `RpcTaskError` errors.
#[derive(Clone, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum InitUtxoError {
    #[display(fmt = "Coin '{}' is activated already", coin)]
    CoinIsActivated {
        coin: String,
    },
    #[display(fmt = "Coin '{}' is used without a corresponding configuration", coin)]
    CoinConfNotFound {
        coin: String,
    },
    #[display(fmt = "Coin '{}' is not a UTXO", coin)]
    CoinIsNotUtxo {
        coin: String,
    },
    #[display(fmt = "Invalid 'protocol' in coin's config: {}", _0)]
    InvalidCoinProtocol(String),
    #[display(fmt = "'name' field is not found in config")]
    CurrencyNameIsNotSet,
    #[display(fmt = "Invalid 'derivation_path' purpose '{}'. BIP44 is supported only", actual)]
    InvalidDerivationPathPurpose {
        actual: u32,
    },
    #[display(
        fmt = "Invalid length '{}' of 'derivation_path'. Expected \"m/purpose'/coin_type'/\" path, i.e 2 children",
        found_children
    )]
    InvalidDerivationPathLen {
        found_children: usize,
    },
    #[display(fmt = "Error deserializing 'derivation_path': {}", _0)]
    ErrorDeserializingDerivationPath(String),
    #[display(fmt = "Invalid 'consensus_branch_id' in coin's config: {}", _0)]
    InvalidConsensusBranchId(String),
    #[display(fmt = "Invalid 'version_group_id' in coin's config: {}", _0)]
    InvalidVersionGroupId(String),
    #[display(fmt = "Invalid 'address_format' in coin's config: {}", _0)]
    InvalidAddressFormat(String),
    #[display(fmt = "Invalid 'network' in coin's config: {}", _0)]
    InvalidBlockchainNetwork(String),
    #[display(fmt = "Invalid 'decimals' in coin's config: {}", _0)]
    InvalidDecimals(String),
    #[display(fmt = "Native RPC client is only supported in native mode")]
    NativeRpcNotSupportedInWasm,
    ErrorReadingNativeModeConf(String),
    #[display(fmt = "Rpc port is not set neither in `coins` file nor in native daemon config")]
    RpcPortIsNotSet,
    ErrorDetectingFeeMethod(String),
    ErrorDetectingDecimals(String),
    ErrorGettingBlockCount(String),
    #[display(
        fmt = "Failed to connect to at least 1 of {:?} in {} seconds.",
        electrum_servers,
        seconds
    )]
    FailedToConnectToElectrums {
        electrum_servers: Vec<ElectrumRpcRequest>,
        seconds: u64,
    },
    ElectrumProtocolVersionCheckError(String),
    #[display(fmt = "Can not detect the user home directory")]
    CantDetectUserHome,
    #[display(fmt = "Withdraw timed out {:?}", _0)]
    Timeout(Duration),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl From<RpcTaskError> for InitUtxoError {
    fn from(e: RpcTaskError) -> Self {
        let error = e.to_string();
        match e {
            RpcTaskError::Canceled => InitUtxoError::Internal("Canceled".to_owned()),
            RpcTaskError::Timeout(timeout) => InitUtxoError::Timeout(timeout),
            RpcTaskError::NoSuchTask(_) | RpcTaskError::UnexpectedTaskStatus { .. } => InitUtxoError::Internal(error),
            RpcTaskError::Internal(internal) => InitUtxoError::Internal(internal),
        }
    }
}

impl From<RegisterCoinError> for InitUtxoError {
    fn from(e: RegisterCoinError) -> Self {
        match e {
            RegisterCoinError::CoinIsInitializedAlready { coin } => InitUtxoError::CoinIsActivated { coin },
            RegisterCoinError::ErrorGettingBlockCount(error) => InitUtxoError::ErrorGettingBlockCount(error),
            RegisterCoinError::Internal(internal) => InitUtxoError::Internal(internal),
        }
    }
}

impl From<CryptoInitError> for InitUtxoError {
    fn from(e: CryptoInitError) -> Self { InitUtxoError::Internal(e.to_string()) }
}

impl From<UtxoCoinBuildError> for InitUtxoError {
    fn from(e: UtxoCoinBuildError) -> Self {
        match e {
            UtxoCoinBuildError::ConfError(conf_err) => InitUtxoError::from(conf_err),
            UtxoCoinBuildError::NativeRpcNotSupportedInWasm => InitUtxoError::NativeRpcNotSupportedInWasm,
            UtxoCoinBuildError::ErrorReadingNativeModeConf(error) => InitUtxoError::ErrorReadingNativeModeConf(error),
            UtxoCoinBuildError::RpcPortIsNotSet => InitUtxoError::RpcPortIsNotSet,
            UtxoCoinBuildError::ErrorDetectingFeeMethod(error) => InitUtxoError::ErrorDetectingFeeMethod(error),
            UtxoCoinBuildError::ErrorDetectingDecimals(error) => InitUtxoError::ErrorDetectingDecimals(error),
            UtxoCoinBuildError::FailedToConnectToElectrums {
                electrum_servers,
                seconds,
            } => InitUtxoError::FailedToConnectToElectrums {
                electrum_servers,
                seconds,
            },
            UtxoCoinBuildError::ElectrumProtocolVersionCheckError(error) => {
                InitUtxoError::ElectrumProtocolVersionCheckError(error)
            },
            UtxoCoinBuildError::CantDetectUserHome => InitUtxoError::CantDetectUserHome,
            UtxoCoinBuildError::Internal(internal) => InitUtxoError::Internal(internal),
        }
    }
}

impl From<UtxoConfError> for InitUtxoError {
    fn from(e: UtxoConfError) -> Self {
        match e {
            UtxoConfError::CurrencyNameIsNotSet => InitUtxoError::CurrencyNameIsNotSet,
            UtxoConfError::InvalidDerivationPathPurpose { purpose: actual } => {
                InitUtxoError::InvalidDerivationPathPurpose { actual }
            },
            UtxoConfError::InvalidDerivationPathLen { found_children } => {
                InitUtxoError::InvalidDerivationPathLen { found_children }
            },
            UtxoConfError::ErrorDeserializingDerivationPath(error) => {
                InitUtxoError::ErrorDeserializingDerivationPath(error)
            },
            UtxoConfError::InvalidConsensusBranchId(error) => InitUtxoError::InvalidConsensusBranchId(error),
            UtxoConfError::InvalidVersionGroupId(error) => InitUtxoError::InvalidVersionGroupId(error),
            UtxoConfError::InvalidAddressFormat(error) => InitUtxoError::InvalidAddressFormat(error),
            UtxoConfError::InvalidBlockchainNetwork(error) => InitUtxoError::InvalidBlockchainNetwork(error),
            UtxoConfError::InvalidDecimals(error) => InitUtxoError::InvalidDecimals(error),
        }
    }
}

impl HttpStatusCode for InitUtxoError {
    fn status_code(&self) -> StatusCode {
        match self {
            InitUtxoError::CoinIsActivated { .. }
            | InitUtxoError::CoinIsNotUtxo { .. }
            | InitUtxoError::InvalidCoinProtocol(_)
            | InitUtxoError::CurrencyNameIsNotSet
            | InitUtxoError::InvalidDerivationPathPurpose { .. }
            | InitUtxoError::InvalidDerivationPathLen { .. }
            | InitUtxoError::ErrorDeserializingDerivationPath(_)
            | InitUtxoError::InvalidConsensusBranchId(_)
            | InitUtxoError::InvalidVersionGroupId(_)
            | InitUtxoError::InvalidAddressFormat(_)
            | InitUtxoError::InvalidBlockchainNetwork(_)
            | InitUtxoError::InvalidDecimals(_)
            | InitUtxoError::NativeRpcNotSupportedInWasm
            | InitUtxoError::ErrorReadingNativeModeConf(_)
            | InitUtxoError::RpcPortIsNotSet => StatusCode::BAD_REQUEST,
            InitUtxoError::CoinConfNotFound { .. } => StatusCode::NOT_FOUND,
            InitUtxoError::ElectrumProtocolVersionCheckError(_)
            | InitUtxoError::ErrorDetectingFeeMethod(_)
            | InitUtxoError::ErrorDetectingDecimals(_)
            | InitUtxoError::ErrorGettingBlockCount(_)
            | InitUtxoError::FailedToConnectToElectrums { .. }
            | InitUtxoError::CantDetectUserHome
            | InitUtxoError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            InitUtxoError::Timeout(_) => StatusCode::REQUEST_TIMEOUT,
        }
    }
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum InitUtxoStatusError {
    NoSuchTask(TaskId),
    Internal(String),
}

impl HttpStatusCode for InitUtxoStatusError {
    fn status_code(&self) -> StatusCode { StatusCode::NOT_FOUND }
}

#[derive(Serialize)]
pub struct InitWithdrawResponse {
    task_id: TaskId,
}

#[derive(Deserialize)]
pub struct InitUtxoRequest {
    coin: String,
    #[serde(flatten)]
    params: UtxoActivationParams,
}

pub async fn init_utxo(ctx: MmArc, request: InitUtxoRequest) -> InitUtxoResult<InitWithdrawResponse> {
    let task = InitUtxoTask {
        ctx: ctx.clone(),
        request,
    };
    let coins_act_ctx = CoinsActivationContext::from_ctx(&ctx).map_to_mm(InitUtxoError::Internal)?;
    let task_id = UtxoInitTaskManager::spawn_rpc_task(&coins_act_ctx.init_utxo_task_manager, task)?;
    Ok(InitWithdrawResponse { task_id })
}

#[derive(Deserialize)]
pub struct InitUtxoStatusRequest {
    task_id: TaskId,
    #[serde(default = "true_f")]
    forget_if_finished: bool,
}

pub async fn init_utxo_status(
    ctx: MmArc,
    req: InitUtxoStatusRequest,
) -> Result<UtxoInitStatus, MmError<InitUtxoStatusError>> {
    let coins_act_ctx = CoinsActivationContext::from_ctx(&ctx).map_to_mm(InitUtxoStatusError::Internal)?;
    let mut task_manager = coins_act_ctx
        .init_utxo_task_manager
        .lock()
        .map_to_mm(|poison| InitUtxoStatusError::Internal(poison.to_string()))?;
    task_manager
        .task_status(req.task_id, req.forget_if_finished)
        .or_mm_err(|| InitUtxoStatusError::NoSuchTask(req.task_id))
}

#[derive(Clone, Serialize)]
pub enum InitUtxoInProgressStatus {
    ActivatingCoin,
    /// This status doesn't require the user to send `UserAction`,
    /// but it tells the user that he should confirm/decline an address on his device.
    #[allow(dead_code)]
    WaitingForUserToConfirmAddress,
}

#[derive(Clone, Deserialize, Serialize)]
pub enum InitUtxoAwaitingStatus {
    WaitForTrezorPin,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "action_type")]
pub enum InitUtxoUserAction {
    TrezorPin(TrezorPinMatrix3x3Response),
}

pub struct InitUtxoTask {
    ctx: MmArc,
    request: InitUtxoRequest,
}

/// TODO return `addresses` with balances.
#[derive(Clone, Serialize)]
pub struct InitUtxoResponse {
    coin: String,
    required_confirmations: u64,
    requires_notarization: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    mature_confirmations: Option<u32>,
}

#[async_trait]
impl RpcTask for InitUtxoTask {
    type Item = InitUtxoResponse;
    type Error = InitUtxoError;
    type InProgressStatus = InitUtxoInProgressStatus;
    type AwaitingStatus = InitUtxoAwaitingStatus;
    type UserAction = InitUtxoUserAction;

    fn initial_status(&self) -> Self::InProgressStatus { InitUtxoInProgressStatus::ActivatingCoin }

    async fn run(self, _task_handle: &UtxoInitTaskHandle) -> Result<Self::Item, MmError<Self::Error>> {
        if let Ok(Some(_)) = lp_coinfind(&self.ctx, &self.request.coin).await {
            return MmError::err(InitUtxoError::CoinIsActivated {
                coin: self.request.coin.clone(),
            });
        }

        let coin = self.activate_utxo_coin().await?;
        // TODO get accounts, addresses and request their balances
        Ok(InitUtxoResponse {
            coin: coin.ticker().to_owned(),
            required_confirmations: coin.required_confirmations(),
            requires_notarization: coin.requires_notarization(),
            mature_confirmations: coin.mature_confirmations(),
        })
    }
}

impl InitUtxoTask {
    /// TODO refactor it at the next iteration.
    async fn activate_utxo_coin(&self) -> InitUtxoResult<MmCoinEnum> {
        let coins_en = coin_conf(&self.ctx, &self.request.coin);
        if coins_en.is_null() {
            return MmError::err(InitUtxoError::CoinConfNotFound {
                coin: self.request.coin.clone(),
            });
        }

        let crypto_ctx = CryptoCtx::from_ctx(&self.ctx)?;

        let protocol: CoinProtocol = json::from_value(coins_en["protocol"].clone())
            .map_to_mm(|e| InitUtxoError::InvalidCoinProtocol(e.to_string()))?;

        let priv_key_policy = match *crypto_ctx {
            CryptoCtx::KeyPair(ref key_pair_ctx) => PrivKeyBuildPolicy::PrivKey(key_pair_ctx.secp256k1_privkey_bytes()),
            CryptoCtx::HardwareWallet(_) => PrivKeyBuildPolicy::HardwareWallet,
        };

        let coin = match protocol {
            CoinProtocol::UTXO => {
                let coin: UtxoStandardCoin = utxo_common::utxo_arc_from_conf_and_params(
                    &self.ctx,
                    &self.request.coin,
                    &coins_en,
                    self.request.params.clone(),
                    priv_key_policy,
                    UtxoStandardCoin::from,
                )
                .await?;
                MmCoinEnum::from(coin)
            },
            CoinProtocol::QTUM => {
                let coin: QtumCoin = utxo_common::utxo_arc_from_conf_and_params(
                    &self.ctx,
                    &self.request.coin,
                    &coins_en,
                    self.request.params.clone(),
                    priv_key_policy,
                    QtumCoin::from,
                )
                .await?;
                MmCoinEnum::from(coin)
            },
            _ => {
                return MmError::err(InitUtxoError::CoinIsNotUtxo {
                    coin: self.request.coin.clone(),
                })
            },
        };

        let register_params = RegisterCoinParams {
            ticker: self.request.coin.clone(),
            tx_history: self.request.params.tx_history,
        };

        lp_register_coin(&self.ctx, coin.clone(), register_params).await?;
        Ok(coin)
    }
}

fn true_f() -> bool { true }
