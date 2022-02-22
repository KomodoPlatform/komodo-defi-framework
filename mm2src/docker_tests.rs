#![feature(async_closure)]
#![feature(custom_test_frameworks)]
#![feature(test)]
#![test_runner(docker_tests_runner)]
#![feature(drain_filter)]
#![feature(hash_raw_entry)]
#![feature(map_first_last)]
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
#[cfg(all(test, not(target_arch = "wasm32")))]
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
#[cfg(test)]
#[macro_use]
extern crate ser_error_derive;
#[cfg(test)] extern crate test;

#[cfg(test)]
#[path = "mm2.rs"]
pub mod mm2;

fn main() { unimplemented!() }

/// rustfmt cannot resolve the module path within docker_tests.
/// Specify the path manually outside the docker_tests.
#[cfg(rustfmt)]
#[path = "docker_tests/docker_ordermatch_tests.rs"]
mod docker_ordermatch_tests;

#[cfg(rustfmt)]
#[path = "docker_tests/swaps_confs_settings_sync_tests.rs"]
mod swaps_confs_settings_sync_tests;

#[cfg(rustfmt)]
#[path = "docker_tests/swaps_file_lock_tests.rs"]
mod swaps_file_lock_tests;

#[cfg(rustfmt)]
#[path = "docker_tests/qrc20_tests.rs"]
mod qrc20_tests;

#[cfg(all(test, target_arch = "wasm32"))]
mod docker_tests {
    use test::{test_main, StaticBenchFn, StaticTestFn, TestDescAndFn};

    pub fn docker_tests_runner(tests: &[&TestDescAndFn]) {
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
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod docker_tests {
    #[rustfmt::skip]
    mod docker_ordermatch_tests;
    #[rustfmt::skip]
    mod docker_tests_common;
    #[rustfmt::skip]
    mod qrc20_tests;
    #[rustfmt::skip]
    mod slp_tests;
    #[rustfmt::skip]
    mod swaps_confs_settings_sync_tests;
    #[rustfmt::skip]
    mod swaps_file_lock_tests;

    use docker_tests_common::*;

    use bigdecimal::BigDecimal;
    use bitcrypto::ChecksumType;
    use chain::{OutPoint, TransactionOutput};
    use coins::eth::{eth_coin_from_conf_and_request, EthCoin};
    use coins::utxo::bch::{bch_coin_from_conf_and_params, BchActivationRequest, BchCoin};
    use coins::utxo::rpc_clients::{UnspentInfo, UtxoRpcClientEnum};
    use coins::utxo::slp::SlpToken;
    use coins::utxo::slp::{slp_genesis_output, SlpOutput};
    use coins::utxo::utxo_common::send_outputs_from_my_address;
    use coins::utxo::utxo_standard::{utxo_standard_coin_with_priv_key, UtxoStandardCoin};
    use coins::utxo::{dhash160, UtxoActivationParams, UtxoCommonOps};
    use coins::{CoinProtocol, FoundSwapTxSpend, MarketCoinOps, MmCoin, SwapOps, Transaction, TransactionEnum,
                WithdrawRequest};
    use common::for_tests::{check_my_swap_status_amounts, enable_electrum};
    use common::mm_ctx::{MmArc, MmCtxBuilder};
    use common::mm_number::MmNumber;
    use common::privkey::{key_pair_from_secret, key_pair_from_seed};
    use common::{block_on, now_ms};
    use futures01::Future;
    use keys::{Address, KeyPair, NetworkPrefix as CashAddrPrefix, Private};
    use qrc20_tests::{qtum_docker_node, QtumDockerOps, QTUM_REGTEST_DOCKER_IMAGE};
    use script::Builder;
    use secp256k1::SecretKey;
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
            let priv_key = hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap();
            let ctx = MmCtxBuilder::new().into_mm_arc();
            let params = UtxoActivationParams::from_legacy_req(&req).unwrap();

            let coin = block_on(utxo_standard_coin_with_priv_key(
                &ctx, ticker, &conf, &params, &priv_key,
            ))
            .unwrap();
            UtxoAssetDockerOps { ctx, coin }
        }
    }

    struct BchDockerOps {
        #[allow(dead_code)]
        ctx: MmArc,
        coin: BchCoin,
    }

