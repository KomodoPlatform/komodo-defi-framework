use crate::lp_native_dex::lp_init;
use coins::siacoin::sia_rust::transport::endpoints::DebugMineRequest;
use coins::siacoin::sia_rust::types::Address;
use coins::siacoin::{client_error::ClientError as SiaClientError, SiaApiClient, SiaClientConf,
                     SiaClientType as SiaClient};

use common::log::{LogLevel, UnifiedLoggerBuilder};
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};
use mm2_test_helpers::for_tests::MarketMakerIt;

use mm2_rpc::data::legacy::CoinInitResponse;

use chrono::Local;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use testcontainers::clients::Cli;
use testcontainers::{Container, GenericImage, RunnableImage};
use tokio::task::yield_now;
use url::Url;

/// Filename for the log file for each test utilizing `init_test_dir()`
/// Each MarketMaker instance will log to <temp directory>/kdf.log generally.
const LOG_FILENAME: &str = "kdf.log";

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
    pub static ref COINS: Json = json!(
        [
            {
                "coin": "DSIA",
                "mm2": 1,
                "required_confirmations": 1,
                "protocol": {
                "type": "SIA"
                }
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
            }
        ]
    );

    /// Sia Address from the iguana seed "sell sell sell sell sell sell sell sell sell sell sell sell"
    pub static ref BOB_SIA_ADDRESS: Address = Address::from_str("c34caa97740668de2bbdb7174572ed64c861342bf27e80313cbfa02e9251f52e30aad3892533").unwrap();

    /// A Sia Address that is not Alice's or Bob's
    pub static ref CHARLIE_SIA_ADDRESS: Address = Address::from_str("465f2b9e9e3bae4903c5b449ea896087b4a9f19b5063bcbbc8e0340772d1dc5afa323bdc2faa").unwrap();

}

/// Response from `get_directly_connected_peers` RPC endpoint.
/// eg, {"<PeerId>": ["<Multiaddr>", "<Multiaddr>"], "<PeerId>": ["<Multiaddr>"]}}
/// TODO: Should technically be HashMap<Peerid, Vec<Multiaddr>> but not needed for current use cases.
#[derive(Debug, Serialize, Deserialize)]
pub struct GetDirectlyConnectedPeersResponse(pub HashMap<String, Vec<String>>);

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

Use unique values for `rpc_port`` and `netid`` for each test if they are intended to run concurrently
alongside other unrelated tests.

All configurations other than rpc_port and netid are hardcoded for simplicity.
**/
pub async fn init_alice(kdf_dir: &PathBuf, rpc_port: u16, netid: u16) -> (MmArc, MarketMakerIt) {
    let alice_interface = (IpAddr::from([127, 0, 0, 1]), rpc_port);
    let alice_db_dir = kdf_dir.join("DB_alice");
    let test_case_string = kdf_dir.to_str().unwrap().to_string();
    let datetime = "init_alice".to_string();

    let alice_conf = json!({
        "gui": format!("{}_alice", test_case_string),
        "netid": netid,
        "passphrase": "buyer buyer buyer buyer buyer buyer buyer buyer buyer buyer buyer cabin",
        "coins": *COINS,
        "myipaddr": "127.0.0.1",
        "rpc_password": "password",
        "rpcport": rpc_port,
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
    tokio::spawn(async move { lp_init(ctx, test_case_string, datetime).await });

    let mm_alice = MarketMakerIt {
        folder: alice_db_dir,
        ip: alice_interface.0,
        rpc_port: Some(alice_interface.1),
        log_path: kdf_dir.join(LOG_FILENAME),
        pc: None,
        userpass: "password".to_string(),
    };
    wait_for_rpc_started(ctx_clone.clone(), Duration::from_secs(20))
        .await
        .unwrap();

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

Use unique values for `rpc_port`` and `netid`` for each test if they are intended to run concurrently
alongside other unrelated tests.

All configurations other than rpc_port and netid are hardcoded for simplicity.
**/
pub async fn init_bob(kdf_dir: &PathBuf, rpc_port: u16, netid: u16) -> (MmArc, MarketMakerIt) {
    let bob_interface = (IpAddr::from([127, 0, 0, 1]), rpc_port);
    let bob_db_dir = kdf_dir.join("DB_bob");
    let test_case_string = kdf_dir.to_str().unwrap().to_string();
    let datetime = "init_bob".to_string();

    let bob_conf = json!({
        "gui": format!("{}_bob", test_case_string),
        "netid": netid,
        "passphrase": "sell sell sell sell sell sell sell sell sell sell sell sell",
        "coins": *COINS,
        "myipaddr": bob_interface.0.to_string(),
        "rpc_password": "password",
        "rpcport": bob_interface.1,
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
    tokio::spawn(async move { lp_init(ctx, test_case_string, datetime).await });

    let mm_bob = MarketMakerIt {
        folder: bob_db_dir,
        ip: bob_interface.0,
        rpc_port: Some(bob_interface.1),
        log_path: kdf_dir.join(LOG_FILENAME),
        pc: None,
        userpass: "password".to_string(),
    };

    wait_for_rpc_started(ctx_clone.clone(), Duration::from_secs(20))
        .await
        .unwrap();

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
/// Note: These containers are never cleaned up as these tests are run on temporary VMs.
pub fn init_walletd_container(docker: &Cli) -> (Container<GenericImage>, u16) {
    // Define the Docker image with a tag
    let image = GenericImage::new("docker.io/alrighttt/walletd-komodo", "latest").with_exposed_port(9980);

    // Wrap the image in `RunnableImage` to allow custom port mapping to an available host port
    // 0 indicates that the host port will be automatically assigned to an available port
    let runnable_image = RunnableImage::from(image).with_mapped_port((0, 9980));

    // Start the container. It will run until `Container` falls out of scope
    let container = docker.run(runnable_image);

    // Retrieve the host port that is mapped to the container's 9980 port
    let host_port = container.get_host_port_ipv4(9980);

    (container, host_port)
}

// Wait for `ctx.rpc_started.is_some()` or timeout
pub async fn wait_for_rpc_started(ctx: MmArc, timeout_duration: Duration) -> Result<(), ()> {
    let start_time = tokio::time::Instant::now();
    loop {
        {
            if ctx.rpc_started.is_some() {
                return Ok(());
            }
        }

        // Check if we've reached the timeout
        if start_time.elapsed() >= timeout_duration {
            return Err(()); // Timed out
        }

        // Yield to avoid busy-waiting
        yield_now().await;
    }
}

/**
Mine `n` blocks to the given Sia Address, `addr`.
This is intended for use in tests that utilize `init_walletd_container`.
Does not wait for the blocks to be mined. Returns immediately after receiving a response from the walletd node.
This endpoint is only available on Walletd nodes that have been started with `-debug`.
**/
pub async fn mine_sia_blocks(client: &SiaClient, n: i64, addr: &Address) -> Result<(), SiaClientError> {
    client
        .dispatcher(DebugMineRequest {
            address: addr.clone(),
            blocks: n,
        })
        .await?;
    Ok(())
}
