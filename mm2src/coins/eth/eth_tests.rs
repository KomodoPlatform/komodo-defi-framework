use super::*;

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
