use crate::docker_tests::docker_tests_common::{eth_distributor, generate_jst_with_seed};
use crate::integration_tests_common::*;
use crate::{generate_utxo_coin_with_privkey, generate_utxo_coin_with_random_privkey, random_secp256k1_secret};
use coins::coin_errors::ValidatePaymentError;
use coins::utxo::{dhash160, UtxoCommonOps};
use coins::{ConfirmPaymentInput, FoundSwapTxSpend, MarketCoinOps, MmCoin, MmCoinEnum, RefundPaymentArgs, RewardTarget,
            SearchForSwapTxSpendInput, SendPaymentArgs, SwapOps, WatcherOps, WatcherValidatePaymentInput,
            WatcherValidateTakerFeeInput, EARLY_CONFIRMATION_ERR_LOG, INVALID_CONTRACT_ADDRESS_ERR_LOG,
            INVALID_PAYMENT_STATE_ERR_LOG, INVALID_RECEIVER_ERR_LOG, INVALID_REFUND_TX_ERR_LOG,
            INVALID_SCRIPT_ERR_LOG, INVALID_SENDER_ERR_LOG, INVALID_SWAP_ID_ERR_LOG, OLD_TRANSACTION_ERR_LOG};
use common::{block_on, now_sec, wait_until_sec, DEX_FEE_ADDR_RAW_PUBKEY};
use crypto::privkey::{key_pair_from_secret, key_pair_from_seed};
use futures01::Future;
use mm2_main::mm2::lp_swap::{dex_fee_amount, dex_fee_amount_from_taker_coin, dex_fee_threshold, get_payment_locktime,
                             MakerSwap, MAKER_PAYMENT_SENT_LOG, MAKER_PAYMENT_SPEND_FOUND_LOG,
                             MAKER_PAYMENT_SPEND_SENT_LOG, TAKER_PAYMENT_REFUND_SENT_LOG, WATCHER_MESSAGE_SENT_LOG};
use mm2_number::BigDecimal;
use mm2_number::MmNumber;
use mm2_test_helpers::for_tests::{enable_eth_coin, eth_jst_testnet_conf, eth_testnet_conf, mm_dump, my_balance,
                                  mycoin1_conf, mycoin_conf, start_swaps, MarketMakerIt, Mm2TestConf,
                                  DEFAULT_RPC_PASSWORD, ETH_DEV_NODES, ETH_DEV_SWAP_CONTRACT};
use mm2_test_helpers::get_passphrase;
use mm2_test_helpers::structs::WatcherConf;
use num_traits::{One, Zero};
use primitives::hash::H256;
use std::str::FromStr;
use std::thread;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct BalanceResult {
    alice_acoin_balance_before: BigDecimal,
    alice_acoin_balance_middle: BigDecimal,
    alice_acoin_balance_after: BigDecimal,
    alice_bcoin_balance_before: BigDecimal,
    alice_bcoin_balance_middle: BigDecimal,
    alice_bcoin_balance_after: BigDecimal,
    alice_eth_balance_middle: BigDecimal,
    alice_eth_balance_after: BigDecimal,
    bob_acoin_balance_before: BigDecimal,
    bob_acoin_balance_after: BigDecimal,
    bob_bcoin_balance_before: BigDecimal,
    bob_bcoin_balance_after: BigDecimal,
    watcher_acoin_balance_before: BigDecimal,
    watcher_acoin_balance_after: BigDecimal,
    watcher_bcoin_balance_before: BigDecimal,
    watcher_bcoin_balance_after: BigDecimal,
}

fn enable_coin(mm_node: &MarketMakerIt, coin: &str) {
    if coin == "MYCOIN" {
        log!("{:?}", block_on(enable_native(mm_node, coin, &[], None)));
    } else {
        enable_eth(mm_node, coin);
    }
}

fn enable_eth(mm_node: &MarketMakerIt, coin: &str) {
    dbg!(block_on(enable_eth_coin(
        mm_node,
        coin,
        ETH_DEV_NODES,
        ETH_DEV_SWAP_CONTRACT,
        Some(ETH_DEV_SWAP_CONTRACT),
        true
    )));
}

#[allow(clippy::enum_variant_names)]
enum SwapFlow {
    WatcherSpendsMakerPayment,
    WatcherRefundsTakerPayment,
    TakerSpendsMakerPayment,
}

