use super::*;
use crate::utxo::rpc_clients::{ElectrumProtocol};
use mocktopus::mocking::*;

fn utxo_coin_for_test() -> UtxoCoin {
    let checksum_type = ChecksumType::DSHA256;
    let key_pair = key_pair_from_seed("spice describe gravity federal blast come thank unfair canal monkey style afraid").unwrap();
    let my_address = Address {
        prefix: 60,
        hash: key_pair.public().address_hash(),
        t_addr_prefix: 0,
        checksum_type,
    };

    let mut client = ElectrumClientImpl::new();
    client.add_server(&ElectrumRpcRequest {
        url: "electrum1.cipig.net:10025".into(),
        protocol: ElectrumProtocol::TCP,
        disable_cert_verification: false,
    }).unwrap();

    let coin = UtxoCoinImpl {
        decimals: 8,
        rpc_client: UtxoRpcClientEnum::Electrum(ElectrumClient(Arc::new(client))),
        key_pair,
        is_pos: false,
        notarized: false,
        overwintered: true,
        segwit: false,
        tx_version: 4,
        my_address,
        asset_chain: true,
        p2sh_addr_prefix: 85,
        p2sh_t_addr_prefix: 0,
        pub_addr_prefix: 60,
        pub_t_addr_prefix: 0,
        ticker: "ETOMIC".into(),
        wif_prefix: 0,
        tx_fee: TxFee::Fixed(1000),
        version_group_id: 0x892f2085,
        zcash: true,
        checksum_type,
        fork_id: 0,
        signature_version: SignatureVersion::Base,
        history_sync_state: Mutex::new(HistorySyncState::NotEnabled),
    };

    UtxoCoin(Arc::new(coin))
}

#[test]
fn test_extract_secret() {
    let tx: UtxoTx = "0100000001de7aa8d29524906b2b54ee2e0281f3607f75662cbc9080df81d1047b78e21dbc00000000d7473044022079b6c50820040b1fbbe9251ced32ab334d33830f6f8d0bf0a40c7f1336b67d5b0220142ccf723ddabb34e542ed65c395abc1fbf5b6c3e730396f15d25c49b668a1a401209da937e5609680cb30bff4a7661364ca1d1851c2506fa80c443f00a3d3bf7365004c6b6304f62b0e5cb175210270e75970bb20029b3879ec76c4acd320a8d0589e003636264d01a7d566504bfbac6782012088a9142fb610d856c19fd57f2d0cffe8dff689074b3d8a882103f368228456c940ac113e53dad5c104cf209f2f102a409207269383b6ab9b03deac68ffffffff01d0dc9800000000001976a9146d9d2b554d768232320587df75c4338ecc8bf37d88ac40280e5c".into();
    let secret = tx.extract_secret().unwrap();
    let expected_secret = hex::decode("9da937e5609680cb30bff4a7661364ca1d1851c2506fa80c443f00a3d3bf7365").unwrap();
    assert_eq!(expected_secret, secret);
}

#[test]
fn test_generate_transaction() {
    let coin = utxo_coin_for_test();
    let unspents = vec![UnspentInfo {
        value: 10000000000,
        outpoint: OutPoint::default(),
    }];

    let outputs = vec![TransactionOutput {
        script_pubkey: vec![].into(),
        value: 999,
    }];

    let generated = coin.generate_transaction(unspents, outputs, FeePolicy::SendExact).wait();
    // must not allow to use output with value < dust
    unwrap_err!(generated);

    let unspents = vec![UnspentInfo {
        value: 100000,
        outpoint: OutPoint::default(),
    }];

    let outputs = vec![TransactionOutput {
        script_pubkey: vec![].into(),
        value: 98001,
    }];

    let generated = unwrap!(coin.generate_transaction(unspents, outputs, FeePolicy::SendExact).wait());
    // the change that is less than dust must be included to miner fee
    // so no extra outputs should appear in generated transaction
    assert_eq!(generated.0.outputs.len(), 1);

    assert_eq!(generated.1.fee_amount, 1999);
    assert_eq!(generated.1.received_by_me, 0);
    assert_eq!(generated.1.spent_by_me, 100000);

    let unspents = vec![UnspentInfo {
        value: 100000,
        outpoint: OutPoint::default(),
    }];

    let outputs = vec![TransactionOutput {
        script_pubkey: "76a91405aab5342166f8594baf17a7d9bef5d56744332788ac".into(),
        value: 100000,
    }];

    // test that fee is properly deducted from output amount equal to input amount (max withdraw case)
    let generated = unwrap!(coin.generate_transaction(unspents, outputs, FeePolicy::DeductFromOutput(0)).wait());
    assert_eq!(generated.0.outputs.len(), 1);

    assert_eq!(generated.1.fee_amount, 1000);
    assert_eq!(generated.1.received_by_me, 99000);
    assert_eq!(generated.1.spent_by_me, 100000);
    assert_eq!(generated.0.outputs[0].value, 99000);

    let unspents = vec![UnspentInfo {
        value: 100000,
        outpoint: OutPoint::default(),
    }];

    let outputs = vec![TransactionOutput {
        script_pubkey: vec![].into(),
        value: 100000,
    }];

    // test that generate_transaction returns an error when input amount is not sufficient to cover output + fee
    unwrap_err!(coin.generate_transaction(unspents, outputs, FeePolicy::SendExact).wait());
}

