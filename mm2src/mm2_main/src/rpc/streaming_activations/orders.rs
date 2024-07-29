//! RPC activation and deactivation of the order status streamer.
use super::EnableStreamingResponse;
use crate::mm2::lp_ordermatch::order_events::OrderStatusStreamer;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::{map_to_mm::MapToMmResult, mm_error::MmResult};

use common::HttpStatusCode;
use http::StatusCode;

#[derive(Deserialize)]
pub struct EnableOrderStatusStreamingRequest {
    pub client_id: u64,
}

#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum OrderStatusStreamingRequestError {
    EnableError(String),
}

impl HttpStatusCode for OrderStatusStreamingRequestError {
    fn status_code(&self) -> StatusCode { StatusCode::BAD_REQUEST }
}

pub async fn enable_order_status(
    ctx: MmArc,
    req: EnableOrderStatusStreamingRequest,
) -> MmResult<EnableStreamingResponse, OrderStatusStreamingRequestError> {
    let order_status_streamer = OrderStatusStreamer::new();
    ctx.event_stream_manager
        .add(req.client_id, order_status_streamer, ctx.spawner())
        .await
        .map(EnableStreamingResponse::new)
        .map_to_mm(|e| OrderStatusStreamingRequestError::EnableError(format!("{e:?}")))
}
