use common::block_on;
use mm2_test_helpers::for_tests::{atom_testnet_conf, enable_tendermint, MarketMakerIt, Mm2TestConf};

const ATOM_TEST_BALANCE_SEED: &str = "atom test seed";
const ATOM_TICKER: &str = "ATOM";
const ATOM_TENDERMINT_RPC_URLS: &[&str] = &["https://cosmos-testnet-rpc.allthatnode.com:26657"];

#[test]
fn test_tendermint_activation() {
    let coins = json!([atom_testnet_conf()]);

    let conf = Mm2TestConf::seednode(ATOM_TEST_BALANCE_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, conf.local).unwrap();

    let activation_result = block_on(enable_tendermint(&mm, ATOM_TICKER, ATOM_TENDERMINT_RPC_URLS));

    println!("{}", serde_json::to_string(&activation_result).unwrap());
}
