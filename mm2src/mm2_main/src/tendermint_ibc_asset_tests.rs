use super::*;
use mm2_test_helpers::for_tests::{enable_tendermint, iris_testnet_conf, my_balance, orderbook, orderbook_v2,
                                  set_price, usdc_ibc_iris_testnet_conf};

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
    ));

    let response: RpcV2Response<TendermintActivationResult> = json::from_value(activation_result).unwrap();

    let expected_address = "iaa1udqnpvaw3uyv3gsl7m6800wyask5wj7quvd4nm";
    assert_eq!(response.result.address, expected_address);

    let expected_iris_balance = BigDecimal::from(100);
    assert_eq!(response.result.balance.spendable, expected_iris_balance);

    let expected_usdc_balance: BigDecimal = "0.683142".parse().unwrap();

    let actual_usdc_balance = response.result.tokens_balances.get(USDC_IBC_TICKER).unwrap();
    assert_eq!(actual_usdc_balance.spendable, expected_usdc_balance);

    let usdc_balance_response = block_on(my_balance(&mm, USDC_IBC_TICKER));
    let actual_usdc_balance: MyBalanceResponse = json::from_value(usdc_balance_response).unwrap();
    assert_eq!(actual_usdc_balance.balance, expected_usdc_balance);

    let set_price_res = block_on(set_price(&mm, USDC_IBC_TICKER, IRIS_TICKER, "1", "0.1"));
    let set_price_res: SetPriceResponse = json::from_value(set_price_res).unwrap();
    println!("{:?}", set_price_res);

    let set_price_res = block_on(set_price(&mm, IRIS_TICKER, USDC_IBC_TICKER, "1", "0.1"));
    let set_price_res: SetPriceResponse = json::from_value(set_price_res).unwrap();
    println!("{:?}", set_price_res);

    let orderbook = block_on(orderbook(&mm, USDC_IBC_TICKER, IRIS_TICKER));
    let orderbook: OrderbookResponse = json::from_value(orderbook).unwrap();

    let first_ask = orderbook.asks.first().unwrap();
    assert_eq!(first_ask.address, expected_address);

    let first_bid = orderbook.bids.first().unwrap();
    assert_eq!(first_bid.address, expected_address);

    let orderbook_v2 = block_on(orderbook_v2(&mm, USDC_IBC_TICKER, IRIS_TICKER));
    let orderbook_v2: RpcV2Response<OrderbookV2Response> = json::from_value(orderbook_v2).unwrap();

    let expected_address = OrderbookAddress::Transparent(expected_address.into());
    let first_ask = orderbook_v2.result.asks.first().unwrap();
    assert_eq!(first_ask.entry.address, expected_address);

    let first_bid = orderbook_v2.result.bids.first().unwrap();
    assert_eq!(first_bid.entry.address, expected_address);
}
