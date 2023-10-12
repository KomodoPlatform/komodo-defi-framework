use std::collections::HashMap;

use async_trait::async_trait;
use coins::my_tx_history_v2::TxHistoryStorage;
use coins::tx_history_storage::{CreateTxHistoryStorageError, TxHistoryStorageBuilder};
use coins::{lp_coinfind, lp_register_coin, CoinsContext, MmCoinEnum, RegisterCoinError, RegisterCoinParams};
use common::{log, SuccessResponse};
use crypto::trezor::trezor_rpc_task::RpcTaskHandle;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_metrics::MetricsArc;
use mm2_number::BigDecimal;
use rpc_task::rpc_common::{CancelRpcTaskRequest, InitRpcTaskResponse, RpcTaskStatusRequest, RpcTaskUserActionRequest};
use rpc_task::{RpcTask, RpcTaskManager, RpcTaskManagerShared, RpcTaskStatus, RpcTaskTypes};
use serde_derive::Deserialize;
use serde_json::Value as Json;

use crate::context::CoinsActivationContext;
use crate::prelude::*;
use crate::standalone_coin::init_standalone_coin_error::{CancelInitStandaloneCoinError, InitStandaloneCoinError,
                                                         InitStandaloneCoinStatusError,
                                                         InitStandaloneCoinUserActionError};

pub type InitStandaloneCoinResponse = InitRpcTaskResponse;
pub type InitStandaloneCoinStatusRequest = RpcTaskStatusRequest;
pub type InitStandaloneCoinUserActionRequest<UserAction> = RpcTaskUserActionRequest<UserAction>;
pub type InitStandaloneCoinTaskManagerShared<Standalone> = RpcTaskManagerShared<InitStandaloneCoinTask<Standalone>>;
pub type InitStandaloneCoinTaskHandle<Standalone> = RpcTaskHandle<InitStandaloneCoinTask<Standalone>>;

#[derive(Debug, Deserialize)]
pub struct InitStandaloneCoinReq<T> {
    ticker: String,
    activation_params: T,
}

#[async_trait]
pub trait InitStandaloneCoinActivationOps: Into<MmCoinEnum> + Send + Sync + 'static {
    type ActivationRequest: TxHistory + Clone + Send + Sync;
    type StandaloneProtocol: TryFromCoinProtocol + Clone + Send + Sync;
    // The following types are related to `RpcTask` management.
    type ActivationResult: serde::Serialize + Clone + CurrentBlock + GetAddressesBalances + Send + Sync + 'static;
    type ActivationError: From<RegisterCoinError>
        + From<CreateTxHistoryStorageError>
        + Into<InitStandaloneCoinError>
        + SerMmErrorType
        + NotEqual
        + Clone
        + Send
        + Sync
        + 'static;
    type InProgressStatus: InitStandaloneCoinInitialStatus + serde::Serialize + Clone + Send + Sync + 'static;
    type AwaitingStatus: serde::Serialize + Clone + Send + Sync + 'static;
    type UserAction: serde::de::DeserializeOwned + NotMmError + Send + Sync + 'static;

    fn rpc_task_manager(activation_ctx: &CoinsActivationContext) -> &InitStandaloneCoinTaskManagerShared<Self>;

    /// Initialization of the standalone coin spawned as `RpcTask`.
    async fn init_standalone_coin(
        ctx: MmArc,
        ticker: String,
        coin_conf: Json,
        activation_request: &Self::ActivationRequest,
        protocol_info: Self::StandaloneProtocol,
        task_handle: &InitStandaloneCoinTaskHandle<Self>,
    ) -> Result<Self, MmError<Self::ActivationError>>;

    async fn get_activation_result(
        &self,
        ctx: MmArc,
        task_handle: &InitStandaloneCoinTaskHandle<Self>,
        activation_request: &Self::ActivationRequest,
    ) -> Result<Self::ActivationResult, MmError<Self::ActivationError>>;

    fn start_history_background_fetching(
        &self,
        metrics: MetricsArc,
        storage: impl TxHistoryStorage,
        current_balances: HashMap<String, BigDecimal>,
    );
}

pub async fn init_standalone_coin<Standalone>(
    ctx: MmArc,
    request: InitStandaloneCoinReq<Standalone::ActivationRequest>,
) -> MmResult<InitStandaloneCoinResponse, InitStandaloneCoinError>
where
    Standalone: InitStandaloneCoinActivationOps + Send + Sync + 'static,
    Standalone::InProgressStatus: InitStandaloneCoinInitialStatus,
    InitStandaloneCoinError: From<Standalone::ActivationError>,
    (Standalone::ActivationError, InitStandaloneCoinError): NotEqual,
{
    if let Ok(Some(_)) = lp_coinfind(&ctx, &request.ticker).await {
        return MmError::err(InitStandaloneCoinError::CoinIsAlreadyActivated { ticker: request.ticker });
    }

    let (coin_conf, protocol_info) = coin_conf_with_protocol(&ctx, &request.ticker)?;

    let coins_act_ctx = CoinsActivationContext::from_ctx(&ctx).map_to_mm(InitStandaloneCoinError::Internal)?;
    let spawner = ctx.spawner();
    let task = InitStandaloneCoinTask::<Standalone> {
        ctx,
        request,
        coin_conf,
        protocol_info,
    };
    let task_manager = Standalone::rpc_task_manager(&coins_act_ctx);

    let task_id = RpcTaskManager::spawn_rpc_task(task_manager, &spawner, task)
        .mm_err(|e| InitStandaloneCoinError::Internal(e.to_string()))?;

    Ok(InitStandaloneCoinResponse { task_id })
}

