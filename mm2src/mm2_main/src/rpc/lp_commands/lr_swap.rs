//? RPC implementations for swaps with liquidity routing (LR) of EVM tokens

use super::one_inch::types::ClassicSwapDetails;
use crate::rpc::lp_commands::one_inch::errors::ApiIntegrationRpcError;
use crate::rpc::lp_commands::one_inch::rpcs::get_coin_for_one_inch;
use coins::lp_coinfind_or_err;
use lr_impl::find_best_fill_ask_with_lr;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::{map_mm_error::MapMmError,
                     mm_error::{MmError, MmResult}};
use types::{LrBestQuoteRequest, LrBestQuoteResponse, LrFillOrderRequest, LrFillOrderResponse, LrQuotesForTokensRequest};

mod errors;
mod lr_impl;
mod types;

/// Find the best swap with liquidity routing of EVM tokens, to select from multiple orders.
/// For the provided list of orderbook entries this RPC will find out the most price-effective swap with LR.
///
/// More info:
/// User is interested in buying some coin. There are orders available with the desired coin but User does not have tokens to fill those orders.
/// User may fill the order with User my_token running an preliminary LR swap combined with the ordinary dex-swap (using 1inch as the LR provider).
/// Or, User may convert the tokens from the order into my_token with a subsequent LR swap.
///
/// User calls this RPC with an ask/bid order list, base or rel coin, amount to buy or sell and User token name to fill the order with.
/// The RPC runs 1inch swap quotes to convert User's my_token into orders' tokens (or backwards)
/// and returns the most price-effective swap path, taking into account order and LR prices.
/// TODO: should also returns total fees.
///
/// TODO: currently supported only ask orders with rel=token_x, with routing User's my_token into token_x before the dex-swap.
/// The RPC should also support:
/// bid orders with rel=token_x, with routing token_x into my_token after the dex-swap
/// ask orders with base=token_x, with routing token_x into my_token after the dex-swap
/// bid orders with base=token_x, with routing User's my_token into token_x before the dex-swap
pub async fn lr_best_quote_rpc(
    ctx: MmArc,
    req: LrBestQuoteRequest,
) -> MmResult<LrBestQuoteResponse, ApiIntegrationRpcError> {
    // TODO: add validation:
    // order.base_min_volume << req.amount <= order.base_max_volume
    // order.coin is supported in 1inch
    // order.price not zero
    // when best order is selected validate against req.rel_max_volume and req.rel_min_volume

    let coin = lp_coinfind_or_err(&ctx, &req.base).await?;
    let (eth_coin, _) = get_coin_for_one_inch(&ctx, &req.my_token).await?;
    let (swap_data, best_order, total_price) =
        find_best_fill_ask_with_lr(&ctx, req.my_token, &req.asks, &req.amount).await?;
    let lr_swap_details =
        ClassicSwapDetails::from_api_classic_swap_data(&ctx, eth_coin.chain_id(), swap_data, coin.decimals())
            .await
            .mm_err(|err| ApiIntegrationRpcError::ApiDataError(err.to_string()))?;
    Ok(LrBestQuoteResponse {
        lr_swap_details,
        best_order,
        total_price,
        // trade_fee: // TODO: implement later
    })
}

/// Find possible swaps with liquidity routing of several user tokens to fill one order.
/// For the provided single order the RPC searches for the most price-effective swap path with LR for user tokens.
///
/// More info:
/// User is interested in buying some coin. There is an order available the User would like to fill but the User does not have tokens from the order.
/// User calls this RPC with the order, desired coin name, amount to buy or sell and list of User tokens to convert to/from with LR.
/// The RPC calls several 1inch classic swap quotes (to find most efficient token conversions)
/// and return possible LR paths to fill the order, with total swap prices.
/// TODO: should also returns total fees.
///
/// NOTE: this RPC does not select the best quote between User tokens because it finds routes for different tokens (with own value),
/// so returns all of them.
/// That is, it's up to the User to select the most cost effective swap, for e.g. comparing token fiat value.
/// In fact, this could be done even in this RPC as 1inch also can get value in fiat but maybe User evaludation is more prefferable.
/// Again, it's a TODO.
pub async fn lr_quotes_for_tokens_rpc(
    _ctx: MmArc,
    _req: LrQuotesForTokensRequest,
) -> MmResult<LrBestQuoteResponse, ApiIntegrationRpcError> {
    // TODO: impl later
    MmError::err(ApiIntegrationRpcError::SomeError("unimplemented".to_owned()))
}

