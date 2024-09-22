#![allow(clippy::result_large_err)]

use super::client::QueryParams;
use super::errors::ApiClientError;
use common::{def_with_opt_param, push_if_some};
use ethereum_types::Address;
use mm2_err_handle::mm_error::MmResult;
use serde::{Deserialize, Serialize};

const ONE_INCH_MAX_SLIPPAGE: f32 = 50.0;
const ONE_INCH_MAX_FEE_SHARE: f32 = 3.0;
const ONE_INCH_MAX_GAS: u128 = 11500000;
const ONE_INCH_MAX_PARTS: u32 = 100;
const ONE_INCH_MAX_MAIN_ROUTE_PARTS: u32 = 50;
const ONE_INCH_MAX_COMPLEXITY_LEVEL: u32 = 3;

/// API params builder for swap quote
#[derive(Default)]
pub struct ClassicSwapQuoteParams {
    /// Source token address
    src: String,
    /// Destination token address
    dst: String,
    amount: String,
    // Optional fields
    fee: Option<f32>,
    protocols: Option<String>,
    gas_price: Option<String>,
    complexity_level: Option<u32>,
    parts: Option<u32>,
    main_route_parts: Option<u32>,
    gas_limit: Option<u128>,
    include_tokens_info: Option<bool>,
    include_protocols: Option<bool>,
    include_gas: Option<bool>,
    connector_tokens: Option<String>,
}

impl ClassicSwapQuoteParams {
    pub fn new(src: String, dst: String, amount: String) -> Self {
        Self {
            src,
            dst,
            amount,
            ..Default::default()
        }
    }

    def_with_opt_param!(fee, f32);
    def_with_opt_param!(protocols, String);
    def_with_opt_param!(gas_price, String);
    def_with_opt_param!(complexity_level, u32);
    def_with_opt_param!(parts, u32);
    def_with_opt_param!(main_route_parts, u32);
    def_with_opt_param!(gas_limit, u128);
    def_with_opt_param!(include_tokens_info, bool);
    def_with_opt_param!(include_protocols, bool);
    def_with_opt_param!(include_gas, bool);
    def_with_opt_param!(connector_tokens, String);

    pub fn build_query_params(&self) -> MmResult<QueryParams<'static>, ApiClientError> {
        self.validate_params()?;

        let mut params = vec![
            ("src", self.src.clone()),
            ("dst", self.dst.clone()),
            ("amount", self.amount.clone()),
        ];

        push_if_some!(params, "fee", self.fee);
        push_if_some!(params, "protocols", &self.protocols);
        push_if_some!(params, "gasPrice", &self.gas_price);
        push_if_some!(params, "complexityLevel", self.complexity_level);
        push_if_some!(params, "parts", self.parts);
        push_if_some!(params, "mainRouteParts", self.main_route_parts);
        push_if_some!(params, "gasLimit", self.gas_limit);
        push_if_some!(params, "includeTokensInfo", self.include_tokens_info);
        push_if_some!(params, "includeProtocols", self.include_protocols);
        push_if_some!(params, "includeGas", self.include_gas);
        push_if_some!(params, "connectorTokens", &self.connector_tokens);
        Ok(params)
    }

    /// Validate params by 1inch rules (to avoid extra requests)
    fn validate_params(&self) -> MmResult<(), ApiClientError> {
        validate_fee(&self.fee)?;
        validate_complexity_level(&self.complexity_level)?;
        validate_gas_limit(&self.gas_limit)?;
        validate_parts(&self.parts)?;
        validate_main_route_parts(&self.main_route_parts)?;
        Ok(())
    }
}

/// API params builder to create a tx for swap
#[derive(Default)]
pub struct ClassicSwapCreateParams {
    src: String,
    dst: String,
    amount: String,
    from: String,
    slippage: f32,
    // Optional fields
    fee: Option<f32>,
    protocols: Option<String>,
    gas_price: Option<String>,
    complexity_level: Option<u32>,
    parts: Option<u32>,
    main_route_parts: Option<u32>,
    gas_limit: Option<u128>,
    include_tokens_info: Option<bool>,
    include_protocols: Option<bool>,
    include_gas: Option<bool>,
    connector_tokens: Option<String>,
    permit: Option<String>,
    /// Funds receiver
    receiver: Option<String>,
    referrer: Option<String>,
    /// Disable gas estimation
    disable_estimate: Option<bool>,
    /// Allow the swap to be partially filled
    allow_partial_fill: Option<bool>,
}

impl ClassicSwapCreateParams {
    pub fn new(src: String, dst: String, amount: String, from: String, slippage: f32) -> Self {
        Self {
            src,
            dst,
            amount,
            from,
            slippage,
            ..Default::default()
        }
    }

    def_with_opt_param!(fee, f32);
    def_with_opt_param!(protocols, String);
    def_with_opt_param!(gas_price, String);
    def_with_opt_param!(complexity_level, u32);
    def_with_opt_param!(parts, u32);
    def_with_opt_param!(main_route_parts, u32);
    def_with_opt_param!(gas_limit, u128);
    def_with_opt_param!(include_tokens_info, bool);
    def_with_opt_param!(include_protocols, bool);
    def_with_opt_param!(include_gas, bool);
    def_with_opt_param!(connector_tokens, String);
    def_with_opt_param!(permit, String);
    def_with_opt_param!(receiver, String);
    def_with_opt_param!(referrer, String);
    def_with_opt_param!(disable_estimate, bool);
    def_with_opt_param!(allow_partial_fill, bool);

