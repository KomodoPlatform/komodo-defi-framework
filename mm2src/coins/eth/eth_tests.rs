use super::*;
use common::block_on;
use common::for_tests::wait_for_log;
use common::mm_ctx::{MmArc, MmCtxBuilder};
use futures::future::join_all;
use mocktopus::mocking::*;

fn check_sum(addr: &str, expected: &str) {
    let actual = checksum_address(addr);
    assert_eq!(expected, actual);
}

fn eth_coin_for_test(coin_type: EthCoinType, urls: Vec<String>) -> (MmArc, EthCoin) {
    let key_pair = KeyPair::from_secret_slice(
        &hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap(),
    )
    .unwrap();
    let transport = Web3Transport::new(urls).unwrap();
    let web3 = Web3::new(transport);
    let ctx = MmCtxBuilder::new().into_mm_arc();

    let eth_coin = EthCoin(Arc::new(EthCoinImpl {
        coin_type,
        decimals: 18,
        gas_station_url: None,
        history_sync_state: Mutex::new(HistorySyncState::NotEnabled),
        my_address: key_pair.address(),
        key_pair,
        swap_contract_address: Address::from("0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94"),
        ticker: "ETH".into(),
        web3_instances: vec![Web3Instance {
            web3: web3.clone(),
            is_parity: true,
        }],
        web3,
        ctx: ctx.weak(),
        required_confirmations: 1.into(),
    }));
    (ctx, eth_coin)
}

#[test]
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-55.md#test-cases
fn test_check_sum_address() {
    check_sum(
        "0xfb6916095ca1df60bb79ce92ce3ea74c37c5d359",
        "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
    );
    check_sum(
        "0x52908400098527886e0f7030069857d2e4169ee7",
        "0x52908400098527886E0F7030069857D2E4169EE7",
    );
    check_sum(
        "0x8617e340b3d01fa5f11f306f4090fd50e238070d",
        "0x8617E340B3D01FA5F11F306F4090FD50E238070D",
    );
    check_sum(
        "0xde709f2102306220921060314715629080e2fb77",
        "0xde709f2102306220921060314715629080e2fb77",
    );
    check_sum(
        "0x27b1fdb04752bbc536007a920d24acb045561c26",
        "0x27b1fdb04752bbc536007a920d24acb045561c26",
    );
    check_sum(
        "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
        "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
    );
    check_sum(
        "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
        "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
    );
    check_sum(
        "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB",
        "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB",
    );
    check_sum(
        "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb",
        "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb",
    );
}

#[test]
fn test_is_valid_checksum_addr() {
    assert!(is_valid_checksum_addr("0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359"));
    assert!(is_valid_checksum_addr("0x52908400098527886E0F7030069857D2E4169EE7"));
    assert!(!is_valid_checksum_addr("0x8617e340B3D01FA5F11F306F4090FD50E238070D"));
    assert!(!is_valid_checksum_addr("0xd1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb"));
}

#[test]
fn display_u256_with_point() {
    let number = U256::from_dec_str("1000000000000000000").unwrap();
    let string = display_u256_with_decimal_point(number, 18);
    assert_eq!("1.", string);

    let number = U256::from_dec_str("10000000000000000000000000000000000000000000000000000000000").unwrap();
    let string = display_u256_with_decimal_point(number, 18);
    assert_eq!("10000000000000000000000000000000000000000.", string);

    let number = U256::from_dec_str("1234567890000000000").unwrap();
    let string = display_u256_with_decimal_point(number, 18);
    assert_eq!("1.23456789", string);

    let number = U256::from_dec_str("1234567890000000000").unwrap();
    let string = display_u256_with_decimal_point(number, 16);
    assert_eq!("123.456789", string);

    let number = U256::from_dec_str("1234567890000000000").unwrap();
    let string = display_u256_with_decimal_point(number, 0);
    assert_eq!("1234567890000000000.", string);

    let number = U256::from_dec_str("1000000000000000").unwrap();
    let string = display_u256_with_decimal_point(number, 18);
    assert_eq!("0.001", string);

    let number = U256::from_dec_str("0").unwrap();
    let string = display_u256_with_decimal_point(number, 18);
    assert_eq!("0.", string);

    let number = U256::from_dec_str("0").unwrap();
    let string = display_u256_with_decimal_point(number, 0);
    assert_eq!("0.", string);
}

