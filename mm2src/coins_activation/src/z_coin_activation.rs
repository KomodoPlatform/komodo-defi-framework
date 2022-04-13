use crate::context::CoinsActivationContext;
use crate::prelude::*;
use crate::standalone_coin::{InitStandaloneCoinActivationOps, InitStandaloneCoinError,
                             InitStandaloneCoinInitialStatus, InitStandaloneCoinTaskHandle,
                             InitStandaloneCoinTaskManagerShared};
use async_trait::async_trait;
use coins::coin_balance::{EnableCoinBalance, IguanaWalletBalance};
use coins::utxo::rpc_clients::ElectrumRpcRequest;
use coins::utxo::{UtxoActivationParams, UtxoRpcMode};
use coins::z_coin::{z_coin_from_conf_and_params, ZCoin, ZCoinBuildError};
use coins::{BalanceError, CoinProtocol, MarketCoinOps, PrivKeyActivationPolicy, RegisterCoinError};
use common::executor::Timer;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use crypto::hw_rpc_task::{HwRpcTaskAwaitingStatus, HwRpcTaskUserAction};
use crypto::{CryptoCtx, CryptoInitError};
use derive_more::Display;
use futures::compat::Future01CompatExt;
use rpc_task::RpcTaskError;
use ser_error_derive::SerializeErrorType;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::time::Duration;

pub type ZcoinTaskManagerShared = InitStandaloneCoinTaskManagerShared<ZCoin>;
pub type ZcoinRpcTaskHandle = InitStandaloneCoinTaskHandle<ZCoin>;
pub type ZcoinAwaitingStatus = HwRpcTaskAwaitingStatus;
pub type ZcoinUserAction = HwRpcTaskUserAction;

#[derive(Clone, Serialize)]
pub struct ZcoinActivationResult {
    pub current_block: u64,
    pub wallet_balance: EnableCoinBalance,
}

impl GetCurrentBlock for ZcoinActivationResult {
    fn get_current_block(&self) -> u64 { self.current_block }
}

#[derive(Clone, Serialize)]
pub enum ZcoinInProgressStatus {
    ActivatingCoin,
    Scanning,
    RequestingWalletBalance,
    Finishing,
    /// This status doesn't require the user to send `UserAction`,
    /// but it tells the user that he should confirm/decline an address on his device.
    WaitingForTrezorToConnect,
    WaitingForUserToConfirmPubkey,
}

impl InitStandaloneCoinInitialStatus for ZcoinInProgressStatus {
    fn initial_status() -> Self { ZcoinInProgressStatus::ActivatingCoin }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "rpc", content = "rpc_data")]
pub enum ZcoinRpcMode {
    Native,
    Light {
        electrum_servers: Vec<ElectrumRpcRequest>,
        light_wallet_d_servers: Vec<String>,
    },
}

#[derive(Deserialize)]
pub struct ZcoinActivationParams {
    pub mode: ZcoinRpcMode,
    pub required_confirmations: Option<u64>,
    pub requires_notarization: Option<bool>,
}

impl TxHistoryEnabled for ZcoinActivationParams {
    fn tx_history_enabled(&self) -> bool { false }
}

#[derive(Clone, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum ZcoinInitError {
    #[display(fmt = "Error on coin {} creation: {}", ticker, error)]
    CoinCreationError {
        ticker: String,
        error: String,
    },
    CoinIsAlreadyActivated {
        ticker: String,
    },
    HardwareWalletsAreNotSupportedYet,
    #[display(fmt = "Initialization task has timed out {:?}", duration)]
    TaskTimedOut {
        duration: Duration,
    },
    CouldNotGetBalance(String),
    CouldNotGetBlockCount(String),
    Internal(String),
}

impl ZcoinInitError {
    pub fn from_build_err(build_err: ZCoinBuildError, ticker: String) -> Self {
        ZcoinInitError::CoinCreationError {
            ticker,
            error: build_err.to_string(),
        }
    }
}

impl From<BalanceError> for ZcoinInitError {
    fn from(err: BalanceError) -> Self { ZcoinInitError::CouldNotGetBalance(err.to_string()) }
}

