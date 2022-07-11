use coins::lp_coininit;
use common::{mm_number::BigDecimal, Future01CompatExt, HttpStatusCode};
use crypto::{CryptoCtx, CryptoInitError};
use derive_more::Display;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use rpc::v1::types::H160 as H160Json;
use serde_json::Value as Json;

use crate::mm2::{lp_network::subscribe_to_topic, lp_swap::tx_helper_topic};

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
#[allow(dead_code)]
pub enum EnableV2RpcError {
    InvalidPayload(String),
    CoinCouldNotInitialized(String),
    InternalError(String),
}

impl HttpStatusCode for EnableV2RpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            EnableV2RpcError::InvalidPayload(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Serialize, Clone)]
pub struct EnableV2RpcResponse {
    result: String,
    address: String,
    balance: BigDecimal,
    unspendable_balance: BigDecimal,
    coin: String,
    required_confirmations: u64,
    requires_notarization: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    mature_confirmations: Option<u32>,
}

pub async fn enable_v2(ctx: MmArc, req: Json) -> MmResult<EnableV2RpcResponse, EnableV2RpcError> {
    let mut req = req;
    req["_v"] = json!(2_u64);
    drop_mutability!(req);

    let ticker = req["coin"]
        .as_str()
        .ok_or_else(|| EnableV2RpcError::InvalidPayload(String::from("No 'coin' field")))?
        .to_owned();

    let coin = lp_coininit(&ctx, &ticker, &req)
        .await
        .map_err(EnableV2RpcError::CoinCouldNotInitialized)?;

    let balance = coin
        .my_balance()
        .compat()
        .await
        .map_err(|e| EnableV2RpcError::CoinCouldNotInitialized(e.to_string()))?;

    if coin.is_utxo_in_native_mode() {
        subscribe_to_topic(&ctx, tx_helper_topic(coin.ticker()));
    }

    Ok(EnableV2RpcResponse {
        result: String::from("success"),
        address: coin.my_address().map_err(EnableV2RpcError::InternalError)?,
        balance: balance.spendable,
        unspendable_balance: balance.unspendable,
        coin: coin.ticker().to_string(),
        required_confirmations: coin.required_confirmations(),
        requires_notarization: coin.requires_notarization(),
        mature_confirmations: coin.mature_confirmations(),
    })
}
