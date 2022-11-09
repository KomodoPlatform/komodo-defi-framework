use common::block_on;
use mm2_number::BigDecimal;
use mm2_test_helpers::for_tests::{atom_testnet_conf, enable_tendermint, enable_tendermint_token,
                                  iris_nimda_testnet_conf, iris_testnet_conf, my_balance, send_raw_transaction,
                                  withdraw_v1, MarketMakerIt, Mm2TestConf};
use mm2_test_helpers::structs::{MyBalanceResponse, RpcV2Response, TendermintActivationResult, TransactionDetails};
use serde_json::{self as json, json};

const ATOM_TEST_BALANCE_SEED: &str = "atom test seed";
const ATOM_TEST_WITHDRAW_SEED: &str = "atom test withdraw seed";
const ATOM_TICKER: &str = "ATOM";
const ATOM_TENDERMINT_RPC_URLS: &[&str] = &["https://cosmos-testnet-rpc.allthatnode.com:26657"];

#[test]
fn test_tendermint_activation_and_balance() {
    let coins = json!([atom_testnet_conf()]);
    let expected_address = "cosmos1svaw0aqc4584x825ju7ua03g5xtxwd0ahl86hz";

    let conf = Mm2TestConf::seednode(ATOM_TEST_BALANCE_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_result = block_on(enable_tendermint(&mm, ATOM_TICKER, &[], ATOM_TENDERMINT_RPC_URLS));

    let result: RpcV2Response<TendermintActivationResult> = json::from_value(activation_result).unwrap();
    assert_eq!(result.result.address, expected_address);
    let expected_balance: BigDecimal = "0.0959".parse().unwrap();
    assert_eq!(result.result.balance.spendable, expected_balance);

    let my_balance_result = block_on(my_balance(&mm, ATOM_TICKER));
    let my_balance: MyBalanceResponse = json::from_value(my_balance_result).unwrap();

    assert_eq!(my_balance.balance, expected_balance);
    assert_eq!(my_balance.unspendable_balance, BigDecimal::default());
    assert_eq!(my_balance.address, expected_address);
    assert_eq!(my_balance.coin, ATOM_TICKER);
}

#[test]
fn test_tendermint_withdraw() {
    let coins = json!([atom_testnet_conf()]);

    let conf = Mm2TestConf::seednode(ATOM_TEST_WITHDRAW_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_res = block_on(enable_tendermint(&mm, ATOM_TICKER, &[], ATOM_TENDERMINT_RPC_URLS));
    println!("Activation {}", json::to_string(&activation_res).unwrap());

    // just call withdraw without sending to check response correctness
    let withdraw_result = block_on(withdraw_v1(
        &mm,
        ATOM_TICKER,
        "cosmos1svaw0aqc4584x825ju7ua03g5xtxwd0ahl86hz",
        "0.1",
    ));
    println!("Withdraw to other {}", json::to_string(&withdraw_result).unwrap());
    let tx_details: TransactionDetails = json::from_value(withdraw_result).unwrap();
    // TODO how to check it if the fee is dynamic?
    /*
    let expected_total: BigDecimal = "0.15".parse().unwrap();
    assert_eq!(tx_details.total_amount, expected_total);
    assert_eq!(tx_details.spent_by_me, expected_total);
    assert_eq!(tx_details.my_balance_change, expected_total * BigDecimal::from(-1));
    */
    assert_eq!(tx_details.received_by_me, BigDecimal::default());
    assert_eq!(tx_details.to, vec![
        "cosmos1svaw0aqc4584x825ju7ua03g5xtxwd0ahl86hz".to_owned()
    ]);
    assert_eq!(tx_details.from, vec![
        "cosmos1w5h6wud7a8zpa539rc99ehgl9gwkad3wjsjq8v".to_owned()
    ]);

    // withdraw and send transaction to ourselves
    let withdraw_result = block_on(withdraw_v1(
        &mm,
        ATOM_TICKER,
        "cosmos1w5h6wud7a8zpa539rc99ehgl9gwkad3wjsjq8v",
        "0.1",
    ));
    println!("Withdraw to self {}", json::to_string(&withdraw_result).unwrap());

    let tx_details: TransactionDetails = json::from_value(withdraw_result).unwrap();
    // TODO how to check it if the fee is dynamic?
    /*
    let expected_total: BigDecimal = "0.15".parse().unwrap();
    let expected_balance_change: BigDecimal = "-0.05".parse().unwrap();
    assert_eq!(tx_details.total_amount, expected_total);
    assert_eq!(tx_details.spent_by_me, expected_total);
    assert_eq!(tx_details.my_balance_change, expected_balance_change);
     */
    let expected_received: BigDecimal = "0.1".parse().unwrap();
    assert_eq!(tx_details.received_by_me, expected_received);

    assert_eq!(tx_details.to, vec![
        "cosmos1w5h6wud7a8zpa539rc99ehgl9gwkad3wjsjq8v".to_owned()
    ]);
    assert_eq!(tx_details.from, vec![
        "cosmos1w5h6wud7a8zpa539rc99ehgl9gwkad3wjsjq8v".to_owned()
    ]);

    let send_raw_tx = block_on(send_raw_transaction(&mm, ATOM_TICKER, &tx_details.tx_hex));
    println!("Send raw tx {}", json::to_string(&send_raw_tx).unwrap());
}

#[test]
fn test_tendermint_token_activation_and_withdraw() {
    const TEST_SEED: &str = "iris test seed";
    let coins = json!([iris_testnet_conf(), iris_nimda_testnet_conf()]);
    let platform_coin = coins[0]["coin"].as_str().unwrap();
    let token = coins[1]["coin"].as_str().unwrap();

    let conf = Mm2TestConf::seednode(TEST_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_res = block_on(enable_tendermint(&mm, platform_coin, &[], &[
        "http://34.80.202.172:26657",
    ]));
    println!("Activation with assets {}", json::to_string(&activation_res).unwrap());

    let activation_res = block_on(enable_tendermint_token(&mm, token));
    println!("Token activation {}", json::to_string(&activation_res).unwrap());

    // just call withdraw without sending to check response correctness
    let withdraw_result = block_on(withdraw_v1(
        &mm,
        token,
        "iaa1llp0f6qxemgh4g4m5ewk0ew0hxj76avuz8kwd5",
        "0.1",
    ));

    println!("Withdraw to other {}", json::to_string(&withdraw_result).unwrap());
    let tx_details: TransactionDetails = json::from_value(withdraw_result).unwrap();

    let expected_total: BigDecimal = "0.1".parse().unwrap();
    assert_eq!(tx_details.total_amount, expected_total);

    // TODO How to check it if the fee is dynamic?
    /*
    let expected_fee: BigDecimal = "0.05".parse().unwrap();
    let actual_fee: BigDecimal = tx_details.fee_details["amount"].as_str().unwrap().parse().unwrap();
    assert_eq!(actual_fee, expected_fee);
    */

    assert_eq!(tx_details.spent_by_me, expected_total);
    assert_eq!(tx_details.my_balance_change, expected_total * BigDecimal::from(-1));
    assert_eq!(tx_details.received_by_me, BigDecimal::default());
    assert_eq!(tx_details.to, vec![
        "iaa1llp0f6qxemgh4g4m5ewk0ew0hxj76avuz8kwd5".to_owned()
    ]);
    assert_eq!(tx_details.from, vec![
        "iaa1e0rx87mdj79zejewuc4jg7ql9ud2286g2us8f2".to_owned()
    ]);

    // withdraw and send transaction to ourselves
    let withdraw_result = block_on(withdraw_v1(
        &mm,
        token,
        "iaa1e0rx87mdj79zejewuc4jg7ql9ud2286g2us8f2",
        "0.1",
    ));
    println!("Withdraw to self {}", json::to_string(&withdraw_result).unwrap());

    let tx_details: TransactionDetails = json::from_value(withdraw_result).unwrap();
    let expected_total: BigDecimal = "0.1".parse().unwrap();
    let expected_received: BigDecimal = "0.1".parse().unwrap();

    assert_eq!(tx_details.total_amount, expected_total);

    // TODO How to check it if the fee is dynamic?
    /*
    let expected_fee: BigDecimal = "0.05".parse().unwrap();
    let actual_fee: BigDecimal = tx_details.fee_details["amount"].as_str().unwrap().parse().unwrap();
    assert_eq!(actual_fee, expected_fee);
    */

    assert_eq!(tx_details.spent_by_me, expected_total);
    assert_eq!(tx_details.received_by_me, expected_received);
    assert_eq!(tx_details.to, vec![
        "iaa1e0rx87mdj79zejewuc4jg7ql9ud2286g2us8f2".to_owned()
    ]);
    assert_eq!(tx_details.from, vec![
        "iaa1e0rx87mdj79zejewuc4jg7ql9ud2286g2us8f2".to_owned()
    ]);

    let send_raw_tx = block_on(send_raw_transaction(&mm, token, &tx_details.tx_hex));
    println!("Send raw tx {}", json::to_string(&send_raw_tx).unwrap());
}