#[test]
fn test_wei_from_big_decimal() {
    let amount = "0.000001".parse().unwrap();
    let wei = wei_from_big_decimal(&amount, 18).unwrap();
    let expected_wei: U256 = 1000000000000u64.into();
    assert_eq!(expected_wei, wei);

    let amount = "1.000001".parse().unwrap();
    let wei = wei_from_big_decimal(&amount, 18).unwrap();
    let expected_wei: U256 = 1000001000000000000u64.into();
    assert_eq!(expected_wei, wei);

    let amount = 1.into();
    let wei = wei_from_big_decimal(&amount, 18).unwrap();
    let expected_wei: U256 = 1000000000000000000u64.into();
    assert_eq!(expected_wei, wei);

    let amount = "0.000000000000000001".parse().unwrap();
    let wei = wei_from_big_decimal(&amount, 18).unwrap();
    let expected_wei: U256 = 1u64.into();
    assert_eq!(expected_wei, wei);

    let amount = 1234.into();
    let wei = wei_from_big_decimal(&amount, 9).unwrap();
    let expected_wei: U256 = 1234000000000u64.into();
    assert_eq!(expected_wei, wei);

    let amount = 1234.into();
    let wei = wei_from_big_decimal(&amount, 0).unwrap();
    let expected_wei: U256 = 1234u64.into();
    assert_eq!(expected_wei, wei);

    let amount = 1234.into();
    let wei = wei_from_big_decimal(&amount, 1).unwrap();
    let expected_wei: U256 = 12340u64.into();
    assert_eq!(expected_wei, wei);

    let amount = "1234.12345".parse().unwrap();
    let wei = wei_from_big_decimal(&amount, 1).unwrap();
    let expected_wei: U256 = 12341u64.into();
    assert_eq!(expected_wei, wei);
}

#[test]
#[ignore]
/// temporary ignore, will refactor later to use dev chain and properly check transaction statuses
fn send_and_refund_erc20_payment() {
    let key_pair = KeyPair::from_secret_slice(
        &hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap(),
    )
    .unwrap();
    let transport = Web3Transport::new(vec!["http://195.201.0.6:8545".into()]).unwrap();
    let web3 = Web3::new(transport);
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let coin = EthCoin(Arc::new(EthCoinImpl {
        ticker: "ETH".into(),
        coin_type: EthCoinType::Erc20(Address::from("0xc0eb7AeD740E1796992A08962c15661bDEB58003")),
        my_address: key_pair.address(),
        key_pair,
        swap_contract_address: Address::from("0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94"),
        web3_instances: vec![Web3Instance {
            web3: web3.clone(),
            is_parity: true,
        }],
        web3,
        decimals: 18,
        gas_station_url: None,
        history_sync_state: Mutex::new(HistorySyncState::NotStarted),
        ctx: ctx.weak(),
        required_confirmations: 1.into(),
    }));

    let payment = coin
        .send_maker_payment(
            (now_ms() / 1000) as u32 - 200,
            &unwrap!(hex::decode(
                "03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06"
            )),
            &[1; 20],
            "0.001".parse().unwrap(),
            &coin.swap_contract_address(),
        )
        .wait()
        .unwrap();

    log!([payment]);

    thread::sleep(Duration::from_secs(60));

    let refund = coin
        .send_maker_refunds_payment(
            &payment.tx_hex(),
            (now_ms() / 1000) as u32 - 200,
            &unwrap!(hex::decode(
                "03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06"
            )),
            &[1; 20],
            &coin.swap_contract_address(),
        )
        .wait()
        .unwrap();

    log!([refund]);
}

