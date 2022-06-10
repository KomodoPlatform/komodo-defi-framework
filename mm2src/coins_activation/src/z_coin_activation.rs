use crate::context::CoinsActivationContext;
use crate::prelude::*;
use crate::standalone_coin::{InitStandaloneCoinActivationOps, InitStandaloneCoinError,
                             InitStandaloneCoinInitialStatus, InitStandaloneCoinTaskHandle,
                             InitStandaloneCoinTaskManagerShared};
use async_trait::async_trait;
use coins::coin_balance::{EnableCoinBalance, IguanaWalletBalance};
use coins::z_coin::{z_coin_from_conf_and_params, BlockchainScanStopped, SyncStatus, ZCoin, ZCoinBuildError,
                    ZcoinActivationParams, ZcoinConsensusParams};
use coins::{BalanceError, CoinProtocol, MarketCoinOps, RegisterCoinError};
use crypto::hw_rpc_task::{HwRpcTaskAwaitingStatus, HwRpcTaskUserAction};
use crypto::CryptoInitError;
use derive_more::Display;
use futures::compat::Future01CompatExt;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use rpc_task::RpcTaskError;
use ser_error_derive::SerializeErrorType;
use serde_derive::Serialize;
use serde_json::Value as Json;
use std::time::Duration;

pub type ZcoinTaskManagerShared = InitStandaloneCoinTaskManagerShared<ZCoin>;
pub type ZcoinRpcTaskHandle = InitStandaloneCoinTaskHandle<ZCoin>;
pub type ZcoinAwaitingStatus = HwRpcTaskAwaitingStatus;
pub type ZcoinUserAction = HwRpcTaskUserAction;

#[derive(Clone, Serialize)]
pub struct ZcoinActivationResult {
    pub ticker: String,
    pub current_block: u64,
    pub wallet_balance: EnableCoinBalance,
}

impl CurrentBlock for ZcoinActivationResult {
    fn current_block(&self) -> u64 { self.current_block }
}

#[derive(Clone, Serialize)]
#[non_exhaustive]
pub enum ZcoinInProgressStatus {
    ActivatingCoin,
    UpdatingBlocksCache {
        current_scanned_block: u64,
        latest_block: u64,
    },
    BuildingWalletDb,
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

#[derive(Clone, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
#[non_exhaustive]
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

impl From<BlockchainScanStopped> for ZcoinInitError {
    fn from(e: BlockchainScanStopped) -> Self { ZcoinInitError::Internal(e.to_string()) }
}

impl From<ZcoinInitError> for InitStandaloneCoinError {
    fn from(err: ZcoinInitError) -> Self {
        match err {
            ZcoinInitError::CoinCreationError { ticker, error } => {
                InitStandaloneCoinError::CoinCreationError { ticker, error }
            },
            ZcoinInitError::CoinIsAlreadyActivated { ticker } => {
                InitStandaloneCoinError::CoinIsAlreadyActivated { ticker }
            },
            ZcoinInitError::HardwareWalletsAreNotSupportedYet => {
                InitStandaloneCoinError::Internal("Hardware wallets are not supported yet".into())
            },
            ZcoinInitError::TaskTimedOut { duration } => InitStandaloneCoinError::TaskTimedOut { duration },
            ZcoinInitError::CouldNotGetBalance(e) | ZcoinInitError::CouldNotGetBlockCount(e) => {
                InitStandaloneCoinError::Transport(e)
            },
            ZcoinInitError::Internal(e) => InitStandaloneCoinError::Internal(e),
        }
    }
}

impl TryFromCoinProtocol for ZcoinConsensusParams {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::ZHTLC { consensus_params } => Ok(consensus_params),
            protocol => MmError::err(protocol),
        }
    }
}

#[async_trait]
impl InitStandaloneCoinActivationOps for ZCoin {
    type ActivationRequest = ZcoinActivationParams;
    type StandaloneProtocol = ZcoinConsensusParams;
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
        protocol_info: ZcoinConsensusParams,
        task_handle: &ZcoinRpcTaskHandle,
    ) -> MmResult<Self, ZcoinInitError> {
        let secp_privkey = ctx.secp256k1_key_pair().private().secret;
        let coin = z_coin_from_conf_and_params(
            &ctx,
            &ticker,
            &coin_conf,
            activation_request,
            protocol_info,
            secp_privkey.as_slice(),
        )
        .await
        .mm_err(|e| ZcoinInitError::from_build_err(e, ticker))?;

        loop {
            let in_progress_status = match coin.sync_status().await? {
                SyncStatus::UpdatingBlocksCache {
                    current_scanned_block,
                    latest_block,
                } => ZcoinInProgressStatus::UpdatingBlocksCache {
                    current_scanned_block,
                    latest_block,
                },
                SyncStatus::BuildingWalletDb => ZcoinInProgressStatus::BuildingWalletDb,
                SyncStatus::Finished { .. } => break,
            };
            task_handle.update_in_progress_status(in_progress_status)?;
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
            ticker: self.ticker().into(),
            current_block,
            wallet_balance: EnableCoinBalance::Iguana(IguanaWalletBalance {
                address: self.my_z_address_encoded(),
                balance,
            }),
        })
    }
}
