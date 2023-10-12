use common::block_on;
use crypto::StandardHDCoinAddress;
use mm2_number::BigDecimal;
use mm2_test_helpers::for_tests::{atom_testnet_conf, disable_coin, disable_coin_err, enable_tendermint,
                                  enable_tendermint_token, enable_tendermint_without_balance,
                                  get_tendermint_my_tx_history, ibc_withdraw, iris_nimda_testnet_conf,
                                  iris_testnet_conf, my_balance, send_raw_transaction, withdraw_v1, MarketMakerIt,
                                  Mm2TestConf};
use mm2_test_helpers::structs::{RpcV2Response, TendermintActivationResult, TransactionDetails};
use serde_json::json;

const ATOM_TEST_BALANCE_SEED: &str = "atom test seed";
const ATOM_TEST_WITHDRAW_SEED: &str = "atom test withdraw seed";
const ATOM_TICKER: &str = "ATOM";
const ATOM_TENDERMINT_RPC_URLS: &[&str] = &["https://rpc.sentry-02.theta-testnet.polypore.xyz"];

const IRIS_TEST_SEED: &str = "iris test seed";
const IRIS_TESTNET_RPC_URLS: &[&str] = &["http://34.80.202.172:26657"];

const TENDERMINT_TEST_BIP39_SEED: &str =
    "emerge canoe salmon dolphin glow priority random become gasp sell blade argue";

