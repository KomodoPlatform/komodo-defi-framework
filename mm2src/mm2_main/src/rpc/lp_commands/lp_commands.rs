use crate::mm2::{lp_network::subscribe_to_topic, lp_swap::tx_helper_topic};
use coins::rpc_command::activate_eth_coin::{activate_eth_coin, EnableV2RpcError, EnableV2RpcRequest,
                                            EnableV2RpcResponse};
use common::{Future01CompatExt, HttpStatusCode};
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

pub async fn activate_eth_coin_wrapper(
    ctx: MmArc,
    req: EnableV2RpcRequest,
) -> MmResult<EnableV2RpcResponse, EnableV2RpcError> {
    let coin = activate_eth_coin(&ctx, req).await?;

    let balance = coin
        .my_balance()
        .compat()
        .await
        .map_err(|e| EnableV2RpcError::CouldNotFetchBalance(e.to_string()))?;

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
