//! Swap with LR in ETH chain, implementation code

use crate::lp_ordermatch::RpcOrderbookEntryV2;
use crate::rpc::lp_commands::one_inch::errors::ApiIntegrationRpcError;
use crate::rpc::lp_commands::one_inch::rpcs::get_coin_for_one_inch;
use coins::eth::eth_addr_to_hex;
use coins::eth::{u256_to_big_decimal, wei_from_big_decimal};
use coins::lp_coinfind_or_err;
use coins::MmCoin;
use coins::NumConversResult;
use coins::Ticker;
use common::log;
use ethereum_types::Address as EthAddress;
use ethereum_types::{FromDecStrErr, U256};
use futures::future::join_all;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::MmNumber;
use num_traits::CheckedDiv;
use std::collections::HashMap;
use std::str::FromStr;
use trading_api::one_inch_api::classic_swap_types::{ClassicSwapData, ClassicSwapQuoteParams};
use trading_api::one_inch_api::client::ApiClient;
use trading_api::one_inch_api::portfolio_types::{CrossPriceParams, CrossPricesData, DataGranularity};

/// To estimate src/dst price query price history for last 5 min
const CROSS_PRICES_GRANULARITY: DataGranularity = DataGranularity::FiveMin;
/// Use no more than 10 price history samples to estimate src/dst price
const CROSS_PRICES_LIMIT: u32 = 10;

#[inline]
fn mm_number_to_u256(mm_number: &MmNumber) -> Result<U256, FromDecStrErr> {
    U256::from_dec_str(mm_number.to_ratio().to_integer().to_string().as_str())
}

#[inline]
fn mm_number_from_u256(u256: U256) -> MmNumber { MmNumber::from(u256.to_string().as_str()) }

#[inline]
fn wei_from_coins_mm_number(mm_number: &MmNumber, decimals: u8) -> NumConversResult<U256> {
    wei_from_big_decimal(&mm_number.to_decimal(), decimals)
}

#[inline]
#[allow(unused)]
fn wei_to_coins_mm_number(u256: U256, decimals: u8) -> NumConversResult<MmNumber> {
    Ok(MmNumber::from(u256_to_big_decimal(u256, decimals)?))
}

/// Internal struct to collect data for selecting the best swap with LR
struct LrData {
    order: RpcOrderbookEntryV2,
    src_contract: Option<EthAddress>,
    /// Source token (to do LR from) amount in wei
    src_amount: Option<U256>,
    src_decimals: Option<u8>,
    dst_contract: Option<EthAddress>,
    /// Destination token (to do LR into) amount in wei
    dst_amount: Option<U256>,
    dst_decimals: Option<u8>,
    chain_id: Option<u64>,
    /// Queried src token / dst token price
    lr_price: Option<MmNumber>,
    lr_swap_data: Option<ClassicSwapData>,
}

struct LrDataMap {
    /// Map to store data needed for swaps with LR estimations,
    /// the key is source and destination tokens
    inner: HashMap<(Ticker, Ticker), LrData>,
}

impl LrDataMap {
    /// Init LR data map from the source token (mytoken) and tokens from orders
    fn new_with_src_token(src_token: Ticker, orders: &[RpcOrderbookEntryV2]) -> Self {
        Self {
            inner: orders
                .iter()
                .map(|order| {
                    ((src_token.clone(), order.coin.clone()), LrData {
                        order: order.clone(),
                        src_contract: None,
                        src_decimals: None,
                        src_amount: None,
                        dst_contract: None,
                        dst_amount: None,
                        dst_decimals: None,
                        chain_id: None,
                        lr_price: None,
                        lr_swap_data: None,
                    })
                })
                .collect::<HashMap<(_, _), _>>(),
        }
    }