/// Run a swap with LR to fill a maker order
pub async fn lr_fill_order_rpc(
    _ctx: MmArc,
    _req: LrFillOrderRequest,
) -> MmResult<LrFillOrderResponse, ApiIntegrationRpcError> {
    MmError::err(ApiIntegrationRpcError::SomeError("unimplemented".to_owned()))
}

#[cfg(test)]
mod tests {
    use crate::lp_ordermatch::{OrderbookAddress, RpcOrderbookEntryV2};
    use crate::rpc::lp_commands::legacy::electrum;
    use crate::rpc::lp_commands::lr_swap::LrBestQuoteRequest;
    use coins::eth::EthCoin;
    use coins_activation::platform_for_tests::init_platform_coin_with_tokens_loop;
    use crypto::CryptoCtx;
    use mm2_number::{MmNumber, MmNumberMultiRepr};
    use mm2_test_helpers::for_tests::{btc_with_spv_conf, mm_ctx_with_custom_db_with_conf};
    use std::str::FromStr;
    use uuid::Uuid;

    /// Test to find best swap with LR.
    /// checks how to find an order from an utxo/token ask order list, which is the most price efficient if route from my token into the token in the order.
    /// With this test use --features test-ext-api and set ONE_INCH_API_TEST_AUTH env to the 1inch dev auth key
    /// TODO: make it mockable to run within CI
    #[tokio::test]
    async fn test_find_best_lr_swap_for_order_list() {
        let platform_coin = "ETH".to_owned();
        let base_conf = btc_with_spv_conf();
        let platform_coin_conf = json!({
            "coin": platform_coin.clone(),
            "name": "ethereum",
            "derivation_path": "m/44'/1'",
            "chain_id": 1,
            "protocol": {
                "type": "ETH"
            }
        });

        // WETH = 2696.90 USD
        let weth_conf = json!({
            "coin": "WETH-ERC20",
            "name": "WETH-ERC20",
            "derivation_path": "m/44'/1'",
            "chain_id": 1,
            "decimals": 18,
            "protocol": {
                "type": "ERC20",
                "protocol_data": {
                    "platform": "ETH",
                    "contract_address": "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
                }
            }
        });

        // BNB = 612.36 USD
        let bnb_conf = json!({
            "coin": "BNB-ERC20",
            "name": "BNB token",
            "derivation_path": "m/44'/1'",
            "chain_id": 1,
            "decimals": 18,
            "protocol": {
                "type": "ERC20",
                "protocol_data": {
                    "platform": "ETH",
                    "contract_address": "0xB8c77482e45F1F44dE1745F52C74426C631bDD52"
                }
            }
        });
        // AAVE 258.75 USD
        let aave_conf = json!({
            "coin": "AAVE-ERC20",
            "name": "AAVE token",
            "derivation_path": "m/44'/1'",
            "chain_id": 1,
            "decimals": 18,
            "protocol": {
                "type": "ERC20",
                "protocol_data": {
                    "platform": "ETH",
                    "contract_address": "0x7Fc66500c84A76Ad7e9c93437bFc5Ac33E2DDaE9"
                }
            }
        });

        let base_ticker = base_conf["coin"].as_str().unwrap().to_owned();
        let weth_ticker = weth_conf["coin"].as_str().unwrap().to_owned();
        let bnb_ticker = bnb_conf["coin"].as_str().unwrap().to_owned();
        let aave_ticker = aave_conf["coin"].as_str().unwrap().to_owned();

        let conf = json!({
            "coins": [base_conf, platform_coin_conf, weth_conf, bnb_conf, aave_conf],
            "1inch_api": "https://api.1inch.dev"
        });
        let ctx = mm_ctx_with_custom_db_with_conf(Some(conf));
        CryptoCtx::init_with_iguana_passphrase(ctx.clone(), "123").unwrap();

        electrum(
            ctx.clone(),
            json!({
                "coin": base_ticker,
                "mm2": 1,
                "method": "electrum",
                "servers": [
                    {"url": "electrum1.cipig.net:10001"},
                    {"url": "electrum2.cipig.net:10001"},
                    {"url": "electrum3.cipig.net:10001"}
                ],
                "tx_history": false
            }),
        )
        .await
        .unwrap();
        init_platform_coin_with_tokens_loop::<EthCoin>(
            ctx.clone(),
            serde_json::from_value(json!({
                "ticker": platform_coin.clone(),
                "rpc_mode": "Default",
                "nodes": [
                    {"url": "https://mainnet.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b"}
                ],
                "swap_contract_address": "0xeA6D65434A15377081495a9E7C5893543E7c32cB",
                "erc20_tokens_requests": [
                    {"ticker": weth_ticker.clone()},
                    {"ticker": bnb_ticker.clone()},
                    {"ticker": aave_ticker.clone()}
                ],
                "priv_key_policy": "ContextPrivKey"
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let asks = vec![
            RpcOrderbookEntryV2 {
                coin: bnb_ticker,
                address: OrderbookAddress::Transparent("RLL6n4ayAv1haokcEd1QUEYniyeoiYkn7W".into()),
                price: MmNumberMultiRepr::from(MmNumber::from("145.69")),
                pubkey: "02f3578fbc0fc76056eae34180a71e9190ee08ad05d40947aab7a286666e2ce798".to_owned(),
                uuid: Uuid::from_str("7f26dc6a-39ab-4685-b5f1-55f12268ea50").unwrap(),
                is_mine: false,
                base_max_volume: MmNumberMultiRepr::from(MmNumber::from("1")),
                base_min_volume: MmNumberMultiRepr::from(MmNumber::from("0.1")),
                rel_max_volume: MmNumberMultiRepr::from(MmNumber::from("145.69")),
                rel_min_volume: MmNumberMultiRepr::from(MmNumber::from("14.569")),
                conf_settings: Default::default(),
            },
            RpcOrderbookEntryV2 {
                coin: aave_ticker,
                address: OrderbookAddress::Transparent("RK1JDwZ1LvH47Tvqm6pQM7aSqC2Zo6JwRF".into()),
                price: MmNumberMultiRepr::from(MmNumber::from("370.334")),
                pubkey: "02470bfb8e7710be4a7c2b8e9ba4bcfc5362a71643e64fc2e33b0d64c844ee9123".to_owned(),
                uuid: Uuid::from_str("2aadf450-6a8e-4e4e-8b89-19ca10f23cc3").unwrap(),
                is_mine: false,
                base_max_volume: MmNumberMultiRepr::from(MmNumber::from("1")),
                base_min_volume: MmNumberMultiRepr::from(MmNumber::from("0.1")),
                rel_max_volume: MmNumberMultiRepr::from(MmNumber::from("370.334")),
                rel_min_volume: MmNumberMultiRepr::from(MmNumber::from("37.0334")),
                conf_settings: Default::default(),
            },
        ];

        let req = LrBestQuoteRequest {
            base: base_ticker,
            amount: "0.123".into(),
            asks,
            my_token: weth_ticker,
        };

        let response = super::lr_best_quote_rpc(ctx, req).await;
        assert!(response.is_ok());
        println!("response={:?}", response);
        println!("response total_price={}", response.unwrap().total_price.to_decimal());
        // BTC / WETH price around 35.0
    }
}
