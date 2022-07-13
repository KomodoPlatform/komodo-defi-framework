use crate::{eth::GasStationPricePolicy, lp_coininit, MmCoinEnum};
use common::{HttpStatusCode, StatusCode};
use derive_more::Display;
use ethereum_types::Address;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmResult;
use mm2_number::BigDecimal;
use serde_json::Value as Json;

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
#[allow(dead_code)]
pub enum EnableV2RpcError {
    InvalidPayload(String),
    CoinCouldNotInitialized(String),
    CouldNotFetchBalance(String),
    InternalError(String),
}

impl HttpStatusCode for EnableV2RpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            EnableV2RpcError::InvalidPayload(_) => StatusCode::BAD_REQUEST,
            EnableV2RpcError::CoinCouldNotInitialized(_) => StatusCode::BAD_REQUEST,
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

impl EnableV2RpcRequest {
    #[inline(always)]
    pub fn from_json_payload(payload: Json) -> Self {
        serde_json::from_value(payload)
            .map_err(|e| EnableV2RpcError::InvalidPayload(e.to_string()))
            .unwrap()
    }
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

pub async fn enable_v2(ctx: &MmArc, req: Json) -> MmResult<MmCoinEnum, EnableV2RpcError> {
    let mut req = req;
    req["_v"] = json!(2_u64);
    drop_mutability!(req);

    let ticker = req["coin"]
        .as_str()
        .ok_or_else(|| EnableV2RpcError::InvalidPayload(String::from("No 'coin' field")))?
        .to_owned();

    Ok(lp_coininit(ctx, &ticker, &req)
        .await
        .map_err(EnableV2RpcError::CoinCouldNotInitialized)?)
}
