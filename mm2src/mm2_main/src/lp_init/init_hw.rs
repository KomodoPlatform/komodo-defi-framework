use crate::mm2::lp_native_dex::init_context::MmInitContext;
use async_trait::async_trait;
use common::mm_ctx::MmArc;
use common::{HttpStatusCode, SuccessResponse};
use crypto::hw_rpc_task::{HwConnectStatuses, HwRpcTaskAwaitingStatus, HwRpcTaskUserAction, HwRpcTaskUserActionRequest,
                          TrezorRpcTaskConnectProcessor};
use crypto::{CryptoCtx, CryptoInitError, HwCtxInitError, HwError, HwWalletType};
use derive_more::Display;
use http::StatusCode;
use mm2_ehandle::mm_error::prelude::*;
use rpc_task::rpc_common::{InitRpcTaskResponse, RpcTaskStatusError, RpcTaskStatusRequest, RpcTaskUserActionError};
use rpc_task::{RpcTask, RpcTaskError, RpcTaskHandle, RpcTaskManager, RpcTaskManagerShared, RpcTaskStatus, RpcTaskTypes};
use std::time::Duration;

const TREZOR_CONNECT_TIMEOUT: Duration = Duration::from_secs(300);
const TREZOR_PIN_TIMEOUT: Duration = Duration::from_secs(600);

pub type InitHwAwaitingStatus = HwRpcTaskAwaitingStatus;
pub type InitHwUserAction = HwRpcTaskUserAction;

pub type InitHwTaskManagerShared = RpcTaskManagerShared<InitHwTask>;
pub type InitHwStatus = RpcTaskStatus<SuccessResponse, InitHwError, InitHwInProgressStatus, InitHwAwaitingStatus>;
type InitHwTaskHandle = RpcTaskHandle<InitHwTask>;

#[derive(Clone, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum InitHwError {
    /* ----------- Trezor device errors ----------- */
    #[display(fmt = "Trezor internal error: {}", _0)]
    TrezorInternal(String),
    #[display(fmt = "No Trezor device available")]
    NoTrezorDeviceAvailable,
    /* ---------------- RPC error ----------------- */
    #[display(fmt = "Hardware Wallet context is initializing already")]
    HwContextInitializingAlready,
    #[display(fmt = "Hardware Wallet context is initialized already")]
    HwContextInitializedAlready,
    #[display(fmt = "RPC timed out {:?}", _0)]
    Timeout(Duration),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
}

impl From<CryptoInitError> for InitHwError {
    fn from(e: CryptoInitError) -> Self { InitHwError::Internal(e.to_string()) }
}

impl From<HwCtxInitError<RpcTaskError>> for InitHwError {
    fn from(e: HwCtxInitError<RpcTaskError>) -> Self {
        match e {
            HwCtxInitError::InitializingAlready => InitHwError::HwContextInitializingAlready,
            HwCtxInitError::InitializedAlready => InitHwError::HwContextInitializedAlready,
            HwCtxInitError::HwError(hw_error) => InitHwError::from(hw_error),
            HwCtxInitError::ProcessorError(rpc) => InitHwError::from(rpc),
        }
    }
}

impl From<HwError> for InitHwError {
    fn from(e: HwError) -> Self {
        match e {
            HwError::NoTrezorDeviceAvailable => InitHwError::NoTrezorDeviceAvailable,
            trezor => InitHwError::TrezorInternal(trezor.to_string()),
        }
    }
}

impl From<RpcTaskError> for InitHwError {
    fn from(e: RpcTaskError) -> Self {
        let error = e.to_string();
        match e {
            RpcTaskError::Canceled => InitHwError::Internal("Canceled".to_owned()),
            RpcTaskError::Timeout(timeout) => InitHwError::Timeout(timeout),
            RpcTaskError::NoSuchTask(_) | RpcTaskError::UnexpectedTaskStatus { .. } => InitHwError::Internal(error),
            RpcTaskError::Internal(internal) => InitHwError::Internal(internal),
        }
    }
}

