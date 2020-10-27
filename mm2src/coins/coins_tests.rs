use crate::update_coins_config;
use crate::utxo::rpc_clients::NativeClientImpl;
use base64::{encode_config as base64_encode, URL_SAFE};
use futures::lock::Mutex as AsyncMutex;
use futures01::Future;
use std::collections::HashMap;

pub fn test_list_unspent() {
    let client = NativeClientImpl {
        coin_ticker: "RICK".into(),
        uri: "http://127.0.0.1:10271".to_owned(),
        auth: fomat!("Basic "(base64_encode(
            "user481805103:pass97a61c8d048bcf468c6c39a314970e557f57afd1d8a5edee917fb29bafb3a43371",
            URL_SAFE
        ))),
        event_handlers: Default::default(),
        request_id: 0u64.into(),
        recently_sent_txs: AsyncMutex::new(HashMap::new()),
    };
    let unspents = client.list_unspent(0, std::i32::MAX, vec!["RBs52D7pVq7txo6SCz1Tuyw2WrPmdqU3qw".to_owned()]);
    let unspents = unwrap!(unspents.wait());
    log!("Unspents "[unspents]);
}

pub fn test_get_block_count() {
    let client = NativeClientImpl {
        coin_ticker: "RICK".into(),
        uri: "http://127.0.0.1:10271".to_owned(),
        auth: fomat!("Basic "(base64_encode(
            "user481805103:pass97a61c8d048bcf468c6c39a314970e557f57afd1d8a5edee917fb29bafb3a43371",
            URL_SAFE
        ))),
        event_handlers: Default::default(),
        request_id: 0u64.into(),
        recently_sent_txs: AsyncMutex::new(HashMap::new()),
    };
    let block_count = unwrap!(client
        .validate_address("RBs52D7pVq7txo6SCz1Tuyw2WrPmdqU3qw".to_owned())
        .wait());
    log!("Block count "[block_count]);
}

pub fn test_import_address() {
    let client = NativeClientImpl {
        coin_ticker: "RICK".into(),
        uri: "http://127.0.0.1:10271".to_owned(),
        auth: fomat!("Basic "(base64_encode(
            "user481805103:pass97a61c8d048bcf468c6c39a314970e557f57afd1d8a5edee917fb29bafb3a43371",
            URL_SAFE
        ))),
        event_handlers: Default::default(),
        request_id: 0u64.into(),
        recently_sent_txs: AsyncMutex::new(HashMap::new()),
    };
    let import_addr = client.import_address(
        "bMjWGCinft5qEvsuf9Wg1fgz1CjpXBXbTB",
        "bMjWGCinft5qEvsuf9Wg1fgz1CjpXBXbTB",
        true,
    );
    import_addr.wait().unwrap();
}

#[test]
fn test_update_coin_config_success() {
    let conf = json!([
        {
            "coin": "RICK",
            "asset": "RICK",
            "fname": "RICK (TESTCOIN)",
            "rpcport": 25435,
            "txversion": 4,
            "overwintered": 1,
            "mm2": 1,
        },
        {
            "coin": "MORTY",
            "asset": "MORTY",
            "fname": "MORTY (TESTCOIN)",
            "rpcport": 16348,
            "txversion": 4,
            "overwintered": 1,
            "mm2": 1,
        },
        {
            "coin": "ETH",
            "name": "ethereum",
            "fname": "Ethereum",
            "etomic": "0x0000000000000000000000000000000000000000",
            "rpcport": 80,
            "mm2": 1,
            "required_confirmations": 3,
        },
        {
            "coin": "ARPA",
            "name": "arpa-chain",
            "fname": "ARPA Chain",
            // ARPA coin contains the protocol already. This coin should be skipped.
            "protocol": {
                "type":"ERC20",
                "protocol_data": {
                    "platform": "ETH",
                    "contract_address": "0xBA50933C268F567BDC86E1aC131BE072C6B0b71a"
                }
            },
            "rpcport": 80,
            "mm2": 1,
            "required_confirmations": 3,
        },
        {
            "coin": "JST",
            "name": "JST",
            "fname": "JST (TESTCOIN)",
            "etomic": "0x996a8ae0304680f6a69b8a9d7c6e37d65ab5ab56",
            "rpcport": 80,
            "mm2": 1,
        },
    ]);
    let actual = update_coins_config(conf).unwrap();
    let expected = json!([
        {
            "coin": "RICK",
            "asset": "RICK",
            "fname": "RICK (TESTCOIN)",
            "rpcport": 25435,
            "txversion": 4,
            "overwintered": 1,
            "mm2": 1,
            "protocol": {
                "type": "UTXO"
            },
        },
        {
            "coin": "MORTY",
            "asset": "MORTY",
            "fname": "MORTY (TESTCOIN)",
            "rpcport": 16348,
            "txversion": 4,
            "overwintered": 1,
            "mm2": 1,
            "protocol": {
                "type": "UTXO"
            },
        },
        {
            "coin": "ETH",
            "name": "ethereum",
            "fname": "Ethereum",
            "rpcport": 80,
            "mm2": 1,
            "required_confirmations": 3,
            "protocol": {
                "type": "ETH"
            },
        },
        {
            "coin": "ARPA",
            "name": "arpa-chain",
            "fname": "ARPA Chain",
            "protocol": {
                "type": "ERC20",
                "protocol_data": {
                    "platform": "ETH",
                    "contract_address": "0xBA50933C268F567BDC86E1aC131BE072C6B0b71a"
                }
            },
            "rpcport": 80,
            "mm2": 1,
            "required_confirmations": 3,
        },
        {
            "coin": "JST",
            "name": "JST",
            "fname": "JST (TESTCOIN)",
            "rpcport": 80,
            "mm2": 1,
            "protocol": {
                "type": "ERC20",
                "protocol_data": {
                    "platform": "ETH",
                    "contract_address": "0x996a8ae0304680f6a69b8a9d7c6e37d65ab5ab56"
                }
            },
        },
    ]);
    assert_eq!(actual, expected);
}

#[test]
fn test_update_coin_config_error_not_array() {
    let conf = json!({
        "coin": "RICK",
        "asset": "RICK",
        "fname": "RICK (TESTCOIN)",
        "rpcport": 25435,
        "txversion": 4,
        "overwintered": 1,
        "mm2": 1,
    });
    let error = update_coins_config(conf).err().unwrap();
    assert!(error.contains("Coins config must be an array"));
}

#[test]
fn test_update_coin_config_error_not_object() {
    let conf = json!([["Ford", "BMW", "Fiat"]]);
    let error = update_coins_config(conf).err().unwrap();
    assert!(error.contains("Expected object, found"));
}

#[test]
fn test_update_coin_config_invalid_etomic() {
    let conf = json!([
        {
            "coin": "JST",
            "name": "JST",
            "fname": "JST (TESTCOIN)",
            "etomic": 12345678,
            "rpcport": 80,
            "mm2": 1,
        },
    ]);
    let error = update_coins_config(conf).err().unwrap();
    assert!(error.contains("Expected etomic as string, found"));
}
