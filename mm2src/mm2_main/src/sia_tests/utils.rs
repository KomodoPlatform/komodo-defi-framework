use crate::lp_native_dex::lp_init;
use coins::siacoin::sia_rust::types::Address;
use coins::siacoin::{SiaApiClient, SiaClientConf, SiaClientType as SiaClient};
use coins::utxo::zcash_params_path;

use common::log::{LogLevel, UnifiedLoggerBuilder};
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};
use mm2_rpc::data::legacy::CoinInitResponse;
use mm2_test_helpers::for_tests::MarketMakerIt;

use chrono::Local;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::HashMap;
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;
use testcontainers::clients::Cli;
use testcontainers::{core::WaitFor, Container, GenericImage, RunnableImage};
use url::Url;

mod komodod_client;
pub use komodod_client::*;

/// Filename for the log file for each test utilizing `init_test_dir()`
/// Each MarketMaker instance will log to <temp directory>/kdf.log generally.
const LOG_FILENAME: &str = "kdf.log";

pub const ALICE_KMD_WIF: &str = "UqubgosgQT3cjt488P2qLoqP3oMGgNccXHTGeVQBSUFsMwCA459Q";
pub const ALICE_KMD_ADDRESS: &str = "RNa3bJJC2L3UUCGQ9WY5fhCSzSd5ExiAWr";
pub const ALICE_KMD_PUBLIC_KEY: &str = "033ca097f047603318d7191ecb8e75b96a15b6bfac97853c4f25619177c5992427";
pub const ALICE_SIA_ADDRESS_STR: &str = "a0cfbc1089d129f52d00bc0b0fac190d4d87976a1d7f34da7ca0c295c99a628de344d19ad469";
pub const ALICE_KMD_KEY: TestKeyPair = TestKeyPair {
    address: ALICE_KMD_ADDRESS,
    pubkey: ALICE_KMD_PUBLIC_KEY,
    wif: ALICE_KMD_WIF,
};

pub const BOB_KMD_WIF: &str = "UvU3bn2bucriZVDaSSB51aGGu9emUbmf9ZK72sdRjrD2Vb4smQ8T";
pub const BOB_KMD_ADDRESS: &str = "RLHqXM7q689D1PZvt9nH5nmouSPMG9sopG";
pub const BOB_KMD_PUBLIC_KEY: &str = "02f5e06a51ac7723d8d07792b6b2f36e7953264ce0756006c3859baaad4c016266";
pub const BOB_SIA_ADDRESS_STR: &str = "c34caa97740668de2bbdb7174572ed64c861342bf27e80313cbfa02e9251f52e30aad3892533";
pub const BOB_KMD_KEY: TestKeyPair = TestKeyPair {
    address: BOB_KMD_ADDRESS,
    pubkey: BOB_KMD_PUBLIC_KEY,
    wif: BOB_KMD_WIF,
};

pub const CHARLIE_KMD_WIF: &str = "UxBSSjJ8TDPJd3PofYDVjkEoBHVhRgh1fF8h2ge579aRyJcS5AoS";
pub const CHARLIE_KMD_ADDRESS: &str = "RQwbjQqzcEKUN4DVbh1MpE1NaVek9qEJBg";
pub const CHARLIE_KMD_PUBLIC_KEY: &str = "024429b5eeb590fef378945f847459f8c186c6d6216123e3b058fb6c6fadece454";
pub const CHARLIE_SIA_ADDRESS_STR: &str =
    "465f2b9e9e3bae4903c5b449ea896087b4a9f19b5063bcbbc8e0340772d1dc5afa323bdc2faa";
pub const CHARLIE_KMD_KEY: TestKeyPair = TestKeyPair {
    address: CHARLIE_KMD_ADDRESS,
    pubkey: CHARLIE_KMD_PUBLIC_KEY,
    wif: CHARLIE_KMD_WIF,
};