#[allow(clippy::too_many_arguments)]
fn start_swaps_and_get_balances(
    a_coin: &'static str,
    b_coin: &'static str,
    maker_price: f64,
    taker_price: f64,
    volume: f64,
    envs: &[(&str, &str)],
    swap_flow: SwapFlow,
    alice_privkey: &str,
    bob_privkey: &str,
    watcher_privkey: &str,
) -> BalanceResult {
    let coins = json!([
        eth_testnet_conf(),
        eth_jst_testnet_conf(),
        mycoin_conf(1000),
        mycoin1_conf(1000)
    ]);

    let alice_conf = Mm2TestConf::seednode(&format!("0x{}", alice_privkey), &coins);
    let mut mm_alice = block_on(MarketMakerIt::start_with_envs(
        alice_conf.conf.clone(),
        alice_conf.rpc_password.clone(),
        None,
        envs,
    ))
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {}", mm_alice.log_path.display());

    let bob_conf = Mm2TestConf::light_node(&format!("0x{}", bob_privkey), &coins, &[&mm_alice.ip.to_string()]);
    let mut mm_bob = block_on(MarketMakerIt::start_with_envs(
        bob_conf.conf.clone(),
        bob_conf.rpc_password,
        None,
        envs,
    ))
    .unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {}", mm_bob.log_path.display());

    generate_utxo_coin_with_privkey("MYCOIN", 100.into(), H256::from_str(bob_privkey).unwrap());
    generate_utxo_coin_with_privkey("MYCOIN", 100.into(), H256::from_str(alice_privkey).unwrap());
    generate_utxo_coin_with_privkey("MYCOIN1", 100.into(), H256::from_str(bob_privkey).unwrap());
    generate_utxo_coin_with_privkey("MYCOIN1", 100.into(), H256::from_str(alice_privkey).unwrap());

    let (watcher_conf, watcher_log_to_wait) = match swap_flow {
        SwapFlow::WatcherSpendsMakerPayment => (
            WatcherConf {
                wait_taker_payment: 0.,
                wait_maker_payment_spend_factor: 0.,
                refund_start_factor: 1.5,
                search_interval: 1.0,
            },
            MAKER_PAYMENT_SPEND_SENT_LOG,
        ),
        SwapFlow::WatcherRefundsTakerPayment => (
            WatcherConf {
                wait_taker_payment: 0.,
                wait_maker_payment_spend_factor: 1.,
                refund_start_factor: 0.,
                search_interval: 1.,
            },
            TAKER_PAYMENT_REFUND_SENT_LOG,
        ),
        SwapFlow::TakerSpendsMakerPayment => (
            WatcherConf {
                wait_taker_payment: 0.,
                wait_maker_payment_spend_factor: 1.,
                refund_start_factor: 1.5,
                search_interval: 1.0,
            },
            MAKER_PAYMENT_SPEND_FOUND_LOG,
        ),
    };

    let watcher_conf = Mm2TestConf::watcher_light_node(
        &format!("0x{}", watcher_privkey),
        &coins,
        &[&mm_alice.ip.to_string()],
        watcher_conf,
    )
    .conf;

    let mut mm_watcher = block_on(MarketMakerIt::start_with_envs(
        watcher_conf,
        DEFAULT_RPC_PASSWORD.to_string(),
        None,
        envs,
    ))
    .unwrap();
    let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

    enable_coin(&mm_alice, a_coin);
    enable_coin(&mm_alice, b_coin);
    enable_coin(&mm_bob, a_coin);
    enable_coin(&mm_bob, b_coin);
    enable_coin(&mm_watcher, a_coin);
    enable_coin(&mm_watcher, b_coin);

    if a_coin != "ETH" && b_coin != "ETH" {
        enable_coin(&mm_alice, "ETH");
    }

    let alice_acoin_balance_before = block_on(my_balance(&mm_alice, a_coin)).balance;
    let alice_bcoin_balance_before = block_on(my_balance(&mm_alice, b_coin)).balance;
    let bob_acoin_balance_before = block_on(my_balance(&mm_bob, a_coin)).balance;
    let bob_bcoin_balance_before = block_on(my_balance(&mm_bob, b_coin)).balance;
    let watcher_acoin_balance_before = block_on(my_balance(&mm_watcher, a_coin)).balance;
    let watcher_bcoin_balance_before = block_on(my_balance(&mm_watcher, b_coin)).balance;

    let mut alice_acoin_balance_middle = BigDecimal::zero();
    let mut alice_bcoin_balance_middle = BigDecimal::zero();
    let mut alice_eth_balance_middle = BigDecimal::zero();
    let mut bob_acoin_balance_after = BigDecimal::zero();
    let mut bob_bcoin_balance_after = BigDecimal::zero();

    block_on(start_swaps(
        &mut mm_bob,
        &mut mm_alice,
        &[(b_coin, a_coin)],
        maker_price,
        taker_price,
        volume,
    ));

    if matches!(swap_flow, SwapFlow::WatcherRefundsTakerPayment) {
        block_on(mm_bob.wait_for_log(120., |log| log.contains(MAKER_PAYMENT_SENT_LOG))).unwrap();
        block_on(mm_bob.stop()).unwrap();
    }
    if !matches!(swap_flow, SwapFlow::TakerSpendsMakerPayment) {
        block_on(mm_alice.wait_for_log(120., |log| log.contains("Taker payment confirmed"))).unwrap();
        alice_acoin_balance_middle = block_on(my_balance(&mm_alice, a_coin)).balance;
        alice_bcoin_balance_middle = block_on(my_balance(&mm_alice, b_coin)).balance;
        alice_eth_balance_middle = block_on(my_balance(&mm_alice, "ETH")).balance;
        block_on(mm_alice.stop()).unwrap();
    }

    block_on(mm_watcher.wait_for_log(120., |log| log.contains(watcher_log_to_wait))).unwrap();
    thread::sleep(Duration::from_secs(20));

    let mm_alice = MarketMakerIt::start(alice_conf.conf, alice_conf.rpc_password, None).unwrap();
    enable_coin(&mm_alice, a_coin);
    enable_coin(&mm_alice, b_coin);

    if a_coin != "ETH" && b_coin != "ETH" {
        enable_coin(&mm_alice, "ETH");
    }

    let alice_acoin_balance_after = block_on(my_balance(&mm_alice, a_coin)).balance;
    let alice_bcoin_balance_after = block_on(my_balance(&mm_alice, b_coin)).balance;
    let alice_eth_balance_after = block_on(my_balance(&mm_alice, "ETH")).balance;
    if !matches!(swap_flow, SwapFlow::WatcherRefundsTakerPayment) {
        bob_acoin_balance_after = block_on(my_balance(&mm_bob, a_coin)).balance;
        bob_bcoin_balance_after = block_on(my_balance(&mm_bob, b_coin)).balance;
    }
    let watcher_acoin_balance_after = block_on(my_balance(&mm_watcher, a_coin)).balance;
    let watcher_bcoin_balance_after = block_on(my_balance(&mm_watcher, b_coin)).balance;

    BalanceResult {
        alice_acoin_balance_before,
        alice_acoin_balance_middle,
        alice_acoin_balance_after,
        alice_bcoin_balance_before,
        alice_bcoin_balance_middle,
        alice_bcoin_balance_after,
        alice_eth_balance_middle,
        alice_eth_balance_after,
        bob_acoin_balance_before,
        bob_acoin_balance_after,
        bob_bcoin_balance_before,
        bob_bcoin_balance_after,
        watcher_acoin_balance_before,
        watcher_acoin_balance_after,
        watcher_bcoin_balance_before,
        watcher_bcoin_balance_after,
    }
}

#[test]
fn test_watcher_spends_maker_payment_utxo_utxo() {
    let alice_privkey = hex::encode(random_secp256k1_secret());
    let bob_privkey = hex::encode(random_secp256k1_secret());
    let watcher_privkey = hex::encode(random_secp256k1_secret());

    let balances = start_swaps_and_get_balances(
        "MYCOIN",
        "MYCOIN1",
        25.,
        25.,
        2.,
        &[("USE_WATCHERS", "")],
        SwapFlow::WatcherSpendsMakerPayment,
        &alice_privkey,
        &bob_privkey,
        &watcher_privkey,
    );

    let acoin_volume = BigDecimal::from_str("50").unwrap();
    let bcoin_volume = BigDecimal::from_str("2").unwrap();

    assert_eq!(
        balances.alice_acoin_balance_after.round(0),
        balances.alice_acoin_balance_before - acoin_volume.clone()
    );
    assert_eq!(
        balances.alice_bcoin_balance_after.round(0),
        balances.alice_bcoin_balance_before + bcoin_volume.clone()
    );
    assert_eq!(
        balances.bob_acoin_balance_after.round(0),
        balances.bob_acoin_balance_before + acoin_volume
    );
    assert_eq!(
        balances.bob_bcoin_balance_after.round(0),
        balances.bob_bcoin_balance_before - bcoin_volume
    );
}

#[test]
fn test_watcher_spends_maker_payment_utxo_eth() {
    let alice_privkey = "0af1b1a4cdfbec12c9014e2422c8819e02e5d0f6539f8bf15190d3ea592e4f14";
    let bob_privkey = "3245331f141578d8c4604639deb1e6f38f107a65642525ef32387325a079a463";
    let watcher_privkey = "9d1d86be257b3bd2504757689d0da24dd052fdff0641be073f1ea8aa5cccf597";

    let balances = start_swaps_and_get_balances(
        "ETH",
        "MYCOIN",
        0.01,
        0.01,
        1.,
        &[("USE_WATCHERS", ""), ("USE_WATCHER_REWARD", "")],
        SwapFlow::WatcherSpendsMakerPayment,
        alice_privkey,
        bob_privkey,
        watcher_privkey,
    );

    let mycoin_volume = BigDecimal::from_str("1").unwrap();
    let eth_volume = BigDecimal::from_str("0.01").unwrap();

    assert_eq!(
        balances.alice_bcoin_balance_after.round(0),
        balances.alice_bcoin_balance_before + mycoin_volume
    );
    assert_eq!(
        balances.bob_acoin_balance_after.with_scale(2),
        balances.bob_acoin_balance_before.with_scale(2) + eth_volume
    );
    assert!(balances.alice_acoin_balance_after > balances.alice_acoin_balance_middle);
}

#[test]
fn test_watcher_spends_maker_payment_eth_utxo() {
    let alice_privkey = "0591b2acbe4798c6156a26bc8106c36d6fc09a85c9e02710eec32c1b41f047ec";
    let bob_privkey = "b6e59dee1112486573989f07d480691ca7e3eab81b499fe801d94b65ea1f1341";
    let watcher_privkey = "dc8ad0723a6a2c02d3239e8b009d4de6f3f0ad8b9bc51838cbed41edb378dd86";

    let balances = start_swaps_and_get_balances(
        "MYCOIN",
        "ETH",
        100.,
        100.,
        0.01,
        &[
            ("USE_WATCHERS", ""),
            ("TEST_COIN_PRICE", "0.01"),
            ("USE_WATCHER_REWARD", ""),
        ],
        SwapFlow::WatcherSpendsMakerPayment,
        alice_privkey,
        bob_privkey,
        watcher_privkey,
    );

    let eth_volume = BigDecimal::from_str("0.01").unwrap();
    let mycoin_volume = BigDecimal::from_str("1").unwrap();
    let dex_fee_threshold = dex_fee_threshold(BigDecimal::from_str("0.00001").unwrap().into());

    let dex_fee: BigDecimal = dex_fee_amount(
        "MYCOIN",
        "ETH",
        &MmNumber::from(mycoin_volume.clone()),
        &dex_fee_threshold,
    )
    .into();
    let alice_mycoin_reward_sent = balances.alice_acoin_balance_before
        - balances.alice_acoin_balance_after.clone()
        - mycoin_volume.clone()
        - dex_fee.with_scale(8);

    assert_eq!(
        balances.alice_bcoin_balance_after,
        balances.alice_bcoin_balance_middle + eth_volume
    );
    assert_eq!(
        balances.bob_acoin_balance_after.round(2),
        balances.bob_acoin_balance_before + mycoin_volume + alice_mycoin_reward_sent.round(2)
    );
    assert!(balances.watcher_bcoin_balance_after > balances.watcher_bcoin_balance_before);
}

