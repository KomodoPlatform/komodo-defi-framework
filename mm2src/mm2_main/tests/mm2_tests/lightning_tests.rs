use crate::integration_tests_common::{enable_coins_rick_morty_electrum, enable_electrum};
use coins::lightning::ln_events::{SUCCESSFUL_CLAIM_LOG, SUCCESSFUL_SEND_LOG};
use common::executor::Timer;
use common::{block_on, log};
use gstuff::now_ms;
use http::StatusCode;
use mm2_number::BigDecimal;
use mm2_test_helpers::for_tests::{init_lightning, init_lightning_status, my_balance, sign_message, start_swaps,
                                  verify_message, MarketMakerIt};
use mm2_test_helpers::structs::{InitLightningStatus, InitTaskResult, LightningActivationResult, MyBalanceResponse,
                                RpcV2Response, SignatureResponse, VerificationResponse};
use serde_json::{self as json, json, Value as Json};
use std::env;
use std::str::FromStr;

const T_BTC_ELECTRUMS: &[&str] = &[
    "electrum1.cipig.net:10068",
    "electrum2.cipig.net:10068",
    "electrum3.cipig.net:10068",
];

async fn enable_lightning(mm: &MarketMakerIt, coin: &str, timeout: u64) -> LightningActivationResult {
    let init = init_lightning(mm, coin).await;
    let init: RpcV2Response<InitTaskResult> = json::from_value(init).unwrap();
    let timeout = now_ms() + (timeout * 1000);

    loop {
        if now_ms() > timeout {
            panic!("{} initialization timed out", coin);
        }

        let status = init_lightning_status(mm, init.result.task_id).await;
        let status: RpcV2Response<InitLightningStatus> = json::from_value(status).unwrap();
        log!("init_lightning_status: {:?}", status);
        match status.result {
            InitLightningStatus::Ok(result) => break result,
            InitLightningStatus::Error(e) => panic!("{} initialization error {:?}", coin, e),
            _ => Timer::sleep(1.).await,
        }
    }
}