/// Used inconjunction with init_test_dir() to create a unique directory for each test
/// Not intended to be used otherwise due to hardcoded suffix value.
#[macro_export]
macro_rules! current_function_name {
    () => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str { std::any::type_name::<T>() }
        let name = type_name_of(f);
        name.strip_suffix("::{{closure}}::f")
            .unwrap()
            .rsplit("::")
            .next()
            .unwrap()
    }};
}

pub(crate) use current_function_name;

lazy_static! {
    pub static ref DOCKER: Cli = Cli::default();

    pub static ref COINS: Json = json!(
        [
            // Dockerized Sia coin
            {
                "coin": "DSIA",
                "mm2": 1,
                "required_confirmations": 1,
                "protocol": {
                "type": "SIA"
                }
            },
            // Dockerized UTXO coin
            // init_alice and init_bob both rely on this being COINS[1] while setting 'confpath'
            {
                "coin": "DUTXO",
                "asset": "DUTXO",
                "fname": "DUTXO",
                "rpcport": 10001,
                "txversion": 4,
                "overwintered": 1,
                "mm2": 1,
                "sign_message_prefix": "Komodo Signed Message:\n",
                "is_testnet": true,
                "required_confirmations": 1,
                "requires_notarization": false,
                "avg_blocktime": 60,
                "protocol": {
                "type": "UTXO"
                },
                "derivation_path": "m/44'/141'",
                "trezor_coin": "Komodo"
            },
            {
                "coin": "DOC",
                "asset": "DOC",
                "fname": "DOC",
                "rpcport": 62415,
                "txversion": 4,
                "overwintered": 1,
                "mm2": 1,
                "sign_message_prefix": "Komodo Signed Message:\n",
                "is_testnet": true,
                "required_confirmations": 1,
                "requires_notarization": false,
                "avg_blocktime": 60,
                "protocol": {
                "type": "UTXO"
                },
                "derivation_path": "m/44'/141'",
                "trezor_coin": "Komodo"
            },
        ]
    );

    /// Sia Address from the iguana seed "buyer buyer buyer buyer buyer buyer buyer buyer buyer buyer buyer cabin"
    pub static ref ALICE_SIA_ADDRESS: Address = Address::from_str(ALICE_SIA_ADDRESS_STR).unwrap();

    /// Sia Address from the iguana seed "sell sell sell sell sell sell sell sell sell sell sell sell"
    pub static ref BOB_SIA_ADDRESS: Address = Address::from_str(BOB_SIA_ADDRESS_STR).unwrap();

    /// A Sia Address that is not Alice's or Bob's
    pub static ref CHARLIE_SIA_ADDRESS: Address = Address::from_str(CHARLIE_SIA_ADDRESS_STR).unwrap();
}

pub struct TestKeyPair<'a> {
    pub address: &'a str,
    pub pubkey: &'a str,
    pub wif: &'a str,
}

/// Response from `get_directly_connected_peers` RPC endpoint.
/// eg, {"<PeerId>": ["<Multiaddr>", "<Multiaddr>"], "<PeerId>": ["<Multiaddr>"]}}
/// TODO: Should technically be HashMap<Peerid, Vec<Multiaddr>> but not needed for current use cases.
#[derive(Debug, Serialize, Deserialize)]
#[serde(transparent, rename = "result")]
pub struct GetDirectlyConnectedPeersResponse(pub HashMap<String, Vec<String>>);

/// Response from `get_my_peer_id` RPC endpoint.
/// eg, {"result:" "<PeerId>"}
/// TODO: Should technically be Peerid but not needed for current use cases.
#[derive(Debug, Serialize, Deserialize)]
pub struct GetMyPeerIdResponse {
    pub result: String,
}

pub async fn enable_dsia(mm: &MarketMakerIt, walletd_port: u16) -> CoinInitResponse {
    let url = format!("http://127.0.0.1:{}/", walletd_port);
    mm.rpc_typed::<CoinInitResponse>(&json!({
        "method": "enable",
        "coin": "DSIA",
        "tx_history": true,
        "client_conf": {
            "server_url": url,
            "password": "password"
        }
    }))
    .await
    .unwrap()
}