#[test]
fn test_watcher_spends_maker_payment_eth_erc20() {
    let alice_privkey = "92ee1f48f07dcaab03ff3d5077211912fdf2229bb401e7a969f73fc2c3d4fe3f";
    let bob_privkey = "59e8c09c3aace4eb9301b2f70547fc0936be2bc662b9c0a7a625b5e8929491c7";
    let watcher_privkey = "e0915d112440fdc58405faace4626a983bb3fd8cb51f0e5a7ed8565b552b5751";

    let balances = start_swaps_and_get_balances(
        "JST",
        "ETH",
        100.,
        100.,
        0.01,
        &[
            ("USE_WATCHERS", ""),
            ("TEST_COIN_PRICE", "0.01"),
            ("USE_WATCHER_REWARD", ""),
        ],
        SwapFlow::WatcherSpendsMakerPayment,
        alice_privkey,
        bob_privkey,
        watcher_privkey,
    );

    let eth_volume = BigDecimal::from_str("0.01").unwrap();
    let jst_volume = BigDecimal::from_str("1").unwrap();

    assert_eq!(
        balances.alice_bcoin_balance_after,
        balances.alice_bcoin_balance_middle + eth_volume
    );
    assert_eq!(
        balances.bob_acoin_balance_after,
        balances.bob_acoin_balance_before + jst_volume
    );
    assert!(balances.watcher_bcoin_balance_after > balances.watcher_bcoin_balance_before);
}

#[test]
fn test_watcher_spends_maker_payment_erc20_eth() {
    let alice_privkey = "2fd8d83e3b9799fa0a02cdaf6776dd36eee3243a62d399a54dc9a68f5e77b27c";
    let bob_privkey = "6425a922265573100165b60ff380fba5035c7406169087a43aefdee66aceccc1";
    let watcher_privkey = "b9b5fa738dcf7c99073b0f7d518a50b72139a7636ba3488766944fd3dc4df646";

    let balances = start_swaps_and_get_balances(
        "ETH",
        "JST",
        0.01,
        0.01,
        1.,
        &[("USE_WATCHERS", ""), ("USE_WATCHER_REWARD", "")],
        SwapFlow::WatcherSpendsMakerPayment,
        alice_privkey,
        bob_privkey,
        watcher_privkey,
    );

    let jst_volume = BigDecimal::from_str("1").unwrap();
    let eth_volume = BigDecimal::from_str("0.01").unwrap();

    assert_eq!(
        balances.alice_bcoin_balance_after,
        balances.alice_bcoin_balance_before + jst_volume
    );
    assert_eq!(
        balances.bob_acoin_balance_after.with_scale(2),
        balances.bob_acoin_balance_before.with_scale(2) + eth_volume
    );
    assert!(balances.watcher_acoin_balance_after > balances.watcher_acoin_balance_before);
}

#[test]
fn test_watcher_spends_maker_payment_utxo_erc20() {
    let alice_privkey = "e4fc65b69c323312ee3ba46406671bc9f2d524190621d82eeb51452701cfe43b";
    let bob_privkey = "721fc6b7f56495f7f721e1e11cddcaf593351264705c4044e83656f06eb595ef";
    let watcher_privkey = "a1f1c2666be032492a3cb772abc8a2845adfd6dca299fbed13416ccc6feb57ee";

    let balances = start_swaps_and_get_balances(
        "JST",
        "MYCOIN",
        1.,
        1.,
        1.,
        &[
            ("USE_WATCHERS", ""),
            ("TEST_COIN_PRICE", "0.01"),
            ("USE_WATCHER_REWARD", ""),
        ],
        SwapFlow::WatcherSpendsMakerPayment,
        alice_privkey,
        bob_privkey,
        watcher_privkey,
    );

    let mycoin_volume = BigDecimal::from_str("1").unwrap();
    let jst_volume = BigDecimal::from_str("1").unwrap();

    assert_eq!(
        balances.alice_bcoin_balance_after.round(0),
        balances.alice_bcoin_balance_before + mycoin_volume
    );
    assert_eq!(
        balances.bob_acoin_balance_after,
        balances.bob_acoin_balance_before + jst_volume
    );
    assert!(balances.alice_eth_balance_after > balances.alice_eth_balance_middle);
}

#[test]
fn test_watcher_spends_maker_payment_erc20_utxo() {
    let alice_privkey = "5c9fbc69376c3ee6bb56d8d2b715f24b3bb92ccd47e93332d4d94899aa9fc7ae";
    let bob_privkey = "ccc24b9653087d939949d513756cefe1eff657de4c5bf34febc97843a6b26782";
    let watcher_privkey = "a1f1c2666be032492a3cb772abc8a2845adfd6dca299fbed13416ccc6feb57ee";

    let balances = start_swaps_and_get_balances(
        "MYCOIN",
        "JST",
        1.,
        1.,
        1.,
        &[
            ("USE_WATCHERS", ""),
            ("TEST_COIN_PRICE", "0.01"),
            ("USE_WATCHER_REWARD", ""),
        ],
        SwapFlow::WatcherSpendsMakerPayment,
        alice_privkey,
        bob_privkey,
        watcher_privkey,
    );

    let mycoin_volume = BigDecimal::from_str("1").unwrap();
    let jst_volume = BigDecimal::from_str("1").unwrap();

    let dex_fee_threshold = dex_fee_threshold(BigDecimal::from_str("0.00001").unwrap().into());
    let dex_fee: BigDecimal = dex_fee_amount(
        "MYCOIN",
        "JST",
        &MmNumber::from(mycoin_volume.clone()),
        &dex_fee_threshold,
    )
    .into();
    let alice_mycoin_reward_sent = balances.alice_acoin_balance_before
        - balances.alice_acoin_balance_after.clone()
        - mycoin_volume.clone()
        - dex_fee.with_scale(8);

    let bob_jst_reward_sent =
        balances.bob_bcoin_balance_before - jst_volume.clone() - balances.bob_bcoin_balance_after.clone();

    assert_eq!(
        balances.alice_bcoin_balance_after,
        balances.alice_bcoin_balance_before + jst_volume
    );
    assert_eq!(
        balances.bob_acoin_balance_after.round(2),
        balances.bob_acoin_balance_before + mycoin_volume + alice_mycoin_reward_sent.round(2)
    );
    assert_eq!(
        balances.watcher_bcoin_balance_after,
        balances.watcher_bcoin_balance_before + bob_jst_reward_sent
    );
}

#[test]
fn test_watcher_refunds_taker_payment_utxo() {
    let alice_privkey = &hex::encode(random_secp256k1_secret());
    let bob_privkey = &hex::encode(random_secp256k1_secret());
    let watcher_privkey = &hex::encode(random_secp256k1_secret());

    let balances = start_swaps_and_get_balances(
        "MYCOIN1",
        "MYCOIN",
        25.,
        25.,
        2.,
        &[("USE_WATCHERS", ""), ("USE_TEST_LOCKTIME", "")],
        SwapFlow::WatcherRefundsTakerPayment,
        alice_privkey,
        bob_privkey,
        watcher_privkey,
    );

    assert_eq!(
        balances.alice_acoin_balance_after.round(0),
        balances.alice_acoin_balance_before
    );
    assert_eq!(balances.alice_bcoin_balance_after, balances.alice_bcoin_balance_before);
}

