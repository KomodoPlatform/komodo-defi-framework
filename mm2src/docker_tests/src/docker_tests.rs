#![feature(custom_test_frameworks)]
#![feature(test)]
#![test_runner(docker_tests_runner)]

#[cfg(test)] use docker_tests::docker_tests_runner;
#[cfg(test)] #[macro_use] extern crate common;
#[cfg(test)] #[macro_use] extern crate fomat_macros;
#[cfg(test)] #[macro_use] extern crate lazy_static;
#[cfg(test)] #[macro_use] extern crate serde_json;
#[cfg(test)] extern crate test;
#[cfg(test)] #[macro_use] extern crate unwrap;

#[cfg(test)]
mod docker_tests {
    use coins::{FoundSwapTxSpend, MarketCoinOps, SwapOps};
    use coins::utxo::{coin_daemon_data_dir, dhash160, utxo_coin_from_conf_and_request, zcash_params_path, UtxoCoin};
    use coins::utxo::rpc_clients::{UtxoRpcClientEnum, UtxoRpcClientOps};
    use futures01::Future;
    use futures::executor::block_on;
    use gstuff::now_ms;
    use secp256k1::SecretKey;
    use std::io::{BufRead, BufReader};
    use std::process::Command;
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;
    use test::{list_tests_console, Options, parse_opts, run_tests_console, StaticTestFn, StaticBenchFn, TestDescAndFn};
    use testcontainers::{Container, Docker, Image};
    use testcontainers::clients::Cli;
    use testcontainers::images::generic::{GenericImage, WaitFor};

    // The copy of libtest function returning the exit code instead of immediate process exit
    fn test_main(args: &[String], tests: Vec<TestDescAndFn>, options: Options) -> i32 {
        let mut opts = match parse_opts(args) {
            Some(Ok(o)) => o,
            Some(Err(msg)) => {
                eprintln!("error: {}", msg);
                return 101
            },
            None => return 0,
        };

        opts.options = options;
        if opts.list {
            if let Err(e) = list_tests_console(&opts, tests) {
                eprintln!("error: io error when listing tests: {:?}", e);
                return 101;
            }
            0
        } else {
            match run_tests_console(&opts, tests) {
                Ok(true) => 0,
                Ok(false) => 101,
                Err(e) => {
                    eprintln!("error: io error when listing tests: {:?}", e);
                    101
                }
            }
        }
    }

    // AP: custom test runner is intended to initialize the required environment (e.g. coin daemons in the docker containers)
    // and then gracefully clear it by dropping the RAII docker container handlers
    // I've tried to use static for such singleton initialization but it turned out that despite
    // rustc allows to use Drop as static the drop fn won't ever be called
    // NB: https://github.com/rust-lang/rfcs/issues/1111
    pub fn docker_tests_runner(tests: &[&TestDescAndFn]) {
        // the pre clean up is still required since previous run might be forcefully killed
        // so containers are not stopped
        let ps = Command::new("docker")
            .arg("ps")
            .arg("-f")
            .arg("ancestor=artempikulin/testblockchain")
            .arg("-q")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("Failed to execute docker command");

        let stdout = ps.stdout.unwrap();
        let reader = BufReader::new(stdout);
        let ids: Vec<_> = reader.lines().map(|line| line.unwrap()).collect();
        if !ids.is_empty() {
            Command::new("docker")
                .arg("rm")
                .arg("-f")
                .args(ids)
                .spawn()
                .expect("Failed to execute docker command");
        }

        let docker = Cli::default();
        let utxo_node = utxo_docker_node(&docker);
        utxo_node.wait_ready();
        // detect if docker is installed
        // skip the tests that use docker if not installed
        let owned_tests = tests
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
        let exit_code = test_main(&args, owned_tests, Options::new());
        // drop explicitly as process::exit breaks standard Rust lifecycle
        drop(utxo_node);
        std::process::exit(exit_code);
    }