pub async fn enable_dutxo(mm: &MarketMakerIt) -> CoinInitResponse {
    mm.rpc_typed::<CoinInitResponse>(&json!({
        "method": "enable",
        "coin": "DUTXO",
        "tx_history": true
    }))
    .await
    .unwrap()
}

/// Create a unique directory for each test case.
/// This relies on std::env::temp_dir() so it will only be cleaned up when the OS chooses to do so.
/// This is intended for CI/CD pipelines as they are generally run on temporary VMs.
/// Additionally sets the MM_LOG environment variable to the log file in the temp directory.
pub fn init_test_dir(fn_path: &str) -> PathBuf {
    let init_time = Local::now().format("%Y-%m-%d_%H-%M-%S-%3f").to_string();

    // Initialize env_logger that is shared amongst all KDF instances
    UnifiedLoggerBuilder::new().init();

    let test_case = format!("kdf_{}_{}", fn_path, init_time);
    let temp_dir = std::env::temp_dir().join(test_case);

    // MarketMakerIt::wait_for_log() requires MM_LOG to be set
    std::env::set_var("MM_LOG", temp_dir.join(LOG_FILENAME).to_str().unwrap());
    std::fs::create_dir_all(&temp_dir).unwrap();
    temp_dir
}

/**
Initialize a MarketMaker instance with a configuration suitable for the taker aka Alice.

Intended to be used in conjunction with `init_bob` to create a taker/maker setup.

This node will not act as a seed node and will not listen on the p2p port.

This node will attempt to connect to a seed node on the host that is using the same
`netid` value. ie, `localhost:<p2p_port>` where <p2p_port> is influenced by the `netid` value.

`rpc_port` - The port the MarketMaker instance will listen on for RPC commands.
`netid` - The network id for the MarketMaker instance. This directly influences the p2p port
          used to comminucate with other MarketMaker instances. This is not the literal port number
          but rather the input to the function `mm2_main::lp_network::lp_ports`.
`utxo_rpc_port` - If set, enables the marketmaker instance to connect to a native UTXO node at the given
    port. This is only needed if multiple *native UTXO nodes for the same coin* are being used.
    Eg, Alice's personal DUTXO node http://test:test@127.0.0.1:10001/
        and Bob's http://test:test@127.0.0.1:10000/

Use unique values for `rpc_port`` and `netid`` for each test if they are intended to run concurrently
alongside other unrelated tests.

All configurations other than rpc_port and netid are hardcoded for simplicity.
**/
pub async fn init_alice(kdf_dir: &Path, netid: u16, utxo_rpc_port: Option<u16>) -> (MmArc, MarketMakerIt) {
    let alice_db_dir = kdf_dir.join("DB_alice");
    let test_case_string = kdf_dir.to_str().unwrap().to_string();
    let datetime = "init_alice".to_string();
    let ip = IpAddr::from([127, 0, 0, 1]);

    // `enable` method using native UTXO node is too stupid to allow setting the rpc credentials anywhere
    // other than a config file specified in the coins json. So using different UTXO nodes for Alice
    // and Bob means we need a unique coins json for each. If `utxo_rpc_port` is set, we create the
    // equivalent of `~/.komodo/DUTXO/DUTXO.conf` that would typically be created by Komodod and set
    // DUTXO['confpath'] to that file.
    let alice_coins = match utxo_rpc_port {
        Some(utxo_port) => {
            let mut coins = COINS.clone();
            let file_contents = format!("rpcuser=test\nrpcpassword=test\nrpcport={}\n", utxo_port);
            let utxo_conf_file_path = kdf_dir.join("ALICE_DUTXO.conf");
            let mut conf_file = std::fs::File::create(&utxo_conf_file_path).unwrap();
            conf_file.write_all(file_contents.as_bytes()).unwrap();
            coins[1]["confpath"] = json!(utxo_conf_file_path);
            coins
        },
        None => COINS.clone(),
    };

    let alice_conf = json!({
        "gui": format!("{}_alice", test_case_string),
        "netid": netid,
        "passphrase": "buyer buyer buyer buyer buyer buyer buyer buyer buyer buyer buyer cabin",
        "coins": alice_coins,
        "myipaddr": ip.to_string(),
        "rpc_password": "password",
        "rpcport": 0, // 0 value will assign an available port that can be read from ctx.rpc_started
        "i_am_seed": false,
        "enable_hd": false,
        "dbdir": alice_db_dir.to_str().unwrap(),
        "seednodes": [
            "127.0.0.1"
        ]
    });

    let ctx = MmCtxBuilder::new()
        .with_conf(alice_conf)
        .with_log_level(LogLevel::Debug)
        .with_version(test_case_string.clone())
        .with_datetime(datetime.clone())
        .into_mm_arc();
    let ctx_clone = ctx.clone();
    tokio::spawn(async move { lp_init(ctx, test_case_string, datetime).await.unwrap() });

    wait_for_rpc_started(ctx_clone.clone(), Duration::from_secs(20))
        .await
        .unwrap();
    let rpc_port = *ctx_clone.rpc_port.get().unwrap();

    let mm_alice = MarketMakerIt {
        folder: alice_db_dir,
        ip,
        rpc_port: Some(rpc_port),
        log_path: kdf_dir.join(LOG_FILENAME),
        pc: None,
        userpass: "password".to_string(),
    };

    (ctx_clone, mm_alice)
}