#[test]
#[ignore]
/// temporary ignore, will refactor later to use dev chain and properly check transaction statuses
fn send_and_refund_eth_payment() {
    let key_pair = KeyPair::from_secret_slice(
        &hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap(),
    )
    .unwrap();
    let transport = Web3Transport::new(vec!["http://195.201.0.6:8545".into()]).unwrap();
    let web3 = Web3::new(transport);
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let coin = EthCoin(Arc::new(EthCoinImpl {
        ticker: "ETH".into(),
        coin_type: EthCoinType::Eth,
        my_address: key_pair.address(),
        key_pair,
        swap_contract_address: Address::from("0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94"),
        web3_instances: vec![Web3Instance {
            web3: web3.clone(),
            is_parity: true,
        }],
        web3,
        decimals: 18,
        gas_station_url: None,
        history_sync_state: Mutex::new(HistorySyncState::NotStarted),
        ctx: ctx.weak(),
        required_confirmations: 1.into(),
    }));

    let payment = coin
        .send_maker_payment(
            (now_ms() / 1000) as u32 - 200,
            &unwrap!(hex::decode(
                "03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06"
            )),
            &[1; 20],
            "0.001".parse().unwrap(),
            &coin.swap_contract_address(),
        )
        .wait()
        .unwrap();

    log!([payment]);

    thread::sleep(Duration::from_secs(60));

    let refund = coin
        .send_maker_refunds_payment(
            &payment.tx_hex(),
            (now_ms() / 1000) as u32 - 200,
            &unwrap!(hex::decode(
                "03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06"
            )),
            &[1; 20],
            &coin.swap_contract_address(),
        )
        .wait()
        .unwrap();

    log!([refund]);
}

#[test]
#[ignore]
fn test_nonce_several_urls() {
    let key_pair = KeyPair::from_secret_slice(
        &hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap(),
    )
    .unwrap();
    let infura_transport = Web3Transport::new(vec![
        "https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b".into()
    ])
    .unwrap();
    let linkpool_transport = Web3Transport::new(vec!["https://ropsten-rpc.linkpool.io".into()]).unwrap();
    // get nonce must succeed if some nodes are down at the moment for some reason
    let failing_transport = Web3Transport::new(vec!["http://195.201.0.6:8989".into()]).unwrap();

    let web3_infura = Web3::new(infura_transport);
    let web3_linkpool = Web3::new(linkpool_transport);
    let web3_failing = Web3::new(failing_transport);

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let coin = EthCoin(Arc::new(EthCoinImpl {
        ticker: "ETH".into(),
        coin_type: EthCoinType::Eth,
        my_address: key_pair.address(),
        key_pair,
        swap_contract_address: Address::from("0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94"),
        web3_instances: vec![
            Web3Instance {
                web3: web3_infura.clone(),
                is_parity: false,
            },
            Web3Instance {
                web3: web3_linkpool,
                is_parity: false,
            },
            Web3Instance {
                web3: web3_failing,
                is_parity: false,
            },
        ],
        web3: web3_infura,
        decimals: 18,
        gas_station_url: Some("https://ethgasstation.info/json/ethgasAPI.json".into()),
        history_sync_state: Mutex::new(HistorySyncState::NotStarted),
        ctx: ctx.weak(),
        required_confirmations: 1.into(),
    }));

    log!("My address "[coin.my_address]);
    log!("before payment");
    let payment = coin.send_to_address(coin.my_address, 200000000.into()).wait().unwrap();

    log!([payment]);
    let new_nonce = get_addr_nonce(coin.my_address, coin.web3_instances.clone())
        .wait()
        .unwrap();
    log!([new_nonce]);
}