    /// Calculate amounts of destination tokens required to fill ask orders for the requested base_amount:
    /// multiplies base_amount by the order price (base_amount must be in 'coins')
    async fn calc_destination_token_amounts(
        &mut self,
        ctx: &MmArc,
        base_amount: &MmNumber,
    ) -> MmResult<(), ApiIntegrationRpcError> {
        for lr_data in self.inner.values_mut() {
            let price: MmNumber = lr_data.order.price.rational.clone().into();
            let dst_amount = base_amount * &price;
            let coin = lp_coinfind_or_err(ctx, &lr_data.order.coin).await?;
            lr_data.dst_amount = Some(wei_from_coins_mm_number(&dst_amount, coin.decimals())?);
            log::debug!(
                "calc_destination_token_amounts lr_data.order.coin={} coin.decimals()={} lr_data.dst_amount={:?}",
                lr_data.order.coin,
                coin.decimals(),
                lr_data.dst_amount
            );
        }
        Ok(())
    }

    fn update_with_lr_prices(&mut self, lr_prices: &HashMap<(Ticker, Ticker), MmNumber>) {
        for (key, val) in self.inner.iter_mut() {
            val.lr_price = lr_prices.get(key).cloned();
        }
    }

    fn update_with_lr_swap_data(&mut self, lr_swap_data: &HashMap<(Ticker, Ticker), ClassicSwapData>) {
        for (key, val) in self.inner.iter_mut() {
            val.lr_swap_data = lr_swap_data.get(key).cloned();
        }
    }

    async fn update_with_contracts(&mut self, ctx: &MmArc) -> MmResult<(), ApiIntegrationRpcError> {
        for ((src_token, dst_token), lr_data) in self.inner.iter_mut() {
            let (src_coin, src_contract) = get_coin_for_one_inch(ctx, src_token).await?;
            let (dst_coin, dst_contract) = get_coin_for_one_inch(ctx, dst_token).await?;
            lr_data.src_contract = EthAddress::from_str(&src_contract).ok(); // TODO: return error?
            lr_data.dst_contract = EthAddress::from_str(&dst_contract).ok();
            lr_data.src_decimals = Some(src_coin.decimals()); // TODO: maybe we should check for 0 and return error (if missed in coins)? Now it works okay if decimals omitted in coins
            lr_data.dst_decimals = Some(dst_coin.decimals());
            lr_data.chain_id = Some(dst_coin.chain_id());
        }
        Ok(())
    }

    /// Query 1inch token_0/token_1 price in series and calc average price
    /// Assuming the outer RPC-level code ensures that relation src_tokens : dst_tokens will never be M:N (but only 1:M or M:1)
    async fn query_destination_token_prices(&mut self, ctx: &MmArc) -> MmResult<(), ApiIntegrationRpcError> {
        let mut prices_futs = vec![];
        let mut src_dst = vec![];
        for ((src_token, dst_token), lr_data) in self.inner.iter() {
            // Run src / dst token price query:
            let src_contract = eth_addr_to_hex(
                lr_data
                    .src_contract
                    .as_ref()
                    .ok_or(ApiIntegrationRpcError::InternalError("no contract".to_owned()))?,
            );
            let dst_contract = eth_addr_to_hex(
                lr_data
                    .dst_contract
                    .as_ref()
                    .ok_or(ApiIntegrationRpcError::InternalError("no contract".to_owned()))?,
            );
            let chain_id = lr_data
                .chain_id
                .ok_or(ApiIntegrationRpcError::InternalError("no chain id".to_owned()))?;
            let query_params = CrossPriceParams::new(chain_id, src_contract, dst_contract)
                .with_granularity(Some(CROSS_PRICES_GRANULARITY))
                .with_limit(Some(CROSS_PRICES_LIMIT))
                .build_query_params()
                .mm_err(|api_err| ApiIntegrationRpcError::from_api_error(api_err, lr_data.dst_decimals))?;
            let fut = ApiClient::new(ctx)
                .mm_err(|api_err| ApiIntegrationRpcError::from_api_error(api_err, lr_data.dst_decimals))?
                .call_one_inch_api::<Vec<CrossPricesData>>(
                    None,
                    ApiClient::portfolio_prices_endpoint(),
                    ApiClient::cross_prices_method().to_owned(),
                    Some(query_params),
                );
            prices_futs.push(fut);
            src_dst.push((src_token.clone(), dst_token.clone()));
        }
        let prices_in_series = join_all(prices_futs)
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>() // if an error in any future - return the error
            .mm_err(|err| ApiIntegrationRpcError::from_api_error(err, None))?;

        let quotes = src_dst
            .into_iter()
            .zip(prices_in_series.iter())
            .map(|((src, dst), series)| {
                let dst_price = cross_prices_average(series);
                ((src, dst), dst_price)
            })
            .collect::<HashMap<(_, _), _>>();
        for q in &quotes {
            log::debug!(
                "query_destination_token_prices src/dst quote={:?} {}",
                q,
                q.1.to_decimal()
            );
        }
        self.update_with_lr_prices(&quotes);
        Ok(())
    }

