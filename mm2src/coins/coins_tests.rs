use crate::update_coins_config;

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