    // builds the EthCoin using the external dev Parity/OpenEthereum node
    // the address belonging to the default passphrase has million of ETH that it can distribute to
    // random privkeys generated in tests
    fn eth_distributor() -> EthCoin {
        let conf = json!({"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}});
        let req = json!({
            "method": "enable",
            "coin": "ETH",
            "urls": ["http://195.201.0.6:8565"],
            "swap_contract_address": "0xa09ad3cd7e96586ebd05a2607ee56b56fb2db8fd",
        });
        let keypair =
            key_pair_from_seed("spice describe gravity federal blast come thank unfair canal monkey style afraid")
                .unwrap();
        block_on(eth_coin_from_conf_and_request(
            &MM_CTX,
            "ETH",
            &conf,
            &req,
            &*keypair.private().secret,
            CoinProtocol::ETH,
        ))
        .unwrap()
    }

    // pass address without 0x prefix to this fn
    fn fill_eth(to_addr: &str) {
        ETH_DISTRIBUTOR
            .send_to_address(to_addr.parse().unwrap(), 1_000_000_000_000_000_000u64.into())
            .wait()
            .unwrap();
    }

    lazy_static! {
        static ref COINS_LOCK: Mutex<()> = Mutex::new(());
        static ref ETH_DISTRIBUTOR: EthCoin = eth_distributor();
        static ref MM_CTX: MmArc = MmCtxBuilder::new().into_mm_arc();
    }

    impl BchDockerOps {
        fn from_ticker(ticker: &str) -> BchDockerOps {
            let conf = json!({"asset": ticker,"txfee":1000,"network": "regtest","txversion":4,"overwintered":1});
            let req = json!({"method":"enable", "bchd_urls": [], "allow_slp_unsafe_conf": true});
            let priv_key = hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap();
            let ctx = MmCtxBuilder::new().into_mm_arc();
            let params = BchActivationRequest::from_legacy_req(&req).unwrap();

            let coin = block_on(bch_coin_from_conf_and_params(
                &ctx,
                ticker,
                &conf,
                params,
                CashAddrPrefix::SlpTest,
                &priv_key,
            ))
            .unwrap();
            BchDockerOps { ctx, coin }
        }

        fn initialize_slp(&self) {
            fill_address(&self.coin, &self.coin.my_address().unwrap(), 100000.into(), 30);
            let mut slp_privkeys = vec![];

            let slp_genesis_op_ret = slp_genesis_output("ADEXSLP", "ADEXSLP", None, None, 8, None, 1000000_00000000);
            let slp_genesis = TransactionOutput {
                value: self.coin.as_ref().dust_amount,
                script_pubkey: Builder::build_p2pkh(&self.coin.my_public_key().unwrap().address_hash().into())
                    .to_bytes(),
            };

            let mut bch_outputs = vec![slp_genesis_op_ret, slp_genesis];
            let mut slp_outputs = vec![];

            for _ in 0..18 {
                let priv_key = *(SecretKey::new(&mut rand6::thread_rng()).as_ref());
                let key_pair = key_pair_from_secret(priv_key).unwrap();
                let address_hash = key_pair.public().address_hash();
                let address = Address {
                    prefix: self.coin.as_ref().conf.pub_addr_prefix,
                    t_addr_prefix: self.coin.as_ref().conf.pub_t_addr_prefix,
                    hrp: None,
                    hash: address_hash.into(),
                    checksum_type: Default::default(),
                    addr_format: Default::default(),
                };

                self.native_client()
                    .import_address(&address.to_string(), &address.to_string(), false)
                    .wait()
                    .unwrap();

                let script_pubkey = Builder::build_p2pkh(&address_hash.into());

                bch_outputs.push(TransactionOutput {
                    value: 1000_00000000,
                    script_pubkey: script_pubkey.to_bytes(),
                });

                slp_outputs.push(SlpOutput {
                    amount: 1000_00000000,
                    script_pubkey: script_pubkey.to_bytes(),
                });
                slp_privkeys.push(priv_key);
            }

            let slp_genesis_tx = send_outputs_from_my_address(self.coin.clone(), bch_outputs)
                .wait()
                .unwrap();
            self.coin
                .wait_for_confirmations(&slp_genesis_tx.tx_hex(), 1, false, now_ms() / 1000 + 30, 1)
                .wait()
                .unwrap();

            let adex_slp = SlpToken::new(
                8,
                "ADEXSLP".into(),
                slp_genesis_tx.tx_hash().as_slice().into(),
                self.coin.clone(),
                1,
            );

            let tx = block_on(adex_slp.send_slp_outputs(slp_outputs)).unwrap();
            self.coin
                .wait_for_confirmations(&tx.tx_hex(), 1, false, now_ms() / 1000 + 30, 1)
                .wait()
                .unwrap();
            *SLP_TOKEN_OWNERS.lock().unwrap() = slp_privkeys;
            *SLP_TOKEN_ID.lock().unwrap() = slp_genesis_tx.tx_hash().as_slice().into();
        }
    }

    impl CoinDockerOps for BchDockerOps {
        fn rpc_client(&self) -> &UtxoRpcClientEnum { &self.coin.as_ref().rpc_client }
    }

    /// Generate random privkey, create a UTXO coin and fill it's address with the specified balance.
    fn generate_utxo_coin_with_random_privkey(
        ticker: &str,
        balance: BigDecimal,
    ) -> (MmArc, UtxoStandardCoin, [u8; 32]) {
        let priv_key = SecretKey::new(&mut rand6::thread_rng());
        let (ctx, coin) = utxo_coin_from_privkey(ticker, priv_key.as_ref());
        let timeout = 30; // timeout if test takes more than 30 seconds to run
        let my_address = coin.my_address().expect("!my_address");
        fill_address(&coin, &my_address, balance, timeout);
        (ctx, coin, *priv_key.as_ref())
    }

    #[test]
    fn test_search_for_swap_tx_spend_native_was_refunded_taker() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
        let my_public_key = coin.my_public_key().unwrap();

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin
            .send_taker_payment(time_lock, my_public_key, &[0; 20], 1u64.into(), &None)
            .wait()
            .unwrap();

        coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1)
            .wait()
            .unwrap();

        let refund_tx = coin
            .send_taker_refunds_payment(&tx.tx_hex(), time_lock, my_public_key, &[0; 20], &None)
            .wait()
            .unwrap();

        coin.wait_for_confirmations(&refund_tx.tx_hex(), 1, false, timeout, 1)
            .wait()
            .unwrap();

        let found = block_on(coin.search_for_swap_tx_spend_my(
            time_lock,
            &*coin.my_public_key().unwrap(),
            &[0; 20],
            &tx.tx_hex(),
            0,
            &None,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(FoundSwapTxSpend::Refunded(refund_tx), found);
    }

    #[test]
    fn test_search_for_swap_tx_spend_native_was_refunded_maker() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
        let my_public_key = coin.my_public_key().unwrap();

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin
            .send_maker_payment(time_lock, my_public_key, &[0; 20], 1u64.into(), &None)
            .wait()
            .unwrap();

        coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1)
            .wait()
            .unwrap();

        let refund_tx = coin
            .send_maker_refunds_payment(&tx.tx_hex(), time_lock, my_public_key, &[0; 20], &None)
            .wait()
            .unwrap();

        coin.wait_for_confirmations(&refund_tx.tx_hex(), 1, false, timeout, 1)
            .wait()
            .unwrap();

        let found = block_on(coin.search_for_swap_tx_spend_my(
            time_lock,
            &*coin.my_public_key().unwrap(),
            &[0; 20],
            &tx.tx_hex(),
            0,
            &None,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(FoundSwapTxSpend::Refunded(refund_tx), found);
    }

    #[test]
    fn test_search_for_taker_swap_tx_spend_native_was_spent_by_maker() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
        let secret = [0; 32];
        let my_public_key = coin.my_public_key().unwrap();

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin
            .send_taker_payment(time_lock, my_public_key, &*dhash160(&secret), 1u64.into(), &None)
            .wait()
            .unwrap();

        coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1)
            .wait()
            .unwrap();

        let spend_tx = coin
            .send_maker_spends_taker_payment(&tx.tx_hex(), time_lock, my_public_key, &secret, &None)
            .wait()
            .unwrap();

        coin.wait_for_confirmations(&spend_tx.tx_hex(), 1, false, timeout, 1)
            .wait()
            .unwrap();

        let found = block_on(coin.search_for_swap_tx_spend_my(
            time_lock,
            &*coin.my_public_key().unwrap(),
            &*dhash160(&secret),
            &tx.tx_hex(),
            0,
            &None,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(FoundSwapTxSpend::Spent(spend_tx), found);
    }

    #[test]
    fn test_search_for_maker_swap_tx_spend_native_was_spent_by_taker() {
        let timeout = (now_ms() / 1000) + 120; // timeout if test takes more than 120 seconds to run
        let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000u64.into());
        let secret = [0; 32];
        let my_public_key = coin.my_public_key().unwrap();

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let tx = coin
            .send_maker_payment(time_lock, my_public_key, &*dhash160(&secret), 1u64.into(), &None)
            .wait()
            .unwrap();

        coin.wait_for_confirmations(&tx.tx_hex(), 1, false, timeout, 1)
            .wait()
            .unwrap();

        let spend_tx = coin
            .send_taker_spends_maker_payment(&tx.tx_hex(), time_lock, my_public_key, &secret, &None)
            .wait()
            .unwrap();

        coin.wait_for_confirmations(&spend_tx.tx_hex(), 1, false, timeout, 1)
            .wait()
            .unwrap();

        let found = block_on(coin.search_for_swap_tx_spend_my(
            time_lock,
            &*coin.my_public_key().unwrap(),
            &*dhash160(&secret),
            &tx.tx_hex(),
            0,
            &None,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(FoundSwapTxSpend::Spent(spend_tx), found);
    }

    #[test]
    fn test_one_hundred_maker_payments_in_a_row_native() {
        let timeout = 30; // timeout if test takes more than 30 seconds to run
        let (_ctx, coin, _) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        let secret = [0; 32];
        let my_public_key = coin.my_public_key().unwrap();

        let time_lock = (now_ms() / 1000) as u32 - 3600;
        let mut unspents = vec![];
        let mut sent_tx = vec![];
        for i in 0..100 {
            let tx = coin
                .send_maker_payment(
                    time_lock + i,
                    my_public_key,
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
        let (_ctx, _, priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mm_bob = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
                "rpcip": env::var ("BOB_TRADE_IP") .ok(),
                "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
                "passphrase": format!("0x{}", hex::encode(priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "999",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
        let json: Json = json::from_str(&rc.1).unwrap();
        let bob_uuid = json["result"]["uuid"].as_str().unwrap().to_owned();

        log!("Get MYCOIN/MYCOIN1 orderbook");
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("orderbook "[bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let withdraw = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "withdraw",
            "coin": "MYCOIN",
            "max": true,
            "to": "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF",
        })))
        .unwrap();
        assert!(withdraw.0.is_success(), "!withdraw: {}", withdraw.1);

        let withdraw: Json = json::from_str(&withdraw.1).unwrap();

        let send_raw = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "send_raw_transaction",
            "coin": "MYCOIN",
            "tx_hex": withdraw["tx_hex"],
        })))
        .unwrap();
        assert!(send_raw.0.is_success(), "!send_raw: {}", send_raw.1);

        thread::sleep(Duration::from_secs(12));

        log!("Get MYCOIN/MYCOIN1 orderbook");
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("orderbook "(json::to_string(&bob_orderbook).unwrap()));
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 0, "MYCOIN/MYCOIN1 orderbook must have exactly 0 asks");

        log!("Get my orders");
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "my_orders",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!my_orders: {}", rc.1);
        let orders: Json = json::from_str(&rc.1).unwrap();
        log!("my_orders "(json::to_string(&orders).unwrap()));
        assert!(
            orders["result"]["maker_orders"].as_object().unwrap().is_empty(),
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
        block_on(mm_bob.stop()).unwrap();
    }

    #[test]
    fn order_should_be_updated_when_balance_is_decreased_alice_subscribes_after_update() {
        let (_ctx, _, priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mm_bob = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
                "rpcip": env::var ("BOB_TRADE_IP") .ok(),
                "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
                "passphrase": format!("0x{}", hex::encode(priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);

        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "999",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        log!("Get MYCOIN/MYCOIN1 orderbook");
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("orderbook "[bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let withdraw = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "withdraw",
            "coin": "MYCOIN",
            "amount": "499.99998",
            "to": "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF",
        })))
        .unwrap();
        assert!(withdraw.0.is_success(), "!withdraw: {}", withdraw.1);

        let withdraw: Json = json::from_str(&withdraw.1).unwrap();

        let send_raw = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "send_raw_transaction",
            "coin": "MYCOIN",
            "tx_hex": withdraw["tx_hex"],
        })))
        .unwrap();
        assert!(send_raw.0.is_success(), "!send_raw: {}", send_raw.1);

        thread::sleep(Duration::from_secs(12));

        log!("Get MYCOIN/MYCOIN1 orderbook on Bob side");
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("orderbook "(json::to_string(&bob_orderbook).unwrap()));
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let order_volume = asks[0]["maxvolume"].as_str().unwrap();
        assert_eq!("500", order_volume);

        log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let alice_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("orderbook "(json::to_string(&alice_orderbook).unwrap()));
        let asks = alice_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Alice MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let order_volume = asks[0]["maxvolume"].as_str().unwrap();
        assert_eq!("500", order_volume);

        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
    }

    #[test]
    fn order_should_be_updated_when_balance_is_decreased_alice_subscribes_before_update() {
        let (_ctx, _, priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mm_bob = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
                "rpcip": env::var ("BOB_TRADE_IP") .ok(),
                "canbind": env::var ("BOB_TRADE_PORT") .ok().map (|s| s.parse::<i64>().unwrap()),
                "passphrase": format!("0x{}", hex::encode(priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);

        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "999",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        log!("Get MYCOIN/MYCOIN1 orderbook");
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("orderbook "[bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        thread::sleep(Duration::from_secs(2));
        log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let alice_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("orderbook "(json::to_string(&alice_orderbook).unwrap()));
        let asks = alice_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Alice MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let withdraw = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "withdraw",
            "coin": "MYCOIN",
            "amount": "499.99998",
            "to": "R9imXLs1hEcU9KbFDQq2hJEEJ1P5UoekaF",
        })))
        .unwrap();
        assert!(withdraw.0.is_success(), "!withdraw: {}", withdraw.1);

        let withdraw: Json = json::from_str(&withdraw.1).unwrap();

        let send_raw = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "send_raw_transaction",
            "coin": "MYCOIN",
            "tx_hex": withdraw["tx_hex"],
        })))
        .unwrap();
        assert!(send_raw.0.is_success(), "!send_raw: {}", send_raw.1);

        thread::sleep(Duration::from_secs(12));

        log!("Get MYCOIN/MYCOIN1 orderbook on Bob side");
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("orderbook "(json::to_string(&bob_orderbook).unwrap()));
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let order_volume = asks[0]["maxvolume"].as_str().unwrap();
        assert_eq!("500", order_volume);

        log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let alice_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("orderbook "(json::to_string(&alice_orderbook).unwrap()));
        let asks = alice_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Alice MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let order_volume = asks[0]["maxvolume"].as_str().unwrap();
        assert_eq!("500", order_volume);

        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
    }

    #[test]
    fn test_order_should_be_updated_when_matched_partially() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 2000.into());
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mut mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);

        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "1000",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "500",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
        block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap();

        log!("Get MYCOIN/MYCOIN1 orderbook on Bob side");
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("orderbook "(json::to_string(&bob_orderbook).unwrap()));
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        let order_volume = asks[0]["maxvolume"].as_str().unwrap();
        assert_eq!("500", order_volume);

        log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let alice_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("orderbook "(json::to_string(&alice_orderbook).unwrap()));
        let asks = alice_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Alice MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");

        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
    }

    // https://github.com/KomodoPlatform/atomicDEX-API/issues/471
    #[test]
    fn test_match_and_trade_setprice_max() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mut mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
        let json: Json = json::from_str(&rc.1).unwrap();
        let bob_uuid = json["result"]["uuid"].as_str().unwrap().to_owned();

        log!("Get MYCOIN/MYCOIN1 orderbook");
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("orderbook "[bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "MYCOIN/MYCOIN1 orderbook must have exactly 1 ask");
        assert_eq!(asks[0]["maxvolume"], Json::from("999.99999"));

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "999.99999",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
        block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap();

        thread::sleep(Duration::from_secs(3));

        let rmd160 = rmd160_from_priv(bob_priv_key);
        let order_path = mm_bob.folder.join(format!(
            "DB/{}/ORDERS/MY/MAKER/{}.json",
            hex::encode(rmd160.take()),
            bob_uuid,
        ));
        log!("Order path "(order_path.display()));
        assert!(!order_path.exists());
        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
    }

    #[test]
    // https://github.com/KomodoPlatform/atomicDEX-API/issues/888
    fn test_max_taker_vol_swap() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 50.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = block_on(MarketMakerIt::start_with_envs(
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
            &[("MYCOIN_FEE_DISCOUNT", "")],
        ))
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);
        block_on(mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))).unwrap();

        let mut mm_alice = block_on(MarketMakerIt::start_with_envs(
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
            &[("MYCOIN_FEE_DISCOUNT", "")],
        ))
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);
        block_on(mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))).unwrap();

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);
        let price = MmNumber::from((100, 1620));
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": price,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "MYCOIN1",
            "rel": "MYCOIN",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);
        log!((rc.1));
        thread::sleep(Duration::from_secs(3));

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "max_taker_vol",
            "coin": "MYCOIN1",
            "trade_with": "MYCOIN",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!max_taker_vol: {}", rc.1);
        let vol: MaxTakerVolResponse = json::from_str(&rc.1).unwrap();
        let expected_vol = MmNumber::from((647499741, 12965000));

        let actual_vol = MmNumber::from(vol.result.clone());
        assert_eq!(expected_vol, actual_vol);

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "sell",
            "base": "MYCOIN1",
            "rel": "MYCOIN",
            "price": "16",
            "volume": vol.result,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!sell: {}", rc.1);
        let sell_res: BuyOrSellRpcResult = json::from_str(&rc.1).unwrap();

        block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
        block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap();

        thread::sleep(Duration::from_secs(3));

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "my_swap_status",
            "params": {
                "uuid": sell_res.result.uuid
            }
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!my_swap_status: {}", rc.1);

        let status_response: Json = json::from_str(&rc.1).unwrap();
        let events_array = status_response["result"]["events"].as_array().unwrap();
        let first_event_type = events_array[0]["event"]["type"].as_str().unwrap();
        assert_eq!("Started", first_event_type);
        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
    }

    #[test]
    fn test_buy_when_coins_locked_by_other_swap() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mut mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = block_on(mm_alice.rpc(json! ({
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
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
        block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap();
        // TODO when buy call is made immediately swap might be not put into swap ctx yet so locked
        // amount returns 0
        thread::sleep(Duration::from_secs(3));

        let rc = block_on(mm_alice.rpc(json! ({
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
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "buy success, but should fail: {}", rc.1);
        assert!(rc.1.contains("Not enough MYCOIN1 for swap"), "{}", rc.1);
        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
    }

    #[test]
    fn test_sell_when_coins_locked_by_other_swap() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mut mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = block_on(mm_alice.rpc(json! ({
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
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!sell: {}", rc.1);

        block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
        block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap();
        // TODO when sell call is made immediately swap might be not put into swap ctx yet so locked
        // amount returns 0
        thread::sleep(Duration::from_secs(3));

        let rc = block_on(mm_alice.rpc(json! ({
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
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "sell success, but should fail: {}", rc.1);
        assert!(rc.1.contains("Not enough MYCOIN1 for swap"), "{}", rc.1);
        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
    }

    #[test]
    fn test_buy_max() {
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 1.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mm_alice = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);
        let rc = block_on(mm_alice.rpc(json! ({
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
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        let rc = block_on(mm_alice.rpc(json! ({
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
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "buy success, but should fail: {}", rc.1);
        // assert! (rc.1.contains("MYCOIN1 balance 1 is too low"));
        block_on(mm_alice.stop()).unwrap();
    }

    #[test]
    fn test_maker_trade_preimage() {
        let priv_key = SecretKey::new(&mut rand6::thread_rng());

        let (_ctx, mycoin) = utxo_coin_from_privkey("MYCOIN", &priv_key[..]);
        let my_address = mycoin.my_address().expect("!my_address");
        fill_address(&mycoin, &my_address, 10.into(), 30);

        let (_ctx, mycoin1) = utxo_coin_from_privkey("MYCOIN1", &priv_key[..]);
        let my_address = mycoin1.my_address().expect("!my_address");
        fill_address(&mycoin1, &my_address, 20.into(), 30);

        let coins = json!([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":2000,"protocol":{"type":"UTXO"}},
        ]);
        let mm = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(&priv_key[..])),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);

        log!([block_on(enable_native(&mm, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm, "MYCOIN", &[]))]);

        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "swap_method": "setprice",
                "price": 1,
                "max": true,
            },
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);
        let base_coin_fee = TradeFeeForTest::new("MYCOIN", "0.00001", false);
        let rel_coin_fee = TradeFeeForTest::new("MYCOIN1", "0.00002", true);
        let volume = MmNumber::from("9.99999");

        let my_coin_total = TotalTradeFeeForTest::new("MYCOIN", "0.00001", "0.00001");
        let my_coin1_total = TotalTradeFeeForTest::new("MYCOIN1", "0.00002", "0");

        let expected = TradePreimageResult::MakerPreimage(MakerPreimage {
            base_coin_fee,
            rel_coin_fee,
            volume: Some(volume.to_decimal()),
            volume_rat: Some(volume.to_ratio()),
            volume_fraction: Some(volume.to_fraction()),
            total_fees: vec![my_coin_total, my_coin1_total],
        });

        let mut actual: RpcSuccessResponse<TradePreimageResult> = json::from_str(&rc.1).unwrap();
        actual.result.sort_total_fees();
        assert_eq!(expected, actual.result);

        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN1",
                "rel": "MYCOIN",
                "swap_method": "setprice",
                "price": 1,
                "max": true,
            },
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);
        let mut actual: RpcSuccessResponse<TradePreimageResult> = json::from_str(&rc.1).unwrap();
        actual.result.sort_total_fees();

        let base_coin_fee = TradeFeeForTest::new("MYCOIN1", "0.00002", false);
        let rel_coin_fee = TradeFeeForTest::new("MYCOIN", "0.00001", true);
        let volume = MmNumber::from("19.99998");

        let my_coin_total = TotalTradeFeeForTest::new("MYCOIN", "0.00001", "0");
        let my_coin1_total = TotalTradeFeeForTest::new("MYCOIN1", "0.00002", "0.00002");
        let expected = TradePreimageResult::MakerPreimage(MakerPreimage {
            base_coin_fee,
            rel_coin_fee,
            volume: Some(volume.to_decimal()),
            volume_rat: Some(volume.to_ratio()),
            volume_fraction: Some(volume.to_fraction()),
            total_fees: vec![my_coin_total, my_coin1_total],
        });

        actual.result.sort_total_fees();
        assert_eq!(expected, actual.result);

        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN1",
                "rel": "MYCOIN",
                "swap_method": "setprice",
                "price": 1,
                "volume": "19.99998",
            },
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);
        let mut actual: RpcSuccessResponse<TradePreimageResult> = json::from_str(&rc.1).unwrap();
        actual.result.sort_total_fees();

        let base_coin_fee = TradeFeeForTest::new("MYCOIN1", "0.00002", false);
        let rel_coin_fee = TradeFeeForTest::new("MYCOIN", "0.00001", true);

        let total_my_coin = TotalTradeFeeForTest::new("MYCOIN", "0.00001", "0");
        let total_my_coin1 = TotalTradeFeeForTest::new("MYCOIN1", "0.00002", "0.00002");

        let expected = TradePreimageResult::MakerPreimage(MakerPreimage {
            base_coin_fee,
            rel_coin_fee,
            volume: None,
            volume_rat: None,
            volume_fraction: None,
            total_fees: vec![total_my_coin, total_my_coin1],
        });

        actual.result.sort_total_fees();
        assert_eq!(expected, actual.result);
    }

    #[test]
    fn test_taker_trade_preimage() {
        let priv_key = SecretKey::new(&mut rand6::thread_rng());

        let (_ctx, mycoin) = utxo_coin_from_privkey("MYCOIN", priv_key.as_ref());
        let my_address = mycoin.my_address().expect("!my_address");
        fill_address(&mycoin, &my_address, 10.into(), 30);

        let (_ctx, mycoin1) = utxo_coin_from_privkey("MYCOIN1", priv_key.as_ref());
        let my_address = mycoin1.my_address().expect("!my_address");
        fill_address(&mycoin1, &my_address, 20.into(), 30);

        let coins = json!([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":2000,"protocol":{"type":"UTXO"}},
        ]);
        let mm = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(priv_key.as_ref())),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);

        log!([block_on(enable_native(&mm, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm, "MYCOIN", &[]))]);

        // `max` field is not supported for `buy/sell` swap methods
        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "swap_method": "sell",
                "max": true,
                "price": 1,
            },
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);

        let actual: RpcErrorResponse<trade_preimage_error::InvalidParam> = json::from_str(&rc.1).unwrap();
        assert_eq!(actual.error_type, "InvalidParam", "Unexpected error_type: {}", rc.1);
        let expected = trade_preimage_error::InvalidParam {
            param: "max".to_owned(),
            reason: "'max' cannot be used with 'sell' or 'buy' method".to_owned(),
        };
        assert_eq!(actual.error_data, Some(expected));

        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "swap_method": "sell",
                "volume": "7.77",
                "price": "2",
            },
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);

        let mut actual: RpcSuccessResponse<TradePreimageResult> = json::from_str(&rc.1).unwrap();
        actual.result.sort_total_fees();

        let base_coin_fee = TradeFeeForTest::new("MYCOIN", "0.00001", false);
        let rel_coin_fee = TradeFeeForTest::new("MYCOIN1", "0.00002", true);
        let taker_fee = TradeFeeForTest::new("MYCOIN", "0.01", false);
        let fee_to_send_taker_fee = TradeFeeForTest::new("MYCOIN", "0.00001", false);

        let my_coin_total_fee = TotalTradeFeeForTest::new("MYCOIN", "0.01002", "0.01002");
        let my_coin1_total_fee = TotalTradeFeeForTest::new("MYCOIN1", "0.00002", "0");

        let expected = TradePreimageResult::TakerPreimage(TakerPreimage {
            base_coin_fee,
            rel_coin_fee,
            taker_fee,
            fee_to_send_taker_fee,
            total_fees: vec![my_coin_total_fee, my_coin1_total_fee],
        });
        assert_eq!(expected, actual.result);

        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "swap_method": "buy",
                "volume": "7.77",
                "price": "2",
            },
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);
        let mut actual: RpcSuccessResponse<TradePreimageResult> = json::from_str(&rc.1).unwrap();
        actual.result.sort_total_fees();

        let base_coin_fee = TradeFeeForTest::new("MYCOIN", "0.00001", true);
        let rel_coin_fee = TradeFeeForTest::new("MYCOIN1", "0.00002", false);
        let taker_fee = TradeFeeForTest::new("MYCOIN1", "0.02", false);
        let fee_to_send_taker_fee = TradeFeeForTest::new("MYCOIN1", "0.00002", false);

        let my_coin_total_fee = TotalTradeFeeForTest::new("MYCOIN", "0.00001", "0");
        let my_coin1_total_fee = TotalTradeFeeForTest::new("MYCOIN1", "0.02004", "0.02004");

        let expected = TradePreimageResult::TakerPreimage(TakerPreimage {
            base_coin_fee,
            rel_coin_fee,
            taker_fee,
            fee_to_send_taker_fee,
            total_fees: vec![my_coin_total_fee, my_coin1_total_fee],
        });
        assert_eq!(expected, actual.result);
    }

    #[test]
    fn test_trade_preimage_not_sufficient_balance() {
        #[track_caller]
        fn expect_not_sufficient_balance(
            res: &str,
            available: BigDecimal,
            required: BigDecimal,
            locked_by_swaps: Option<BigDecimal>,
        ) {
            let actual: RpcErrorResponse<trade_preimage_error::NotSufficientBalance> = json::from_str(res).unwrap();
            assert_eq!(actual.error_type, "NotSufficientBalance");
            let expected = trade_preimage_error::NotSufficientBalance {
                coin: "MYCOIN".to_owned(),
                available,
                required,
                locked_by_swaps,
            };
            assert_eq!(actual.error_data, Some(expected));
        }

        let priv_key = SecretKey::new(&mut rand6::thread_rng());
        let fill_balance_functor = |amount: BigDecimal| {
            let (_ctx, mycoin) = utxo_coin_from_privkey("MYCOIN", priv_key.as_ref());
            let my_address = mycoin.my_address().expect("!my_address");
            fill_address(&mycoin, &my_address, amount, 30);
        };

        let coins = json!([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":2000,"protocol":{"type":"UTXO"}},
        ]);
        let mm = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(priv_key.as_ref())),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);

        log!([block_on(enable_native(&mm, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm, "MYCOIN", &[]))]);

        fill_balance_functor(MmNumber::from("0.000015").to_decimal());
        // Try sell the max amount with the zero balance.
        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "swap_method": "setprice",
                "price": 1,
                "max": true,
            },
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
        let available = MmNumber::from("0.000015").to_decimal();
        // Required at least 0.00002 MYCOIN to pay the transaction_fee(0.00001) and to send a value not less than dust(0.00001).
        let required = MmNumber::from("0.00002").to_decimal();
        expect_not_sufficient_balance(&rc.1, available, required, None);

        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "swap_method": "setprice",
                "price": 1,
                "volume": 0.1,
            },
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
        // Required 0.00001 MYCOIN to pay the transaction fee and the specified 0.1 volume.
        let available = MmNumber::from("0.000015").to_decimal();
        let required = MmNumber::from("0.10001").to_decimal();
        expect_not_sufficient_balance(&rc.1, available, required, None);

        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "swap_method": "setprice",
                "price": 1,
                "max": true,
            },
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
        // balance(0.000015)
        let available = MmNumber::from("0.000015").to_decimal();
        // balance(0.000015) + transaction_fee(0.00001)
        let required = MmNumber::from("0.00002").to_decimal();
        expect_not_sufficient_balance(&rc.1, available, required, None);

        fill_balance_functor(MmNumber::from("7.770085").to_decimal());
        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "swap_method": "sell",
                "price": 1,
                "volume": 7.77,
            },
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
        let available = MmNumber::from("7.7701").to_decimal();
        // `required = volume + fee_to_send_taker_payment + dex_fee + fee_to_send_dex_fee`,
        // where `volume = 7.77`, `fee_to_send_taker_payment = fee_to_send_dex_fee = 0.00001`, `dex_fee = 0.01`.
        // Please note `dex_fee = 7.77 / 777` with dex_fee = 0.01
        // required = 7.77 + 0.01 (dex_fee) + (0.0001 * 2) = 7.78002
        let required = MmNumber::from("7.78002");
        expect_not_sufficient_balance(&rc.1, available, required.to_decimal(), Some(BigDecimal::from(0)));
    }

    /// This test ensures that `trade_preimage` will not succeed on input that will fail on `buy/sell/setprice`.
    /// https://github.com/KomodoPlatform/atomicDEX-API/issues/902
    #[test]
    fn test_trade_preimage_additional_validation() {
        let priv_key = SecretKey::new(&mut rand6::thread_rng());

        let (_ctx, mycoin1) = utxo_coin_from_privkey("MYCOIN1", priv_key.as_ref());
        let my_address = mycoin1.my_address().expect("!my_address");
        fill_address(&mycoin1, &my_address, 20.into(), 30);

        let (_ctx, mycoin) = utxo_coin_from_privkey("MYCOIN", priv_key.as_ref());
        let my_address = mycoin.my_address().expect("!my_address");
        fill_address(&mycoin, &my_address, 10.into(), 30);

        let coins = json!([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":2000,"protocol":{"type":"UTXO"}},
        ]);
        let mm = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(priv_key.as_ref())),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);

        log!([block_on(enable_native(&mm, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm, "MYCOIN", &[]))]);

        // Price is too low
        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "swap_method": "setprice",
                "price": 0,
                "volume": 0.1,
            },
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
        let actual: RpcErrorResponse<trade_preimage_error::PriceTooLow> = json::from_str(&rc.1).unwrap();
        assert_eq!(actual.error_type, "PriceTooLow");
        // currently the minimum price is 0.00000001
        let price_threshold = BigDecimal::from(1) / BigDecimal::from(100_000_000);
        let expected = trade_preimage_error::PriceTooLow {
            price: BigDecimal::from(0),
            threshold: price_threshold,
        };
        assert_eq!(actual.error_data, Some(expected));

        // volume 0.00001 is too low, min trading volume 0.0001
        let low_volume = BigDecimal::from(1) / BigDecimal::from(100_000);

        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "swap_method": "setprice",
                "price": 1,
                "volume": low_volume,
            },
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
        let actual: RpcErrorResponse<trade_preimage_error::VolumeTooLow> = json::from_str(&rc.1).unwrap();
        assert_eq!(actual.error_type, "VolumeTooLow");
        // Min MYCOIN trading volume is 0.0001.
        let volume_threshold = BigDecimal::from(1) / BigDecimal::from(10_000);
        let expected = trade_preimage_error::VolumeTooLow {
            coin: "MYCOIN".to_owned(),
            volume: low_volume.clone(),
            threshold: volume_threshold,
        };
        assert_eq!(actual.error_data, Some(expected));

        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "swap_method": "sell",
                "price": 1,
                "volume": low_volume,
            },
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
        let actual: RpcErrorResponse<trade_preimage_error::VolumeTooLow> = json::from_str(&rc.1).unwrap();
        assert_eq!(actual.error_type, "VolumeTooLow");
        // Min MYCOIN trading volume is 0.0001.
        let volume_threshold = BigDecimal::from(1) / BigDecimal::from(10_000);
        let expected = trade_preimage_error::VolumeTooLow {
            coin: "MYCOIN".to_owned(),
            volume: low_volume,
            threshold: volume_threshold,
        };
        assert_eq!(actual.error_data, Some(expected));

        // rel volume is too low
        // Min MYCOIN trading volume is 0.0001.
        let volume = BigDecimal::from(1) / BigDecimal::from(10_000);
        let low_price = BigDecimal::from(1) / BigDecimal::from(10);
        // Min MYCOIN1 trading volume is 0.0001, but the actual volume is 0.00001
        let low_rel_volume = &volume * &low_price;
        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "mmrpc": "2.0",
            "method": "trade_preimage",
            "params": {
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "swap_method": "sell",
                "price": low_price,
                "volume": volume,
            },
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
        let actual: RpcErrorResponse<trade_preimage_error::VolumeTooLow> = json::from_str(&rc.1).unwrap();
        assert_eq!(actual.error_type, "VolumeTooLow");
        // Min MYCOIN1 trading volume is 0.0001.
        let volume_threshold = BigDecimal::from(1) / BigDecimal::from(10_000);
        let expected = trade_preimage_error::VolumeTooLow {
            coin: "MYCOIN1".to_owned(),
            volume: low_rel_volume,
            threshold: volume_threshold,
        };
        assert_eq!(actual.error_data, Some(expected));
    }

    #[test]
    fn test_trade_preimage_legacy() {
        let priv_key = SecretKey::new(&mut rand6::thread_rng());
        let (_ctx, mycoin) = utxo_coin_from_privkey("MYCOIN", priv_key.as_ref());
        let my_address = mycoin.my_address().expect("!my_address");
        fill_address(&mycoin, &my_address, 10.into(), 30);
        let (_ctx, mycoin1) = utxo_coin_from_privkey("MYCOIN1", priv_key.as_ref());
        let my_address = mycoin1.my_address().expect("!my_address");
        fill_address(&mycoin1, &my_address, 20.into(), 30);

        let coins = json!([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":2000,"protocol":{"type":"UTXO"}},
        ]);
        let mm = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(priv_key.as_ref())),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_dump_log, _dump_dashboard) = mm_dump(&mm.log_path);

        log!([block_on(enable_native(&mm, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm, "MYCOIN", &[]))]);

        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "method": "trade_preimage",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "setprice",
            "max": true,
            "price": "1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);
        let _: TradePreimageResponse = json::from_str(&rc.1).unwrap();

        // vvv test a taker method vvv

        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "method": "trade_preimage",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "sell",
            "volume": "7.77",
            "price": "2",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!trade_preimage: {}", rc.1);
        let _: TradePreimageResponse = json::from_str(&rc.1).unwrap();

        // vvv test the error response vvv

        // `max` field is not supported for `buy/sell` swap methods
        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "method": "trade_preimage",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "swap_method": "sell",
            "max": true,
            "price": "1",
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "trade_preimage success, but should fail: {}", rc.1);
        assert!(rc
            .1
            .contains("Incorrect use of the 'max' parameter: 'max' cannot be used with 'sell' or 'buy' method"));
    }

    #[test]
    fn test_get_max_taker_vol() {
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 1.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mm_alice = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "max_taker_vol",
            "coin": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!max_taker_vol: {}", rc.1);
        let json: MaxTakerVolResponse = json::from_str(&rc.1).unwrap();
        // the result of equation `max_vol + max_vol / 777 + 0.00002 = 1`
        // derived from `max_vol = balance - locked - trade_fee - fee_to_send_taker_fee - dex_fee(max_vol)`
        // where balance = 1, locked = 0, trade_fee = fee_to_send_taker_fee = 0.00001, dex_fee = max_vol / 777
        let expected = MmNumber::from((38849223, 38900000)).to_fraction();
        assert_eq!(json.result, expected);
        assert_eq!(json.coin, "MYCOIN1");

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "sell",
            "base": "MYCOIN1",
            "rel": "MYCOIN",
            "price": 1,
            "volume": json.result,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!sell: {}", rc.1);

        block_on(mm_alice.stop()).unwrap();
    }

    // https://github.com/KomodoPlatform/atomicDEX-API/issues/733
    #[test]
    fn test_get_max_taker_vol_dex_fee_threshold() {
        let (_ctx, _, alice_priv_key) =
            generate_utxo_coin_with_random_privkey("MYCOIN1", "0.05328455".parse().unwrap());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":10000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":10000,"protocol":{"type":"UTXO"}},
        ]);
        let mm_alice = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "max_taker_vol",
            "coin": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!max_taker_vol: {}", rc.1);
        let json: Json = json::from_str(&rc.1).unwrap();
        // the result of equation x + 0.0001 (dex fee) + 0.0002 (miner fee * 2) = 0.05328455
        assert_eq!(json["result"]["numer"], Json::from("1059691"));
        assert_eq!(json["result"]["denom"], Json::from("20000000"));

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "sell",
            "base": "MYCOIN1",
            "rel": "MYCOIN",
            "price": 1,
            "volume": {
                "numer": json["result"]["numer"],
                "denom": json["result"]["denom"],
            }
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!sell: {}", rc.1);

        block_on(mm_alice.stop()).unwrap();
    }

    /// Test if the `max_taker_vol` cannot return a volume less than the coin's dust.
    /// The minimum required balance for trading can be obtained by solving the equation:
    /// `volume + taker_fee + trade_fee + fee_to_send_taker_fee = x`.
    /// Let `dust = 0.000728` like for Qtum, `trade_fee = 0.0001`, `fee_to_send_taker_fee = 0.0001` and `taker_fee` is the `0.000728` threshold,
    /// therefore to find a minimum required balance, we should pass the `dust` as the `volume` into the equation above:
    /// `2 * 0.000728 + 0.0002 = x`, so `x = 0.001656`
    #[test]
    fn test_get_max_taker_vol_dust_threshold() {
        // first, try to test with the balance slightly less than required
        let (_ctx, coin, priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", "0.001656".parse().unwrap());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":10000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":10000,"protocol":{"type":"UTXO"},"dust":72800},
        ]);
        let mm = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm.log_path);

        log!([block_on(enable_native(&mm, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm, "MYCOIN", &[]))]);

        let rc = block_on(mm.rpc(json!({
            "userpass": mm.userpass,
            "method": "max_taker_vol",
            "coin": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!max_taker_vol {}", rc.1);
        let json: Json = json::from_str(&rc.1).unwrap();
        let result: MmNumber = json::from_value(json["result"].clone()).unwrap();
        assert!(result.is_zero());

        fill_address(&coin, &coin.my_address().unwrap(), "0.00001".parse().unwrap(), 30);

        let rc = block_on(mm.rpc(json! ({
            "userpass": mm.userpass,
            "method": "max_taker_vol",
            "coin": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!max_taker_vol: {}", rc.1);
        let json: Json = json::from_str(&rc.1).unwrap();
        // the result of equation x + 0.000728 (dex fee) + 0.0002 (miner fee * 2) = 0.001666
        assert_eq!(json["result"]["numer"], Json::from("369"));
        assert_eq!(json["result"]["denom"], Json::from("500000"));

        block_on(mm.stop()).unwrap();
    }

    #[test]
    fn test_get_max_taker_vol_with_kmd() {
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 1.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":10000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":10000,"protocol":{"type":"UTXO"}},
            {"coin":"KMD","txversion":4,"overwintered":1,"txfee":10000,"protocol":{"type":"UTXO"}},
        ]);
        let mm_alice = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_electrum(&mm_alice, "KMD", false, &[
            "electrum1.cipig.net:10001",
            "electrum2.cipig.net:10001",
            "electrum3.cipig.net:10001"
        ]))]);
        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "max_taker_vol",
            "coin": "MYCOIN1",
            "trade_with": "KMD",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!max_taker_vol: {}", rc.1);
        let json: Json = json::from_str(&rc.1).unwrap();
        // the result of equation x + x * 9 / 7770 + 0.0002 = 1
        assert_eq!(json["result"]["numer"], Json::from("1294741"));
        assert_eq!(json["result"]["denom"], Json::from("1296500"));

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "sell",
            "base": "MYCOIN1",
            "rel": "KMD",
            "price": 1,
            "volume": {
                "numer": json["result"]["numer"],
                "denom": json["result"]["denom"],
            }
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!sell: {}", rc.1);

        block_on(mm_alice.stop()).unwrap();
    }

    #[test]
    fn test_set_price_max() {
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mm_alice = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(alice_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);
        let rc = block_on(mm_alice.rpc(json! ({
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
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = block_on(mm_alice.rpc(json! ({
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
        })))
        .unwrap();
        assert!(!rc.0.is_success(), "setprice success, but should fail: {}", rc.1);
        block_on(mm_alice.stop()).unwrap();
    }

    #[test]
    fn swaps_should_stop_on_stop_rpc() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mut mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
        let mut uuids = Vec::with_capacity(3);

        for _ in 0..3 {
            let rc = block_on(mm_alice.rpc(json! ({
                "userpass": mm_alice.userpass,
                "method": "buy",
                "base": "MYCOIN",
                "rel": "MYCOIN1",
                "price": 1,
                "volume": "1",
            })))
            .unwrap();
            assert!(rc.0.is_success(), "!buy: {}", rc.1);
            let buy: Json = json::from_str(&rc.1).unwrap();
            uuids.push(buy["result"]["uuid"].as_str().unwrap().to_owned());
        }
        for uuid in uuids.iter() {
            block_on(mm_bob.wait_for_log(22., |log| {
                log.contains(&format!(
                    "Entering the maker_swap_loop MYCOIN/MYCOIN1 with uuid: {}",
                    uuid
                ))
            }))
            .unwrap();
            block_on(mm_alice.wait_for_log(22., |log| {
                log.contains(&format!(
                    "Entering the taker_swap_loop MYCOIN/MYCOIN1 with uuid: {}",
                    uuid
                ))
            }))
            .unwrap();
        }
        thread::sleep(Duration::from_secs(3));
        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
        for uuid in uuids {
            block_on(mm_bob.wait_for_log_after_stop(22., |log| log.contains(&format!("swap {} stopped", uuid))))
                .unwrap();
            block_on(mm_alice.wait_for_log_after_stop(22., |log| log.contains(&format!("swap {} stopped", uuid))))
                .unwrap();
        }
    }

    #[test]
    fn test_maker_order_should_kick_start_and_appear_in_orderbook_on_restart() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
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
        let mm_bob = MarketMakerIt::start(bob_conf.clone(), "pass".to_string(), None).unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        // mm_bob using same DB dir that should kick start the order
        bob_conf["dbdir"] = mm_bob.folder.join("DB").to_str().unwrap().into();
        bob_conf["log"] = mm_bob.folder.join("mm2_dup.log").to_str().unwrap().into();
        block_on(mm_bob.stop()).unwrap();

        let mm_bob_dup = MarketMakerIt::start(bob_conf, "pass".to_string(), None).unwrap();
        let (_bob_dup_dump_log, _bob_dup_dump_dashboard) = mm_dump(&mm_bob_dup.log_path);
        log!([block_on(enable_native(&mm_bob_dup, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob_dup, "MYCOIN1", &[]))]);

        thread::sleep(Duration::from_secs(2));

        log!("Get RICK/MORTY orderbook on Bob side");
        let rc = block_on(mm_bob_dup.rpc(json! ({
            "userpass": mm_bob_dup.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("Bob orderbook "[bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 1, "Bob MYCOIN/MYCOIN1 orderbook must have exactly 1 asks");
    }

    #[test]
    fn test_maker_order_should_not_kick_start_and_appear_in_orderbook_if_balance_is_withdrawn() {
        let (_ctx, coin, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
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
        let mm_bob = MarketMakerIt::start(bob_conf.clone(), "pass".to_string(), None).unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
        let res: SetPriceResponse = json::from_str(&rc.1).unwrap();
        let uuid = res.result.uuid;

        // mm_bob using same DB dir that should kick start the order
        bob_conf["dbdir"] = mm_bob.folder.join("DB").to_str().unwrap().into();
        bob_conf["log"] = mm_bob.folder.join("mm2_dup.log").to_str().unwrap().into();
        block_on(mm_bob.stop()).unwrap();

        let withdraw = coin
            .withdraw(WithdrawRequest::new_max(
                "MYCOIN".to_string(),
                "RRYmiZSDo3UdHHqj1rLKf8cbJroyv9NxXw".to_string(),
            ))
            .wait()
            .unwrap();
        coin.send_raw_tx(&hex::encode(&withdraw.tx_hex.0)).wait().unwrap();
        coin.wait_for_confirmations(&withdraw.tx_hex.0, 1, false, (now_ms() / 1000) + 10, 1)
            .wait()
            .unwrap();

        let mm_bob_dup = MarketMakerIt::start(bob_conf, "pass".to_string(), None).unwrap();
        let (_bob_dup_dump_log, _bob_dup_dump_dashboard) = mm_dump(&mm_bob_dup.log_path);
        log!([block_on(enable_native(&mm_bob_dup, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob_dup, "MYCOIN1", &[]))]);

        thread::sleep(Duration::from_secs(2));

        log!("Get RICK/MORTY orderbook on Bob side");
        let rc = block_on(mm_bob_dup.rpc(json! ({
            "userpass": mm_bob_dup.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("Bob orderbook "[bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert!(asks.is_empty(), "Bob MYCOIN/MYCOIN1 orderbook must not have asks");

        let rc = block_on(mm_bob_dup.rpc(json! ({
            "userpass": mm_bob_dup.userpass,
            "method": "my_orders",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!my_orders: {}", rc.1);

        let res: MyOrdersRpcResult = json::from_str(&rc.1).unwrap();
        assert!(res.result.maker_orders.is_empty(), "Bob maker orders must be empty");

        let order_path = mm_bob.folder.join(format!(
            "DB/{}/ORDERS/MY/MAKER/{}.json",
            hex::encode(rmd160_from_priv(bob_priv_key).take()),
            uuid
        ));

        println!("Order path {}", order_path.display());
        assert!(!order_path.exists());
    }

    #[test]
    fn test_maker_order_kick_start_should_trigger_subscription_and_match() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 1000.into());
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
        let relay = MarketMakerIt::start(relay_conf, "pass".to_string(), None).unwrap();
        let (_relay_dump_log, _relay_dump_dashboard) = mm_dump(&relay.log_path);

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
        let mm_bob = MarketMakerIt::start(bob_conf.clone(), "pass".to_string(), None).unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mut mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);

        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        // mm_bob using same DB dir that should kick start the order
        bob_conf["dbdir"] = mm_bob.folder.join("DB").to_str().unwrap().into();
        bob_conf["log"] = mm_bob.folder.join("mm2_dup.log").to_str().unwrap().into();
        block_on(mm_bob.stop()).unwrap();

        let mut mm_bob_dup = MarketMakerIt::start(bob_conf, "pass".to_string(), None).unwrap();
        let (_bob_dup_dump_log, _bob_dup_dump_dashboard) = mm_dump(&mm_bob_dup.log_path);
        log!([block_on(enable_native(&mm_bob_dup, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob_dup, "MYCOIN1", &[]))]);

        log!("Give restarted Bob 2 seconds to kickstart the order");
        thread::sleep(Duration::from_secs(2));

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": 1,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        block_on(mm_bob_dup.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap();
        block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap();
    }

    #[test]
    fn test_orders_should_match_on_both_nodes_with_same_priv() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mut mm_alice_1 = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_1_dump_log, _alice_1_dump_dashboard) = mm_dump(&mm_alice_1.log_path);

        let mut mm_alice_2 = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_2_dump_log, _alice_2_dump_dashboard) = mm_dump(&mm_alice_2.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice_1, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice_1, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice_2, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice_2, "MYCOIN1", &[]))]);

        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = block_on(mm_alice_1.rpc(json! ({
            "userpass": mm_alice_1.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        block_on(mm_alice_1.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap();

        let rc = block_on(mm_alice_2.rpc(json! ({
            "userpass": mm_alice_2.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        block_on(mm_alice_2.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap();

        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice_1.stop()).unwrap();
        block_on(mm_alice_2.stop()).unwrap();
    }

    #[test]
    fn test_maker_and_taker_order_created_with_same_priv_should_not_match() {
        let (_ctx, coin, priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, coin1, _) = generate_utxo_coin_with_random_privkey("MYCOIN1", 1000.into());
        fill_address(&coin1, &coin.my_address().unwrap(), 1000.into(), 30);
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mut mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_1_dump_log, _alice_1_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);

        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": "1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        block_on(mm_bob.wait_for_log(5., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap_err();
        block_on(mm_alice.wait_for_log(5., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap_err();

        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
    }

    #[test]
    fn test_taker_order_converted_to_maker_should_cancel_properly_when_matched() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 2000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mut mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "sell",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": 1,
            "timeout": 2,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!sell: {}", rc.1);

        log!("Give Bob 4 seconds to convert order to maker");
        thread::sleep(Duration::from_secs(4));

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "volume": 1,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
        block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap();

        log!("Give Bob 2 seconds to cancel the order");
        thread::sleep(Duration::from_secs(2));
        log!("Get my_orders on Bob side");
        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "my_orders",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!my_orders: {}", rc.1);
        let my_orders_json: Json = json::from_str(&rc.1).unwrap();
        let maker_orders: HashMap<String, Json> =
            json::from_value(my_orders_json["result"]["maker_orders"].clone()).unwrap();
        assert!(maker_orders.is_empty());

        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let bob_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("Bob orderbook "[bob_orderbook]);
        let asks = bob_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 0, "Bob MYCOIN/MYCOIN1 orderbook must be empty");

        log!("Get MYCOIN/MYCOIN1 orderbook on Alice side");
        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!orderbook: {}", rc.1);

        let alice_orderbook: Json = json::from_str(&rc.1).unwrap();
        log!("Alice orderbook "[alice_orderbook]);
        let asks = alice_orderbook["asks"].as_array().unwrap();
        assert_eq!(asks.len(), 0, "Alice MYCOIN/MYCOIN1 orderbook must be empty");

        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
    }

    #[test]
    fn test_utxo_merge() {
        let timeout = 30; // timeout if test takes more than 30 seconds to run
        let (_ctx, coin, privkey) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        // fill several times to have more UTXOs on address
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);

        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

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

        block_on(mm_bob.wait_for_log(4., |log| log.contains("Starting UTXO merge loop for coin MYCOIN"))).unwrap();

        block_on(mm_bob.wait_for_log(4., |log| log.contains("Trying to merge 5 UTXOs of coin MYCOIN"))).unwrap();

        block_on(mm_bob.wait_for_log(4., |log| log.contains("UTXO merge successful for coin MYCOIN, tx_hash")))
            .unwrap();

        thread::sleep(Duration::from_secs(2));
        let (unspents, _) =
            block_on(coin.list_unspent_ordered(&coin.as_ref().derivation_method.unwrap_iguana())).unwrap();
        assert_eq!(unspents.len(), 1);
    }

    #[test]
    fn test_utxo_merge_max_merge_at_once() {
        let timeout = 30; // timeout if test takes more than 30 seconds to run
        let (_ctx, coin, privkey) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        // fill several times to have more UTXOs on address
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);
        fill_address(&coin, &coin.my_address().unwrap(), 2.into(), timeout);

        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mut mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

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

        block_on(mm_bob.wait_for_log(4., |log| log.contains("Starting UTXO merge loop for coin MYCOIN"))).unwrap();

        block_on(mm_bob.wait_for_log(4., |log| log.contains("Trying to merge 4 UTXOs of coin MYCOIN"))).unwrap();

        block_on(mm_bob.wait_for_log(4., |log| log.contains("UTXO merge successful for coin MYCOIN, tx_hash")))
            .unwrap();

        thread::sleep(Duration::from_secs(2));
        let (unspents, _) =
            block_on(coin.list_unspent_ordered(&coin.as_ref().derivation_method.unwrap_iguana())).unwrap();
        // 4 utxos are merged of 5 so the resulting unspents len must be 2
        assert_eq!(unspents.len(), 2);
    }

    #[test]
    fn test_withdraw_not_sufficient_balance() {
        let privkey = SecretKey::new(&mut rand6::thread_rng());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mm = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(privkey.as_ref())),
                "coins": coins,
                "rpc_password": "pass",
                "i_am_seed": true,
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm.log_path);
        log!([block_on(enable_native(&mm, "MYCOIN", &[]))]);

        // balance = 0, but amount = 1
        let amount = BigDecimal::from(1);
        let withdraw = block_on(mm.rpc(json! ({
            "mmrpc": "2.0",
            "userpass": mm.userpass,
            "method": "withdraw",
            "params": {
                "coin": "MYCOIN",
                "to": "RJTYiYeJ8eVvJ53n2YbrVmxWNNMVZjDGLh",
                "amount": amount,
            },
            "id": 0,
        })))
        .unwrap();

        assert!(withdraw.0.is_client_error(), "RICK withdraw: {}", withdraw.1);
        log!("error: "[withdraw.1]);
        let error: RpcErrorResponse<withdraw_error::NotSufficientBalance> =
            json::from_str(&withdraw.1).expect("Expected 'RpcErrorResponse<NotSufficientBalance>'");
        let expected_error = withdraw_error::NotSufficientBalance {
            coin: "MYCOIN".to_owned(),
            available: 0.into(),
            required: amount,
        };
        assert_eq!(error.error_type, "NotSufficientBalance");
        assert_eq!(error.error_data, Some(expected_error));

        // fill the MYCOIN balance
        let balance = BigDecimal::from(1) / BigDecimal::from(2);
        let (_ctx, coin) = utxo_coin_from_privkey("MYCOIN", privkey.as_ref());
        fill_address(&coin, &coin.my_address().unwrap(), balance.clone(), 30);

        // txfee = 0.00001, amount = 0.5 => required = 0.50001
        // but balance = 0.5
        let txfee = BigDecimal::from(1) / BigDecimal::from(100000);
        let withdraw = block_on(mm.rpc(json! ({
            "mmrpc": "2.0",
            "userpass": mm.userpass,
            "method": "withdraw",
            "params": {
                "coin": "MYCOIN",
                "to": "RJTYiYeJ8eVvJ53n2YbrVmxWNNMVZjDGLh",
                "amount": balance,
            },
            "id": 0,
        })))
        .unwrap();

        assert!(withdraw.0.is_client_error(), "RICK withdraw: {}", withdraw.1);
        log!("error: "[withdraw.1]);
        let error: RpcErrorResponse<withdraw_error::NotSufficientBalance> =
            json::from_str(&withdraw.1).expect("Expected 'RpcErrorResponse<NotSufficientBalance>'");
        let expected_error = withdraw_error::NotSufficientBalance {
            coin: "MYCOIN".to_owned(),
            available: balance.clone(),
            required: balance + txfee,
        };
        assert_eq!(error.error_type, "NotSufficientBalance");
        assert_eq!(error.error_data, Some(expected_error));
    }

    // https://github.com/KomodoPlatform/atomicDEX-API/issues/1053
    #[test]
    fn test_taker_should_match_with_best_price_buy() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 2000.into());
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 4000.into());
        let (_ctx, _, eve_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 2000.into());

        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mut mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        let mut mm_eve = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(eve_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "seednodes": vec![format!("{}", mm_bob.ip)],
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_eve_dump_log, _eve_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_eve, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_eve, "MYCOIN1", &[]))]);

        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 2,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = block_on(mm_eve.rpc(json! ({
            "userpass": mm_eve.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        // subscribe alice to the orderbook topic to not miss eve's message
        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!alice orderbook: {}", rc.1);
        log!("alice orderbook "(rc.1));

        thread::sleep(Duration::from_secs(1));

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 3,
            "volume": "1000",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);
        let alice_buy: BuyOrSellRpcResult = json::from_str(&rc.1).unwrap();

        block_on(mm_eve.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
        block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap();

        thread::sleep(Duration::from_secs(2));

        block_on(check_my_swap_status_amounts(
            &mm_alice,
            alice_buy.result.uuid,
            1000.into(),
            1000.into(),
        ));
        block_on(check_my_swap_status_amounts(
            &mm_eve,
            alice_buy.result.uuid,
            1000.into(),
            1000.into(),
        ));

        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
        block_on(mm_eve.stop()).unwrap();
    }

    // https://github.com/KomodoPlatform/atomicDEX-API/issues/1053
    #[test]
    fn test_taker_should_match_with_best_price_sell() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 2000.into());
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN1", 4000.into());
        let (_ctx, _, eve_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 2000.into());

        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"MYCOIN1","asset":"MYCOIN1","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        ]);
        let mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mut mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        let mut mm_eve = MarketMakerIt::start(
            json! ({
                "gui": "nogui",
                "netid": 9000,
                "dht": "on",  // Enable DHT without delay.
                "passphrase": format!("0x{}", hex::encode(eve_priv_key)),
                "coins": coins,
                "rpc_password": "pass",
                "seednodes": vec![format!("{}", mm_bob.ip)],
            }),
            "pass".to_string(),
            None,
        )
        .unwrap();
        let (_eve_dump_log, _eve_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_bob, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN1", &[]))]);
        log!([block_on(enable_native(&mm_eve, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_eve, "MYCOIN1", &[]))]);

        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 2,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = block_on(mm_eve.rpc(json! ({
            "userpass": mm_eve.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
            "price": 1,
            "max": true,
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        // subscribe alice to the orderbook topic to not miss eve's message
        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "orderbook",
            "base": "MYCOIN",
            "rel": "MYCOIN1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!alice orderbook: {}", rc.1);
        log!("alice orderbook "(rc.1));

        thread::sleep(Duration::from_secs(1));

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "sell",
            "base": "MYCOIN1",
            "rel": "MYCOIN",
            "price": "0.1",
            "volume": "1000",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);
        let alice_buy: BuyOrSellRpcResult = json::from_str(&rc.1).unwrap();

        block_on(mm_eve.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/MYCOIN1"))).unwrap();
        block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/MYCOIN1")))
            .unwrap();

        thread::sleep(Duration::from_secs(2));

        block_on(check_my_swap_status_amounts(
            &mm_alice,
            alice_buy.result.uuid,
            1000.into(),
            1000.into(),
        ));
        block_on(check_my_swap_status_amounts(
            &mm_eve,
            alice_buy.result.uuid,
            1000.into(),
            1000.into(),
        ));

        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
        block_on(mm_eve.stop()).unwrap();
    }

    #[test]
    // https://github.com/KomodoPlatform/atomicDEX-API/issues/1074
    fn test_match_utxo_with_eth_taker_sell() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        ]);
        let mut mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mut mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        let eth_bob = block_on(enable_native(&mm_bob, "ETH", &["http://195.201.0.6:8565"]));
        let eth_bob: EnableElectrumResponse = json::from_value(eth_bob).unwrap();
        // pass without 0x
        fill_eth(&eth_bob.address[2..]);

        let eth_alice = block_on(enable_native(&mm_alice, "ETH", &["http://195.201.0.6:8565"]));
        let eth_alice: EnableElectrumResponse = json::from_value(eth_alice).unwrap();
        // pass without 0x
        fill_eth(&eth_alice.address[2..]);

        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "ETH",
            "price": 1,
            "volume": "0.1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);
        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "sell",
            "base": "ETH",
            "rel": "MYCOIN",
            "price": 1,
            "volume": "0.1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!sell: {}", rc.1);

        block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/ETH"))).unwrap();
        block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/ETH"))).unwrap();

        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
    }

    #[test]
    // https://github.com/KomodoPlatform/atomicDEX-API/issues/1074
    fn test_match_utxo_with_eth_taker_buy() {
        let (_ctx, _, bob_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let (_ctx, _, alice_priv_key) = generate_utxo_coin_with_random_privkey("MYCOIN", 1000.into());
        let coins = json! ([
            {"coin":"MYCOIN","asset":"MYCOIN","txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
            {"coin":"ETH","name":"ethereum","protocol":{"type":"ETH"}},
        ]);
        let mut mm_bob = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_bob_dump_log, _bob_dump_dashboard) = mm_dump(&mm_bob.log_path);

        let mut mm_alice = MarketMakerIt::start(
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
        )
        .unwrap();
        let (_alice_dump_log, _alice_dump_dashboard) = mm_dump(&mm_alice.log_path);

        log!([block_on(enable_native(&mm_bob, "MYCOIN", &[]))]);
        log!([block_on(enable_native(&mm_alice, "MYCOIN", &[]))]);
        let eth_bob = block_on(enable_native(&mm_bob, "ETH", &["http://195.201.0.6:8565"]));
        let eth_bob: EnableElectrumResponse = json::from_value(eth_bob).unwrap();
        // pass without 0x
        fill_eth(&eth_bob.address[2..]);

        let eth_alice = block_on(enable_native(&mm_alice, "ETH", &["http://195.201.0.6:8565"]));
        let eth_alice: EnableElectrumResponse = json::from_value(eth_alice).unwrap();
        // pass without 0x
        fill_eth(&eth_alice.address[2..]);

        let rc = block_on(mm_bob.rpc(json! ({
            "userpass": mm_bob.userpass,
            "method": "setprice",
            "base": "MYCOIN",
            "rel": "ETH",
            "price": 1,
            "volume": "0.1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!setprice: {}", rc.1);

        let rc = block_on(mm_alice.rpc(json! ({
            "userpass": mm_alice.userpass,
            "method": "buy",
            "base": "MYCOIN",
            "rel": "ETH",
            "price": 1,
            "volume": "0.1",
        })))
        .unwrap();
        assert!(rc.0.is_success(), "!buy: {}", rc.1);

        block_on(mm_bob.wait_for_log(22., |log| log.contains("Entering the maker_swap_loop MYCOIN/ETH"))).unwrap();
        block_on(mm_alice.wait_for_log(22., |log| log.contains("Entering the taker_swap_loop MYCOIN/ETH"))).unwrap();

        block_on(mm_bob.stop()).unwrap();
        block_on(mm_alice.stop()).unwrap();
    }
}