    pub fn build_query_params(&self) -> MmResult<QueryParams<'static>, ApiClientError> {
        self.validate_params()?;

        let mut params = vec![
            ("src", self.src.clone()),
            ("dst", self.dst.clone()),
            ("amount", self.amount.clone()),
            ("from", self.from.clone()),
            ("slippage", self.slippage.to_string()),
        ];

        push_if_some!(params, "fee", self.fee);
        push_if_some!(params, "protocols", &self.protocols);
        push_if_some!(params, "gasPrice", &self.gas_price);
        push_if_some!(params, "complexityLevel", self.complexity_level);
        push_if_some!(params, "parts", self.parts);
        push_if_some!(params, "mainRouteParts", self.main_route_parts);
        push_if_some!(params, "gasLimit", self.gas_limit);
        push_if_some!(params, "includeTokensInfo", self.include_tokens_info);
        push_if_some!(params, "includeProtocols", self.include_protocols);
        push_if_some!(params, "includeGas", self.include_gas);
        push_if_some!(params, "connectorTokens", &self.connector_tokens);
        push_if_some!(params, "permit", &self.permit);
        push_if_some!(params, "receiver", &self.receiver);
        push_if_some!(params, "referrer", &self.referrer);
        push_if_some!(params, "disableEstimate", self.disable_estimate);
        push_if_some!(params, "allowPartialFill", self.allow_partial_fill);

        Ok(params)
    }

    /// Validate params by 1inch rules (to avoid extra requests)
    fn validate_params(&self) -> MmResult<(), ApiClientError> {
        validate_slippage(self.slippage)?;
        validate_fee(&self.fee)?;
        validate_complexity_level(&self.complexity_level)?;
        validate_gas_limit(&self.gas_limit)?;
        validate_parts(&self.parts)?;
        validate_main_route_parts(&self.main_route_parts)?;
        Ok(())
    }
}

#[derive(Deserialize, Debug, Serialize)]
pub struct TokenInfo {
    pub address: Address,
    pub symbol: String,
    pub name: String,
    pub decimals: u32,
    pub eip2612: bool,
    #[serde(rename = "isFoT")]
    pub is_fot: bool,
    #[serde(rename = "logoURI")]
    pub logo_uri: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ProtocolInfo {
    pub name: String,
    pub part: f64,
    #[serde(rename = "fromTokenAddress")]
    pub from_token_address: Address,
    #[serde(rename = "toTokenAddress")]
    pub to_token_address: Address,
}

#[derive(Deserialize, Debug)]
pub struct ClassicSwapData {
    /// dst token amount to receive, in api is a decimal number as string
    #[serde(rename = "dstAmount")]
    pub dst_amount: String,
    #[serde(rename = "srcToken")]
    pub src_token: Option<TokenInfo>,
    #[serde(rename = "dstToken")]
    pub dst_token: Option<TokenInfo>,
    pub protocols: Option<Vec<Vec<Vec<ProtocolInfo>>>>,
    pub tx: Option<TxFields>,
    pub gas: Option<u128>,
}

#[derive(Deserialize, Debug)]
pub struct TxFields {
    pub from: Address,
    pub to: Address,
    pub data: String,
    /// tx value, in api is a decimal number as string
    pub value: String,
    /// gas price, in api is a decimal number as string
    #[serde(rename = "gasPrice")]
    pub gas_price: String,
    /// gas limit, in api is a decimal number
    pub gas: u128,
}

fn validate_slippage(slippage: f32) -> MmResult<(), ApiClientError> {
    if !(0.0..=ONE_INCH_MAX_SLIPPAGE).contains(&slippage) {
        return Err(ApiClientError::InvalidParam("invalid slippage param".to_owned()).into());
    }
    Ok(())
}

fn validate_fee(fee: &Option<f32>) -> MmResult<(), ApiClientError> {
    if let Some(fee) = fee {
        if !(0.0..=ONE_INCH_MAX_FEE_SHARE).contains(fee) {
            return Err(ApiClientError::InvalidParam("invalid fee param".to_owned()).into());
        }
    }
    Ok(())
}

fn validate_gas_limit(gas_limit: &Option<u128>) -> MmResult<(), ApiClientError> {
    if let Some(gas_limit) = gas_limit {
        if gas_limit > &ONE_INCH_MAX_GAS {
            return Err(ApiClientError::InvalidParam("invalid gas param".to_owned()).into());
        }
    }
    Ok(())
}

fn validate_parts(parts: &Option<u32>) -> MmResult<(), ApiClientError> {
    if let Some(parts) = parts {
        if parts > &ONE_INCH_MAX_PARTS {
            return Err(ApiClientError::InvalidParam("invalid max parts param".to_owned()).into());
        }
    }
    Ok(())
}

fn validate_main_route_parts(main_route_parts: &Option<u32>) -> MmResult<(), ApiClientError> {
    if let Some(parts) = main_route_parts {
        if parts > &ONE_INCH_MAX_MAIN_ROUTE_PARTS {
            return Err(ApiClientError::InvalidParam("invalid max main route parts param".to_owned()).into());
        }
    }
    Ok(())
}

fn validate_complexity_level(complexity_level: &Option<u32>) -> MmResult<(), ApiClientError> {
    if let Some(complexity_level) = complexity_level {
        if complexity_level > &ONE_INCH_MAX_COMPLEXITY_LEVEL {
            return Err(ApiClientError::InvalidParam("invalid max complexity level param".to_owned()).into());
        }
    }
    Ok(())
}
