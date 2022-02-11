use crate::coin_balance::HDAccountBalance;
use crate::hd_pubkey::{HDXPubExtractor, RpcTaskXPubExtractor};
use crate::hd_wallet::HDWalletRpcError;
use crate::{lp_coinfind_or_err, CoinBalance, CoinWithDerivationMethod, CoinsContext, MmCoinEnum};
use async_trait::async_trait;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::SuccessResponse;
use crypto::hw_rpc_task::{HwConnectStatuses, HwRpcTaskAwaitingStatus, HwRpcTaskUserAction, HwRpcTaskUserActionRequest};
use crypto::RpcDerivationPath;
use rpc_task::rpc_common::{InitRpcTaskResponse, RpcTaskStatusError, RpcTaskStatusRequest, RpcTaskUserActionError};
use rpc_task::{RpcTask, RpcTaskHandle, RpcTaskManager, RpcTaskManagerShared, RpcTaskStatus};

pub type CreateAccountUserAction = HwRpcTaskUserAction;
pub type CreateAccountAwaitingStatus = HwRpcTaskAwaitingStatus;
pub type CreateAccountTaskManager = RpcTaskManager<
    HDAccountBalance,
    HDWalletRpcError,
    CreateAccountInProgressStatus,
    CreateAccountAwaitingStatus,
    CreateAccountUserAction,
>;
pub type CreateAccountTaskManagerShared = RpcTaskManagerShared<
    HDAccountBalance,
    HDWalletRpcError,
    CreateAccountInProgressStatus,
    CreateAccountAwaitingStatus,
    CreateAccountUserAction,
>;
pub type CreateAccountTaskHandle = RpcTaskHandle<
    HDAccountBalance,
    HDWalletRpcError,
    CreateAccountInProgressStatus,
    CreateAccountAwaitingStatus,
    CreateAccountUserAction,
>;
pub type CreateAccountRpcTaskStatus =
    RpcTaskStatus<HDAccountBalance, HDWalletRpcError, CreateAccountInProgressStatus, CreateAccountAwaitingStatus>;

type CreateAccountXPubExtractor<'task> = RpcTaskXPubExtractor<
    'task,
    HDAccountBalance,
    HDWalletRpcError,
    CreateAccountInProgressStatus,
    CreateAccountAwaitingStatus,
    CreateAccountUserAction,
>;

#[derive(Deserialize)]
pub struct CreateNewAccountRequest {
    coin: String,
    #[serde(flatten)]
    params: CreateNewAccountParams,
}

#[derive(Deserialize)]
pub struct CreateNewAccountParams {
    gap_limit: Option<u32>,
}

#[derive(Clone, Serialize)]
pub enum CreateAccountInProgressStatus {
    Preparing,
    RequestingAccountBalance,
    Finishing,
    /// The following statuses don't require the user to send `UserAction`,
    /// but they tell the user that he should confirm/decline the operation on his device.
    WaitingForTrezorToConnect,
    WaitingForUserToConfirmPubkey,
}

#[async_trait]
pub trait InitCreateHDAccountRpcOps {
    async fn init_create_account_rpc<XPubExtractor>(
        &self,
        params: CreateNewAccountParams,
        xpub_extractor: &XPubExtractor,
    ) -> MmResult<HDAccountBalance, HDWalletRpcError>
    where
        XPubExtractor: HDXPubExtractor + Sync;
}

pub struct InitCreateAccountTask<Coin> {
    ctx: MmArc,
    coin: Coin,
    req: CreateNewAccountRequest,
}

#[async_trait]
impl<Coin> RpcTask for InitCreateAccountTask<Coin>
where
    Coin: InitCreateHDAccountRpcOps + Send + Sync,
{
    type Item = HDAccountBalance;
    type Error = HDWalletRpcError;
    type InProgressStatus = CreateAccountInProgressStatus;
    type AwaitingStatus = CreateAccountAwaitingStatus;
    type UserAction = CreateAccountUserAction;

    fn initial_status(&self) -> Self::InProgressStatus { CreateAccountInProgressStatus::Preparing }

    async fn run(self, task_handle: &CreateAccountTaskHandle) -> Result<Self::Item, MmError<Self::Error>> {
        let hw_statuses = HwConnectStatuses {
            on_connect: CreateAccountInProgressStatus::WaitingForTrezorToConnect,
            on_connected: CreateAccountInProgressStatus::Preparing,
            on_connection_failed: CreateAccountInProgressStatus::Finishing,
            on_button_request: CreateAccountInProgressStatus::WaitingForUserToConfirmPubkey,
            on_pin_request: CreateAccountAwaitingStatus::WaitForTrezorPin,
            on_ready: CreateAccountInProgressStatus::RequestingAccountBalance,
        };
        let xpub_extractor = CreateAccountXPubExtractor::new(&self.ctx, task_handle, hw_statuses)?;
        self.coin
            .init_create_account_rpc(self.req.params, &xpub_extractor)
            .await
    }
}