#[test]
fn test_watcher_refunds_taker_payment_eth() {
    let alice_privkey = "0816c0558b934fafa845946bdd2b3163fe6b928e6160ea9aa10a8bea221e3813";
    let bob_privkey = "e5cb76954c5160d7df5bfa5798540d3583c73c9daa46903b98abb9eed2edecc6";
    let watcher_privkey = "ccd7f2c0da8f6428b60b42a27c0e37af59abd42251773156f4f59c5d16855f8c";

    let balances = start_swaps_and_get_balances(
        "ETH",
        "JST",
        0.01,
        0.01,
        1.,
        &[
            ("USE_WATCHERS", ""),
            ("USE_TEST_LOCKTIME", ""),
            ("USE_WATCHER_REWARD", ""),
        ],
        SwapFlow::WatcherRefundsTakerPayment,
        alice_privkey,
        bob_privkey,
        watcher_privkey,
    );
    assert_eq!(
        balances.alice_acoin_balance_after.with_scale(2),
        balances.alice_acoin_balance_before.with_scale(2)
    );
    assert_eq!(balances.alice_bcoin_balance_after, balances.alice_bcoin_balance_before);
    assert!(balances.watcher_acoin_balance_after > balances.watcher_acoin_balance_before);
}

#[test]
fn test_watcher_refunds_taker_payment_erc20() {
    let alice_privkey = "82c1bb28bb13488f901eff67f886e9895c4dfa28e3e24f1ed7873a73231c9492";
    let bob_privkey = "9a4721db00336ea0d8b7a373cdbdefc321285e7959fff8aea493af6f485b683f";
    let watcher_privkey = "8fdf25f087140b2797deb2a1d3ce66bd59e2449cc805b99958b3bfa8cd621eb8";

    let balances = start_swaps_and_get_balances(
        "JST",
        "ETH",
        100.,
        100.,
        0.01,
        &[
            ("USE_WATCHERS", ""),
            ("USE_TEST_LOCKTIME", ""),
            ("TEST_COIN_PRICE", "0.01"),
            ("USE_WATCHER_REWARD", ""),
        ],
        SwapFlow::WatcherRefundsTakerPayment,
        alice_privkey,
        bob_privkey,
        watcher_privkey,
    );
    let jst_volume = BigDecimal::from_str("1").unwrap();

    assert_eq!(
        balances.alice_acoin_balance_after,
        balances.alice_acoin_balance_middle + jst_volume
    );

    assert!(balances.watcher_bcoin_balance_after > balances.watcher_bcoin_balance_before);
}

#[test]
fn test_watcher_waits_for_taker_utxo() {
    let alice_privkey = &hex::encode(random_secp256k1_secret());
    let bob_privkey = &hex::encode(random_secp256k1_secret());
    let watcher_privkey = &hex::encode(random_secp256k1_secret());

    start_swaps_and_get_balances(
        "MYCOIN1",
        "MYCOIN",
        25.,
        25.,
        2.,
        &[],
        SwapFlow::TakerSpendsMakerPayment,
        alice_privkey,
        bob_privkey,
        watcher_privkey,
    );
}

#[test]
fn test_watcher_waits_for_taker_eth() {
    let alice_privkey = "814ea055c807c1ff2d49c81abfc3434fa0d10a427369b1f8d60fc78ab1da7d16";
    let bob_privkey = "36533ec51a61f4b32856c8ce2ee811a263c625ae26e45ee68e6d28b65c8f9298";
    let watcher_privkey = "baa1c83a0993ba96f88ffc943919991792ce9e2498fc41f42b38030915d58f9f";

    start_swaps_and_get_balances(
        "JST",
        "ETH",
        100.,
        100.,
        0.01,
        &[
            ("USE_WATCHERS", ""),
            ("TEST_COIN_PRICE", "0.01"),
            ("USE_WATCHER_REWARD", ""),
        ],
        SwapFlow::TakerSpendsMakerPayment,
        alice_privkey,
        bob_privkey,
        watcher_privkey,
    );
}

#[test]
fn test_two_watchers_spend_maker_payment_eth_erc20() {
    let coins = json!([eth_testnet_conf(), eth_jst_testnet_conf()]);

    let alice_passphrase =
        String::from("spice describe gravity federal blast come thank unfair canal monkey style afraid");
    let alice_conf = Mm2TestConf::seednode(&alice_passphrase, &coins);
    let mut mm_alice = MarketMakerIt::start(alice_conf.conf.clone(), alice_conf.rpc_password.clone(), None).unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {}", mm_alice.log_path.display());

    let bob_passphrase = String::from("also shoot benefit prefer juice shell elder veteran woman mimic image kidney");
    let bob_conf = Mm2TestConf::light_node(&bob_passphrase, &coins, &[&mm_alice.ip.to_string()]);
    let mut mm_bob = MarketMakerIt::start(bob_conf.conf, bob_conf.rpc_password, None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {}", mm_bob.log_path.display());

    let watcher1_passphrase =
        String::from("also shoot benefit prefer juice shell thank unfair canal monkey style afraid");
    let watcher1_conf =
        Mm2TestConf::watcher_light_node(&watcher1_passphrase, &coins, &[&mm_alice.ip.to_string()], WatcherConf {
            wait_taker_payment: 0.,
            wait_maker_payment_spend_factor: 0.,
            refund_start_factor: 1.5,
            search_interval: 1.0,
        })
        .conf;
    let mut mm_watcher1 = MarketMakerIt::start(watcher1_conf, DEFAULT_RPC_PASSWORD.to_string(), None).unwrap();
    let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher1.log_path);

    let watcher2_passphrase =
        String::from("also shoot benefit shell thank prefer juice unfair canal monkey style afraid");
    let watcher2_conf =
        Mm2TestConf::watcher_light_node(&watcher2_passphrase, &coins, &[&mm_alice.ip.to_string()], WatcherConf {
            wait_taker_payment: 0.,
            wait_maker_payment_spend_factor: 0.,
            refund_start_factor: 1.5,
            search_interval: 1.0,
        })
        .conf;
    let mut mm_watcher2 = MarketMakerIt::start(watcher2_conf, DEFAULT_RPC_PASSWORD.to_string(), None).unwrap();
    let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher1.log_path);

    enable_eth(&mm_alice, "ETH");
    enable_eth(&mm_alice, "JST");
    enable_eth(&mm_bob, "ETH");
    enable_eth(&mm_bob, "JST");
    enable_eth(&mm_watcher1, "ETH");
    enable_eth(&mm_watcher1, "JST");
    enable_eth(&mm_watcher2, "ETH");
    enable_eth(&mm_watcher2, "JST");

    let alice_eth_balance_before = block_on(my_balance(&mm_alice, "ETH")).balance.with_scale(2);
    let alice_jst_balance_before = block_on(my_balance(&mm_alice, "JST")).balance.with_scale(2);
    let bob_eth_balance_before = block_on(my_balance(&mm_bob, "ETH")).balance.with_scale(2);
    let bob_jst_balance_before = block_on(my_balance(&mm_bob, "JST")).balance.with_scale(2);
    let watcher1_eth_balance_before = block_on(my_balance(&mm_watcher1, "ETH")).balance;
    let watcher2_eth_balance_before = block_on(my_balance(&mm_watcher2, "ETH")).balance;

    block_on(start_swaps(&mut mm_bob, &mut mm_alice, &[("ETH", "JST")], 1., 1., 0.01));

    block_on(mm_alice.wait_for_log(180., |log| log.contains(WATCHER_MESSAGE_SENT_LOG))).unwrap();
    block_on(mm_alice.stop()).unwrap();
    block_on(mm_watcher1.wait_for_log(180., |log| log.contains(MAKER_PAYMENT_SPEND_SENT_LOG))).unwrap();
    block_on(mm_watcher2.wait_for_log(180., |log| log.contains(MAKER_PAYMENT_SPEND_SENT_LOG))).unwrap();
    thread::sleep(Duration::from_secs(25));

    let mm_alice = MarketMakerIt::start(alice_conf.conf.clone(), alice_conf.rpc_password, None).unwrap();
    enable_eth(&mm_alice, "ETH");
    enable_eth(&mm_alice, "JST");

    let alice_eth_balance_after = block_on(my_balance(&mm_alice, "ETH")).balance.with_scale(2);
    let alice_jst_balance_after = block_on(my_balance(&mm_alice, "JST")).balance.with_scale(2);
    let bob_eth_balance_after = block_on(my_balance(&mm_bob, "ETH")).balance.with_scale(2);
    let bob_jst_balance_after = block_on(my_balance(&mm_bob, "JST")).balance.with_scale(2);
    let watcher1_eth_balance_after = block_on(my_balance(&mm_watcher1, "ETH")).balance;
    let watcher2_eth_balance_after = block_on(my_balance(&mm_watcher2, "ETH")).balance;

    let volume = BigDecimal::from_str("0.01").unwrap();
    assert_eq!(alice_jst_balance_before - volume.clone(), alice_jst_balance_after);
    assert_eq!(bob_jst_balance_before + volume.clone(), bob_jst_balance_after);
    assert_eq!(alice_eth_balance_before + volume.clone(), alice_eth_balance_after);
    assert_eq!(bob_eth_balance_before - volume, bob_eth_balance_after);
    if watcher1_eth_balance_after > watcher1_eth_balance_before {
        assert_eq!(watcher2_eth_balance_after, watcher2_eth_balance_after);
    }
    if watcher2_eth_balance_after > watcher2_eth_balance_before {
        assert_eq!(watcher1_eth_balance_after, watcher1_eth_balance_after);
    }
}

