use base58::ToBase58;
use common::{block_on, block_on_f01, Future01CompatExt};
use rpc::v1::types::Bytes;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_sdk::{bs58,
                 signature::{Signature, Signer}};
use solana_transaction_status::UiTransactionEncoding;
use std::{ops::Neg, str::FromStr};

use super::solana_common_tests::{generate_key_pair_from_iguana_seed, generate_key_pair_from_seed,
                                 solana_coin_for_test, SolanaNet};
use super::solana_decode_tx_helpers::SolanaConfirmedTransaction;
use super::*;
use crate::{MarketCoinOps, SwapTxTypeWithSecretHash};

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

// Research tests
// TODO remove `ignore` attribute once the test is stable.
#[test]
#[ignore]
#[cfg(not(target_arch = "wasm32"))]
fn solana_prerequisites() {
    // same test as trustwallet
    {
        let fin =
            generate_key_pair_from_seed("hood vacant left trim hard mushroom device flavor ask better arrest again");
        let public_address = fin.pubkey().to_string();
        let priv_key = &fin.secret().to_bytes()[..].to_base58();
        assert_eq!(public_address.len(), 44);
        assert_eq!(public_address, "4rmosKwMH7zeaXGbej1PFybZBUyuUNQLf8RfyzCcYvkx");
        assert_eq!(priv_key, "CZtxt17aTfDrJrzwBWdVqcmFwVVptW8EX7RRnth9tT3M");
        let client = solana_client::rpc_client::RpcClient::new("https://api.testnet.solana.com/".to_string());
        let balance = client.get_balance(&fin.pubkey()).expect("Expect to retrieve balance");
        assert_eq!(balance, 0);
    }

    {
        let key_pair = generate_key_pair_from_iguana_seed("passphrase not really secure".to_string());
        let public_address = key_pair.pubkey().to_string();
        assert_eq!(public_address.len(), 44);
        assert_eq!(public_address, "2jTgfhf98GosnKSCXjL5YSiEa3MLrmR42yy9kZZq1i2c");
        let client = solana_client::rpc_client::RpcClient::new("https://api.testnet.solana.com/".to_string());
        let balance = client
            .get_balance(&key_pair.pubkey())
            .expect("Expect to retrieve balance");
        assert_eq!(lamports_to_sol(balance), BigDecimal::from(0));
        assert_eq!(balance, 0);

        //  This will fetch all the balance from all tokens
        let token_accounts = client
            .get_token_accounts_by_owner(&key_pair.pubkey(), TokenAccountsFilter::ProgramId(spl_token::id()))
            .expect("");
        log!("{:?}", token_accounts);
        let actual_token_pubkey = solana_sdk::pubkey::Pubkey::from_str(token_accounts[0].pubkey.as_str()).unwrap();
        let amount = client.get_token_account_balance(&actual_token_pubkey).unwrap();
        assert_ne!(amount.ui_amount_string.as_str(), "0");
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn solana_coin_creation() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Testnet);
    assert_eq!(
        sol_coin.my_address().unwrap(),
        "FJktmyjV9aBHEShT4hfnLpr9ELywdwVtEL1w1rSWgbVf"
    );
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn solana_my_balance() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Testnet);
    let res = block_on(sol_coin.my_balance().compat()).unwrap();
    assert_ne!(res.spendable, BigDecimal::from(0));
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn solana_block_height() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Testnet);
    let res = block_on(sol_coin.current_block().compat()).unwrap();
    log!("block is : {}", res);
    assert!(res > 0);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn solana_validate_address() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Testnet);

    // invalid len
    let res = sol_coin.validate_address("invalidaddressobviously");
    assert!(!res.is_valid);
    let res = sol_coin.validate_address("GMtMFbuVgjDnzsBd3LLBfM4X8RyYcDGCM92tPq2PG6B2");
    assert!(res.is_valid);

    // Typo
    let res = sol_coin.validate_address("Fr8fraJXAe1cFU81mF7NhHTrUzXjZAJkQE1gUQ11riH");
    assert!(!res.is_valid);

    // invalid len
    let res = sol_coin.validate_address("r8fraJXAe1cFU81mF7NhHTrUzXjZAJkQE1gUQ11riHn");
    assert!(!res.is_valid);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_sign_message() {
    let passphrase = "spice describe gravity federal blast come thank unfair canal monkey style afraid".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Testnet);
    let signature = sol_coin.sign_message("test").unwrap();
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
    let is_valid = sol_coin
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
fn solana_transaction_simulations() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Devnet);
    let request_amount = BigDecimal::try_from(0.0001).unwrap();
    let valid_tx_details = block_on(
        sol_coin
            .withdraw(WithdrawRequest {
                coin: "SOL".to_string(),
                from: None,
                to: sol_coin.my_address.clone(),
                amount: request_amount.clone(),
                max: false,
                fee: None,
                memo: None,
                ibc_source_channel: None,
            })
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
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Devnet);
    let invalid_tx_details = block_on(
        sol_coin
            .withdraw(WithdrawRequest {
                coin: "SOL".to_string(),
                from: None,
                to: sol_coin.my_address.clone(),
                amount: BigDecimal::from_str("0.000001").unwrap(),
                max: false,
                fee: None,
                memo: None,
                ibc_source_channel: None,
            })
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
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Devnet);
    let invalid_tx_details = block_on(
        sol_coin
            .withdraw(WithdrawRequest {
                coin: "SOL".to_string(),
                from: None,
                to: sol_coin.my_address.clone(),
                amount: BigDecimal::from(1),
                max: false,
                fee: None,
                memo: None,
                ibc_source_channel: None,
            })
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
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Devnet);
    let valid_tx_details = block_on(
        sol_coin
            .withdraw(WithdrawRequest {
                coin: "SOL".to_string(),
                from: None,
                to: sol_coin.my_address.clone(),
                amount: BigDecimal::from(0),
                max: true,
                fee: None,
                memo: None,
                ibc_source_channel: None,
            })
            .compat(),
    )
    .unwrap();
    let balance = block_on(sol_coin.my_balance().compat()).unwrap().spendable;
    let (_, fees) = block_on(sol_coin.estimate_withdraw_fees()).unwrap();
    let sol_required = lamports_to_sol(fees);
    assert_eq!(valid_tx_details.my_balance_change, sol_required.clone().neg());
    assert_eq!(valid_tx_details.total_amount, balance);
    assert_eq!(valid_tx_details.spent_by_me, balance);
    assert_eq!(valid_tx_details.received_by_me, &balance - &sol_required);
    log!("{:?}", valid_tx_details);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn solana_test_transactions() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Devnet);
    let valid_tx_details = block_on(
        sol_coin
            .withdraw(WithdrawRequest {
                coin: "SOL".to_string(),
                from: None,
                to: sol_coin.my_address.clone(),
                amount: BigDecimal::try_from(0.0001).unwrap(),
                max: false,
                fee: None,
                memo: None,
                ibc_source_channel: None,
            })
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

    //log!("{:?}", res);
}

