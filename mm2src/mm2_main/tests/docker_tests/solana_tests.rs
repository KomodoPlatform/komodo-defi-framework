use crate::docker_tests::docker_tests_common::*;
use bitcrypto::sha256;
use coins::{solana::{solana_common::lamports_to_sol,
                     solana_common_tests::{generate_key_pair_from_iguana_seed, solana_coin_for_test, SolanaNet},
                     solana_sdk::{bs58, pubkey::Pubkey, signer::Signer}},
            MarketCoinOps, MmCoin, RefundPaymentArgs, SendPaymentArgs, SpendPaymentArgs, SwapOps,
            SwapTxTypeWithSecretHash, WithdrawError, WithdrawRequest};
use common::{block_on, Future01CompatExt};
use futures01::Future;
use mm2_number::{bigdecimal::Zero, BigDecimal};
use mm2_test_helpers::{for_tests::{disable_coin, enable_solana_with_tokens, enable_spl, sign_message, verify_message},
                       structs::{EnableSolanaWithTokensResponse, EnableSplResponse, RpcV2Response, SignatureResponse,
                                 VerificationResponse}};
use rpc::v1::types::Bytes;
use serde_json as json;
use std::{convert::TryFrom, ops::Neg, str::FromStr};

const SOLANA_CLIENT_URL: &str = "http://localhost:8899";

#[test]
fn test_solana_and_spl_balance_enable_spl_v2() {
    let mm = _solana_supplied_node();
    let tx_history = false;
    let enable_solana_with_tokens = block_on(enable_solana_with_tokens(
        &mm,
        "SOL-DEVNET",
        &["USDC-SOL-DEVNET"],
        SOLANA_CLIENT_URL,
        tx_history,
    ));
    let enable_solana_with_tokens: RpcV2Response<EnableSolanaWithTokensResponse> =
        json::from_value(enable_solana_with_tokens).unwrap();

    let (_, solana_balance) = enable_solana_with_tokens
        .result
        .solana_addresses_infos
        .into_iter()
        .next()
        .unwrap();
    assert!(solana_balance.balances.unwrap().spendable > 0.into());

    let spl_balances = enable_solana_with_tokens
        .result
        .spl_addresses_infos
        .into_iter()
        .next()
        .unwrap()
        .1
        .balances
        .unwrap();
    let usdc_spl = spl_balances.get("USDC-SOL-DEVNET").unwrap();
    assert!(usdc_spl.spendable.is_zero());

    let enable_spl = block_on(enable_spl(&mm, "ADEX-SOL-DEVNET"));
    let enable_spl: RpcV2Response<EnableSplResponse> = json::from_value(enable_spl).unwrap();
    assert_eq!(1, enable_spl.result.balances.len());

    let (_, balance) = enable_spl.result.balances.into_iter().next().unwrap();
    assert!(balance.spendable > 0.into());
}

#[test]
fn test_sign_verify_message_solana() {
    let mm = _solana_supplied_node();
    let tx_history = false;
    block_on(enable_solana_with_tokens(
        &mm,
        "SOL-DEVNET",
        &["USDC-SOL-DEVNET"],
        SOLANA_CLIENT_URL,
        tx_history,
    ));

    let response = block_on(sign_message(&mm, "SOL-DEVNET"));
    let response: RpcV2Response<SignatureResponse> = json::from_value(response).unwrap();
    let response = response.result;

    assert_eq!(
        response.signature,
        "3AoWCXHq3ACYHYEHUsCzPmRNiXn5c6kodXn9KDd1tz52e1da3dZKYXD5nrJW31XLtN6zzJiwHWtDta52w7Cd7qyE"
    );

    let response = block_on(verify_message(
        &mm,
        "SOL-DEVNET",
        "3AoWCXHq3ACYHYEHUsCzPmRNiXn5c6kodXn9KDd1tz52e1da3dZKYXD5nrJW31XLtN6zzJiwHWtDta52w7Cd7qyE",
        "FJktmyjV9aBHEShT4hfnLpr9ELywdwVtEL1w1rSWgbVf",
    ));
    let response: RpcV2Response<VerificationResponse> = json::from_value(response).unwrap();
    let response = response.result;

    assert!(response.is_valid);
}

