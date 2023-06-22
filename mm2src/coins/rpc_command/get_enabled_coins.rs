use crate::CoinsContext;
use common::HttpStatusCode;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmResult;
use mm2_err_handle::prelude::*;

#[derive(Serialize, Display, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetEnabledCoinsError {
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl HttpStatusCode for GetEnabledCoinsError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetEnabledCoinsError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Deserialize)]
pub struct GetEnabledCoinsRequest;

#[derive(Serialize)]
pub struct GetEnabledCoinsResponse {
    coins: Vec<EnabledCoinV2>,
}

#[derive(Serialize)]
pub struct EnabledCoinV2 {
    ticker: String,
}

pub async fn get_enabled_coins(
    ctx: MmArc,
    _req: GetEnabledCoinsRequest,
) -> MmResult<GetEnabledCoinsResponse, GetEnabledCoinsError> {
    let coins_ctx = CoinsContext::from_ctx(&ctx).map_to_mm(GetEnabledCoinsError::Internal)?;
    let coins_map = coins_ctx.coins.lock().await;

    let coins = coins_map
        .iter()
        .map(|(ticker, _coin)| EnabledCoinV2 { ticker: ticker.clone() })
        .collect();
    Ok(GetEnabledCoinsResponse { coins })
}
