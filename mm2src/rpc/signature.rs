use coins::lp_coinfind_or_err;
use common::mm_ctx::MmArc;
use common::mm_error::MmError;
use common::HttpStatusCode;
use derive_more::Display;
use http::StatusCode;

#[derive(Serialize, Display, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum SignatureError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Key error: {}", _0)]
    WalletError(String),
}

#[derive(Serialize, Display, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum VerificationError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
}

#[derive(Serialize)]
pub struct SignatureResponse {
    signature: String,
}

#[derive(Serialize, Deserialize)]
pub struct SignatureRequest {
    coin: String,
    message: String
}

#[derive(Serialize, Deserialize)]
pub struct VerificationRequest {
    coin: String,
    message: String,
    signature: String,
    address: String
}

#[derive(Serialize)]
pub struct VerificationResponse {
    is_valid: bool,
}

pub type SignatureResult<T> = Result<T, MmError<SignatureError>>;

pub type VerificationResult<T> = Result<T, MmError<VerificationError>>;

impl HttpStatusCode for SignatureError {
    fn status_code(&self) -> StatusCode {
        match self {
            SignatureError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            SignatureError::WalletError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl HttpStatusCode for VerificationError {
    fn status_code(&self) -> StatusCode {
        match self {
            VerificationError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
        }
    }
}

pub async fn sign_message(ctx: MmArc, req: SignatureRequest) -> SignatureResult<SignatureResponse> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin)
        .await
        .map_err(|_| SignatureError::InvalidRequest(String::from("No such coin")))?;
    let signature = coin.sign_message(&req.message).map_err(SignatureError::WalletError)?;
    Ok(SignatureResponse { signature })
}

pub async fn verify_message(ctx: MmArc, req: VerificationRequest) -> VerificationResult<VerificationResponse> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin)
        .await
        .map_err(|_| VerificationError::InvalidRequest(String::from("No such coin")))?;

    let validate_address_result = coin.validate_address(&req.address);
    if !validate_address_result.is_valid {
        return MmError::err(VerificationError::InvalidRequest(
            validate_address_result.reason.unwrap_or_else(|| "Unknown".to_string()),
        ));
    }

    let is_valid = coin
        .verify_message(&req.signature, &req.message, &req.address)
        .map_err(VerificationError::InvalidRequest)?;

    Ok(VerificationResponse { is_valid })
}
