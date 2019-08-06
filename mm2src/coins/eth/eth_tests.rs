use super::*;
use mocktopus::mocking::*;

fn check_sum(addr: &str, expected: &str) {
    let actual = checksum_address(addr);
    assert_eq!(expected, actual);
}

#[test]
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-55.md#test-cases
fn test_check_sum_address() {
    check_sum("0xfb6916095ca1df60bb79ce92ce3ea74c37c5d359", "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359");
    check_sum("0x52908400098527886e0f7030069857d2e4169ee7", "0x52908400098527886E0F7030069857D2E4169EE7");
    check_sum("0x8617e340b3d01fa5f11f306f4090fd50e238070d", "0x8617E340B3D01FA5F11F306F4090FD50E238070D");
    check_sum("0xde709f2102306220921060314715629080e2fb77", "0xde709f2102306220921060314715629080e2fb77");
    check_sum("0x27b1fdb04752bbc536007a920d24acb045561c26", "0x27b1fdb04752bbc536007a920d24acb045561c26");
    check_sum("0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed", "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed");
    check_sum("0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359", "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359");
    check_sum("0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB", "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB");
    check_sum("0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb", "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb");
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
    let key_pair = KeyPair::from_secret_slice(&hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap()).unwrap();
    let transport = Web3Transport::new(vec!["http://195.201.0.6:8545".into()]).unwrap();
    let web3 = Web3::new(transport);
    let coin = EthCoin(Arc::new(EthCoinImpl {
        ticker: "ETH".into(),
        coin_type: EthCoinType::Erc20(Address::from("0xc0eb7AeD740E1796992A08962c15661bDEB58003")),
        my_address: key_pair.address(),
        key_pair,
        swap_contract_address: Address::from("0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94"),
        web3_instances: vec![Web3Instance {web3: web3.clone(), is_parity: true}],
        web3,
        decimals: 18,
        gas_station_url: None,
        history_sync_state: Mutex::new(HistorySyncState::NotStarted),
    }));

    let payment = coin.send_maker_payment(
        (now_ms() / 1000) as u32 - 200,
        &unwrap!(hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06")),
        &[1; 20],
        "0.001".parse().unwrap(),
    ).wait().unwrap();

    log!([payment]);

    thread::sleep(Duration::from_secs(60));

    let refund = coin.send_maker_refunds_payment(
        &payment.tx_hex(),
        (now_ms() / 1000) as u32 - 200,
        &unwrap!(hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06")),
        &[1; 20],
    ).wait().unwrap();

    log!([refund]);
}

#[test]
#[ignore]
/// temporary ignore, will refactor later to use dev chain and properly check transaction statuses
fn send_and_refund_eth_payment() {
    let key_pair = KeyPair::from_secret_slice(&hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap()).unwrap();
    let transport = Web3Transport::new(vec!["http://195.201.0.6:8545".into()]).unwrap();
    let web3 = Web3::new(transport);
    let coin = EthCoin(Arc::new(EthCoinImpl {
        ticker: "ETH".into(),
        coin_type: EthCoinType::Eth,
        my_address: key_pair.address(),
        key_pair,
        swap_contract_address: Address::from("0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94"),
        web3_instances: vec![Web3Instance {web3: web3.clone(), is_parity: true}],
        web3,
        decimals: 18,
        gas_station_url: None,
        history_sync_state: Mutex::new(HistorySyncState::NotStarted),
    }));

    let payment = coin.send_maker_payment(
        (now_ms() / 1000) as u32 - 200,
        &unwrap!(hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06")),
        &[1; 20],
        "0.001".parse().unwrap(),
    ).wait().unwrap();

    log!([payment]);

    thread::sleep(Duration::from_secs(60));

    let refund = coin.send_maker_refunds_payment(
        &payment.tx_hex(),
        (now_ms() / 1000) as u32 - 200,
        &unwrap!(hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06")),
        &[1; 20],
    ).wait().unwrap();

    log!([refund]);
}

#[test]
#[ignore]
fn test_nonce_several_urls() {
    let key_pair = KeyPair::from_secret_slice(&hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap()).unwrap();
    let my_transport = Web3Transport::new(vec!["https://ropsten.infura.io/v3/c01c1b4cf66642528547624e1d6d9d6b".into()]).unwrap();
    let infura_transport = Web3Transport::new(vec!["https://ropsten-rpc.linkpool.io".into()]).unwrap();
    let web_infura = Web3::new(infura_transport);
    let web3 = Web3::new(my_transport);
    let coin = EthCoin(Arc::new(EthCoinImpl {
        ticker: "ETH".into(),
        coin_type: EthCoinType::Eth,
        my_address: key_pair.address(),
        key_pair,
        swap_contract_address: Address::from("0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94"),
        web3_instances: vec![Web3Instance { web3: web_infura, is_parity: false }, Web3Instance { web3: web3.clone(), is_parity: false }],
        web3,
        decimals: 18,
        gas_station_url: Some("https://ethgasstation.info/json/ethgasAPI.json".into()),
        history_sync_state: Mutex::new(HistorySyncState::NotStarted),
    }));

    log!("My address " [coin.my_address]);
    log!("before payment");
    let payment = coin.send_to_address(coin.my_address, 200000000.into()).wait().unwrap();

    log!([payment]);
    let new_nonce = get_addr_nonce(coin.my_address, &coin.web3_instances).wait().unwrap();
    log!([new_nonce]);
}

#[test]
fn test_wait_for_payment_spend_timeout() {
    EthCoinImpl::spend_events.mock_safe(|_, _| MockResult::Return(Box::new(futures::future::ok(vec![]))));

    let key_pair = KeyPair::from_secret_slice(&hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap()).unwrap();
    let transport = Web3Transport::new(vec!["http://195.201.0.6:8555".into()]).unwrap();
    let web3 = Web3::new(transport);

    let coin = EthCoinImpl {
        coin_type: EthCoinType::Eth,
        decimals: 18,
        gas_station_url: None,
        history_sync_state: Mutex::new(HistorySyncState::NotEnabled),
        my_address: key_pair.address(),
        key_pair,
        swap_contract_address: Address::from("0x7Bc1bBDD6A0a722fC9bffC49c921B685ECB84b94"),
        ticker: "ETH".into(),
        web3_instances: vec![Web3Instance {web3: web3.clone(), is_parity: true}],
        web3,
    };

    let coin = EthCoin(Arc::new(coin));
    let wait_until = (now_ms() / 1000) - 1;
    let from_block = 1;
    // raw transaction bytes of https://etherscan.io/tx/0x0869be3e5d4456a29d488a533ad6c118620fef450f36778aecf31d356ff8b41f
    let tx_bytes = [248, 240, 3, 133, 1, 42, 5, 242, 0, 131, 2, 73, 240, 148, 133, 0, 175, 192, 188, 82, 20, 114, 128, 130, 22, 51, 38, 194, 255, 12, 115, 244, 168, 113, 135, 110, 205, 245, 24, 127, 34, 254, 184, 132, 21, 44, 243, 175, 73, 33, 143, 82, 117, 16, 110, 27, 133, 82, 200, 114, 233, 42, 140, 198, 35, 21, 201, 249, 187, 180, 20, 46, 148, 40, 9, 228, 193, 130, 71, 199, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 152, 41, 132, 9, 201, 73, 19, 94, 237, 137, 35, 61, 4, 194, 207, 239, 152, 75, 175, 245, 157, 174, 10, 214, 161, 207, 67, 70, 87, 246, 231, 212, 47, 216, 119, 68, 237, 197, 125, 141, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 93, 72, 125, 102, 28, 159, 180, 237, 198, 97, 87, 80, 82, 200, 104, 40, 245, 221, 7, 28, 122, 104, 91, 99, 1, 159, 140, 25, 131, 101, 74, 87, 50, 168, 146, 187, 90, 160, 51, 1, 123, 247, 6, 108, 165, 181, 188, 40, 56, 47, 211, 229, 221, 73, 5, 15, 89, 81, 117, 225, 216, 108, 98, 226, 119, 232, 94, 184, 42, 106];

    assert!(coin.wait_for_tx_spend(&tx_bytes, wait_until, from_block).is_err());
}
