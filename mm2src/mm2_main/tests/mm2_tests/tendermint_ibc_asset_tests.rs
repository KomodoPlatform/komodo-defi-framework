use std::collections::HashSet;
use std::iter::FromIterator;

use common::block_on;
use mm2_number::BigDecimal;
use mm2_rpc::data::legacy::OrderbookResponse;
use mm2_test_helpers::for_tests::{enable_tendermint, enable_tendermint_without_balance, iris_testnet_conf, my_balance,
                                  orderbook, orderbook_v2, set_price, usdc_ibc_iris_testnet_conf, MarketMakerIt,
                                  Mm2TestConf};
use mm2_test_helpers::structs::{OrderbookAddress, OrderbookV2Response, RpcV2Response, TendermintActivationResult};
use serde_json::{self, json};

const IRIS_TESTNET_RPCS: &[&str] = &["http://34.80.202.172:26657"];
const IRIS_TICKER: &str = "IRIS-TEST";
const USDC_IBC_TICKER: &str = "USDC-IBC-IRIS";
const IRIS_USDC_ACTIVATION_SEED: &str = "iris usdc activation";

#[test]
fn test_iris_with_usdc_activation_balance_orderbook() {
    let coins = json!([iris_testnet_conf(), usdc_ibc_iris_testnet_conf()]);

    let conf = Mm2TestConf::seednode(IRIS_USDC_ACTIVATION_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_result = block_on(enable_tendermint(
        &mm,
        IRIS_TICKER,
        &[USDC_IBC_TICKER],
        IRIS_TESTNET_RPCS,
        false,
    ));

    let response: RpcV2Response<TendermintActivationResult> = serde_json::from_value(activation_result).unwrap();

    let expected_address = "iaa1udqnpvaw3uyv3gsl7m6800wyask5wj7quvd4nm";
    assert_eq!(response.result.address, expected_address);

    let expected_iris_balance = BigDecimal::from(100);
    assert_eq!(response.result.balance.unwrap().spendable, expected_iris_balance);

    let expected_usdc_balance: BigDecimal = "0.683142".parse().unwrap();

    let tokens_balances = response.result.tokens_balances.unwrap();
    let actual_usdc_balance = tokens_balances.get(USDC_IBC_TICKER).unwrap();
    assert_eq!(actual_usdc_balance.spendable, expected_usdc_balance);

    let actual_usdc_balance = block_on(my_balance(&mm, USDC_IBC_TICKER)).balance;
    assert_eq!(actual_usdc_balance, expected_usdc_balance);

    let set_price_res = block_on(set_price(&mm, USDC_IBC_TICKER, IRIS_TICKER, "1", "0.1", false));
    println!("{:?}", set_price_res);

    let set_price_res = block_on(set_price(&mm, IRIS_TICKER, USDC_IBC_TICKER, "1", "0.1", false));
    println!("{:?}", set_price_res);

    let orderbook = block_on(orderbook(&mm, USDC_IBC_TICKER, IRIS_TICKER));
    let orderbook: OrderbookResponse = serde_json::from_value(orderbook).unwrap();

    let first_ask = orderbook.asks.first().unwrap();
    assert_eq!(first_ask.entry.address, expected_address);

    let first_bid = orderbook.bids.first().unwrap();
    assert_eq!(first_bid.entry.address, expected_address);

    let orderbook_v2 = block_on(orderbook_v2(&mm, USDC_IBC_TICKER, IRIS_TICKER));
    let orderbook_v2: RpcV2Response<OrderbookV2Response> = serde_json::from_value(orderbook_v2).unwrap();

    let expected_address = OrderbookAddress::Transparent(expected_address.into());
    let first_ask = orderbook_v2.result.asks.first().unwrap();
    assert_eq!(first_ask.entry.address, expected_address);

    let first_bid = orderbook_v2.result.bids.first().unwrap();
    assert_eq!(first_bid.entry.address, expected_address);
}

#[test]
fn test_iris_with_usdc_activation_without_balance() {
    let coins = json!([iris_testnet_conf(), usdc_ibc_iris_testnet_conf()]);

    let conf = Mm2TestConf::seednode(IRIS_USDC_ACTIVATION_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_result = block_on(enable_tendermint_without_balance(
        &mm,
        IRIS_TICKER,
        &[USDC_IBC_TICKER],
        IRIS_TESTNET_RPCS,
        false,
    ));

    let result: RpcV2Response<TendermintActivationResult> = serde_json::from_value(activation_result).unwrap();

    assert!(result.result.balance.is_none());
    assert!(result.result.tokens_balances.is_none());
    assert_eq!(
        result.result.tokens_tickers.unwrap(),
        HashSet::from_iter(vec![USDC_IBC_TICKER.to_string()])
    );
}
