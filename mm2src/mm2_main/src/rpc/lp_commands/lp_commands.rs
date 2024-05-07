use common::HttpStatusCode;
use crypto::{CryptoCtx, CryptoCtxError, HwConnectionStatus, HwPubkey};
use derive_more::Display;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_rpc::data::version2::{GetPublicKeyHashResponse, GetPublicKeyResponse};
use rpc::v1::types::H160 as H160Json;
use serde_json::Value as Json;

pub type GetPublicKeyRpcResult<T> = Result<T, MmError<GetPublicKeyError>>;
pub type GetSharedDbIdResult<T> = Result<T, MmError<GetSharedDbIdError>>;
pub type GetSharedDbIdError = GetPublicKeyError;

#[derive(Serialize, Display, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetPublicKeyError {
    Internal(String),
}

impl From<CryptoCtxError> for GetPublicKeyError {
    fn from(_: CryptoCtxError) -> Self { GetPublicKeyError::Internal("public_key not available".to_string()) }
}

impl HttpStatusCode for GetPublicKeyError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetPublicKeyError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub async fn get_public_key(ctx: MmArc, _req: Json) -> GetPublicKeyRpcResult<GetPublicKeyResponse> {
    let public_key = CryptoCtx::from_ctx(&ctx)?.mm2_internal_pubkey().to_string();
    Ok(GetPublicKeyResponse { public_key })
}

pub async fn get_public_key_hash(ctx: MmArc, _req: Json) -> GetPublicKeyRpcResult<GetPublicKeyHashResponse> {
    let public_key_hash = ctx.rmd160().to_owned().into();
    Ok(GetPublicKeyHashResponse { public_key_hash })
}

#[derive(Serialize)]
pub struct GetSharedDbIdResponse {
    shared_db_id: H160Json,
}

pub async fn get_shared_db_id(ctx: MmArc, _req: Json) -> GetSharedDbIdResult<GetSharedDbIdResponse> {
    let shared_db_id = ctx.shared_db_id().to_owned().into();
    Ok(GetSharedDbIdResponse { shared_db_id })
}

#[derive(Serialize, Display, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum TrezorConnectionError {
    #[display(fmt = "Trezor hasn't been initialized yet")]
    TrezorNotInitialized,
    #[display(fmt = "Found unexpected device. Please re-initialize Hardware wallet")]
    FoundUnexpectedDevice,
    Internal(String),
}

impl From<CryptoCtxError> for TrezorConnectionError {
    fn from(e: CryptoCtxError) -> Self { TrezorConnectionError::Internal(format!("'CryptoCtx' is not available: {e}")) }
}

impl HttpStatusCode for TrezorConnectionError {
    fn status_code(&self) -> StatusCode {
        match self {
            TrezorConnectionError::TrezorNotInitialized => StatusCode::BAD_REQUEST,
            TrezorConnectionError::FoundUnexpectedDevice | TrezorConnectionError::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
        }
    }
}

#[derive(Deserialize)]
pub struct TrezorConnectionStatusReq {
    /// Can be used to make sure that the Trezor device is expected.
    device_pubkey: Option<HwPubkey>,
}

#[derive(Serialize)]
pub struct TrezorConnectionStatusRes {
    status: HwConnectionStatus,
}

pub async fn trezor_connection_status(
    ctx: MmArc,
    req: TrezorConnectionStatusReq,
) -> MmResult<TrezorConnectionStatusRes, TrezorConnectionError> {
    let crypto_ctx = CryptoCtx::from_ctx(&ctx)?;
    let hw_ctx = crypto_ctx
        .hw_ctx()
        .or_mm_err(|| TrezorConnectionError::TrezorNotInitialized)?;

    if let Some(expected) = req.device_pubkey {
        if hw_ctx.hw_pubkey() != expected {
            return MmError::err(TrezorConnectionError::FoundUnexpectedDevice);
        }
    }

    Ok(TrezorConnectionStatusRes {
        status: hw_ctx.trezor_connection_status().await,
    })
}
