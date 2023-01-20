use crate::docker_tests::docker_tests_common::{eth_distributor, generate_eth_coin_with_random_privkey,
                                               generate_jst_with_seed};
use crate::integration_tests_common::*;
use crate::{generate_utxo_coin_with_privkey, generate_utxo_coin_with_random_privkey, random_secp256k1_secret,
            SecretKey};
use coins::coin_errors::ValidatePaymentError;
use coins::utxo::UtxoCommonOps;
use coins::{FoundSwapTxSpend, MarketCoinOps, MmCoinEnum, SearchForSwapTxSpendInput, SendTakerPaymentArgs,
            SendWatcherRefundsPaymentArgs, SwapOps, WatcherOps, WatcherValidateTakerFeeInput,
            EARLY_CONFIRMATION_ERR_LOG, INVALID_RECEIVER_ERR_LOG, INVALID_SENDER_ERR_LOG, OLD_TRANSACTION_ERR_LOG};
use common::{block_on, now_ms, DEX_FEE_ADDR_RAW_PUBKEY};
use futures01::Future;
use mm2_main::mm2::lp_swap::{dex_fee_amount_from_taker_coin, get_payment_locktime, MAKER_PAYMENT_SENT_LOG,
                             MAKER_PAYMENT_SPEND_FOUND_LOG, MAKER_PAYMENT_SPEND_SENT_LOG,
                             TAKER_PAYMENT_REFUND_SENT_LOG, WATCHER_MESSAGE_SENT_LOG};
use mm2_number::BigDecimal;
use mm2_number::MmNumber;
use mm2_test_helpers::for_tests::{enable_eth_coin, eth_jst_conf, eth_testnet_conf, mm_dump, my_balance, mycoin1_conf,
                                  mycoin_conf, start_swaps, MarketMakerIt, Mm2TestConf, WatcherConf,
                                  DEFAULT_RPC_PASSWORD, ETH_SEPOLIA_NODE, ETH_SEPOLIA_SWAP_CONTRACT,
                                  ETH_SEPOLIA_TOKEN_CONTRACT};
use std::str::FromStr;
use std::thread;
use std::time::Duration;
use uuid::Uuid;

fn enable_eth_and_jst(mm_node: &MarketMakerIt) {
    dbg!(block_on(enable_eth_coin(
        mm_node,
        "ETH",
        ETH_SEPOLIA_NODE,
        ETH_SEPOLIA_SWAP_CONTRACT,
        Some(ETH_SEPOLIA_SWAP_CONTRACT)
    )));

    dbg!(block_on(enable_eth_coin(
        mm_node,
        "JST",
        ETH_SEPOLIA_NODE,
        ETH_SEPOLIA_SWAP_CONTRACT,
        Some(ETH_SEPOLIA_SWAP_CONTRACT)
    )));
}

