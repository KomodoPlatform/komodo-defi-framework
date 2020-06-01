#![feature(custom_test_frameworks)]
#![feature(test)]
#![test_runner(docker_tests_runner)]
#![feature(drain_filter)]
#![feature(non_ascii_idents)]

#[cfg(test)] use docker_tests::docker_tests_runner;
#[cfg(test)] #[macro_use] extern crate common;
#[cfg(test)] #[macro_use] extern crate fomat_macros;
#[cfg(test)] #[macro_use] extern crate gstuff;
#[cfg(test)] #[macro_use] extern crate lazy_static;
#[cfg(test)] #[macro_use] extern crate serde_json;
#[cfg(test)] #[macro_use] extern crate serde_derive;
#[cfg(test)] #[macro_use] extern crate serialization_derive;
#[cfg(test)] extern crate test;
#[cfg(test)] #[macro_use] extern crate unwrap;

#[cfg(test)]
#[path = "mm2.rs"]
pub mod mm2;

fn main() {
    unimplemented!()
}

#[cfg(all(test, feature = "native"))]
mod docker_tests {
    mod swaps_file_lock_tests;

    use bitcrypto::ChecksumType;
    use common::block_on;
    use common::{
        file_lock::FileLock,
        for_tests::{enable_native, MarketMakerIt, new_mm2_temp_folder_path, mm_dump},
        mm_ctx::{MmArc, MmCtxBuilder}
    };
    use coins::{FoundSwapTxSpend, MarketCoinOps, SwapOps};
    use coins::utxo::{coin_daemon_data_dir, dhash160, utxo_coin_from_conf_and_request, zcash_params_path, UtxoCoin};
    use coins::utxo::rpc_clients::{UtxoRpcClientEnum, UtxoRpcClientOps};
    use futures01::Future;
    use gstuff::now_ms;
    use keys::{KeyPair, Private};
    use secp256k1::SecretKey;
    use serde_json::{self as json, Value as Json};
    use std::env;
    use std::io::{BufRead, BufReader};
    use std::process::Command;
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;
    use test::{test_main, StaticTestFn, StaticBenchFn, TestDescAndFn};
    use testcontainers::{Container, Docker, Image};
    use testcontainers::clients::Cli;
    use testcontainers::images::generic::{GenericImage, WaitFor};

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
            Command::new("docker").arg("pull").arg("artempikulin/testblockchain")
                .status().expect("Failed to execute docker command");

            let stdout = Command::new("docker")
                .arg("ps")
                .arg("-f")
                .arg("ancestor=artempikulin/testblockchain")
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

            let utxo_node = utxo_docker_node(&docker, "MYCOIN", 7000);
            let utxo_node1 = utxo_docker_node(&docker, "MYCOIN1", 8000);
            utxo_node.wait_ready();
            utxo_node1.wait_ready();
            containers.push(utxo_node);
            containers.push(utxo_node1);
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

