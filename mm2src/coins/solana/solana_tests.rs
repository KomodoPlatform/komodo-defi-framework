use super::*;
use crate::common::Future01CompatExt;
use crate::solana::solana_common_tests::{generate_key_pair_from_iguana_seed, generate_key_pair_from_seed,
                                         solana_coin_for_test, SolanaNet};
use crate::MarketCoinOps;
use base58::ToBase58;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_sdk::signature::Signer;
use std::str::FromStr;

mod tests {
    use super::*;
    use crate::solana::solana_decode_tx_helpers::SolanaConfirmedTransaction;
    use solana_sdk::signature::Signature;
    use solana_transaction_status::UiTransactionEncoding;
    use std::ops::Neg;

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
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn solana_prerequisites() {
        // same test as trustwallet
        {
            let fin = generate_key_pair_from_seed(
                "shoot island position soft burden budget tooth cruel issue economy destroy above".to_string(),
            );
            let public_address = fin.pubkey().to_string();
            let priv_key = &fin.secret().to_bytes()[..].to_base58();
            assert_eq!(public_address.len(), 44);
            assert_eq!(public_address, "2bUBiBNZyD29gP1oV6de7nxowMLoDBtopMMTGgMvjG5m");
            assert_eq!(priv_key, "F6czu7fdefbsCDH52JesQrBSJS5Sz25AkPLWFf8zUWhm");
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
            assert_eq!(lamports_to_sol(balance), 0.0.into());
            assert_eq!(balance, 0);

            //  This will fetch all the balance from all tokens
            let token_accounts = client
                .get_token_accounts_by_owner(&key_pair.pubkey(), TokenAccountsFilter::ProgramId(spl_token::id()))
                .expect("");
            println!("{:?}", token_accounts);
            let actual_token_pubkey = solana_sdk::pubkey::Pubkey::from_str(token_accounts[0].pubkey.as_str()).unwrap();
            let amount = client.get_token_account_balance(&actual_token_pubkey).unwrap();
            assert_ne!(amount.ui_amount_string.as_str(), "0");
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn solana_coin_creation() {
        let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
        let (_, sol_coin) = solana_coin_for_test(passphrase.clone(), SolanaNet::Testnet);
        assert_eq!(
            sol_coin.my_address().unwrap(),
            "FJktmyjV9aBHEShT4hfnLpr9ELywdwVtEL1w1rSWgbVf"
        );
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn solana_my_balance() {
        let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
        let (_, sol_coin) = solana_coin_for_test(passphrase.clone(), SolanaNet::Testnet);
        let res = sol_coin.my_balance().compat().await.unwrap();
        assert_ne!(res.spendable, BigDecimal::from(0.0));
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn solana_block_height() {
        let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
        let (_, sol_coin) = solana_coin_for_test(passphrase.clone(), SolanaNet::Testnet);
        let res = sol_coin.current_block().compat().await.unwrap();
        println!("block is : {}", res);
        assert!(res > 0);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn solana_validate_address() {
        let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
        let (_, sol_coin) = solana_coin_for_test(passphrase.clone(), SolanaNet::Testnet);

        // invalid len
        let res = sol_coin.validate_address("invalidaddressobviously");
        assert_eq!(res.is_valid, false);

        let res = sol_coin.validate_address("GMtMFbuVgjDnzsBd3LLBfM4X8RyYcDGCM92tPq2PG6B2");
        assert_eq!(res.is_valid, true);

        // Typo
        let res = sol_coin.validate_address("Fr8fraJXAe1cFU81mF7NhHTrUzXjZAJkQE1gUQ11riH");
        assert_eq!(res.is_valid, false);

        // invalid len
        let res = sol_coin.validate_address("r8fraJXAe1cFU81mF7NhHTrUzXjZAJkQE1gUQ11riHn");
        assert_eq!(res.is_valid, false);
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn solana_transaction_simulations() {
        let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
        let (_, sol_coin) = solana_coin_for_test(passphrase.clone(), SolanaNet::Devnet);
        let request_amount: BigDecimal = 0.0001.into();
        let valid_tx_details = sol_coin
            .withdraw(WithdrawRequest {
                coin: "SOL".to_string(),
                from: None,
                to: sol_coin.my_address.clone(),
                amount: request_amount.clone(),
                max: false,
                fee: None,
            })
            .compat()
            .await
            .unwrap();
        let (_, fees) = sol_coin.estimate_withdraw_fees().await.unwrap();
        let sol_required = lamports_to_sol(fees);
        let expected_spent_by_me = &request_amount + &sol_required;
        assert_eq!(valid_tx_details.spent_by_me, expected_spent_by_me);
        assert_eq!(valid_tx_details.received_by_me, request_amount);
        assert_eq!(valid_tx_details.total_amount, expected_spent_by_me);
        assert_eq!(valid_tx_details.my_balance_change, sol_required.neg());
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn solana_transaction_zero_balance() {
        let passphrase = "fake passphrase".to_string();
        let (_, sol_coin) = solana_coin_for_test(passphrase.clone(), SolanaNet::Devnet);
        let invalid_tx_details = sol_coin
            .withdraw(WithdrawRequest {
                coin: "SOL".to_string(),
                from: None,
                to: sol_coin.my_address.clone(),
                amount: BigDecimal::from_str("0.000001").unwrap(),
                max: false,
                fee: None,
            })
            .compat()
            .await;
        let error = invalid_tx_details.unwrap_err();
        let (_, fees) = sol_coin.estimate_withdraw_fees().await.unwrap();
        let sol_required = lamports_to_sol(fees);
        match error.into_inner() {
            WithdrawError::NotSufficientBalance { required, .. } => {
                assert_eq!(required, sol_required);
            },
            e @ _ => panic!("Unexpected err {:?}", e),
        };
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn solana_transaction_simulations_not_enough_for_fees() {
        let passphrase = "non existent passphrase".to_string();
        let (_, sol_coin) = solana_coin_for_test(passphrase.clone(), SolanaNet::Devnet);
        let invalid_tx_details = sol_coin
            .withdraw(WithdrawRequest {
                coin: "SOL".to_string(),
                from: None,
                to: sol_coin.my_address.clone(),
                amount: BigDecimal::from(1),
                max: false,
                fee: None,
            })
            .compat()
            .await;
        let error = invalid_tx_details.unwrap_err();
        let (_, fees) = sol_coin.estimate_withdraw_fees().await.unwrap();
        let sol_required = lamports_to_sol(fees);
        match error.into_inner() {
            WithdrawError::NotSufficientBalance {
                coin,
                available,
                required,
            } => {
                assert_eq!(available, 0.into());
                assert_eq!(required, sol_required);
            },
            e @ _ => panic!("Unexpected err {:?}", e),
        };
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn solana_transaction_simulations_max() {
        let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
        let (_, sol_coin) = solana_coin_for_test(passphrase.clone(), SolanaNet::Devnet);
        let valid_tx_details = sol_coin
            .withdraw(WithdrawRequest {
                coin: "SOL".to_string(),
                from: None,
                to: sol_coin.my_address.clone(),
                amount: BigDecimal::from(0),
                max: true,
                fee: None,
            })
            .compat()
            .await
            .unwrap();
        let balance = sol_coin.my_balance().compat().await.unwrap().spendable;
        let (_, fees) = sol_coin.estimate_withdraw_fees().await.unwrap();
        let sol_required = lamports_to_sol(fees);
        assert_eq!(valid_tx_details.my_balance_change, sol_required.clone().neg());
        assert_eq!(valid_tx_details.total_amount, balance);
        assert_eq!(valid_tx_details.spent_by_me, balance);
        assert_eq!(valid_tx_details.received_by_me, &balance - &sol_required);
        println!("{:?}", valid_tx_details);
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn solana_test_transactions() {
        let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
        let (_, sol_coin) = solana_coin_for_test(passphrase.clone(), SolanaNet::Devnet);
        let valid_tx_details = sol_coin
            .withdraw(WithdrawRequest {
                coin: "SOL".to_string(),
                from: None,
                to: sol_coin.my_address.clone(),
                amount: BigDecimal::from(0.0001),
                max: false,
                fee: None,
            })
            .compat()
            .await
            .unwrap();
        println!("{:?}", valid_tx_details);

        let tx_str = hex::encode(&*valid_tx_details.tx_hex.0);
        let res = sol_coin.send_raw_tx(&tx_str).compat().await.unwrap();

        let res2 = sol_coin
            .send_raw_tx_bytes(&*valid_tx_details.tx_hex.0)
            .compat()
            .await
            .unwrap();
        assert_eq!(res, res2);

        //println!("{:?}", res);
    }

    // This test is just a unit test for brainstorming around tx_history for base_coin.
    #[tokio::test]
    #[ignore]
    #[cfg(not(target_arch = "wasm32"))]
    async fn solana_test_tx_history() {
        let passphrase = "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron".to_string();
        let (_, sol_coin) = solana_coin_for_test(passphrase.clone(), SolanaNet::Testnet);
        let res = sol_coin
            .client
            .get_signatures_for_address(&sol_coin.key_pair.pubkey())
            .await
            .unwrap();
        let mut history = Vec::new();
        for cur in res.iter() {
            let signature = Signature::from_str(cur.signature.clone().as_str()).unwrap();
            let res = sol_coin
                .client
                .get_transaction(&signature, UiTransactionEncoding::JsonParsed)
                .await
                .unwrap();
            println!("{}", serde_json::to_string(&res).unwrap());
            let parsed = serde_json::to_value(&res).unwrap();
            let tx_infos: SolanaConfirmedTransaction = serde_json::from_value(parsed).unwrap();
            let mut txs = tx_infos.extract_solana_transactions(&sol_coin);
            history.append(&mut txs);
        }
        println!("{}", serde_json::to_string(&history).unwrap());
    }
}
