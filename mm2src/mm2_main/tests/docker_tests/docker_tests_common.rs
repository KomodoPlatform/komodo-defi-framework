pub use common::{block_on, now_ms};
pub use mm2_number::MmNumber;
pub use mm2_test_helpers::for_tests::{check_my_swap_status, check_recent_swaps, check_stats_swap_status,
                                      enable_native_bch, mm_dump, MarketMakerIt, MAKER_ERROR_EVENTS,
                                      MAKER_SUCCESS_EVENTS, TAKER_ERROR_EVENTS, TAKER_SUCCESS_EVENTS};
pub use secp256k1::{PublicKey, SecretKey};
pub use std::env;
pub use std::thread;

use bitcrypto::{dhash160, ChecksumType};
use chain::TransactionOutput;
use coins::eth::{eth_coin_from_conf_and_request, EthCoin};
use coins::qrc20::rpc_clients::for_tests::Qrc20NativeWalletOps;
use coins::qrc20::{qrc20_coin_with_priv_key, Qrc20ActivationParams, Qrc20Coin};
use coins::utxo::bch::{bch_coin_with_priv_key, BchActivationRequest, BchCoin};
use coins::utxo::qtum::{qtum_coin_with_priv_key, QtumBasedCoin, QtumCoin};
use coins::utxo::rpc_clients::{NativeClient, UtxoRpcClientEnum, UtxoRpcClientOps};
use coins::utxo::slp::{slp_genesis_output, SlpOutput, SlpToken};
use coins::utxo::utxo_common::send_outputs_from_my_address;
use coins::utxo::utxo_standard::{utxo_standard_coin_with_priv_key, UtxoStandardCoin};
use coins::utxo::{coin_daemon_data_dir, sat_from_big_decimal, zcash_params_path, UtxoActivationParams,
                  UtxoAddressFormat, UtxoCoinFields, UtxoCommonOps};
use coins::{CoinProtocol, MarketCoinOps, PrivKeyBuildPolicy, Transaction};
use crypto::privkey::key_pair_from_seed;
use crypto::Secp256k1Secret;
use ethereum_types::H160 as H160Eth;
use futures01::Future;
use http::StatusCode;
use keys::{Address, AddressHashEnum, KeyPair, NetworkPrefix as CashAddrPrefix};
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};
use mm2_number::BigDecimal;
use mm2_test_helpers::for_tests::enable_native;
use mm2_test_helpers::structs::{MyBalanceResponse, TransactionDetails};
use primitives::hash::{H160, H256};
use script::Builder;
use secp256k1::Secp256k1;
use serde_json::{self as json, Value as Json};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;
use testcontainers::clients::Cli;
use testcontainers::images::generic::{GenericImage, WaitFor};
use testcontainers::{Container, Docker, Image};

lazy_static! {
    static ref MY_COIN_LOCK: Mutex<()> = Mutex::new(());
    static ref MY_COIN1_LOCK: Mutex<()> = Mutex::new(());
    static ref QTUM_LOCK: Mutex<()> = Mutex::new(());
    static ref FOR_SLP_LOCK: Mutex<()> = Mutex::new(());
    pub static ref SLP_TOKEN_ID: Mutex<H256> = Mutex::new(H256::default());
    // Private keys supplied with 1000 SLP tokens on tests initialization.
    // Due to the SLP protocol limitations only 19 outputs (18 + change) can be sent in one transaction, which is sufficient for now though.
    // Supply more privkeys when 18 will be not enough.
    pub static ref SLP_TOKEN_OWNERS: Mutex<Vec<[u8; 32]>> = Mutex::new(Vec::with_capacity(18));
    static ref ETH_DISTRIBUTOR: EthCoin = eth_distributor();
    static ref MM_CTX: MmArc = MmCtxBuilder::new().into_mm_arc();
}

pub static mut QICK_TOKEN_ADDRESS: Option<H160Eth> = None;
pub static mut QORTY_TOKEN_ADDRESS: Option<H160Eth> = None;
pub static mut QRC20_SWAP_CONTRACT_ADDRESS: Option<H160Eth> = None;
pub static mut QTUM_CONF_PATH: Option<PathBuf> = None;

pub const UTXO_ASSET_DOCKER_IMAGE: &str = "docker.io/artempikulin/testblockchain:multiarch";

pub const QTUM_ADDRESS_LABEL: &str = "MM2_ADDRESS_LABEL";

