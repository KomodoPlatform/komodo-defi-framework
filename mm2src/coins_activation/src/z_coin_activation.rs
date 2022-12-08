use crate::context::CoinsActivationContext;
use crate::prelude::*;
use crate::standalone_coin::{InitStandaloneCoinActivationOps, InitStandaloneCoinError,
                             InitStandaloneCoinInitialStatus, InitStandaloneCoinTaskHandle,
                             InitStandaloneCoinTaskManagerShared};
use async_trait::async_trait;
use coins::coin_balance::{CoinBalanceReport, IguanaWalletBalance};
use coins::my_tx_history_v2::TxHistoryStorage;
use coins::tx_history_storage::CreateTxHistoryStorageError;
use coins::z_coin::{z_coin_from_conf_and_params, BlockchainScanStopped, SyncStatus, ZCoin, ZCoinBuildError,
                    ZcoinActivationParams, ZcoinProtocolInfo};
use coins::{BalanceError, CoinProtocol, MarketCoinOps, PrivKeyBuildPolicy, RegisterCoinError};
use crypto::hw_rpc_task::{HwRpcTaskAwaitingStatus, HwRpcTaskUserAction};
use crypto::CryptoCtxError;
use derive_more::Display;
use futures::compat::Future01CompatExt;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_metrics::MetricsArc;
use mm2_number::BigDecimal;
use rpc_task::RpcTaskError;
use ser_error_derive::SerializeErrorType;
use serde_derive::Serialize;
use serde_json::Value as Json;
use std::collections::HashMap;
use std::time::Duration;

pub type ZcoinTaskManagerShared = InitStandaloneCoinTaskManagerShared<ZCoin>;
pub type ZcoinRpcTaskHandle = InitStandaloneCoinTaskHandle<ZCoin>;
pub type ZcoinAwaitingStatus = HwRpcTaskAwaitingStatus;
pub type ZcoinUserAction = HwRpcTaskUserAction;

#[derive(Clone, Serialize)]
pub struct ZcoinActivationResult {
    pub ticker: String,
    pub current_block: u64,
    pub wallet_balance: CoinBalanceReport,
}

impl CurrentBlock for ZcoinActivationResult {
    fn current_block(&self) -> u64 { self.current_block }
}

impl GetAddressesBalances for ZcoinActivationResult {
    fn get_addresses_balances(&self) -> HashMap<String, BigDecimal> {
        self.wallet_balance.to_addresses_total_balances()
    }
}

#[derive(Clone, Serialize)]
#[non_exhaustive]
pub enum ZcoinInProgressStatus {
    ActivatingCoin,
    UpdatingBlocksCache {
        current_scanned_block: u64,
        latest_block: u64,
    },
    BuildingWalletDb {
        current_scanned_block: u64,
        latest_block: u64,
    },
    TemporaryError(String),
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

impl From<CryptoCtxError> for ZcoinInitError {
    fn from(err: CryptoCtxError) -> Self { ZcoinInitError::Internal(err.to_string()) }
}

impl From<BlockchainScanStopped> for ZcoinInitError {
    fn from(e: BlockchainScanStopped) -> Self { ZcoinInitError::Internal(e.to_string()) }
}

impl From<CreateTxHistoryStorageError> for ZcoinInitError {
    fn from(e: CreateTxHistoryStorageError) -> Self {
        match e {
            CreateTxHistoryStorageError::Internal(internal) => ZcoinInitError::Internal(internal),
        }
    }
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

impl From<CryptoCtxError> for InitStandaloneCoinError {
    fn from(e: CryptoCtxError) -> Self { InitStandaloneCoinError::Internal(e.to_string()) }
}

impl TryFromCoinProtocol for ZcoinProtocolInfo {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::ZHTLC(info) => Ok(info),
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
        protocol_info: ZcoinProtocolInfo,
        task_handle: &ZcoinRpcTaskHandle,
    ) -> MmResult<Self, ZcoinInitError> {
        // When `ZCoin` supports Trezor, we'll need to check [`ZcoinActivationParams::priv_key_policy`]
        // instead of using [`PrivKeyBuildPolicy::detect_priv_key_policy`].
        let priv_key_policy = PrivKeyBuildPolicy::detect_priv_key_policy(&ctx)?;

        let coin = z_coin_from_conf_and_params(
            &ctx,
            &ticker,
            &coin_conf,
            activation_request,
            protocol_info,
            priv_key_policy,
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
                SyncStatus::BuildingWalletDb {
                    current_scanned_block,
                    latest_block,
                } => ZcoinInProgressStatus::BuildingWalletDb {
                    current_scanned_block,
                    latest_block,
                },
                SyncStatus::TemporaryError(e) => ZcoinInProgressStatus::TemporaryError(e),
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
            wallet_balance: CoinBalanceReport::Iguana(IguanaWalletBalance {
                address: self.my_z_address_encoded(),
                balance,
            }),
        })
    }

    /// Transaction history is fetching from a wallet database for `ZCoin`.
    fn start_history_background_fetching(
        &self,
        _metrics: MetricsArc,
        _storage: impl TxHistoryStorage,
        _current_balances: HashMap<String, BigDecimal>,
    ) {
    }
}