impl From<RegisterCoinError> for ZcoinInitError {
    fn from(reg_err: RegisterCoinError) -> ZcoinInitError {
        match reg_err {
            RegisterCoinError::CoinIsInitializedAlready { coin } => {
                ZcoinInitError::CoinIsAlreadyActivated { ticker: coin }
            },
            RegisterCoinError::Internal(internal) => ZcoinInitError::Internal(internal),
        }
    }
}

impl From<RpcTaskError> for ZcoinInitError {
    fn from(rpc_err: RpcTaskError) -> Self {
        match rpc_err {
            RpcTaskError::Timeout(duration) => ZcoinInitError::TaskTimedOut { duration },
            internal_error => ZcoinInitError::Internal(internal_error.to_string()),
        }
    }
}

impl From<CryptoInitError> for ZcoinInitError {
    fn from(err: CryptoInitError) -> Self { ZcoinInitError::Internal(err.to_string()) }
}

impl From<ZcoinInitError> for InitStandaloneCoinError {
    fn from(_: ZcoinInitError) -> Self { todo!() }
}

pub struct ZcoinProtocolInfo;

impl TryFromCoinProtocol for ZcoinProtocolInfo {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::ZHTLC => Ok(ZcoinProtocolInfo),
            protocol => MmError::err(protocol),
        }
    }
}

#[async_trait]
impl InitStandaloneCoinActivationOps for ZCoin {
    type ActivationRequest = ZcoinActivationParams;
    type StandaloneProtocol = ZcoinProtocolInfo;
    type ActivationResult = ZcoinActivationResult;
    type ActivationError = ZcoinInitError;
    type InProgressStatus = ZcoinInProgressStatus;
    type AwaitingStatus = ZcoinAwaitingStatus;
    type UserAction = ZcoinUserAction;

    fn rpc_task_manager(activation_ctx: &CoinsActivationContext) -> &ZcoinTaskManagerShared {
        &activation_ctx.init_z_coin_task_manager
    }

    async fn init_standalone_coin(
        ctx: MmArc,
        ticker: String,
        coin_conf: Json,
        activation_request: &ZcoinActivationParams,
        _protocol_info: ZcoinProtocolInfo,
        task_handle: &ZcoinRpcTaskHandle,
    ) -> MmResult<Self, ZcoinInitError> {
        let utxo_mode = match &activation_request.mode {
            ZcoinRpcMode::Native => UtxoRpcMode::Native,
            ZcoinRpcMode::Light { electrum_servers, .. } => UtxoRpcMode::Electrum {
                servers: electrum_servers.clone(),
            },
        };
        let utxo_params = UtxoActivationParams {
            mode: utxo_mode,
            utxo_merge_params: None,
            tx_history: false,
            required_confirmations: activation_request.required_confirmations,
            requires_notarization: activation_request.requires_notarization,
            address_format: None,
            gap_limit: None,
            scan_policy: Default::default(),
            priv_key_policy: PrivKeyActivationPolicy::IguanaPrivKey,
            check_utxo_maturity: None,
        };
        let crypto_ctx = CryptoCtx::from_ctx(&ctx)?;
        let priv_key = crypto_ctx.iguana_ctx().secp256k1_privkey().secret;
        let coin = z_coin_from_conf_and_params(&ctx, &ticker, &coin_conf, &utxo_params, priv_key.as_slice())
            .await
            .mm_err(|e| ZcoinInitError::from_build_err(e, ticker))?;

        task_handle.update_in_progress_status(ZcoinInProgressStatus::Scanning)?;
        while !coin.is_sapling_state_synced() {
            Timer::sleep(1.).await;
        }
        Ok(coin)
    }

    async fn get_activation_result(
        &self,
        _ctx: MmArc,
        task_handle: &ZcoinRpcTaskHandle,
        _activation_request: &Self::ActivationRequest,
    ) -> MmResult<Self::ActivationResult, ZcoinInitError> {
        task_handle.update_in_progress_status(ZcoinInProgressStatus::RequestingWalletBalance)?;
        let current_block = self
            .current_block()
            .compat()
            .await
            .map_to_mm(ZcoinInitError::CouldNotGetBlockCount)?;

        let balance = self.my_balance().compat().await?;
        Ok(ZcoinActivationResult {
            current_block,
            wallet_balance: EnableCoinBalance::Iguana(IguanaWalletBalance {
                address: self.my_z_address_encoded(),
                balance,
            }),
        })
    }
}