pub trait CoinDockerOps {
    fn rpc_client(&self) -> &UtxoRpcClientEnum;

    fn native_client(&self) -> &NativeClient {
        match self.rpc_client() {
            UtxoRpcClientEnum::Native(native) => native,
            _ => panic!("UtxoRpcClientEnum::Native is expected"),
        }
    }

    fn wait_ready(&self, expected_tx_version: i32) {
        let timeout = now_ms() + 120000;
        loop {
            match self.rpc_client().get_block_count().wait() {
                Ok(n) => {
                    if n > 1 {
                        if let UtxoRpcClientEnum::Native(client) = self.rpc_client() {
                            let hash = client.get_block_hash(n).wait().unwrap();
                            let block = client.get_block(hash).wait().unwrap();
                            let coinbase = client.get_verbose_transaction(&block.tx[0]).wait().unwrap();
                            println!("Coinbase tx {:?} in block {}", coinbase, n);
                            if coinbase.version == expected_tx_version {
                                break;
                            }
                        }
                    }
                },
                Err(e) => log!("{:?}", e),
            }
            assert!(now_ms() < timeout, "Test timed out");
            thread::sleep(Duration::from_secs(1));
        }
    }
}

pub struct UtxoAssetDockerOps {
    #[allow(dead_code)]
    ctx: MmArc,
    coin: UtxoStandardCoin,
}

impl CoinDockerOps for UtxoAssetDockerOps {
    fn rpc_client(&self) -> &UtxoRpcClientEnum { &self.coin.as_ref().rpc_client }
}

impl UtxoAssetDockerOps {
    pub fn from_ticker(ticker: &str) -> UtxoAssetDockerOps {
        let conf = json!({"asset": ticker, "txfee": 1000, "network": "regtest"});
        let req = json!({"method":"enable"});
        let priv_key = Secp256k1Secret::from("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f");
        let ctx = MmCtxBuilder::new().into_mm_arc();
        let params = UtxoActivationParams::from_legacy_req(&req).unwrap();

        let coin = block_on(utxo_standard_coin_with_priv_key(&ctx, ticker, &conf, &params, priv_key)).unwrap();
        UtxoAssetDockerOps { ctx, coin }
    }
}

pub struct BchDockerOps {
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
        key_pair_from_seed("spice describe gravity federal blast come thank unfair canal monkey style afraid").unwrap();
    let priv_key_policy = PrivKeyBuildPolicy::IguanaPrivKey(keypair.private().secret);
    block_on(eth_coin_from_conf_and_request(
        &MM_CTX,
        "ETH",
        &conf,
        &req,
        CoinProtocol::ETH,
        priv_key_policy,
    ))
    .unwrap()
}

// pass address without 0x prefix to this fn
pub fn fill_eth(to_addr: &str) {
    ETH_DISTRIBUTOR
        .send_to_address(to_addr.parse().unwrap(), 1_000_000_000_000_000_000u64.into())
        .wait()
        .unwrap();
}

impl BchDockerOps {
    pub fn from_ticker(ticker: &str) -> BchDockerOps {
        let conf = json!({"asset": ticker,"txfee":1000,"network": "regtest","txversion":4,"overwintered":1});
        let req = json!({"method":"enable", "bchd_urls": [], "allow_slp_unsafe_conf": true});
        let priv_key = Secp256k1Secret::from("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f");
        let ctx = MmCtxBuilder::new().into_mm_arc();
        let params = BchActivationRequest::from_legacy_req(&req).unwrap();

        let coin = block_on(bch_coin_with_priv_key(
            &ctx,
            ticker,
            &conf,
            params,
            CashAddrPrefix::SlpTest,
            priv_key,
        ))
        .unwrap();
        BchDockerOps { ctx, coin }
    }