fn start_lightning_nodes(enable_0_confs: bool) -> (MarketMakerIt, MarketMakerIt, String, String) {
    let node_1_seed = "become nominee mountain person volume business diet zone govern voice debris hidden";
    let node_2_seed = "february coast tortoise grab shadow vast volcano affair ordinary gesture brass oxygen";

    let coins = json!([
        {
            "coin": "tBTC-TEST-segwit",
            "name": "tbitcoin",
            "fname": "tBitcoin",
            "rpcport": 18332,
            "pubtype": 111,
            "p2shtype": 196,
            "wiftype": 239,
            "segwit": true,
            "bech32_hrp": "tb",
            "address_format":{"format":"segwit"},
            "orderbook_ticker": "tBTC-TEST",
            "txfee": 0,
            "estimate_fee_mode": "ECONOMICAL",
            "mm2": 1,
            "required_confirmations": 0,
            "protocol": {
              "type": "UTXO"
            }
          },
          {
            "coin": "tBTC-TEST-lightning",
            "mm2": 1,
            "decimals": 11,
            "our_channels_configs": {
              "inbound_channels_confirmations": 1,
              "max_inbound_in_flight_htlc_percent": 100
            },
            "counterparty_channel_config_limits": {
              "outbound_channels_confirmations": 1,
              // If true, this enables sending payments between the 2 nodes straight away without waiting for on-chain confirmations
              // if the other node added this node as trusted. It also overrides "outbound_channels_confirmations".
              "allow_outbound_0conf": enable_0_confs
            },
            "protocol": {
              "type": "LIGHTNING",
              "protocol_data":{
                "platform": "tBTC-TEST-segwit",
                "network": "testnet",
                "avg_block_time": 600,
                "confirmation_targets": {
                  "background": 12,
                  "normal": 6,
                  "high_priority": 1
                }
              }
            }
          },
        {"coin":"RICK","asset":"RICK","rpcport":8923,"txversion":4,"overwintered":1,"required_confirmations":0,"protocol":{"type":"UTXO"}},
        {"coin":"MORTY","asset":"MORTY","rpcport":11608,"txversion":4,"overwintered":1,"required_confirmations":0,"protocol":{"type":"UTXO"}}
    ]);

    let mm_node_1 = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": node_1_seed.to_string(),
            "coins": coins,
            "rpc_password": "pass",
            "i_am_seed": true,
        }),
        "pass".into(),
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm_node_1.mm_dump();
    log!("Node 1 log path: {}", mm_node_1.log_path.display());

    let electrum = block_on(enable_electrum(&mm_node_1, "tBTC-TEST-segwit", false, T_BTC_ELECTRUMS));
    log!("Node 1 tBTC address: {}", electrum.address);

    let enable_lightning_1 = block_on(enable_lightning(&mm_node_1, "tBTC-TEST-lightning", 600));
    let node_1_address = enable_lightning_1.address;

    let mm_node_2 = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("ALICE_TRADE_IP") .ok(),
            "rpcip": env::var ("ALICE_TRADE_IP") .ok(),
            "passphrase": node_2_seed.to_string(),
            "coins": coins,
            "rpc_password": "pass",
            "seednodes": [mm_node_1.my_seed_addr()],
        }),
        "pass".into(),
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm_node_2.mm_dump();
    log!("Node 2 log path: {}", mm_node_2.log_path.display());

    let electrum = block_on(enable_electrum(&mm_node_2, "tBTC-TEST-segwit", false, T_BTC_ELECTRUMS));
    log!("Node 2 tBTC address: {}", electrum.address);

    let enable_lightning_2 = block_on(enable_lightning(&mm_node_2, "tBTC-TEST-lightning", 600));
    let node_2_address = enable_lightning_2.address;

    (mm_node_1, mm_node_2, node_1_address, node_2_address)
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_enable_lightning() {
    let seed = "valley embody about obey never adapt gesture trust screen tube glide bread";

    let coins = json!([
        {
            "coin": "tBTC-TEST-segwit",
            "name": "tbitcoin",
            "fname": "tBitcoin",
            "rpcport": 18332,
            "pubtype": 111,
            "p2shtype": 196,
            "wiftype": 239,
            "segwit": true,
            "bech32_hrp": "tb",
            "address_format":{"format":"segwit"},
            "orderbook_ticker": "tBTC-TEST",
            "txfee": 0,
            "estimate_fee_mode": "ECONOMICAL",
            "mm2": 1,
            "required_confirmations": 0,
            "protocol": {
              "type": "UTXO"
            }
          },
          {
            "coin": "tBTC-TEST-lightning",
            "mm2": 1,
            "decimals": 11,
            "protocol": {
              "type": "LIGHTNING",
              "protocol_data":{
                "platform": "tBTC-TEST-segwit",
                "network": "testnet",
                "avg_block_time": 600,
                "confirmation_targets": {
                  "background": 12,
                  "normal": 6,
                  "high_priority": 1
                }
              }
            }
          }
    ]);

    let mm = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": seed.to_string(),
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm.mm_dump();
    log!("log path: {}", mm.log_path.display());

    let _electrum = block_on(enable_electrum(&mm, "tBTC-TEST-segwit", false, T_BTC_ELECTRUMS));

    let enable_lightning_coin = block_on(enable_lightning(&mm, "tBTC-TEST-lightning", 600));
    assert_eq!(&enable_lightning_coin.platform_coin, "tBTC-TEST-segwit");
    assert_eq!(
        &enable_lightning_coin.address,
        "02ce55b18d617bf4ac27b0f045301a0bb4e71669ae45cb5f2529f2f217520ffca1"
    );
    assert_eq!(enable_lightning_coin.balance.spendable, BigDecimal::from(0));
    assert_eq!(enable_lightning_coin.balance.unspendable, BigDecimal::from(0));

    // Disable tBTC-TEST-lightning
    let disabled = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "disable_coin",
        "coin": "tBTC-TEST-lightning",
    })))
    .unwrap();
    assert_eq!(disabled.0, StatusCode::OK);

    // Enable tBTC-TEST-lightning
    let enable_lightning_coin = block_on(enable_lightning(&mm, "tBTC-TEST-lightning", 600));
    assert_eq!(&enable_lightning_coin.platform_coin, "tBTC-TEST-segwit");
    assert_eq!(
        &enable_lightning_coin.address,
        "02ce55b18d617bf4ac27b0f045301a0bb4e71669ae45cb5f2529f2f217520ffca1"
    );

    // Disable tBTC-TEST-segwit
    let disabled = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "disable_coin",
        "coin": "tBTC-TEST-segwit",
    })))
    .unwrap();
    assert_eq!(disabled.0, StatusCode::OK);

    // Restart unit test to cover disabling platform coin with it's tokens for LightningCoin
    let _electrum = block_on(enable_electrum(&mm, "tBTC-TEST-segwit", false, T_BTC_ELECTRUMS));
    let enable_lightning = block_on(enable_lightning(&mm, "tBTC-TEST-lightning", 600));
    assert_eq!(&enable_lightning.platform_coin, "tBTC-TEST-segwit");

    // We try to disable tBTC-TEST-segwit
    let disabled = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "disable_coin",
        "coin": "tBTC-TEST-segwit",
    })))
    .unwrap();
    assert_eq!(disabled.0, StatusCode::OK);

    // Confirm that tBTC-TEST-lightning is also disabled!.
    let response = block_on(mm.rpc(&json! ({
        "userpass": mm.userpass,
        "method": "my_balance",
        "coin": "tBTC-TEST-lightning",
    })))
    .unwrap();
    assert_eq!(response.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.1.contains("No such coin: tBTC-TEST-lightning"));

    // Stop mm2
    block_on(mm.stop()).unwrap();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_connect_to_node() {
    let (mm_node_1, mm_node_2, node_1_id, _) = start_lightning_nodes(false);
    let node_1_address = format!("{}@{}:9735", node_1_id, mm_node_1.ip.to_string());

    let connect = block_on(mm_node_2.rpc(&json!({
        "userpass": mm_node_2.userpass,
        "mmrpc": "2.0",
        "method": "lightning::nodes::connect_to_node",
        "params": {
            "coin": "tBTC-TEST-lightning",
            "node_address": node_1_address,
        },
    })))
    .unwrap();
    assert!(
        connect.0.is_success(),
        "!lightning::nodes::connect_to_node: {}",
        connect.1
    );
    let connect_res: Json = json::from_str(&connect.1).unwrap();
    let expected = format!("Connected successfully to node : {}", node_1_address);
    assert_eq!(connect_res["result"], expected);

    block_on(mm_node_1.stop()).unwrap();
    block_on(mm_node_2.stop()).unwrap();
}