    struct UtxoDockerNode<'a> {
        container: Container<'a, Cli, GenericImage>
    }

    impl<'a> UtxoDockerNode<'a> {
        pub fn wait_ready(&self) {
            let conf = json!({"asset":"MYCOIN"});
            let req = json!({"method":"enable"});
            let priv_key = unwrap!(hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f"));
            let coin = unwrap!(block_on(utxo_coin_from_conf_and_request("MYCOIN", &conf, &req, &priv_key)));
            let timeout = now_ms() + 30000;
            loop {
                match coin.rpc_client().get_block_count().wait() {
                    Ok(n) => if n > 1 { break },
                    Err(e) => log!([e]),
                }
                assert!(now_ms() < timeout, "Test timed out");
                thread::sleep(Duration::from_secs(1));
            }
        }
    }

    fn utxo_docker_node(docker: &Cli) -> UtxoDockerNode {
        let image = GenericImage::new("artempikulin/testblockchain")
            .with_args(vec!["-v".into(), format!("{}:/root/.zcash-params", zcash_params_path().display()), "-p".into(), "127.0.0.1:7000:7000".into()])
            .with_env_var("CLIENTS", "2")
            .with_env_var("CHAIN", "MYCOIN")
            .with_env_var("TEST_ADDY", "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF")
            .with_env_var("TEST_WIF", "UqqW7f766rADem9heD8vSBvvrdfJb3zg5r8du9rJxPtccjWf7RG9")
            .with_env_var("TEST_PUBKEY", "021607076d7a2cb148d542fb9644c04ffc22d2cca752f80755a0402a24c567b17a")
            .with_wait_for(WaitFor::message_on_stdout("config is ready"));
        let container = docker.run(image);
        let mut conf_path = coin_daemon_data_dir("MYCOIN", true);
        unwrap!(std::fs::create_dir_all(&conf_path));
        conf_path.push("MYCOIN.conf");
        Command::new("docker")
            .arg("cp")
            .arg(format!("{}:/node_0/MYCOIN.conf", container.id()))
            .arg(&conf_path)
            .spawn()
            .expect("Failed to execute docker command");
        let timeout = now_ms() + 3000;
        loop {
            if conf_path.exists() { break };
            assert!(now_ms() < timeout, "Test timed out");
        }
        UtxoDockerNode {
            container
        }
    }

    lazy_static! {
        static ref COINS_LOCK: Mutex<()> = Mutex::new(());
    }

    // generate random privkey, create a coin and fill it's address with 1000 coins
    fn generate_coin_with_random_privkey() -> UtxoCoin {
        // prevent concurrent initialization since daemon RPC returns errors if send_to_address
        // is called concurrently (insufficient funds) and it also may return other errors
        // if previous transaction is not confirmed yet
        let _lock = unwrap!(COINS_LOCK.lock());
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let conf = json!({"asset":"MYCOIN","txversion":4,"overwintered":1});
        let req = json!({"method":"enable"});
        let priv_key = SecretKey::random(&mut rand::thread_rng());
        let coin = unwrap!(block_on(utxo_coin_from_conf_and_request("MYCOIN", &conf, &req, &priv_key.serialize())));
        if let UtxoRpcClientEnum::Native(client) = &coin.rpc_client() {
            unwrap!(client.import_address(&coin.my_address(), &coin.my_address(), false).wait());
            let hash = client.send_to_address(&coin.my_address(), &1000.into()).wait().unwrap();
            let tx_bytes = client.get_transaction_bytes(hash).wait().unwrap();
            unwrap!(coin.wait_for_confirmations(&tx_bytes, 1, timeout, 1));
            log!({ "{:02x}", tx_bytes });
            loop {
                let unspents = client.list_unspent(0, std::i32::MAX, vec![coin.my_address().into()]).wait().unwrap();
                log!([unspents]);
                if !unspents.is_empty() {
                    break;
                }
                assert!(now_ms() / 1000 < timeout, "Test timed out");
                thread::sleep(Duration::from_secs(1));
            };
        };
        coin
    }

    #[test]
    fn test_search_for_swap_tx_spend_native_was_refunded() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let coin = generate_coin_with_random_privkey();

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin.send_taker_payment(
            time_lock,
            &*coin.my_public_key(),
            &[0; 20],
            1.into(),
        ).wait().unwrap();

        unwrap!(coin.wait_for_confirmations(&tx.tx_hex(), 1, timeout, 1));

        let refund_tx = coin.send_taker_refunds_payment(
            &tx.tx_hex(),
            time_lock,
            &*coin.my_public_key(),
            &[0; 20],
        ).wait().unwrap();

        unwrap!(coin.wait_for_confirmations(&refund_tx.tx_hex(), 1, timeout, 1));

        let found = unwrap!(unwrap!(coin.search_for_swap_tx_spend_my(
            time_lock,
            &*coin.my_public_key(),
            &[0; 20],
            &tx.tx_hex(),
            0,
        )));
        assert_eq!(FoundSwapTxSpend::Refunded(refund_tx), found);
    }

    #[test]
    fn test_search_for_swap_tx_spend_native_was_spent() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let coin = generate_coin_with_random_privkey();
        let secret = [0; 32];

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin.send_taker_payment(
            time_lock,
            &*coin.my_public_key(),
            &*dhash160(&secret),
            1.into(),
        ).wait().unwrap();

        unwrap!(coin.wait_for_confirmations(&tx.tx_hex(), 1, timeout, 1));

        let spend_tx = coin.send_maker_spends_taker_payment(
            &tx.tx_hex(),
            time_lock,
            &*coin.my_public_key(),
            &secret,
        ).wait().unwrap();

        unwrap!(coin.wait_for_confirmations(&spend_tx.tx_hex(), 1, timeout, 1));

        let found = unwrap!(unwrap!(coin.search_for_swap_tx_spend_my(
            time_lock,
            &*coin.my_public_key(),
            &*dhash160(&secret),
            &tx.tx_hex(),
            0,
        )));
        assert_eq!(FoundSwapTxSpend::Spent(spend_tx), found);
    }
}
