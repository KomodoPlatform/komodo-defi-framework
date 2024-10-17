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

/// Request to get quote for 1inch classic swap.
/// See 1inch docs for more details: https://portal.1inch.dev/documentation/apis/swap/classic-swap/Parameter%20Descriptions/quote_params
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassicSwapQuoteRequest {
    /// Base coin ticker
    pub base: String,
    /// Rel coin ticker
    pub rel: String,
    /// Swap amount in coins (with fraction)
    pub amount: MmNumber,
    /// Partner fee, percentage of src token amount will be sent to referrer address, min: 0; max: 3. 
    /// Should be the same for quote and swap rpc. Default is 0
    pub fee: Option<f32>,
    /// Specify liquidity sources 
    /// e.g.: &protocols=WETH,CURVE,BALANCER,...,ZRX
    /// (by default - all used)
    pub protocols: Option<String>,
    /// Network price per gas, in Gwei for this rpc. 
    /// 1inch takes in account gas expenses to determine exchange route. Should be the same for a quote and swap.
    /// If not set the 'fast' network gas price will be used
    pub gas_price: Option<String>,
    /// Maximum number of token-connectors to be used in a transaction, min: 0; max: 3; default: 2
    pub complexity_level: Option<u32>,
    /// Limit maximum number of parts each main route parts can be split into.
    /// Should be the same for a quote and swap. Default: 20; max: 100
    pub parts: Option<u32>,
    /// Limit maximum number of main route parts. Should be the same for a quote and swap. Default: 20; max: 50;
    pub main_route_parts: Option<u32>,
    /// Maximum amount of gas for a swap. 
    /// Should be the same for a quote and swap. Default: 11500000; max: 11500000
    pub gas_limit: Option<u128>,
    /// Return fromToken and toToken info in response (default is true)
    #[serde(default = "true_f")]
    pub include_tokens_info: bool,
    /// Return used swap protocols in response (default is true)
    #[serde(default = "true_f")]
    pub include_protocols: bool,
    /// Include estimated gas in return value (default is true)
    #[serde(default = "true_f")]
    pub include_gas: bool,
    /// Token-connectors can be specified via this parameter. If not set, default token-connectors will be used
    pub connector_tokens: Option<String>,
}

/// Request to create transaction for 1inch classic swap.
/// See 1inch docs for more details: https://portal.1inch.dev/documentation/apis/swap/classic-swap/Parameter%20Descriptions/swap_params
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassicSwapCreateRequest {
    /// Base coin ticker
    pub base: String,
    /// Rel coin ticker
    pub rel: String,
    /// Swap amount in coins (with fraction)
    pub amount: MmNumber,
    /// Allowed slippage, min: 0; max: 50
    pub slippage: f32,
    /// Partner fee, percentage of src token amount will be sent to referrer address, min: 0; max: 3. 
    /// Should be the same for quote and swap rpc. Default is 0
    pub fee: Option<f32>,
    /// Specify liquidity sources 
    /// e.g.: &protocols=WETH,CURVE,BALANCER,...,ZRX
    /// (by default - all used)
    pub protocols: Option<String>,
    /// Network price per gas, in Gwei for this rpc. 
    /// 1inch takes in account gas expenses to determine exchange route. Should be the same for a quote and swap.
    /// If not set the 'fast' network gas price will be used
    pub gas_price: Option<String>,
    /// Maximum number of token-connectors to be used in a transaction, min: 0; max: 3; default: 2
    pub complexity_level: Option<u32>,
    /// Limit maximum number of parts each main route parts can be split into.
    /// Should be the same for a quote and swap. Default: 20; max: 100
    pub parts: Option<u32>,
    /// Limit maximum number of main route parts. Should be the same for a quote and swap. Default: 20; max: 50;
    pub main_route_parts: Option<u32>,
    /// Maximum amount of gas for a swap. 
    /// Should be the same for a quote and swap. Default: 11500000; max: 11500000
    pub gas_limit: Option<u128>,
    /// Return fromToken and toToken info in response (default is true)
    #[serde(default = "true_f")]
    pub include_tokens_info: bool,
    /// Return used swap protocols in response (default is true)
    #[serde(default = "true_f")]
    pub include_protocols: bool,
    /// Include estimated gas in response (default is true)
    #[serde(default = "true_f")]
    pub include_gas: bool,
    /// Token-connectors can be specified via this parameter. If not set, default token-connectors will be used
    pub connector_tokens: Option<String>,
    /// Excluded supported liquidity sources. Should be the same for a quote and swap, max: 5
    pub excluded_protocols: Option<String>,
    /// Used according https://eips.ethereum.org/EIPS/eip-2612
    pub permit: Option<String>,
    /// Exclude the Unoswap method
    pub compatibility: Option<bool>,
    /// This address will receive funds after the swap. By default same address as 'my address'
    pub receiver: Option<String>,
    /// Address to receive partner fee
    pub referrer: Option<String>,
    /// if true, disable most of the checks, default: false
    pub disable_estimate: Option<bool>,
    /// if true, the algorithm can cancel part of the route, if the rate has become less attractive. 
    /// Unswapped tokens will return to 'my address'. Default: true
    pub allow_partial_fill: Option<bool>,
    /// Enable this flag in case you did an approval to permit2 smart contract (default false)
    pub use_permit2: Option<bool>,
}

/// Response for both classic swap quote or create swap calls
#[derive(Serialize, Debug)]
pub struct ClassicSwapResponse {
    /// Destination token amount, in coins (with fraction)
    pub dst_amount: MmNumber,
    /// Source (base) token info
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src_token: Option<TokenInfo>,
    /// Destination (rel) token info
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst_token: Option<TokenInfo>,
    /// Used liquidity sources
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocols: Option<Vec<Vec<Vec<ProtocolInfo>>>>,
    /// Swap tx fields (returned only for create swap rpc)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx: Option<TxFields>,
    /// Estimated (returned only for quote rpc)
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