#[test]
fn test_tendermint_activation_and_balance() {
    let coins = json!([atom_testnet_conf()]);
    let expected_address = "cosmos1svaw0aqc4584x825ju7ua03g5xtxwd0ahl86hz";

    let conf = Mm2TestConf::seednode(ATOM_TEST_BALANCE_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_result = block_on(enable_tendermint(
        &mm,
        ATOM_TICKER,
        &[],
        ATOM_TENDERMINT_RPC_URLS,
        false,
    ));

    let result: RpcV2Response<TendermintActivationResult> = serde_json::from_value(activation_result).unwrap();
    assert_eq!(result.result.address, expected_address);
    let expected_balance: BigDecimal = "0.575457".parse().unwrap();
    assert_eq!(result.result.balance.unwrap().spendable, expected_balance);

    let my_balance = block_on(my_balance(&mm, ATOM_TICKER));
    assert_eq!(my_balance.balance, expected_balance);
    assert_eq!(my_balance.unspendable_balance, BigDecimal::default());
    assert_eq!(my_balance.address, expected_address);
    assert_eq!(my_balance.coin, ATOM_TICKER);
}

#[test]
fn test_tendermint_activation_without_balance() {
    let coins = json!([atom_testnet_conf()]);

    let conf = Mm2TestConf::seednode(ATOM_TEST_BALANCE_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_result = block_on(enable_tendermint_without_balance(
        &mm,
        ATOM_TICKER,
        &[],
        ATOM_TENDERMINT_RPC_URLS,
        false,
    ));

    let result: RpcV2Response<TendermintActivationResult> = serde_json::from_value(activation_result).unwrap();

    assert!(result.result.balance.is_none());
    assert!(result.result.tokens_balances.is_none());
    assert!(result.result.tokens_tickers.unwrap().is_empty());
}

#[test]
fn test_tendermint_hd_address() {
    let coins = json!([atom_testnet_conf()]);
    // Default address m/44'/118'/0'/0/0 when no path_to_address is specified in activation request
    let expected_address = "cosmos1nv4mqaky7n7rqjhch7829kgypx5s8fh62wdtr8";

    let conf = Mm2TestConf::seednode_with_hd_account(TENDERMINT_TEST_BIP39_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_result = block_on(enable_tendermint(
        &mm,
        ATOM_TICKER,
        &[],
        ATOM_TENDERMINT_RPC_URLS,
        false,
    ));

    let result: RpcV2Response<TendermintActivationResult> = serde_json::from_value(activation_result).unwrap();
    assert_eq!(result.result.address, expected_address);
}

#[test]
fn test_tendermint_withdraw() {
    let coins = json!([atom_testnet_conf()]);

    let conf = Mm2TestConf::seednode(ATOM_TEST_WITHDRAW_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_res = block_on(enable_tendermint(
        &mm,
        ATOM_TICKER,
        &[],
        ATOM_TENDERMINT_RPC_URLS,
        false,
    ));
    println!("Activation {}", serde_json::to_string(&activation_res).unwrap());

    // just call withdraw without sending to check response correctness
    let tx_details = block_on(withdraw_v1(
        &mm,
        ATOM_TICKER,
        "cosmos1svaw0aqc4584x825ju7ua03g5xtxwd0ahl86hz",
        "0.1",
        None,
    ));
    println!("Withdraw to other {}", serde_json::to_string(&tx_details).unwrap());
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
    let tx_details = block_on(withdraw_v1(
        &mm,
        ATOM_TICKER,
        "cosmos1w5h6wud7a8zpa539rc99ehgl9gwkad3wjsjq8v",
        "0.1",
        None,
    ));
    println!("Withdraw to self {}", serde_json::to_string(&tx_details).unwrap());

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

    let tx_details = block_on(withdraw_v1(
        &mm,
        ATOM_TICKER,
        "cosmos1w5h6wud7a8zpa539rc99ehgl9gwkad3wjsjq8v",
        "0.1",
        None,
    ));
    let send_raw_tx = block_on(send_raw_transaction(&mm, ATOM_TICKER, &tx_details.tx_hex));
    println!("Send raw tx {}", serde_json::to_string(&send_raw_tx).unwrap());
}

#[test]
fn test_tendermint_withdraw_hd() {
    let coins = json!([iris_testnet_conf()]);
    let coin = coins[0]["coin"].as_str().unwrap();

    let conf = Mm2TestConf::seednode_with_hd_account(TENDERMINT_TEST_BIP39_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_res = block_on(enable_tendermint(&mm, coin, &[], IRIS_TESTNET_RPC_URLS, false));
    println!(
        "Activation with assets {}",
        serde_json::to_string(&activation_res).unwrap()
    );

    // We will withdraw from HD account 0 and change 0 and address_index 1
    let path_to_address = StandardHDCoinAddress {
        account: 0,
        is_change: false,
        address_index: 1,
    };

    // just call withdraw without sending to check response correctness
    let tx_details = block_on(withdraw_v1(
        &mm,
        coin,
        "iaa1llp0f6qxemgh4g4m5ewk0ew0hxj76avuz8kwd5",
        "0.1",
        Some(path_to_address.clone()),
    ));
    println!("Withdraw to other {}", serde_json::to_string(&tx_details).unwrap());
    // TODO how to check it if the fee is dynamic?
    /*
    let expected_total: BigDecimal = "0.15".parse().unwrap();
    assert_eq!(tx_details.total_amount, expected_total);
    assert_eq!(tx_details.spent_by_me, expected_total);
    assert_eq!(tx_details.my_balance_change, expected_total * BigDecimal::from(-1));
    */
    assert_eq!(tx_details.received_by_me, BigDecimal::default());
    assert_eq!(tx_details.to, vec![
        "iaa1llp0f6qxemgh4g4m5ewk0ew0hxj76avuz8kwd5".to_owned()
    ]);
    assert_eq!(tx_details.from, vec![
        "iaa1tpd0um0r3z0y88p3gkv3y38dq8lmqc2xs9u0pv".to_owned()
    ]);

    // withdraw and send transaction to ourselves
    let tx_details = block_on(withdraw_v1(
        &mm,
        coin,
        "iaa1tpd0um0r3z0y88p3gkv3y38dq8lmqc2xs9u0pv",
        "0.1",
        Some(path_to_address.clone()),
    ));
    println!("Withdraw to self {}", serde_json::to_string(&tx_details).unwrap());

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
        "iaa1tpd0um0r3z0y88p3gkv3y38dq8lmqc2xs9u0pv".to_owned()
    ]);
    assert_eq!(tx_details.from, vec![
        "iaa1tpd0um0r3z0y88p3gkv3y38dq8lmqc2xs9u0pv".to_owned()
    ]);

    let tx_details = block_on(withdraw_v1(
        &mm,
        coin,
        "iaa1tpd0um0r3z0y88p3gkv3y38dq8lmqc2xs9u0pv",
        "0.1",
        Some(path_to_address),
    ));
    let send_raw_tx = block_on(send_raw_transaction(&mm, coin, &tx_details.tx_hex));
    println!("Send raw tx {}", serde_json::to_string(&send_raw_tx).unwrap());
}

#[test]
fn test_custom_gas_limit_on_tendermint_withdraw() {
    let coins = json!([atom_testnet_conf()]);

    let conf = Mm2TestConf::seednode(ATOM_TEST_WITHDRAW_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_res = block_on(enable_tendermint(
        &mm,
        ATOM_TICKER,
        &[],
        ATOM_TENDERMINT_RPC_URLS,
        false,
    ));
    println!("Activation {}", serde_json::to_string(&activation_res).unwrap());

    let request = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": ATOM_TICKER,
        "to": "cosmos1svaw0aqc4584x825ju7ua03g5xtxwd0ahl86hz",
        "amount": "0.1",
        "fee": {
            "type": "CosmosGas",
            "gas_limit": 150000,
            "gas_price": 0.1
        }
    })))
    .unwrap();
    assert_eq!(request.0, common::StatusCode::OK, "'withdraw' failed: {}", request.1);
    let tx_details: TransactionDetails = serde_json::from_str(&request.1).unwrap();

    assert_eq!(tx_details.fee_details["gas_limit"], 150000);
}