#[test]
fn test_wait_for_payment_spend_timeout() {
    EthCoinImpl::spend_events.mock_safe(|_, _, _| MockResult::Return(Box::new(futures01::future::ok(vec![]))));

    let key_pair = KeyPair::from_secret_slice(
        &hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap(),
    )
    .unwrap();
    let transport = Web3Transport::new(vec!["http://195.201.0.6:8555".into()]).unwrap();
    let web3 = Web3::new(transport);
    let ctx = MmCtxBuilder::new().into_mm_arc();

    let coin = EthCoinImpl {
        coin_type: EthCoinType::Eth,
        decimals: 18,
        gas_station_url: None,
        history_sync_state: Mutex::new(HistorySyncState::NotEnabled),
        my_address: key_pair.address(),
        key_pair,
        swap_contract_address: Address::from("0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94"),
        ticker: "ETH".into(),
        web3_instances: vec![Web3Instance {
            web3: web3.clone(),
            is_parity: true,
        }],
        web3,
        ctx: ctx.weak(),
        required_confirmations: 1.into(),
    };

    let coin = EthCoin(Arc::new(coin));
    let wait_until = (now_ms() / 1000) - 1;
    let from_block = 1;
    // raw transaction bytes of https://etherscan.io/tx/0x0869be3e5d4456a29d488a533ad6c118620fef450f36778aecf31d356ff8b41f
    let tx_bytes = [
        248, 240, 3, 133, 1, 42, 5, 242, 0, 131, 2, 73, 240, 148, 133, 0, 175, 192, 188, 82, 20, 114, 128, 130, 22, 51,
        38, 194, 255, 12, 115, 244, 168, 113, 135, 110, 205, 245, 24, 127, 34, 254, 184, 132, 21, 44, 243, 175, 73, 33,
        143, 82, 117, 16, 110, 27, 133, 82, 200, 114, 233, 42, 140, 198, 35, 21, 201, 249, 187, 180, 20, 46, 148, 40,
        9, 228, 193, 130, 71, 199, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 152, 41, 132, 9, 201, 73, 19, 94, 237, 137, 35,
        61, 4, 194, 207, 239, 152, 75, 175, 245, 157, 174, 10, 214, 161, 207, 67, 70, 87, 246, 231, 212, 47, 216, 119,
        68, 237, 197, 125, 141, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 93, 72, 125, 102, 28, 159, 180, 237, 198, 97, 87, 80, 82, 200, 104, 40, 245,
        221, 7, 28, 122, 104, 91, 99, 1, 159, 140, 25, 131, 101, 74, 87, 50, 168, 146, 187, 90, 160, 51, 1, 123, 247,
        6, 108, 165, 181, 188, 40, 56, 47, 211, 229, 221, 73, 5, 15, 89, 81, 117, 225, 216, 108, 98, 226, 119, 232, 94,
        184, 42, 106,
    ];

    assert!(coin
        .wait_for_tx_spend(&tx_bytes, wait_until, from_block, &coin.swap_contract_address())
        .wait()
        .is_err());
}