#[test]
fn test_watcher_validate_taker_fee_utxo() {
    let timeout = wait_until_sec(120); // timeout if test takes more than 120 seconds to run
    let lock_duration = get_payment_locktime();
    let (_ctx, taker_coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    let (_ctx, maker_coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    let taker_pubkey = taker_coin.my_public_key().unwrap();

    let taker_amount = MmNumber::from((10, 1));
    let fee_amount = dex_fee_amount_from_taker_coin(&taker_coin, maker_coin.ticker(), &taker_amount);

    let taker_fee = taker_coin
        .send_taker_fee(&DEX_FEE_ADDR_RAW_PUBKEY, fee_amount.into(), Uuid::new_v4().as_bytes())
        .wait()
        .unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: taker_fee.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };

    taker_coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let validate_taker_fee_res = taker_coin
        .watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
            taker_fee_hash: taker_fee.tx_hash().into_vec(),
            sender_pubkey: taker_pubkey.to_vec(),
            min_block_number: 0,
            fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
            lock_duration,
        })
        .wait();
    assert!(validate_taker_fee_res.is_ok());

    let error = taker_coin
        .watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
            taker_fee_hash: taker_fee.tx_hash().into_vec(),
            sender_pubkey: maker_coin.my_public_key().unwrap().to_vec(),
            min_block_number: 0,
            fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
            lock_duration,
        })
        .wait()
        .unwrap_err()
        .into_inner();

    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_SENDER_ERR_LOG))
        },
        _ => panic!("Expected `WrongPaymentTx` invalid public key, found {:?}", error),
    }

    let error = taker_coin
        .watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
            taker_fee_hash: taker_fee.tx_hash().into_vec(),
            sender_pubkey: taker_pubkey.to_vec(),
            min_block_number: std::u64::MAX,
            fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
            lock_duration,
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(EARLY_CONFIRMATION_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` confirmed before min_block, found {:?}",
            error
        ),
    }

    let error = taker_coin
        .watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
            taker_fee_hash: taker_fee.tx_hash().into_vec(),
            sender_pubkey: taker_pubkey.to_vec(),
            min_block_number: 0,
            fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
            lock_duration: 0,
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(OLD_TRANSACTION_ERR_LOG))
        },
        _ => panic!("Expected `WrongPaymentTx` transaction too old, found {:?}", error),
    }

    let error = taker_coin
        .watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
            taker_fee_hash: taker_fee.tx_hash().into_vec(),
            sender_pubkey: taker_pubkey.to_vec(),
            min_block_number: 0,
            fee_addr: taker_pubkey.to_vec(),
            lock_duration,
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_RECEIVER_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` tx output script_pubkey doesn't match expected, found {:?}",
            error
        ),
    }
}

#[test]
fn test_watcher_validate_taker_fee_eth() {
    let timeout = wait_until_sec(120); // timeout if test takes more than 120 seconds to run
    let lock_duration = get_payment_locktime();

    let taker_coin = eth_distributor();
    let taker_keypair = taker_coin.derive_htlc_key_pair(&[]);
    let taker_pubkey = taker_keypair.public();

    let taker_amount = MmNumber::from((1, 1));
    let fee_amount = dex_fee_amount_from_taker_coin(&taker_coin, "ETH", &taker_amount);
    let taker_fee = taker_coin
        .send_taker_fee(&DEX_FEE_ADDR_RAW_PUBKEY, fee_amount.into(), Uuid::new_v4().as_bytes())
        .wait()
        .unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: taker_fee.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    taker_coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let validate_taker_fee_res = taker_coin
        .watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
            taker_fee_hash: taker_fee.tx_hash().into_vec(),
            sender_pubkey: taker_pubkey.to_vec(),
            min_block_number: 0,
            fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
            lock_duration,
        })
        .wait();
    assert!(validate_taker_fee_res.is_ok());

    let wrong_keypair = key_pair_from_secret(random_secp256k1_secret().as_slice()).unwrap();
    let error = taker_coin
        .watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
            taker_fee_hash: taker_fee.tx_hash().into_vec(),
            sender_pubkey: wrong_keypair.public().to_vec(),
            min_block_number: 0,
            fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
            lock_duration,
        })
        .wait()
        .unwrap_err()
        .into_inner();

    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_SENDER_ERR_LOG))
        },
        _ => panic!("Expected `WrongPaymentTx` invalid public key, found {:?}", error),
    }

    let error = taker_coin
        .watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
            taker_fee_hash: taker_fee.tx_hash().into_vec(),
            sender_pubkey: taker_pubkey.to_vec(),
            min_block_number: std::u64::MAX,
            fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
            lock_duration,
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(EARLY_CONFIRMATION_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` confirmed before min_block, found {:?}",
            error
        ),
    }

    let error = taker_coin
        .watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
            taker_fee_hash: taker_fee.tx_hash().into_vec(),
            sender_pubkey: taker_pubkey.to_vec(),
            min_block_number: 0,
            fee_addr: taker_pubkey.to_vec(),
            lock_duration,
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_RECEIVER_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` tx output script_pubkey doesn't match expected, found {:?}",
            error
        ),
    }
}

