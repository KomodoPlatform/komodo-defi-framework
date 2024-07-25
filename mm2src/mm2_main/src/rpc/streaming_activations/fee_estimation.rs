//! RPC activation and deactivation for different fee estimation streamers.
use super::{DisableStreamingRequest, EnableStreamingResponse};

use coins::eth::fee_estimation::eth_fee_events::EthFeeEventStreamer;
use coins::{lp_coinfind, MmCoinEnum};
use common::HttpStatusCode;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::{map_to_mm::MapToMmResult, mm_error::MmResult};

use serde_json::Value as Json;

#[derive(Deserialize)]
pub struct EnableFeeStreamingRequest {
    pub client_id: u64,
    pub coin: String,
    pub config: Option<Json>,
}

#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum FeeStreamingRequestError {
    EnableError(String),
    DisableError(String),
    CoinNotFound,
    CoinNotSupported,
    Internal(String),
}

impl HttpStatusCode for FeeStreamingRequestError {
    fn status_code(&self) -> StatusCode {
        match self {
            FeeStreamingRequestError::EnableError(_) => StatusCode::BAD_REQUEST,
            FeeStreamingRequestError::DisableError(_) => StatusCode::BAD_REQUEST,
            FeeStreamingRequestError::CoinNotFound => StatusCode::NOT_FOUND,
            FeeStreamingRequestError::CoinNotSupported => StatusCode::NOT_IMPLEMENTED,
            FeeStreamingRequestError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub async fn enable_fee_estimation(
    ctx: MmArc,
    req: EnableFeeStreamingRequest,
) -> MmResult<EnableStreamingResponse, FeeStreamingRequestError> {
    let coin = lp_coinfind(&ctx, &req.coin)
        .await
        .map_err(FeeStreamingRequestError::Internal)?
        .ok_or(FeeStreamingRequestError::CoinNotFound)?;

    match coin {
        MmCoinEnum::EthCoin(coin) => {
            let eth_fee_estimator_streamer = EthFeeEventStreamer::try_new(req.config, coin)
                .map_to_mm(|e| FeeStreamingRequestError::EnableError(format!("{e:?}")))?;
            ctx.event_stream_manager
                .add(req.client_id, eth_fee_estimator_streamer, ctx.spawner())
                .await
                .map(EnableStreamingResponse::new)
                .map_to_mm(|e| FeeStreamingRequestError::EnableError(format!("{e:?}")))
        },
        _ => Err(FeeStreamingRequestError::CoinNotSupported)?,
    }
}

pub async fn disable_fee_estimation(
    ctx: MmArc,
    req: DisableStreamingRequest,
) -> MmResult<(), FeeStreamingRequestError> {
    ctx.event_stream_manager
        .stop(req.client_id, &req.streamer_id)
        .map_to_mm(|e| FeeStreamingRequestError::DisableError(format!("{e:?}")))
}
