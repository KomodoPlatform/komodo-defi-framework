use crate::docker_tests::{docker_tests_common::SOL_USDC_PUBKEY,
                          solana_common_tests::{solana_coin_for_test, spl_coin_for_test, SolanaNet, ACCOUNT_PUBKEY,
                                                SOL_ADDITIONAL_PASSPHRASE, SOL_PASSPHRASE}};
use coins::{solana::solana_sdk, MarketCoinOps, MmCoin, WithdrawRequest};
use common::{block_on, Future01CompatExt};
use mm2_number::BigDecimal;
use std::{env, ops::Neg, str::FromStr};

const USDC: &str = "USDC";

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn spl_coin_creation() {
    let (_, sol_coin) = solana_coin_for_test(SOL_PASSPHRASE.to_owned(), SolanaNet::Local);
    let sol_spl_usdc_coin = spl_coin_for_test(
        sol_coin,
        USDC.to_string(),
        6,
        solana_sdk::pubkey::Pubkey::from_str("3e9KpjwQejx9Y7WkfaXJTybH6ecG7AdXoAoxk279hdFh").unwrap(),
    );

    log!("address: {}", sol_spl_usdc_coin.my_address().unwrap());
    assert_eq!(sol_spl_usdc_coin.my_address().unwrap(), ACCOUNT_PUBKEY);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_sign_message() {
    let (_, sol_coin) = solana_coin_for_test(SOL_ADDITIONAL_PASSPHRASE.to_owned(), SolanaNet::Local);
    let sol_spl_usdc_coin = spl_coin_for_test(
        sol_coin,
        USDC.to_string(),
        6,
        solana_sdk::pubkey::Pubkey::from_str("CpMah17kQEL2wqyMKt3mZBdTnZbkbfx4nqmQMFDP5vwp").unwrap(),
    );
    let signature = sol_spl_usdc_coin.sign_message("TEST").unwrap();
    assert_eq!(
        signature,
        "4dzKwEteN8nch76zPMEjPX19RsaQwGTxsbtfg2bwGTkGenLfrdm31zvn9GH5rvaJBwivp6ESXx1KYR672ngs3UfF"
    );
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_verify_message() {
    let (_, sol_coin) = solana_coin_for_test(SOL_ADDITIONAL_PASSPHRASE.to_owned(), SolanaNet::Local);
    let sol_spl_usdc_coin = spl_coin_for_test(
        sol_coin,
        USDC.to_string(),
        6,
        solana_sdk::pubkey::Pubkey::from_str("CpMah17kQEL2wqyMKt3mZBdTnZbkbfx4nqmQMFDP5vwp").unwrap(),
    );
    let is_valid = sol_spl_usdc_coin
        .verify_message(
            "4dzKwEteN8nch76zPMEjPX19RsaQwGTxsbtfg2bwGTkGenLfrdm31zvn9GH5rvaJBwivp6ESXx1KYR672ngs3UfF",
            "test",
            "8UF6jSVE1jW8mSiGqt8Hft1rLwPjdKLaTfhkNozFwoAG",
        )
        .unwrap();
    assert!(is_valid);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn spl_my_balance() {
    let adex_token_address = env::var("ADEX_TOKEN_ADDRESS").expect("ADEX_TOKEN_ADDRESS not set");
    let (_, sol_coin) = solana_coin_for_test(SOL_PASSPHRASE.to_owned(), SolanaNet::Local);
    let sol_spl_adex_coin = spl_coin_for_test(
        sol_coin.clone(),
        "ADEX".to_string(),
        9,
        solana_sdk::pubkey::Pubkey::from_str(&adex_token_address).unwrap(),
    );

    let res = block_on(sol_spl_adex_coin.my_balance().compat()).unwrap();
    assert_ne!(res.spendable, BigDecimal::from(0));
    assert!(res.spendable < BigDecimal::from(1000000001));

    let sol_spl_wsol_coin = spl_coin_for_test(
        sol_coin,
        "WSOL".to_string(),
        8,
        solana_sdk::pubkey::Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap(),
    );
    let res = block_on(sol_spl_wsol_coin.my_balance().compat()).unwrap();
    assert_eq!(res.spendable, BigDecimal::from(0));
}

// Stop ignoring when Solana is released
#[test]
#[ignore]
#[cfg(not(target_arch = "wasm32"))]
fn test_spl_transactions() {
    let (_, sol_coin) = solana_coin_for_test(SOL_PASSPHRASE.to_owned(), SolanaNet::Local);
    let usdc_sol_coin = spl_coin_for_test(
        sol_coin,
        USDC.to_string(),
        6,
        solana_sdk::pubkey::Pubkey::from_str(SOL_USDC_PUBKEY).unwrap(),
    );
    let withdraw_amount = BigDecimal::from_str("0.0001").unwrap();
    let valid_tx_details = block_on(
        usdc_sol_coin
            .withdraw(WithdrawRequest::new(
                USDC.to_string(),
                None,
                "AYJmtzc9D4KU6xsDzhKShFyYKUNXY622j9QoQEo4LfpX".to_string(),
                withdraw_amount.clone(),
                false,
                None,
                None,
            ))
            .compat(),
    )
    .unwrap();
    log!("{:?}", valid_tx_details);
    assert_eq!(valid_tx_details.total_amount, withdraw_amount);
    assert_eq!(valid_tx_details.my_balance_change, withdraw_amount.neg());
    assert_eq!(valid_tx_details.coin, USDC.to_string());
    assert_ne!(valid_tx_details.timestamp, 0);

    let tx_str = hex::encode(&*valid_tx_details.tx.tx_hex().unwrap().0);
    let res = block_on(usdc_sol_coin.send_raw_tx(&tx_str).compat()).unwrap();
    log!("{:?}", res);

    let res2 = block_on(
        usdc_sol_coin
            .send_raw_tx_bytes(&valid_tx_details.tx.tx_hex().unwrap().0)
            .compat(),
    )
    .unwrap();
    assert_eq!(res, res2);
}