/**
Initialize a MarketMaker instance with a configuration suitable for the maker aka Bob.

Intended to be used in conjunction with `init_alice` to create a taker/maker setup.

This node will act as a seed node and will listen on the p2p port.

`rpc_port` - The port the MarketMaker instance will listen on for RPC commands.
`netid` - The network id for the MarketMaker instance. This directly influences the p2p port
          used to comminucate with other MarketMaker instances. This is not the literal port number
          but rather the input to the function `mm2_main::lp_network::lp_ports`.
`utxo_rpc_port` - If set, enables the marketmaker instance to connect to a native UTXO node at the given
    port. This is only needed if multiple *native UTXO nodes for the same coin* are being used.
    Eg, Alice's personal DUTXO node http://test:test@127.0.0.1:10001/
        and Bob's http://test:test@127.0.0.1:10000/

Use unique values for `rpc_port`` and `netid`` for each test if they are intended to run concurrently
alongside other unrelated tests.

All configurations other than rpc_port and netid are hardcoded for simplicity.
**/
pub async fn init_bob(kdf_dir: &Path, netid: u16, utxo_rpc_port: Option<u16>) -> (MmArc, MarketMakerIt) {
    let bob_db_dir = kdf_dir.join("DB_bob");
    let test_case_string = kdf_dir.to_str().unwrap().to_string();
    let datetime = "init_bob".to_string();
    let ip = IpAddr::from([127, 0, 0, 1]);

    // `enable` method using native UTXO node is too stupid to allow setting the rpc credentials anywhere
    // other than a config file specified in the coins json. So using different UTXO nodes for Alice
    // and Bob means we need a unique coins json for each. If `utxo_rpc_port` is set, we create the
    // equivalent of `~/.komodo/DUTXO/DUTXO.conf` that would typically be created by Komodod and set
    // DUTXO['confpath'] to that file.
    let coins = match utxo_rpc_port {
        Some(utxo_port) => {
            let mut coins = COINS.clone();
            let file_contents = format!("rpcuser=test\nrpcpassword=test\nrpcport={}\n", utxo_port);
            let utxo_conf_file_path = kdf_dir.join("BOB_DUTXO.conf");
            let mut conf_file = std::fs::File::create(&utxo_conf_file_path).unwrap();
            conf_file.write_all(file_contents.as_bytes()).unwrap();
            coins[1]["confpath"] = json!(utxo_conf_file_path);
            coins
        },
        None => COINS.clone(),
    };

    let bob_conf = json!({
        "gui": format!("{}_bob", test_case_string),
        "netid": netid,
        "passphrase": "sell sell sell sell sell sell sell sell sell sell sell sell",
        "coins": coins,
        "myipaddr": ip.to_string(),
        "rpc_password": "password",
        "rpcport": 0, // 0 value will assign an available port that can be read from ctx.rpc_started
        "i_am_seed": true,
        "enable_hd": false,
        "dbdir": bob_db_dir.to_str().unwrap(),
    });

    let ctx = MmCtxBuilder::new()
        .with_conf(bob_conf)
        .with_log_level(LogLevel::Debug)
        .with_version(test_case_string.clone())
        .with_datetime(datetime.clone())
        .into_mm_arc();
    let ctx_clone = ctx.clone();
    tokio::spawn(async move { lp_init(ctx, test_case_string, datetime).await.unwrap() });

    wait_for_rpc_started(ctx_clone.clone(), Duration::from_secs(20))
        .await
        .unwrap();

    let rpc_port = *ctx_clone.rpc_port.get().unwrap();

    let mm_bob = MarketMakerIt {
        folder: bob_db_dir,
        ip,
        rpc_port: Some(rpc_port),
        log_path: kdf_dir.join(LOG_FILENAME),
        pc: None,
        userpass: "password".to_string(),
    };

    (ctx_clone, mm_bob)
}

