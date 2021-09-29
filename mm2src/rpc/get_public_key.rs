use common::mm_ctx::MmArc;
use common::mm_error::MmError;
use common::HttpStatusCode;
use derive_more::Display;
use http::StatusCode;
use serde_json::Value as Json;

#[derive(Serialize, Display, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetPublicKeyError {
    Internal(String),
}

#[derive(Serialize)]
pub struct GetPublicKeyResponse {
    public_key: String,
}

pub type GetPublicKeyRpcResult<T> = Result<T, MmError<GetPublicKeyError>>;

impl HttpStatusCode for GetPublicKeyError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetPublicKeyError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub async fn get_public_key(ctx: MmArc, _req: Json) -> GetPublicKeyRpcResult<GetPublicKeyResponse> {
    let public_key = match ctx.secp256k1_key_pair.as_option() {
        None => return MmError::err(GetPublicKeyError::Internal("public_key not available".to_string())),
        Some(keypair) => keypair.public().to_string(),
    };
    Ok(GetPublicKeyResponse { public_key })
}