#[test]
fn test_search_for_swap_tx_spend_was_spent() {
    let key_pair = KeyPair::from_secret_slice(
        &hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap(),
    )
    .unwrap();
    let transport = Web3Transport::new(vec![
        "https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b".into()
    ])
    .unwrap();
    let web3 = Web3::new(transport);
    let ctx = MmCtxBuilder::new().into_mm_arc();

    let swap_contract_address = Address::from("0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94");
    let coin = EthCoin(Arc::new(EthCoinImpl {
        coin_type: EthCoinType::Eth,
        decimals: 18,
        gas_station_url: None,
        history_sync_state: Mutex::new(HistorySyncState::NotEnabled),
        my_address: key_pair.address(),
        key_pair,
        swap_contract_address,
        ticker: "ETH".into(),
        web3_instances: vec![Web3Instance {
            web3: web3.clone(),
            is_parity: true,
        }],
        web3,
        ctx: ctx.weak(),
        required_confirmations: 1.into(),
    }));

    // raw transaction bytes of https://ropsten.etherscan.io/tx/0xb1c987e2ac79581bb8718267b5cb49a18274890494299239d1d0dfdb58d6d76a
    let payment_tx = [
        248, 240, 52, 132, 119, 53, 148, 0, 131, 2, 73, 240, 148, 123, 193, 187, 221, 106, 10, 114, 47, 201, 191, 252,
        73, 201, 33, 182, 133, 236, 184, 75, 148, 135, 71, 13, 228, 223, 130, 0, 0, 184, 132, 21, 44, 243, 175, 188,
        96, 248, 252, 165, 132, 81, 30, 243, 34, 85, 165, 46, 224, 176, 90, 137, 30, 19, 123, 224, 67, 83, 53, 74, 57,
        148, 140, 95, 45, 70, 147, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 117, 244, 28, 175, 51, 95, 91, 184, 141, 201,
        45, 116, 26, 102, 210, 119, 151, 124, 143, 52, 215, 128, 89, 116, 30, 25, 35, 128, 122, 186, 177, 228, 149,
        250, 55, 53, 62, 196, 51, 72, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 93, 56, 62, 80, 28, 160, 65, 22, 195, 212, 184, 202, 226, 151, 224, 111,
        174, 31, 160, 219, 39, 69, 137, 37, 8, 127, 177, 4, 104, 248, 27, 41, 245, 176, 131, 188, 215, 136, 160, 91,
        134, 199, 67, 1, 58, 57, 103, 23, 215, 176, 64, 124, 1, 44, 88, 161, 200, 160, 64, 110, 13, 145, 127, 180, 27,
        171, 131, 253, 90, 48, 147,
    ];
    // raw transaction bytes of https://ropsten.etherscan.io/tx/0xcb7c14d3ff309996d582400369393b6fa42314c52245115d4a3f77f072c36da9
    let spend_tx = [
        249, 1, 9, 37, 132, 119, 53, 148, 0, 131, 2, 73, 240, 148, 123, 193, 187, 221, 106, 10, 114, 47, 201, 191, 252,
        73, 201, 33, 182, 133, 236, 184, 75, 148, 128, 184, 164, 2, 237, 41, 43, 188, 96, 248, 252, 165, 132, 81, 30,
        243, 34, 85, 165, 46, 224, 176, 90, 137, 30, 19, 123, 224, 67, 83, 53, 74, 57, 148, 140, 95, 45, 70, 147, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 71, 13, 228, 223, 130, 0, 0, 168, 151, 11,
        232, 224, 253, 63, 180, 26, 114, 23, 184, 27, 10, 161, 80, 178, 251, 73, 204, 80, 174, 97, 118, 149, 204, 186,
        187, 243, 185, 19, 128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 157, 73, 251, 238, 138, 245, 142, 240, 85, 44, 209, 63, 194, 242,
        109, 242, 246, 6, 76, 176, 27, 160, 29, 157, 226, 23, 81, 174, 34, 82, 93, 182, 41, 248, 119, 42, 221, 214, 38,
        243, 128, 2, 235, 208, 193, 192, 74, 208, 242, 26, 221, 83, 54, 74, 160, 111, 29, 92, 8, 75, 61, 97, 103, 199,
        100, 189, 72, 74, 221, 144, 66, 170, 68, 121, 29, 105, 19, 194, 35, 245, 196, 131, 236, 29, 105, 101, 30,
    ];
    let spend_tx = FoundSwapTxSpend::Spent(unwrap!(signed_eth_tx_from_bytes(&spend_tx)).into());

    let found_tx = unwrap!(unwrap!(coin.search_for_swap_tx_spend(
        &payment_tx,
        swap_contract_address,
        6051857,
    )));
    assert_eq!(spend_tx, found_tx);
}

