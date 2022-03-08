use crate::context::CoinsActivationContext;
use crate::prelude::{coin_conf_with_protocol, TryFromCoinProtocol};
use crate::standalone_coin::init_standalone_coin_error::{InitStandaloneCoinError, InitStandaloneCoinStatusError,
                                                         InitStandaloneCoinUserActionError};
use async_trait::async_trait;
use coins::{lp_coinfind, MmCoinEnum, PrivKeyBuildPolicy};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::{NotSame, SuccessResponse};
use crypto::trezor::trezor_rpc_task::RpcTaskHandle;
use crypto::CryptoCtx;
use rpc_task::rpc_common::{InitRpcTaskResponse, RpcTaskStatusRequest, RpcTaskUserActionRequest};
use rpc_task::{RpcTask, RpcTaskManager, RpcTaskManagerShared, RpcTaskStatus, RpcTaskTypes};
use serde_derive::Deserialize;
use serde_json::Value as Json;
use std::sync::Arc;

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
    type ActivationRequest: Sync + Send;
    type StandaloneProtocol: TryFromCoinProtocol + Send;
    // The following types are related to `RpcTask` management.
    type ActivationResult: serde::Serialize + Clone + Send + Sync + 'static;
    type ActivationError: Into<InitStandaloneCoinError> + SerMmErrorType + NotMmError + Clone + Send + Sync + 'static;
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
        priv_key_policy: PrivKeyBuildPolicy<'_>,
        task_handle: &InitStandaloneCoinTaskHandle<Self>,
    ) -> Result<Self, MmError<Self::ActivationError>>;

    async fn get_activation_result(
        &self,
        ctx: MmArc,
        task_handle: &InitStandaloneCoinTaskHandle<Self>,
        activation_request: &Self::ActivationRequest,
    ) -> Result<Self::ActivationResult, MmError<Self::ActivationError>>;
}

pub async fn init_standalone_coin<Standalone>(
    ctx: MmArc,
    request: InitStandaloneCoinReq<Standalone::ActivationRequest>,
) -> MmResult<InitStandaloneCoinResponse, InitStandaloneCoinError>
where
    Standalone: InitStandaloneCoinActivationOps + Send + Sync + 'static,
    Standalone::InProgressStatus: InitStandaloneCoinInitialStatus,
    InitStandaloneCoinError: From<Standalone::ActivationError>,
    (Standalone::ActivationError, InitStandaloneCoinError): NotSame,
{
    let crypto_ctx = CryptoCtx::from_ctx(&ctx).mm_err(|e| InitStandaloneCoinError::Internal(e.to_string()))?;

    if let Ok(Some(_)) = lp_coinfind(&ctx, &request.ticker).await {
        return MmError::err(InitStandaloneCoinError::CoinIsAlreadyActivated { ticker: request.ticker });
    }

    let (coin_conf, protocol_info) = coin_conf_with_protocol(&ctx, &request.ticker)?;

    let coins_act_ctx = CoinsActivationContext::from_ctx(&ctx).map_to_mm(InitStandaloneCoinError::Internal)?;
    let task = InitStandaloneCoinTask::<Standalone> {
        ctx,
        crypto_ctx,
        request,
        coin_conf,
        protocol_info,
    };
    let task_manager = Standalone::rpc_task_manager(&coins_act_ctx);

    let task_id = RpcTaskManager::spawn_rpc_task(task_manager, task)
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

pub struct InitStandaloneCoinTask<Standalone: InitStandaloneCoinActivationOps> {
    ctx: MmArc,
    crypto_ctx: Arc<CryptoCtx>,
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

    async fn run(self, task_handle: &RpcTaskHandle<Self>) -> Result<Self::Item, MmError<Self::Error>> {
        let priv_key_policy = PrivKeyBuildPolicy::from_crypto_ctx(&self.crypto_ctx);
        let coin = Standalone::init_standalone_coin(
            self.ctx.clone(),
            self.request.ticker,
            self.coin_conf,
            &self.request.activation_params,
            self.protocol_info,
            priv_key_policy,
            task_handle,
        )
        .await?;

        coin.get_activation_result(self.ctx, task_handle, &self.request.activation_params)
            .await
    }
}

pub trait InitStandaloneCoinInitialStatus {
    fn initial_status() -> Self;
}
