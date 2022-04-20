use common::{mm_ctx::MmArc, mm_error::MmError, HttpStatusCode};
use derive_more::Display;
use http::StatusCode;
use serde_json::Value as Json;

#[derive(Serialize)]
pub struct GetPublicKeyHashResponse {
    public_key_hash: String,
}
#[derive(Serialize, Display, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetPublicKeyHashError {
    Internal(String),
}

pub type GetPublicKeyHashRpcResult<T> = Result<T, MmError<GetPublicKeyHashError>>;

impl HttpStatusCode for GetPublicKeyHashError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetPublicKeyHashError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub async fn get_public_key_hash(ctx: MmArc, _req: Json) -> GetPublicKeyHashRpcResult<GetPublicKeyHashResponse> {
    let public_key_hash = match ctx.secp256k1_key_pair.as_option() {
        Some(key_pair) => key_pair.public().address_hash().to_string(),
        None => {
            return MmError::err(GetPublicKeyHashError::Internal(
                "A valid public key is needed to get hashed version. Please provide one".to_string(),
            ))
        },
    };
    Ok(GetPublicKeyHashResponse { public_key_hash })
}
