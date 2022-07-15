use crate::{coin_conf,
            eth::{eth_coin_from_conf_and_request_v2, GasStationPricePolicy},
            lp_register_coin, CoinProtocol, MmCoinEnum, RegisterCoinParams};
use common::{HttpStatusCode, StatusCode};
use crypto::CryptoCtx;
use derive_more::Display;
use ethereum_types::Address;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmResult;
use mm2_number::BigDecimal;

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
#[allow(dead_code)]
pub enum EnableV2RpcError {
    InvalidPayload(String),
    CoinCouldNotInitialized(String),
    CouldNotFetchBalance(String),
    UnreachableNodes(String),
    AtLeastOneNodeRequired(String),
    InternalError(String),
}

impl HttpStatusCode for EnableV2RpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            EnableV2RpcError::InvalidPayload(_)
            | EnableV2RpcError::CoinCouldNotInitialized(_)
            | EnableV2RpcError::UnreachableNodes(_)
            | EnableV2RpcError::AtLeastOneNodeRequired(_) => StatusCode::BAD_REQUEST,
            EnableV2RpcError::CouldNotFetchBalance(_) => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct EnableV2RpcRequest {
    pub coin: String,
    pub nodes: Vec<EnableV2NodesRpc>,
    pub swap_contract_address: Address,
    pub fallback_swap_contract: Option<Address>,
    pub gas_station_url: Option<String>,
    pub gas_station_decimals: Option<u8>,
    #[serde(default)]
    pub gas_station_policy: GasStationPricePolicy,
    pub mm2: Option<u8>,
    pub tx_history: Option<bool>,
    pub required_confirmations: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EnableV2NodesRpc {
    pub url: String,
    pub gui_auth: bool,
}

#[derive(Serialize, Clone)]
pub struct EnableV2RpcResponse {
    pub result: String,
    pub address: String,
    pub balance: BigDecimal,
    pub unspendable_balance: BigDecimal,
    pub coin: String,
    pub required_confirmations: u64,
    pub requires_notarization: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mature_confirmations: Option<u32>,
}

pub async fn activate_eth_coin(ctx: &MmArc, req: EnableV2RpcRequest) -> MmResult<MmCoinEnum, EnableV2RpcError> {
    let secret = CryptoCtx::from_ctx(ctx)
        .map_err(|e| EnableV2RpcError::InternalError(e.to_string()))?
        .iguana_ctx()
        .secp256k1_privkey_bytes()
        .to_vec();

    let coins_en = coin_conf(ctx, &req.coin);

    let protocol: CoinProtocol = serde_json::from_value(coins_en["protocol"].clone())
        .map_err(|e| EnableV2RpcError::CoinCouldNotInitialized(e.to_string()))?;

    let coin: MmCoinEnum = eth_coin_from_conf_and_request_v2(ctx, &req.coin, &coins_en, req.clone(), &secret, protocol)
        .await?
        .into();

    let register_params = RegisterCoinParams {
        ticker: req.coin,
        tx_history: req.tx_history.unwrap_or(false),
    };

    lp_register_coin(ctx, coin.clone(), register_params)
        .await
        .map_err(|e| EnableV2RpcError::CoinCouldNotInitialized(e.to_string()))?;

    Ok(coin)
}
