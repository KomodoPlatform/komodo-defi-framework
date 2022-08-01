use crate::mm2::mm2_tests::structs::{RpcV2Response, TendermintActivationResult};
use common::block_on;
use mm2_number::BigDecimal;
use mm2_test_helpers::for_tests::{atom_testnet_conf, enable_tendermint, send_raw_transaction, withdraw_v1,
                                  MarketMakerIt, Mm2TestConf};
use serde_json as json;

const ATOM_TEST_BALANCE_SEED: &str = "atom test seed";
const ATOM_TICKER: &str = "ATOM";
const ATOM_TENDERMINT_RPC_URLS: &[&str] = &["https://cosmos-testnet-rpc.allthatnode.com:26657"];

#[test]
fn test_tendermint_activation() {
    let coins = json!([atom_testnet_conf()]);

    let conf = Mm2TestConf::seednode(ATOM_TEST_BALANCE_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, conf.local).unwrap();

    let activation_result = block_on(enable_tendermint(&mm, ATOM_TICKER, ATOM_TENDERMINT_RPC_URLS));

    let result: RpcV2Response<TendermintActivationResult> = json::from_value(activation_result).unwrap();
    assert_eq!(result.result.address, "cosmos1svaw0aqc4584x825ju7ua03g5xtxwd0ahl86hz");
    let expected_balance: BigDecimal = "1.8989".parse().unwrap();
    assert_eq!(result.result.balance.spendable, expected_balance);
}

#[test]
#[ignore]
fn test_tendermint_withdraw() {
    let coins = json!([atom_testnet_conf()]);

    let conf = Mm2TestConf::seednode(ATOM_TEST_BALANCE_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, conf.local).unwrap();

    block_on(enable_tendermint(&mm, ATOM_TICKER, ATOM_TENDERMINT_RPC_URLS));

    let withdraw_result = block_on(withdraw_v1(
        &mm,
        ATOM_TICKER,
        "cosmos1svaw0aqc4584x825ju7ua03g5xtxwd0ahl86hz",
        "0.1",
    ));
    println!("{}", json::to_string(&withdraw_result).unwrap());

    let tx_hex = withdraw_result["tx_hex"].as_str().unwrap();
    let send_raw_tx = block_on(send_raw_transaction(&mm, ATOM_TICKER, tx_hex));
    println!("{}", json::to_string(&send_raw_tx).unwrap());
}