#[test]
fn test_search_for_swap_tx_spend_was_refunded() {
    let key_pair = KeyPair::from_secret_slice(
        &hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap(),
    )
    .unwrap();
    let transport = Web3Transport::new(vec![
        "https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b".into()
    ])
    .unwrap();
    let web3 = Web3::new(transport);
    let ctx = MmCtxBuilder::new().into_mm_arc();

    let swap_contract_address = Address::from("0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94");
    let coin = EthCoin(Arc::new(EthCoinImpl {
        coin_type: EthCoinType::Erc20(Address::from("0xc0eb7aed740e1796992a08962c15661bdeb58003")),
        decimals: 18,
        gas_station_url: None,
        history_sync_state: Mutex::new(HistorySyncState::NotEnabled),
        my_address: key_pair.address(),
        key_pair,
        swap_contract_address,
        ticker: "ETH".into(),
        web3_instances: vec![Web3Instance {
            web3: web3.clone(),
            is_parity: true,
        }],
        web3,
        ctx: ctx.weak(),
        required_confirmations: 1.into(),
    }));

    // raw transaction bytes of https://ropsten.etherscan.io/tx/0xe18bbca69dea9a4624e1f5b0b2021d5fe4c8daa03f36084a8ba011b08e5cd938
    let payment_tx = [
        249, 1, 43, 130, 10, 96, 132, 149, 2, 249, 0, 131, 2, 73, 240, 148, 123, 193, 187, 221, 106, 10, 114, 47, 201,
        191, 252, 73, 201, 33, 182, 133, 236, 184, 75, 148, 128, 184, 196, 155, 65, 91, 42, 192, 158, 192, 175, 210,
        198, 159, 244, 116, 46, 255, 28, 236, 147, 240, 68, 91, 16, 19, 6, 59, 187, 149, 138, 179, 151, 121, 47, 14,
        80, 251, 147, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 141, 126, 164, 198,
        128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 192, 235, 122, 237, 116, 14, 23, 150, 153, 42, 8, 150, 44, 21, 102,
        27, 222, 181, 128, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 216, 153, 121, 65, 221, 19, 70, 233, 35, 17, 24, 213,
        104, 93, 134, 98, 148, 245, 158, 91, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 93,
        23, 98, 207, 27, 160, 4, 198, 61, 242, 141, 248, 157, 72, 229, 2, 162, 163, 250, 159, 26, 66, 37, 42, 159, 35,
        58, 94, 57, 121, 252, 166, 34, 25, 206, 193, 113, 198, 160, 68, 125, 142, 153, 210, 177, 60, 173, 67, 127, 138,
        52, 112, 9, 49, 108, 109, 44, 177, 142, 9, 124, 10, 200, 37, 100, 52, 137, 196, 74, 67, 192,
    ];
    // raw transaction bytes of https://ropsten.etherscan.io/tx/0x9a50ac4d1737f4f04b94177996da7fa942b09469de52cfdadce891cd85afc37c
    let refund_tx = [
        249, 1, 11, 130, 10, 97, 132, 149, 2, 249, 0, 131, 2, 73, 240, 148, 123, 193, 187, 221, 106, 10, 114, 47, 201,
        191, 252, 73, 201, 33, 182, 133, 236, 184, 75, 148, 128, 184, 164, 70, 252, 2, 148, 192, 158, 192, 175, 210,
        198, 159, 244, 116, 46, 255, 28, 236, 147, 240, 68, 91, 16, 19, 6, 59, 187, 149, 138, 179, 151, 121, 47, 14,
        80, 251, 147, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 141, 126, 164, 198,
        128, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 192, 235, 122, 237, 116, 14, 23, 150, 153, 42, 8, 150, 44, 21, 102, 27, 222, 181,
        128, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 216, 153, 121, 65, 221, 19, 70, 233, 35, 17, 24, 213, 104, 93, 134,
        98, 148, 245, 158, 91, 28, 160, 127, 220, 190, 77, 221, 188, 140, 162, 198, 6, 127, 102, 222, 66, 38, 96, 10,
        19, 27, 208, 119, 219, 60, 231, 2, 118, 91, 169, 99, 78, 209, 135, 160, 51, 115, 90, 189, 124, 172, 205, 134,
        203, 159, 238, 40, 39, 99, 88, 48, 160, 189, 37, 60, 20, 117, 65, 238, 36, 98, 226, 48, 22, 235, 86, 183,
    ];
    let refund_tx = FoundSwapTxSpend::Refunded(unwrap!(signed_eth_tx_from_bytes(&refund_tx)).into());

    let found_tx = unwrap!(unwrap!(coin.search_for_swap_tx_spend(
        &payment_tx,
        swap_contract_address,
        5886908,
    )));
    assert_eq!(refund_tx, found_tx);
}

