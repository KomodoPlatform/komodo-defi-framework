/*
Sia docker tests runner. This module is used to run the tests in the sia_docker_tests module.

An environment variable, SKIP_DOCKER_TESTS_RUNNER, can be set to skip the docker container initialization. This will run the tests with the assumption
that there is a walletd instance at 127.0.0.1:9980. This was added to help with debugging the tests in a local environment.
It can be useful otherwise to inspect the state of the walletd instance after the tests have run.

This module used the existing docker test suite at the time of Sia integration. It is not a good example of how to write tests in the mm2 codebase.

Usage:
    Run all sia docker tests:
        cargo test --test docker_tests_sia_unique --all-features -- --nocapture

    Run a specific test:
        cargo test --test docker_tests_sia_unique --all-features test_sia_endpoint_debug_mine -- --nocapture

    Run all sia docker tests without running the docker container:
        SKIP_DOCKER_TESTS_RUNNER=1 cargo test --test docker_tests_sia_unique --all-features -- --nocapture

    Run a specific test without running the docker container:
        SKIP_DOCKER_TESTS_RUNNER=1 cargo test --test docker_tests_sia_unique --all-features test_sia_endpoint_debug_mine -- --nocapture

note: `--nocapture` is shown in the example usage, but it is not neccesary.
*/
#![cfg(feature = "enable-sia")]
#![feature(custom_test_frameworks)]
#![feature(test)]
#![test_runner(docker_tests_runner)]
#![cfg(not(target_arch = "wasm32"))]
#![feature(local_key_cell_methods)]

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
#[cfg(test)] extern crate test;

use std::env;
use std::io::{BufRead, BufReader};
use std::process::Command;
use test::{test_main, StaticBenchFn, StaticTestFn, TestDescAndFn};
use testcontainers::clients::Cli;

// TODO Alright - This docker_tests module is a mess.
// Separate common pieces into a docker_tests_common module that doesn't import an insane amount of unrelated code.
// the use of this tests_runner feature seems unnecessary. Why can't each module initialize its own docker containers?
#[allow(unused_imports, dead_code)] mod docker_tests;
use docker_tests::docker_tests_common::*;

#[allow(dead_code)] mod integration_tests_common;

/// Custom test runner intended to initialize the SIA coin daemon in a Docker container.
pub fn docker_tests_runner(tests: &[&TestDescAndFn]) {
    let docker = Cli::default();
    let mut containers = vec![];

    // SKIP_DOCKER_TESTS_RUNNER=1 is helpful for debugging the docker container itself. You can
    // start the container manaully then run the tests with this environment variable set.
    let skip_docker_tests_runner = std::env::var("SKIP_DOCKER_TESTS_RUNNER")
        .map(|v| v == "1")
        .unwrap_or(false);

    if !skip_docker_tests_runner {
        const IMAGES: &[&str] = &[SIA_DOCKER_IMAGE_WITH_TAG];

        for image in IMAGES {
            pull_docker_image(image);
            remove_docker_containers(image);
        }

        let sia_node = sia_docker_node(&docker, "SIA", 9980);
        containers.push(sia_node);
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
    let args: Vec<String> = env::args().collect();
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