    /// Estimate the needed source amount for LR swap, by dividing the known dst amount by the src/dst price
    #[allow(clippy::result_large_err)]
    fn estimate_source_token_amounts(&mut self) -> MmResult<(), ApiIntegrationRpcError> {
        for lr_data in self.inner.values_mut() {
            let Some(ref dst_price) = lr_data.lr_price else {
                continue;
            };
            let dst_amount = lr_data
                .dst_amount
                .ok_or(ApiIntegrationRpcError::InternalError("no contract".to_owned()))?;
            let dst_amount = mm_number_from_u256(dst_amount);
            if let Some(src_amount) = &dst_amount.checked_div(dst_price) {
                lr_data.src_amount = Some(mm_number_to_u256(src_amount)?);
                log::debug!(
                    "estimate_source_token_amounts lr_data.order.coin={} dst_price={} lr_data.src_amount={:?}",
                    lr_data.order.coin,
                    dst_price.to_decimal(),
                    lr_data.src_amount
                );
            }
        }
        Ok(())
    }

    /// Run 1inch requests to get LR quotes to convert source tokens to tokens in orders
    async fn run_lr_quotes(&mut self, ctx: &MmArc) -> MmResult<(), ApiIntegrationRpcError> {
        let mut src_dst = vec![];
        let mut quote_futs = vec![];
        for ((src_token, dst_token), lr_data) in self.inner.iter() {
            let Some(src_amount) = lr_data.src_amount else {
                continue;
            };
            let src_contract = eth_addr_to_hex(
                lr_data
                    .src_contract
                    .as_ref()
                    .ok_or(ApiIntegrationRpcError::InternalError("no contract".to_owned()))?,
            );
            let dst_contract = eth_addr_to_hex(
                lr_data
                    .dst_contract
                    .as_ref()
                    .ok_or(ApiIntegrationRpcError::InternalError("no contract".to_owned()))?,
            );
            let chain_id = lr_data
                .chain_id
                .ok_or(ApiIntegrationRpcError::InternalError("no chain id".to_owned()))?;
            let query_params = ClassicSwapQuoteParams::new(src_contract, dst_contract, src_amount.to_string())
                .with_include_tokens_info(Some(true))
                .with_include_gas(Some(true))
                .build_query_params()
                .mm_err(|api_err| ApiIntegrationRpcError::from_api_error(api_err, lr_data.dst_decimals))?;
            let api_client = ApiClient::new(ctx)
                .mm_err(|api_err| ApiIntegrationRpcError::from_api_error(api_err, lr_data.dst_decimals))?;
            let fut = api_client.call_one_inch_api::<ClassicSwapData>(
                Some(chain_id),
                ApiClient::classic_swap_endpoint(),
                ApiClient::quote_method().to_owned(),
                Some(query_params),
            );
            quote_futs.push(fut);
            src_dst.push((src_token.clone(), dst_token.clone()));
        }

        let swap_data = join_all(quote_futs)
            .await
            .into_iter()
            .collect::<MmResult<Vec<_>, _>>()
            .mm_err(|cli_err| ApiIntegrationRpcError::from_api_error(cli_err, None))?;
        let swap_data_map = src_dst
            .into_iter()
            .zip(swap_data.into_iter())
            .collect::<HashMap<(_, _), _>>();
        self.update_with_lr_swap_data(&swap_data_map);
        Ok(())
    }