#[test]
fn test_tendermint_ibc_withdraw() {
    // visit `{rpc_url}/ibc/core/channel/v1/channels?pagination.limit=10000` to see the full list of ibc channels
    const IBC_SOURCE_CHANNEL: &str = "channel-93";

    const IBC_TARGET_ADDRESS: &str = "cosmos1r5v5srda7xfth3hn2s26txvrcrntldjumt8mhl";
    const MY_ADDRESS: &str = "iaa1e0rx87mdj79zejewuc4jg7ql9ud2286g2us8f2";

    let coins = json!([iris_testnet_conf(), iris_nimda_testnet_conf()]);
    let platform_coin = coins[0]["coin"].as_str().unwrap();
    let token = coins[1]["coin"].as_str().unwrap();

    let conf = Mm2TestConf::seednode(IRIS_TEST_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_res = block_on(enable_tendermint(&mm, platform_coin, &[], IRIS_TESTNET_RPC_URLS, false));
    println!(
        "Activation with assets {}",
        serde_json::to_string(&activation_res).unwrap()
    );

    let activation_res = block_on(enable_tendermint_token(&mm, token));
    println!("Token activation {}", serde_json::to_string(&activation_res).unwrap());

    let tx_details = block_on(ibc_withdraw(
        &mm,
        IBC_SOURCE_CHANNEL,
        token,
        IBC_TARGET_ADDRESS,
        "0.1",
        None,
    ));
    println!(
        "IBC transfer to atom address {}",
        serde_json::to_string(&tx_details).unwrap()
    );

    let expected_spent: BigDecimal = "0.1".parse().unwrap();
    assert_eq!(tx_details.spent_by_me, expected_spent);

    assert_eq!(tx_details.to, vec![IBC_TARGET_ADDRESS.to_owned()]);
    assert_eq!(tx_details.from, vec![MY_ADDRESS.to_owned()]);

    let tx_details = block_on(ibc_withdraw(
        &mm,
        IBC_SOURCE_CHANNEL,
        token,
        IBC_TARGET_ADDRESS,
        "0.1",
        None,
    ));
    let send_raw_tx = block_on(send_raw_transaction(&mm, token, &tx_details.tx_hex));
    println!("Send raw tx {}", serde_json::to_string(&send_raw_tx).unwrap());
}
#[test]
fn test_tendermint_ibc_withdraw_hd() {
    // visit `{rpc_url}/ibc/core/channel/v1/channels?pagination.limit=10000` to see the full list of ibc channels
    const IBC_SOURCE_CHANNEL: &str = "channel-93";

    const IBC_TARGET_ADDRESS: &str = "cosmos1r5v5srda7xfth3hn2s26txvrcrntldjumt8mhl";
    const MY_ADDRESS: &str = "iaa1tpd0um0r3z0y88p3gkv3y38dq8lmqc2xs9u0pv";

    let coins = json!([iris_testnet_conf()]);
    let coin = coins[0]["coin"].as_str().unwrap();

    let conf = Mm2TestConf::seednode_with_hd_account(TENDERMINT_TEST_BIP39_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_res = block_on(enable_tendermint(&mm, coin, &[], IRIS_TESTNET_RPC_URLS, false));
    println!(
        "Activation with assets {}",
        serde_json::to_string(&activation_res).unwrap()
    );

    // We will withdraw from HD account 0 and change 0 and address_index 1
    let path_to_address = StandardHDCoinAddress {
        account: 0,
        is_change: false,
        address_index: 1,
    };

    let tx_details = block_on(ibc_withdraw(
        &mm,
        IBC_SOURCE_CHANNEL,
        coin,
        IBC_TARGET_ADDRESS,
        "0.1",
        Some(path_to_address.clone()),
    ));
    println!(
        "IBC transfer to atom address {}",
        serde_json::to_string(&tx_details).unwrap()
    );

    assert_eq!(tx_details.to, vec![IBC_TARGET_ADDRESS.to_owned()]);
    assert_eq!(tx_details.from, vec![MY_ADDRESS.to_owned()]);

    let tx_details = block_on(ibc_withdraw(
        &mm,
        IBC_SOURCE_CHANNEL,
        coin,
        IBC_TARGET_ADDRESS,
        "0.1",
        Some(path_to_address),
    ));
    let send_raw_tx = block_on(send_raw_transaction(&mm, coin, &tx_details.tx_hex));
    println!("Send raw tx {}", serde_json::to_string(&send_raw_tx).unwrap());
}

#[test]
fn test_tendermint_token_activation_and_withdraw() {
    let coins = json!([iris_testnet_conf(), iris_nimda_testnet_conf()]);
    let platform_coin = coins[0]["coin"].as_str().unwrap();
    let token = coins[1]["coin"].as_str().unwrap();

    let conf = Mm2TestConf::seednode(IRIS_TEST_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    let activation_res = block_on(enable_tendermint(&mm, platform_coin, &[], IRIS_TESTNET_RPC_URLS, false));
    println!(
        "Activation with assets {}",
        serde_json::to_string(&activation_res).unwrap()
    );

    let activation_res = block_on(enable_tendermint_token(&mm, token));
    println!("Token activation {}", serde_json::to_string(&activation_res).unwrap());

    // just call withdraw without sending to check response correctness
    let tx_details = block_on(withdraw_v1(
        &mm,
        token,
        "iaa1llp0f6qxemgh4g4m5ewk0ew0hxj76avuz8kwd5",
        "0.1",
        None,
    ));

    println!("Withdraw to other {}", serde_json::to_string(&tx_details).unwrap());

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
    let tx_details = block_on(withdraw_v1(
        &mm,
        token,
        "iaa1e0rx87mdj79zejewuc4jg7ql9ud2286g2us8f2",
        "0.1",
        None,
    ));
    println!("Withdraw to self {}", serde_json::to_string(&tx_details).unwrap());

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

    let tx_details = block_on(withdraw_v1(
        &mm,
        token,
        "iaa1e0rx87mdj79zejewuc4jg7ql9ud2286g2us8f2",
        "0.1",
        None,
    ));
    let send_raw_tx = block_on(send_raw_transaction(&mm, token, &tx_details.tx_hex));
    println!("Send raw tx {}", serde_json::to_string(&send_raw_tx).unwrap());
}

#[test]
fn test_tendermint_tx_history() {
    const TEST_SEED: &str = "Vdo8Xt8pTAetRlMq3kV0LzE393eVYbPSn5Mhtw4p";
    const TX_FINISHED_LOG: &str = "Tx history fetching finished for IRIS-TEST.";
    const TX_HISTORY_PAGE_LIMIT: usize = 50;
    const IRIS_TEST_EXPECTED_TX_COUNT: u64 = 16;
    const IRIS_NIMDA_EXPECTED_TX_COUNT: u64 = 10;

    let iris_test_constant_history_txs = include_str!("../../../mm2_test_helpers/dummy_files/iris_test_history.json");
    let iris_test_constant_history_txs: Vec<TransactionDetails> =
        serde_json::from_str(iris_test_constant_history_txs).unwrap();

    let iris_nimda_constant_history_txs = include_str!("../../../mm2_test_helpers/dummy_files/iris_nimda_history.json");
    let iris_nimda_constant_history_txs: Vec<TransactionDetails> =
        serde_json::from_str(iris_nimda_constant_history_txs).unwrap();

    let coins = json!([iris_testnet_conf(), iris_nimda_testnet_conf()]);
    let platform_coin = coins[0]["coin"].as_str().unwrap();
    let token = coins[1]["coin"].as_str().unwrap();

    let conf = Mm2TestConf::seednode(TEST_SEED, &coins);
    let mut mm = block_on(MarketMakerIt::start_async(conf.conf, conf.rpc_password, None)).unwrap();

    block_on(enable_tendermint(
        &mm,
        platform_coin,
        &[token],
        IRIS_TESTNET_RPC_URLS,
        true,
    ));

    if block_on(mm.wait_for_log(60., |log| log.contains(TX_FINISHED_LOG))).is_err() {
        println!("{}", mm.log_as_utf8().unwrap());
        panic!("Tx history didn't finish which is not expected");
    }

    // testing IRIS-TEST history
    let iris_tx_history_response = block_on(get_tendermint_my_tx_history(
        &mm,
        platform_coin,
        TX_HISTORY_PAGE_LIMIT,
        1,
    ));
    let total_txs = iris_tx_history_response["result"]["total"].as_u64().unwrap();
    assert_eq!(total_txs, IRIS_TEST_EXPECTED_TX_COUNT);

    let mut iris_txs_from_request = iris_tx_history_response["result"]["transactions"].clone();
    for i in 0..IRIS_TEST_EXPECTED_TX_COUNT {
        iris_txs_from_request[i as usize]
            .as_object_mut()
            .unwrap()
            .remove("confirmations");
    }
    let iris_txs_from_request: Vec<TransactionDetails> = serde_json::from_value(iris_txs_from_request).unwrap();
    assert_eq!(iris_test_constant_history_txs, iris_txs_from_request);

    // testing IRIS-NIMDA history
    let nimda_tx_history_response = block_on(get_tendermint_my_tx_history(&mm, token, TX_HISTORY_PAGE_LIMIT, 1));
    let total_txs = nimda_tx_history_response["result"]["total"].as_u64().unwrap();
    assert_eq!(total_txs, IRIS_NIMDA_EXPECTED_TX_COUNT);

    let mut nimda_txs_from_request = nimda_tx_history_response["result"]["transactions"].clone();
    for i in 0..IRIS_NIMDA_EXPECTED_TX_COUNT {
        nimda_txs_from_request[i as usize]
            .as_object_mut()
            .unwrap()
            .remove("confirmations");
    }
    let nimda_txs_from_request: Vec<TransactionDetails> = serde_json::from_value(nimda_txs_from_request).unwrap();

    assert_eq!(iris_nimda_constant_history_txs, nimda_txs_from_request);

    block_on(mm.stop()).unwrap();
}

#[test]
fn test_disable_tendermint_platform_coin_with_token() {
    const TEST_SEED: &str = "iris test seed";
    let coins = json!([iris_testnet_conf(), iris_nimda_testnet_conf()]);
    let platform_coin = coins[0]["coin"].as_str().unwrap();
    let token = coins[1]["coin"].as_str().unwrap();

    let conf = Mm2TestConf::seednode(TEST_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();
    // Enable platform coin IRIS-TEST
    let activation_res = block_on(enable_tendermint(&mm, platform_coin, &[], IRIS_TESTNET_RPC_URLS, false));
    assert!(&activation_res.get("result").unwrap().get("address").is_some());

    // Enable platform coin token IRIS-NIMDA
    let activation_res = block_on(enable_tendermint_token(&mm, token));
    assert!(&activation_res.get("result").unwrap().get("balances").is_some());

    // Try to passive platform coin, IRIS-TEST.
    let res = block_on(disable_coin(&mm, "IRIS-TEST", false));
    assert!(res.passivized);

    // Try to disable IRIS-NIMDA token when platform coin is passived.
    // This should work, because platform coin is still in the memory.
    let res = block_on(disable_coin(&mm, "IRIS-NIMDA", false));
    assert!(!res.passivized);

    // Then try to force disable IRIS-TEST platform coin.
    let res = block_on(disable_coin(&mm, "IRIS-TEST", true));
    assert!(!res.passivized);
}

#[test]
fn test_passive_coin_and_force_disable() {
    const TEST_SEED: &str = "iris test seed";
    let coins = json!([iris_testnet_conf(), iris_nimda_testnet_conf()]);
    let platform_coin = coins[0]["coin"].as_str().unwrap();
    let token = coins[1]["coin"].as_str().unwrap();

    let conf = Mm2TestConf::seednode(TEST_SEED, &coins);
    let mm = MarketMakerIt::start(conf.conf, conf.rpc_password, None).unwrap();

    // Enable platform coin IRIS-TEST
    let activation_res = block_on(enable_tendermint(&mm, platform_coin, &[], IRIS_TESTNET_RPC_URLS, false));
    assert!(&activation_res.get("result").unwrap().get("address").is_some());

    // Enable platform coin token IRIS-NIMDA
    let activation_res = block_on(enable_tendermint_token(&mm, token));
    assert!(&activation_res.get("result").unwrap().get("balances").is_some());

    // Try to passive platform coin, IRIS-TEST.
    let res = block_on(disable_coin(&mm, "IRIS-TEST", false));
    assert!(res.passivized);

    // Try to disable IRIS-NIMDA token when platform coin is passived.
    // This should work, because platform coin is still in the memory.
    let res = block_on(disable_coin(&mm, "IRIS-NIMDA", false));
    assert!(!res.passivized);

    // Re-activate passive coin
    let activation_res = block_on(enable_tendermint(&mm, platform_coin, &[], IRIS_TESTNET_RPC_URLS, false));
    assert!(&activation_res.get("result").unwrap().get("address").is_some());

    // Enable platform coin token IRIS-NIMDA
    let activation_res = block_on(enable_tendermint_token(&mm, token));
    assert!(&activation_res.get("result").unwrap().get("balances").is_some());

    // Try to force disable platform coin, IRIS-TEST.
    let res = block_on(disable_coin(&mm, "IRIS-TEST", true));
    assert!(!res.passivized);

    // Try to disable IRIS-NIMDA token when platform coin force disabled.
    // This should failed, because platform coin was purged with it's tokens.
    block_on(disable_coin_err(&mm, "IRIS-NIMDA", false));
}

mod swap {
    use std::convert::TryFrom;
    use std::{env, thread};

    use common::executor::Timer;
    use common::log;
    use instant::Duration;
    use mm2_rpc::data::legacy::OrderbookResponse;
    use mm2_test_helpers::for_tests::{check_my_swap_status, check_recent_swaps, check_stats_swap_status,
                                      enable_eth_coin, rick_conf, tbnb_conf, usdc_ibc_iris_testnet_conf,
                                      RICK_ELECTRUM_ADDRS};

    use super::*;
    use crate::integration_tests_common::enable_electrum;

    const BOB_PASSPHRASE: &str = "iris test seed";
    const ALICE_PASSPHRASE: &str = "iris test2 seed";

    // https://academy.binance.com/en/articles/connecting-metamask-to-binance-smart-chain
    const TBNB_URLS: &[&str] = &["https://data-seed-prebsc-1-s1.binance.org:8545/"];
    // https://testnet.bscscan.com/address/0xb1ad803ea4f57401639c123000c75f5b66e4d123
    const TBNB_SWAP_CONTRACT: &str = "0xB1Ad803ea4F57401639c123000C75F5B66E4D123";

    #[test]
    fn swap_usdc_ibc_with_nimda() {
        let bob_passphrase = String::from(BOB_PASSPHRASE);
        let alice_passphrase = String::from(ALICE_PASSPHRASE);

        let coins = json!([
            usdc_ibc_iris_testnet_conf(),
            iris_nimda_testnet_conf(),
            iris_testnet_conf(),
        ]);

        let mm_bob = MarketMakerIt::start(
            json!({
                "gui": "nogui",
                "netid": 8999,
                "dht": "on",
                "myipaddr": env::var("BOB_TRADE_IP") .ok(),
                "rpcip": env::var("BOB_TRADE_IP") .ok(),
                "canbind": env::var("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
                "passphrase": bob_passphrase,
                "coins": coins,
                "rpc_password": "password",
                "i_am_seed": true,
            }),
            "password".into(),
            None,
        )
        .unwrap();

        thread::sleep(Duration::from_secs(1));

        let mm_alice = MarketMakerIt::start(
            json!({
                "gui": "nogui",
                "netid": 8999,
                "dht": "on",
                "myipaddr": env::var("ALICE_TRADE_IP") .ok(),
                "rpcip": env::var("ALICE_TRADE_IP") .ok(),
                "passphrase": alice_passphrase,
                "coins": coins,
                "seednodes": [mm_bob.my_seed_addr()],
                "rpc_password": "password",
                "skip_startup_checks": true,
            }),
            "password".into(),
            None,
        )
        .unwrap();

        thread::sleep(Duration::from_secs(1));

        dbg!(block_on(enable_tendermint(
            &mm_bob,
            "IRIS-TEST",
            &["IRIS-NIMDA", "USDC-IBC-IRIS"],
            &["http://34.80.202.172:26657"],
            false
        )));

        dbg!(block_on(enable_tendermint(
            &mm_alice,
            "IRIS-TEST",
            &["IRIS-NIMDA", "USDC-IBC-IRIS"],
            &["http://34.80.202.172:26657"],
            false
        )));

        block_on(trade_base_rel_tendermint(
            mm_bob,
            mm_alice,
            "USDC-IBC-IRIS",
            "IRIS-NIMDA",
            1,
            2,
            0.008,
        ));
    }

    #[test]
    fn swap_iris_with_rick() {
        let bob_passphrase = String::from(BOB_PASSPHRASE);
        let alice_passphrase = String::from(ALICE_PASSPHRASE);

        let coins = json!([iris_testnet_conf(), rick_conf()]);

        let mm_bob = MarketMakerIt::start(
            json!({
                "gui": "nogui",
                "netid": 8999,
                "dht": "on",
                "myipaddr": env::var("BOB_TRADE_IP") .ok(),
                "rpcip": env::var("BOB_TRADE_IP") .ok(),
                "canbind": env::var("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
                "passphrase": bob_passphrase,
                "coins": coins,
                "rpc_password": "password",
                "i_am_seed": true,
            }),
            "password".into(),
            None,
        )
        .unwrap();

        thread::sleep(Duration::from_secs(1));

        let mm_alice = MarketMakerIt::start(
            json!({
                "gui": "nogui",
                "netid": 8999,
                "dht": "on",
                "myipaddr": env::var("ALICE_TRADE_IP") .ok(),
                "rpcip": env::var("ALICE_TRADE_IP") .ok(),
                "passphrase": alice_passphrase,
                "coins": coins,
                "seednodes": [mm_bob.my_seed_addr()],
                "rpc_password": "password",
                "skip_startup_checks": true,
            }),
            "password".into(),
            None,
        )
        .unwrap();

        thread::sleep(Duration::from_secs(1));

        dbg!(block_on(enable_tendermint(
            &mm_bob,
            "IRIS-TEST",
            &[],
            &["http://34.80.202.172:26657"],
            false
        )));

        dbg!(block_on(enable_tendermint(
            &mm_alice,
            "IRIS-TEST",
            &[],
            &["http://34.80.202.172:26657"],
            false
        )));

        dbg!(block_on(enable_electrum(
            &mm_bob,
            "RICK",
            false,
            RICK_ELECTRUM_ADDRS,
            None
        )));

        dbg!(block_on(enable_electrum(
            &mm_alice,
            "RICK",
            false,
            RICK_ELECTRUM_ADDRS,
            None
        )));

        block_on(trade_base_rel_tendermint(
            mm_bob,
            mm_alice,
            "IRIS-TEST",
            "RICK",
            1,
            2,
            0.008,
        ));
    }

    #[test]
    #[ignore] // having fund problems with tBNB
    fn swap_iris_with_tbnb() {
        let bob_passphrase = String::from(BOB_PASSPHRASE);
        let alice_passphrase = String::from(ALICE_PASSPHRASE);

        let coins = json!([iris_testnet_conf(), tbnb_conf()]);

        let mm_bob = MarketMakerIt::start(
            json!({
                "gui": "nogui",
                "netid": 8999,
                "dht": "on",
                "myipaddr": env::var("BOB_TRADE_IP") .ok(),
                "rpcip": env::var("BOB_TRADE_IP") .ok(),
                "canbind": env::var("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
                "passphrase": bob_passphrase,
                "coins": coins,
                "rpc_password": "password",
                "i_am_seed": true,
            }),
            "password".into(),
            None,
        )
        .unwrap();

        thread::sleep(Duration::from_secs(1));

        let mm_alice = MarketMakerIt::start(
            json!({
                "gui": "nogui",
                "netid": 8999,
                "dht": "on",
                "myipaddr": env::var("ALICE_TRADE_IP") .ok(),
                "rpcip": env::var("ALICE_TRADE_IP") .ok(),
                "passphrase": alice_passphrase,
                "coins": coins,
                "seednodes": [mm_bob.my_seed_addr()],
                "rpc_password": "password",
                "skip_startup_checks": true,
            }),
            "password".into(),
            None,
        )
        .unwrap();

        thread::sleep(Duration::from_secs(1));

        dbg!(block_on(enable_tendermint(
            &mm_bob,
            "IRIS-TEST",
            &[],
            &["http://34.80.202.172:26657"],
            false
        )));

        dbg!(block_on(enable_tendermint(
            &mm_alice,
            "IRIS-TEST",
            &[],
            &["http://34.80.202.172:26657"],
            false
        )));

        dbg!(block_on(enable_eth_coin(
            &mm_bob,
            "tBNB",
            TBNB_URLS,
            TBNB_SWAP_CONTRACT,
            None,
            false
        )));

        dbg!(block_on(enable_eth_coin(
            &mm_alice,
            "tBNB",
            TBNB_URLS,
            TBNB_SWAP_CONTRACT,
            None,
            false
        )));

        block_on(trade_base_rel_tendermint(
            mm_bob,
            mm_alice,
            "IRIS-TEST",
            "tBNB",
            1,
            2,
            0.008,
        ));
    }

    pub async fn trade_base_rel_tendermint(
        mut mm_bob: MarketMakerIt,
        mut mm_alice: MarketMakerIt,
        base: &str,
        rel: &str,
        maker_price: i32,
        taker_price: i32,
        volume: f64,
    ) {
        log!("Issue bob {}/{} sell request", base, rel);
        let rc = mm_bob
            .rpc(&json!({
                "userpass": mm_bob.userpass,
                "method": "setprice",
                "base": base,
                "rel": rel,
                "price": maker_price,
                "volume": volume
            }))
            .await
            .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let mut uuids = vec![];

        common::log::info!(
            "Trigger alice subscription to {}/{} orderbook topic first and sleep for 1 second",
            base,
            rel
        );
        let rc = mm_alice
            .rpc(&json!({
                "userpass": mm_alice.userpass,
                "method": "orderbook",
                "base": base,
                "rel": rel,
            }))
            .await
            .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);
        Timer::sleep(1.).await;
        common::log::info!("Issue alice {}/{} buy request", base, rel);
        let rc = mm_alice
            .rpc(&json!({
                "userpass": mm_alice.userpass,
                "method": "buy",
                "base": base,
                "rel": rel,
                "volume": volume,
                "price": taker_price
            }))
            .await
            .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);
        let buy_json: serde_json::Value = serde_json::from_str(&rc.1).unwrap();
        uuids.push(buy_json["result"]["uuid"].as_str().unwrap().to_owned());

        // ensure the swaps are started
        let expected_log = format!("Entering the taker_swap_loop {}/{}", base, rel);
        mm_alice
            .wait_for_log(5., |log| log.contains(&expected_log))
            .await
            .unwrap();
        let expected_log = format!("Entering the maker_swap_loop {}/{}", base, rel);
        mm_bob
            .wait_for_log(5., |log| log.contains(&expected_log))
            .await
            .unwrap();

        for uuid in uuids.iter() {
            // ensure the swaps are indexed to the SQLite database
            let expected_log = format!("Inserting new swap {} to the SQLite database", uuid);
            mm_alice
                .wait_for_log(5., |log| log.contains(&expected_log))
                .await
                .unwrap();
            mm_bob
                .wait_for_log(5., |log| log.contains(&expected_log))
                .await
                .unwrap()
        }

        for uuid in uuids.iter() {
            match mm_bob
                .wait_for_log(900., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))
                .await
            {
                Ok(_) => (),
                Err(_) => {
                    println!("{}", mm_bob.log_as_utf8().unwrap());
                },
            }

            match mm_alice
                .wait_for_log(900., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))
                .await
            {
                Ok(_) => (),
                Err(_) => {
                    println!("{}", mm_alice.log_as_utf8().unwrap());
                },
            }

            log!("Waiting a few second for the fresh swap status to be saved..");
            Timer::sleep(5.).await;

            println!("{}", mm_alice.log_as_utf8().unwrap());
            log!("Checking alice/taker status..");
            check_my_swap_status(
                &mm_alice,
                uuid,
                BigDecimal::try_from(volume).unwrap(),
                BigDecimal::try_from(volume).unwrap(),
            )
            .await;

            println!("{}", mm_bob.log_as_utf8().unwrap());
            log!("Checking bob/maker status..");
            check_my_swap_status(
                &mm_bob,
                uuid,
                BigDecimal::try_from(volume).unwrap(),
                BigDecimal::try_from(volume).unwrap(),
            )
            .await;
        }

        log!("Waiting 3 seconds for nodes to broadcast their swaps data..");
        Timer::sleep(3.).await;

        for uuid in uuids.iter() {
            log!("Checking alice status..");
            check_stats_swap_status(&mm_alice, uuid).await;

            log!("Checking bob status..");
            check_stats_swap_status(&mm_bob, uuid).await;
        }

        log!("Checking alice recent swaps..");
        check_recent_swaps(&mm_alice, uuids.len()).await;
        log!("Checking bob recent swaps..");
        check_recent_swaps(&mm_bob, uuids.len()).await;
        log!("Get {}/{} orderbook", base, rel);
        let rc = mm_bob
            .rpc(&json!({
                "userpass": mm_bob.userpass,
                "method": "orderbook",
                "base": base,
                "rel": rel,
            }))
            .await
            .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: OrderbookResponse = serde_json::from_str(&rc.1).unwrap();
        log!("{}/{} orderbook {:?}", base, rel, bob_orderbook);

        assert_eq!(0, bob_orderbook.bids.len(), "{} {} bids must be empty", base, rel);
        assert_eq!(0, bob_orderbook.asks.len(), "{} {} asks must be empty", base, rel);

        mm_bob.stop().await.unwrap();
        mm_alice.stop().await.unwrap();
    }
}
