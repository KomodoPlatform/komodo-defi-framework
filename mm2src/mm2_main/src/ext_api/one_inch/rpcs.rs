use super::errors::ApiIntegrationRpcError;
use super::types::{AggregationContractRequest, ClassicSwapCreateRequest, ClassicSwapQuoteRequest, ClassicSwapResponse};
use coins::eth::{display_eth_address, wei_from_big_decimal, EthCoin, EthCoinType};
use coins::{lp_coinfind_or_err, CoinWithDerivationMethod, MmCoin, MmCoinEnum};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use trading_api::one_inch_api::client::ApiClient;
use trading_api::one_inch_api::types::{ClassicSwapCreateParams, ClassicSwapQuoteParams};

/// "1inch_v6_0_classic_swap_contract" rpc impl
/// used to get contract address (for e.g. to approve funds)
pub async fn one_inch_v6_0_classic_swap_contract_rpc(
    _ctx: MmArc,
    _req: AggregationContractRequest,
) -> MmResult<String, ApiIntegrationRpcError> {
    Ok(ApiClient::classic_swap_contract().to_owned())
}

/// "1inch_classic_swap_quote" rpc impl
pub async fn one_inch_v6_0_classic_swap_quote_rpc(
    ctx: MmArc,
    req: ClassicSwapQuoteRequest,
) -> MmResult<ClassicSwapResponse, ApiIntegrationRpcError> {
    let (base, base_contract) = get_coin_for_one_inch(&ctx, &req.base).await?;
    api_supports_coin(&base)?;
    let (rel, rel_contract) = get_coin_for_one_inch(&ctx, &req.rel).await?;
    let sell_amount = wei_from_big_decimal(&req.amount.to_decimal(), base.decimals())
        .mm_err(|err| ApiIntegrationRpcError::InvalidParam(err.to_string()))?;
    let query_params = ClassicSwapQuoteParams::new(base_contract, rel_contract, sell_amount.to_string())
        .with_fee(req.fee)
        .with_protocols(req.protocols)
        .with_gas_price(req.gas_price)
        .with_complexity_level(req.complexity_level)
        .with_parts(req.parts)
        .with_main_route_parts(req.main_route_parts)
        .with_gas_limit(req.gas_limit)
        .with_include_tokens_info(Some(req.include_tokens_info))
        .with_include_protocols(Some(req.include_protocols))
        .with_include_gas(Some(req.include_gas))
        .with_connector_tokens(req.connector_tokens)
        .build_query_params()
        .mm_err(|api_err| ApiIntegrationRpcError::from_api_error(api_err, base.decimals()))?;
    let quote = ApiClient::new(ctx)
        .mm_err(|api_err| ApiIntegrationRpcError::from_api_error(api_err, base.decimals()))?
        .call_swap_api(base.chain_id(), ApiClient::get_quote_method().to_owned(), query_params)
        .await
        .mm_err(|api_err| ApiIntegrationRpcError::from_api_error(api_err, base.decimals()))?; // use 'base' as amount in errors is in the src coin
    ClassicSwapResponse::from_api_value(quote, rel.decimals()) // use 'rel' as quote value is in the dst coin
        .mm_err(|err| ApiIntegrationRpcError::ApiDataError(err.to_string()))
}

/// "1inch_classic_swap_create" rpc implementation
/// This rpc actually returns a transaction to call the 1inch swap aggregation contract. GUI should sign it and send to the chain.
/// We don't verify the transaction in any way and trust the 1inch api.
pub async fn one_inch_v6_0_classic_swap_create_rpc(
    ctx: MmArc,
    req: ClassicSwapCreateRequest,
) -> MmResult<ClassicSwapResponse, ApiIntegrationRpcError> {
    let (base, base_contract) = get_coin_for_one_inch(&ctx, &req.base).await?;
    api_supports_coin(&base)?;
    let (_, rel_contract) = get_coin_for_one_inch(&ctx, &req.rel).await?;

    let sell_amount = wei_from_big_decimal(&req.amount.to_decimal(), base.decimals())
        .mm_err(|err| ApiIntegrationRpcError::InvalidParam(err.to_string()))?;
    let single_address = base.derivation_method().single_addr_or_err().await?;

    let query_params = ClassicSwapCreateParams::new(
        base_contract,
        rel_contract,
        sell_amount.to_string(),
        display_eth_address(&single_address),
        req.slippage,
    )
    .with_fee(req.fee)
    .with_protocols(req.protocols)
    .with_gas_price(req.gas_price)
    .with_complexity_level(req.complexity_level)
    .with_parts(req.parts)
    .with_main_route_parts(req.main_route_parts)
    .with_gas_limit(req.gas_limit)
    .with_include_tokens_info(Some(req.include_tokens_info))
    .with_include_protocols(Some(req.include_protocols))
    .with_include_gas(Some(req.include_gas))
    .with_connector_tokens(req.connector_tokens)
    .with_permit(req.permit)
    .with_receiver(req.receiver)
    .with_referrer(req.referrer)
    .with_disable_estimate(req.disable_estimate)
    .with_allow_partial_fill(req.allow_partial_fill)
    .build_query_params()
    .mm_err(|api_err| ApiIntegrationRpcError::from_api_error(api_err, base.decimals()))?;
    let swap_with_tx = ApiClient::new(ctx)
        .mm_err(|api_err| ApiIntegrationRpcError::from_api_error(api_err, base.decimals()))?
        .call_swap_api(base.chain_id(), ApiClient::get_swap_method().to_owned(), query_params)
        .await
        .mm_err(|api_err| ApiIntegrationRpcError::from_api_error(api_err, base.decimals()))?; // use 'base' as amount in errors is in the src coin
    ClassicSwapResponse::from_api_value(swap_with_tx, base.decimals()) // use 'base' as we spend in the src coin
        .mm_err(|err| ApiIntegrationRpcError::ApiDataError(err.to_string()))
}

async fn get_coin_for_one_inch(ctx: &MmArc, ticker: &str) -> MmResult<(EthCoin, String), ApiIntegrationRpcError> {
    let coin = match lp_coinfind_or_err(ctx, ticker).await? {
        MmCoinEnum::EthCoin(coin) => coin,
        _ => return Err(MmError::new(ApiIntegrationRpcError::CoinTypeError)),
    };
    let contract = match coin.coin_type {
        EthCoinType::Eth => ApiClient::eth_special_contract().to_owned(),
        EthCoinType::Erc20 { token_addr, .. } => display_eth_address(&token_addr),
        EthCoinType::Nft { .. } => return Err(MmError::new(ApiIntegrationRpcError::NftNotSupported)),
    };
    Ok((coin, contract))
}

#[allow(clippy::result_large_err)]
fn api_supports_coin(coin: &EthCoin) -> MmResult<(), ApiIntegrationRpcError> {
    if ApiClient::is_chain_supported(coin.chain_id()) {
        Ok(())
    } else {
        Err(MmError::new(ApiIntegrationRpcError::ChainNotSupported))
    }
}
