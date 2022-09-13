use super::*;
use mm2_test_helpers::for_tests::{enable_tendermint, iris_testnet_conf, usdc_ibc_iris_testnet_conf};

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

    let result: RpcV2Response<TendermintActivationResult> = json::from_value(activation_result).unwrap();
    println!("{:?}", result);
}