    pub fn initialize_slp(&self) {
        fill_address(&self.coin, &self.coin.my_address().unwrap(), 100000.into(), 30);
        let mut slp_privkeys = vec![];

        let slp_genesis_op_ret = slp_genesis_output("ADEXSLP", "ADEXSLP", None, None, 8, None, 1000000_00000000);
        let slp_genesis = TransactionOutput {
            value: self.coin.as_ref().dust_amount,
            script_pubkey: Builder::build_p2pkh(&self.coin.my_public_key().unwrap().address_hash().into()).to_bytes(),
        };

        let mut bch_outputs = vec![slp_genesis_op_ret, slp_genesis];
        let mut slp_outputs = vec![];

        for _ in 0..18 {
            let key_pair = KeyPair::random_compressed();
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
            slp_privkeys.push(*key_pair.private_ref());
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
        )
        .unwrap();

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

pub struct UtxoDockerNode<'a> {
    #[allow(dead_code)]
    pub container: Container<'a, Cli, GenericImage>,
    #[allow(dead_code)]
    pub ticker: String,
    #[allow(dead_code)]
    pub port: u16,
}

pub fn random_secp256k1_secret() -> Secp256k1Secret {
    let priv_key = SecretKey::new(&mut rand6::thread_rng());
    Secp256k1Secret::from(*priv_key.as_ref())
}

pub fn utxo_asset_docker_node<'a>(docker: &'a Cli, ticker: &'static str, port: u16) -> UtxoDockerNode<'a> {
    let args = vec![
        "-v".into(),
        format!("{}:/root/.zcash-params", zcash_params_path().display()),
        "-p".into(),
        format!("{}:{}", port, port).into(),
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
    std::fs::create_dir_all(&conf_path).unwrap();
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

pub fn rmd160_from_priv(privkey: Secp256k1Secret) -> H160 {
    let secret = SecretKey::from_slice(privkey.as_slice()).unwrap();
    let public = PublicKey::from_secret_key(&Secp256k1::new(), &secret);
    dhash160(&public.serialize())
}

pub fn get_prefilled_slp_privkey() -> [u8; 32] { SLP_TOKEN_OWNERS.lock().unwrap().remove(0) }

pub fn get_slp_token_id() -> String { hex::encode(SLP_TOKEN_ID.lock().unwrap().as_slice()) }

pub fn import_address<T>(coin: &T)
where
    T: MarketCoinOps + AsRef<UtxoCoinFields>,
{
    match coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Native(ref native) => {
            let my_address = coin.my_address().unwrap();
            native.import_address(&my_address, &my_address, false).wait().unwrap()
        },
        UtxoRpcClientEnum::Electrum(_) => panic!("Expected NativeClient"),
    }
}

/// Build `Qrc20Coin` from ticker and privkey without filling the balance.
pub fn qrc20_coin_from_privkey(ticker: &str, priv_key: Secp256k1Secret) -> (MmArc, Qrc20Coin) {
    let (contract_address, swap_contract_address) = unsafe {
        let contract_address = match ticker {
            "QICK" => QICK_TOKEN_ADDRESS
                .expect("QICK_TOKEN_ADDRESS must be set already")
                .clone(),
            "QORTY" => QORTY_TOKEN_ADDRESS
                .expect("QORTY_TOKEN_ADDRESS must be set already")
                .clone(),
            _ => panic!("Expected QICK or QORTY ticker"),
        };
        (
            contract_address,
            QRC20_SWAP_CONTRACT_ADDRESS
                .expect("QRC20_SWAP_CONTRACT_ADDRESS must be set already")
                .clone(),
        )
    };
    let platform = "QTUM";
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let confpath = unsafe { QTUM_CONF_PATH.as_ref().expect("Qtum config is not set yet") };
    let conf = json!({
        "coin":ticker,
        "decimals": 8,
        "required_confirmations":0,
        "pubtype":120,
        "p2shtype":110,
        "wiftype":128,
        "mm2":1,
        "mature_confirmations":500,
        "network":"regtest",
        "confpath": confpath,
        "dust": 72800,
    });
    let req = json!({
        "method": "enable",
        "swap_contract_address": format!("{:#02x}", swap_contract_address),
    });
    let params = Qrc20ActivationParams::from_legacy_req(&req).unwrap();

    let coin = block_on(qrc20_coin_with_priv_key(
        &ctx,
        ticker,
        platform,
        &conf,
        &params,
        priv_key,
        contract_address,
    ))
    .unwrap();

    import_address(&coin);
    (ctx, coin)
}

fn qrc20_coin_conf_item(ticker: &str) -> Json {
    let contract_address = unsafe {
        match ticker {
            "QICK" => QICK_TOKEN_ADDRESS
                .expect("QICK_TOKEN_ADDRESS must be set already")
                .clone(),
            "QORTY" => QORTY_TOKEN_ADDRESS
                .expect("QORTY_TOKEN_ADDRESS must be set already")
                .clone(),
            _ => panic!("Expected either QICK or QORTY ticker, found {}", ticker),
        }
    };
    let contract_address = format!("{:#02x}", contract_address);

    let confpath = unsafe { QTUM_CONF_PATH.as_ref().expect("Qtum config is not set yet") };
    json!({
        "coin":ticker,
        "required_confirmations":1,
        "pubtype":120,
        "p2shtype":110,
        "wiftype":128,
        "mature_confirmations":500,
        "confpath":confpath,
        "network":"regtest",
        "protocol":{"type":"QRC20","protocol_data":{"platform":"QTUM","contract_address":contract_address}}})
}

/// Build asset `UtxoStandardCoin` from ticker and privkey without filling the balance.
pub fn utxo_coin_from_privkey(ticker: &str, priv_key: Secp256k1Secret) -> (MmArc, UtxoStandardCoin) {
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let conf = json!({"asset":ticker,"txversion":4,"overwintered":1,"txfee":1000,"network":"regtest"});
    let req = json!({"method":"enable"});
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(utxo_standard_coin_with_priv_key(&ctx, ticker, &conf, &params, priv_key)).unwrap();
    import_address(&coin);
    (ctx, coin)
}

/// Create a UTXO coin for the given privkey and fill it's address with the specified balance.
pub fn generate_utxo_coin_with_privkey(ticker: &str, balance: BigDecimal, priv_key: Secp256k1Secret) {
    let (_, coin) = utxo_coin_from_privkey(ticker, priv_key);
    let timeout = 30; // timeout if test takes more than 30 seconds to run
    let my_address = coin.my_address().expect("!my_address");
    fill_address(&coin, &my_address, balance, timeout);
}

/// Generate random privkey, create a UTXO coin and fill it's address with the specified balance.
pub fn generate_utxo_coin_with_random_privkey(
    ticker: &str,
    balance: BigDecimal,
) -> (MmArc, UtxoStandardCoin, Secp256k1Secret) {
    let priv_key = random_secp256k1_secret();
    let (ctx, coin) = utxo_coin_from_privkey(ticker, priv_key);
    let timeout = 30; // timeout if test takes more than 30 seconds to run
    let my_address = coin.my_address().expect("!my_address");
    fill_address(&coin, &my_address, balance, timeout);
    (ctx, coin, priv_key)
}

/// Get only one address assigned the specified label.
pub fn get_address_by_label<T>(coin: T, label: &str) -> String
where
    T: AsRef<UtxoCoinFields>,
{
    let native = match coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Native(ref native) => native,
        UtxoRpcClientEnum::Electrum(_) => panic!("NativeClient expected"),
    };
    let mut addresses = native
        .get_addresses_by_label(label)
        .wait()
        .expect("!getaddressesbylabel")
        .into_iter();
    match addresses.next() {
        Some((addr, _purpose)) if addresses.next().is_none() => addr,
        Some(_) => panic!("Expected only one address by {:?}", label),
        None => panic!("Expected one address by {:?}", label),
    }
}

pub fn fill_qrc20_address(coin: &Qrc20Coin, amount: BigDecimal, timeout: u64) {
    // prevent concurrent fill since daemon RPC returns errors if send_to_address
    // is called concurrently (insufficient funds) and it also may return other errors
    // if previous transaction is not confirmed yet
    let _lock = QTUM_LOCK.lock().unwrap();
    let timeout = now_ms() / 1000 + timeout;
    let client = match coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Native(ref client) => client,
        UtxoRpcClientEnum::Electrum(_) => panic!("Expected NativeClient"),
    };