#[test]
fn test_watcher_validate_taker_fee_erc20() {
    let timeout = wait_until_sec(120); // timeout if test takes more than 120 seconds to run
    let lock_duration = get_payment_locktime();

    let seed = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();
    let taker_coin = generate_jst_with_seed(&seed);
    let taker_keypair = taker_coin.derive_htlc_key_pair(&[]);
    let taker_pubkey = taker_keypair.public();

    let taker_amount = MmNumber::from((1, 1));
    let fee_amount = dex_fee_amount_from_taker_coin(&taker_coin, "ETH", &taker_amount);
    let taker_fee = taker_coin
        .send_taker_fee(&DEX_FEE_ADDR_RAW_PUBKEY, fee_amount.into(), Uuid::new_v4().as_bytes())
        .wait()
        .unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: taker_fee.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    taker_coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let validate_taker_fee_res = taker_coin
        .watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
            taker_fee_hash: taker_fee.tx_hash().into_vec(),
            sender_pubkey: taker_pubkey.to_vec(),
            min_block_number: 0,
            fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
            lock_duration,
        })
        .wait();
    assert!(validate_taker_fee_res.is_ok());

    let wrong_keypair = key_pair_from_secret(random_secp256k1_secret().as_slice()).unwrap();
    let error = taker_coin
        .watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
            taker_fee_hash: taker_fee.tx_hash().into_vec(),
            sender_pubkey: wrong_keypair.public().to_vec(),
            min_block_number: 0,
            fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
            lock_duration,
        })
        .wait()
        .unwrap_err()
        .into_inner();

    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_SENDER_ERR_LOG))
        },
        _ => panic!("Expected `WrongPaymentTx` invalid public key, found {:?}", error),
    }

    let error = taker_coin
        .watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
            taker_fee_hash: taker_fee.tx_hash().into_vec(),
            sender_pubkey: taker_pubkey.to_vec(),
            min_block_number: std::u64::MAX,
            fee_addr: DEX_FEE_ADDR_RAW_PUBKEY.to_vec(),
            lock_duration,
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(EARLY_CONFIRMATION_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` confirmed before min_block, found {:?}",
            error
        ),
    }

    let error = taker_coin
        .watcher_validate_taker_fee(WatcherValidateTakerFeeInput {
            taker_fee_hash: taker_fee.tx_hash().into_vec(),
            sender_pubkey: taker_pubkey.to_vec(),
            min_block_number: 0,
            fee_addr: taker_pubkey.to_vec(),
            lock_duration,
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_RECEIVER_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` tx output script_pubkey doesn't match expected, found {:?}",
            error
        ),
    }
}