impl HttpStatusCode for InitHwError {
    fn status_code(&self) -> StatusCode {
        match self {
            InitHwError::HwContextInitializingAlready | InitHwError::HwContextInitializedAlready => {
                StatusCode::BAD_REQUEST
            },
            InitHwError::Timeout(_) => StatusCode::REQUEST_TIMEOUT,
            InitHwError::TrezorInternal(_) | InitHwError::NoTrezorDeviceAvailable | InitHwError::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub enum InitHwInProgressStatus {
    Initializing,
    WaitingForTrezorToConnect,
    ReadPublicKeyFromTrezor,
}

pub struct InitHwTask {
    ctx: MmArc,
    hw_wallet_type: HwWalletType,
}

impl RpcTaskTypes for InitHwTask {
    type Item = SuccessResponse;
    type Error = InitHwError;
    type InProgressStatus = InitHwInProgressStatus;
    type AwaitingStatus = InitHwAwaitingStatus;
    type UserAction = InitHwUserAction;
}

#[async_trait]
impl RpcTask for InitHwTask {
    fn initial_status(&self) -> Self::InProgressStatus { InitHwInProgressStatus::Initializing }

    async fn run(self, task_handle: &InitHwTaskHandle) -> Result<Self::Item, MmError<Self::Error>> {
        let crypto_ctx = CryptoCtx::from_ctx(&self.ctx)?;

        match self.hw_wallet_type {
            HwWalletType::Trezor => {
                let trezor_connect_processor = TrezorRpcTaskConnectProcessor::new(task_handle, HwConnectStatuses {
                    on_connect: InitHwInProgressStatus::WaitingForTrezorToConnect,
                    on_connected: InitHwInProgressStatus::Initializing,
                    on_connection_failed: InitHwInProgressStatus::Initializing,
                    on_button_request: InitHwInProgressStatus::ReadPublicKeyFromTrezor,
                    on_pin_request: InitHwAwaitingStatus::WaitForTrezorPin,
                    on_ready: InitHwInProgressStatus::Initializing,
                })
                .with_connect_timeout(TREZOR_CONNECT_TIMEOUT)
                .with_pin_timeout(TREZOR_PIN_TIMEOUT);

                crypto_ctx.init_hw_ctx_with_trezor(&trezor_connect_processor).await?;
            },
        }
        Ok(SuccessResponse::new())
    }
}

#[derive(Deserialize)]
pub struct InitTrezorRequest;

pub async fn init_trezor(ctx: MmArc, _req: InitTrezorRequest) -> MmResult<InitRpcTaskResponse, InitHwError> {
    let init_ctx = MmInitContext::from_ctx(&ctx).map_to_mm(InitHwError::Internal)?;
    let task = InitHwTask {
        ctx,
        hw_wallet_type: HwWalletType::Trezor,
    };
    let task_id = RpcTaskManager::spawn_rpc_task(&init_ctx.init_hw_task_manager, task)?;
    Ok(InitRpcTaskResponse { task_id })
}

pub async fn init_trezor_status(ctx: MmArc, req: RpcTaskStatusRequest) -> MmResult<InitHwStatus, RpcTaskStatusError> {
    let coins_ctx = MmInitContext::from_ctx(&ctx).map_to_mm(RpcTaskStatusError::Internal)?;
    let mut task_manager = coins_ctx
        .init_hw_task_manager
        .lock()
        .map_to_mm(|e| RpcTaskStatusError::Internal(e.to_string()))?;
    task_manager
        .task_status(req.task_id, req.forget_if_finished)
        .or_mm_err(|| RpcTaskStatusError::NoSuchTask(req.task_id))
}

pub async fn init_trezor_user_action(
    ctx: MmArc,
    req: HwRpcTaskUserActionRequest,
) -> MmResult<SuccessResponse, RpcTaskUserActionError> {
    let coins_ctx = MmInitContext::from_ctx(&ctx).map_to_mm(RpcTaskUserActionError::Internal)?;
    let mut task_manager = coins_ctx
        .init_hw_task_manager
        .lock()
        .map_to_mm(|e| RpcTaskUserActionError::Internal(e.to_string()))?;
    task_manager.on_user_action(req.task_id, req.user_action)?;
    Ok(SuccessResponse::new())
}