    let from_addr = get_address_by_label(coin, QTUM_ADDRESS_LABEL);
    let to_addr = coin.my_addr_as_contract_addr().unwrap();
    let satoshis = sat_from_big_decimal(&amount, coin.as_ref().decimals).expect("!sat_from_big_decimal");

    let hash = client
        .transfer_tokens(
            &coin.contract_address,
            &from_addr,
            to_addr,
            satoshis.into(),
            coin.as_ref().decimals,
        )
        .wait()
        .expect("!transfer_tokens")
        .txid;

    let tx_bytes = client.get_transaction_bytes(&hash).wait().unwrap();
    log!("{:02x}", tx_bytes);
    coin.wait_for_confirmations(&tx_bytes, 1, false, timeout, 1)
        .wait()
        .unwrap();
}

/// Generate random privkey, create a QRC20 coin and fill it's address with the specified balance.
pub fn generate_qrc20_coin_with_random_privkey(
    ticker: &str,
    qtum_balance: BigDecimal,
    qrc20_balance: BigDecimal,
) -> (MmArc, Qrc20Coin, Secp256k1Secret) {
    let priv_key = random_secp256k1_secret();
    let (ctx, coin) = qrc20_coin_from_privkey(ticker, priv_key);

    let timeout = 30; // timeout if test takes more than 30 seconds to run
    let my_address = coin.my_address().expect("!my_address");
    fill_address(&coin, &my_address, qtum_balance, timeout);
    fill_qrc20_address(&coin, qrc20_balance, timeout);
    (ctx, coin, priv_key)
}

