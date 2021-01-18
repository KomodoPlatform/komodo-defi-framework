#![feature(async_closure)]
#![feature(custom_test_frameworks)]
#![feature(test)]
#![test_runner(docker_tests_runner)]
#![feature(drain_filter)]
#![feature(hash_raw_entry)]
#![feature(non_ascii_idents)]
#![recursion_limit = "512"]

#[cfg(test)] use docker_tests::docker_tests_runner;
#[cfg(test)]
#[macro_use]
extern crate common;
#[cfg(test)]
#[macro_use]
extern crate fomat_macros;
#[cfg(test)]
#[macro_use]
extern crate gstuff;
#[cfg(test)]
#[macro_use]
extern crate lazy_static;
#[cfg(test)]
#[macro_use]
extern crate serde_json;
#[cfg(test)]
#[macro_use]
extern crate serde_derive;
#[cfg(test)]
#[macro_use]
extern crate serialization_derive;
#[cfg(test)] extern crate test;
#[cfg(test)]
#[macro_use]
extern crate unwrap;

#[cfg(test)]
#[path = "mm2.rs"]
pub mod mm2;

fn main() { unimplemented!() }

/// rustfmt cannot resolve the module path within docker_tests.
/// Specify the path manually outside the docker_tests.
#[cfg(rustfmt)]
#[path = "docker_tests/swaps_confs_settings_sync_tests.rs"]
mod swaps_confs_settings_sync_tests;

#[cfg(rustfmt)]
#[path = "docker_tests/swaps_file_lock_tests.rs"]
mod swaps_file_lock_tests;

#[cfg(rustfmt)]
#[path = "docker_tests/qrc20_tests.rs"]
mod qrc20_tests;

#[cfg(all(test, feature = "native"))]
mod docker_tests {
    #[rustfmt::skip]
    mod swaps_confs_settings_sync_tests;
    #[rustfmt::skip]
    mod swaps_file_lock_tests;
    #[rustfmt::skip]
    mod qrc20_tests;

    use bigdecimal::BigDecimal;
    use bitcrypto::ChecksumType;
    use chain::OutPoint;
    use coins::utxo::rpc_clients::{UnspentInfo, UtxoRpcClientEnum, UtxoRpcClientOps};
    use coins::utxo::utxo_standard::{utxo_standard_coin_from_conf_and_request, UtxoStandardCoin};
    use coins::utxo::{coin_daemon_data_dir, dhash160, zcash_params_path, UtxoCoinFields, UtxoCommonOps};
    use coins::{FoundSwapTxSpend, MarketCoinOps, MmCoin, SwapOps, TransactionEnum};
    use common::block_on;
    use common::for_tests::enable_electrum;
    use common::{file_lock::FileLock,
                 for_tests::{enable_native, mm_dump, new_mm2_temp_folder_path, MarketMakerIt},
                 mm_ctx::{MmArc, MmCtxBuilder}};
    use futures01::Future;
    use gstuff::now_ms;
    use keys::{KeyPair, Private};
    use primitives::hash::H160;
    use qrc20_tests::{qtum_docker_node, QtumDockerOps, QTUM_REGTEST_DOCKER_IMAGE};
    use secp256k1::{PublicKey, SecretKey};
    use serde_json::{self as json, Value as Json};
    use std::collections::HashMap;
    use std::env;
    use std::io::{BufRead, BufReader};
    use std::process::Command;
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;
    use test::{test_main, StaticBenchFn, StaticTestFn, TestDescAndFn};
    use testcontainers::clients::Cli;
    use testcontainers::images::generic::{GenericImage, WaitFor};
    use testcontainers::{Container, Docker, Image};

    fn rmd160_from_priv(privkey: [u8; 32]) -> H160 {
        let secret = SecretKey::parse(&privkey).unwrap();
        let public = PublicKey::from_secret_key(&secret);
        dhash160(&public.serialize_compressed())
    }

    const UTXO_ASSET_DOCKER_IMAGE: &str = "artempikulin/testblockchain";

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

            let utxo_ops = UtxoAssetDockerOps::from_ticker("MYCOIN");
            let utxo_ops1 = UtxoAssetDockerOps::from_ticker("MYCOIN1");
            let qtum_ops = QtumDockerOps::new();

            utxo_ops.wait_ready();
            utxo_ops1.wait_ready();
            qtum_ops.wait_ready();
            qtum_ops.initialize_contracts();

            containers.push(utxo_node);
            containers.push(utxo_node1);
            containers.push(qtum_node);
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

    trait CoinDockerOps {
        fn rpc_client(&self) -> &UtxoRpcClientEnum;

        fn wait_ready(&self) {
            let timeout = now_ms() + 30000;
            loop {
                match self.rpc_client().get_block_count().wait() {
                    Ok(n) => {
                        if n > 1 {
                            break;
                        }
                    },
                    Err(e) => log!([e]),
                }
                assert!(now_ms() < timeout, "Test timed out");
                thread::sleep(Duration::from_secs(1));
            }
        }
    }

    struct UtxoAssetDockerOps {
        #[allow(dead_code)]
        ctx: MmArc,
        coin: UtxoStandardCoin,
    }

    impl CoinDockerOps for UtxoAssetDockerOps {
        fn rpc_client(&self) -> &UtxoRpcClientEnum { &self.coin.as_ref().rpc_client }
    }