#[test]
fn test_addresses_from_script() {
    let coin = utxo_coin_for_test();
    // P2PKH
    let script: Script = "76a91405aab5342166f8594baf17a7d9bef5d56744332788ac".into();
    let expected_addr: Vec<Address> = vec!["R9o9xTocqr6CeEDGDH6mEYpwLoMz6jNjMW".into()];
    let actual_addr = unwrap!(coin.addresses_from_script(&script));
    assert_eq!(expected_addr, actual_addr);

    // P2SH
    let script: Script = "a914e71a6120653ebd526e0f9d7a29cde5969db362d487".into();
    let expected_addr: Vec<Address> = vec!["bZoEPR7DjTqSDiQTeRFNDJuQPTRY2335LD".into()];
    let actual_addr = unwrap!(coin.addresses_from_script(&script));
    assert_eq!(expected_addr, actual_addr);
}

#[test]
fn test_kmd_interest() {
    let value = 64605500822;
    let lock_time = 1556623906;
    let current_time = 1556623906 + 3600 + 300;
    let expected = 36870;
    let actual = kmd_interest(1000001, value, lock_time, current_time);
    assert_eq!(expected, actual);

    // UTXO amount must be at least 10 KMD to be eligible for interest
    let value = 999999999;
    let lock_time = 1556623906;
    let current_time = 1556623906 + 3600 + 300;
    let expected = 0;
    let actual = kmd_interest(1000001, value, lock_time, current_time);
    assert_eq!(expected, actual);
}

#[test]
fn test_sat_from_big_decimal() {
    let amount = "0.000001".parse().unwrap();
    let sat = sat_from_big_decimal(&amount, 18).unwrap();
    let expected_sat = 1000000000000;
    assert_eq!(expected_sat, sat);

    let amount = "0.12345678".parse().unwrap();
    let sat = sat_from_big_decimal(&amount, 8).unwrap();
    let expected_sat = 12345678;
    assert_eq!(expected_sat, sat);

    let amount = "1.000001".parse().unwrap();
    let sat = sat_from_big_decimal(&amount, 18).unwrap();
    let expected_sat = 1000001000000000000;
    assert_eq!(expected_sat, sat);

    let amount = 1.into();
    let sat = sat_from_big_decimal(&amount, 18).unwrap();
    let expected_sat = 1000000000000000000;
    assert_eq!(expected_sat, sat);

    let amount = "0.000000000000000001".parse().unwrap();
    let sat = sat_from_big_decimal(&amount, 18).unwrap();
    let expected_sat = 1u64;
    assert_eq!(expected_sat, sat);

    let amount = 1234.into();
    let sat = sat_from_big_decimal(&amount, 9).unwrap();
    let expected_sat = 1234000000000;
    assert_eq!(expected_sat, sat);

    let amount = 1234.into();
    let sat = sat_from_big_decimal(&amount, 0).unwrap();
    let expected_sat = 1234;
    assert_eq!(expected_sat, sat);

    let amount = 1234.into();
    let sat = sat_from_big_decimal(&amount, 1).unwrap();
    let expected_sat = 12340;
    assert_eq!(expected_sat, sat);

    let amount = "1234.12345".parse().unwrap();
    let sat = sat_from_big_decimal(&amount, 1).unwrap();
    let expected_sat = 12341;
    assert_eq!(expected_sat, sat);
}