#[test]
fn test_withdraw_impl_manual_fee() {
    let (ctx, coin) = eth_coin_for_test(EthCoinType::Eth, vec!["http://dummy.dummy".into()]);

    EthCoin::my_balance.mock_safe(|_| {
        let balance = wei_from_big_decimal(&1000000000.into(), 18).unwrap();
        MockResult::Return(Box::new(futures01::future::ok(balance)))
    });
    get_addr_nonce.mock_safe(|_, _| MockResult::Return(Box::new(futures01::future::ok(0.into()))));

    let withdraw_req = WithdrawRequest {
        amount: 1.into(),
        to: "0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94".to_string(),
        coin: "ETH".to_string(),
        max: false,
        fee: Some(WithdrawFee::EthGas {
            gas: 150000,
            gas_price: 1.into(),
        }),
    };
    coin.my_balance().wait().unwrap();

    let tx_details = unwrap!(block_on(withdraw_impl(ctx, coin.clone(), withdraw_req)));
    let expected = Some(
        EthTxFeeDetails {
            coin: "ETH".into(),
            gas_price: "0.000000001".parse().unwrap(),
            gas: 150000,
            total_fee: "0.00015".parse().unwrap(),
        }
        .into(),
    );
    assert_eq!(expected, tx_details.fee_details);
}

#[test]
fn test_nonce_lock() {
    // send several transactions concurrently to check that they are not using same nonce
    // using real ETH dev node
    let (ctx, coin) = eth_coin_for_test(EthCoinType::Eth, vec!["http://195.201.0.6:8565".into()]);
    let mut futures = vec![];
    for _ in 0..5 {
        futures.push(sign_and_send_transaction_impl(
            ctx.clone(),
            coin.clone(),
            1000000000000u64.into(),
            Action::Call(coin.my_address),
            vec![],
            21000.into(),
        ));
    }
    let results = block_on(join_all(futures));
    for result in results {
        unwrap!(result);
    }
    // Waiting for NONCE_LOCK… might not appear at all if waiting takes less than 0.5 seconds
    // but all transactions are sent successfully still
    // unwrap!(wait_for_log(&ctx.log, 1.1, &|line| line.contains("Waiting for NONCE_LOCK…")));
    unwrap!(wait_for_log(&ctx.log, 1.1, &|line| line.contains("get_addr_nonce…")));
}

#[cfg(feature = "w-bindgen")]
mod wasm_bindgen_tests {
    use super::*;
    use crate::lp_coininit;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_test::*;
    use web_sys::console;

    #[wasm_bindgen_test]
    fn pass() {
        use super::CoinsContext;
        use common::mm_ctx::MmCtxBuilder;
        let ctx = MmCtxBuilder::default().into_mm_arc();
        let coins_context = unwrap!(CoinsContext::from_ctx(&ctx));
        assert_eq!(1, 1);
    }

    #[wasm_bindgen]
    extern "C" {
        fn setInterval(closure: &Closure<FnMut()>, millis: u32) -> f64;
        fn cancelInterval(token: f64);
    }

    wasm_bindgen_test_configure!(run_in_browser);

    pub struct Interval {
        closure: Closure<FnMut()>,
    }