#[test]
#[ignore]
fn test_watcher_spends_maker_payment_spend_eth_erc20() {
    let coins = json!([eth_testnet_conf(), eth_jst_conf(ETH_SEPOLIA_TOKEN_CONTRACT)]);

    let alice_passphrase =
        String::from("spice describe gravity federal blast come thank unfair canal monkey style afraid");
    let alice_conf = Mm2TestConf::seednode_using_watchers(&alice_passphrase, &coins);
    let mut mm_alice = MarketMakerIt::start(alice_conf.conf.clone(), alice_conf.rpc_password.clone(), None).unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {}", mm_alice.log_path.display());

    let bob_passphrase = String::from("also shoot benefit prefer juice shell elder veteran woman mimic image kidney");
    let bob_conf = Mm2TestConf::light_node(&bob_passphrase, &coins, &[&mm_alice.ip.to_string()]);
    let mut mm_bob = MarketMakerIt::start(bob_conf.conf, bob_conf.rpc_password, None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {}", mm_bob.log_path.display());

    let watcher_passphrase =
        String::from("also shoot benefit prefer juice shell thank unfair canal monkey style afraid");
    let watcher_conf =
        Mm2TestConf::watcher_light_node(&watcher_passphrase, &coins, &[&mm_alice.ip.to_string()], WatcherConf {
            wait_taker_payment: 0.,
            wait_maker_payment_spend_factor: 0.,
            refund_start_factor: 1.5,
            search_interval: 1.0,
        })
        .conf;
    let mut mm_watcher = MarketMakerIt::start(watcher_conf, DEFAULT_RPC_PASSWORD.to_string(), None).unwrap();
    let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

    enable_eth_and_jst(&mm_alice);
    enable_eth_and_jst(&mm_bob);
    enable_eth_and_jst(&mm_watcher);

    let alice_eth_balance_before = block_on(my_balance(&mm_alice, "ETH")).balance.with_scale(2);
    let alice_jst_balance_before = block_on(my_balance(&mm_alice, "JST")).balance.with_scale(2);
    let bob_eth_balance_before = block_on(my_balance(&mm_bob, "ETH")).balance.with_scale(2);
    let bob_jst_balance_before = block_on(my_balance(&mm_bob, "JST")).balance.with_scale(2);

    block_on(start_swaps(&mut mm_bob, &mut mm_alice, &[("ETH", "JST")], 1., 1., 0.01));

    block_on(mm_alice.wait_for_log(180., |log| log.contains(WATCHER_MESSAGE_SENT_LOG))).unwrap();
    block_on(mm_alice.stop()).unwrap();
    block_on(mm_watcher.wait_for_log(180., |log| log.contains(MAKER_PAYMENT_SPEND_SENT_LOG))).unwrap();
    thread::sleep(Duration::from_secs(25));

    let mm_alice = MarketMakerIt::start(alice_conf.conf.clone(), alice_conf.rpc_password.clone(), None).unwrap();
    enable_eth_and_jst(&mm_alice);

    let alice_eth_balance_after = block_on(my_balance(&mm_alice, "ETH")).balance.with_scale(2);
    let alice_jst_balance_after = block_on(my_balance(&mm_alice, "JST")).balance.with_scale(2);
    let bob_eth_balance_after = block_on(my_balance(&mm_bob, "ETH")).balance.with_scale(2);
    let bob_jst_balance_after = block_on(my_balance(&mm_bob, "JST")).balance.with_scale(2);

    let volume = BigDecimal::from_str("0.01").unwrap();
    assert_eq!(alice_jst_balance_before - volume.clone(), alice_jst_balance_after);
    assert_eq!(bob_jst_balance_before + volume.clone(), bob_jst_balance_after);
    assert_eq!(alice_eth_balance_before + volume.clone(), alice_eth_balance_after);
    assert_eq!(bob_eth_balance_before - volume.clone(), bob_eth_balance_after);
}

#[test]
#[ignore]
fn test_watcher_spends_maker_payment_spend_erc20_eth() {
    let coins = json!([eth_testnet_conf(), eth_jst_conf(ETH_SEPOLIA_TOKEN_CONTRACT)]);

    let alice_passphrase =
        String::from("spice describe gravity federal blast come thank unfair canal monkey style afraid");
    let alice_conf = Mm2TestConf::seednode_using_watchers(&alice_passphrase, &coins);
    let mut mm_alice = MarketMakerIt::start(alice_conf.conf.clone(), alice_conf.rpc_password.clone(), None).unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {}", mm_alice.log_path.display());

    let bob_passphrase = String::from("also shoot benefit prefer juice shell elder veteran woman mimic image kidney");
    let bob_conf = Mm2TestConf::light_node(&bob_passphrase, &coins, &[&mm_alice.ip.to_string()]);
    let mut mm_bob = MarketMakerIt::start(bob_conf.conf, bob_conf.rpc_password, None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {}", mm_bob.log_path.display());

    let watcher_passphrase =
        String::from("also shoot benefit prefer juice shell thank unfair canal monkey style afraid");
    let watcher_conf =
        Mm2TestConf::watcher_light_node(&watcher_passphrase, &coins, &[&mm_alice.ip.to_string()], WatcherConf {
            wait_taker_payment: 0.,
            wait_maker_payment_spend_factor: 0.,
            refund_start_factor: 1.5,
            search_interval: 1.0,
        })
        .conf;

    let mut mm_watcher = MarketMakerIt::start(watcher_conf, DEFAULT_RPC_PASSWORD.to_string(), None).unwrap();
    let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

    enable_eth_and_jst(&mm_alice);
    enable_eth_and_jst(&mm_bob);
    enable_eth_and_jst(&mm_watcher);

    let alice_eth_balance_before = block_on(my_balance(&mm_alice, "ETH")).balance.with_scale(2);
    let alice_jst_balance_before = block_on(my_balance(&mm_alice, "JST")).balance.with_scale(2);
    let bob_eth_balance_before = block_on(my_balance(&mm_bob, "ETH")).balance.with_scale(2);
    let bob_jst_balance_before = block_on(my_balance(&mm_bob, "JST")).balance.with_scale(2);

    block_on(start_swaps(&mut mm_bob, &mut mm_alice, &[("JST", "ETH")], 1., 1., 0.01));

    block_on(mm_alice.wait_for_log(180., |log| log.contains(WATCHER_MESSAGE_SENT_LOG))).unwrap();
    block_on(mm_alice.stop()).unwrap();
    block_on(mm_watcher.wait_for_log(180., |log| log.contains(MAKER_PAYMENT_SPEND_SENT_LOG))).unwrap();
    thread::sleep(Duration::from_secs(25));

    let mm_alice = MarketMakerIt::start(alice_conf.conf.clone(), alice_conf.rpc_password.clone(), None).unwrap();
    enable_eth_and_jst(&mm_alice);

    let alice_eth_balance_after = block_on(my_balance(&mm_alice, "ETH")).balance.with_scale(2);
    let alice_jst_balance_after = block_on(my_balance(&mm_alice, "JST")).balance.with_scale(2);
    let bob_eth_balance_after = block_on(my_balance(&mm_bob, "ETH")).balance.with_scale(2);
    let bob_jst_balance_after = block_on(my_balance(&mm_bob, "JST")).balance.with_scale(2);

    let volume = BigDecimal::from_str("0.01").unwrap();

    assert_eq!(alice_jst_balance_before + volume.clone(), alice_jst_balance_after);
    assert_eq!(bob_jst_balance_before - volume.clone(), bob_jst_balance_after);
    assert_eq!(alice_eth_balance_before - volume.clone(), alice_eth_balance_after);
    assert_eq!(bob_eth_balance_before + volume.clone(), bob_eth_balance_after);
}

#[test]
#[ignore]
fn test_watcher_refunds_taker_payment_erc20() {
    let coins = json!([eth_testnet_conf(), eth_jst_conf(ETH_SEPOLIA_TOKEN_CONTRACT)]);

    let alice_passphrase =
        String::from("spice describe gravity federal blast come thank unfair canal monkey style afraid");
    let alice_conf = Mm2TestConf::seednode_using_watchers(&alice_passphrase, &coins);
    let mut mm_alice = block_on(MarketMakerIt::start_with_envs(
        alice_conf.conf.clone(),
        alice_conf.rpc_password.clone(),
        None,
        &[("USE_TEST_LOCKTIME", "")],
    ))
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {}", mm_alice.log_path.display());

    let bob_passphrase = String::from("also shoot benefit prefer juice shell elder veteran woman mimic image kidney");
    let bob_conf = Mm2TestConf::light_node(&bob_passphrase, &coins, &[&mm_alice.ip.to_string()]);
    let mut mm_bob = MarketMakerIt::start(bob_conf.conf, bob_conf.rpc_password, None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {}", mm_bob.log_path.display());

    let watcher_passphrase =
        String::from("also shoot benefit prefer juice shell thank unfair canal monkey style afraid");
    let watcher_conf =
        Mm2TestConf::watcher_light_node(&watcher_passphrase, &coins, &[&mm_alice.ip.to_string()], WatcherConf {
            wait_taker_payment: 0.,
            wait_maker_payment_spend_factor: 1.,
            refund_start_factor: 0.,
            search_interval: 1.,
        })
        .conf;
    let mut mm_watcher = block_on(MarketMakerIt::start_with_envs(
        watcher_conf,
        DEFAULT_RPC_PASSWORD.to_string(),
        None,
        &[("REFUND_TEST", "")],
    ))
    .unwrap();
    let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

    enable_eth_and_jst(&mm_alice);
    enable_eth_and_jst(&mm_bob);
    enable_eth_and_jst(&mm_watcher);

    let alice_eth_balance_before = block_on(my_balance(&mm_alice, "ETH")).balance.with_scale(2);
    let alice_jst_balance_before = block_on(my_balance(&mm_alice, "JST")).balance.with_scale(2);

    block_on(start_swaps(&mut mm_bob, &mut mm_alice, &[("ETH", "JST")], 1., 1., 0.01));

    block_on(mm_bob.wait_for_log(160., |log| log.contains(MAKER_PAYMENT_SENT_LOG))).unwrap();
    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.wait_for_log(160., |log| log.contains(WATCHER_MESSAGE_SENT_LOG))).unwrap();
    block_on(mm_alice.stop()).unwrap();
    block_on(mm_watcher.wait_for_log(160., |log| log.contains(TAKER_PAYMENT_REFUND_SENT_LOG))).unwrap();
    thread::sleep(Duration::from_secs(25));

    let mm_alice = MarketMakerIt::start(alice_conf.conf, alice_conf.rpc_password, None).unwrap();
    enable_eth_and_jst(&mm_alice);

    let alice_eth_balance_after = block_on(my_balance(&mm_alice, "ETH")).balance.with_scale(2);
    let alice_jst_balance_after = block_on(my_balance(&mm_alice, "JST")).balance.with_scale(2);

    assert_eq!(alice_jst_balance_before, alice_jst_balance_after);
    assert_eq!(alice_eth_balance_before, alice_eth_balance_after);
}

#[test]
#[ignore]
fn test_watcher_refunds_taker_payment_eth() {
    let coins = json!([eth_testnet_conf(), eth_jst_conf(ETH_SEPOLIA_TOKEN_CONTRACT)]);

    let alice_passphrase =
        String::from("spice describe gravity federal blast come thank unfair canal monkey style afraid");
    let alice_conf = Mm2TestConf::seednode_using_watchers(&alice_passphrase, &coins);
    let mut mm_alice = block_on(MarketMakerIt::start_with_envs(
        alice_conf.conf.clone(),
        alice_conf.rpc_password.clone(),
        None,
        &[("USE_TEST_LOCKTIME", "")],
    ))
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_alice.mm_dump();
    log!("Alice log path: {}", mm_alice.log_path.display());

    let bob_passphrase = String::from("also shoot benefit prefer juice shell elder veteran woman mimic image kidney");
    let bob_conf = Mm2TestConf::light_node(&bob_passphrase, &coins, &[&mm_alice.ip.to_string()]);
    let mut mm_bob = MarketMakerIt::start(bob_conf.conf, bob_conf.rpc_password, None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_bob.mm_dump();
    log!("Bob log path: {}", mm_bob.log_path.display());

    let watcher_passphrase =
        String::from("also shoot benefit prefer juice shell thank unfair canal monkey style afraid");
    let watcher_conf =
        Mm2TestConf::watcher_light_node(&watcher_passphrase, &coins, &[&mm_alice.ip.to_string()], WatcherConf {
            wait_taker_payment: 0.,
            wait_maker_payment_spend_factor: 1.,
            refund_start_factor: 0.,
            search_interval: 1.,
        })
        .conf;
    let mut mm_watcher = block_on(MarketMakerIt::start_with_envs(
        watcher_conf,
        DEFAULT_RPC_PASSWORD.to_string(),
        None,
        &[("REFUND_TEST", "")],
    ))
    .unwrap();
    let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

    enable_eth_and_jst(&mm_alice);
    enable_eth_and_jst(&mm_bob);
    enable_eth_and_jst(&mm_watcher);

    let alice_eth_balance_before = block_on(my_balance(&mm_alice, "ETH")).balance.with_scale(2);
    let alice_jst_balance_before = block_on(my_balance(&mm_alice, "JST")).balance.with_scale(2);

    block_on(start_swaps(&mut mm_bob, &mut mm_alice, &[("JST", "ETH")], 1., 1., 0.01));

    block_on(mm_bob.wait_for_log(160., |log| log.contains(MAKER_PAYMENT_SENT_LOG))).unwrap();
    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.wait_for_log(160., |log| log.contains(WATCHER_MESSAGE_SENT_LOG))).unwrap();
    block_on(mm_alice.stop()).unwrap();
    block_on(mm_watcher.wait_for_log(160., |log| log.contains(TAKER_PAYMENT_REFUND_SENT_LOG))).unwrap();
    thread::sleep(Duration::from_secs(25));

    let mm_alice = MarketMakerIt::start(alice_conf.conf, alice_conf.rpc_password, None).unwrap();
    enable_eth_and_jst(&mm_alice);

    let alice_eth_balance_after = block_on(my_balance(&mm_alice, "ETH")).balance.with_scale(2);
    let alice_jst_balance_after = block_on(my_balance(&mm_alice, "JST")).balance.with_scale(2);

    assert_eq!(alice_jst_balance_before, alice_jst_balance_after);
    assert_eq!(alice_eth_balance_before, alice_eth_balance_after);
}

#[test]
fn test_watcher_validate_taker_fee_eth() {
    let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
    let lock_duration = get_payment_locktime();

    let taker_coin = eth_distributor();
    let taker_keypair = taker_coin.derive_htlc_key_pair(&[]);
    let taker_pubkey = taker_keypair.public();

    let maker_coin = generate_eth_coin_with_random_privkey();
    let maker_keypair = maker_coin.derive_htlc_key_pair(&[]);
    let maker_pubkey = maker_keypair.public();

    let taker_amount = MmNumber::from((10, 1));
    let fee_amount = dex_fee_amount_from_taker_coin(
        &MmCoinEnum::EthCoin(taker_coin.clone()),
        maker_coin.ticker(),
        &taker_amount,
    );
    let taker_fee = taker_coin
        .send_taker_fee(
            &DEX_FEE_ADDR_RAW_PUBKEY,
            fee_amount.clone().into(),
            Uuid::new_v4().as_bytes(),
        )
        .wait()
        .unwrap();

    taker_coin
        .wait_for_confirmations(&taker_fee.tx_hex(), 1, false, timeout, 1)
        .wait()
        .unwrap();

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
            sender_pubkey: maker_pubkey.to_vec(),
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
    let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
    let lock_duration = get_payment_locktime();

    let seed = String::from("spice describe gravity federal blast come thank unfair canal monkey style afraid");
    let taker_coin = generate_jst_with_seed(&seed);
    let taker_keypair = taker_coin.derive_htlc_key_pair(&[]);
    let taker_pubkey = taker_keypair.public();

    let maker_coin = generate_eth_coin_with_random_privkey();
    let maker_keypair = maker_coin.derive_htlc_key_pair(&[]);
    let maker_pubkey = maker_keypair.public();

    let taker_amount = MmNumber::from((10, 1));
    let fee_amount = dex_fee_amount_from_taker_coin(
        &MmCoinEnum::EthCoin(taker_coin.clone()),
        maker_coin.ticker(),
        &taker_amount,
    );
    let taker_fee = taker_coin
        .send_taker_fee(
            &DEX_FEE_ADDR_RAW_PUBKEY,
            fee_amount.clone().into(),
            Uuid::new_v4().as_bytes(),
        )
        .wait()
        .unwrap();

    taker_coin
        .wait_for_confirmations(&taker_fee.tx_hex(), 1, false, timeout, 1)
        .wait()
        .unwrap();

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
            sender_pubkey: maker_pubkey.to_vec(),
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
fn test_watcher_spends_maker_payment_spend_utxo() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 100.into());
    generate_utxo_coin_with_privkey("MYCOIN1", 100.into(), bob_priv_key);
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 100.into());
    generate_utxo_coin_with_privkey("MYCOIN", 100.into(), alice_priv_key);

    let watcher_priv_key = random_secp256k1_secret();

    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);

    let alice_conf = Mm2TestConf::seednode_using_watchers(&format!("0x{}", hex::encode(alice_priv_key)), &coins).conf;
    let mut mm_alice = MarketMakerIt::start(alice_conf.clone(), DEFAULT_RPC_PASSWORD.to_string(), None).unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    let bob_conf = Mm2TestConf::light_node(&format!("0x{}", hex::encode(bob_priv_key)), &coins, &[&mm_alice
        .ip
        .to_string()])
    .conf;
    let mut mm_bob = MarketMakerIt::start(bob_conf, DEFAULT_RPC_PASSWORD.to_string(), None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let watcher_conf = Mm2TestConf::watcher_light_node(
        &format!("0x{}", hex::encode(watcher_priv_key)),
        &coins,
        &[&mm_alice.ip.to_string()],
        WatcherConf {
            wait_taker_payment: 0.,
            wait_maker_payment_spend_factor: 0.,
            refund_start_factor: 1.5,
            search_interval: 1.0,
        },
    )
    .conf;
    let mut mm_watcher = MarketMakerIt::start(watcher_conf, DEFAULT_RPC_PASSWORD.to_string(), None).unwrap();
    let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[])));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[])));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[])));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[])));
    log!("{:?}", block_on(enable_native(&mm_watcher, "MYCOIN", &[])));
    log!("{:?}", block_on(enable_native(&mm_watcher, "MYCOIN1", &[])));

    block_on(start_swaps(
        &mut mm_bob,
        &mut mm_alice,
        &[("MYCOIN", "MYCOIN1")],
        25.,
        25.,
        2.,
    ));
    block_on(mm_alice.wait_for_log(60., |log| log.contains(WATCHER_MESSAGE_SENT_LOG))).unwrap();
    block_on(mm_alice.stop()).unwrap();
    block_on(mm_watcher.wait_for_log(60., |log| log.contains(MAKER_PAYMENT_SPEND_SENT_LOG))).unwrap();
    thread::sleep(Duration::from_secs(5));

    let mm_alice = MarketMakerIt::start(alice_conf, DEFAULT_RPC_PASSWORD.to_string(), None).unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[])));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[])));

    assert_eq!(
        block_on(my_balance(&mm_alice, "MYCOIN1")).balance,
        BigDecimal::from_str("49.93562994").unwrap()
    );
    assert_eq!(
        block_on(my_balance(&mm_alice, "MYCOIN")).balance,
        BigDecimal::from_str("101.99999").unwrap()
    );
    assert_eq!(
        block_on(my_balance(&mm_bob, "MYCOIN1")).balance,
        BigDecimal::from_str("149.99999").unwrap()
    );
    assert_eq!(
        block_on(my_balance(&mm_bob, "MYCOIN")).balance,
        BigDecimal::from_str("97.99999").unwrap()
    );
}