#[test]
fn test_wait_for_payment_spend_timeout_native() {
    use rpc::v1::types::VerboseBlockClient;

    let client = NativeClientImpl {
        uri: "http://127.0.0.1:10271".to_owned(),
        auth: fomat!("Basic " (base64_encode("user481805103:pass97a61c8d048bcf468c6c39a314970e557f57afd1d8a5edee917fb29bafb3a43371", URL_SAFE))),
    };

    NativeClientImpl::get_block_count.mock_safe(|_| MockResult::Return(Box::new(futures::future::ok(1000))));
    NativeClientImpl::get_block.mock_safe(|_, _| {
        let block = VerboseBlockClient {
            bits: "".into(),
            chainwork: 0.into(),
            confirmations: 0,
            difficulty: 0.,
            hash: 0.into(),
            height: None,
            mediantime: None,
            merkleroot: 0.into(),
            nextblockhash: None,
            nonce: "".into(),
            previousblockhash: None,
            size: 0,
            strippedsize: None,
            time: 0,
            tx: vec![],
            version: 0,
            version_hex: None,
            weight: None,
        };
        MockResult::Return(Box::new(futures::future::ok(block)))
    });
    let client = NativeClient(Arc::new(client));
    let transaction: UtxoTx = "01000000000102fff7f7881a8099afa6940d42d1e7f6362bec38171ea3edf433541db4e4ad969f00000000494830450221008b9d1dc26ba6a9cb62127b02742fa9d754cd3bebf337f7a55d114c8e5cdd30be022040529b194ba3f9281a99f2b1c0a19c0489bc22ede944ccf4ecbab4cc618ef3ed01eeffffffef51e1b804cc89d182d279655c3aa89e815b1b309fe287d9b2b55d57b90ec68a0100000000ffffffff02202cb206000000001976a9148280b37df378db99f66f85c95a783a76ac7a6d5988ac9093510d000000001976a9143bde42dbee7e4dbe6a21b2d50ce2f0167faa815988ac000247304402203609e17b84f6a7d30c80bfa610b5b4542f32a8a0d5447a12fb1366d7f01cc44a0220573a954c4518331561406f90300e8f3358f51928d43c212a8caed02de67eebee0121025476c2e83188368da1ff3e292e7acafcdb3566bb0ad253f62fc70f07aeee635711000000".into();
    let vout = 0;
    let wait_until = now_ms() / 1000 - 1;
    let from_block = 1000;

    assert!(client.wait_for_payment_spend(&transaction, vout, wait_until, from_block).is_err());
}

#[test]
fn test_wait_for_payment_spend_timeout_electrum() {
    ElectrumClientImpl::scripthash_get_history.mock_safe(|_, _| MockResult::Return(Box::new(futures::future::ok(vec![]))));

    let client = ElectrumClientImpl::new();
    let client = ElectrumClient(Arc::new(client));
    let transaction: UtxoTx = "01000000000102fff7f7881a8099afa6940d42d1e7f6362bec38171ea3edf433541db4e4ad969f00000000494830450221008b9d1dc26ba6a9cb62127b02742fa9d754cd3bebf337f7a55d114c8e5cdd30be022040529b194ba3f9281a99f2b1c0a19c0489bc22ede944ccf4ecbab4cc618ef3ed01eeffffffef51e1b804cc89d182d279655c3aa89e815b1b309fe287d9b2b55d57b90ec68a0100000000ffffffff02202cb206000000001976a9148280b37df378db99f66f85c95a783a76ac7a6d5988ac9093510d000000001976a9143bde42dbee7e4dbe6a21b2d50ce2f0167faa815988ac000247304402203609e17b84f6a7d30c80bfa610b5b4542f32a8a0d5447a12fb1366d7f01cc44a0220573a954c4518331561406f90300e8f3358f51928d43c212a8caed02de67eebee0121025476c2e83188368da1ff3e292e7acafcdb3566bb0ad253f62fc70f07aeee635711000000".into();
    let vout = 0;
    let wait_until = now_ms() / 1000 - 1;
    let from_block = 1000;

    assert!(client.wait_for_payment_spend(&transaction, vout, wait_until, from_block).is_err());
}