#[test]
fn test_sign_verify_message_spl() {
    let mm = _solana_supplied_node();
    let tx_history = false;
    block_on(enable_solana_with_tokens(
        &mm,
        "SOL-DEVNET",
        &["USDC-SOL-DEVNET"],
        SOLANA_CLIENT_URL,
        tx_history,
    ));

    block_on(enable_spl(&mm, "ADEX-SOL-DEVNET"));

    let response = block_on(sign_message(&mm, "ADEX-SOL-DEVNET"));
    let response: RpcV2Response<SignatureResponse> = json::from_value(response).unwrap();
    let response = response.result;

    assert_eq!(
        response.signature,
        "3AoWCXHq3ACYHYEHUsCzPmRNiXn5c6kodXn9KDd1tz52e1da3dZKYXD5nrJW31XLtN6zzJiwHWtDta52w7Cd7qyE"
    );

    let response = block_on(verify_message(
        &mm,
        "ADEX-SOL-DEVNET",
        "3AoWCXHq3ACYHYEHUsCzPmRNiXn5c6kodXn9KDd1tz52e1da3dZKYXD5nrJW31XLtN6zzJiwHWtDta52w7Cd7qyE",
        "FJktmyjV9aBHEShT4hfnLpr9ELywdwVtEL1w1rSWgbVf",
    ));
    let response: RpcV2Response<VerificationResponse> = json::from_value(response).unwrap();
    let response = response.result;

    assert!(response.is_valid);
}

