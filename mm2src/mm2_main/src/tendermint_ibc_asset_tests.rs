use super::*;
use mm2_test_helpers::for_tests::{enable_tendermint, iris_testnet_conf, my_balance, usdc_ibc_iris_testnet_conf};

const IRIS_TESTNET_RPCS: &[&str] = &["http://34.80.202.172:26657"];
const IRIS_TICKER: &str = "IRIS-TEST";
const USDC_IBC_TICKER: &str = "USDC-IBC-IRIS";
const IRIS_USDC_ACTIVATION_SEED: &str = "iris usdc activation";

#[test]
fn test_iris_with_usdc_activation_and_balance() {
    let coins = json!([iris_testnet_conf(), usdc_ibc_iris_testnet_conf()]);

    let conf = Mm2TestConf::seednode(IRIS_USDC_ACTIVATION_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, conf.local).unwrap();

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

    let actual_usdc_balance = response.result.ibc_assets_balances.get(USDC_IBC_TICKER).unwrap();
    assert_eq!(actual_usdc_balance.spendable, expected_usdc_balance);

    let usdc_balance_response = block_on(my_balance(&mm, USDC_IBC_TICKER));
    let actual_usdc_balance: MyBalanceResponse = json::from_value(usdc_balance_response).unwrap();
    assert_eq!(actual_usdc_balance.balance, expected_usdc_balance);
}