pub fn generate_qtum_coin_with_random_privkey(
    ticker: &str,
    balance: BigDecimal,
    txfee: Option<u64>,
) -> (MmArc, QtumCoin, [u8; 32]) {
    let confpath = unsafe { QTUM_CONF_PATH.as_ref().expect("Qtum config is not set yet") };
    let conf = json!({
        "coin":ticker,
        "decimals":8,
        "required_confirmations":0,
        "pubtype":120,
        "p2shtype": 110,
        "wiftype":128,
        "txfee": txfee,
        "txfee_volatility_percent":0.1,
        "mm2":1,
        "mature_confirmations":500,
        "network":"regtest",
        "confpath": confpath,
        "dust": 72800,
    });
    let req = json!({"method": "enable"});
    let priv_key = random_secp256k1_secret();
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(qtum_coin_with_priv_key(&ctx, "QTUM", &conf, &params, priv_key)).unwrap();

    let timeout = 30; // timeout if test takes more than 30 seconds to run
    let my_address = coin.my_address().expect("!my_address");
    fill_address(&coin, &my_address, balance, timeout);
    (ctx, coin, priv_key.take())
}

pub fn generate_segwit_qtum_coin_with_random_privkey(
    ticker: &str,
    balance: BigDecimal,
    txfee: Option<u64>,
) -> (MmArc, QtumCoin, Secp256k1Secret) {
    let confpath = unsafe { QTUM_CONF_PATH.as_ref().expect("Qtum config is not set yet") };
    let conf = json!({
        "coin":ticker,
        "decimals":8,
        "required_confirmations":0,
        "pubtype":120,
        "p2shtype": 110,
        "wiftype":128,
        "segwit":true,
        "txfee": txfee,
        "txfee_volatility_percent":0.1,
        "mm2":1,
        "mature_confirmations":500,
        "network":"regtest",
        "confpath": confpath,
        "dust": 72800,
        "bech32_hrp":"qcrt",
        "address_format": {
            "format": "segwit",
        },
    });
    let req = json!({"method": "enable"});
    let priv_key = random_secp256k1_secret();
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(qtum_coin_with_priv_key(&ctx, "QTUM", &conf, &params, priv_key)).unwrap();

    let timeout = 30; // timeout if test takes more than 30 seconds to run
    let my_address = coin.my_address().expect("!my_address");
    fill_address(&coin, &my_address, balance, timeout);
    (ctx, coin, priv_key)
}

