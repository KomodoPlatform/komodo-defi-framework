use common::mm_ctx::MmArc;
use common::mm_error::MmError;
use common::HttpStatusCode;
use crypto::{CryptoCtx, CryptoInitError};
use derive_more::Display;
use http::StatusCode;
use serde_json::Value as Json;

// Start get_public_key rpc implementation
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
// End get_public_key rpc implementation


// Start get_public_key_hash rpc implementation
#[derive(Serialize)]
pub struct GetPublicKeyHashResponse {
    public_key_hash: String,
}
pub async fn get_public_key_hash(ctx: MmArc, _req: Json) -> GetPublicKeyRpcResult<GetPublicKeyHashResponse> {
    let public_key_hash = ctx.rmd160().to_string();
    Ok(GetPublicKeyHashResponse { public_key_hash })
}
// end get_public_key_hash rpc implementation