/// Initialize a Sia standalone SiaClient.
/// This is useful to interact with a Sia testnet container for commands that are not from Alice or
/// Bob. Eg, mining blocks to progress the chain.
#[allow(dead_code)]
pub async fn init_sia_client(ip: &str, port: u16, password: &str) -> SiaClient {
    let conf = SiaClientConf {
        server_url: Url::parse(&format!("http://{}:{}/", ip, port)).unwrap(),
        password: Some(password.to_string()),
        timeout: Some(10),
    };
    SiaClient::new(conf).await.unwrap()
}

/// Initialize a walletd docker container with walletd API bound to a random port on the host.
/// Returns the container and the host port it is bound to.
/// The container will run until it falls out of scope.
pub fn init_walletd_container(docker: &Cli) -> (Container<GenericImage>, u16) {
    // Define the Docker image with a tag
    let image = GenericImage::new("docker.io/alrighttt/walletd-komodo", "latest")
        .with_exposed_port(9980)
        .with_wait_for(WaitFor::message_on_stdout("node started"));

    // Wrap the image in `RunnableImage` to allow custom port mapping to an available host port
    // 0 indicates that the host port will be automatically assigned to an available port
    let runnable_image = RunnableImage::from(image).with_mapped_port((0, 9980));

    // Start the container. It will run until `Container` falls out of scope
    let container = docker.run(runnable_image);

    // Retrieve the host port that is mapped to the container's 9980 port
    let host_port = container.get_host_port_ipv4(9980);

    (container, host_port)
}

// Initialize a container with 2 komodod nodes.
// Binds "main" node(has address imported and mines blocks) to `port`
// Binds additional node to `port` - 1
// Auth for both nodes is "test:test"
pub fn init_komodod_container(docker: &Cli) -> (Container<'_, GenericImage>, u16, u16) {
    // the ports komodod will listen on the container's network interface
    let mining_node_port = 10000;
    let nonmining_node_port = mining_node_port - 1;
    let image = GenericImage::new("docker.io/artempikulin/testblockchain", "multiarch")
        .with_volume(zcash_params_path().display().to_string(), "/root/.zcash-params")
        .with_env_var("CLIENTS", "2")
        .with_env_var("CHAIN", "ANYTHING")
        .with_env_var("TEST_ADDY", CHARLIE_KMD_KEY.address)
        .with_env_var("TEST_WIF", CHARLIE_KMD_KEY.wif)
        .with_env_var("TEST_PUBKEY", CHARLIE_KMD_KEY.pubkey)
        .with_env_var("DAEMON_URL", "http://test:test@127.0.0.1:7000")
        .with_env_var("COIN", "Komodo")
        .with_env_var("COIN_RPC_PORT", nonmining_node_port.to_string())
        .with_wait_for(WaitFor::message_on_stdout("'name': 'ANYTHING'"));
    let image = RunnableImage::from(image)
        .with_mapped_port((mining_node_port, mining_node_port))
        .with_mapped_port((nonmining_node_port, nonmining_node_port));
    let container = docker.run(image);
    let mining_host_port = container.get_host_port_ipv4(mining_node_port);
    let nonmining_host_port = container.get_host_port_ipv4(nonmining_node_port);
    (container, mining_host_port, nonmining_host_port)
}

