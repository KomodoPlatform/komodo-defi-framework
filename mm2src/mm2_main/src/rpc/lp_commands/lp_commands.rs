use common::HttpStatusCode;
use crypto::{CryptoCtx, CryptoInitError};
use derive_more::Display;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use rpc::v1::types::H160 as H160Json;
use serde_json::Value as Json;

pub type GetPublicKeyRpcResult<T> = Result<T, MmError<GetPublicKeyError>>;

#[derive(Serialize, Display, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetPublicKeyError {
    Internal(String),
}

impl From<CryptoInitError> for GetPublicKeyError {
    fn from(_: CryptoInitError) -> Self { GetPublicKeyError::Internal("public_key not available".to_string()) }
}

#[derive(Serialize)]
pub struct GetPublicKeyResponse {
    public_key: String,
}

impl HttpStatusCode for GetPublicKeyError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetPublicKeyError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub async fn get_public_key(ctx: MmArc, _req: Json) -> GetPublicKeyRpcResult<GetPublicKeyResponse> {
    let public_key = CryptoCtx::from_ctx(&ctx)?.secp256k1_pubkey().to_string();
    Ok(GetPublicKeyResponse { public_key })
}

#[derive(Serialize)]
pub struct GetPublicKeyHashResponse {
    public_key_hash: H160Json,
}

pub async fn get_public_key_hash(ctx: MmArc, _req: Json) -> GetPublicKeyRpcResult<GetPublicKeyHashResponse> {
    let public_key_hash = ctx.rmd160().to_owned().into();
    Ok(GetPublicKeyHashResponse { public_key_hash })
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum EnableV2RpcError {
    CoinIsNotSupported(String),
    InternalError(String),
}

impl HttpStatusCode for EnableV2RpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            EnableV2RpcError::CoinIsNotSupported(_) => StatusCode::NOT_FOUND,
            EnableV2RpcError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Serialize)]
pub struct EnableV2RpcResponse {
    pub test: bool
    // result: &'a str,
    // address: String,
    // balance: BigDecimal,
    // unspendable_balance: BigDecimal,
    // coin: &'a str,
    // required_confirmations: u64,
    // requires_notarization: bool,
    // #[serde(skip_serializing_if = "Option::is_none")]
    // mature_confirmations: Option<u32>,
}

/// v2 of `fn enable(ctx: MmArc, req: Json)`.
pub async fn enable_v2(ctx: MmArc, req: Json) -> MmResult<EnableV2RpcResponse, EnableV2RpcError> {
    println!("request {:?}", req);
    let ticker = req["coin"].as_str().ok_or("No 'coin' field").map_err(|e| EnableV2RpcError::CoinIsNotSupported(e.to_string()))?.to_owned();
    println!("ticker {}", ticker);

    Ok(EnableV2RpcResponse { test: true })
}
