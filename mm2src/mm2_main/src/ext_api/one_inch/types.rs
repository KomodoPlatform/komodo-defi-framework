use crate::ext_api::one_inch::errors::FromApiValueError;
use coins::eth::{u256_to_big_decimal, wei_to_gwei_decimal};
use common::true_f;
use ethereum_types::{Address, U256};
use mm2_err_handle::prelude::*;
use mm2_number::{BigDecimal, MmNumber};
use rpc::v1::types::Bytes as BytesJson;
use serde::{Deserialize, Serialize};
use trading_api::one_inch_api::{self,
                                types::{ProtocolInfo, TokenInfo}};

#[derive(Clone, Debug, Deserialize)]
pub struct AggregationContractRequest {}

#[derive(Clone, Debug, Deserialize)]
pub struct ClassicSwapQuoteRequest {
    pub base: String,
    pub rel: String,
    pub amount: MmNumber,
    // Optional fields
    pub fee: Option<f32>,
    pub protocols: Option<String>,
    pub gas_price: Option<String>,
    pub complexity_level: Option<u32>,
    pub parts: Option<u32>,
    pub main_route_parts: Option<u32>,
    pub gas_limit: Option<u128>,
    #[serde(default = "true_f")]
    pub include_tokens_info: bool,
    #[serde(default = "true_f")]
    pub include_protocols: bool,
    #[serde(default = "true_f")]
    pub include_gas: bool,
    pub connector_tokens: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ClassicSwapCreateRequest {
    pub base: String,
    pub rel: String,
    pub amount: MmNumber,
    pub slippage: f32,
    // Optional fields
    pub fee: Option<f32>,
    pub protocols: Option<String>,
    pub gas_price: Option<String>,
    pub complexity_level: Option<u32>,
    pub parts: Option<u32>,
    pub main_route_parts: Option<u32>,
    pub gas_limit: Option<u128>,
    #[serde(default = "true_f")]
    pub include_tokens_info: bool,
    #[serde(default = "true_f")]
    pub include_protocols: bool,
    #[serde(default = "true_f")]
    pub include_gas: bool,
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
    pub dst_amount: MmNumber,
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
            dst_amount: MmNumber::from(u256_to_big_decimal(U256::from_dec_str(&data.dst_amount)?, decimals)?),
            src_token: data.src_token,
            dst_token: data.dst_token,
            protocols: data.protocols,
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
        tx_fields: one_inch_api::types::TxFields,
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