#[test]
// This test is ignored because it requires refilling the tBTC addresses with test coins periodically.
#[ignore]
#[cfg(not(target_arch = "wasm32"))]
fn test_open_channel() {
    let (mm_node_1, mut mm_node_2, node_1_id, node_2_id) = start_lightning_nodes(false);
    let node_1_address = format!("{}@{}:9735", node_1_id, mm_node_1.ip.to_string());

    let open_channel = block_on(mm_node_2.rpc(&json!({
        "userpass": mm_node_2.userpass,
        "mmrpc": "2.0",
        "method": "lightning::channels::open_channel",
        "params": {
            "coin": "tBTC-TEST-lightning",
            "node_address": node_1_address,
            "amount": {
                "type": "Exact",
                "value": 0.0002,
            },
        },
    })))
    .unwrap();
    assert!(
        open_channel.0.is_success(),
        "!lightning::channels::open_channel: {}",
        open_channel.1
    );

    block_on(mm_node_2.wait_for_log(60., |log| log.contains("Transaction broadcasted successfully"))).unwrap();

    let list_channels_node_1 = block_on(mm_node_1.rpc(&json!({
        "userpass": mm_node_1.userpass,
        "mmrpc": "2.0",
        "method": "lightning::channels::list_open_channels_by_filter",
        "params": {
            "coin": "tBTC-TEST-lightning",
        },
    })))
    .unwrap();
    assert!(
        list_channels_node_1.0.is_success(),
        "!lightning::channels::list_open_channels_by_filter: {}",
        list_channels_node_1.1
    );
    let list_channels_node_1_res: Json = json::from_str(&list_channels_node_1.1).unwrap();
    log!("list_channels_node_1_res {:?}", list_channels_node_1_res);
    assert_eq!(
        list_channels_node_1_res["result"]["open_channels"][0]["counterparty_node_id"],
        node_2_id
    );
    assert_eq!(
        list_channels_node_1_res["result"]["open_channels"][0]["is_outbound"],
        false
    );
    assert_eq!(
        list_channels_node_1_res["result"]["open_channels"][0]["balance_msat"],
        0
    );

    let list_channels_node_2 = block_on(mm_node_2.rpc(&json!({
      "userpass": mm_node_2.userpass,
      "mmrpc": "2.0",
      "method": "lightning::channels::list_open_channels_by_filter",
      "params": {
          "coin": "tBTC-TEST-lightning",
      },
    })))
    .unwrap();
    assert!(
        list_channels_node_2.0.is_success(),
        "!lightning::channels::list_open_channels_by_filter: {}",
        list_channels_node_2.1
    );
    let list_channels_node_2_res: Json = json::from_str(&list_channels_node_2.1).unwrap();
    assert_eq!(
        list_channels_node_2_res["result"]["open_channels"][0]["counterparty_node_id"],
        node_1_id
    );
    assert_eq!(
        list_channels_node_2_res["result"]["open_channels"][0]["is_outbound"],
        true
    );
    assert_eq!(
        list_channels_node_2_res["result"]["open_channels"][0]["balance_msat"],
        20000000
    );

    block_on(mm_node_1.stop()).unwrap();
    block_on(mm_node_2.stop()).unwrap();
}

