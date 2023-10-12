use async_trait::async_trait;
use common::{SerdeInfallible, SuccessResponse};
use crypto::RpcDerivationPath;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use rpc_task::rpc_common::{CancelRpcTaskError, CancelRpcTaskRequest, InitRpcTaskResponse, RpcTaskStatusError,
                           RpcTaskStatusRequest};
use rpc_task::{RpcTask, RpcTaskHandle, RpcTaskManager, RpcTaskManagerShared, RpcTaskStatus, RpcTaskTypes};

use crate::coin_balance::HDAddressBalance;
use crate::rpc_command::hd_account_balance_rpc_error::HDAccountBalanceRpcError;
use crate::{lp_coinfind_or_err, CoinsContext, MmCoinEnum};

pub type ScanAddressesUserAction = SerdeInfallible;
pub type ScanAddressesAwaitingStatus = SerdeInfallible;
pub type ScanAddressesTaskManager = RpcTaskManager<InitScanAddressesTask>;
pub type ScanAddressesTaskManagerShared = RpcTaskManagerShared<InitScanAddressesTask>;
pub type ScanAddressesTaskHandle = RpcTaskHandle<InitScanAddressesTask>;
pub type ScanAddressesRpcTaskStatus = RpcTaskStatus<
    ScanAddressesResponse,
    HDAccountBalanceRpcError,
    ScanAddressesInProgressStatus,
    ScanAddressesAwaitingStatus,
>;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScanAddressesResponse {
    pub account_index: u32,
    pub derivation_path: RpcDerivationPath,
    pub new_addresses: Vec<HDAddressBalance>,
}

#[derive(Deserialize)]
pub struct ScanAddressesRequest {
    coin: String,
    #[serde(flatten)]
    params: ScanAddressesParams,
}

#[derive(Clone, Deserialize)]
pub struct ScanAddressesParams {
    pub account_index: u32,
    // The max number of empty addresses in a row.
    // If transactions were sent to an address outside the `gap_limit`, they will not be identified.
    pub gap_limit: Option<u32>,
}

#[derive(Clone, Serialize)]
pub enum ScanAddressesInProgressStatus {
    InProgress,
}

#[async_trait]
pub trait InitScanAddressesRpcOps {
    async fn init_scan_for_new_addresses_rpc(
        &self,
        params: ScanAddressesParams,
    ) -> MmResult<ScanAddressesResponse, HDAccountBalanceRpcError>;
}

pub struct InitScanAddressesTask {
    req: ScanAddressesRequest,
    coin: MmCoinEnum,
}

impl RpcTaskTypes for InitScanAddressesTask {
    type Item = ScanAddressesResponse;
    type Error = HDAccountBalanceRpcError;
    type InProgressStatus = ScanAddressesInProgressStatus;
    type AwaitingStatus = ScanAddressesAwaitingStatus;
    type UserAction = ScanAddressesUserAction;
}

#[async_trait]
impl RpcTask for InitScanAddressesTask {
    #[inline]
    fn initial_status(&self) -> Self::InProgressStatus { ScanAddressesInProgressStatus::InProgress }

    // Do nothing if the task has been cancelled.
    async fn cancel(self) {}

    async fn run(&mut self, _task_handle: &ScanAddressesTaskHandle) -> Result<Self::Item, MmError<Self::Error>> {
        match self.coin {
            MmCoinEnum::UtxoCoin(ref utxo) => utxo.init_scan_for_new_addresses_rpc(self.req.params.clone()).await,
            MmCoinEnum::QtumCoin(ref qtum) => qtum.init_scan_for_new_addresses_rpc(self.req.params.clone()).await,
            _ => MmError::err(HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet),
        }
    }
}

pub async fn init_scan_for_new_addresses(
    ctx: MmArc,
    req: ScanAddressesRequest,
) -> MmResult<InitRpcTaskResponse, HDAccountBalanceRpcError> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let spawner = coin.spawner();
    let coins_ctx = CoinsContext::from_ctx(&ctx).map_to_mm(HDAccountBalanceRpcError::Internal)?;
    let task = InitScanAddressesTask { req, coin };
    let task_id = ScanAddressesTaskManager::spawn_rpc_task(&coins_ctx.scan_addresses_manager, &spawner, task)?;
    Ok(InitRpcTaskResponse { task_id })
}

pub async fn init_scan_for_new_addresses_status(
    ctx: MmArc,
    req: RpcTaskStatusRequest,
) -> MmResult<ScanAddressesRpcTaskStatus, RpcTaskStatusError> {
    let coins_ctx = CoinsContext::from_ctx(&ctx).map_to_mm(RpcTaskStatusError::Internal)?;
    let mut task_manager = coins_ctx
        .scan_addresses_manager
        .lock()
        .map_to_mm(|e| RpcTaskStatusError::Internal(e.to_string()))?;
    task_manager
        .task_status(req.task_id, req.forget_if_finished)
        .or_mm_err(|| RpcTaskStatusError::NoSuchTask(req.task_id))
}

pub async fn cancel_scan_for_new_addresses(
    ctx: MmArc,
    req: CancelRpcTaskRequest,
) -> MmResult<SuccessResponse, CancelRpcTaskError> {
    let coins_ctx = CoinsContext::from_ctx(&ctx).map_to_mm(CancelRpcTaskError::Internal)?;
    let mut task_manager = coins_ctx
        .scan_addresses_manager
        .lock()
        .map_to_mm(|e| CancelRpcTaskError::Internal(e.to_string()))?;
    task_manager.cancel_task(req.task_id)?;
    Ok(SuccessResponse::new())
}

pub mod common_impl {
    use std::fmt;
    use std::ops::DerefMut;

    use super::*;
    use crate::coin_balance::HDWalletBalanceOps;
    use crate::hd_wallet::{HDAccountOps, HDWalletCoinOps, HDWalletOps};
    use crate::CoinWithDerivationMethod;

    pub async fn scan_for_new_addresses_rpc<Coin>(
        coin: &Coin,
        params: ScanAddressesParams,
    ) -> MmResult<ScanAddressesResponse, HDAccountBalanceRpcError>
    where
        Coin: CoinWithDerivationMethod<HDWallet = <Coin as HDWalletCoinOps>::HDWallet> + HDWalletBalanceOps + Sync,
        <Coin as HDWalletCoinOps>::Address: fmt::Display,
    {
        let hd_wallet = coin.derivation_method().hd_wallet_or_err()?;

        let account_id = params.account_index;
        let mut hd_account = hd_wallet
            .get_account_mut(account_id)
            .await
            .or_mm_err(|| HDAccountBalanceRpcError::CoinIsActivatedNotWithHDWallet)?;
        let account_derivation_path = hd_account.account_derivation_path();
        let address_scanner = coin.produce_hd_address_scanner().await?;
        let gap_limit = params.gap_limit.unwrap_or_else(|| hd_wallet.gap_limit());

        let new_addresses = coin
            .scan_for_new_addresses(hd_wallet, hd_account.deref_mut(), &address_scanner, gap_limit)
            .await?;

        Ok(ScanAddressesResponse {
            account_index: account_id,
            derivation_path: RpcDerivationPath(account_derivation_path),
            new_addresses,
        })
    }
}
