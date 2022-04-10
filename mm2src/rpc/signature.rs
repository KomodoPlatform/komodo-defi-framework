use coins::lp_coinfind_or_err;
use common::mm_ctx::MmArc;
use common::mm_error::MmError;
use common::HttpStatusCode;
use derive_more::Display;
use http::StatusCode;
use serde_json::{self as json, Value as Json};

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
    #[display(fmt = "Wallet error: {}", _0)]
    WalletError(String),
}

#[derive(Serialize)]
pub struct SignatureResponse {
    signature: String,
}

#[derive(Serialize)]
pub struct VerificationResponse {
    is_valid: bool,
    address: String,
    pubkey: String,
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
            VerificationError::WalletError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub async fn sign_message(ctx: MmArc, req: Json) -> SignatureResult<SignatureResponse> {
    let coin_name: String = json::from_value(req["coin"].clone())
        .map_err(|_| SignatureError::InvalidRequest(String::from("No coin field")))?;
    let message: String = json::from_value(req["message"].clone())
        .map_err(|_| SignatureError::InvalidRequest(String::from("No message field")))?;
    let coin = lp_coinfind_or_err(&ctx, &coin_name)
        .await
        .map_err(|_| SignatureError::InvalidRequest(String::from("No such coin")))?;
    let signature = coin
        .sign_message(&message)
        .map_err(|e| SignatureError::WalletError(String::from(e)))?;
    Ok(SignatureResponse { signature })
}

pub async fn verify_message(ctx: MmArc, req: Json) -> VerificationResult<VerificationResponse> {
    let coin_name: String = json::from_value(req["coin"].clone())
        .map_err(|_| VerificationError::InvalidRequest(String::from("No coin field")))?;
    let message: String = json::from_value(req["message"].clone())
        .map_err(|_| VerificationError::InvalidRequest(String::from("No message field")))?;
    let signature: String = json::from_value(req["signature"].clone())
        .map_err(|_| VerificationError::InvalidRequest(String::from("No signature field")))?;
    let address: String = json::from_value(req["address"].clone())
        .map_err(|_| VerificationError::InvalidRequest(String::from("No address field")))?;

    let coin = lp_coinfind_or_err(&ctx, &coin_name)
        .await
        .map_err(|_| VerificationError::InvalidRequest(String::from("No such coin")))?;

    let validate_address_result = coin.validate_address(&address);
    if !validate_address_result.is_valid {
        return MmError::err(VerificationError::InvalidRequest(
            validate_address_result.reason.unwrap_or_else(|| "Unknown".to_string()),
        ));
    }

    let address = coin.my_address().map_err(|e| VerificationError::WalletError(e))?;
    let is_valid = coin
        .verify_message(&signature, &message, &address)
        .map_err(|e| VerificationError::InvalidRequest(e))?;
    let pubkey = coin
        .get_public_key()
        .map_err(|e| VerificationError::WalletError(e.to_string()))?;

    Ok(VerificationResponse {
        is_valid,
        address,
        pubkey,
    })
}
