use super::*;
use mm2_test_helpers::for_tests::{enable_lightning, sign_message, verify_message};

const T_BTC_ELECTRUMS: &[&str] = &[
    "electrum1.cipig.net:10068",
    "electrum2.cipig.net:10068",
    "electrum3.cipig.net:10068",
];

fn start_lightning_nodes(enable_0_confs: bool) -> (MarketMakerIt, MarketMakerIt, String, String) {
    let node_1_seed = "become nominee mountain person volume business diet zone govern voice debris hidden";
    let node_2_seed = "february coast tortoise grab shadow vast volcano affair ordinary gesture brass oxygen";

    let coins = json! ([
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
              "inbound_channels_confirmations": 1
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
        json! ({
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
        local_start!("bob"),
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm_node_1.mm_dump();
    log!("Node 1 log path: {}", mm_node_1.log_path.display());

    let electrum = block_on(enable_electrum(&mm_node_1, "tBTC-TEST-segwit", false, T_BTC_ELECTRUMS));
    log!("Node 1 tBTC address: {}", electrum.address);

    let enable_lightning_1 = block_on(enable_lightning(&mm_node_1, "tBTC-TEST-lightning"));
    let node_1_address = enable_lightning_1["result"]["address"].as_str().unwrap().to_string();

    let mm_node_2 = MarketMakerIt::start(
        json! ({
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
        local_start!("alice"),
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm_node_2.mm_dump();
    log!("Node 2 log path: {}", mm_node_2.log_path.display());

    let electrum = block_on(enable_electrum(&mm_node_2, "tBTC-TEST-segwit", false, T_BTC_ELECTRUMS));
    log!("Node 2 tBTC address: {}", electrum.address);

    let enable_lightning_2 = block_on(enable_lightning(&mm_node_2, "tBTC-TEST-lightning"));
    let node_2_address = enable_lightning_2["result"]["address"].as_str().unwrap().to_string();

    (mm_node_1, mm_node_2, node_1_address, node_2_address)
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_enable_lightning() {
    let seed = "valley embody about obey never adapt gesture trust screen tube glide bread";

    let coins = json! ([
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
        json! ({
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
        local_start!("bob"),
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm.mm_dump();
    log!("log path: {}", mm.log_path.display());

    let _electrum = block_on(enable_electrum(&mm, "tBTC-TEST-segwit", false, T_BTC_ELECTRUMS));

    let enable_lightning = block_on(enable_lightning(&mm, "tBTC-TEST-lightning"));
    assert_eq!(enable_lightning["result"]["platform_coin"], "tBTC-TEST-segwit");
    assert_eq!(
        enable_lightning["result"]["address"],
        "02ce55b18d617bf4ac27b0f045301a0bb4e71669ae45cb5f2529f2f217520ffca1"
    );
    assert_eq!(enable_lightning["result"]["balance"]["spendable"], "0");
    assert_eq!(enable_lightning["result"]["balance"]["unspendable"], "0");

    block_on(mm.stop()).unwrap();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_connect_to_node() {
    let (mm_node_1, mm_node_2, node_1_id, _) = start_lightning_nodes(false);
    let node_1_address = format!("{}@{}:9735", node_1_id, mm_node_1.ip.to_string());

    let connect = block_on(mm_node_2.rpc(&json! ({
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

    let open_channel = block_on(mm_node_2.rpc(&json! ({
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

    let list_channels_node_1 = block_on(mm_node_1.rpc(&json! ({
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

    let list_channels_node_2 = block_on(mm_node_2.rpc(&json! ({
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
    let (mut mm_node_2, mm_node_1, node_2_id, node_1_id) = start_lightning_nodes(true);
    let node_1_address = format!("{}@{}:9735", node_1_id, mm_node_1.ip.to_string());

    let add_trusted_node = block_on(mm_node_1.rpc(&json! ({
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

    let open_channel = block_on(mm_node_2.rpc(&json! ({
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

    let send_payment = block_on(mm_node_2.rpc(&json! ({
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

    block_on(mm_node_2.wait_for_log(60., |log| log.contains("Successfully sent payment"))).unwrap();

    // Check payment on the sending node side
    let get_payment_details = block_on(mm_node_2.rpc(&json! ({
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
    let get_payment_details = block_on(mm_node_1.rpc(&json! ({
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

    block_on(mm_node_1.stop()).unwrap();
    block_on(mm_node_2.stop()).unwrap();
}

#[test]
// This test is ignored because it requires refilling the tBTC and RICK addresses with test coins periodically.
#[ignore]
#[cfg(not(target_arch = "wasm32"))]
fn test_lightning_taker_swap() {
    let (mut mm_node_1, mut mm_node_2, node_1_id, node_2_id) = start_lightning_nodes(true);
    let node_1_address = format!("{}@{}:9735", node_1_id, mm_node_1.ip.to_string());

    let add_trusted_node = block_on(mm_node_1.rpc(&json! ({
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

    let open_channel = block_on(mm_node_2.rpc(&json! ({
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

    // mm_node_1 is maker
    let set_price = block_on(mm_node_1.rpc(&json! ({
        "userpass": mm_node_1.userpass,
        "method": "setprice",
        "base": "RICK",
        "rel": "tBTC-TEST-lightning",
        "price": 0.000001,
        "volume": 0.1
    })))
    .unwrap();
    assert!(set_price.0.is_success(), "!setprice: {}", set_price.1);

    let orderbook = block_on(mm_node_2.rpc(&json! ({
        "userpass": mm_node_2.userpass,
        "method": "orderbook",
        "base": "RICK",
        "rel": "tBTC-TEST-lightning",
    })))
    .unwrap();
    assert!(orderbook.0.is_success(), "!orderbook: {}", orderbook.1);

    block_on(Timer::sleep(1.));

    // mm_node_2 is taker
    let buy = block_on(mm_node_2.rpc(&json! ({
        "userpass": mm_node_2.userpass,
        "method": "buy",
        "base": "RICK",
        "rel": "tBTC-TEST-lightning",
        "price": 0.000001,
        "volume": 0.1
    })))
    .unwrap();
    assert!(buy.0.is_success(), "!buy: {}", buy.1);
    let buy_json: Json = serde_json::from_str(&buy.1).unwrap();
    let uuid = buy_json["result"]["uuid"].as_str().unwrap().to_owned();

    // ensure the swaps are started
    block_on(mm_node_2.wait_for_log(5., |log| {
        log.contains("Entering the taker_swap_loop RICK/tBTC-TEST-lightning")
    }))
    .unwrap();
    block_on(mm_node_1.wait_for_log(5., |log| {
        log.contains("Entering the maker_swap_loop RICK/tBTC-TEST-lightning")
    }))
    .unwrap();

    block_on(mm_node_1.wait_for_log(900., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))).unwrap();

    block_on(mm_node_2.wait_for_log(900., |log| log.contains(&format!("[swap uuid={}] Finished", uuid)))).unwrap();

    block_on(mm_node_1.stop()).unwrap();
    block_on(mm_node_2.stop()).unwrap();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_sign_verify_message_lightning() {
    let seed = "spice describe gravity federal blast come thank unfair canal monkey style afraid";

    let coins = json! ([
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
        json! ({
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
        local_start!("bob"),
    )
    .unwrap();
    let (_dump_log, _dump_dashboard) = mm.mm_dump();
    log!("log path: {}", mm.log_path.display());

    block_on(enable_electrum(&mm, "tBTC-TEST-segwit", false, T_BTC_ELECTRUMS));
    block_on(enable_lightning(&mm, "tBTC-TEST-lightning"));

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