#[test]
fn test_disable_solana_platform_coin_with_tokens() {
    let mm = _solana_supplied_node();
    block_on(enable_solana_with_tokens(
        &mm,
        "SOL-DEVNET",
        &["USDC-SOL-DEVNET"],
        SOLANA_CLIENT_URL,
        false,
    ));
    block_on(enable_spl(&mm, "ADEX-SOL-DEVNET"));

    // Try to passive platform coin, SOL-DEVNET.
    let res = block_on(disable_coin(&mm, "SOL-DEVNET", false));
    assert!(res.passivized);

    // Then try to force disable SOL-DEVNET platform coin.
    let res = block_on(disable_coin(&mm, "SOL-DEVNET", true));
    assert!(!res.passivized);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn solana_keypair_from_secp() {
    let solana_key_pair = generate_key_pair_from_iguana_seed("federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string());
    assert_eq!(
        "FJktmyjV9aBHEShT4hfnLpr9ELywdwVtEL1w1rSWgbVf",
        solana_key_pair.pubkey().to_string()
    );

    let other_solana_keypair = generate_key_pair_from_iguana_seed("bob passphrase".to_string());
    assert_eq!(
        "B7KMMHyc3eYguUMneXRznY1NWh91HoVA2muVJetstYKE",
        other_solana_keypair.pubkey().to_string()
    );
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn solana_transaction_simulation() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Local);
    let request_amount = BigDecimal::try_from(0.0001).unwrap();
    let valid_tx_details = block_on(
        sol_coin
            .withdraw(WithdrawRequest::new(
                "SOL".to_string(),
                None,
                sol_coin.my_address.clone(),
                request_amount.clone(),
                false,
                None,
                None,
            ))
            .compat(),
    )
    .unwrap();
    let (_, fees) = block_on(sol_coin.estimate_withdraw_fees()).unwrap();
    let sol_required = lamports_to_sol(fees);
    let expected_spent_by_me = &request_amount + &sol_required;
    assert_eq!(valid_tx_details.spent_by_me, expected_spent_by_me);
    assert_eq!(valid_tx_details.received_by_me, request_amount);
    assert_eq!(valid_tx_details.total_amount, expected_spent_by_me);
    assert_eq!(valid_tx_details.my_balance_change, sol_required.neg());
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn solana_transaction_zero_balance() {
    let passphrase = "fake passphrase".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Local);
    let invalid_tx_details = block_on(
        sol_coin
            .withdraw(WithdrawRequest::new(
                "SOL".to_string(),
                None,
                sol_coin.my_address.clone(),
                BigDecimal::from_str("0.000001").unwrap(),
                false,
                None,
                None,
            ))
            .compat(),
    );
    let error = invalid_tx_details.unwrap_err();
    let (_, fees) = block_on(sol_coin.estimate_withdraw_fees()).unwrap();
    let sol_required = lamports_to_sol(fees);
    match error.into_inner() {
        WithdrawError::NotSufficientBalance { required, .. } => {
            assert_eq!(required, sol_required);
        },
        e => panic!("Unexpected err {:?}", e),
    };
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn solana_transaction_simulations_not_enough_for_fees() {
    let passphrase = "non existent passphrase".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Local);
    let invalid_tx_details = block_on(
        sol_coin
            .withdraw(WithdrawRequest::new(
                "SOL".to_string(),
                None,
                sol_coin.my_address.clone(),
                BigDecimal::from(1),
                false,
                None,
                None,
            ))
            .compat(),
    );
    let error = invalid_tx_details.unwrap_err();
    let (_, fees) = block_on(sol_coin.estimate_withdraw_fees()).unwrap();
    let sol_required = lamports_to_sol(fees);
    match error.into_inner() {
        WithdrawError::NotSufficientBalance {
            coin: _,
            available,
            required,
        } => {
            assert_eq!(available, 0.into());
            assert_eq!(required, sol_required);
        },
        e => panic!("Unexpected err {:?}", e),
    };
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn solana_transaction_simulations_max() {
    let passphrase = "non existent passphrase".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Local);
    let invalid_tx_details = block_on(
        sol_coin
            .withdraw(WithdrawRequest::new(
                "SOL".to_string(),
                None,
                sol_coin.my_address.clone(),
                BigDecimal::from(1),
                false,
                None,
                None,
            ))
            .compat(),
    );
    let error = invalid_tx_details.unwrap_err();
    let (_, fees) = block_on(sol_coin.estimate_withdraw_fees()).unwrap();
    let sol_required = lamports_to_sol(fees);
    match error.into_inner() {
        WithdrawError::NotSufficientBalance {
            coin: _,
            available,
            required,
        } => {
            assert_eq!(available, 0.into());
            assert_eq!(required, sol_required);
        },
        e => panic!("Unexpected err {:?}", e),
    };
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn solana_test_transactions() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Local);
    let valid_tx_details = block_on(
        sol_coin
            .withdraw(WithdrawRequest::new(
                "SOL".to_string(),
                None,
                sol_coin.my_address.clone(),
                BigDecimal::try_from(0.0001).unwrap(),
                false,
                None,
                None,
            ))
            .compat(),
    )
    .unwrap();
    log!("{:?}", valid_tx_details);

    let tx_str = hex::encode(&*valid_tx_details.tx.tx_hex().unwrap().0);
    let res = block_on(sol_coin.send_raw_tx(&tx_str).compat()).unwrap();

    let res2 = block_on(
        sol_coin
            .send_raw_tx_bytes(&valid_tx_details.tx.tx_hex().unwrap().0)
            .compat(),
    )
    .unwrap();
    assert_eq!(res, res2);
}

// This test is just a unit test for brainstorming around tx_history for base_coin.
#[test]
#[ignore]
#[cfg(not(target_arch = "wasm32"))]
fn solana_test_tx_history() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Local);
    let valid_tx_details = block_on(
        sol_coin
            .withdraw(WithdrawRequest::new(
                "SOL".to_string(),
                None,
                sol_coin.my_address.clone(),
                BigDecimal::try_from(0.0001).unwrap(),
                false,
                None,
                None,
            ))
            .compat(),
    )
    .unwrap();
    log!("{:?}", valid_tx_details);

    let tx_str = hex::encode(&*valid_tx_details.tx.tx_hex().unwrap().0);
    let res = block_on(sol_coin.send_raw_tx(&tx_str).compat()).unwrap();

    let res2 = block_on(
        sol_coin
            .send_raw_tx_bytes(&valid_tx_details.tx.tx_hex().unwrap().0)
            .compat(),
    )
    .unwrap();
    assert_eq!(res, res2);
}