pub fn fill_address<T>(coin: &T, address: &str, amount: BigDecimal, timeout: u64)
where
    T: MarketCoinOps + AsRef<UtxoCoinFields>,
{
    // prevent concurrent fill since daemon RPC returns errors if send_to_address
    // is called concurrently (insufficient funds) and it also may return other errors
    // if previous transaction is not confirmed yet
    let mutex = match coin.ticker() {
        "MYCOIN" => &*MY_COIN_LOCK,
        "MYCOIN1" => &*MY_COIN1_LOCK,
        "QTUM" | "QICK" | "QORTY" => &*QTUM_LOCK,
        "FORSLP" => &*FOR_SLP_LOCK,
        ticker => panic!("Unknown ticker {}", ticker),
    };
    let _lock = mutex.lock().unwrap();
    let timeout = now_ms() / 1000 + timeout;

    if let UtxoRpcClientEnum::Native(client) = &coin.as_ref().rpc_client {
        client.import_address(address, address, false).wait().unwrap();
        let hash = client.send_to_address(address, &amount).wait().unwrap();
        let tx_bytes = client.get_transaction_bytes(&hash).wait().unwrap();
        coin.wait_for_confirmations(&tx_bytes, 1, false, timeout, 1)
            .wait()
            .unwrap();
        log!("{:02x}", tx_bytes);
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

/// Wait for the `estimatesmartfee` returns no errors.
pub fn wait_for_estimate_smart_fee(timeout: u64) -> Result<(), String> {
    enum EstimateSmartFeeState {
        Idle,
        Ok,
        NotAvailable,
    }
    lazy_static! {
        static ref LOCK: Mutex<EstimateSmartFeeState> = Mutex::new(EstimateSmartFeeState::Idle);
    }

    let state = &mut *LOCK.lock().unwrap();
    match state {
        EstimateSmartFeeState::Ok => return Ok(()),
        EstimateSmartFeeState::NotAvailable => return ERR!("estimatesmartfee not available"),
        EstimateSmartFeeState::Idle => log!("Start wait_for_estimate_smart_fee"),
    }

    let priv_key = random_secp256k1_secret();
    let (_ctx, coin) = qrc20_coin_from_privkey("QICK", priv_key);
    let timeout = now_ms() / 1000 + timeout;
    let client = match coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Native(ref client) => client,
        UtxoRpcClientEnum::Electrum(_) => panic!("Expected NativeClient"),
    };
    while now_ms() / 1000 < timeout {
        if let Ok(res) = client.estimate_smart_fee(&None, 1).wait() {
            if res.errors.is_empty() {
                *state = EstimateSmartFeeState::Ok;
                return Ok(());
            }
        }
        thread::sleep(Duration::from_secs(1));
    }

    *state = EstimateSmartFeeState::NotAvailable;
    ERR!("Waited too long for estimate_smart_fee to work")
}

pub async fn enable_qrc20_native(mm: &MarketMakerIt, coin: &str) -> Json {
    let swap_contract_address = unsafe {
        QRC20_SWAP_CONTRACT_ADDRESS
            .expect("QRC20_SWAP_CONTRACT_ADDRESS must be set already")
            .clone()
    };

    let native = mm
        .rpc(&json! ({
            "userpass": mm.userpass,
            "method": "enable",
            "coin": coin,
            "swap_contract_address": format!("{:#02x}", swap_contract_address),
            "mm2": 1,
        }))
        .await
        .unwrap();
    assert_eq!(native.0, StatusCode::OK, "'enable' failed: {}", native.1);
    json::from_str(&native.1).unwrap()
}

pub fn trade_base_rel((base, rel): (&str, &str)) {
    /// Generate a wallet with the random private key and fill the wallet with Qtum (required by gas_fee) and specified in `ticker` coin.
    fn generate_and_fill_priv_key(ticker: &str) -> Secp256k1Secret {
        let timeout = 30; // timeout if test takes more than 30 seconds to run

        match ticker {
            "QTUM" => {
                //Segwit QTUM
                wait_for_estimate_smart_fee(timeout).expect("!wait_for_estimate_smart_fee");
                let (_ctx, _coin, priv_key) = generate_segwit_qtum_coin_with_random_privkey("QTUM", 10.into(), Some(0));

                priv_key
            },
            "QICK" | "QORTY" => {
                let priv_key = random_secp256k1_secret();
                let (_ctx, coin) = qrc20_coin_from_privkey(ticker, priv_key);
                let my_address = coin.my_address().expect("!my_address");
                fill_address(&coin, &my_address, 10.into(), timeout);
                fill_qrc20_address(&coin, 10.into(), timeout);

                priv_key
            },
            "MYCOIN" | "MYCOIN1" => {
                let priv_key = random_secp256k1_secret();
                let (_ctx, coin) = utxo_coin_from_privkey(ticker, priv_key);
                let my_address = coin.my_address().expect("!my_address");
                fill_address(&coin, &my_address, 10.into(), timeout);
                // also fill the Qtum
                let (_ctx, coin) = qrc20_coin_from_privkey("QICK", priv_key);
                let my_address = coin.my_address().expect("!my_address");
                fill_address(&coin, &my_address, 10.into(), timeout);

                priv_key
            },
            "ADEXSLP" | "FORSLP" => Secp256k1Secret::from(get_prefilled_slp_privkey()),
            _ => panic!("Expected either QICK or QORTY or MYCOIN or MYCOIN1, found {}", ticker),
        }
    }

    let bob_priv_key = generate_and_fill_priv_key(base);
    let alice_priv_key = generate_and_fill_priv_key(rel);

    let confpath = unsafe { QTUM_CONF_PATH.as_ref().expect("Qtum config is not set yet") };
    let coins = json! ([
        qrc20_coin_conf_item("QICK"),
        qrc20_coin_conf_item("QORTY"),
        {"coin":"MYCOIN","asset":"MYCOIN","required_confirmations":0,"txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        {"coin":"MYCOIN1","asset":"MYCOIN1","required_confirmations":0,"txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"UTXO"}},
        {"coin":"QTUM","asset":"QTUM","required_confirmations":0,"decimals":8,"pubtype":120,"p2shtype":110,"wiftype":128,"segwit":true,"txfee":0,"txfee_volatility_percent":0.1,
        "mm2":1,"network":"regtest","confpath":confpath,"protocol":{"type":"UTXO"},"bech32_hrp":"qcrt","address_format":{"format":"segwit"}},
        {"coin":"FORSLP","asset":"FORSLP","required_confirmations":0,"txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"BCH","protocol_data":{"slp_prefix":"slptest"}}},
        {"coin":"ADEXSLP","protocol":{"type":"SLPTOKEN","protocol_data":{"decimals":8,"token_id":get_slp_token_id(),"platform":"FORSLP"}}}
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
    block_on(mm_bob.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))).unwrap();

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
    block_on(mm_alice.wait_for_log(22., |log| log.contains(">>>>>>>>> DEX stats "))).unwrap();

    log!("{:?}", block_on(enable_qrc20_native(&mm_bob, "QICK")));
    log!("{:?}", block_on(enable_qrc20_native(&mm_bob, "QORTY")));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN", &[])));
    log!("{:?}", block_on(enable_native(&mm_bob, "MYCOIN1", &[])));
    log!("{:?}", block_on(enable_native(&mm_bob, "QTUM", &[])));
    log!("{:?}", block_on(enable_native_bch(&mm_bob, "FORSLP", &[])));
    log!("{:?}", block_on(enable_native(&mm_bob, "ADEXSLP", &[])));

    log!("{:?}", block_on(enable_qrc20_native(&mm_alice, "QICK")));
    log!("{:?}", block_on(enable_qrc20_native(&mm_alice, "QORTY")));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN", &[])));
    log!("{:?}", block_on(enable_native(&mm_alice, "MYCOIN1", &[])));
    log!("{:?}", block_on(enable_native(&mm_alice, "QTUM", &[])));
    log!("{:?}", block_on(enable_native_bch(&mm_alice, "FORSLP", &[])));
    log!("{:?}", block_on(enable_native(&mm_alice, "ADEXSLP", &[])));
    let rc = block_on(mm_bob.rpc(&json! ({
        "userpass": mm_bob.userpass,
        "method": "setprice",
        "base": base,
        "rel": rel,
        "price": 1,
        "volume": "3",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!setprice: {}", rc.1);

    thread::sleep(Duration::from_secs(1));

    log!("Issue alice {}/{} buy request", base, rel);
    let rc = block_on(mm_alice.rpc(&json! ({
        "userpass": mm_alice.userpass,
        "method": "buy",
        "base": base,
        "rel": rel,
        "price": 1,
        "volume": "2",
    })))
    .unwrap();
    assert!(rc.0.is_success(), "!buy: {}", rc.1);
    let buy_json: Json = serde_json::from_str(&rc.1).unwrap();
    let uuid = buy_json["result"]["uuid"].as_str().unwrap().to_owned();

    // ensure the swaps are started
    block_on(mm_bob.wait_for_log(22., |log| {
        log.contains(&format!("Entering the maker_swap_loop {}/{}", base, rel))
    }))
    .unwrap();
    block_on(mm_alice.wait_for_log(22., |log| {
        log.contains(&format!("Entering the taker_swap_loop {}/{}", base, rel))
    }))
    .unwrap();

    // ensure the swaps are finished
    block_on(mm_bob.wait_for_log(600., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))).unwrap();
    block_on(mm_alice.wait_for_log(600., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))).unwrap();

    log!("Checking alice/taker status..");
    block_on(check_my_swap_status(
        &mm_alice,
        &uuid,
        &TAKER_SUCCESS_EVENTS,
        &TAKER_ERROR_EVENTS,
        "2".parse().unwrap(),
        "2".parse().unwrap(),
    ));

    log!("Checking bob/maker status..");
    block_on(check_my_swap_status(
        &mm_bob,
        &uuid,
        &MAKER_SUCCESS_EVENTS,
        &MAKER_ERROR_EVENTS,
        "2".parse().unwrap(),
        "2".parse().unwrap(),
    ));

    log!("Waiting 3 seconds for nodes to broadcast their swaps data..");
    thread::sleep(Duration::from_secs(3));

    log!("Checking alice status..");
    block_on(check_stats_swap_status(
        &mm_alice,
        &uuid,
        &MAKER_SUCCESS_EVENTS,
        &TAKER_SUCCESS_EVENTS,
    ));

    log!("Checking bob status..");
    block_on(check_stats_swap_status(
        &mm_bob,
        &uuid,
        &MAKER_SUCCESS_EVENTS,
        &TAKER_SUCCESS_EVENTS,
    ));

    log!("Checking alice recent swaps..");
    block_on(check_recent_swaps(&mm_alice, 1));
    log!("Checking bob recent swaps..");
    block_on(check_recent_swaps(&mm_bob, 1));

    block_on(mm_bob.stop()).unwrap();
    block_on(mm_alice.stop()).unwrap();
}

pub fn slp_supplied_node() -> MarketMakerIt {
    let coins = json! ([
        {"coin":"FORSLP","asset":"FORSLP","required_confirmations":0,"txversion":4,"overwintered":1,"txfee":1000,"protocol":{"type":"BCH","protocol_data":{"slp_prefix":"slptest"}}},
        {"coin":"ADEXSLP","protocol":{"type":"SLPTOKEN","protocol_data":{"decimals":8,"token_id":get_slp_token_id(),"platform":"FORSLP"}}}
    ]);

    let priv_key = get_prefilled_slp_privkey();
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

    mm
}

pub fn solana_supplied_node() -> MarketMakerIt {
    let coins = json! ([
        {"coin": "SOL-DEVNET","name": "solana","fname": "Solana","rpcport": 80,"mm2": 1,"required_confirmations": 1,"avg_blocktime": 0.25,"protocol": {"type": "SOLANA"}},
        {"coin":"USDC-SOL-DEVNET","protocol":{"type":"SPLTOKEN","protocol_data":{"decimals":6,"token_contract_address":"4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU","platform":"SOL-DEVNET"}},"mm2": 1},
        {"coin":"ADEX-SOL-DEVNET","protocol":{"type":"SPLTOKEN","protocol_data":{"decimals":9,"token_contract_address":"5tSm6PqMosy1rz1AqV3kD28yYT5XqZW3QYmZommuFiPJ","platform":"SOL-DEVNET"}},"mm2": 1},
    ]);

    let mm = MarketMakerIt::start(
        json! ({
            "gui": "nogui",
            "netid": 9000,
            "dht": "on",  // Enable DHT without delay.
            "passphrase": "federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron",
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".to_string(),
        None,
    )
    .unwrap();

    mm
}

pub fn get_balance(mm: &MarketMakerIt, coin: &str) -> MyBalanceResponse {
    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "my_balance",
        "coin": coin,
    })))
    .unwrap();
    assert_eq!(rc.0, StatusCode::OK, "my_balance request failed {}", rc.1);
    json::from_str(&rc.1).unwrap()
}

pub fn utxo_burn_address() -> Address {
    Address {
        prefix: 60,
        hash: AddressHashEnum::default_address_hash(),
        t_addr_prefix: 0,
        checksum_type: ChecksumType::DSHA256,
        hrp: None,
        addr_format: UtxoAddressFormat::Standard,
    }
}

pub fn withdraw_max_and_send_v1(mm: &MarketMakerIt, coin: &str, to: &str) -> TransactionDetails {
    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "withdraw",
        "coin": coin,
        "max": true,
        "to": to,
    })))
    .unwrap();
    assert_eq!(rc.0, StatusCode::OK, "withdraw request failed {}", rc.1);
    let tx_details: TransactionDetails = json::from_str(&rc.1).unwrap();

    let rc = block_on(mm.rpc(&json!({
        "userpass": mm.userpass,
        "method": "send_raw_transaction",
        "tx_hex": tx_details.tx_hex,
        "coin": coin,
    })))
    .unwrap();
    assert_eq!(rc.0, StatusCode::OK, "send_raw_transaction request failed {}", rc.1);

    tx_details
}