#[test]
// This test is ignored because it requires refilling the tBTC addresses with test coins periodically.
#[ignore]
#[cfg(not(target_arch = "wasm32"))]
// This also tests 0_confs_channels
fn test_send_payment() {
    let (mut mm_node_2, mut mm_node_1, node_2_id, node_1_id) = start_lightning_nodes(true);
    let node_1_address = format!("{}@{}:9735", node_1_id, mm_node_1.ip.to_string());

    let add_trusted_node = block_on(mm_node_1.rpc(&json!({
        "userpass": mm_node_1.userpass,
        "mmrpc": "2.0",
        "method": "lightning::nodes::add_trusted_node",
        "params": {
            "coin": "tBTC-TEST-lightning",
            "node_id": node_2_id
        },
    })))
    .unwrap();
    assert!(
        add_trusted_node.0.is_success(),
        "!lightning::nodes::add_trusted_node: {}",
        add_trusted_node.1
    );

    let open_channel = block_on(mm_node_2.rpc(&json!({
        "userpass": mm_node_2.userpass,
        "mmrpc": "2.0",
        "method": "lightning::channels::open_channel",
        "params": {
            "coin": "tBTC-TEST-lightning",
            "node_address": node_1_address,
            "amount": {
                "type": "Exact",
                "value": 0.0002,
            },
        },
    })))
    .unwrap();
    assert!(
        open_channel.0.is_success(),
        "!lightning::channels::open_channel: {}",
        open_channel.1
    );

    block_on(mm_node_2.wait_for_log(60., |log| log.contains("Received message ChannelReady"))).unwrap();

    let send_payment = block_on(mm_node_2.rpc(&json!({
        "userpass": mm_node_2.userpass,
        "mmrpc": "2.0",
        "method": "lightning::payments::send_payment",
        "params": {
            "coin": "tBTC-TEST-lightning",
            "payment": {
                "type": "keysend",
                "destination": node_1_id,
                "amount_in_msat": 1000,
                "expiry": 24
            }
        },
    })))
    .unwrap();
    assert!(
        send_payment.0.is_success(),
        "!lightning::payments::send_payment: {}",
        send_payment.1
    );

    let send_payment_res: Json = json::from_str(&send_payment.1).unwrap();
    log!("send_payment_res {:?}", send_payment_res);
    let payment_hash = send_payment_res["result"]["payment_hash"].as_str().unwrap();

    block_on(mm_node_2.wait_for_log(60., |log| log.contains(SUCCESSFUL_SEND_LOG))).unwrap();

    // Check payment on the sending node side
    let get_payment_details = block_on(mm_node_2.rpc(&json!({
      "userpass": mm_node_2.userpass,
      "mmrpc": "2.0",
      "method": "lightning::payments::get_payment_details",
      "params": {
          "coin": "tBTC-TEST-lightning",
          "payment_hash": payment_hash
      },
    })))
    .unwrap();
    assert!(
        get_payment_details.0.is_success(),
        "!lightning::payments::get_payment_details: {}",
        get_payment_details.1
    );

    let get_payment_details_res: Json = json::from_str(&get_payment_details.1).unwrap();
    let payment = &get_payment_details_res["result"]["payment_details"];
    assert_eq!(payment["status"], "succeeded");
    assert_eq!(payment["amount_in_msat"], 1000);
    assert_eq!(payment["payment_type"]["type"], "Outbound Payment");

    // Check payment on the receiving node side
    let get_payment_details = block_on(mm_node_1.rpc(&json!({
      "userpass": mm_node_1.userpass,
      "mmrpc": "2.0",
      "method": "lightning::payments::get_payment_details",
      "params": {
          "coin": "tBTC-TEST-lightning",
          "payment_hash": payment_hash
      },
    })))
    .unwrap();
    assert!(
        get_payment_details.0.is_success(),
        "!lightning::payments::get_payment_details: {}",
        get_payment_details.1
    );

    let get_payment_details_res: Json = json::from_str(&get_payment_details.1).unwrap();
    let payment = &get_payment_details_res["result"]["payment_details"];
    assert_eq!(payment["status"], "succeeded");
    assert_eq!(payment["amount_in_msat"], 1000);
    assert_eq!(payment["payment_type"]["type"], "Inbound Payment");

    // Test generate and pay invoice
    let generate_invoice = block_on(mm_node_1.rpc(&json!({
        "userpass": mm_node_1.userpass,
        "mmrpc": "2.0",
        "method": "lightning::payments::generate_invoice",
        "params": {
            "coin": "tBTC-TEST-lightning",
            "description": "test invoice",
            "amount_in_msat": 10000
        },
    })))
    .unwrap();
    assert!(
        generate_invoice.0.is_success(),
        "!lightning::payments::generate_invoice: {}",
        generate_invoice.1
    );

    let generate_invoice_res: Json = json::from_str(&generate_invoice.1).unwrap();
    log!("generate_invoice_res {:?}", generate_invoice_res);
    let invoice = generate_invoice_res["result"]["invoice"].as_str().unwrap();
    let invoice_payment_hash = generate_invoice_res["result"]["payment_hash"].as_str().unwrap();

    let pay_invoice = block_on(mm_node_2.rpc(&json!({
        "userpass": mm_node_2.userpass,
        "mmrpc": "2.0",
        "method": "lightning::payments::send_payment",
        "params": {
            "coin": "tBTC-TEST-lightning",
            "payment": {
                "type": "invoice",
                "invoice": invoice
            }
        },
    })))
    .unwrap();
    assert!(
        pay_invoice.0.is_success(),
        "!lightning::payments::send_payment: {}",
        pay_invoice.1
    );

    let pay_invoice_res: Json = json::from_str(&pay_invoice.1).unwrap();
    log!("pay_invoice_res {:?}", pay_invoice_res);
    let payment_hash = pay_invoice_res["result"]["payment_hash"].as_str().unwrap();

    block_on(mm_node_1.wait_for_log(60., |log| log.contains(SUCCESSFUL_CLAIM_LOG))).unwrap();
    block_on(mm_node_2.wait_for_log(60., |log| {
        log.contains(&format!(
            "{} of 10000 millisatoshis with payment hash {}",
            SUCCESSFUL_SEND_LOG, payment_hash
        ))
    }))
    .unwrap();

    // Check payment on the sending node side
    let get_payment_details = block_on(mm_node_2.rpc(&json!({
      "userpass": mm_node_2.userpass,
      "mmrpc": "2.0",
      "method": "lightning::payments::get_payment_details",
      "params": {
          "coin": "tBTC-TEST-lightning",
          "payment_hash": payment_hash
      },
    })))
    .unwrap();
    assert!(
        get_payment_details.0.is_success(),
        "!lightning::payments::get_payment_details: {}",
        get_payment_details.1
    );

    let get_payment_details_res: Json = json::from_str(&get_payment_details.1).unwrap();
    let payment = &get_payment_details_res["result"]["payment_details"];
    assert_eq!(payment["status"], "succeeded");
    assert_eq!(payment["amount_in_msat"], 10000);
    assert_eq!(payment["payment_type"]["type"], "Outbound Payment");
    assert_eq!(payment["description"], "test invoice");

    // Check payment on the receiving node side
    let get_payment_details = block_on(mm_node_1.rpc(&json!({
      "userpass": mm_node_1.userpass,
      "mmrpc": "2.0",
      "method": "lightning::payments::get_payment_details",
      "params": {
          "coin": "tBTC-TEST-lightning",
          "payment_hash": invoice_payment_hash
      },
    })))
    .unwrap();
    assert!(
        get_payment_details.0.is_success(),
        "!lightning::payments::get_payment_details: {}",
        get_payment_details.1
    );

    let get_payment_details_res: Json = json::from_str(&get_payment_details.1).unwrap();
    let payment = &get_payment_details_res["result"]["payment_details"];
    assert_eq!(payment["status"], "succeeded");
    assert_eq!(payment["amount_in_msat"], 10000);
    assert_eq!(payment["payment_type"]["type"], "Inbound Payment");
    assert_eq!(payment["description"], "test invoice");

    block_on(mm_node_1.stop()).unwrap();
    block_on(mm_node_2.stop()).unwrap();
}

