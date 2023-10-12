use std::ops::Neg;
use std::str::FromStr;

use common::{block_on, Future01CompatExt};

use super::*;
use crate::{solana::solana_common_tests::solana_coin_for_test,
            solana::solana_common_tests::{spl_coin_for_test, SolanaNet}};

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn spl_coin_creation() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Testnet);
    let sol_spl_usdc_coin = spl_coin_for_test(
        sol_coin,
        "USDC".to_string(),
        6,
        solana_sdk::pubkey::Pubkey::from_str("CpMah17kQEL2wqyMKt3mZBdTnZbkbfx4nqmQMFDP5vwp").unwrap(),
    );

    println!("address: {}", sol_spl_usdc_coin.my_address().unwrap());
    assert_eq!(
        sol_spl_usdc_coin.my_address().unwrap(),
        "FJktmyjV9aBHEShT4hfnLpr9ELywdwVtEL1w1rSWgbVf"
    );
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_sign_message() {
    let passphrase = "spice describe gravity federal blast come thank unfair canal monkey style afraid".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Testnet);
    let sol_spl_usdc_coin = spl_coin_for_test(
        sol_coin,
        "USDC".to_string(),
        6,
        solana_sdk::pubkey::Pubkey::from_str("CpMah17kQEL2wqyMKt3mZBdTnZbkbfx4nqmQMFDP5vwp").unwrap(),
    );
    let signature = sol_spl_usdc_coin.sign_message("test").unwrap();
    assert_eq!(
        signature,
        "4dzKwEteN8nch76zPMEjPX19RsaQwGTxsbtfg2bwGTkGenLfrdm31zvn9GH5rvaJBwivp6ESXx1KYR672ngs3UfF"
    );
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_verify_message() {
    let passphrase = "spice describe gravity federal blast come thank unfair canal monkey style afraid".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Testnet);
    let sol_spl_usdc_coin = spl_coin_for_test(
        sol_coin,
        "USDC".to_string(),
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
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Testnet);
    let sol_spl_usdc_coin = spl_coin_for_test(
        sol_coin.clone(),
        "USDC".to_string(),
        6,
        solana_sdk::pubkey::Pubkey::from_str("CpMah17kQEL2wqyMKt3mZBdTnZbkbfx4nqmQMFDP5vwp").unwrap(),
    );

    let res = block_on(sol_spl_usdc_coin.my_balance().compat()).unwrap();
    assert_ne!(res.spendable, BigDecimal::from(0));
    assert!(res.spendable < BigDecimal::from(10));

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
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Testnet);
    let usdc_sol_coin = spl_coin_for_test(
        sol_coin,
        "USDC".to_string(),
        6,
        solana_sdk::pubkey::Pubkey::from_str("CpMah17kQEL2wqyMKt3mZBdTnZbkbfx4nqmQMFDP5vwp").unwrap(),
    );
    let withdraw_amount = BigDecimal::from_str("0.0001").unwrap();
    let valid_tx_details = block_on(
        usdc_sol_coin
            .withdraw(WithdrawRequest {
                coin: "USDC".to_string(),
                from: None,
                to: "AYJmtzc9D4KU6xsDzhKShFyYKUNXY622j9QoQEo4LfpX".to_string(),
                amount: withdraw_amount.clone(),
                max: false,
                fee: None,
                memo: None,
            })
            .compat(),
    )
    .unwrap();
    println!("{:?}", valid_tx_details);
    assert_eq!(valid_tx_details.total_amount, withdraw_amount);
    assert_eq!(valid_tx_details.my_balance_change, withdraw_amount.neg());
    assert_eq!(valid_tx_details.coin, "USDC".to_string());
    assert_ne!(valid_tx_details.timestamp, 0);

    let tx_str = hex::encode(&*valid_tx_details.tx_hex.0);
    let res = block_on(usdc_sol_coin.send_raw_tx(&tx_str).compat()).unwrap();
    println!("{:?}", res);

    let res2 = block_on(usdc_sol_coin.send_raw_tx_bytes(&valid_tx_details.tx_hex.0).compat()).unwrap();
    assert_eq!(res, res2);
}
