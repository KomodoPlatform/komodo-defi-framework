use crate::docker_tests::docker_tests_common::*;
use bigdecimal::BigDecimal;
use common::for_tests::{enable_bch_with_tokens, enable_slp, UtxoRpcMode};
use serde_json::{self as json};
use std::time::Duration;

#[test]
fn trade_test_with_maker_slp() { trade_base_rel(("ADEXSLP", "FORSLP")); }

#[test]
fn trade_test_with_taker_slp() { trade_base_rel(("FORSLP", "ADEXSLP")); }

#[test]
fn test_bch_and_slp_balance() {
    // MM2 should mark the SLP-related and other UTXOs as unspendable BCH balance
    let mm = slp_supplied_node();

    let enable_bch = block_on(enable_native_bch(&mm, "FORSLP", &[]));
    let enable_bch: EnableElectrumResponse = json::from_value(enable_bch).unwrap();

    let expected_spendable = BigDecimal::from(1000);
    assert_eq!(expected_spendable, enable_bch.balance);

    let expected_unspendable: BigDecimal = "0.00001".parse().unwrap();
    assert_eq!(expected_unspendable, enable_bch.unspendable_balance);

    let bch_balance = get_balance(&mm, "FORSLP");

    assert_eq!(expected_spendable, bch_balance.balance);
    assert_eq!(expected_unspendable, bch_balance.unspendable_balance);

    let enable_slp = block_on(enable_native(&mm, "ADEXSLP", &[]));
    let enable_slp: EnableElectrumResponse = json::from_value(enable_slp).unwrap();

    let expected_spendable = BigDecimal::from(1000);
    assert_eq!(expected_spendable, enable_slp.balance);

    let expected_unspendable: BigDecimal = 0.into();
    assert_eq!(expected_unspendable, enable_slp.unspendable_balance);

    let slp_balance = get_balance(&mm, "ADEXSLP");

    assert_eq!(expected_spendable, slp_balance.balance);
    assert_eq!(expected_unspendable, slp_balance.unspendable_balance);
}

#[test]
fn test_bch_and_slp_balance_enable_slp_v2() {
    // MM2 should mark the SLP-related and other UTXOs as unspendable BCH balance
    let mm = slp_supplied_node();

    let enable_bch = block_on(enable_native_bch(&mm, "FORSLP", &[]));
    let enable_bch: EnableElectrumResponse = json::from_value(enable_bch).unwrap();

    let expected_spendable = BigDecimal::from(1000);
    assert_eq!(expected_spendable, enable_bch.balance);

    let expected_unspendable: BigDecimal = "0.00001".parse().unwrap();
    assert_eq!(expected_unspendable, enable_bch.unspendable_balance);

    let bch_balance = get_balance(&mm, "FORSLP");

    assert_eq!(expected_spendable, bch_balance.balance);
    assert_eq!(expected_unspendable, bch_balance.unspendable_balance);

    let enable_slp = block_on(enable_slp(&mm, "ADEXSLP"));
    let enable_slp: RpcV2Response<EnableSlpResponse> = json::from_value(enable_slp).unwrap();
    assert_eq!(1, enable_slp.result.balances.len());

    let (_, balance) = enable_slp.result.balances.into_iter().next().unwrap();

    let expected_spendable = BigDecimal::from(1000);
    assert_eq!(expected_spendable, balance.spendable);

    let expected_unspendable: BigDecimal = 0.into();
    assert_eq!(expected_unspendable, balance.unspendable);

    let slp_balance = get_balance(&mm, "ADEXSLP");

    assert_eq!(expected_spendable, slp_balance.balance);
    assert_eq!(expected_unspendable, slp_balance.unspendable_balance);
}

#[test]
fn test_bch_and_slp_balance_enable_bch_with_tokens_v2() {
    let mm = slp_supplied_node();

    let tx_history = false;
    let enable_bch_with_tokens = block_on(enable_bch_with_tokens(
        &mm,
        "FORSLP",
        &["ADEXSLP"],
        UtxoRpcMode::Native,
        tx_history,
    ));
    let enable_bch_with_tokens: RpcV2Response<EnableBchWithTokensResponse> =
        json::from_value(enable_bch_with_tokens).unwrap();

    let expected_bch_spendable = BigDecimal::from(1000);
    let expected_bch_unspendable: BigDecimal = "0.00001".parse().unwrap();

    let (_, bch_balance) = enable_bch_with_tokens
        .result
        .bch_addresses_infos
        .into_iter()
        .next()
        .unwrap();

    assert_eq!(expected_bch_spendable, bch_balance.balances.spendable);
    assert_eq!(expected_bch_unspendable, bch_balance.balances.unspendable);

    let (_, slp_balances) = enable_bch_with_tokens
        .result
        .slp_addresses_infos
        .into_iter()
        .next()
        .unwrap();

    let expected_slp_spendable = BigDecimal::from(1000);
    let expected_slp_unspendable: BigDecimal = 0.into();

    let actual_slp = slp_balances.balances.get("ADEXSLP").unwrap();
    assert_eq!(expected_slp_spendable, actual_slp.spendable);
    assert_eq!(expected_slp_unspendable, actual_slp.unspendable);
}

#[test]
fn test_withdraw_bch_max_must_not_spend_slp() {
    let mm = slp_supplied_node();

    block_on(enable_native_bch(&mm, "FORSLP", &[]));
    block_on(enable_native(&mm, "ADEXSLP", &[]));

    withdraw_max_and_send_v1(&mm, "FORSLP", &utxo_burn_address().to_string());
    thread::sleep(Duration::from_secs(1));

    let bch_balance = get_balance(&mm, "FORSLP");
    let expected_spendable = BigDecimal::from(0);
    let expected_unspendable: BigDecimal = "0.00001".parse().unwrap();

    assert_eq!(expected_spendable, bch_balance.balance);
    assert_eq!(expected_unspendable, bch_balance.unspendable_balance);

    let slp_balance = get_balance(&mm, "ADEXSLP");
    let expected_spendable = BigDecimal::from(1000);

    assert_eq!(expected_spendable, slp_balance.balance);
}