pub async fn init_standalone_coin_status<Standalone: InitStandaloneCoinActivationOps>(
    ctx: MmArc,
    req: InitStandaloneCoinStatusRequest,
) -> MmResult<
    RpcTaskStatus<
        Standalone::ActivationResult,
        InitStandaloneCoinError,
        Standalone::InProgressStatus,
        Standalone::AwaitingStatus,
    >,
    InitStandaloneCoinStatusError,
>
where
    InitStandaloneCoinError: From<Standalone::ActivationError>,
{
    let coins_act_ctx = CoinsActivationContext::from_ctx(&ctx).map_to_mm(InitStandaloneCoinStatusError::Internal)?;
    let mut task_manager = Standalone::rpc_task_manager(&coins_act_ctx)
        .lock()
        .map_to_mm(|poison| InitStandaloneCoinStatusError::Internal(poison.to_string()))?;
    task_manager
        .task_status(req.task_id, req.forget_if_finished)
        .or_mm_err(|| InitStandaloneCoinStatusError::NoSuchTask(req.task_id))
        .map(|rpc_task| rpc_task.map_err(InitStandaloneCoinError::from))
}

pub async fn init_standalone_coin_user_action<Standalone: InitStandaloneCoinActivationOps>(
    ctx: MmArc,
    req: InitStandaloneCoinUserActionRequest<Standalone::UserAction>,
) -> MmResult<SuccessResponse, InitStandaloneCoinUserActionError> {
    let coins_act_ctx =
        CoinsActivationContext::from_ctx(&ctx).map_to_mm(InitStandaloneCoinUserActionError::Internal)?;
    let mut task_manager = Standalone::rpc_task_manager(&coins_act_ctx)
        .lock()
        .map_to_mm(|poison| InitStandaloneCoinUserActionError::Internal(poison.to_string()))?;
    task_manager.on_user_action(req.task_id, req.user_action)?;
    Ok(SuccessResponse::new())
}

pub async fn cancel_init_standalone_coin<Standalone: InitStandaloneCoinActivationOps>(
    ctx: MmArc,
    req: CancelRpcTaskRequest,
) -> MmResult<SuccessResponse, CancelInitStandaloneCoinError> {
    let coins_act_ctx = CoinsActivationContext::from_ctx(&ctx).map_to_mm(CancelInitStandaloneCoinError::Internal)?;
    let mut task_manager = Standalone::rpc_task_manager(&coins_act_ctx)
        .lock()
        .map_to_mm(|poison| CancelInitStandaloneCoinError::Internal(poison.to_string()))?;
    task_manager.cancel_task(req.task_id)?;
    Ok(SuccessResponse::new())
}

pub struct InitStandaloneCoinTask<Standalone: InitStandaloneCoinActivationOps> {
    ctx: MmArc,
    request: InitStandaloneCoinReq<Standalone::ActivationRequest>,
    coin_conf: Json,
    protocol_info: Standalone::StandaloneProtocol,
}

impl<Standalone: InitStandaloneCoinActivationOps> RpcTaskTypes for InitStandaloneCoinTask<Standalone> {
    type Item = Standalone::ActivationResult;
    type Error = Standalone::ActivationError;
    type InProgressStatus = Standalone::InProgressStatus;
    type AwaitingStatus = Standalone::AwaitingStatus;
    type UserAction = Standalone::UserAction;
}

#[async_trait]
impl<Standalone> RpcTask for InitStandaloneCoinTask<Standalone>
where
    Standalone: InitStandaloneCoinActivationOps,
{
    fn initial_status(&self) -> Self::InProgressStatus {
        <Standalone::InProgressStatus as InitStandaloneCoinInitialStatus>::initial_status()
    }

    /// Try to disable the coin in case if we managed to register it already.
    async fn cancel(self) {
        if let Ok(c_ctx) = CoinsContext::from_ctx(&self.ctx) {
            if let Ok(Some(coin)) = lp_coinfind(&self.ctx, &self.request.ticker).await {
                c_ctx.remove_coin(coin).await;
            };
        };
    }

    async fn run(&mut self, task_handle: &RpcTaskHandle<Self>) -> Result<Self::Item, MmError<Self::Error>> {
        let ticker = self.request.ticker.clone();
        let coin = Standalone::init_standalone_coin(
            self.ctx.clone(),
            ticker.clone(),
            self.coin_conf.clone(),
            &self.request.activation_params,
            self.protocol_info.clone(),
            task_handle,
        )
        .await?;

        let result = coin
            .get_activation_result(self.ctx.clone(), task_handle, &self.request.activation_params)
            .await?;
        log::info!("{} current block {}", ticker, result.current_block());

        let tx_history = self.request.activation_params.tx_history();
        if tx_history {
            let current_balances = result.get_addresses_balances();
            coin.start_history_background_fetching(
                self.ctx.metrics.clone(),
                TxHistoryStorageBuilder::new(&self.ctx).build()?,
                current_balances,
            );
        }

        lp_register_coin(&self.ctx, coin.into(), RegisterCoinParams { ticker }).await?;

        Ok(result)
    }
}

pub trait InitStandaloneCoinInitialStatus {
    fn initial_status() -> Self;
}