#[test]
fn test_watcher_validate_taker_payment_utxo() {
    let timeout = wait_until_sec(120); // timeout if test takes more than 120 seconds to run
    let time_lock_duration = get_payment_locktime();
    let wait_for_confirmation_until = wait_until_sec(time_lock_duration);
    let time_lock = wait_for_confirmation_until;

    let (_ctx, taker_coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    let taker_pubkey = taker_coin.my_public_key().unwrap();

    let (_ctx, maker_coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    let maker_pubkey = maker_coin.my_public_key().unwrap();

    let secret_hash = dhash160(&MakerSwap::generate_secret().unwrap());

    let taker_payment = taker_coin
        .send_taker_payment(SendPaymentArgs {
            time_lock_duration,
            time_lock,
            other_pubkey: maker_pubkey,
            secret_hash: secret_hash.as_slice(),
            amount: BigDecimal::from(10),
            swap_contract_address: &None,
            swap_unique_data: &[],
            payment_instructions: &None,
            watcher_reward: None,
            wait_for_confirmation_until,
        })
        .wait()
        .unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: taker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    taker_coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let taker_payment_refund_preimage = taker_coin
        .create_taker_payment_refund_preimage(
            &taker_payment.tx_hex(),
            time_lock,
            maker_pubkey,
            secret_hash.as_slice(),
            &None,
            &[],
        )
        .wait()
        .unwrap();
    let validate_taker_payment_res = taker_coin
        .watcher_validate_taker_payment(WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: taker_payment_refund_preimage.tx_hex(),
            time_lock,
            taker_pub: taker_pubkey.to_vec(),
            maker_pub: maker_pubkey.to_vec(),
            secret_hash: secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::UtxoCoin(maker_coin.clone()),
        })
        .wait();
    assert!(validate_taker_payment_res.is_ok());

    let error = taker_coin
        .watcher_validate_taker_payment(WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: taker_payment_refund_preimage.tx_hex(),
            time_lock,
            taker_pub: maker_pubkey.to_vec(),
            maker_pub: maker_pubkey.to_vec(),
            secret_hash: secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::UtxoCoin(maker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();

    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_SENDER_ERR_LOG))
        },
        _ => panic!("Expected `WrongPaymentTx` {INVALID_SENDER_ERR_LOG}, found {:?}", error),
    }

    // Used to get wrong swap id
    let wrong_secret_hash = dhash160(&MakerSwap::generate_secret().unwrap());
    let error = taker_coin
        .watcher_validate_taker_payment(WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: taker_payment_refund_preimage.tx_hex(),
            time_lock,
            taker_pub: taker_pubkey.to_vec(),
            maker_pub: maker_pubkey.to_vec(),
            secret_hash: wrong_secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::UtxoCoin(maker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();

    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_SCRIPT_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` {}, found {:?}",
            INVALID_SCRIPT_ERR_LOG, error
        ),
    }

    let taker_payment_wrong_secret = taker_coin
        .send_taker_payment(SendPaymentArgs {
            time_lock_duration,
            time_lock,
            other_pubkey: maker_pubkey,
            secret_hash: wrong_secret_hash.as_slice(),
            amount: BigDecimal::from(10),
            swap_contract_address: &taker_coin.swap_contract_address(),
            swap_unique_data: &[],
            payment_instructions: &None,
            watcher_reward: None,
            wait_for_confirmation_until,
        })
        .wait()
        .unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: taker_payment_wrong_secret.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    taker_coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let error = taker_coin
        .watcher_validate_taker_payment(WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: taker_payment_refund_preimage.tx_hex(),
            time_lock: 500,
            taker_pub: taker_pubkey.to_vec(),
            maker_pub: maker_pubkey.to_vec(),
            secret_hash: wrong_secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::UtxoCoin(maker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();

    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_SCRIPT_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` {}, found {:?}",
            INVALID_SCRIPT_ERR_LOG, error
        ),
    }

    let wrong_taker_payment_refund_preimage = taker_coin
        .create_taker_payment_refund_preimage(
            &taker_payment.tx_hex(),
            time_lock,
            maker_pubkey,
            wrong_secret_hash.as_slice(),
            &None,
            &[],
        )
        .wait()
        .unwrap();

    let error = taker_coin
        .watcher_validate_taker_payment(WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: wrong_taker_payment_refund_preimage.tx_hex(),
            time_lock,
            taker_pub: taker_pubkey.to_vec(),
            maker_pub: maker_pubkey.to_vec(),
            secret_hash: secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::UtxoCoin(maker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();

    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_REFUND_TX_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` {}, found {:?}",
            INVALID_REFUND_TX_ERR_LOG, error
        ),
    }
}

#[test]
fn test_watcher_validate_taker_payment_eth() {
    let timeout = wait_until_sec(120); // timeout if test takes more than 120 seconds to run

    let taker_coin = eth_distributor();
    let taker_keypair = taker_coin.derive_htlc_key_pair(&[]);
    let taker_pub = taker_keypair.public();

    let maker_seed = get_passphrase!(".env.seed", "BOB_PASSPHRASE").unwrap();
    let maker_keypair = key_pair_from_seed(&maker_seed).unwrap();
    let maker_pub = maker_keypair.public();

    let time_lock_duration = get_payment_locktime();
    let wait_for_confirmation_until = wait_until_sec(time_lock_duration);
    let time_lock = wait_for_confirmation_until;
    let taker_amount = BigDecimal::from_str("0.01").unwrap();
    let maker_amount = BigDecimal::from_str("0.01").unwrap();
    let secret_hash = dhash160(&MakerSwap::generate_secret().unwrap());
    let watcher_reward = Some(
        block_on(taker_coin.get_taker_watcher_reward(
            &MmCoinEnum::from(taker_coin.clone()),
            Some(taker_amount.clone()),
            Some(maker_amount),
            None,
            wait_for_confirmation_until,
        ))
        .unwrap(),
    );

    let taker_payment = taker_coin
        .send_taker_payment(SendPaymentArgs {
            time_lock_duration,
            time_lock,
            other_pubkey: maker_pub,
            secret_hash: secret_hash.as_slice(),
            amount: taker_amount.clone(),
            swap_contract_address: &taker_coin.swap_contract_address(),
            swap_unique_data: &[],
            payment_instructions: &None,
            watcher_reward: watcher_reward.clone(),
            wait_for_confirmation_until,
        })
        .wait()
        .unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: taker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    taker_coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let validate_taker_payment_res = taker_coin
        .watcher_validate_taker_payment(coins::WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: Vec::new(),
            time_lock,
            taker_pub: taker_pub.to_vec(),
            maker_pub: maker_pub.to_vec(),
            secret_hash: secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::EthCoin(taker_coin.clone()),
        })
        .wait();
    assert!(validate_taker_payment_res.is_ok());

    let error = taker_coin
        .watcher_validate_taker_payment(coins::WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: Vec::new(),
            time_lock,
            taker_pub: maker_pub.to_vec(),
            maker_pub: maker_pub.to_vec(),
            secret_hash: secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::EthCoin(taker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_SENDER_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` {}, found {:?}",
            INVALID_SENDER_ERR_LOG, error
        ),
    }

    let taker_payment_wrong_contract = taker_coin
        .send_taker_payment(SendPaymentArgs {
            time_lock_duration,
            time_lock,
            other_pubkey: maker_pub,
            secret_hash: secret_hash.as_slice(),
            amount: taker_amount.clone(),
            swap_contract_address: &Some("9130b257d37a52e52f21054c4da3450c72f595ce".into()),
            swap_unique_data: &[],
            payment_instructions: &None,
            watcher_reward: watcher_reward.clone(),
            wait_for_confirmation_until,
        })
        .wait()
        .unwrap();

    let error = taker_coin
        .watcher_validate_taker_payment(coins::WatcherValidatePaymentInput {
            payment_tx: taker_payment_wrong_contract.tx_hex(),
            taker_payment_refund_preimage: Vec::new(),
            time_lock,
            taker_pub: taker_pub.to_vec(),
            maker_pub: maker_pub.to_vec(),
            secret_hash: secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::EthCoin(taker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_CONTRACT_ADDRESS_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` {}, found {:?}",
            INVALID_CONTRACT_ADDRESS_ERR_LOG, error
        ),
    }

    // Used to get wrong swap id
    let wrong_secret_hash = dhash160(&MakerSwap::generate_secret().unwrap());
    let error = taker_coin
        .watcher_validate_taker_payment(coins::WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: Vec::new(),
            time_lock,
            taker_pub: taker_pub.to_vec(),
            maker_pub: maker_pub.to_vec(),
            secret_hash: wrong_secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::EthCoin(taker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::UnexpectedPaymentState(err) => {
            assert!(err.contains(INVALID_PAYMENT_STATE_ERR_LOG))
        },
        _ => panic!(
            "Expected `UnexpectedPaymentState` {}, found {:?}",
            INVALID_PAYMENT_STATE_ERR_LOG, error
        ),
    }

    let taker_payment_wrong_secret = taker_coin
        .send_taker_payment(SendPaymentArgs {
            time_lock_duration,
            time_lock,
            other_pubkey: maker_pub,
            secret_hash: wrong_secret_hash.as_slice(),
            amount: taker_amount,
            swap_contract_address: &taker_coin.swap_contract_address(),
            swap_unique_data: &[],
            payment_instructions: &None,
            watcher_reward,
            wait_for_confirmation_until,
        })
        .wait()
        .unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: taker_payment_wrong_secret.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    taker_coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let error = taker_coin
        .watcher_validate_taker_payment(WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: Vec::new(),
            time_lock,
            taker_pub: taker_pub.to_vec(),
            maker_pub: maker_pub.to_vec(),
            secret_hash: wrong_secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::EthCoin(taker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_SWAP_ID_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` {}, found {:?}",
            INVALID_SWAP_ID_ERR_LOG, error
        ),
    }

    let error = taker_coin
        .watcher_validate_taker_payment(WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: Vec::new(),
            time_lock,
            taker_pub: taker_pub.to_vec(),
            maker_pub: taker_pub.to_vec(),
            secret_hash: secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::EthCoin(taker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_RECEIVER_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` {}, found {:?}",
            INVALID_RECEIVER_ERR_LOG, error
        ),
    }
}

#[test]
fn test_watcher_validate_taker_payment_erc20() {
    let timeout = wait_until_sec(120); // timeout if test takes more than 120 seconds to run

    let seed = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();
    let taker_coin = generate_jst_with_seed(&seed);
    let taker_keypair = taker_coin.derive_htlc_key_pair(&[]);
    let taker_pub = taker_keypair.public();

    let maker_seed = get_passphrase!(".env.seed", "BOB_PASSPHRASE").unwrap();
    let maker_keypair = key_pair_from_seed(&maker_seed).unwrap();
    let maker_pub = maker_keypair.public();

    let time_lock_duration = get_payment_locktime();
    let wait_for_confirmation_until = wait_until_sec(time_lock_duration);
    let time_lock = wait_for_confirmation_until;

    let secret_hash = dhash160(&MakerSwap::generate_secret().unwrap());

    let taker_amount = BigDecimal::from_str("0.01").unwrap();
    let maker_amount = BigDecimal::from_str("0.01").unwrap();

    let watcher_reward = Some(
        block_on(taker_coin.get_taker_watcher_reward(
            &MmCoinEnum::from(taker_coin.clone()),
            Some(taker_amount.clone()),
            Some(maker_amount),
            None,
            wait_for_confirmation_until,
        ))
        .unwrap(),
    );

    let taker_payment = taker_coin
        .send_taker_payment(SendPaymentArgs {
            time_lock_duration,
            time_lock,
            other_pubkey: maker_pub,
            secret_hash: secret_hash.as_slice(),
            amount: taker_amount.clone(),
            swap_contract_address: &taker_coin.swap_contract_address(),
            swap_unique_data: &[],
            payment_instructions: &None,
            watcher_reward: watcher_reward.clone(),
            wait_for_confirmation_until,
        })
        .wait()
        .unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: taker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    taker_coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let validate_taker_payment_res = taker_coin
        .watcher_validate_taker_payment(WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: Vec::new(),
            time_lock,
            taker_pub: taker_pub.to_vec(),
            maker_pub: maker_pub.to_vec(),
            secret_hash: secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::EthCoin(taker_coin.clone()),
        })
        .wait();
    assert!(validate_taker_payment_res.is_ok());

    let error = taker_coin
        .watcher_validate_taker_payment(WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: Vec::new(),
            time_lock,
            taker_pub: maker_pub.to_vec(),
            maker_pub: maker_pub.to_vec(),
            secret_hash: secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::EthCoin(taker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_SENDER_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` {}, found {:?}",
            INVALID_SENDER_ERR_LOG, error
        ),
    }

    let taker_payment_wrong_contract = taker_coin
        .send_taker_payment(SendPaymentArgs {
            time_lock_duration,
            time_lock,
            other_pubkey: maker_pub,
            secret_hash: secret_hash.as_slice(),
            amount: taker_amount.clone(),
            swap_contract_address: &Some("9130b257d37a52e52f21054c4da3450c72f595ce".into()),
            swap_unique_data: &[],
            payment_instructions: &None,
            watcher_reward: watcher_reward.clone(),
            wait_for_confirmation_until,
        })
        .wait()
        .unwrap();

    let error = taker_coin
        .watcher_validate_taker_payment(WatcherValidatePaymentInput {
            payment_tx: taker_payment_wrong_contract.tx_hex(),
            taker_payment_refund_preimage: Vec::new(),
            time_lock,
            taker_pub: taker_pub.to_vec(),
            maker_pub: maker_pub.to_vec(),
            secret_hash: secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::EthCoin(taker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_CONTRACT_ADDRESS_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` {}, found {:?}",
            INVALID_CONTRACT_ADDRESS_ERR_LOG, error
        ),
    }

    // Used to get wrong swap id
    let wrong_secret_hash = dhash160(&MakerSwap::generate_secret().unwrap());
    let error = taker_coin
        .watcher_validate_taker_payment(WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: Vec::new(),
            time_lock,
            taker_pub: taker_pub.to_vec(),
            maker_pub: maker_pub.to_vec(),
            secret_hash: wrong_secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::EthCoin(taker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::UnexpectedPaymentState(err) => {
            assert!(err.contains(INVALID_PAYMENT_STATE_ERR_LOG))
        },
        _ => panic!(
            "Expected `UnexpectedPaymentState` {}, found {:?}",
            INVALID_PAYMENT_STATE_ERR_LOG, error
        ),
    }

    let taker_payment_wrong_secret = taker_coin
        .send_taker_payment(SendPaymentArgs {
            time_lock_duration,
            time_lock,
            other_pubkey: maker_pub,
            secret_hash: wrong_secret_hash.as_slice(),
            amount: taker_amount,
            swap_contract_address: &taker_coin.swap_contract_address(),
            swap_unique_data: &[],
            payment_instructions: &None,
            watcher_reward,
            wait_for_confirmation_until,
        })
        .wait()
        .unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: taker_payment_wrong_secret.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    taker_coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let error = taker_coin
        .watcher_validate_taker_payment(WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: Vec::new(),
            time_lock,
            taker_pub: taker_pub.to_vec(),
            maker_pub: maker_pub.to_vec(),
            secret_hash: wrong_secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::EthCoin(taker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_SWAP_ID_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` {}, found {:?}",
            INVALID_SWAP_ID_ERR_LOG, error
        ),
    }

    let error = taker_coin
        .watcher_validate_taker_payment(WatcherValidatePaymentInput {
            payment_tx: taker_payment.tx_hex(),
            taker_payment_refund_preimage: Vec::new(),
            time_lock,
            taker_pub: taker_pub.to_vec(),
            maker_pub: taker_pub.to_vec(),
            secret_hash: secret_hash.to_vec(),
            wait_until: timeout,
            confirmations: 1,
            maker_coin: MmCoinEnum::EthCoin(taker_coin.clone()),
        })
        .wait()
        .unwrap_err()
        .into_inner();
    log!("error: {:?}", error);
    match error {
        ValidatePaymentError::WrongPaymentTx(err) => {
            assert!(err.contains(INVALID_RECEIVER_ERR_LOG))
        },
        _ => panic!(
            "Expected `WrongPaymentTx` {}, found {:?}",
            INVALID_RECEIVER_ERR_LOG, error
        ),
    }
}

#[test]
fn test_send_taker_payment_refund_preimage_utxo() {
    let timeout = wait_until_sec(120); // timeout if test takes more than 120 seconds to run
    let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    let my_public_key = coin.my_public_key().unwrap();

    let time_lock = now_sec() - 3600;
    let taker_payment_args = SendPaymentArgs {
        time_lock_duration: 0,
        time_lock,
        other_pubkey: my_public_key,
        secret_hash: &[0; 20],
        amount: 1u64.into(),
        swap_contract_address: &None,
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let tx = coin.send_taker_payment(taker_payment_args).wait().unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: tx.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let refund_tx = coin
        .create_taker_payment_refund_preimage(&tx.tx_hex(), time_lock, my_public_key, &[0; 20], &None, &[])
        .wait()
        .unwrap();

    let refund_tx = coin
        .send_taker_payment_refund_preimage(RefundPaymentArgs {
            payment_tx: &refund_tx.tx_hex(),
            swap_contract_address: &None,
            secret_hash: &[0; 20],
            other_pubkey: my_public_key,
            time_lock,
            swap_unique_data: &[],
            watcher_reward: false,
        })
        .wait()
        .unwrap();

    let confirm_payment_input = ConfirmPaymentInput {
        payment_tx: refund_tx.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: timeout,
        check_every: 1,
    };
    coin.wait_for_confirmations(confirm_payment_input).wait().unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: coin.my_public_key().unwrap(),
        secret_hash: &[0; 20],
        tx: &tx.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &None,
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let found = block_on(coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();
    assert_eq!(FoundSwapTxSpend::Refunded(refund_tx), found);
}

#[test]
fn test_watcher_reward() {
    let timeout = wait_until_sec(300); // timeout if test takes more than 300 seconds to run
    let (_ctx, utxo_coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    let eth_coin = eth_distributor();

    let watcher_reward =
        block_on(eth_coin.get_taker_watcher_reward(&MmCoinEnum::EthCoin(eth_coin.clone()), None, None, None, timeout))
            .unwrap();
    assert!(!watcher_reward.is_exact_amount);
    assert!(matches!(watcher_reward.reward_target, RewardTarget::Contract));
    assert!(!watcher_reward.send_contract_reward_on_spend);

    let watcher_reward = block_on(eth_coin.get_taker_watcher_reward(
        &MmCoinEnum::EthCoin(eth_coin.clone()),
        None,
        None,
        Some(BigDecimal::one()),
        timeout,
    ))
    .unwrap();
    assert!(watcher_reward.is_exact_amount);
    assert!(matches!(watcher_reward.reward_target, RewardTarget::Contract));
    assert!(!watcher_reward.send_contract_reward_on_spend);
    assert_eq!(watcher_reward.amount, BigDecimal::one());

    let watcher_reward = block_on(eth_coin.get_taker_watcher_reward(
        &MmCoinEnum::UtxoCoin(utxo_coin.clone()),
        None,
        None,
        None,
        timeout,
    ))
    .unwrap();
    assert!(!watcher_reward.is_exact_amount);
    assert!(matches!(watcher_reward.reward_target, RewardTarget::PaymentSender));
    assert!(!watcher_reward.send_contract_reward_on_spend);

    let watcher_reward =
        block_on(eth_coin.get_maker_watcher_reward(&MmCoinEnum::EthCoin(eth_coin.clone()), None, timeout))
            .unwrap()
            .unwrap();
    assert!(!watcher_reward.is_exact_amount);
    assert!(matches!(watcher_reward.reward_target, RewardTarget::None));
    assert!(watcher_reward.send_contract_reward_on_spend);

    let watcher_reward = block_on(eth_coin.get_maker_watcher_reward(
        &MmCoinEnum::EthCoin(eth_coin.clone()),
        Some(BigDecimal::one()),
        timeout,
    ))
    .unwrap()
    .unwrap();
    assert!(watcher_reward.is_exact_amount);
    assert!(matches!(watcher_reward.reward_target, RewardTarget::None));
    assert!(watcher_reward.send_contract_reward_on_spend);
    assert_eq!(watcher_reward.amount, BigDecimal::one());

    let watcher_reward =
        block_on(eth_coin.get_maker_watcher_reward(&MmCoinEnum::UtxoCoin(utxo_coin.clone()), None, timeout))
            .unwrap()
            .unwrap();
    assert!(!watcher_reward.is_exact_amount);
    assert!(matches!(watcher_reward.reward_target, RewardTarget::PaymentSpender));
    assert!(!watcher_reward.send_contract_reward_on_spend);

    let watcher_reward = block_on(utxo_coin.get_taker_watcher_reward(
        &MmCoinEnum::EthCoin(eth_coin),
        Some(BigDecimal::from_str("0.01").unwrap()),
        Some(BigDecimal::from_str("1").unwrap()),
        None,
        timeout,
    ))
    .unwrap();
    assert!(!watcher_reward.is_exact_amount);
    assert!(matches!(watcher_reward.reward_target, RewardTarget::PaymentReceiver));
    assert!(!watcher_reward.send_contract_reward_on_spend);

    let watcher_reward =
        block_on(utxo_coin.get_maker_watcher_reward(&MmCoinEnum::UtxoCoin(utxo_coin.clone()), None, timeout)).unwrap();
    assert!(matches!(watcher_reward, None));
}
