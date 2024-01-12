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

use coins::eth::{u256_to_big_decimal, ERC20_ABI, SWAP_CONTRACT_ABI};
use std::io::{BufRead, BufReader};
use std::process::Command;
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

            let to_addr = ethereum_types::Address::zero();

            let to_addr_balance = block_on(GETH_WEB3.eth().balance(to_addr, None)).unwrap();
            println!("To address balance before transfer {}", to_addr_balance);

            let tx_request = TransactionRequest {
                from: GETH_ACCOUNT,
                to: Some(to_addr),
                gas: None,
                gas_price: None,
                value: Some(U256::from(10).pow(U256::from(18))),
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

            let to_addr_balance = block_on(GETH_WEB3.eth().balance(to_addr, None)).unwrap();
            println!("To address balance {}", to_addr_balance);

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

            let deploy_tx_receipt = block_on(GETH_WEB3.eth().transaction_receipt(deploy_erc20_tx_hash)).unwrap();
            println!("Deploy tx receipt {:?}", deploy_tx_receipt);

            GETH_ERC20_CONTRACT = deploy_tx_receipt.unwrap().contract_address.unwrap();
            println!("GETH_ERC20_CONTRACT {:?}", GETH_ERC20_CONTRACT);

            let erc20_contract =
                Contract::from_json(GETH_WEB3.eth(), GETH_ERC20_CONTRACT, ERC20_ABI.as_bytes()).unwrap();
            let balance: U256 =
                block_on(erc20_contract.query("balanceOf", GETH_ACCOUNT, None, Options::default(), None)).unwrap();
            println!("Token balance {}", u256_to_big_decimal(balance, 8).unwrap());

            let token_receiver = [1; 20].into();
            let erc20_transfer = block_on(erc20_contract.call(
                "transfer",
                (Token::Address(token_receiver), Token::Uint(U256::from(1000))),
                GETH_ACCOUNT,
                Options::default(),
            ))
            .unwrap();
            println!("Sent erc20_transfer {:?}", erc20_transfer);

            let balance: U256 =
                block_on(erc20_contract.query("balanceOf", token_receiver, None, Options::default(), None)).unwrap();
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

            let deploy_swap_tx_receipt = block_on(GETH_WEB3.eth().transaction_receipt(deploy_swap_tx_hash)).unwrap();
            println!("Deploy swap tx receipt {:?}", deploy_swap_tx_receipt);

            GETH_SWAP_CONTRACT = deploy_swap_tx_receipt.unwrap().contract_address.unwrap();
            println!("GETH_SWAP_CONTRACT {:?}", GETH_SWAP_CONTRACT);

            let swap_contract =
                Contract::from_json(GETH_WEB3.eth(), GETH_SWAP_CONTRACT, SWAP_CONTRACT_ABI.as_bytes()).unwrap();
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
