#![cfg(feature = "run-docker-tests")]
#![feature(async_closure)]
#![feature(custom_test_frameworks)]
#![feature(test)]
#![test_runner(docker_tests_runner)]
#![feature(drain_filter)]
#![feature(hash_raw_entry)]
#![cfg(not(target_arch = "wasm32"))]

#[cfg(test)]
#[macro_use]
extern crate common;
#[cfg(test)]
#[macro_use]
extern crate gstuff;
#[cfg(test)]
#[macro_use]
extern crate lazy_static;
#[cfg(test)]
#[macro_use]
extern crate serde_json;
#[cfg(test)] extern crate ser_error_derive;
#[cfg(test)] extern crate test;

use coins::eth::{checksum_address, eth_coin_from_conf_and_request, u256_to_big_decimal, ERC20_ABI};
use coins::{CoinProtocol, ConfirmPaymentInput, MarketCoinOps, PrivKeyBuildPolicy, RefundPaymentArgs, SendPaymentArgs,
            SwapOps, SwapTxTypeWithSecretHash};
use futures01::Future;
use mm2_test_helpers::for_tests::{erc20_dev_conf, eth_dev_conf};
use std::io::{BufRead, BufReader};
use std::process::Command;
use std::time::Duration;
use test::{test_main, StaticBenchFn, StaticTestFn, TestDescAndFn};
use testcontainers::clients::Cli;
use web3::contract::{Contract, Options};
use web3::ethabi::Token;
use web3::types::{TransactionRequest, U256};

mod docker_tests;
use docker_tests::docker_tests_common::*;
use docker_tests::qrc20_tests::{qtum_docker_node, QtumDockerOps, QTUM_REGTEST_DOCKER_IMAGE_WITH_TAG};

#[allow(dead_code)] mod integration_tests_common;