pub async fn init_create_new_account(
    ctx: MmArc,
    req: CreateNewAccountRequest,
) -> MmResult<InitRpcTaskResponse, HDWalletRpcError> {
    async fn init_create_new_account_helper<Coin>(
        ctx: MmArc,
        coin: Coin,
        req: CreateNewAccountRequest,
    ) -> MmResult<InitRpcTaskResponse, HDWalletRpcError>
    where
        Coin: InitCreateHDAccountRpcOps + Send + Sync + 'static,
    {
        let coins_ctx = CoinsContext::from_ctx(&ctx).map_to_mm(HDWalletRpcError::Internal)?;
        let task = InitCreateAccountTask { ctx, coin, req };
        let task_id = CreateAccountTaskManager::spawn_rpc_task(&coins_ctx.create_account_manager, task)?;
        Ok(InitRpcTaskResponse { task_id })
    }

    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    match coin {
        MmCoinEnum::UtxoCoin(utxo) => init_create_new_account_helper(ctx, utxo, req).await,
        MmCoinEnum::QtumCoin(qtum) => init_create_new_account_helper(ctx, qtum, req).await,
        _ => MmError::err(HDWalletRpcError::ExpectedHDWalletDerivationMethod { coin: req.coin }),
    }
}

pub async fn init_create_new_account_status(
    ctx: MmArc,
    req: RpcTaskStatusRequest,
) -> MmResult<CreateAccountRpcTaskStatus, RpcTaskStatusError> {
    let coins_ctx = CoinsContext::from_ctx(&ctx).map_to_mm(RpcTaskStatusError::Internal)?;
    let mut task_manager = coins_ctx
        .create_account_manager
        .lock()
        .map_to_mm(|e| RpcTaskStatusError::Internal(e.to_string()))?;
    task_manager
        .task_status(req.task_id, req.forget_if_finished)
        .or_mm_err(|| RpcTaskStatusError::NoSuchTask(req.task_id))
}

pub async fn init_create_new_account_user_action(
    ctx: MmArc,
    req: HwRpcTaskUserActionRequest,
) -> MmResult<SuccessResponse, RpcTaskUserActionError> {
    let coins_ctx = CoinsContext::from_ctx(&ctx).map_to_mm(RpcTaskUserActionError::Internal)?;
    let mut task_manager = coins_ctx
        .create_account_manager
        .lock()
        .map_to_mm(|e| RpcTaskUserActionError::Internal(e.to_string()))?;
    task_manager.on_user_action(req.task_id, req.user_action)?;
    Ok(SuccessResponse::new())
}

pub(crate) mod common_impl {
    use super::*;
    use crate::coin_balance::HDWalletBalanceOps;
    use crate::hd_wallet::{HDAccountOps, HDWalletCoinOps, HDWalletOps};
    use crate::MarketCoinOps;

    pub async fn init_create_new_account_rpc<'a, Coin, XPubExtractor>(
        coin: &Coin,
        params: CreateNewAccountParams,
        xpub_extractor: &XPubExtractor,
    ) -> MmResult<HDAccountBalance, HDWalletRpcError>
    where
        Coin: HDWalletBalanceOps
            + CoinWithDerivationMethod<HDWallet = <Coin as HDWalletCoinOps>::HDWallet>
            + Send
            + Sync
            + MarketCoinOps,
        XPubExtractor: HDXPubExtractor + Sync,
    {
        let hd_wallet =
            coin.derivation_method()
                .hd_wallet()
                .or_mm_err(|| HDWalletRpcError::ExpectedHDWalletDerivationMethod {
                    coin: coin.ticker().to_owned(),
                })?;

        let mut new_account = coin.create_new_account(hd_wallet, xpub_extractor).await?;
        let address_checker = coin.produce_hd_address_checker().await?;
        let account_index = new_account.account_id();
        let account_derivation_path = new_account.account_derivation_path();

        let gap_limit = params.gap_limit.unwrap_or_else(|| hd_wallet.gap_limit());
        let addresses = coin
            .scan_for_new_addresses(&mut new_account, &address_checker, gap_limit)
            .await?;

        let total_balance = addresses
            .iter()
            .fold(CoinBalance::default(), |total_balance, address_balance| {
                total_balance + address_balance.balance.clone()
            });

        Ok(HDAccountBalance {
            account_index,
            derivation_path: RpcDerivationPath(account_derivation_path),
            total_balance,
            addresses,
        })
    }
}