    impl Interval {
        fn new() -> Interval {
            let closure = Closure::new(common::executor::run);
            Interval { closure }
        }
    }

    unsafe impl Send for Interval {}

    unsafe impl Sync for Interval {}

    lazy_static! {
        static ref EXECUTOR_INTERVAL: Interval = Interval::new();
    }

    #[wasm_bindgen_test(async)]
    fn test_send() -> impl Future<Item = (), Error = JsValue> {
        setInterval(&EXECUTOR_INTERVAL.closure, 200);
        Box::pin(async move {
            let key_pair = KeyPair::from_secret_slice(
                &hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap(),
            )
            .unwrap();
            let transport = Web3Transport::new(vec!["http://195.201.0.6:8565".into()]).unwrap();
            let web3 = Web3::new(transport);
            let ctx = MmCtxBuilder::new().into_mm_arc();
            let coin = EthCoin(Arc::new(EthCoinImpl {
                ticker: "ETH".into(),
                coin_type: EthCoinType::Eth,
                my_address: key_pair.address(),
                key_pair,
                swap_contract_address: Address::from("0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94"),
                web3_instances: vec![Web3Instance {
                    web3: web3.clone(),
                    is_parity: true,
                }],
                web3,
                decimals: 18,
                gas_station_url: None,
                history_sync_state: Mutex::new(HistorySyncState::NotStarted),
                ctx: ctx.weak(),
                required_confirmations: 1.into(),
            }));
            let tx = coin
                .send_maker_payment(
                    1000,
                    &unwrap!(hex::decode(
                        "03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06"
                    )),
                    &[1; 20],
                    "0.001".parse().unwrap(),
                )
                .compat()
                .await;
            console::log_1(&format!("{:?}", tx).into());

            let block = coin.current_block().compat().await;
            console::log_1(&format!("{:?}", block).into());
            Ok(())
        })
        .compat()
    }

    #[wasm_bindgen_test(async)]
    fn test_init_eth_coin() -> impl Future<Item = (), Error = JsValue> {
        use common::privkey::key_pair_from_seed;

        setInterval(&EXECUTOR_INTERVAL.closure, 200);
        Box::pin(async move {
            let key_pair =
                key_pair_from_seed("spice describe gravity federal blast come thank unfair canal monkey style afraid")
                    .unwrap();
            let conf = json!({
                "coins": [{
                    "coin": "ETH",
                    "name": "ethereum",
                    "fname": "Ethereum",
                    "protocol":{
                        "type": "ETH"
                    },
                    "rpcport": 80,
                    "mm2": 1
                }]
            });
            let ctx = MmCtxBuilder::new()
                .with_conf(conf)
                .with_secp256k1_key_pair(key_pair)
                .into_mm_arc();

            let req = json!({
                "urls":["http://195.201.0.6:8565"],
                "swap_contract_address":"0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
            });
            let coin = lp_coininit(&ctx, "ETH", &req).await.unwrap();
            Ok(())
        })
        .compat()
    }
}

#[test]
fn test_add_ten_pct_one_gwei() {
    let num = wei_from_big_decimal(&"0.1".parse().unwrap(), 9).unwrap();
    let expected = wei_from_big_decimal(&"1.1".parse().unwrap(), 9).unwrap();
    let actual = add_ten_pct_one_gwei(num);
    assert_eq!(expected, actual);

    let num = wei_from_big_decimal(&"9.9".parse().unwrap(), 9).unwrap();
    let expected = wei_from_big_decimal(&"10.9".parse().unwrap(), 9).unwrap();
    let actual = add_ten_pct_one_gwei(num);
    assert_eq!(expected, actual);

    let num = wei_from_big_decimal(&"30.1".parse().unwrap(), 9).unwrap();
    let expected = wei_from_big_decimal(&"33.11".parse().unwrap(), 9).unwrap();
    let actual = add_ten_pct_one_gwei(num);
    assert_eq!(expected, actual);
}