// AP: custom test runner is intended to initialize the required environment (e.g. coin daemons in the docker containers)
// and then gracefully clear it by dropping the RAII docker container handlers
// I've tried to use static for such singleton initialization but it turned out that despite
// rustc allows to use Drop as static the drop fn won't ever be called
// NB: https://github.com/rust-lang/rfcs/issues/1111
// the only preparation step required is Zcash params files downloading:
// Windows - https://github.com/KomodoPlatform/komodo/blob/master/zcutil/fetch-params.bat
// Linux and MacOS - https://github.com/KomodoPlatform/komodo/blob/master/zcutil/fetch-params.sh
pub fn docker_tests_runner(tests: &[&TestDescAndFn]) {
    // pretty_env_logger::try_init();
    let docker = Cli::default();
    let mut containers = vec![];
    // skip Docker containers initialization if we are intended to run test_mm_start only
    if std::env::var("_MM2_TEST_CONF").is_err() {
        pull_docker_image(UTXO_ASSET_DOCKER_IMAGE_WITH_TAG);
        pull_docker_image(QTUM_REGTEST_DOCKER_IMAGE_WITH_TAG);
        pull_docker_image(GETH_DOCKER_IMAGE_WITH_TAG);
        remove_docker_containers(UTXO_ASSET_DOCKER_IMAGE_WITH_TAG);
        remove_docker_containers(QTUM_REGTEST_DOCKER_IMAGE_WITH_TAG);
        remove_docker_containers(GETH_DOCKER_IMAGE_WITH_TAG);

        let utxo_node = utxo_asset_docker_node(&docker, "MYCOIN", 7000);
        let utxo_node1 = utxo_asset_docker_node(&docker, "MYCOIN1", 8000);
        let qtum_node = qtum_docker_node(&docker, 9000);
        let for_slp_node = utxo_asset_docker_node(&docker, "FORSLP", 10000);
        let geth_node = geth_docker_node(&docker, "ETH", 8545);

        let utxo_ops = UtxoAssetDockerOps::from_ticker("MYCOIN");
        let utxo_ops1 = UtxoAssetDockerOps::from_ticker("MYCOIN1");
        let qtum_ops = QtumDockerOps::new();
        let for_slp_ops = BchDockerOps::from_ticker("FORSLP");

        qtum_ops.wait_ready(2);
        qtum_ops.initialize_contracts();
        for_slp_ops.wait_ready(4);
        for_slp_ops.initialize_slp();
        utxo_ops.wait_ready(4);
        utxo_ops1.wait_ready(4);

        let accounts = block_on(GETH_WEB3.eth().accounts()).unwrap();
        unsafe {
            GETH_ACCOUNT = accounts[0];
            println!("GETH ACCOUNT {:?}", GETH_ACCOUNT);

            let tx_request_deploy_erc20 = TransactionRequest {
                from: GETH_ACCOUNT,
                to: None,
                gas: None,
                gas_price: None,
                value: None,
                data: Some(hex::decode(ERC20_TOKEN_BYTES).unwrap().into()),
                nonce: None,
                condition: None,
                transaction_type: None,
                access_list: None,
                max_fee_per_gas: None,
                max_priority_fee_per_gas: None,
            };

            let deploy_erc20_tx_hash = block_on(GETH_WEB3.eth().send_transaction(tx_request_deploy_erc20)).unwrap();
            println!("Sent deploy transaction {:?}", deploy_erc20_tx_hash);

            loop {
                let deploy_tx_receipt = block_on(GETH_WEB3.eth().transaction_receipt(deploy_erc20_tx_hash)).unwrap();
                println!("Deploy tx receipt {:?}", deploy_tx_receipt);

                if let Some(receipt) = deploy_tx_receipt {
                    GETH_ERC20_CONTRACT = receipt.contract_address.unwrap();
                    println!("GETH_ERC20_CONTRACT {:?}", GETH_ERC20_CONTRACT);
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }

            let erc20_contract =
                Contract::from_json(GETH_WEB3.eth(), GETH_ERC20_CONTRACT, ERC20_ABI.as_bytes()).unwrap();
            let balance: U256 =
                block_on(erc20_contract.query("balanceOf", GETH_ACCOUNT, None, Options::default(), None)).unwrap();
            println!("Token balance {}", u256_to_big_decimal(balance, 8).unwrap());

            let tx_request_deploy_swap_contract = TransactionRequest {
                from: GETH_ACCOUNT,
                to: None,
                gas: None,
                gas_price: None,
                value: None,
                data: Some(hex::decode(SWAP_CONTRACT_BYTES).unwrap().into()),
                nonce: None,
                condition: None,
                transaction_type: None,
                access_list: None,
                max_fee_per_gas: None,
                max_priority_fee_per_gas: None,
            };
            let deploy_swap_tx_hash =
                block_on(GETH_WEB3.eth().send_transaction(tx_request_deploy_swap_contract)).unwrap();
            println!("Sent deploy swap contract transaction {:?}", deploy_swap_tx_hash);

            loop {
                let deploy_swap_tx_receipt =
                    block_on(GETH_WEB3.eth().transaction_receipt(deploy_swap_tx_hash)).unwrap();
                println!("Deploy tx receipt {:?}", deploy_swap_tx_receipt);

                if let Some(receipt) = deploy_swap_tx_receipt {
                    GETH_SWAP_CONTRACT = receipt.contract_address.unwrap();
                    println!("GETH_SWAP_CONTRACT {:?}", GETH_SWAP_CONTRACT);
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }

            let eth_conf = eth_dev_conf();
            let req = json!({
                "method": "enable",
                "coin": "ETH",
                "urls": ["http://127.0.0.1:8545"],
                "swap_contract_address": GETH_SWAP_CONTRACT,
            });

            let eth_coin = block_on(eth_coin_from_conf_and_request(
                &MM_CTX,
                "ETH",
                &eth_conf,
                &req,
                CoinProtocol::ETH,
                PrivKeyBuildPolicy::IguanaPrivKey(random_secp256k1_secret()),
            ))
            .unwrap();

            let tx_request = TransactionRequest {
                from: GETH_ACCOUNT,
                to: Some(eth_coin.my_address),
                gas: None,
                gas_price: None,
                // 100 ETH
                value: Some(U256::from(10).pow(U256::from(20))),
                data: None,
                nonce: None,
                condition: None,
                transaction_type: None,
                access_list: None,
                max_fee_per_gas: None,
                max_priority_fee_per_gas: None,
            };
            let tx_hash = block_on(GETH_WEB3.eth().send_transaction(tx_request)).unwrap();
            println!("Sent transaction {:?}", tx_hash);

            let balance = eth_coin.my_balance().wait().unwrap();
            println!("New address ETH balance {}", balance.spendable);

            let erc20_conf = erc20_dev_conf(GETH_ERC20_CONTRACT);
            let req = json!({
                "method": "enable",
                "coin": "ERC20DEV",
                "urls": ["http://127.0.0.1:8545"],
                "swap_contract_address": GETH_SWAP_CONTRACT,
            });
            let erc20_coin = block_on(eth_coin_from_conf_and_request(
                &MM_CTX,
                "ERC20DEV",
                &erc20_conf,
                &req,
                CoinProtocol::ERC20 {
                    platform: "ETH".to_string(),
                    contract_address: checksum_address(&format!("{:02x}", GETH_ERC20_CONTRACT)),
                },
                PrivKeyBuildPolicy::IguanaPrivKey(random_secp256k1_secret()),
            ))
            .unwrap();

            let erc20_transfer = block_on(erc20_contract.call(
                "transfer",
                (
                    Token::Address(erc20_coin.my_address),
                    Token::Uint(U256::from(100000000)),
                ),
                GETH_ACCOUNT,
                Options::default(),
            ))
            .unwrap();
            println!("Sent erc20_transfer {:?}", erc20_transfer);

            let balance = erc20_coin.my_balance().wait().unwrap();
            println!("New address ERC20 balance {}", balance.spendable);

            let time_lock = now_sec() - 100;
            let other_pubkey = &[
                0x02, 0xc6, 0x6e, 0x7d, 0x89, 0x66, 0xb5, 0xc5, 0x55, 0xaf, 0x58, 0x05, 0x98, 0x9d, 0xa9, 0xfb, 0xf8,
                0xdb, 0x95, 0xe1, 0x56, 0x31, 0xce, 0x35, 0x8c, 0x3a, 0x17, 0x10, 0xc9, 0x62, 0x67, 0x90, 0x63,
            ];

            let send_payment_args = SendPaymentArgs {
                time_lock_duration: 100,
                time_lock,
                other_pubkey,
                secret_hash: &[0; 20],
                amount: 1.into(),
                swap_contract_address: &Some(GETH_SWAP_CONTRACT.as_bytes().into()),
                swap_unique_data: &[],
                payment_instructions: &None,
                watcher_reward: None,
                wait_for_confirmation_until: 0,
            };
            let eth_maker_payment = eth_coin.send_maker_payment(send_payment_args).wait().unwrap();

            let confirm_input = ConfirmPaymentInput {
                payment_tx: eth_maker_payment.tx_hex(),
                confirmations: 1,
                requires_nota: false,
                wait_until: now_sec() + 60,
                check_every: 1,
            };
            eth_coin.wait_for_confirmations(confirm_input).wait().unwrap();

            let refund_args = RefundPaymentArgs {
                payment_tx: &eth_maker_payment.tx_hex(),
                time_lock,
                other_pubkey,
                tx_type_with_secret_hash: SwapTxTypeWithSecretHash::TakerOrMakerPayment {
                    maker_secret_hash: &[0; 20],
                },
                swap_contract_address: &Some(GETH_SWAP_CONTRACT.as_bytes().into()),
                swap_unique_data: &[],
                watcher_reward: false,
            };
            let payment_refund = block_on(eth_coin.send_maker_refunds_payment(refund_args)).unwrap();
            println!("Payment refund tx hash {:02x}", payment_refund.tx_hash());

            let confirm_input = ConfirmPaymentInput {
                payment_tx: payment_refund.tx_hex(),
                confirmations: 1,
                requires_nota: false,
                wait_until: now_sec() + 60,
                check_every: 1,
            };
            eth_coin.wait_for_confirmations(confirm_input).wait().unwrap();
        };

        containers.push(utxo_node);
        containers.push(utxo_node1);
        containers.push(qtum_node);
        containers.push(for_slp_node);
        containers.push(geth_node);
    }
    // detect if docker is installed
    // skip the tests that use docker if not installed
    let owned_tests: Vec<_> = tests
        .iter()
        .map(|t| match t.testfn {
            StaticTestFn(f) => TestDescAndFn {
                testfn: StaticTestFn(f),
                desc: t.desc.clone(),
            },
            StaticBenchFn(f) => TestDescAndFn {
                testfn: StaticBenchFn(f),
                desc: t.desc.clone(),
            },
            _ => panic!("non-static tests passed to lp_coins test runner"),
        })
        .collect();
    let args: Vec<String> = std::env::args().collect();
    test_main(&args, owned_tests, None);
}

fn pull_docker_image(name: &str) {
    Command::new("docker")
        .arg("pull")
        .arg(name)
        .status()
        .expect("Failed to execute docker command");
}

fn remove_docker_containers(name: &str) {
    let stdout = Command::new("docker")
        .arg("ps")
        .arg("-f")
        .arg(format!("ancestor={}", name))
        .arg("-q")
        .output()
        .expect("Failed to execute docker command");

    let reader = BufReader::new(stdout.stdout.as_slice());
    let ids: Vec<_> = reader.lines().map(|line| line.unwrap()).collect();
    if !ids.is_empty() {
        Command::new("docker")
            .arg("rm")
            .arg("-f")
            .args(ids)
            .status()
            .expect("Failed to execute docker command");
    }
}