#[test]
fn test_watcher_waits_for_taker_utxo() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 100.into());
    generate_utxo_coin_with_privkey("MYCOIN1", 100.into(), bob_priv_key);
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 100.into());
    generate_utxo_coin_with_privkey("MYCOIN", 100.into(), alice_priv_key);
    let watcher_priv_key = *SecretKey::new(&mut rand6::thread_rng()).as_ref();

    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);

    let alice_conf = Mm2TestConf::seednode_using_watchers(&format!("0x{}", hex::encode(alice_priv_key)), &coins).conf;
    let mut mm_alice = MarketMakerIt::start(alice_conf.clone(), DEFAULT_RPC_PASSWORD.to_string(), None).unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    let bob_conf = Mm2TestConf::light_node(&format!("0x{}", hex::encode(bob_priv_key)), &coins, &[&mm_alice
        .ip
        .to_string()])
    .conf;
    let mut mm_bob = MarketMakerIt::start(bob_conf, DEFAULT_RPC_PASSWORD.to_string(), None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let watcher_conf = Mm2TestConf::watcher_light_node(
        &format!("0x{}", hex::encode(watcher_priv_key)),
        &coins,
        &[&mm_alice.ip.to_string()],
        WatcherConf {
            wait_taker_payment: 0.,
            wait_maker_payment_spend_factor: 1.,
            refund_start_factor: 1.5,
            search_interval: 1.,
        },
    )
    .conf;
    let mut mm_watcher = MarketMakerIt::start(watcher_conf, DEFAULT_RPC_PASSWORD.to_string(), None).unwrap();
    let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[])));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[])));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[])));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[])));
    log!("{:?}", block_on(enable_native(&mm_watcher, "MYCOIN", &[])));
    log!("{:?}", block_on(enable_native(&mm_watcher, "MYCOIN1", &[])));

    block_on(start_swaps(
        &mut mm_bob,
        &mut mm_alice,
        &[("MYCOIN", "MYCOIN1")],
        25.,
        25.,
        2.,
    ));
    block_on(mm_watcher.wait_for_log(160., |log| log.contains(MAKER_PAYMENT_SPEND_FOUND_LOG))).unwrap();
}

