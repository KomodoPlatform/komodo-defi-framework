use common::{HttpStatusCode, StatusCode};
use derive_more::Display;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;

use crate::{lp_coinfind_or_err,
            utxo::{rpc_clients::UtxoRpcError, UtxoCommonOps},
            CoinFindError, MmCoinEnum};

pub type GetCurrentMtpRpcResult<T> = Result<T, MmError<GetCurrentMtpError>>;

#[derive(Deserialize)]
pub struct GetCurrentMtpRequest {
    coin: String,
}

#[derive(Serialize)]
pub struct GetCurrentMtpResponse {
    mtp: u32,
}

#[derive(Serialize, Display, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetCurrentMtpError {
    NotSupported(String),
    Internal(String),
}

impl HttpStatusCode for GetCurrentMtpError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetCurrentMtpError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            GetCurrentMtpError::NotSupported(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<UtxoRpcError> for GetCurrentMtpError {
    fn from(_: UtxoRpcError) -> Self { GetCurrentMtpError::Internal("Unable to get current mtp".to_string()) }
}

impl From<CoinFindError> for GetCurrentMtpError {
    fn from(_: CoinFindError) -> Self { GetCurrentMtpError::Internal("Coin not founded or not activated".to_string()) }
}

pub async fn get_current_mtp_rpc(
    ctx: MmArc,
    req: GetCurrentMtpRequest,
) -> GetCurrentMtpRpcResult<GetCurrentMtpResponse> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;

    match coin {
        MmCoinEnum::UtxoCoin(utxo) => Ok(GetCurrentMtpResponse {
            mtp: utxo.get_current_mtp().await?,
        }),
        MmCoinEnum::QtumCoin(qtum) => Ok(GetCurrentMtpResponse {
            mtp: qtum.get_current_mtp().await?,
        }),
        MmCoinEnum::Qrc20Coin(qrc) => Ok(GetCurrentMtpResponse {
            mtp: qrc.get_current_mtp().await?,
        }),
        #[cfg(not(target_arch = "wasm32"))]
        MmCoinEnum::ZCoin(zcoin) => Ok(GetCurrentMtpResponse {
            mtp: zcoin.get_current_mtp().await?,
        }),
        MmCoinEnum::Bch(bch) => Ok(GetCurrentMtpResponse {
            mtp: bch.get_current_mtp().await?,
        }),
        _ => Err(MmError::new(GetCurrentMtpError::NotSupported("Internal".to_string()))),
    }
}