    impl UtxoAssetDockerOps {
        fn from_ticker(ticker: &str) -> UtxoAssetDockerOps {
            let conf = json!({"asset": ticker, "txfee": 1000, "network": "regtest"});
            let req = json!({"method":"enable"});
            let priv_key = unwrap!(hex::decode(
                "809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f"
            ));
            let ctx = MmCtxBuilder::new().into_mm_arc();
            let coin = unwrap!(block_on(utxo_standard_coin_from_conf_and_request(
                &ctx, ticker, &conf, &req, &priv_key,
            )));
            UtxoAssetDockerOps { ctx, coin }
        }
    }

    pub struct UtxoDockerNode<'a> {
        #[allow(dead_code)]
        container: Container<'a, Cli, GenericImage>,
        #[allow(dead_code)]
        ticker: String,
        #[allow(dead_code)]
        port: u16,
    }

    fn utxo_asset_docker_node<'a>(docker: &'a Cli, ticker: &'static str, port: u16) -> UtxoDockerNode<'a> {
        let args = vec![
            "-v".into(),
            format!("{}:/data/.zcash-params", zcash_params_path().display()),
            "-p".into(),
            format!("127.0.0.1:{}:{}", port, port).into(),
        ];
        let image = GenericImage::new(UTXO_ASSET_DOCKER_IMAGE)
            .with_args(args)
            .with_env_var("CLIENTS", "2")
            .with_env_var("CHAIN", ticker)
            .with_env_var("TEST_ADDY", "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF")
            .with_env_var("TEST_WIF", "UqqW7f766rADem9heD8vSBvvrdfJb3zg5r8du9rJxPtccjWf7RG9")
            .with_env_var(
                "TEST_PUBKEY",
                "021607076d7a2cb148d542fb9644c04ffc22d2cca752f80755a0402a24c567b17a",
            )
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
            if conf_path.exists() {
                break;
            };
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

    /// Build asset `UtxoStandardCoin` from ticker and privkey without filling the balance.
    fn utxo_coin_from_privkey(ticker: &str, priv_key: &[u8]) -> (MmArc, UtxoStandardCoin) {
        let ctx = MmCtxBuilder::new().into_mm_arc();
        let conf = json!({"asset":ticker,"txversion":4,"overwintered":1,"txfee":1000,"network":"regtest"});
        let req = json!({"method":"enable"});
        let coin = unwrap!(block_on(utxo_standard_coin_from_conf_and_request(
            &ctx, ticker, &conf, &req, priv_key
        )));
        import_address(&coin);
        (ctx, coin)
    }

    /// Generate random privkey, create a coin and fill it's address with the specified balance.
    fn generate_coin_with_random_privkey(ticker: &str, balance: BigDecimal) -> (MmArc, UtxoStandardCoin, [u8; 32]) {
        let priv_key = SecretKey::random(&mut rand4::thread_rng()).serialize();
        let (ctx, coin) = utxo_coin_from_privkey(ticker, &priv_key);
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let my_address = coin.my_address().expect("!my_address");
        fill_address(&coin, &my_address, balance, timeout);
        (ctx, coin, priv_key)
    }

    fn import_address<T>(coin: &T)
    where
        T: MarketCoinOps + AsRef<UtxoCoinFields>,
    {
        match coin.as_ref().rpc_client {
            UtxoRpcClientEnum::Native(ref native) => {
                let my_address = coin.my_address().unwrap();
                unwrap!(native.import_address(&my_address, &my_address, false).wait())
            },
            UtxoRpcClientEnum::Electrum(_) => panic!("Expected NativeClient"),
        }
    }

    fn fill_address<T>(coin: &T, address: &str, amount: BigDecimal, timeout: u64)
    where
        T: MarketCoinOps + AsRef<UtxoCoinFields>,
    {
        // prevent concurrent fill since daemon RPC returns errors if send_to_address
        // is called concurrently (insufficient funds) and it also may return other errors
        // if previous transaction is not confirmed yet
        let _lock = unwrap!(COINS_LOCK.lock());

        if let UtxoRpcClientEnum::Native(client) = &coin.as_ref().rpc_client {
            unwrap!(client.import_address(address, address, false).wait());
            let hash = client.send_to_address(address, &amount).wait().unwrap();
            let tx_bytes = client.get_transaction_bytes(hash).wait().unwrap();
            unwrap!(coin.wait_for_confirmations(&tx_bytes, 1, false, timeout, 1).wait());
            log!({ "{:02x}", tx_bytes });
            loop {
                let unspents = client
                    .list_unspent_impl(0, std::i32::MAX, vec![address.to_string()])
                    .wait()
                    .unwrap();
                if !unspents.is_empty() {
                    break;
                }
                assert!(now_ms() / 1000 < timeout, "Test timed out");
                thread::sleep(Duration::from_secs(1));
            }
        };
    }

    #[test]
    fn test_search_for_swap_tx_spend_native_was_refunded_taker() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, _) = generate_coin_with_random_privkey("MYCOIN", 1000.into());

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin
            .send_taker_payment(time_lock, &*coin.my_public_key(), &[0; 20], 1.into(), &None)
            .wait()
            .unwrap();

        unwrap!(coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1).wait());

        let refund_tx = coin
            .send_taker_refunds_payment(&tx.tx_hex(), time_lock, &*coin.my_public_key(), &[0; 20], &None)
            .wait()
            .unwrap();

        unwrap!(coin
            .wait_for_confirmations(&refund_tx.tx_hex(), 1, false, timeout, 1)
            .wait());

        let found = unwrap!(unwrap!(coin.search_for_swap_tx_spend_my(
            time_lock,
            &*coin.my_public_key(),
            &[0; 20],
            &tx.tx_hex(),
            0,
            &None
        )));
        assert_eq!(FoundSwapTxSpend::Refunded(refund_tx), found);
    }

    #[test]
    fn test_search_for_swap_tx_spend_native_was_refunded_maker() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, _) = generate_coin_with_random_privkey("MYCOIN", 1000.into());

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin
            .send_maker_payment(time_lock, &*coin.my_public_key(), &[0; 20], 1.into(), &None)
            .wait()
            .unwrap();

        unwrap!(coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1).wait());

        let refund_tx = coin
            .send_maker_refunds_payment(&tx.tx_hex(), time_lock, &*coin.my_public_key(), &[0; 20], &None)
            .wait()
            .unwrap();

        unwrap!(coin
            .wait_for_confirmations(&refund_tx.tx_hex(), 1, false, timeout, 1)
            .wait());

        let found = unwrap!(unwrap!(coin.search_for_swap_tx_spend_my(
            time_lock,
            &*coin.my_public_key(),
            &[0; 20],
            &tx.tx_hex(),
            0,
            &None
        )));
        assert_eq!(FoundSwapTxSpend::Refunded(refund_tx), found);
    }

    #[test]
    fn test_search_for_taker_swap_tx_spend_native_was_spent_by_maker() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, _) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let secret = [0; 32];

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin
            .send_taker_payment(time_lock, &*coin.my_public_key(), &*dhash160(&secret), 1.into(), &None)
            .wait()
            .unwrap();

        unwrap!(coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1).wait());

        let spend_tx = coin
            .send_maker_spends_taker_payment(&tx.tx_hex(), time_lock, &*coin.my_public_key(), &secret, &None)
            .wait()
            .unwrap();

        unwrap!(coin
            .wait_for_confirmations(&spend_tx.tx_hex(), 1, false, timeout, 1)
            .wait());

        let found = unwrap!(unwrap!(coin.search_for_swap_tx_spend_my(
            time_lock,
            &*coin.my_public_key(),
            &*dhash160(&secret),
            &tx.tx_hex(),
            0,
            &None
        )));
        assert_eq!(FoundSwapTxSpend::Spent(spend_tx), found);
    }

    #[test]
    fn test_search_for_maker_swap_tx_spend_native_was_spent_by_taker() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, _) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let secret = [0; 32];

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin
            .send_maker_payment(time_lock, &*coin.my_public_key(), &*dhash160(&secret), 1.into(), &None)
            .wait()
            .unwrap();

        unwrap!(coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1).wait());

        let spend_tx = coin
            .send_taker_spends_maker_payment(&tx.tx_hex(), time_lock, &*coin.my_public_key(), &secret, &None)
            .wait()
            .unwrap();

        unwrap!(coin
            .wait_for_confirmations(&spend_tx.tx_hex(), 1, false, timeout, 1)
            .wait());

        let found = unwrap!(unwrap!(coin.search_for_swap_tx_spend_my(
            time_lock,
            &*coin.my_public_key(),
            &*dhash160(&secret),
            &tx.tx_hex(),
            0,
            &None
        )));
        assert_eq!(FoundSwapTxSpend::Spent(spend_tx), found);
    }

    #[test]
    fn test_one_hundred_maker_payments_in_a_row_native() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, _) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        let secret = [0; 32];

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let mut unspents = vec![];
        let mut sent_tx = vec![];
        for i in 0..100 {
            let tx = coin
                .send_maker_payment(
                    time_lock + i,
                    &*coin.my_public_key(),
                    &*dhash160(&secret),
                    1.into(),
                    &coin.swap_contract_address(),
                )
                .wait()
                .unwrap();
            if let TransactionEnum::UtxoTx(tx) = tx {
                unspents.push(UnspentInfo {
                    outpoint: OutPoint {
                        hash: tx.hash(),
                        index: 2,
                    },
                    value: tx.outputs[2].value,
                    height: None,
                });
                sent_tx.push(tx);
            }
        }

        let recently_sent = block_on(coin.as_ref().recently_spent_outpoints.lock());

        let before = now_ms();
        unspents = recently_sent
            .replace_spent_outputs_with_cache(unspents.into_iter().collect())
            .into_iter()
            .collect();

        let after = now_ms();
        log!("Took "(after - before));

        let last_tx = sent_tx.last().unwrap();
        let expected_unspent = UnspentInfo {
            outpoint: OutPoint {
                hash: last_tx.hash(),
                index: 2,
            },
            value: last_tx.outputs[2].value,
            height: None,
        };
        assert_eq!(vec![expected_unspent], unspents);
    }

    // https://github.com/KomodoPlatform/atomicDEX-API/issues/554
    #[test]
    fn order_should_be_cancelled_when_entire_balance_is_withdrawn() {
        let (_ctx, _, priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = unwrap!(MarketMakerIt::start(
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
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(60., |log| log.contains(">>>>>>>>> DEX stats "))
        ));
        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "999",
        }))));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
        let json: Json = json::from_str(&rc.1).unwrap();
        let bob_uuid = json["result"]["uuid"].as_str().unwrap().to_owned();

        log!("Get MYCOIN/MYCOIN1 orderbook");
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook "[bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let withdraw = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "withdraw",
            "coin": "MYCOIN",
            "max": true,
            "to": "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF",
        }))));
        assert!(withdraw.0.is_success(), "!withdraw: {}", withdraw.1);

        let withdraw: Json = unwrap!(json::from_str(&withdraw.1));

        let send_raw = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "send_raw_transaction",
            "coin": "MYCOIN",
            "tx_hex": withdraw["tx_hex"],
        }))));
        assert!(send_raw.0.is_success(), "!send_raw: {}", send_raw.1);

        thread::sleep(Duration::from_secs(12));

        log!("Get MYCOIN/MYCOIN1 orderbook");
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook "(unwrap!(json::to_string(&bob_orderbook))));
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 0, "MYCOIN/MYCOIN1 orderbook must have exactly 0 asks");

        log!("Get my orders");
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "my_orders",
        }))));
        assert!(rc.0.is_success(), "!my_orders: {}", rc.1);
        let orders: Json = unwrap!(json::from_str(&rc.1));
        log!("my_orders "(unwrap!(json::to_string(&orders))));
        assert!(
            unwrap!(orders["result"]["maker_orders"].as_object()).is_empty(),
            "maker_orders must be empty"
        );

        let rmd160 = rmd160_from_priv(priv_key);
        let order_path = mm_bob.folder.join(format!(
            "DB/{}/ORDERS/MY/MAKER/{}.json",
            hex::encode(rmd160.take()),
            bob_uuid,
        ));
        log!("Order path "(order_path.display()));
        assert!(!order_path.exists());
        unwrap!(block_on(mm_bob.stop()));
    }

    #[test]
    fn order_should_be_updated_when_balance_is_decreased_alice_subscribes_after_update() {
        let (_ctx, _, priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = unwrap!(MarketMakerIt::start(
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
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(60., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let mut mm_alice = unwrap!(MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": "alice passphrase",
                "coins": coins,
                "rpc_password": "pass",
                "seednodes": vec![format!("{}", mm_bob.ip)],
            }),
            "pass".to_string(),
            None,
        ));
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);

        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "999",
        }))));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        log!("Get MYCOIN/MYCOIN1 orderbook");
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook "[bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let withdraw = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "withdraw",
            "coin": "MYCOIN",
            "amount": "499.99998",
            "to": "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF",
        }))));
        assert!(withdraw.0.is_success(), "!withdraw: {}", withdraw.1);

        let withdraw: Json = unwrap!(json::from_str(&withdraw.1));

        let send_raw = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "send_raw_transaction",
            "coin": "MYCOIN",
            "tx_hex": withdraw["tx_hex"],
        }))));
        assert!(send_raw.0.is_success(), "!send_raw: {}", send_raw.1);

        thread::sleep(Duration::from_secs(12));

        log!("Get MYCOIN/MYCOIN1 orderbook on Bob side");
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook "(unwrap!(json::to_string(&bob_orderbook))));
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let order_volume = asks[0]["maxvolume"].as_str().unwrap();
        assert_eq!("500", order_volume);

        log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook "(unwrap!(json::to_string(&alice_orderbook))));
        let asks = alice_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Alice MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let order_volume = asks[0]["maxvolume"].as_str().unwrap();
        assert_eq!("500", order_volume);

        unwrap!(block_on(mm_bob.stop()));
        unwrap!(block_on(mm_alice.stop()));
    }

    #[test]
    fn order_should_be_updated_when_balance_is_decreased_alice_subscribes_before_update() {
        let (_ctx, _, priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = unwrap!(MarketMakerIt::start(
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
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(60., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let mut mm_alice = unwrap!(MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": "alice passphrase",
                "coins": coins,
                "rpc_password": "pass",
                "seednodes": vec![format!("{}", mm_bob.ip)],
            }),
            "pass".to_string(),
            None,
        ));
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);

        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "999",
        }))));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        log!("Get MYCOIN/MYCOIN1 orderbook");
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook "[bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        thread::sleep(Duration::from_secs(2));
        log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook "(unwrap!(json::to_string(&alice_orderbook))));
        let asks = alice_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Alice MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let withdraw = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "withdraw",
            "coin": "MYCOIN",
            "amount": "499.99998",
            "to": "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF",
        }))));
        assert!(withdraw.0.is_success(), "!withdraw: {}", withdraw.1);

        let withdraw: Json = unwrap!(json::from_str(&withdraw.1));

        let send_raw = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "send_raw_transaction",
            "coin": "MYCOIN",
            "tx_hex": withdraw["tx_hex"],
        }))));
        assert!(send_raw.0.is_success(), "!send_raw: {}", send_raw.1);

        thread::sleep(Duration::from_secs(12));

        log!("Get MYCOIN/MYCOIN1 orderbook on Bob side");
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook "(unwrap!(json::to_string(&bob_orderbook))));
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let order_volume = asks[0]["maxvolume"].as_str().unwrap();
        assert_eq!("500", order_volume);

        log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook "(unwrap!(json::to_string(&alice_orderbook))));
        let asks = alice_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Alice MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let order_volume = asks[0]["maxvolume"].as_str().unwrap();
        assert_eq!("500", order_volume);

        unwrap!(block_on(mm_bob.stop()));
        unwrap!(block_on(mm_alice.stop()));
    }

    #[test]
    fn test_order_should_be_updated_when_matched_partially() {
        let (_ctx, _, bob_priv_key) = generate_coin_with_random_privkey("MYCOIN", 2000.into());
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", 2000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = unwrap!(MarketMakerIt::start(
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
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let mut mm_alice = unwrap!(MarketMakerIt::start(
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
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);

        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "1000",
        }))));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "500",
        }))));
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        unwrap!(block_on(mm_bob.wait_for_log(22., |log| {
            log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1")
        })));
        unwrap!(block_on(mm_alice.wait_for_log(22., |log| {
            log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")
        })));

        log!("Get MYCOIN/MYCOIN1 orderbook on Bob side");
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook "(unwrap!(json::to_string(&bob_orderbook))));
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let order_volume = asks[0]["maxvolume"].as_str().unwrap();
        assert_eq!("500", order_volume);

        log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook "(unwrap!(json::to_string(&alice_orderbook))));
        let asks = alice_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Alice MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        unwrap!(block_on(mm_bob.stop()));
        unwrap!(block_on(mm_alice.stop()));
    }

    // https://github.com/KomodoPlatform/atomicDEX-API/issues/471
    #[test]
    fn test_match_and_trade_max() {
        let (_ctx, _, bob_priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", 2000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = unwrap!(MarketMakerIt::start(
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
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let mut mm_alice = unwrap!(MarketMakerIt::start(
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
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        }))));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
        let json: Json = json::from_str(&rc.1).unwrap();
        let bob_uuid = json["result"]["uuid"].as_str().unwrap().to_owned();

        log!("Get MYCOIN/MYCOIN1 orderbook");
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("orderbook "[bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");
        assert_eq!(asks[0]["maxvolume"], Json::from("999.99999"));

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "999.99999",
        }))));
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        unwrap!(block_on(mm_bob.wait_for_log(22., |log| {
            log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1")
        })));
        unwrap!(block_on(mm_alice.wait_for_log(22., |log| {
            log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")
        })));

        thread::sleep(Duration::from_secs(3));

        let rmd160 = rmd160_from_priv(bob_priv_key);
        let order_path = mm_bob.folder.join(format!(
            "DB/{}/ORDERS/MY/MAKER/{}.json",
            hex::encode(rmd160.take()),
            bob_uuid,
        ));
        log!("Order path "(order_path.display()));
        assert!(!order_path.exists());
        unwrap!(block_on(mm_bob.stop()));
        unwrap!(block_on(mm_alice.stop()));
    }

    #[test]
    fn test_buy_when_coins_locked_by_other_swap() {
        let (_ctx, _, bob_priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", 2.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = unwrap!(MarketMakerIt::start(
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
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let mut mm_alice = unwrap!(MarketMakerIt::start(
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
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        }))));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            // the result of equation x + x / 777 + 0.00002 = 1
            "volume": {
                "numer":"77698446",
                "denom":"77800000"
            },
        }))));
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        unwrap!(block_on(mm_bob.wait_for_log(22., |log| {
            log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1")
        })));
        unwrap!(block_on(mm_alice.wait_for_log(22., |log| {
            log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")
        })));
        // TODO when buy call is made immediately swap might be not put into swap ctx yet so locked
        // amount returns 0
        thread::sleep(Duration::from_secs(3));

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            // it is slightly more than previous volume so it should fail
            // because the total sum of used funds will be slightly more than available 2
            "volume": {
                "numer":"77698447",
                "denom":"77800000"
            },
        }))));
        assert!(!rc.0.is_success(), "buy success, but should fail: {}", rc.1);
        assert!(rc.1.contains("is larger than available"), rc.1);
        unwrap!(block_on(mm_bob.stop()));
        unwrap!(block_on(mm_alice.stop()));
    }

    #[test]
    fn test_sell_when_coins_locked_by_other_swap() {
        let (_ctx, _, bob_priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", 2.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = unwrap!(MarketMakerIt::start(
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
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let mut mm_alice = unwrap!(MarketMakerIt::start(
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
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        }))));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "sell",
            "base": "MYCOIN1",
            "rel": "MYCOIN",
            "price": 1,
            // the result of equation x + x / 777 + 0.00002 = 1
            "volume": {
                "numer":"77698446",
                "denom":"77800000"
            },
        }))));
        assert!(rc.0.is_success(), "!sell: {}", rc.1);

        unwrap!(block_on(mm_bob.wait_for_log(22., |log| {
            log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1")
        })));
        unwrap!(block_on(mm_alice.wait_for_log(22., |log| {
            log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")
        })));
        // TODO when sell call is made immediately swap might be not put into swap ctx yet so locked
        // amount returns 0
        thread::sleep(Duration::from_secs(3));

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "sell",
            "base": "MYCOIN1",
            "rel": "MYCOIN",
            "price": 1,
            // it is slightly more than previous volume so it should fail
            // because the total sum of used funds will be slightly more than available 2
            "volume": {
                "numer":"77698447",
                "denom":"77800000"
            },
        }))));
        assert!(!rc.0.is_success(), "sell success, but should fail: {}", rc.1);
        assert!(rc.1.contains("is larger than available"), rc.1);
        unwrap!(block_on(mm_bob.stop()));
        unwrap!(block_on(mm_alice.stop()));
    }

    #[test]
    fn test_buy_max() {
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", 1.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_alice = unwrap!(MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_see": true,
            }),
            "pass".to_string(),
            None,
        ));
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);
        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            // the result of equation x + x / 777 + 0.00002 = 1
            "volume": {
                "numer":"77698446",
                "denom":"77800000"
            },
        }))));
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            // it is slightly more than previous volume so it should fail
            "volume": {
                "numer":"77698447",
                "denom":"77800000"
            },
        }))));
        assert!(!rc.0.is_success(), "buy success, but should fail: {}", rc.1);
        // assert! (rc.1.contains("MYCOIN1 balance 1 is too low"));
        unwrap!(block_on(mm_alice.stop()));
    }

    #[test]
    fn test_get_max_taker_vol() {
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", 1.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_alice = unwrap!(MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_see": true,
            }),
            "pass".to_string(),
            None,
        ));
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "max_taker_vol",
            "coin": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!max_taker_vol: {}", rc.1);
        let json: Json = json::from_str(&rc.1).unwrap();
        // the result of equation x + x / 777 + 0.00002 = 1
        assert_eq!(json["result"]["numer"], Json::from("38849223"));
        assert_eq!(json["result"]["denom"], Json::from("38900000"));

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "sell",
            "base": "MYCOIN1",
            "rel": "MYCOIN",
            "price": 1,
            "volume": {
                "numer": json["result"]["numer"],
                "denom": json["result"]["denom"],
            }
        }))));
        assert!(rc.0.is_success(), "!sell: {}", rc.1);

        unwrap!(block_on(mm_alice.stop()));
    }

    // https://github.com/KomodoPlatform/atomicDEX-API/issues/733
    #[test]
    fn test_get_max_taker_vol_dex_fee_threshold() {
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", "0.05328455".parse().unwrap());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":10000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":10000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_alice = unwrap!(MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_see": true,
            }),
            "pass".to_string(),
            None,
        ));
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "max_taker_vol",
            "coin": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!max_taker_vol: {}", rc.1);
        let json: Json = json::from_str(&rc.1).unwrap();
        // the result of equation x + 0.0001 (dex fee) + 0.0002 (miner fee * 2) = 0.05328455
        assert_eq!(json["result"]["numer"], Json::from("1059691"));
        assert_eq!(json["result"]["denom"], Json::from("20000000"));

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "sell",
            "base": "MYCOIN1",
            "rel": "MYCOIN",
            "price": 1,
            "volume": {
                "numer": json["result"]["numer"],
                "denom": json["result"]["denom"],
            }
        }))));
        assert!(rc.0.is_success(), "!sell: {}", rc.1);

        unwrap!(block_on(mm_alice.stop()));
    }

    #[test]
    fn test_get_max_taker_vol_with_kmd() {
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", 1.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":10000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":10000,"protocol":{"type":"UTXO"}},
            {"coin":"KMD","txversion":4,"overwintered":1,"txfee":10000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_alice = unwrap!(MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_see": true,
            }),
            "pass".to_string(),
            None,
        ));
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_electrum(&mm_alice, "KMD", vec![
            "electrum1.cipig.net:10001",
            "electrum2.cipig.net:10001",
            "electrum3.cipig.net:10001"
        ]))]);
        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "max_taker_vol",
            "coin": "MYCOIN1",
            "trade_with": "KMD",
        }))));
        assert!(rc.0.is_success(), "!max_taker_vol: {}", rc.1);
        let json: Json = json::from_str(&rc.1).unwrap();
        // the result of equation x + x * 9 / 7770 + 0.0002 = 1
        assert_eq!(json["result"]["numer"], Json::from("1294741"));
        assert_eq!(json["result"]["denom"], Json::from("1296500"));

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "sell",
            "base": "MYCOIN1",
            "rel": "KMD",
            "price": 1,
            "volume": {
                "numer": json["result"]["numer"],
                "denom": json["result"]["denom"],
            }
        }))));
        assert!(rc.0.is_success(), "!sell: {}", rc.1);

        unwrap!(block_on(mm_alice.stop()));
    }

    #[test]
    fn test_set_price_max() {
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN", 1.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_alice = unwrap!(MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_see": true,
            }),
            "pass".to_string(),
            None,
        ));
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);
        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            // the result of equation x + 0.00001 = 1
            "volume": {
                "numer":"99999",
                "denom":"100000"
            },
        }))));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            // it is slightly more than previous volume so it should fail
            "volume": {
                "numer":"100000",
                "denom":"100000"
            },
        }))));
        assert!(!rc.0.is_success(), "setprice success, but should fail: {}", rc.1);
        unwrap!(block_on(mm_alice.stop()));
    }

    #[test]
    fn swaps_should_stop_on_stop_rpc() {
        let (_ctx, _, bob_priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", 2000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = unwrap!(MarketMakerIt::start(
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
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let mut mm_alice = unwrap!(MarketMakerIt::start(
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
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        }))));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
        let mut uuids = Vec::with_capacity(3);

        for _ in 0..3 {
            let rc = unwrap!(block_on(mm_alice.rpc(json! ({
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
            unwrap!(block_on(mm_bob.wait_for_log(22., |log| log.contains(&format!(
                "Entering the maker_swap_loop MYCOIN/MYCOIN1 with uuid: {}",
                uuid
            )))));
            unwrap!(block_on(mm_alice.wait_for_log(22., |log| log.contains(&format!(
                "Entering the taker_swap_loop MYCOIN/MYCOIN1 with uuid: {}",
                uuid
            )))));
        }
        thread::sleep(Duration::from_secs(3));
        unwrap!(block_on(mm_bob.stop()));
        unwrap!(block_on(mm_alice.stop()));
        for uuid in uuids {
            unwrap!(block_on(mm_bob.wait_for_log_after_stop(22., |log| {
                log.contains(&format!("swap {} stopped", uuid))
            })));
            unwrap!(block_on(mm_alice.wait_for_log_after_stop(22., |log| {
                log.contains(&format!("swap {} stopped", uuid))
            })));
        }
    }

    #[test]
    fn test_maker_order_should_kick_start_and_appear_in_orderbook_on_restart() {
        let (_ctx, _, bob_priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut bob_conf = json! ({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        });
        let mut mm_bob = unwrap!(MarketMakerIt::start(bob_conf.clone(), "pass".to_string(), None,));
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        }))));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        // mm_bob using same DB dir that should kick start the order
        bob_conf["dbdir"] = mm_bob.folder.join("DB").to_str().unwrap().into();
        bob_conf["log"] = mm_bob.folder.join("mm2_dup.log").to_str().unwrap().into();
        unwrap!(block_on(mm_bob.stop()));

        let mut mm_bob_dup = unwrap!(MarketMakerIt::start(bob_conf, "pass".to_string(), None,));
        let (_bob_dup_dump_log, _bob_dup_dump_dashboard) = mm_dump(&mm_bob_dup.log_path);
        unwrap!(block_on(
            mm_bob_dup.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));
        log!([block_on(enable_native(&mm_bob_dup, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob_dup, "MYCOIN1", vec![]))]);

        thread::sleep(Duration::from_secs(2));

        log!("Get RICK/MORTY orderbook on Bob side");
        let rc = unwrap!(block_on(mm_bob_dup.rpc(json! ({
            "userpass": mm_bob_dup.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("Bob orderbook "[bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 asks");
    }

    #[test]
    fn test_maker_order_kick_start_should_trigger_subscription_and_match() {
        let (_ctx, _, bob_priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", 1000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);

        let relay_conf = json! ({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": "relay",
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        });
        let mut relay = unwrap!(MarketMakerIt::start(relay_conf, "pass".to_string(), None,));
        let (_relay_dump_log, _relay_dump_dashboard) = mm_dump(&relay.log_path);
        unwrap!(block_on(
            relay.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let mut bob_conf = json! ({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": format!("0x{}", hex::encode(bob_priv_key)),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": vec![format!("{}", relay.ip)],
            "i_am_seed": false,
        });
        let mut mm_bob = unwrap!(MarketMakerIt::start(bob_conf.clone(), "pass".to_string(), None,));
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let mut mm_alice = unwrap!(MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "seednodes": vec![format!("{}", relay.ip)],
            }),
            "pass".to_string(),
            None,
        ));
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);

        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        }))));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        // mm_bob using same DB dir that should kick start the order
        bob_conf["dbdir"] = mm_bob.folder.join("DB").to_str().unwrap().into();
        bob_conf["log"] = mm_bob.folder.join("mm2_dup.log").to_str().unwrap().into();
        unwrap!(block_on(mm_bob.stop()));

        let mut mm_bob_dup = unwrap!(MarketMakerIt::start(bob_conf, "pass".to_string(), None,));
        let (_bob_dup_dump_log, _bob_dup_dump_dashboard) = mm_dump(&mm_bob_dup.log_path);
        unwrap!(block_on(
            mm_bob_dup.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));
        log!([block_on(enable_native(&mm_bob_dup, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob_dup, "MYCOIN1", vec![]))]);

        log!("Give restarted Bob 2 seconds to kickstart the order");
        thread::sleep(Duration::from_secs(2));

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": 1,
        }))));
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        unwrap!(block_on(mm_bob_dup.wait_for_log(22., |log| {
            log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1")
        })));
        unwrap!(block_on(mm_alice.wait_for_log(22., |log| {
            log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")
        })));
    }

    #[test]
    fn test_orders_should_match_on_both_nodes_with_same_priv() {
        let (_ctx, _, bob_priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", 2000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = unwrap!(MarketMakerIt::start(
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
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let mut mm_alice_1 = unwrap!(MarketMakerIt::start(
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
        let (_alice_1_dump_log, _alice_1_dump_dashboard) = mm_dump(&mm_alice_1.log_path);
        unwrap!(block_on(
            mm_alice_1.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let mut mm_alice_2 = unwrap!(MarketMakerIt::start(
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
        let (_alice_2_dump_log, _alice_2_dump_dashboard) = mm_dump(&mm_alice_2.log_path);
        unwrap!(block_on(
            mm_alice_2.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice_1, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice_1, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice_2, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice_2, "MYCOIN1", vec![]))]);

        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        }))));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = unwrap!(block_on(mm_alice_1.rpc(json! ({
            "userpass": mm_alice_1.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "1",
        }))));
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        unwrap!(block_on(mm_alice_1.wait_for_log(22., |log| {
            log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")
        })));

        let rc = unwrap!(block_on(mm_alice_2.rpc(json! ({
            "userpass": mm_alice_2.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "1",
        }))));
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        unwrap!(block_on(mm_alice_2.wait_for_log(22., |log| {
            log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")
        })));

        unwrap!(block_on(mm_bob.stop()));
        unwrap!(block_on(mm_alice_1.stop()));
        unwrap!(block_on(mm_alice_2.stop()));
    }

    #[test]
    fn test_maker_and_taker_order_created_with_same_priv_should_not_match() {
        let (_ctx, coin, priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, coin1, _) = generate_coin_with_random_privkey("MYCOIN1", 1000.into());
        fill_address(&coin1, &coin.my_address().unwrap(), 1000.into(), now_ms() / 1000 + 60);
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = unwrap!(MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        ));
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let mut mm_alice = unwrap!(MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "seednodes": vec![format!("{}", mm_bob.ip)],
            }),
            "pass".to_string(),
            None,
        ));
        let (_alice_1_dump_log, _alice_1_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);

        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        }))));
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "1",
        }))));
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        block_on(mm_bob.wait_for_log(5., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap_err();
        block_on(mm_alice.wait_for_log(5., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap_err();

        unwrap!(block_on(mm_bob.stop()));
        unwrap!(block_on(mm_alice.stop()));
    }

    #[test]
    fn test_taker_order_converted_to_maker_should_cancel_properly_when_matched() {
        let (_ctx, _, bob_priv_key) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_coin_with_random_privkey("MYCOIN1", 2000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = unwrap!(MarketMakerIt::start(
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
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let mut mm_alice = unwrap!(MarketMakerIt::start(
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
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        unwrap!(block_on(
            mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        log!([block_on(enable_native(&mm_bob, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", vec![]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", vec![]))]);
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "sell",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": 1,
        }))));
        assert!(rc.0.is_success(), "!sell: {}", rc.1);

        log!("Give Bob 35 seconds to convert order to maker");
        thread::sleep(Duration::from_secs(35));

        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": 1,
        }))));
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        unwrap!(block_on(mm_bob.wait_for_log(22., |log| {
            log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1")
        })));
        unwrap!(block_on(mm_alice.wait_for_log(22., |log| {
            log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")
        })));

        log!("Give Bob 2 seconds to cancel the order");
        thread::sleep(Duration::from_secs(2));
        log!("Get my_orders on Bob side");
        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "my_orders",
        }))));
        assert!(rc.0.is_success(), "!my_orders: {}", rc.1);
        let my_orders_json: Json = json::from_str(&rc.1).unwrap();
        let maker_orders: HashMap<String, Json> =
            json::from_value(my_orders_json["result"]["maker_orders"].clone()).unwrap();
        assert!(maker_orders.is_empty());

        let rc = unwrap!(block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("Bob orderbook "[bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 0, "Bob MYCOIN/MYCOIN1 orderbook must be empty");

        log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
        let rc = unwrap!(block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        }))));
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let alice_orderbook: Json = unwrap!(json::from_str(&rc.1));
        log!("Alice orderbook "[alice_orderbook]);
        let asks = alice_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 0, "Alice MYCOIN/MYCOIN1 orderbook must be empty");

        unwrap!(block_on(mm_bob.stop()));
        unwrap!(block_on(mm_alice.stop()));
    }

    #[test]
    fn test_utxo_merge() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, privkey) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        // fill several times to have more UTXOs on address
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);

        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = unwrap!(MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(privkey)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        ));
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let native = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "enable",
            "coin": "MYCOIN",
            "mm2": 1,
            "utxo_merge_params": {
                "merge_at": 2,
                "check_every": 1,
            }
        })))
        .unwrap();
        assert!(native.0.is_success(), "'enable' failed: {}", native.1);
        log!("Enable result "(native.1));

        unwrap!(block_on(mm_bob.wait_for_log(4., |log| {
            log.contains("Starting UTXO merge loop for coin MYCOIN")
        })));

        unwrap!(block_on(mm_bob.wait_for_log(4., |log| {
            log.contains("Trying to merge 5 UTXOs of coin MYCOIN")
        })));

        unwrap!(block_on(mm_bob.wait_for_log(4., |log| {
            log.contains("UTXO merge successful for coin MYCOIN, tx_hash")
        })));

        thread::sleep(Duration::from_secs(2));
        let (unspents, _) = block_on(coin.list_unspent_ordered(&coin.as_ref().my_address)).unwrap();
        assert_eq!(unspents.len(), 1);
    }

    #[test]
    fn test_utxo_merge_max_merge_at_once() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, privkey) = generate_coin_with_random_privkey("MYCOIN", 1000.into());
        // fill several times to have more UTXOs on address
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);

        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = unwrap!(MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(privkey)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        ));
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        unwrap!(block_on(
            mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))
        ));

        let native = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "enable",
            "coin": "MYCOIN",
            "mm2": 1,
            "utxo_merge_params": {
                "merge_at": 3,
                "check_every": 1,
                "max_merge_at_once": 4,
            }
        })))
        .unwrap();
        assert!(native.0.is_success(), "'enable' failed: {}", native.1);
        log!("Enable result "(native.1));

        unwrap!(block_on(mm_bob.wait_for_log(4., |log| {
            log.contains("Starting UTXO merge loop for coin MYCOIN")
        })));

        unwrap!(block_on(mm_bob.wait_for_log(4., |log| {
            log.contains("Trying to merge 4 UTXOs of coin MYCOIN")
        })));

        unwrap!(block_on(mm_bob.wait_for_log(4., |log| {
            log.contains("UTXO merge successful for coin MYCOIN, tx_hash")
        })));

        thread::sleep(Duration::from_secs(2));
        let (unspents, _) = block_on(coin.list_unspent_ordered(&coin.as_ref().my_address)).unwrap();
        // 4 utxos are merged of 5 so the resulting unspents len must be 2
        assert_eq!(unspents.len(), 2);
    }
}