#[test]
fn test_watcher_refunds_taker_payment_utxo() {
    let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 100.into());
    generate_utxo_coin_with_privkey("MYCOIN1", 100.into(), bob_priv_key);
    let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 100.into());
    generate_utxo_coin_with_privkey("MYCOIN", 100.into(), alice_priv_key);
    let watcher_priv_key = *SecretKey::new(&mut rand6::thread_rng()).as_ref();

    let coins = json!([mycoin_conf(1000), mycoin1_conf(1000)]);

    let alice_conf = Mm2TestConf::seednode_using_watchers(&format!("0x{}", hex::encode(alice_priv_key)), &coins).conf;
    let mut mm_alice = block_on(MarketMakerIt::start_with_envs(
        alice_conf.clone(),
        DEFAULT_RPC_PASSWORD.to_string(),
        None,
        &[("USE_TEST_LOCKTIME", "")],
    ))
    .unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

    let bob_conf = Mm2TestConf::light_node(&format!("0x{}", hex::encode(bob_priv_key)), &coins, &[&mm_alice
        .ip
        .to_string()])
    .conf;
    let mut mm_bob = MarketMakerIt::start(bob_conf, DEFAULT_RPC_PASSWORD.to_string(), None).unwrap();
    let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

    let watcher_conf = Mm2TestConf::watcher_light_node(
        &format!("0x{}", hex::encode(watcher_priv_key)),
        &coins,
        &[&mm_alice.ip.to_string()],
        WatcherConf {
            wait_taker_payment: 0.,
            wait_maker_payment_spend_factor: 1.,
            refund_start_factor: 0.,
            search_interval: 1.,
        },
    )
    .conf;
    let mut mm_watcher = block_on(MarketMakerIt::start_with_envs(
        watcher_conf,
        DEFAULT_RPC_PASSWORD.to_string(),
        None,
        &[("REFUND_TEST", "")],
    ))
    .unwrap();
    let (_watcher_dump_log, _watcher_dump_dashboard) = mm_dump(&mm_watcher.log_path);

    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[])));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[])));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[])));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[])));
    log!("{:?}", block_on(enable_native(&mm_watcher, "MYCOIN", &[])));
    log!("{:?}", block_on(enable_native(&mm_watcher, "MYCOIN1", &[])));

    block_on(start_swaps(
        &mut mm_bob,
        &mut mm_alice,
        &[("MYCOIN", "MYCOIN1")],
        25.,
        25.,
        2.,
    ));
    block_on(mm_bob.wait_for_log(160., |log| log.contains(MAKER_PAYMENT_SENT_LOG))).unwrap();
    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.wait_for_log(160., |log| log.contains(WATCHER_MESSAGE_SENT_LOG))).unwrap();
    block_on(mm_alice.stop()).unwrap();
    block_on(mm_watcher.wait_for_log(160., |log| log.contains(TAKER_PAYMENT_REFUND_SENT_LOG))).unwrap();
    thread::sleep(Duration::from_secs(5));

    let mm_alice = MarketMakerIt::start(alice_conf, DEFAULT_RPC_PASSWORD.to_string(), None).unwrap();
    let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[])));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[])));

    assert_eq!(
        block_on(my_balance(&mm_alice, "MYCOIN1")).balance,
        BigDecimal::from_str("99.93561994").unwrap()
    );
    assert_eq!(
        block_on(my_balance(&mm_alice, "MYCOIN")).balance,
        BigDecimal::from_str("100").unwrap()
    );
}

