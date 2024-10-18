//! RPC activation and deactivation of the swap status streamer.
use super::{EnableStreamingRequest, EnableStreamingResponse};
use crate::lp_swap::swap_events::SwapStatusStreamer;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::{map_to_mm::MapToMmResult, mm_error::MmResult};

use common::HttpStatusCode;
use http::StatusCode;

#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum SwapStatusStreamingRequestError {
    EnableError(String),
}

impl HttpStatusCode for SwapStatusStreamingRequestError {
    fn status_code(&self) -> StatusCode {
        match self {
            SwapStatusStreamingRequestError::EnableError(_) => StatusCode::BAD_REQUEST,
        }
    }
}

pub async fn enable_swap_status(
    ctx: MmArc,
    req: EnableStreamingRequest<()>,
) -> MmResult<EnableStreamingResponse, SwapStatusStreamingRequestError> {
    ctx.event_stream_manager
        .add(req.client_id, SwapStatusStreamer::new(), ctx.spawner())
        .await
        .map(EnableStreamingResponse::new)
        .map_to_mm(|e| SwapStatusStreamingRequestError::EnableError(format!("{e:?}")))
}