/** Initialize a container with 2 komodod nodes and their respective clients.
Mines all blocks to CHARLIE_KMD_KEY including the premine amount of 10,000,000,000 coins
Imports CHARLIE_KMD_KEY.wif to miner node then funds funded_key.address with 1,000,000 coins
Imports funded_key.address to miner node and unfunded_key.address to nonminer node

Returns the container and both clients.
The docker container will run until this container falls out of scope.
**/
pub async fn init_komodod_clients<'a>(
    docker: &'a Cli,
    funded_key: TestKeyPair<'_>,
    unfunded_key: TestKeyPair<'_>,
) -> (Container<'a, GenericImage>, (KomododClient, KomododClient)) {
    let (container, funded_port, unfunded_port) = init_komodod_container(docker);
    let miner_client_conf = KomododClientConf {
        ip: IpAddr::from([127, 0, 0, 1]),
        port: funded_port,
        rpcuser: "test".to_string(),
        rpcpassword: "test".to_string(),
        timeout: None,
    };

    let nonminer_client_conf = KomododClientConf {
        ip: IpAddr::from([127, 0, 0, 1]),
        port: unfunded_port,
        rpcuser: "test".to_string(),
        rpcpassword: "test".to_string(),
        timeout: None,
    };

    let miner = KomododClient::new(miner_client_conf).await;
    let nonminer = KomododClient::new(nonminer_client_conf).await;

    // import Charlie's private key to miner node to allow spending the premined coins
    let _ = miner.rpc("importprivkey", json!([CHARLIE_KMD_KEY.wif])).await;

    // Send 1,000,000 coins from Charlie to funded_key.address
    let _ = miner.rpc("sendtoaddress", json!([funded_key.address, 1000000])).await;

    // Import funded_key.address to miner node and unfunded_key.address to nonminer node
    let _ = miner.rpc("importaddress", json!([funded_key.address])).await;
    let _ = nonminer.rpc("importaddress", json!([unfunded_key.address])).await;

    (container, (miner, nonminer))
}

// Wait for `ctx.rpc_started.is_some()` or timeout
pub async fn wait_for_rpc_started(ctx: MmArc, timeout_duration: Duration) -> Result<(), String> {
    let start_time = tokio::time::Instant::now();
    common::log::debug!("Waiting for RPC to start");
    loop {
        {
            if ctx.rpc_port.get().is_some() {
                return Ok(());
            }
        }

        // Check if we've reached the timeout
        if start_time.elapsed() >= timeout_duration {
            common::log::debug!("Timed out waiting for RPC to start");
            return Err("Timed out waiting for RPC to start".to_string());
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

// Wait until Alice connects to Bob as a peer or timeout
pub async fn wait_for_peers_connected(
    alice: &MarketMakerIt,
    bob: &MarketMakerIt,
    timeout_duration: Duration,
) -> Result<(), ()> {
    let start_time = tokio::time::Instant::now();

    // fetch Bob's PeerId
    let bob_peer_id = bob
        .rpc_typed::<String>(&json!({"method": "get_my_peer_id"}))
        .await
        .unwrap();

    loop {
        // fetch Alice's connected peers
        let alice_peers = alice
            .rpc_typed::<GetDirectlyConnectedPeersResponse>(&json!({"method": "get_directly_connected_peers"}))
            .await
            .unwrap();

        // Check if Bob's PeerId is in Alice's connected peers
        if alice_peers.0.contains_key(&bob_peer_id) {
            return Ok(());
        }

        // Check if we've reached the timeout
        if start_time.elapsed() >= timeout_duration {
            return Err(()); // Timed out
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