#[test]
fn test_watcher_validate_taker_fee_utxo() {
    let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
    let lock_duration = get_payment_locktime();
    let (_ctx, taker_coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    let (_ctx, maker_coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    let taker_pubkey = taker_coin.my_public_key().unwrap();

    let taker_amount = MmNumber::from((10, 1));
    let fee_amount = dex_fee_amount_from_taker_coin(
        &MmCoinEnum::UtxoCoin(taker_coin.clone()),
        maker_coin.ticker(),
        &taker_amount,
    );

    let taker_fee = taker_coin
        .send_taker_fee(
            &DEX_FEE_ADDR_RAW_PUBKEY,
            fee_amount.clone().into(),
            Uuid::new_v4().as_bytes(),
        )
        .wait()
        .unwrap();

    taker_coin
        .wait_for_confirmations(&taker_fee.tx_hex(), 1, false, timeout, 1)
        .wait()
        .unwrap();

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
fn test_send_taker_payment_refund_preimage_utxo() {
    let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
    let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
    let my_public_key = coin.my_public_key().unwrap();

    let time_lock = (now_ms() / 1000) as u32 - 3600;
    let taker_payment_args = SendTakerPaymentArgs {
        time_lock_duration: 0,
        time_lock,
        other_pubkey: my_public_key,
        secret_hash: &[0; 20],
        amount: 1u64.into(),
        swap_contract_address: &None,
        swap_unique_data: &[],
        payment_instructions: &None,
    };
    let tx = coin.send_taker_payment(taker_payment_args).wait().unwrap();

    coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1)
        .wait()
        .unwrap();

    let refund_tx = coin
        .create_taker_payment_refund_preimage(&tx.tx_hex(), time_lock, my_public_key, &[0; 20], &None, &[])
        .wait()
        .unwrap();

    let refund_tx = coin
        .send_taker_payment_refund_preimage(SendWatcherRefundsPaymentArgs {
            payment_tx: &refund_tx.tx_hex(),
            swap_contract_address: &None,
            secret_hash: &[0; 20],
            other_pubkey: my_public_key,
            time_lock,
            swap_unique_data: &[],
        })
        .wait()
        .unwrap();

    coin.wait_for_confirmations(&refund_tx.tx_hex(), 1, false, timeout, 1)
        .wait()
        .unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: &*coin.my_public_key().unwrap(),
        secret_hash: &[0; 20],
        tx: &tx.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &None,
        swap_unique_data: &[],
    };
    let found = block_on(coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();
    assert_eq!(FoundSwapTxSpend::Refunded(refund_tx), found);
}