#[test]
// This test is ignored because it requires refilling the tBTC and RICK addresses with test coins periodically.
#[ignore]
#[cfg(not(target_arch = "wasm32"))]
fn test_lightning_swaps() {
    let (mut mm_node_1, mut mm_node_2, node_1_id, node_2_id) = start_lightning_nodes(true);
    let node_1_address = format!("{}@{}:9735", node_1_id, mm_node_1.ip.to_string());

    let add_trusted_node = block_on(mm_node_1.rpc(&json!({
        "userpass": mm_node_1.userpass,
        "mmrpc": "2.0",
        "method": "lightning::nodes::add_trusted_node",
        "params": {
            "coin": "tBTC-TEST-lightning",
            "node_id": node_2_id
        },
    })))
    .unwrap();
    assert!(
        add_trusted_node.0.is_success(),
        "!lightning::nodes::add_trusted_node: {}",
        add_trusted_node.1
    );

    let open_channel = block_on(mm_node_2.rpc(&json!({
        "userpass": mm_node_2.userpass,
        "mmrpc": "2.0",
        "method": "lightning::channels::open_channel",
        "params": {
            "coin": "tBTC-TEST-lightning",
            "node_address": node_1_address,
            "amount": {
                "type": "Exact",
                "value": 0.0002,
            },
        },
    })))
    .unwrap();
    assert!(
        open_channel.0.is_success(),
        "!lightning::channels::open_channel: {}",
        open_channel.1
    );

    block_on(mm_node_2.wait_for_log(60., |log| log.contains("Received message ChannelReady"))).unwrap();

    // Enable coins on mm_node_1 side. Print the replies in case we need the "address".
    log!(
        "enable_coins (mm_node_1): {:?}",
        block_on(enable_coins_rick_morty_electrum(&mm_node_1))
    );

    // Enable coins on mm_node_2 side. Print the replies in case we need the "address".
    log!(
        "enable_coins (mm_node_2): {:?}",
        block_on(enable_coins_rick_morty_electrum(&mm_node_2))
    );

    // -------------------- Test Lightning Taker Swap --------------------
    let uuids = block_on(start_swaps(
        &mut mm_node_1,
        &mut mm_node_2,
        &[("RICK", "tBTC-TEST-lightning")],
        0.0005,
        0.0005,
        0.1,
    ));
    // Todo: use wait_for_swaps_finish_and_check_status instead after fixing lightning swap events
    block_on(mm_node_1.wait_for_log(120., |log| log.contains(&format!("[swap uuid={}] Finished", uuids[0])))).unwrap();
    block_on(mm_node_2.wait_for_log(120., |log| log.contains(&format!("[swap uuid={}] Finished", uuids[0])))).unwrap();

    // Check node 1 lightning balance after swap
    let node_1_lightning_balance = block_on(my_balance(&mm_node_1, "tBTC-TEST-lightning"));
    let node_1_lightning_balance: MyBalanceResponse = serde_json::from_value(node_1_lightning_balance).unwrap();
    // Channel reserve balance, which is non-spendable, is 1000 sats or 0.00001 BTC.
    // Note: A channel reserve balance is the amount that is set aside by each channel participant which ensures neither have 'nothing at stake' if a cheating attempt occurs.
    assert_eq!(
        node_1_lightning_balance.balance,
        BigDecimal::from_str("0.00004").unwrap()
    );

    // -------------------- Test Lightning Maker Swap --------------------
    let uuids = block_on(start_swaps(
        &mut mm_node_1,
        &mut mm_node_2,
        &[("tBTC-TEST-lightning", "RICK")],
        10.,
        10.,
        0.00004,
    ));
    // Todo: use wait_for_swaps_finish_and_check_status instead after fixing lightning swap events
    block_on(mm_node_1.wait_for_log(120., |log| log.contains(&format!("[swap uuid={}] Finished", uuids[0])))).unwrap();
    block_on(mm_node_2.wait_for_log(120., |log| log.contains(&format!("[swap uuid={}] Finished", uuids[0])))).unwrap();

    block_on(mm_node_1.stop()).unwrap();
    block_on(mm_node_2.stop()).unwrap();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_sign_verify_message_lightning() {
    let seed = "spice describe gravity federal blast come thank unfair canal monkey style afraid";

    let coins = json!([
      {
        "coin": "tBTC-TEST-segwit",
        "name": "tbitcoin",
        "fname": "tBitcoin",
        "rpcport": 18332,
        "pubtype": 111,
        "p2shtype": 196,
        "wiftype": 239,
        "segwit": true,
        "bech32_hrp": "tb",
        "address_format":{"format":"segwit"},
        "orderbook_ticker": "tBTC-TEST",
        "txfee": 0,
        "estimate_fee_mode": "ECONOMICAL",
        "mm2": 1,
        "required_confirmations": 0,
        "protocol": {
          "type": "UTXO"
        }
      },
      {
        "coin": "tBTC-TEST-lightning",
        "mm2": 1,
        "decimals": 11,
        "sign_message_prefix": "Lightning Signed Message:",
        "protocol": {
          "type": "LIGHTNING",
          "protocol_data":{
            "platform": "tBTC-TEST-segwit",
            "network": "testnet",
            "avg_block_time": 600,
            "confirmation_targets": {
              "background": 12,
              "normal": 6,
              "high_priority": 1
            }
          }
        }
      }
    ]);

    let mm = MarketMakerIt::start(
        json!({
            "gui": "nogui",
            "netid": 9998,
            "myipaddr": env::var ("BOB_TRADE_IP") .ok(),
            "rpcip": env::var ("BOB_TRADE_IP") .ok(),
            "passphrase": seed.to_string(),
            "coins": coins,
            "i_am_seed": true,
            "rpc_password": "pass",
        }),
        "pass".into(),
        None,
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm.mm_dump();
    log!("log path: {}", mm.log_path.display());

    block_on(enable_electrum(&mm, "tBTC-TEST-segwit", false, T_BTC_ELECTRUMS));
    block_on(enable_lightning(&mm, "tBTC-TEST-lightning", 600));

    let response = block_on(sign_message(&mm, "tBTC-TEST-lightning"));
    let response: RpcV2Response<SignatureResponse> = json::from_value(response).unwrap();
    let response = response.result;

    assert_eq!(
        response.signature,
        "dhmbgykwzy53uycr6u8mpp3us6poikc5qh7wgex5qn54msq7cs3ygebj3h9swaocboqzi89jazwo7i3mmqou15w4dcty666sq3yqhzhr"
    );

    let response = block_on(verify_message(
        &mm,
        "tBTC-TEST-lightning",
        "dhmbgykwzy53uycr6u8mpp3us6poikc5qh7wgex5qn54msq7cs3ygebj3h9swaocboqzi89jazwo7i3mmqou15w4dcty666sq3yqhzhr",
        "0367c7b9f42eb15205de39454ddf9fcfce70a129b01049d9fe1b3b34eac1d6b933",
    ));
    let response: RpcV2Response<VerificationResponse> = json::from_value(response).unwrap();
    let response = response.result;

    assert!(response.is_valid);
}
