use crate::ext_api::one_inch::errors::FromApiValueError;
use coins::eth::{u256_to_big_decimal, wei_to_gwei_decimal};
use ethereum_types::{Address, U256};
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use rpc::v1::types::Bytes as BytesJson;
use serde::{Deserialize, Serialize};
use trading_api::one_inch_api;

#[derive(Clone, Debug, Deserialize)]
pub struct AggregationContractRequest {}

#[derive(Clone, Debug, Deserialize)]
pub struct ClassicSwapQuoteRequest {
    pub base: String,
    pub rel: String,
    pub amount: BigDecimal,

    // Optional fields
    pub fee: Option<f32>,
    pub protocols: Option<String>,
    pub gas_price: Option<String>,
    pub complexity_level: Option<u32>,
    pub parts: Option<u32>,
    pub main_route_parts: Option<u32>,
    pub gas_limit: Option<u128>,

    pub include_tokens_info: Option<bool>,
    pub include_protocols: Option<bool>,
    pub include_gas: Option<bool>,
    pub connector_tokens: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ClassicSwapCreateRequest {
    pub base: String,
    pub rel: String,
    pub amount: BigDecimal,
    pub slippage: f32,

    // Optional fields
    pub fee: Option<f32>,
    pub protocols: Option<String>,
    pub gas_price: Option<String>,
    pub complexity_level: Option<u32>,
    pub parts: Option<u32>,
    pub main_route_parts: Option<u32>,
    pub gas_limit: Option<u128>,
    pub include_tokens_info: Option<bool>,
    pub include_protocols: Option<bool>,
    pub include_gas: Option<bool>,
    pub connector_tokens: Option<String>,
    pub permit: Option<String>,
    pub receiver: Option<String>,
    pub referrer: Option<String>,

    /// Disable gas estimation
    pub disable_estimate: Option<bool>,
    /// Allow the swap to be partially filled
    pub allow_partial_fill: Option<bool>,
}

#[derive(Serialize, Debug)]
pub struct ClassicSwapResponse {
    pub dst_amount: BigDecimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src_token: Option<TokenInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst_token: Option<TokenInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocols: Option<Vec<Vec<Vec<ProtocolInfo>>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx: Option<TxFields>,
    pub gas: Option<u128>,
}

impl ClassicSwapResponse {
    pub(crate) fn from_api_value(
        data: one_inch_api::types::ClassicSwapData,
        decimals: u8,
    ) -> MmResult<Self, FromApiValueError> {
        Ok(Self {
            dst_amount: u256_to_big_decimal(U256::from_dec_str(&data.dst_amount)?, decimals)?,
            src_token: TokenInfo::from_api_value(data.src_token),
            dst_token: TokenInfo::from_api_value(data.dst_token),
            protocols: ProtocolInfo::from_api_value(data.protocols),
            tx: data.tx.map(|tx| TxFields::from_api_value(tx, decimals)).transpose()?,
            gas: data.gas,
        })
    }
}

#[derive(Serialize, Debug)]
pub struct TxFields {
    pub from: Address,
    pub to: Address,
    pub data: BytesJson,
    pub value: BigDecimal,
    /// Estimated gas price in gwei
    pub gas_price: BigDecimal,
    pub gas: u128, // TODO: in eth EthTxFeeDetails rpc we use u64. Better have identical u128 everywhere
}

impl TxFields {
    pub(crate) fn from_api_value(
        tx_fields: trading_api::one_inch_api::types::TxFields,
        decimals: u8,
    ) -> MmResult<Self, FromApiValueError> {
        Ok(Self {
            from: tx_fields.from,
            to: tx_fields.to,
            data: BytesJson::from(hex::decode(str_strip_0x!(tx_fields.data.as_str()))?),
            value: u256_to_big_decimal(U256::from_dec_str(&tx_fields.value)?, decimals)?,
            gas_price: wei_to_gwei_decimal(U256::from_dec_str(&tx_fields.gas_price)?)?,
            gas: tx_fields.gas,
        })
    }
}

#[derive(Serialize, Debug)]
pub struct TokenInfo {
    pub address: Address,
    pub symbol: String,
    pub name: String,
    pub decimals: u32,
    pub eip2612: bool,
    pub is_fot: bool,
    pub logo_uri: String,
    pub tags: Vec<String>,
}

impl TokenInfo {
    pub(crate) fn from_api_value(opt_info: Option<one_inch_api::types::TokenInfo>) -> Option<Self> {
        opt_info.map(|info| Self {
            address: info.address,
            symbol: info.symbol,
            name: info.name,
            decimals: info.decimals,
            eip2612: info.eip2612,
            is_fot: info.is_fot,
            logo_uri: info.logo_uri,
            tags: info.tags,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct ProtocolInfo {
    pub name: String,
    pub part: f64,
    pub from_token_address: Address,
    pub to_token_address: Address,
}

impl ProtocolInfo {
    pub(crate) fn from_api_value(
        opt_info: Option<Vec<Vec<Vec<one_inch_api::types::ProtocolInfo>>>>,
    ) -> Option<Vec<Vec<Vec<Self>>>> {
        opt_info.map(|v0| {
            v0.into_iter()
                .map(|v1| {
                    v1.into_iter()
                        .map(|v2| {
                            v2.into_iter()
                                .map(|info| Self {
                                    name: info.name,
                                    part: info.part,
                                    from_token_address: info.from_token_address,
                                    to_token_address: info.to_token_address,
                                })
                                .collect::<Vec<_>>()
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
    }
}