// This test is just a unit test for brainstorming around tx_history for base_coin.
#[test]
#[ignore]
#[cfg(not(target_arch = "wasm32"))]
fn solana_test_tx_history() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, sol_coin) = solana_coin_for_test(passphrase, SolanaNet::Testnet);
    let res = sol_coin
        .client
        .get_signatures_for_address(&sol_coin.key_pair.pubkey())
        .unwrap();
    let mut history = Vec::new();
    for cur in res.iter() {
        let signature = Signature::from_str(cur.signature.clone().as_str()).unwrap();
        let res = sol_coin
            .client
            .get_transaction(&signature, UiTransactionEncoding::JsonParsed)
            .unwrap();
        log!("{}", serde_json::to_string(&res).unwrap());
        let parsed = serde_json::to_value(&res).unwrap();
        let tx_infos: SolanaConfirmedTransaction = serde_json::from_value(parsed).unwrap();
        let mut txs = tx_infos.extract_solana_transactions(&sol_coin).unwrap();
        history.append(&mut txs);
    }
    log!("{}", serde_json::to_string(&history).unwrap());
}

#[test]
fn solana_coin_send_and_refund_maker_payment() {
    let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
    let (_, coin) = solana_coin_for_test(passphrase, SolanaNet::Devnet);
    let solana_program_id = "3fystoi7pB1cnDEbRRpSjFJA4fG3W2vQQZ21jSrBc11B";
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
    let tx = block_on_f01(coin.send_maker_payment(args)).unwrap();
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
    let (_, coin) = solana_coin_for_test(passphrase, SolanaNet::Devnet);
    let solana_program_id = "3fystoi7pB1cnDEbRRpSjFJA4fG3W2vQQZ21jSrBc11B";
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

    let tx = block_on_f01(coin.send_maker_payment(maker_payment_args)).unwrap();
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