#[test]
fn solana_coin_send_and_refund_maker_payment() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, coin) = solana_coin_for_test(passphrase, SolanaNet::Local);
    let solana_program_id = "GCJUXKH4VeKzEtr9YgwaNWC3dJonFgsM3yMiBa64CZ8m";
    let solana_program_id = bs58::decode(solana_program_id).into_vec().unwrap_or_else(|e| {
        log!("Failed to decode program ID: {}", e);
        Vec::new()
    });

    let pk_data = [1; 32];
    let time_lock = now_sec() - 3600;
    let taker_pub = coin.key_pair.pubkey().to_string();
    let taker_pub = Pubkey::from_str(taker_pub.as_str()).unwrap();
    let secret = [0; 32];
    let secret_hash = sha256(&secret);

    let args = SendPaymentArgs {
        time_lock_duration: 0,
        time_lock,
        other_pubkey: taker_pub.as_ref(),
        secret_hash: secret_hash.as_slice(),
        amount: "0.01".parse().unwrap(),
        swap_contract_address: &Some(Bytes::from(solana_program_id.clone())),
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let tx = Future::wait(coin.send_maker_payment(args)).unwrap();
    log!("swap tx {:?}", tx);

    let refund_args = RefundPaymentArgs {
        payment_tx: &tx.tx_hex(),
        time_lock,
        other_pubkey: taker_pub.as_ref(),
        tx_type_with_secret_hash: SwapTxTypeWithSecretHash::TakerOrMakerPayment {
            maker_secret_hash: secret_hash.as_slice(),
        },
        swap_contract_address: &Some(Bytes::from(solana_program_id)),
        swap_unique_data: pk_data.as_slice(),
        watcher_reward: false,
    };
    let refund_tx = block_on(coin.send_maker_refunds_payment(refund_args)).unwrap();
    log!("refund tx {:?}", refund_tx);
}

#[test]
fn solana_coin_send_and_spend_maker_payment() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, coin) = solana_coin_for_test(passphrase, SolanaNet::Local);
    let solana_program_id = "GCJUXKH4VeKzEtr9YgwaNWC3dJonFgsM3yMiBa64CZ8m";
    let solana_program_id = bs58::decode(solana_program_id).into_vec().unwrap_or_else(|e| {
        log!("Failed to decode program ID: {}", e);
        Vec::new()
    });

    let pk_data = [1; 32];
    let lock_time = now_sec() - 1000;
    let taker_pub = coin.key_pair.pubkey().to_string();
    let taker_pub = Pubkey::from_str(taker_pub.as_str()).unwrap();
    let secret = [0; 32];
    let secret_hash = sha256(&secret);

    let maker_payment_args = SendPaymentArgs {
        time_lock_duration: 0,
        time_lock: lock_time,
        other_pubkey: taker_pub.as_ref(),
        secret_hash: secret_hash.as_slice(),
        amount: "0.01".parse().unwrap(),
        swap_contract_address: &Some(Bytes::from(solana_program_id.clone())),
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };

    let tx = Future::wait(coin.send_maker_payment(maker_payment_args)).unwrap();
    log!("swap tx {:?}", tx);

    let maker_pub = taker_pub;

    let spends_payment_args = SpendPaymentArgs {
        other_payment_tx: &tx.tx_hex(),
        time_lock: lock_time,
        other_pubkey: maker_pub.as_ref(),
        secret: &secret,
        secret_hash: secret_hash.as_slice(),
        swap_contract_address: &Some(Bytes::from(solana_program_id)),
        swap_unique_data: pk_data.as_slice(),
        watcher_reward: false,
    };

    let spend_tx = block_on(coin.send_taker_spends_maker_payment(spends_payment_args)).unwrap();
    log!("spend tx {}", hex::encode(spend_tx.tx_hash_as_bytes().0));
}