    /// Select the best swap path, by minimum of total swap price (including order and LR swap)
    #[allow(clippy::result_large_err)]
    fn select_best_swap(&self) -> MmResult<(ClassicSwapData, RpcOrderbookEntryV2, MmNumber), ApiIntegrationRpcError> {
        // Calculate swap's total_price (filling the order plus LR swap) as src_amount / order_amount
        // where src_amount is user tokens to pay for the swap with LR, 'order_amount' is amount which will fill the order
        // Tx fee is not accounted here because it is in the platform coin, not token, so we can't compare LR swap tx fee directly here.
        // Instead, GUI may calculate and show to the user the total spendings for LR swap, including fees, in USD or other fiat currency
        let calc_total_price = |src_amount: U256, lr_swap: &ClassicSwapData, order: &RpcOrderbookEntryV2| {
            let src_amount = mm_number_from_u256(src_amount);
            let order_price = MmNumber::from(order.price.rational.clone());
            let dst_amount = MmNumber::from(lr_swap.dst_amount.to_string().as_str());
            if let Some(order_amount) = dst_amount.checked_div(&order_price) {
                let total_price = src_amount.checked_div(&order_amount);
                log::debug!("select_best_swap order.coin={} lr_swap.dst_amount(wei)={} order_amount(to fill order, wei)={} total_price(with LR)={}", 
                    order.coin, lr_swap.dst_amount, order_amount.to_decimal(), total_price.clone().unwrap_or(MmNumber::from(0)).to_decimal());
                total_price
            } else {
                None
            }
        };

        self.inner
            .values()
            // filter out orders for which we did not get LR swap quotes and were not able to estimate needed source amount
            .filter_map(|lr_data| {
                if lr_data.src_amount.is_some() && lr_data.lr_swap_data.is_some() {
                    Some((
                        lr_data.src_amount.unwrap(),
                        lr_data.lr_swap_data.as_ref().unwrap().clone(),
                        lr_data.order.clone(),
                    ))
                } else {
                    None
                }
            })
            // calculate total price and filter out orders for which we could not calculate the total price
            .filter_map(|(src_amount, lr_swap_data, order)| {
                calc_total_price(src_amount, &lr_swap_data, &order)
                    .map(|total_price| (lr_swap_data, order, total_price))
            })
            .min_by(|(_, _, price_0), (_, _, price_1)| price_0.cmp(price_1))
            .map(|(lr_swap_data, order, price)| (lr_swap_data, order, price))
            .ok_or(MmError::new(ApiIntegrationRpcError::SomeError(
                "Best swap not found".to_owned(),
            )))
    }
}

/// Finds the best swap path to buy order 's best "UTXO" coins, including LR quotes to sell my token for the rel tokens from the orders
/// base_amount is amount of UTXO coins user would like to buy
pub async fn find_best_fill_ask_with_lr(
    ctx: &MmArc,
    user_token: Ticker,
    orders: &[RpcOrderbookEntryV2],
    base_amount: &MmNumber,
) -> MmResult<(ClassicSwapData, RpcOrderbookEntryV2, MmNumber), ApiIntegrationRpcError> {
    let mut lr_data_map = LrDataMap::new_with_src_token(user_token, orders);
    let _ = lr_data_map.update_with_contracts(ctx).await;
    let _ = lr_data_map.calc_destination_token_amounts(ctx, base_amount).await;
    let _ = lr_data_map.query_destination_token_prices(ctx).await?;
    let _ = lr_data_map.estimate_source_token_amounts();
    let _ = lr_data_map.run_lr_quotes(ctx).await?;

    lr_data_map.select_best_swap()
}

/// Helper to process 1inch token cross prices data and return average price
fn cross_prices_average(series: &Vec<CrossPricesData>) -> MmNumber {
    if series.is_empty() {
        return MmNumber::from(0);
    }
    let total: MmNumber = series.iter().fold(MmNumber::from(0), |acc, price_data| {
        acc + MmNumber::from(price_data.avg.clone())
    });
    total / MmNumber::from(series.len() as i32)
}
