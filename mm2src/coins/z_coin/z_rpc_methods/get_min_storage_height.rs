use crate::{lp_coinfind_or_err, CoinFindError, MmCoinEnum};
use common::HttpStatusCode;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;

#[derive(Serialize, Display, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetMinimumHeightError {
    NoSuchCoin(String),
    #[display(fmt = "Requested coin: {}; is not supported for this action.", _0)]
    NotSupportedCoin(String),
    BuildingWalletDb(String),
    UpdatingBlocksCache(String),
    TemporaryError(String),
    StorageError(String),
}

impl HttpStatusCode for GetMinimumHeightError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetMinimumHeightError::NoSuchCoin(_) => StatusCode::NOT_FOUND,
            GetMinimumHeightError::NotSupportedCoin(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for GetMinimumHeightError {
    fn from(err: CoinFindError) -> Self { Self::NoSuchCoin(err.to_string()) }
}

#[derive(Deserialize)]
pub struct GetMinimumHeightRequest {
    coin: String,
}

#[derive(Serialize)]
pub struct GetMinimumHeightResponse {
    pub(crate) status: String,
    pub(crate) height: Option<u32>,
}

pub async fn get_minimum_header_from_cache(
    ctx: MmArc,
    req: GetMinimumHeightRequest,
) -> MmResult<GetMinimumHeightResponse, GetMinimumHeightError> {
    return match lp_coinfind_or_err(&ctx, &req.coin).await? {
        MmCoinEnum::ZCoin(zcoin) => zcoin.get_minimum_header_from_cache().await,
        _ => MmError::err(GetMinimumHeightError::NotSupportedCoin(req.coin)),
    };
}