    struct UtxoDockerNode<'a> {
        #[allow(dead_code)]
        container: Container<'a, Cli, GenericImage>,
        ticker: String,
        #[allow(dead_code)]
        port: u16,
    }

    impl<'a> UtxoDockerNode<'a> {
        pub fn wait_ready(&self) {
            let ctx = MmCtxBuilder::new().into_mm_arc();
            let conf = json!({"asset":self.ticker, "txfee": 1000});
            let req = json!({"method":"enable"});
            let priv_key = unwrap!(hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f"));
            let coin = unwrap!(block_on(utxo_coin_from_conf_and_request(
                &ctx, &self.ticker, &conf, &req, &priv_key)));
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

    fn utxo_docker_node<'a>(docker: &'a Cli, ticker: &'static str, port: u16) -> UtxoDockerNode<'a> {
        let args = vec![
            "-v".into(), format!("{}:/data/.zcash-params", zcash_params_path().display()),
            "-p".into(), format!("127.0.0.1:{}:{}", port, port).into()
        ];
        let image = GenericImage::new("artempikulin/testblockchain")
            .with_args(args)
            .with_env_var("CLIENTS", "2")
            .with_env_var("CHAIN", ticker)
            .with_env_var("TEST_ADDY", "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF")
            .with_env_var("TEST_WIF", "UqqW7f766rADem9heD8vSBvvrdfJb3zg5r8du9rJxPtccjWf7RG9")
            .with_env_var("TEST_PUBKEY", "021607076d7a2cb148d542fb9644c04ffc22d2cca752f80755a0402a24c567b17a")
            .with_env_var("DAEMON_URL", "http://test:test@127.0.0.1:7000")
            .with_env_var("COIN", "Komodo")
            .with_env_var("COIN_RPC_PORT", port.to_string())
            .with_wait_for(WaitFor::message_on_stdout("config is ready"));
        let container = docker.run(image);
        let mut conf_path = coin_daemon_data_dir(ticker, true);
        unwrap!(std::fs::create_dir_all(&conf_path));
        conf_path.push(format!("{}.conf", ticker));
        Command::new("docker")
            .arg("cp")
            .arg(format!("{}:/data/node_0/{}.conf", container.id(), ticker))
            .arg(&conf_path)
            .status()
            .expect("Failed to execute docker command");
        let timeout = now_ms() + 3000;
        loop {
            if conf_path.exists() { break };
            assert!(now_ms() < timeout, "Test timed out");
        }
        UtxoDockerNode {
            container,
            ticker: ticker.into(),
            port,
        }
    }

    lazy_static! {
        static ref COINS_LOCK: Mutex<()> = Mutex::new(());
    }

    // generate random privkey, create a coin and fill it's address with 1000 coins
    fn generate_coin_with_random_privkey(ticker: &str, balance: u64) -> (MmArc, UtxoCoin, [u8; 32])  {
        // prevent concurrent initialization since daemon RPC returns errors if send_to_address
        // is called concurrently (insufficient funds) and it also may return other errors
        // if previous transaction is not confirmed yet
        let ctx = MmCtxBuilder::new().into_mm_arc();
        let _lock = unwrap!(COINS_LOCK.lock());
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let conf = json!({"asset":ticker,"txversion":4,"overwintered":1,"txfee":1000});
        let req = json!({"method":"enable"});
        let priv_key = SecretKey::random(&mut rand4::thread_rng()).serialize();
        let coin = unwrap!(block_on(utxo_coin_from_conf_and_request(
            &ctx, ticker, &conf, &req, &priv_key)));
        fill_address(&coin, &coin.my_address(), balance, timeout);
        (ctx, coin, priv_key)
    }

    fn fill_address(coin: &UtxoCoin, address: &str, amount: u64, timeout: u64) {
        if let UtxoRpcClientEnum::Native(client) = &coin.rpc_client() {
            unwrap!(client.import_address(&coin.my_address(), &coin.my_address(), false).wait());
            let hash = client.send_to_address(address, &amount.into()).wait().unwrap();
            let tx_bytes = client.get_transaction_bytes(hash).wait().unwrap();
            unwrap!(coin.wait_for_confirmations(&tx_bytes, 1, false, timeout, 1).wait());
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
    }

    #[test]
    fn test_search_for_swap_tx_spend_native_was_refunded_taker() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, _) = generate_coin_with_random_privkey("MYCOIN", 1000);

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin.send_taker_payment(
            time_lock,
            &*coin.my_public_key(),
            &[0; 20],
            1.into(),
        ).wait().unwrap();

        unwrap!(coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1).wait());

        let refund_tx = coin.send_taker_refunds_payment(
            &tx.tx_hex(),
            time_lock,
            &*coin.my_public_key(),
            &[0; 20],
        ).wait().unwrap();

        unwrap!(coin.wait_for_confirmations(&refund_tx.tx_hex(), 1, false, timeout, 1).wait());

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
    fn test_search_for_swap_tx_spend_native_was_refunded_maker() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, _) = generate_coin_with_random_privkey("MYCOIN", 1000);

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin.send_maker_payment(
            time_lock,
            &*coin.my_public_key(),
            &[0; 20],
            1.into(),
        ).wait().unwrap();

        unwrap!(coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1).wait());

        let refund_tx = coin.send_maker_refunds_payment(
            &tx.tx_hex(),
            time_lock,
            &*coin.my_public_key(),
            &[0; 20],
        ).wait().unwrap();

        unwrap!(coin.wait_for_confirmations(&refund_tx.tx_hex(), 1, false, timeout, 1).wait());

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
    fn test_search_for_taker_swap_tx_spend_native_was_spent_by_maker() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, _) = generate_coin_with_random_privkey("MYCOIN", 1000);
        let secret = [0; 32];

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin.send_taker_payment(
            time_lock,
            &*coin.my_public_key(),
            &*dhash160(&secret),
            1.into(),
        ).wait().unwrap();

        unwrap!(coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1).wait());

        let spend_tx = coin.send_maker_spends_taker_payment(
            &tx.tx_hex(),
            time_lock,
            &*coin.my_public_key(),
            &secret,
        ).wait().unwrap();

        unwrap!(coin.wait_for_confirmations(&spend_tx.tx_hex(), 1, false, timeout, 1).wait());

        let found = unwrap!(unwrap!(coin.search_for_swap_tx_spend_my(
            time_lock,
            &*coin.my_public_key(),
            &*dhash160(&secret),
            &tx.tx_hex(),
            0,
        )));
        assert_eq!(FoundSwapTxSpend::Spent(spend_tx), found);
    }

    #[test]
    fn test_search_for_maker_swap_tx_spend_native_was_spent_by_taker() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, _) = generate_coin_with_random_privkey("MYCOIN", 1000);
        let secret = [0; 32];

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin.send_maker_payment(
            time_lock,
            &*coin.my_public_key(),
            &*dhash160(&secret),
            1.into(),
        ).wait().unwrap();

        unwrap!(coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1).wait());

        let spend_tx = coin.send_taker_spends_maker_payment(
            &tx.tx_hex(),
            time_lock,
            &*coin.my_public_key(),
            &secret,
        ).wait().unwrap();

        unwrap!(coin.wait_for_confirmations(&spend_tx.tx_hex(), 1, false, timeout, 1).wait());

        let found = unwrap!(unwrap!(coin.search_for_swap_tx_spend_my(
            time_lock,
            &*coin.my_public_key(),
            &*dhash160(&secret),
            &tx.tx_hex(),
            0,
        )));
        assert_eq!(FoundSwapTxSpend::Spent(spend_tx), found);
    }

    // https://github.com/KomodoPlatform/atomicDEX-API/issues/554
    #[test]
    fn order_should_be_cancelled_when_entire_balance_is_withdrawn() {
        let (_ctx, _, priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000);
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000},
        ]);
        let mut mm_bob = unwrap! (MarketMakerIt::start (
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
                "rpcip": env::var ("BOB_TRADE_IP") .ok(),
                "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| unwrap! (s.parse::<i64>())),
                "passphrase": format!("0x{}", hex::encode(priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        ));
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump (&mm_bob.log_path);
        unwrap! (block_on (mm_bob.wait_for_log (60., |log| log.contains (">>>>>>>>> DEX stats "))));
        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        let rc = unwrap! (block_on (mm_bob.rpc (json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "999",
        }))));
        assert! (rc.0.is_success(), "!setprice: {}", rc.1);

        thread::sleep(Duration::from_secs(12));

        log!("Get MYCOIN/MYCOIN1 orderbook");
        let rc = unwrap! (block_on (mm_bob.rpc (json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook " [bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let withdraw = unwrap! (block_on (mm_bob.rpc (json! ({
            "userpass": mm_bob.userpass,
            "method": "withdraw",
            "coin": "MYCOIN",
            "max": true,
            "to": "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF",
        }))));
        assert!(withdraw.0.is_success(), "!withdraw: {}", withdraw.1);

        let withdraw: Json = unwrap!(json::from_str(&withdraw.1));

        let send_raw = unwrap! (block_on (mm_bob.rpc (json! ({
            "userpass": mm_bob.userpass,
            "method": "send_raw_transaction",
            "coin": "MYCOIN",
            "tx_hex": withdraw["tx_hex"],
        }))));
        assert!(send_raw.0.is_success(), "!send_raw: {}", send_raw.1);

        thread::sleep(Duration::from_secs(12));

        log!("Get MYCOIN/MYCOIN1 orderbook");
        let rc = unwrap! (block_on (mm_bob.rpc (json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook " (unwrap!(json::to_string(&bob_orderbook))));
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 0, "MYCOIN/MYCOIN1 orderbook must have exactly 0 asks");

        log!("Get my orders");
        let rc = unwrap! (block_on (mm_bob.rpc (json! ({
            "userpass": mm_bob.userpass,
            "method": "my_orders",
        }))));
        assert!(rc.0.is_success(), "!my_orders: {}", rc.1);
        let orders: Json = unwrap!(json::from_str(&rc.1));
        log!("my_orders " (unwrap!(json::to_string(&orders))));
        assert!(unwrap!(orders["result"]["maker_orders"].as_object()).is_empty(), "maker_orders must be empty");

        unwrap!(block_on(mm_bob.stop()));
    }

    // https://github.com/KomodoPlatform/atomicDEX-API/issues/471
    #[test]
    fn match_and_trade_max() {
        let (_ctx, _, bob_priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000);
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", 2000);
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000},
        ]);
        let mut mm_bob = unwrap! (MarketMakerIt::start (
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        ));
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump (&mm_bob.log_path);
        unwrap! (block_on (mm_bob.wait_for_log (22., |log| log.contains (">>>>>>>>> DEX stats "))));

        let mut mm_alice = unwrap! (MarketMakerIt::start (
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "seednodes": vec![format!("{}", mm_bob.ip)],
            }),
            "pass".to_string(),
            None,
        ));
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump (&mm_alice.log_path);
        unwrap! (block_on (mm_alice.wait_for_log (22., |log| log.contains (">>>>>>>>> DEX stats "))));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);
        let rc = unwrap! (block_on (mm_bob.rpc (json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        }))));
        assert! (rc.0.is_success(), "!setprice: {}", rc.1);

        thread::sleep(Duration::from_secs(12));

        log!("Get MYCOIN/MYCOIN1 orderbook");
        let rc = unwrap! (block_on (mm_bob.rpc (json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook " [bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");
        assert_eq!(asks[0]["maxvolume"], Json::from("999.99999"));

        let rc = unwrap! (block_on (mm_alice.rpc (json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "999.99999",
        }))));
        assert! (rc.0.is_success(), "!buy: {}", rc.1);

        unwrap! (block_on (mm_bob.wait_for_log (22., |log| log.contains ("Entering the maker_swap_loop MYCOIN/MYCOIN1"))));
        unwrap! (block_on (mm_alice.wait_for_log (22., |log| log.contains ("Entering the taker_swap_loop MYCOIN/MYCOIN1"))));
        unwrap!(block_on(mm_bob.stop()));
        unwrap!(block_on(mm_alice.stop()));
    }

    #[test]
    fn swaps_should_stop_on_stop_rpc() {
        let (_ctx, _, bob_priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000);
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", 2000);
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000},
        ]);
        let mut mm_bob = unwrap! (MarketMakerIt::start (
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        ));
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump (&mm_bob.log_path);
        unwrap! (block_on (mm_bob.wait_for_log (22., |log| log.contains (">>>>>>>>> DEX stats "))));

        let mut mm_alice = unwrap! (MarketMakerIt::start (
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "seednodes": vec![format!("{}", mm_bob.ip)],
            }),
            "pass".to_string(),
            None,
        ));
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump (&mm_alice.log_path);
        unwrap! (block_on (mm_alice.wait_for_log (22., |log| log.contains (">>>>>>>>> DEX stats "))));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);
        let rc = unwrap! (block_on (mm_bob.rpc (json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        }))));
        assert! (rc.0.is_success(), "!setprice: {}", rc.1);
        let mut uuids = Vec::with_capacity(3);

        for _ in 0..3 {
            let rc = unwrap!(block_on (mm_alice.rpc (json! ({
                "userpass": mm_alice.userpass,
                "method": "buy",
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "price": 1,
                "volume": "1",
            }))));
            assert!(rc.0.is_success(), "!buy: {}", rc.1);
            let buy: Json = json::from_str(&rc.1).unwrap();
            uuids.push(buy["result"]["uuid"].as_str().unwrap().to_owned());
        }
        for uuid in uuids.iter() {
            unwrap!(block_on (mm_bob.wait_for_log (22.,
                |log| log.contains (&format!("Entering the maker_swap_loop MYCOIN/MYCOIN1 with uuid: {}", uuid))
            )));
            unwrap!(block_on (mm_alice.wait_for_log (22.,
                |log| log.contains (&format!("Entering the taker_swap_loop MYCOIN/MYCOIN1 with uuid: {}", uuid))
            )));
        }
        unwrap!(block_on(mm_bob.stop()));
        unwrap!(block_on(mm_alice.stop()));
        for uuid in uuids {
            unwrap!(block_on (mm_bob.wait_for_log_after_stop (22., |log| log.contains (&format!("swap {} stopped", uuid)))));
            unwrap!(block_on (mm_alice.wait_for_log_after_stop (22., |log| log.contains (&format!("swap {} stopped", uuid)))));
        }
    }
}
