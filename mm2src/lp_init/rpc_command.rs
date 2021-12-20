use crate::mm2::lp_native_dex::init_context::MmInitContext;
use crate::mm2::lp_native_dex::mm_init_task::{MmInitStatus, MmInitUserAction};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::{HttpStatusCode, SuccessResponse};
use derive_more::Display;
use http::StatusCode;
use rpc_task::RpcTaskError;

const FORGET_INIT_RESULT_IF_FINISHED: bool = false;

#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum MmInitStatusError {
    InitializationNotStartedYet,
    Internal(String),
}

impl HttpStatusCode for MmInitStatusError {
    fn status_code(&self) -> StatusCode { StatusCode::INTERNAL_SERVER_ERROR }
}

#[derive(Deserialize)]
pub struct MmInitStatusReq;

pub async fn mm_init_status(ctx: MmArc, _req: MmInitStatusReq) -> Result<MmInitStatus, MmError<MmInitStatusError>> {
    let init_ctx = MmInitContext::from_ctx(&ctx).map_to_mm(|_| MmInitStatusError::InitializationNotStartedYet)?;
    let mut task_manager = init_ctx
        .mm_init_task_manager
        .lock()
        .map_to_mm(|poison| MmInitStatusError::Internal(poison.to_string()))?;

    let init_task_id = *init_ctx
        .mm_init_task_id
        .ok_or(MmInitStatusError::InitializationNotStartedYet)?;
    task_manager
        .task_status(init_task_id, FORGET_INIT_RESULT_IF_FINISHED)
        .or_mm_err(|| MmInitStatusError::InitializationNotStartedYet)
}

#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum MmInitUserActionError {
    InitializationNotStartedYet,
    // UnexpectedUserAction,
    Internal(String),
}

impl From<RpcTaskError> for MmInitUserActionError {
    fn from(e: RpcTaskError) -> Self {
        match e {
            RpcTaskError::NoSuchTask(_) => MmInitUserActionError::InitializationNotStartedYet,
            error => MmInitUserActionError::Internal(error.to_string()),
        }
    }
}

impl HttpStatusCode for MmInitUserActionError {
    fn status_code(&self) -> StatusCode { StatusCode::INTERNAL_SERVER_ERROR }
}

pub async fn mm_init_user_action(
    ctx: MmArc,
    req: MmInitUserAction,
) -> Result<SuccessResponse, MmError<MmInitUserActionError>> {
    let init_ctx = MmInitContext::from_ctx(&ctx).map_to_mm(|_| MmInitUserActionError::InitializationNotStartedYet)?;
    let mut task_manager = init_ctx
        .mm_init_task_manager
        .lock()
        .map_to_mm(|poison| MmInitUserActionError::Internal(poison.to_string()))?;

    let init_task_id = *init_ctx
        .mm_init_task_id
        .ok_or(MmInitUserActionError::InitializationNotStartedYet)?;

    task_manager.on_user_action(init_task_id, req)?;
    Ok(SuccessResponse::new())
}
