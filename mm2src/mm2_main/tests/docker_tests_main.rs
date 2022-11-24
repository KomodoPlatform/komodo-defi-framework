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

use std::io::{BufRead, BufReader};
use std::process::Command;
use test::{test_main, StaticBenchFn, StaticTestFn, TestDescAndFn};
use testcontainers::clients::Cli;
mod docker_tests;
use docker_tests::docker_tests_common::*;
use docker_tests::qrc20_tests::{qtum_docker_node, QtumDockerOps, QTUM_REGTEST_DOCKER_IMAGE};
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
        pull_docker_image(UTXO_ASSET_DOCKER_IMAGE);
        pull_docker_image(QTUM_REGTEST_DOCKER_IMAGE);
        remove_docker_containers(UTXO_ASSET_DOCKER_IMAGE);
        remove_docker_containers(QTUM_REGTEST_DOCKER_IMAGE);

        let utxo_node = utxo_asset_docker_node(&docker, "MYCOIN", 7000);
        let utxo_node1 = utxo_asset_docker_node(&docker, "MYCOIN1", 8000);
        let qtum_node = qtum_docker_node(&docker, 9000);
        let for_slp_node = utxo_asset_docker_node(&docker, "FORSLP", 10000);

        let utxo_ops = UtxoAssetDockerOps::from_ticker("MYCOIN");
        let utxo_ops1 = UtxoAssetDockerOps::from_ticker("MYCOIN1");
        let qtum_ops = QtumDockerOps::new();
        let for_slp_ops = BchDockerOps::from_ticker("FORSLP");

        utxo_ops.wait_ready(4);
        utxo_ops1.wait_ready(4);
        qtum_ops.wait_ready(2);
        qtum_ops.initialize_contracts();
        for_slp_ops.wait_ready(4);
        for_slp_ops.initialize_slp();

        containers.push(utxo_node);
        containers.push(utxo_node1);
        containers.push(qtum_node);
        containers.push(for_slp_node);
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
    let _exit_code = test_main(&args, owned_tests, None);
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
