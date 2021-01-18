use super::rpc_clients::{ElectrumProtocol, ListSinceBlockRes, NetworkInfo};
use super::*;
use crate::utxo::rpc_clients::{GetAddressInfoRes, UtxoRpcClientOps, ValidateAddressRes};
use crate::utxo::utxo_standard::{utxo_standard_coin_from_conf_and_request, UtxoStandardCoin};
use crate::{SwapOps, WithdrawFee};
use bigdecimal::BigDecimal;
use chain::OutPoint;
use common::mm_ctx::MmCtxBuilder;
use common::privkey::key_pair_from_seed;
use common::{block_on, OrdRange};
use futures::future::join_all;
use gstuff::now_ms;
use mocktopus::mocking::*;
use rpc::v1::types::{VerboseBlockClient, H256 as H256Json};
use serialization::deserialize;
use std::collections::HashMap;
use std::thread;
use std::time::Duration;

const TEST_COIN_NAME: &'static str = "RICK";

pub fn electrum_client_for_test(servers: &[&str]) -> ElectrumClient {
    let client = ElectrumClientImpl::new(TEST_COIN_NAME.into(), Default::default());
    for server in servers {
        block_on(client.add_server(&ElectrumRpcRequest {
            url: server.to_string(),
            protocol: ElectrumProtocol::TCP,
            disable_cert_verification: false,
        }))
        .unwrap();
    }

    let mut attempts = 0;
    while !block_on(client.is_connected()) {
        if attempts >= 10 {
            panic!("Failed to connect to at least 1 of {:?} in 5 seconds.", servers);
        }

        thread::sleep(Duration::from_millis(500));
        attempts += 1;
    }

    ElectrumClient(Arc::new(client))
}

/// Returned client won't work by default, requires some mocks to be usable
fn native_client_for_test() -> NativeClient { NativeClient(Arc::new(NativeClientImpl::default())) }

fn utxo_coin_fields_for_test(rpc_client: UtxoRpcClientEnum, force_seed: Option<&str>) -> UtxoCoinFields {
    let checksum_type = ChecksumType::DSHA256;
    let default_seed = "spice describe gravity federal blast come thank unfair canal monkey style afraid";
    let seed = match force_seed {
        Some(s) => s.into(),
        None => match std::env::var("BOB_PASSPHRASE") {
            Ok(p) => {
                if p.is_empty() {
                    default_seed.into()
                } else {
                    p
                }
            },
            Err(_) => default_seed.into(),
        },
    };
    let key_pair = key_pair_from_seed(&seed).unwrap();
    let my_address = Address {
        prefix: 60,
        hash: key_pair.public().address_hash(),
        t_addr_prefix: 0,
        checksum_type,
    };
    let my_script_pubkey = Builder::build_p2pkh(&my_address.hash).to_bytes();

    UtxoCoinFields {
        decimals: 8,
        rpc_client,
        key_pair,
        is_pos: false,
        requires_notarization: false.into(),
        overwintered: true,
        segwit: false,
        tx_version: 4,
        my_address,
        address_format: UtxoAddressFormat::Standard,
        asset_chain: true,
        p2sh_addr_prefix: 85,
        p2sh_t_addr_prefix: 0,
        pub_addr_prefix: 60,
        pub_t_addr_prefix: 0,
        ticker: TEST_COIN_NAME.into(),
        wif_prefix: 0,
        tx_fee: TxFee::Fixed(1000),
        version_group_id: 0x892f2085,
        consensus_branch_id: 0x76b809bb,
        zcash: true,
        checksum_type,
        fork_id: 0,
        signature_version: SignatureVersion::Base,
        history_sync_state: Mutex::new(HistorySyncState::NotEnabled),
        required_confirmations: 1.into(),
        force_min_relay_fee: false,
        mtp_block_count: NonZeroU64::new(11).unwrap(),
        estimate_fee_mode: None,
        dust_amount: UTXO_DUST_AMOUNT,
        mature_confirmations: MATURE_CONFIRMATIONS_DEFAULT,
        tx_cache_directory: None,
        recently_spent_outpoints: AsyncMutex::new(RecentlySpentOutPoints::new(my_script_pubkey)),
    }
}

fn utxo_coin_from_fields(coin: UtxoCoinFields) -> UtxoStandardCoin {
    let arc: UtxoArc = coin.into();
    arc.into()
}

fn utxo_coin_for_test(rpc_client: UtxoRpcClientEnum, force_seed: Option<&str>) -> UtxoStandardCoin {
    utxo_coin_from_fields(utxo_coin_fields_for_test(rpc_client, force_seed))
}

#[test]
fn test_extract_secret() {
    let client = electrum_client_for_test(&["electrum1.cipig.net:10017"]);
    let coin = utxo_coin_for_test(client.into(), None);

    let tx_hex = hex::decode("0100000001de7aa8d29524906b2b54ee2e0281f3607f75662cbc9080df81d1047b78e21dbc00000000d7473044022079b6c50820040b1fbbe9251ced32ab334d33830f6f8d0bf0a40c7f1336b67d5b0220142ccf723ddabb34e542ed65c395abc1fbf5b6c3e730396f15d25c49b668a1a401209da937e5609680cb30bff4a7661364ca1d1851c2506fa80c443f00a3d3bf7365004c6b6304f62b0e5cb175210270e75970bb20029b3879ec76c4acd320a8d0589e003636264d01a7d566504bfbac6782012088a9142fb610d856c19fd57f2d0cffe8dff689074b3d8a882103f368228456c940ac113e53dad5c104cf209f2f102a409207269383b6ab9b03deac68ffffffff01d0dc9800000000001976a9146d9d2b554d768232320587df75c4338ecc8bf37d88ac40280e5c").unwrap();
    let expected_secret = hex::decode("9da937e5609680cb30bff4a7661364ca1d1851c2506fa80c443f00a3d3bf7365").unwrap();
    let secret_hash = &*dhash160(&expected_secret);
    let secret = unwrap!(coin.extract_secret(secret_hash, &tx_hex));
    assert_eq!(secret, expected_secret);
}

#[test]
fn test_generate_transaction() {
    let client = electrum_client_for_test(&["electrum1.cipig.net:10017"]);
    let coin = utxo_coin_for_test(client.into(), None);
    let unspents = vec![UnspentInfo {
        value: 10000000000,
        outpoint: OutPoint::default(),
        height: Default::default(),
    }];

    let outputs = vec![TransactionOutput {
        script_pubkey: vec![].into(),
        value: 999,
    }];

    let generated = block_on(coin.generate_transaction(unspents, outputs, FeePolicy::SendExact, None, None));
    // must not allow to use output with value < dust
    unwrap_err!(generated);

    let unspents = vec![UnspentInfo {
        value: 100000,
        outpoint: OutPoint::default(),
        height: Default::default(),
    }];

    let outputs = vec![TransactionOutput {
        script_pubkey: vec![].into(),
        value: 98001,
    }];

    let generated = unwrap!(block_on(coin.generate_transaction(
        unspents,
        outputs,
        FeePolicy::SendExact,
        None,
        None
    )));
    // the change that is less than dust must be included to miner fee
    // so no extra outputs should appear in generated transaction
    assert_eq!(generated.0.outputs.len(), 1);

    assert_eq!(generated.1.fee_amount, 1999);
    assert_eq!(generated.1.received_by_me, 0);
    assert_eq!(generated.1.spent_by_me, 100000);

    let unspents = vec![UnspentInfo {
        value: 100000,
        outpoint: OutPoint::default(),
        height: Default::default(),
    }];

    let outputs = vec![TransactionOutput {
        script_pubkey: Builder::build_p2pkh(&coin.as_ref().my_address.hash).to_bytes(),
        value: 100000,
    }];

    // test that fee is properly deducted from output amount equal to input amount (max withdraw case)
    let generated = unwrap!(block_on(coin.generate_transaction(
        unspents,
        outputs,
        FeePolicy::DeductFromOutput(0),
        None,
        None
    )));
    assert_eq!(generated.0.outputs.len(), 1);

    assert_eq!(generated.1.fee_amount, 1000);
    assert_eq!(generated.1.received_by_me, 99000);
    assert_eq!(generated.1.spent_by_me, 100000);
    assert_eq!(generated.0.outputs[0].value, 99000);

    let unspents = vec![UnspentInfo {
        value: 100000,
        outpoint: OutPoint::default(),
        height: Default::default(),
    }];

    let outputs = vec![TransactionOutput {
        script_pubkey: vec![].into(),
        value: 100000,
    }];

    // test that generate_transaction returns an error when input amount is not sufficient to cover output + fee
    unwrap_err!(block_on(coin.generate_transaction(
        unspents,
        outputs,
        FeePolicy::SendExact,
        None,
        None
    )));
}

#[test]
fn test_addresses_from_script() {
    let client = electrum_client_for_test(&["electrum1.cipig.net:10017", "electrum2.cipig.net:10017"]);
    let coin = utxo_coin_for_test(client.into(), None);
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
    let height = Some(1000001);
    let value = 64605500822;
    let lock_time = 1556623906;
    let current_time = 1556623906 + 3600 + 300;

    let expected = 36870;
    let actual = kmd_interest(height, value, lock_time, current_time).unwrap();
    assert_eq!(expected, actual);

    // UTXO amount must be at least 10 KMD to be eligible for interest
    let actual = kmd_interest(height, 999999999, lock_time, current_time);
    assert_eq!(actual, Err(KmdRewardsNotAccruedReason::UtxoAmountLessThanTen));

    // Transaction is not mined yet (height is None)
    let actual = kmd_interest(None, value, lock_time, current_time);
    assert_eq!(actual, Err(KmdRewardsNotAccruedReason::TransactionInMempool));

    // Locktime is not set
    let actual = kmd_interest(height, value, 0, current_time);
    assert_eq!(actual, Err(KmdRewardsNotAccruedReason::LocktimeNotSet));

    // interest will stop accrue after block 7_777_777
    let actual = kmd_interest(Some(7_777_778), value, lock_time, current_time);
    assert_eq!(actual, Err(KmdRewardsNotAccruedReason::UtxoHeightGreaterThanEndOfEra));

    // interest doesn't accrue for lock_time < 500_000_000
    let actual = kmd_interest(height, value, 499_999_999, current_time);
    assert_eq!(actual, Err(KmdRewardsNotAccruedReason::LocktimeLessThanThreshold));

    // current time must be greater than tx lock_time
    let actual = kmd_interest(height, value, lock_time, lock_time - 1);
    assert_eq!(actual, Err(KmdRewardsNotAccruedReason::OneHourNotPassedYet));

    // at least 1 hour should pass
    let actual = kmd_interest(height, value, lock_time, lock_time + 30);
    assert_eq!(actual, Err(KmdRewardsNotAccruedReason::OneHourNotPassedYet));
}

#[test]
fn test_kmd_interest_accrue_stop_at() {
    let lock_time = 1595845640;
    let height = 1000001;

    let expected = lock_time + 31 * 24 * 60 * 60;
    let actual = kmd_interest_accrue_stop_at(height, lock_time);
    assert_eq!(expected, actual);

    let height = 999999;

    let expected = lock_time + 365 * 24 * 60 * 60;
    let actual = kmd_interest_accrue_stop_at(height, lock_time);
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
    let client = NativeClientImpl::default();

    static mut OUTPUT_SPEND_CALLED: bool = false;
    NativeClient::find_output_spend.mock_safe(|_, _, _, _| {
        unsafe { OUTPUT_SPEND_CALLED = true };
        MockResult::Return(Box::new(futures01::future::ok(None)))
    });
    let client = UtxoRpcClientEnum::Native(NativeClient(Arc::new(client)));
    let coin = utxo_coin_for_test(client, None);
    let transaction = unwrap!(hex::decode("01000000000102fff7f7881a8099afa6940d42d1e7f6362bec38171ea3edf433541db4e4ad969f00000000494830450221008b9d1dc26ba6a9cb62127b02742fa9d754cd3bebf337f7a55d114c8e5cdd30be022040529b194ba3f9281a99f2b1c0a19c0489bc22ede944ccf4ecbab4cc618ef3ed01eeffffffef51e1b804cc89d182d279655c3aa89e815b1b309fe287d9b2b55d57b90ec68a0100000000ffffffff02202cb206000000001976a9148280b37df378db99f66f85c95a783a76ac7a6d5988ac9093510d000000001976a9143bde42dbee7e4dbe6a21b2d50ce2f0167faa815988ac000247304402203609e17b84f6a7d30c80bfa610b5b4542f32a8a0d5447a12fb1366d7f01cc44a0220573a954c4518331561406f90300e8f3358f51928d43c212a8caed02de67eebee0121025476c2e83188368da1ff3e292e7acafcdb3566bb0ad253f62fc70f07aeee635711000000"));
    let wait_until = now_ms() / 1000 - 1;
    let from_block = 1000;

    assert!(coin
        .wait_for_tx_spend(&transaction, wait_until, from_block, &None)
        .wait()
        .is_err());
    assert!(unsafe { OUTPUT_SPEND_CALLED });
}

#[test]
fn test_wait_for_payment_spend_timeout_electrum() {
    static mut OUTPUT_SPEND_CALLED: bool = false;
    ElectrumClient::find_output_spend.mock_safe(|_, _, _, _| {
        unsafe { OUTPUT_SPEND_CALLED = true };
        MockResult::Return(Box::new(futures01::future::ok(None)))
    });

    let client = ElectrumClientImpl::new(TEST_COIN_NAME.into(), Default::default());
    let client = UtxoRpcClientEnum::Electrum(ElectrumClient(Arc::new(client)));
    let coin = utxo_coin_for_test(client, None);
    let transaction = unwrap!(hex::decode("01000000000102fff7f7881a8099afa6940d42d1e7f6362bec38171ea3edf433541db4e4ad969f00000000494830450221008b9d1dc26ba6a9cb62127b02742fa9d754cd3bebf337f7a55d114c8e5cdd30be022040529b194ba3f9281a99f2b1c0a19c0489bc22ede944ccf4ecbab4cc618ef3ed01eeffffffef51e1b804cc89d182d279655c3aa89e815b1b309fe287d9b2b55d57b90ec68a0100000000ffffffff02202cb206000000001976a9148280b37df378db99f66f85c95a783a76ac7a6d5988ac9093510d000000001976a9143bde42dbee7e4dbe6a21b2d50ce2f0167faa815988ac000247304402203609e17b84f6a7d30c80bfa610b5b4542f32a8a0d5447a12fb1366d7f01cc44a0220573a954c4518331561406f90300e8f3358f51928d43c212a8caed02de67eebee0121025476c2e83188368da1ff3e292e7acafcdb3566bb0ad253f62fc70f07aeee635711000000"));
    let wait_until = now_ms() / 1000 - 1;
    let from_block = 1000;

    assert!(coin
        .wait_for_tx_spend(&transaction, wait_until, from_block, &None)
        .wait()
        .is_err());
    assert!(unsafe { OUTPUT_SPEND_CALLED });
}

#[test]
fn test_search_for_swap_tx_spend_electrum_was_spent() {
    let secret = [0; 32];
    let client = electrum_client_for_test(&["electrum1.cipig.net:10017", "electrum2.cipig.net:10017"]);
    let coin = utxo_coin_for_test(
        client.into(),
        Some("spice describe gravity federal blast come thank unfair canal monkey style afraid"),
    );

    // raw tx bytes of https://rick.kmd.dev/tx/ba881ecca15b5d4593f14f25debbcdfe25f101fd2e9cf8d0b5d92d19813d4424
    let payment_tx_bytes = unwrap!(hex::decode("0400008085202f8902e115acc1b9e26a82f8403c9f81785445cc1285093b63b6246cf45aabac5e0865000000006b483045022100ca578f2d6bae02f839f71619e2ced54538a18d7aa92bd95dcd86ac26479ec9f802206552b6c33b533dd6fc8985415a501ebec89d1f5c59d0c923d1de5280e9827858012102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ffffffffb0721bf69163f7a5033fb3d18ba5768621d8c1347ebaa2fddab0d1f63978ea78020000006b483045022100a3309f99167982e97644dbb5cd7279b86630b35fc34855e843f2c5c0cafdc66d02202a8c3257c44e832476b2e2a723dad1bb4ec1903519502a49b936c155cae382ee012102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ffffffff0300e1f5050000000017a91443fde927a77b3c1d104b78155dc389078c4571b0870000000000000000166a14b8bcb07f6344b42ab04250c86a6e8b75d3fdbbc64b8cd736000000001976a91405aab5342166f8594baf17a7d9bef5d56744332788acba0ce35e000000000000000000000000000000"));

    // raw tx bytes of https://rick.kmd.dev/tx/cea8028f93f7556ce0ef96f14b8b5d88ef2cd29f428df5936e02e71ca5b0c795
    let spend_tx_bytes = unwrap!(hex::decode("0400008085202f890124443d81192dd9b5d0f89c2efd01f125fecdbbde254ff193455d5ba1cc1e88ba00000000d74730440220519d3eed69815a16357ff07bf453b227654dc85b27ffc22a77abe077302833ec02205c27f439ddc542d332504112871ecac310ea710b99e1922f48eb179c045e44ee01200000000000000000000000000000000000000000000000000000000000000000004c6b6304a9e5e25eb1752102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ac6782012088a914b8bcb07f6344b42ab04250c86a6e8b75d3fdbbc6882102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ac68ffffffff0118ddf505000000001976a91405aab5342166f8594baf17a7d9bef5d56744332788acbffee25e000000000000000000000000000000"));
    let spend_tx = TransactionEnum::UtxoTx(unwrap!(deserialize(spend_tx_bytes.as_slice())));

    let found = unwrap!(unwrap!(coin.search_for_swap_tx_spend_my(
        1591928233,
        &*coin.my_public_key(),
        &*dhash160(&secret),
        &payment_tx_bytes,
        0,
        &None,
    )));
    assert_eq!(FoundSwapTxSpend::Spent(spend_tx), found);
}

#[test]
fn test_search_for_swap_tx_spend_electrum_was_refunded() {
    let secret = [0; 20];
    let client = electrum_client_for_test(&["electrum1.cipig.net:10017", "electrum2.cipig.net:10017"]);
    let coin = utxo_coin_for_test(
        client.into(),
        Some("spice describe gravity federal blast come thank unfair canal monkey style afraid"),
    );

    // raw tx bytes of https://rick.kmd.dev/tx/78ea7839f6d1b0dafda2ba7e34c1d8218676a58bd1b33f03a5f76391f61b72b0
    let payment_tx_bytes = unwrap!(hex::decode("0400008085202f8902bf17bf7d1daace52e08f732a6b8771743ca4b1cb765a187e72fd091a0aabfd52000000006a47304402203eaaa3c4da101240f80f9c5e9de716a22b1ec6d66080de6a0cca32011cd77223022040d9082b6242d6acf9a1a8e658779e1c655d708379862f235e8ba7b8ca4e69c6012102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ffffffffff023ca13c0e9e085dd13f481f193e8a3e8fd609020936e98b5587342d994f4d020000006b483045022100c0ba56adb8de923975052312467347d83238bd8d480ce66e8b709a7997373994022048507bcac921fdb2302fa5224ce86e41b7efc1a2e20ae63aa738dfa99b7be826012102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ffffffff0300e1f5050000000017a9141ee6d4c38a3c078eab87ad1a5e4b00f21259b10d870000000000000000166a1400000000000000000000000000000000000000001b94d736000000001976a91405aab5342166f8594baf17a7d9bef5d56744332788ac2d08e35e000000000000000000000000000000"));

    // raw tx bytes of https://rick.kmd.dev/tx/65085eacab5af46c24b6633b098512cc455478819f3c40f8826ae2b9c1ac15e1
    let refund_tx_bytes = unwrap!(hex::decode("0400008085202f8901b0721bf69163f7a5033fb3d18ba5768621d8c1347ebaa2fddab0d1f63978ea7800000000b6473044022052e06c1abf639148229a3991fdc6da15fe51c97577f4fda351d9c606c7cf53670220780186132d67d354564cae710a77d94b6bb07dcbd7162a13bebee261ffc0963601514c6b63041dfae25eb1752102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ac6782012088a9140000000000000000000000000000000000000000882102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ac68feffffff0118ddf505000000001976a91405aab5342166f8594baf17a7d9bef5d56744332788ace6fae25e000000000000000000000000000000"));
    let refund_tx = TransactionEnum::UtxoTx(unwrap!(deserialize(refund_tx_bytes.as_slice())));

    let found = unwrap!(unwrap!(coin.search_for_swap_tx_spend_my(
        1591933469,
        coin.as_ref().key_pair.public(),
        &secret,
        &payment_tx_bytes,
        0,
        &None,
    )));
    assert_eq!(FoundSwapTxSpend::Refunded(refund_tx), found);
}

#[test]
fn test_withdraw_impl_set_fixed_fee() {
    UtxoStandardCoin::ordered_mature_unspents.mock_safe(|_, _| {
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::new(futures01::future::ok(unspents)))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None);

    let withdraw_req = WithdrawRequest {
        amount: 1.into(),
        to: "RQq6fWoy8aGGMLjvRfMY5mBNVm2RQxJyLa".to_string(),
        coin: TEST_COIN_NAME.into(),
        max: false,
        fee: Some(WithdrawFee::UtxoFixed {
            amount: "0.1".parse().unwrap(),
        }),
    };
    let expected = Some(
        UtxoFeeDetails {
            amount: "0.1".parse().unwrap(),
        }
        .into(),
    );
    let tx_details = unwrap!(coin.withdraw(withdraw_req).wait());
    assert_eq!(expected, tx_details.fee_details);
}

#[test]
fn test_withdraw_impl_sat_per_kb_fee() {
    UtxoStandardCoin::ordered_mature_unspents.mock_safe(|_, _| {
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::new(futures01::future::ok(unspents)))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None);

    let withdraw_req = WithdrawRequest {
        amount: 1.into(),
        to: "RQq6fWoy8aGGMLjvRfMY5mBNVm2RQxJyLa".to_string(),
        coin: TEST_COIN_NAME.into(),
        max: false,
        fee: Some(WithdrawFee::UtxoPerKbyte {
            amount: "0.1".parse().unwrap(),
        }),
    };
    // The resulting transaction size might be 244 or 245 bytes depending on signature size
    // MM2 always expects the worst case during fee calculation
    // 0.1 * 245 / 1000 ~ 0.0245
    let expected = Some(
        UtxoFeeDetails {
            amount: "0.0245".parse().unwrap(),
        }
        .into(),
    );
    let tx_details = unwrap!(coin.withdraw(withdraw_req).wait());
    assert_eq!(expected, tx_details.fee_details);
}

#[test]
fn test_withdraw_impl_sat_per_kb_fee_amount_equal_to_max() {
    UtxoStandardCoin::ordered_mature_unspents.mock_safe(|_, _| {
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::new(futures01::future::ok(unspents)))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None);

    let withdraw_req = WithdrawRequest {
        amount: "9.9789".parse().unwrap(),
        to: "RQq6fWoy8aGGMLjvRfMY5mBNVm2RQxJyLa".to_string(),
        coin: TEST_COIN_NAME.into(),
        max: false,
        fee: Some(WithdrawFee::UtxoPerKbyte {
            amount: "0.1".parse().unwrap(),
        }),
    };
    let tx_details = unwrap!(coin.withdraw(withdraw_req).wait());
    // The resulting transaction size might be 210 or 211 bytes depending on signature size
    // MM2 always expects the worst case during fee calculation
    // 0.1 * 211 / 1000 = 0.0211
    let expected_fee = Some(
        UtxoFeeDetails {
            amount: "0.0211".parse().unwrap(),
        }
        .into(),
    );
    assert_eq!(expected_fee, tx_details.fee_details);
    let expected_balance_change = BigDecimal::from(-10);
    assert_eq!(expected_balance_change, tx_details.my_balance_change);
}

#[test]
fn test_withdraw_impl_sat_per_kb_fee_amount_equal_to_max_dust_included_to_fee() {
    UtxoStandardCoin::ordered_mature_unspents.mock_safe(|_, _| {
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::new(futures01::future::ok(unspents)))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None);

    let withdraw_req = WithdrawRequest {
        amount: "9.9789".parse().unwrap(),
        to: "RQq6fWoy8aGGMLjvRfMY5mBNVm2RQxJyLa".to_string(),
        coin: TEST_COIN_NAME.into(),
        max: false,
        fee: Some(WithdrawFee::UtxoPerKbyte {
            amount: "0.09999999".parse().unwrap(),
        }),
    };
    let tx_details = unwrap!(coin.withdraw(withdraw_req).wait());
    // The resulting transaction size might be 210 or 211 bytes depending on signature size
    // MM2 always expects the worst case during fee calculation
    // 0.1 * 211 / 1000 = 0.0211
    let expected_fee = Some(
        UtxoFeeDetails {
            amount: "0.0211".parse().unwrap(),
        }
        .into(),
    );
    assert_eq!(expected_fee, tx_details.fee_details);
    let expected_balance_change = BigDecimal::from(-10);
    assert_eq!(expected_balance_change, tx_details.my_balance_change);
}

#[test]
fn test_withdraw_impl_sat_per_kb_fee_amount_over_max() {
    UtxoStandardCoin::ordered_mature_unspents.mock_safe(|_, _| {
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::new(futures01::future::ok(unspents)))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None);

    let withdraw_req = WithdrawRequest {
        amount: "9.97939455".parse().unwrap(),
        to: "RQq6fWoy8aGGMLjvRfMY5mBNVm2RQxJyLa".to_string(),
        coin: TEST_COIN_NAME.into(),
        max: false,
        fee: Some(WithdrawFee::UtxoPerKbyte {
            amount: "0.1".parse().unwrap(),
        }),
    };
    unwrap_err!(coin.withdraw(withdraw_req).wait());
}

#[test]
fn test_withdraw_impl_sat_per_kb_fee_max() {
    UtxoStandardCoin::ordered_mature_unspents.mock_safe(|_, _| {
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::new(futures01::future::ok(unspents)))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None);

    let withdraw_req = WithdrawRequest {
        amount: 0.into(),
        to: "RQq6fWoy8aGGMLjvRfMY5mBNVm2RQxJyLa".to_string(),
        coin: TEST_COIN_NAME.into(),
        max: true,
        fee: Some(WithdrawFee::UtxoPerKbyte {
            amount: "0.1".parse().unwrap(),
        }),
    };
    // The resulting transaction size might be 210 or 211 bytes depending on signature size
    // MM2 always expects the worst case during fee calculation
    // 0.1 * 211 / 1000 = 0.0211
    let expected = Some(
        UtxoFeeDetails {
            amount: "0.0211".parse().unwrap(),
        }
        .into(),
    );
    let tx_details = unwrap!(coin.withdraw(withdraw_req).wait());
    assert_eq!(expected, tx_details.fee_details);
}

#[test]
fn test_ordered_mature_unspents_without_tx_cache() {
    let client = electrum_client_for_test(&["electrum1.cipig.net:10017", "electrum2.cipig.net:10017"]);
    let coin = utxo_coin_for_test(
        client.into(),
        Some("spice describe gravity federal blast come thank unfair canal monkey style afraid"),
    );
    assert!(coin.as_ref().tx_cache_directory.is_none());
    assert_ne!(
        coin.my_balance().wait().unwrap(),
        0.into(),
        "The test address doesn't have unspent outputs"
    );
    let unspents = unwrap!(coin
        .ordered_mature_unspents(&Address::from("R9o9xTocqr6CeEDGDH6mEYpwLoMz6jNjMW"))
        .wait());
    assert!(!unspents.is_empty());
}

#[test]
fn test_utxo_lock() {
    // send several transactions concurrently to check that they are not using same inputs
    let client = electrum_client_for_test(&["electrum1.cipig.net:10017", "electrum2.cipig.net:10017"]);
    let coin = utxo_coin_for_test(client.into(), None);
    let output = TransactionOutput {
        value: 1000000,
        script_pubkey: Builder::build_p2pkh(&coin.as_ref().my_address.hash).to_bytes(),
    };
    let mut futures = vec![];
    for _ in 0..5 {
        futures.push(send_outputs_from_my_address_impl(coin.clone(), vec![output.clone()]));
    }
    let results = block_on(join_all(futures));
    for result in results {
        unwrap!(result);
    }
}

#[test]
fn list_since_block_btc_serde() {
    // https://github.com/KomodoPlatform/atomicDEX-API/issues/563
    let input = r#"{"lastblock":"000000000000000000066f896cca2a6c667ca85fff28ed6731d64e3c39ecb119","removed":[],"transactions":[{"abandoned":false,"address":"1Q3kQ1jsB2VyH83PJT1NXJqEaEcR6Yuknn","amount":-0.01788867,"bip125-replaceable":"no","blockhash":"0000000000000000000db4be4c2df08790e1027326832cc90889554bbebc69b7","blockindex":437,"blocktime":1572174214,"category":"send","confirmations":197,"fee":-0.00012924,"involvesWatchonly":true,"time":1572173721,"timereceived":1572173721,"txid":"29606e6780c69a39767b56dc758e6af31ced5232491ad62dcf25275684cb7701","vout":0,"walletconflicts":[]},{"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":0.1995,"bip125-replaceable":"no","blockhash":"0000000000000000000e75b33bbb27e6af2fc3898108c93c03c293fd72a86c6f","blockindex":157,"blocktime":1572179171,"category":"receive","confirmations":190,"label":"","time":1572178251,"timereceived":1572178251,"txid":"da651c6addc8da7c4b2bec21d43022852a93a9f2882a827704b318eb2966b82e","vout":19,"walletconflicts":[]},{"abandoned":false,"address":"14RXkMTyH4NyK48DbhTQyMBoMb2UkbBEPr","amount":-0.0208,"bip125-replaceable":"no","blockhash":"0000000000000000000611bfe0b3f7612239264459f4f6e7169f8d1a67e1b08f","blockindex":286,"blocktime":1572189657,"category":"send","confirmations":178,"fee":-0.0002,"involvesWatchonly":true,"time":1572189100,"timereceived":1572189100,"txid":"8d10920ce70aeb6c7e61c8d47f3cd903fb69946edd08d8907472a90761965943","vout":0,"walletconflicts":[]},{"abandoned":false,"address":"361JVximBAqkLZERT7XB1rykgLePEHAP7B","amount":-0.01801791,"bip125-replaceable":"no","blockhash":"00000000000000000011e9293c1f07f9711e677389ac101b93116d239ac38c33","blockindex":274,"blocktime":1572173649,"category":"send","confirmations":198,"fee":-0.0000965,"involvesWatchonly":true,"label":"361JVximBAqkLZERT7XB1rykgLePEHAP7B","time":1572173458,"timereceived":1572173458,"txid":"7983cae1afeb7fe58e020878aaedea0fee15be9319bc49c81f3b9ad466782950","vout":0,"walletconflicts":[]},{"abandoned":false,"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":-0.0003447,"bip125-replaceable":"no","blockhash":"00000000000000000011e9293c1f07f9711e677389ac101b93116d239ac38c33","blockindex":274,"blocktime":1572173649,"category":"send","confirmations":198,"fee":-0.0000965,"label":"","time":1572173458,"timereceived":1572173458,"txid":"7983cae1afeb7fe58e020878aaedea0fee15be9319bc49c81f3b9ad466782950","vout":1,"walletconflicts":[]},{"address":"361JVximBAqkLZERT7XB1rykgLePEHAP7B","amount":0.01801791,"bip125-replaceable":"no","blockhash":"00000000000000000011e9293c1f07f9711e677389ac101b93116d239ac38c33","blockindex":274,"blocktime":1572173649,"category":"receive","confirmations":198,"involvesWatchonly":true,"label":"361JVximBAqkLZERT7XB1rykgLePEHAP7B","time":1572173458,"timereceived":1572173458,"txid":"7983cae1afeb7fe58e020878aaedea0fee15be9319bc49c81f3b9ad466782950","vout":0,"walletconflicts":[]},{"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":0.0003447,"bip125-replaceable":"no","blockhash":"00000000000000000011e9293c1f07f9711e677389ac101b93116d239ac38c33","blockindex":274,"blocktime":1572173649,"category":"receive","confirmations":198,"label":"","time":1572173458,"timereceived":1572173458,"txid":"7983cae1afeb7fe58e020878aaedea0fee15be9319bc49c81f3b9ad466782950","vout":1,"walletconflicts":[]},{"abandoned":false,"address":"3B3q1GTLQQ7Fspo6ATy3cd3tg5yu97hkve","amount":-0.021,"bip125-replaceable":"no","blockhash":"0000000000000000000debf11962f89e2ae08f8ff75803b0da6170af6c5c346b","blockindex":2618,"blocktime":1572188894,"category":"send","confirmations":179,"fee":-0.00016026,"involvesWatchonly":true,"label":"3B3q1GTLQQ7Fspo6ATy3cd3tg5yu97hkve","time":1572186009,"timereceived":1572186009,"txid":"54b159ac3a656bbaaf3bf0263b8deafad03b376ec0c2e9c715d0cf1caaf3495e","vout":0,"walletconflicts":[]},{"abandoned":false,"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":-0.17868444,"bip125-replaceable":"no","blockhash":"0000000000000000000debf11962f89e2ae08f8ff75803b0da6170af6c5c346b","blockindex":2618,"blocktime":1572188894,"category":"send","confirmations":179,"fee":-0.00016026,"label":"","time":1572186009,"timereceived":1572186009,"txid":"54b159ac3a656bbaaf3bf0263b8deafad03b376ec0c2e9c715d0cf1caaf3495e","vout":1,"walletconflicts":[]},{"address":"3B3q1GTLQQ7Fspo6ATy3cd3tg5yu97hkve","amount":0.021,"bip125-replaceable":"no","blockhash":"0000000000000000000debf11962f89e2ae08f8ff75803b0da6170af6c5c346b","blockindex":2618,"blocktime":1572188894,"category":"receive","confirmations":179,"involvesWatchonly":true,"label":"3B3q1GTLQQ7Fspo6ATy3cd3tg5yu97hkve","time":1572186009,"timereceived":1572186009,"txid":"54b159ac3a656bbaaf3bf0263b8deafad03b376ec0c2e9c715d0cf1caaf3495e","vout":0,"walletconflicts":[]},{"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":0.17868444,"bip125-replaceable":"no","blockhash":"0000000000000000000debf11962f89e2ae08f8ff75803b0da6170af6c5c346b","blockindex":2618,"blocktime":1572188894,"category":"receive","confirmations":179,"label":"","time":1572186009,"timereceived":1572186009,"txid":"54b159ac3a656bbaaf3bf0263b8deafad03b376ec0c2e9c715d0cf1caaf3495e","vout":1,"walletconflicts":[]},{"abandoned":false,"address":"3AC6k1Y54knEdkgWjX3TjmWGjDHtJCNZZY","amount":-0.17822795,"bip125-replaceable":"no","blockhash":"00000000000000000009a60478f29f4910e29224ea5ed63d77321ac8c624ec45","blockindex":2377,"blocktime":1572190637,"category":"send","confirmations":177,"fee":-0.00009985,"involvesWatchonly":true,"label":"3AC6k1Y54knEdkgWjX3TjmWGjDHtJCNZZY","time":1572189626,"timereceived":1572189626,"txid":"eabc01e45db89ea8cf623f8e22847e4023c69bed3c7d396d573b89dec3fe17a7","vout":0,"walletconflicts":[]},{"abandoned":false,"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":-0.00035664,"bip125-replaceable":"no","blockhash":"00000000000000000009a60478f29f4910e29224ea5ed63d77321ac8c624ec45","blockindex":2377,"blocktime":1572190637,"category":"send","confirmations":177,"fee":-0.00009985,"label":"","time":1572189626,"timereceived":1572189626,"txid":"eabc01e45db89ea8cf623f8e22847e4023c69bed3c7d396d573b89dec3fe17a7","vout":1,"walletconflicts":[]},{"address":"3AC6k1Y54knEdkgWjX3TjmWGjDHtJCNZZY","amount":0.17822795,"bip125-replaceable":"no","blockhash":"00000000000000000009a60478f29f4910e29224ea5ed63d77321ac8c624ec45","blockindex":2377,"blocktime":1572190637,"category":"receive","confirmations":177,"involvesWatchonly":true,"label":"3AC6k1Y54knEdkgWjX3TjmWGjDHtJCNZZY","time":1572189626,"timereceived":1572189626,"txid":"eabc01e45db89ea8cf623f8e22847e4023c69bed3c7d396d573b89dec3fe17a7","vout":0,"walletconflicts":[]},{"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":0.00035664,"bip125-replaceable":"no","blockhash":"00000000000000000009a60478f29f4910e29224ea5ed63d77321ac8c624ec45","blockindex":2377,"blocktime":1572190637,"category":"receive","confirmations":177,"label":"","time":1572189626,"timereceived":1572189626,"txid":"eabc01e45db89ea8cf623f8e22847e4023c69bed3c7d396d573b89dec3fe17a7","vout":1,"walletconflicts":[]},{"abandoned":false,"address":"1Q3kQ1jsB2VyH83PJT1NXJqEaEcR6Yuknn","amount":-0.17809412,"bip125-replaceable":"no","blockhash":"000000000000000000125e17a9540ac901d70e92e987d59a1cf87ca36ebca830","blockindex":1680,"blocktime":1572191122,"category":"send","confirmations":176,"fee":-0.00013383,"involvesWatchonly":true,"time":1572190821,"timereceived":1572190821,"txid":"d3579f7be169ea8fd1358d0eda85bad31ce8080a6020dcd224eac8a663dc9bf7","vout":0,"walletconflicts":[]},{"abandoned":false,"address":"326VCyLKV1w4SxeYs81jQU1SC11njcL1eG","amount":-0.039676,"bip125-replaceable":"no","blockhash":"0000000000000000000d61630db06ed5d3054a39bf71a706efeaa9e86866b9d4","blockindex":2193,"blocktime":1572053656,"category":"send","confirmations":380,"fee":-0.00005653,"involvesWatchonly":true,"label":"326VCyLKV1w4SxeYs81jQU1SC11njcL1eG","time":1572052431,"timereceived":1572052431,"txid":"37b57fb36312e21ec7d069a55ab9bffc6abc7fe3731ed38502c5329025a9edf9","vout":0,"walletconflicts":[]},{"abandoned":false,"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":-0.01845911,"bip125-replaceable":"no","blockhash":"0000000000000000000d61630db06ed5d3054a39bf71a706efeaa9e86866b9d4","blockindex":2193,"blocktime":1572053656,"category":"send","confirmations":380,"fee":-0.00005653,"label":"","time":1572052431,"timereceived":1572052431,"txid":"37b57fb36312e21ec7d069a55ab9bffc6abc7fe3731ed38502c5329025a9edf9","vout":1,"walletconflicts":[]},{"address":"326VCyLKV1w4SxeYs81jQU1SC11njcL1eG","amount":0.039676,"bip125-replaceable":"no","blockhash":"0000000000000000000d61630db06ed5d3054a39bf71a706efeaa9e86866b9d4","blockindex":2193,"blocktime":1572053656,"category":"receive","confirmations":380,"involvesWatchonly":true,"label":"326VCyLKV1w4SxeYs81jQU1SC11njcL1eG","time":1572052431,"timereceived":1572052431,"txid":"37b57fb36312e21ec7d069a55ab9bffc6abc7fe3731ed38502c5329025a9edf9","vout":0,"walletconflicts":[]},{"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":0.01845911,"bip125-replaceable":"no","blockhash":"0000000000000000000d61630db06ed5d3054a39bf71a706efeaa9e86866b9d4","blockindex":2193,"blocktime":1572053656,"category":"receive","confirmations":380,"label":"","time":1572052431,"timereceived":1572052431,"txid":"37b57fb36312e21ec7d069a55ab9bffc6abc7fe3731ed38502c5329025a9edf9","vout":1,"walletconflicts":[]}]}"#;
    let _res: ListSinceBlockRes = unwrap!(json::from_str(input));
}

#[test]
#[ignore]
fn get_tx_details_doge() {
    let conf = json!(  {
        "coin": "DOGE",
        "name": "dogecoin",
        "fname": "Dogecoin",
        "rpcport": 22555,
        "pubtype": 30,
        "p2shtype": 22,
        "wiftype": 158,
        "txfee": 0,
        "mm2": 1,
        "required_confirmations": 2
    });
    let req = json!({
         "method": "electrum",
         "servers": [{"url":"electrum1.cipig.net:10060"},{"url":"electrum2.cipig.net:10060"},{"url":"electrum3.cipig.net:10060"}]
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();

    use common::executor::spawn;
    let coin = unwrap!(block_on(utxo_standard_coin_from_conf_and_request(
        &ctx, "DOGE", &conf, &req, &[1u8; 32]
    )));

    let coin1 = coin.clone();
    let coin2 = coin.clone();
    let fut1 = async move {
        let block = coin1.current_block().compat().await.unwrap();
        log!((block));
        let hash = hex::decode("99caab76bd025d189f10856dc649aad1a191b1cfd9b139ece457c5fedac58132").unwrap();
        loop {
            let tx_details = coin1.tx_details_by_hash(&hash).await.unwrap();
            log!([tx_details]);
            Timer::sleep(1.).await;
        }
    };
    let fut2 = async move {
        let block = coin2.current_block().compat().await.unwrap();
        log!((block));
        let hash = hex::decode("99caab76bd025d189f10856dc649aad1a191b1cfd9b139ece457c5fedac58132").unwrap();
        loop {
            let tx_details = coin2.tx_details_by_hash(&hash).await.unwrap();
            log!([tx_details]);
            Timer::sleep(1.).await;
        }
    };
    spawn(fut1);
    spawn(fut2);
    loop {}
}

#[test]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/587
fn get_tx_details_coinbase_transaction() {
    let client = electrum_client_for_test(&[
        "electrum1.cipig.net:10018",
        "electrum2.cipig.net:10018",
        "electrum3.cipig.net:10018",
    ]);
    let coin = utxo_coin_for_test(
        client.into(),
        Some("spice describe gravity federal blast come thank unfair canal monkey style afraid"),
    );

    let fut = async move {
        // hash of coinbase transaction https://morty.explorer.dexstats.info/tx/b59b093ed97c1798f2a88ee3375a0c11d0822b6e4468478777f899891abd34a5
        let hash = hex::decode("b59b093ed97c1798f2a88ee3375a0c11d0822b6e4468478777f899891abd34a5").unwrap();

        let tx_details = coin.tx_details_by_hash(&hash).await.unwrap();
        assert!(tx_details.from.is_empty());
    };

    block_on(fut);
}

#[test]
fn test_electrum_rpc_client_error() {
    let client = electrum_client_for_test(&["electrum1.cipig.net:10060"]);

    let empty_hash = H256Json::default();
    let err = unwrap_err!(client.get_verbose_transaction(empty_hash).wait());

    // use the static string instead because the actual error message cannot be obtain
    // by serde_json serialization
    let expected = r#"JsonRpcError { client_info: "coin: RICK", request: JsonRpcRequest { jsonrpc: "2.0", id: "0", method: "blockchain.transaction.get", params: [String("0000000000000000000000000000000000000000000000000000000000000000"), Bool(true)] }, error: Response(electrum1.cipig.net:10060, Object({"code": Number(2), "message": String("daemon error: DaemonError({\'code\': -5, \'message\': \'No such mempool or blockchain transaction. Use gettransaction for wallet transactions.\'})")})) }"#;
    let actual = format!("{}", err);

    assert_eq!(expected, actual);
}

#[test]
fn test_network_info_deserialization() {
    let network_info_kmd = r#"{
        "connections": 1,
        "localaddresses": [],
        "localservices": "0000000070000005",
        "networks": [
            {
                "limited": false,
                "name": "ipv4",
                "proxy": "",
                "proxy_randomize_credentials": false,
                "reachable": true
            },
            {
                "limited": false,
                "name": "ipv6",
                "proxy": "",
                "proxy_randomize_credentials": false,
                "reachable": true
            },
            {
                "limited": true,
                "name": "onion",
                "proxy": "",
                "proxy_randomize_credentials": false,
                "reachable": false
            }
        ],
        "protocolversion": 170007,
        "relayfee": 1e-06,
        "subversion": "/MagicBean:2.0.15-rc2/",
        "timeoffset": 0,
        "version": 2001526,
        "warnings": ""
    }"#;
    json::from_str::<NetworkInfo>(network_info_kmd).unwrap();

    let network_info_btc = r#"{
        "version": 180000,
        "subversion": "\/Satoshi:0.18.0\/",
        "protocolversion": 70015,
        "localservices": "000000000000040d",
        "localrelay": true,
        "timeoffset": 0,
        "networkactive": true,
        "connections": 124,
        "networks": [
            {
                "name": "ipv4",
                "limited": false,
                "reachable": true,
                "proxy": "",
                "proxy_randomize_credentials": false
            },
            {
                "name": "ipv6",
                "limited": false,
                "reachable": true,
                "proxy": "",
                "proxy_randomize_credentials": false
            },
            {
                "name": "onion",
                "limited": true,
                "reachable": false,
                "proxy": "",
                "proxy_randomize_credentials": false
            }
        ],
        "relayfee": 1.0e-5,
        "incrementalfee": 1.0e-5,
        "localaddresses": [
            {
                "address": "96.57.248.252",
                "port": 8333,
                "score": 618294
            }
        ],
        "warnings": ""
    }"#;
    json::from_str::<NetworkInfo>(network_info_btc).unwrap();
}

#[test]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/617
fn test_generate_transaction_relay_fee_is_used_when_dynamic_fee_is_lower() {
    let client = NativeClientImpl::default();

    static mut GET_RELAY_FEE_CALLED: bool = false;
    NativeClient::get_relay_fee.mock_safe(|_| {
        unsafe { GET_RELAY_FEE_CALLED = true };
        MockResult::Return(Box::new(futures01::future::ok("1.0".parse().unwrap())))
    });
    let client = UtxoRpcClientEnum::Native(NativeClient(Arc::new(client)));
    let mut coin = utxo_coin_fields_for_test(client, None);
    coin.force_min_relay_fee = true;
    let coin = utxo_coin_from_fields(coin);
    let unspents = vec![UnspentInfo {
        value: 1000000000,
        outpoint: OutPoint::default(),
        height: Default::default(),
    }];

    let outputs = vec![TransactionOutput {
        script_pubkey: vec![].into(),
        value: 900000000,
    }];

    let fut = coin.generate_transaction(
        unspents,
        outputs,
        FeePolicy::SendExact,
        Some(ActualTxFee::Dynamic(100)),
        None,
    );
    let generated = unwrap!(block_on(fut));
    assert_eq!(generated.0.outputs.len(), 1);

    // generated transaction fee must be equal to relay fee if calculated dynamic fee is lower than relay
    assert_eq!(generated.1.fee_amount, 100000000);
    assert_eq!(generated.1.received_by_me, 0);
    assert_eq!(generated.1.spent_by_me, 1000000000);
    assert!(unsafe { GET_RELAY_FEE_CALLED });
}

#[test]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/617
fn test_generate_tx_fee_is_correct_when_dynamic_fee_is_larger_than_relay() {
    let client = NativeClientImpl::default();

    static mut GET_RELAY_FEE_CALLED: bool = false;
    NativeClient::get_relay_fee.mock_safe(|_| {
        unsafe { GET_RELAY_FEE_CALLED = true };
        MockResult::Return(Box::new(futures01::future::ok("0.00001".parse().unwrap())))
    });
    let client = UtxoRpcClientEnum::Native(NativeClient(Arc::new(client)));
    let mut coin = utxo_coin_fields_for_test(client, None);
    coin.force_min_relay_fee = true;
    let coin = utxo_coin_from_fields(coin);
    let unspents = vec![
        UnspentInfo {
            value: 1000000000,
            outpoint: OutPoint::default(),
            height: Default::default(),
        };
        20
    ];

    let outputs = vec![TransactionOutput {
        script_pubkey: vec![].into(),
        value: 19000000000,
    }];

    let fut = coin.generate_transaction(
        unspents,
        outputs,
        FeePolicy::SendExact,
        Some(ActualTxFee::Dynamic(1000)),
        None,
    );
    let generated = unwrap!(block_on(fut));
    assert_eq!(generated.0.outputs.len(), 2);
    assert_eq!(generated.0.inputs.len(), 20);

    // resulting signed transaction size would be 3032 bytes so fee is 3032 sat
    assert_eq!(generated.1.fee_amount, 3032);
    assert_eq!(generated.1.received_by_me, 999996968);
    assert_eq!(generated.1.spent_by_me, 20000000000);
    assert!(unsafe { GET_RELAY_FEE_CALLED });
}

#[test]
fn test_get_median_time_past_from_electrum_kmd() {
    let client = electrum_client_for_test(&[
        "electrum1.cipig.net:10001",
        "electrum2.cipig.net:10001",
        "electrum3.cipig.net:10001",
    ]);

    let mtp = client
        .get_median_time_past(1773390, KMD_MTP_BLOCK_COUNT)
        .wait()
        .unwrap();
    // the MTP is block time of 1773385 in this case
    assert_eq!(1583159915, mtp);
}

#[test]
fn test_get_median_time_past_from_electrum_btc() {
    let client = electrum_client_for_test(&[
        "electrum1.cipig.net:10000",
        "electrum2.cipig.net:10000",
        "electrum3.cipig.net:10000",
    ]);

    let mtp = client.get_median_time_past(632858, KMD_MTP_BLOCK_COUNT).wait().unwrap();
    assert_eq!(1591173041, mtp);
}

#[test]
fn test_get_median_time_past_from_native_has_median_in_get_block() {
    let client = native_client_for_test();
    NativeClientImpl::get_block.mock_safe(|_, block_num| {
        assert_eq!(block_num, "632858".to_string());
        let block_data_str = r#"{"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632858,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591174568,"mediantime":1591173041,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"}"#;
        let block_data = json::from_str(block_data_str).unwrap();
        MockResult::Return(
            Box::new(futures01::future::ok(block_data))
        )
    });

    let mtp = client.get_median_time_past(632858, KMD_MTP_BLOCK_COUNT).wait().unwrap();
    assert_eq!(1591173041, mtp);
}

#[test]
fn test_get_median_time_past_from_native_does_not_have_median_in_get_block() {
    let blocks_json_str = r#"
    [
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632858,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173090,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632857,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173080,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632856,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173070,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632855,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173058,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632854,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173050,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632853,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173041,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632852,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173040,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632851,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173039,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632850,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173038,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632849,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173037,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632848,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173030,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"}
    ]
    "#;

    let blocks: Vec<VerboseBlockClient> = json::from_str(blocks_json_str).unwrap();
    let mut blocks: HashMap<_, _> = blocks
        .into_iter()
        .map(|block| (block.height.unwrap().to_string(), block))
        .collect();
    let client = native_client_for_test();
    NativeClientImpl::get_block.mock_safe(move |_, block_num| {
        let block = blocks.remove(&block_num).unwrap();
        MockResult::Return(Box::new(futures01::future::ok(block)))
    });

    let mtp = client.get_median_time_past(632858, KMD_MTP_BLOCK_COUNT).wait().unwrap();
    assert_eq!(1591173041, mtp);
}

#[test]
fn test_cashaddresses_in_tx_details_by_hash() {
    let conf = json!({
        "coin": "BCH",
        "pubtype": 0,
        "p2shtype": 5,
        "mm2": 1,
        "address_format":{"format":"cashaddress","network":"bchtest"},
    });
    let req = json!({
         "method": "electrum",
         "servers": [{"url":"blackie.c3-soft.com:60001"}, {"url":"bch0.kister.net:51001"}, {"url":"testnet.imaginary.cash:50001"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();

    let coin = unwrap!(block_on(utxo_standard_coin_from_conf_and_request(
        &ctx, "BCH", &conf, &req, &[1u8; 32]
    )));

    let hash = hex::decode("0f2f6e0c8f440c641895023782783426c3aca1acc78d7c0db7751995e8aa5751").unwrap();
    let fut = async {
        let tx_details = coin.tx_details_by_hash(&hash).await.unwrap();
        log!([tx_details]);

        assert!(tx_details
            .from
            .iter()
            .any(|addr| addr == "bchtest:qze8g4gx3z428jjcxzpycpxl7ke7d947gca2a7n2la"));
        assert!(tx_details
            .to
            .iter()
            .any(|addr| addr == "bchtest:qr39na5d25wdeecgw3euh9fkd4ygvd4pnsury96597"));
    };

    block_on(fut);
}

#[test]
fn test_address_from_str_with_cashaddress_activated() {
    let conf = json!({
        "coin": "BCH",
        "pubtype": 0,
        "p2shtype": 5,
        "mm2": 1,
        "address_format":{"format":"cashaddress","network":"bitcoincash"},
    });
    let req = json!({
         "method": "electrum",
         "servers": [{"url":"blackie.c3-soft.com:60001"}, {"url":"bch0.kister.net:51001"}, {"url":"testnet.imaginary.cash:50001"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();

    let coin = unwrap!(block_on(utxo_standard_coin_from_conf_and_request(
        &ctx, "BCH", &conf, &req, &[1u8; 32]
    )));

    assert_eq!(
        coin.address_from_str("bitcoincash:qzxqqt9lh4feptf0mplnk58gnajfepzwcq9f2rxk55"),
        Ok("1DmFp16U73RrVZtYUbo2Ectt8mAnYScpqM".into())
    );

    let error = coin
        .address_from_str("1DmFp16U73RrVZtYUbo2Ectt8mAnYScpqM")
        .err()
        .unwrap();
    assert!(error.contains("Cashaddress address format activated for BCH, but legacy format used instead"));

    // other error on parse
    let error = coin
        .address_from_str("bitcoincash:000000000000000000000000000000000000000000")
        .err()
        .unwrap();
    assert!(error.contains("Checksum verification failed"));
}

#[test]
fn test_address_from_str_with_legacy_address_activated() {
    let conf = json!({
        "coin": "BCH",
        "pubtype": 0,
        "p2shtype": 5,
        "mm2": 1,
    });
    let req = json!({
         "method": "electrum",
         "servers": [{"url":"blackie.c3-soft.com:60001"}, {"url":"bch0.kister.net:51001"}, {"url":"testnet.imaginary.cash:50001"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();

    let coin = unwrap!(block_on(utxo_standard_coin_from_conf_and_request(
        &ctx, "BCH", &conf, &req, &[1u8; 32]
    )));

    let expected = Address::from_cashaddress(
        "bitcoincash:qzxqqt9lh4feptf0mplnk58gnajfepzwcq9f2rxk55",
        coin.as_ref().checksum_type,
        coin.as_ref().pub_addr_prefix,
        coin.as_ref().p2sh_addr_prefix,
    )
    .unwrap();
    assert_eq!(
        coin.address_from_str("1DmFp16U73RrVZtYUbo2Ectt8mAnYScpqM"),
        Ok(expected)
    );

    let error = coin
        .address_from_str("bitcoincash:qzxqqt9lh4feptf0mplnk58gnajfepzwcq9f2rxk55")
        .err()
        .unwrap();
    assert!(error.contains("Legacy address format activated for BCH, but cashaddress format used instead"));

    // other error on parse
    let error = coin
        .address_from_str("0000000000000000000000000000000000")
        .err()
        .unwrap();
    assert!(error.contains("Invalid Address"));
}

#[test]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/673
fn test_network_info_negative_time_offset() {
    let info_str = r#"{"version":1140200,"subversion":"/Shibetoshi:1.14.2/","protocolversion":70015,"localservices":"0000000000000005","localrelay":true,"timeoffset":-1,"networkactive":true,"connections":12,"networks":[{"name":"ipv4","limited":false,"reachable":true,"proxy":"","proxy_randomize_credentials":false},{"name":"ipv6","limited":false,"reachable":true,"proxy":"","proxy_randomize_credentials":false},{"name":"onion","limited":false,"reachable":true,"proxy":"127.0.0.1:9050","proxy_randomize_credentials":true}],"relayfee":1.00000000,"incrementalfee":0.00001000,"localaddresses":[],"warnings":""}"#;
    let _info: NetworkInfo = json::from_str(&info_str).unwrap();
}

#[test]
fn test_unavailable_electrum_proto_version() {
    ElectrumClientImpl::new.mock_safe(|coin_ticker, event_handlers| {
        MockResult::Return(ElectrumClientImpl::with_protocol_version(
            coin_ticker,
            event_handlers,
            OrdRange::new(1.8, 1.9).unwrap(),
        ))
    });

    let conf = json!({"coin":"RICK","asset":"RICK","rpcport":8923});
    let req = json!({
         "method": "electrum",
         "servers": [{"url":"electrum1.cipig.net:10017"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let error = unwrap!(block_on(utxo_standard_coin_from_conf_and_request(
        &ctx, "RICK", &conf, &req, &[1u8; 32]
    ))
    .err());
    log!("Error: "(error));
    assert!(error.contains("There are no Electrums with the required protocol version"));
}

#[test]
#[ignore]
// The test provided to dimxy to recreate "stuck mempool" problem of komodod on RICK chain.
// Leaving this test here for a while because it might be still useful
fn test_spam_rick() {
    let conf = json!({"coin":"RICK","asset":"RICK","fname":"RICK (TESTCOIN)","rpcport":25435,"txversion":4,"overwintered":1,"mm2":1,"required_confirmations":1,"avg_blocktime":1,"protocol":{"type":"UTXO"}});
    let req = json!({
         "method": "enable",
         "coin": "RICK",
    });

    let key_pair = key_pair_from_seed("my_seed").unwrap();
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let coin = unwrap!(block_on(utxo_standard_coin_from_conf_and_request(
        &ctx,
        "RICK",
        &conf,
        &req,
        &*key_pair.private().secret
    )));

    let output = TransactionOutput {
        value: 1000000,
        script_pubkey: Builder::build_p2pkh(&coin.as_ref().my_address.hash).to_bytes(),
    };
    let mut futures = vec![];
    for _ in 0..5 {
        futures.push(send_outputs_from_my_address_impl(coin.clone(), vec![output.clone()]));
    }
    let results = block_on(join_all(futures));
    for result in results {
        unwrap!(result);
    }
}

#[test]
fn test_one_unavailable_electrum_proto_version() {
    ElectrumClientImpl::new.mock_safe(|coin_ticker, event_handlers| {
        MockResult::Return(ElectrumClientImpl::with_protocol_version(
            coin_ticker,
            event_handlers,
            OrdRange::new(1.4, 1.4).unwrap(),
        ))
    });

    // check if the electrum-mona.bitbank.cc:50001 doesn't support the protocol version 1.4
    let client = electrum_client_for_test(&["electrum-mona.bitbank.cc:50001"]);
    let result = client
        .server_version(
            "electrum-mona.bitbank.cc:50001",
            "AtomicDEX",
            &OrdRange::new(1.4, 1.4).unwrap(),
        )
        .wait();
    assert!(result
        .err()
        .unwrap()
        .to_string()
        .contains("unsupported protocol version"));

    drop(client);
    log!("Run BTC coin to test the server.version loop");

    let conf = json!({"coin":"BTC","asset":"BTC","rpcport":8332});
    let req = json!({
         "method": "electrum",
         // electrum-mona.bitbank.cc:50001 supports only 1.2 protocol version
         "servers": [{"url":"electrum1.cipig.net:10000"},{"url":"electrum-mona.bitbank.cc:50001"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let coin = unwrap!(block_on(utxo_standard_coin_from_conf_and_request(
        &ctx, "BTC", &conf, &req, &[1u8; 32]
    )));

    block_on(async { Timer::sleep(0.5).await });

    assert!(coin.as_ref().rpc_client.get_block_count().wait().is_ok());
}

#[test]
fn test_unspendable_balance_failed_once() {
    let mut unspents = vec![
        // unspendable balance (8) > balance (7.777)
        vec![
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 500000000,
                height: Default::default(),
            },
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 300000000,
                height: Default::default(),
            },
        ],
        // unspendable balance (7.777) == balance (7.777)
        vec![
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 333300000,
                height: Default::default(),
            },
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 444400000,
                height: Default::default(),
            },
        ],
    ];
    UtxoStandardCoin::ordered_mature_unspents.mock_safe(move |_, _| {
        let unspents = unspents.pop().unwrap();
        MockResult::Return(Box::new(futures01::future::ok(unspents)))
    });

    let conf = json!({"coin":"RICK","asset":"RICK","rpcport":8923});
    let req = json!({
        "method": "electrum",
        "servers": [{"url":"electrum1.cipig.net:10017"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();

    let priv_key = [
        184, 199, 116, 240, 113, 222, 8, 199, 253, 143, 98, 185, 127, 26, 87, 38, 246, 206, 159, 27, 207, 20, 27, 112,
        184, 102, 137, 37, 78, 214, 113, 78,
    ];
    let coin = unwrap!(block_on(utxo_standard_coin_from_conf_and_request(
        &ctx, "RICK", &conf, &req, &priv_key
    )));

    let balance = coin.my_balance().wait().unwrap();
    let expected = "7.777".parse().unwrap();
    assert_eq!(balance, expected);

    let unspendable_balance = coin.my_unspendable_balance().wait().unwrap();
    let expected = "0.000".parse().unwrap();
    assert_eq!(unspendable_balance, expected);
}

#[test]
fn test_unspendable_balance_failed() {
    UtxoStandardCoin::ordered_mature_unspents.mock_safe(move |_, _| {
        let unspents = vec![
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 500000000,
                height: Default::default(),
            },
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 300000000,
                height: Default::default(),
            },
        ];
        MockResult::Return(Box::new(futures01::future::ok(unspents)))
    });

    let conf = json!({"coin":"RICK","asset":"RICK","rpcport":8923});
    let req = json!({
        "method": "electrum",
        "servers": [{"url":"electrum1.cipig.net:10017"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();

    let priv_key = [
        184, 199, 116, 240, 113, 222, 8, 199, 253, 143, 98, 185, 127, 26, 87, 38, 246, 206, 159, 27, 207, 20, 27, 112,
        184, 102, 137, 37, 78, 214, 113, 78,
    ];
    let coin = unwrap!(block_on(utxo_standard_coin_from_conf_and_request(
        &ctx, "RICK", &conf, &req, &priv_key
    )));

    let balance = coin.my_balance().wait().unwrap();
    let expected = "7.777".parse().unwrap();
    assert_eq!(balance, expected);

    let error = coin.my_unspendable_balance().wait().err().unwrap();
    assert!(error.contains("spendable balance 8 more than total balance 7.777"));
}

#[test]
fn test_tx_history_path_colon_should_be_escaped_for_cash_address() {
    let mut coin = utxo_coin_fields_for_test(native_client_for_test().into(), None);
    coin.address_format = UtxoAddressFormat::CashAddress {
        network: "bitcoincash".into(),
    };
    let coin = utxo_coin_from_fields(coin);
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let path = coin.tx_history_path(&ctx);
    assert!(!path.display().to_string().contains(":"));
}

fn test_ordered_mature_unspents_from_cache_impl(
    unspent_height: Option<u64>,
    cached_height: Option<u64>,
    cached_confs: u32,
    block_count: u64,
    expected_height: Option<u64>,
    expected_confs: u32,
) {
    const TX_HASH: &str = "0a0fda88364b960000f445351fe7678317a1e0c80584de0413377ede00ba696f";
    let tx_hash: H256Json = hex::decode(TX_HASH).unwrap().as_slice().into();
    let client = electrum_client_for_test(&["electrum1.cipig.net:10017"]);
    let mut verbose = client.get_verbose_transaction(tx_hash.clone()).wait().unwrap();
    verbose.confirmations = cached_confs;
    verbose.height = cached_height;

    // prepare mocks
    UtxoStandardCoin::list_unspent_ordered.mock_safe(move |coin, _| {
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: H256::from_reversed_str(TX_HASH),
                index: 0,
            },
            value: 1000000000,
            height: unspent_height,
        }];
        MockResult::Return(Box::pin(futures::future::ok((
            unspents,
            block_on(coin.as_ref().recently_spent_outpoints.lock()),
        ))))
    });
    ElectrumClient::get_block_count
        .mock_safe(move |_| MockResult::Return(Box::new(futures01::future::ok(block_count))));
    UtxoStandardCoin::get_verbose_transaction_from_cache_or_rpc.mock_safe(move |_, txid| {
        assert_eq!(txid, tx_hash);
        MockResult::Return(Box::new(futures01::future::ok(VerboseTransactionFrom::Cache(
            verbose.clone(),
        ))))
    });
    static mut IS_UNSPENT_MATURE_CALLED: bool = false;
    UtxoStandardCoin::is_unspent_mature.mock_safe(move |_, tx: &RpcTransaction| {
        // check if the transaction height and confirmations are expected
        assert_eq!(tx.height, expected_height);
        assert_eq!(tx.confirmations, expected_confs);
        unsafe { IS_UNSPENT_MATURE_CALLED = true }
        MockResult::Return(false)
    });

    // run test
    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Electrum(client), None);
    let unspents = coin
        .ordered_mature_unspents(&Address::from("R9o9xTocqr6CeEDGDH6mEYpwLoMz6jNjMW"))
        .wait()
        .expect("Expected an empty unspent list");
    // unspents should be empty because `is_unspent_mature()` always returns false
    assert!(unspents.is_empty());
    assert!(unsafe { IS_UNSPENT_MATURE_CALLED == true });
}

#[test]
fn test_ordered_mature_unspents_from_cache() {
    let unspent_height = None;
    let cached_height = None;
    let cached_confs = 0;
    let block_count = 1000;
    let expected_height = None; // is unknown
    let expected_confs = 0; // is not changed because height is unknown
    test_ordered_mature_unspents_from_cache_impl(
        unspent_height,
        cached_height,
        cached_confs,
        block_count,
        expected_height,
        expected_confs,
    );

    let unspent_height = None;
    let cached_height = None;
    let cached_confs = 5;
    let block_count = 1000;
    let expected_height = None; // is unknown
    let expected_confs = 5; // is not changed because height is unknown
    test_ordered_mature_unspents_from_cache_impl(
        unspent_height,
        cached_height,
        cached_confs,
        block_count,
        expected_height,
        expected_confs,
    );

    let unspent_height = Some(998);
    let cached_height = None;
    let cached_confs = 0;
    let block_count = 1000;
    let expected_height = Some(998); // as the unspent_height
    let expected_confs = 3; // 1000 - 998 + 1
    test_ordered_mature_unspents_from_cache_impl(
        unspent_height,
        cached_height,
        cached_confs,
        block_count,
        expected_height,
        expected_confs,
    );

    let unspent_height = None;
    let cached_height = Some(998);
    let cached_confs = 0;
    let block_count = 1000;
    let expected_height = Some(998); // as the cached_height
    let expected_confs = 3; // 1000 - 998 + 1
    test_ordered_mature_unspents_from_cache_impl(
        unspent_height,
        cached_height,
        cached_confs,
        block_count,
        expected_height,
        expected_confs,
    );

    let unspent_height = Some(998);
    let cached_height = Some(997);
    let cached_confs = 0;
    let block_count = 1000;
    let expected_height = Some(998); // as the unspent_height
    let expected_confs = 3; // 1000 - 998 + 1
    test_ordered_mature_unspents_from_cache_impl(
        unspent_height,
        cached_height,
        cached_confs,
        block_count,
        expected_height,
        expected_confs,
    );

    // block_count < tx_height
    let unspent_height = None;
    let cached_height = Some(1000);
    let cached_confs = 1;
    let block_count = 999;
    let expected_height = Some(1000); // as the cached_height
    let expected_confs = 1; // is not changed because height cannot be calculated
    test_ordered_mature_unspents_from_cache_impl(
        unspent_height,
        cached_height,
        cached_confs,
        block_count,
        expected_height,
        expected_confs,
    );

    // block_count == tx_height
    let unspent_height = None;
    let cached_height = Some(1000);
    let cached_confs = 1;
    let block_count = 1000;
    let expected_height = Some(1000); // as the cached_height
    let expected_confs = 1; // 1000 - 1000 + 1
    test_ordered_mature_unspents_from_cache_impl(
        unspent_height,
        cached_height,
        cached_confs,
        block_count,
        expected_height,
        expected_confs,
    );

    // tx_height == 0
    let unspent_height = Some(0);
    let cached_height = None;
    let cached_confs = 1;
    let block_count = 1000;
    let expected_height = Some(0); // as the cached_height
    let expected_confs = 1; // is not changed because tx_height is expected to be not zero
    test_ordered_mature_unspents_from_cache_impl(
        unspent_height,
        cached_height,
        cached_confs,
        block_count,
        expected_height,
        expected_confs,
    );
}

#[test]
fn test_native_client_unspents_filtered_using_tx_cache_single_tx_in_cache() {
    let client = native_client_for_test();
    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None);

    let address: Address = "RGfFZaaNV68uVe1uMf6Y37Y8E1i2SyYZBN".into();
    block_on(coin.as_ref().recently_spent_outpoints.lock()).for_script_pubkey =
        Builder::build_p2pkh(&address.hash).to_bytes();

    // https://morty.explorer.dexstats.info/tx/31c7aaae89ab1c39febae164a3190a86ed7c6c6f8c9dc98ec28d508b7929d347
    let tx: UtxoTx = "0400008085202f89027f57730fcbbc2c72fb18bcc3766a713044831a117bb1cade3ed88644864f7333020000006a47304402206e3737b2fcf078b61b16fa67340cc3e79c5d5e2dc9ffda09608371552a3887450220460a332aa1b8ad8f2de92d319666f70751078b221199951f80265b4f7cef8543012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff42b916a80430b80a77e114445b08cf120735447a524de10742fac8f6a9d4170f000000006a473044022004aa053edafb9d161ea8146e0c21ed1593aa6b9404dd44294bcdf920a1695fd902202365eac15dbcc5e9f83e2eed56a8f2f0e5aded36206f9c3fabc668fd4665fa2d012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff03547b16000000000017a9143e8ad0e2bf573d32cb0b3d3a304d9ebcd0c2023b870000000000000000166a144e2b3c0323ab3c2dc6f86dc5ec0729f11e42f56103970400000000001976a91450f4f098306f988d8843004689fae28c83ef16e888ac89c5925f000000000000000000000000000000".into();
    let spent_by_tx = vec![
        UnspentInfo {
            outpoint: tx.inputs[0].previous_output.clone(),
            value: 886737,
            height: Some(642293),
        },
        UnspentInfo {
            outpoint: tx.inputs[1].previous_output.clone(),
            value: 88843,
            height: Some(642293),
        },
    ];

    block_on(coin.as_ref().recently_spent_outpoints.lock()).add_spent(
        spent_by_tx.clone(),
        tx.hash(),
        tx.outputs.clone(),
    );
    NativeClient::list_unspent
        .mock_safe(move |_, _, _| MockResult::Return(Box::new(futures01::future::ok(spent_by_tx.clone()))));

    let address: Address = "RGfFZaaNV68uVe1uMf6Y37Y8E1i2SyYZBN".into();
    let (unspents_ordered, _) = block_on(coin.list_unspent_ordered(&address)).unwrap();
    // output 2 is change so it must be returned
    let expected_unspent = UnspentInfo {
        outpoint: OutPoint {
            hash: tx.hash(),
            index: 2,
        },
        value: tx.outputs[2].value,
        height: None,
    };
    assert_eq!(vec![expected_unspent], unspents_ordered);
}

#[test]
fn test_native_client_unspents_filtered_using_tx_cache_single_several_chained_txs_in_cache() {
    let client = native_client_for_test();
    let coin = utxo_coin_fields_for_test(UtxoRpcClientEnum::Native(client), None);

    let address: Address = "RGfFZaaNV68uVe1uMf6Y37Y8E1i2SyYZBN".into();
    block_on(coin.recently_spent_outpoints.lock()).for_script_pubkey = Builder::build_p2pkh(&address.hash).to_bytes();
    let coin = utxo_coin_from_fields(coin);

    // https://morty.explorer.dexstats.info/tx/31c7aaae89ab1c39febae164a3190a86ed7c6c6f8c9dc98ec28d508b7929d347
    let tx_0: UtxoTx = "0400008085202f89027f57730fcbbc2c72fb18bcc3766a713044831a117bb1cade3ed88644864f7333020000006a47304402206e3737b2fcf078b61b16fa67340cc3e79c5d5e2dc9ffda09608371552a3887450220460a332aa1b8ad8f2de92d319666f70751078b221199951f80265b4f7cef8543012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff42b916a80430b80a77e114445b08cf120735447a524de10742fac8f6a9d4170f000000006a473044022004aa053edafb9d161ea8146e0c21ed1593aa6b9404dd44294bcdf920a1695fd902202365eac15dbcc5e9f83e2eed56a8f2f0e5aded36206f9c3fabc668fd4665fa2d012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff03547b16000000000017a9143e8ad0e2bf573d32cb0b3d3a304d9ebcd0c2023b870000000000000000166a144e2b3c0323ab3c2dc6f86dc5ec0729f11e42f56103970400000000001976a91450f4f098306f988d8843004689fae28c83ef16e888ac89c5925f000000000000000000000000000000".into();
    let spent_by_tx_0 = vec![
        UnspentInfo {
            outpoint: tx_0.inputs[0].previous_output.clone(),
            value: 886737,
            height: Some(642293),
        },
        UnspentInfo {
            outpoint: tx_0.inputs[1].previous_output.clone(),
            value: 88843,
            height: Some(642293),
        },
    ];
    block_on(coin.as_ref().recently_spent_outpoints.lock()).add_spent(
        spent_by_tx_0.clone(),
        tx_0.hash(),
        tx_0.outputs.clone(),
    );

    // https://morty.explorer.dexstats.info/tx/dbfc821e482747a3512ee6d5734f9df2aa73dab07e2fcd86abeadb462e795bf9
    let tx_1: UtxoTx = "0400008085202f890347d329798b508dc28ec99d8c6f6c7ced860a19a364e1bafe391cab89aeaac731020000006a47304402203ea8b380d0a7e64348869ef7c4c2bfa966fc7b148633003332fa8d0ab0c1bc5602202cc63fabdd2a6578c52d8f4f549069b16505f2ead48edc2b8de299be15aadf9a012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff1d1fd3a6b01710647a7f4a08c6de6075cb8e78d5069fa50f10c4a2a10ded2a95000000006a47304402203868945edc0f6dc2ee43d70a69ee4ec46ca188dc493173ce58924ba9bf6ee7a50220648ff99ce458ca72800758f6a1bd3800cd05ff9c3122f23f3653c25e09d22c79012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff7932150df8b4a1852b8b84b89b0d5322bf74665fb7f76a728369fd6895d3fd48000000006a4730440220127918c6f79c11f7f2376a6f3b750ed4c7103183181ad1218afcb2625ece9599022028c05e88d3a2f97cebd84a718cda33b62b48b18f16278fa8e531fd2155e61ee8012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff0329fd12000000000017a914cafb62e3e8bdb8db3735c39b92743ac6ebc9ef20870000000000000000166a14a7416b070c9bb98f4bafae55616f005a2a30bd6014b40c00000000001976a91450f4f098306f988d8843004689fae28c83ef16e888ac8cc5925f000000000000000000000000000000".into();
    let spent_by_tx_1 = vec![
        UnspentInfo {
            outpoint: tx_1.inputs[0].previous_output.clone(),
            value: 300803,
            height: Some(642293),
        },
        UnspentInfo {
            outpoint: tx_1.inputs[1].previous_output.clone(),
            value: 888544,
            height: Some(642293),
        },
        UnspentInfo {
            outpoint: tx_1.inputs[2].previous_output.clone(),
            value: 888642,
            height: Some(642293),
        },
    ];
    block_on(coin.as_ref().recently_spent_outpoints.lock()).add_spent(
        spent_by_tx_1.clone(),
        tx_1.hash(),
        tx_1.outputs.clone(),
    );
    // https://morty.explorer.dexstats.info/tx/12ea22a7cde9efb66b76f9b84345ddfc4c34870e293bfa8eac68d7df83dffa4b
    let tx_2: UtxoTx = "0400008085202f8902f95b792e46dbeaab86cd2f7eb0da73aaf29d4f73d5e62e51a34727481e82fcdb020000006a4730440220347adefe33ed5afbbb8e5d453afd527319f9a50ab790023296a981da095ca4a2022029a68ef6fd5a4decf3793d4c33994eb8658408f3b14a6d439c4753b2dde954ee012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff75bd4348594f8ff2a216e5ad7533b37d47d2a2767b0b88d43972ad51895355e2000000006a473044022069b36c0f65d56e02bc179f7442806374c4163d07939090aba1da736abad9a77d022006dc39adf48e02033ae9d4a48540752ae3b3841e3ec60d2e86dececb88b9e518012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff03414111000000000017a914a153024c826a3a42c2e501eca5d7dacd3fc59976870000000000000000166a14db0e6f4d418d68dce8e5beb26cc5078e01e2e3ace2fe0800000000001976a91450f4f098306f988d8843004689fae28c83ef16e888ac8fc5925f000000000000000000000000000000".into();
    let spent_by_tx_2 = vec![
        UnspentInfo {
            outpoint: tx_2.inputs[0].previous_output.clone(),
            value: 832532,
            height: Some(642293),
        },
        UnspentInfo {
            outpoint: tx_2.inputs[1].previous_output.clone(),
            value: 888823,
            height: Some(642293),
        },
    ];
    block_on(coin.as_ref().recently_spent_outpoints.lock()).add_spent(
        spent_by_tx_2.clone(),
        tx_2.hash(),
        tx_2.outputs.clone(),
    );

    let mut unspents_to_return = spent_by_tx_0;
    unspents_to_return.extend(spent_by_tx_1);
    unspents_to_return.extend(spent_by_tx_2);

    NativeClient::list_unspent
        .mock_safe(move |_, _, _| MockResult::Return(Box::new(futures01::future::ok(unspents_to_return.clone()))));

    let (unspents_ordered, _) = block_on(coin.list_unspent_ordered(&address)).unwrap();

    // output 2 is change so it must be returned
    let expected_unspent = UnspentInfo {
        outpoint: OutPoint {
            hash: tx_2.hash(),
            index: 2,
        },
        value: tx_2.outputs[2].value,
        height: None,
    };
    assert_eq!(vec![expected_unspent], unspents_ordered);
}

#[test]
fn validate_address_res_format() {
    let btc_017_and_above_response = json!({
      "isvalid": true,
      "address": "1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1",
      "scriptPubKey": "76a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac",
      "isscript": false,
      "iswitness": false
    });

    let _: ValidateAddressRes = json::from_value(btc_017_and_above_response).unwrap();

    let btc_016_response = json!({
      "isvalid": true,
      "address": "RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd",
      "scriptPubKey": "76a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac",
      "ismine": false,
      "iswatchonly": true,
      "isscript": false,
      "account": "RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd",
      "timestamp": 0
    });

    let _: ValidateAddressRes = json::from_value(btc_016_response).unwrap();
}

#[test]
fn get_address_info_format() {
    let response = json!({
      "address": "Ld6814QT6fyChvvX3gmhNHbRDyiMBvPr9s",
      "scriptPubKey": "76a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac",
      "ismine": false,
      "solvable": false,
      "iswatchonly": true,
      "isscript": false,
      "iswitness": false,
      "label": "Ld6814QT6fyChvvX3gmhNHbRDyiMBvPr9s",
      "ischange": false,
      "timestamp": 0,
      "labels": [
        {
          "name": "Ld6814QT6fyChvvX3gmhNHbRDyiMBvPr9s",
          "purpose": "receive"
        }
      ]
    });

    let _: GetAddressInfoRes = json::from_value(response).unwrap();
}

#[test]
fn test_native_is_address_imported_validate_address_is_mine() {
    let client = native_client_for_test();
    NativeClientImpl::validate_address.mock_safe(|_, _| {
        let result = ValidateAddressRes {
            is_valid: false,
            address: "".to_string(),
            script_pub_key: Default::default(),
            seg_id: None,
            is_mine: Some(true),
            is_watch_only: Some(false),
            is_script: false,
            account: None,
        };
        MockResult::Return(Box::new(futures01::future::ok(result)))
    });

    let address = "";
    let imported = block_on(client.is_address_imported(address)).unwrap();
    assert!(imported);
}

#[test]
fn test_native_is_address_imported_validate_address_is_watch_only() {
    let client = native_client_for_test();
    NativeClientImpl::validate_address.mock_safe(|_, _| {
        let result = ValidateAddressRes {
            is_valid: false,
            address: "".to_string(),
            script_pub_key: Default::default(),
            seg_id: None,
            is_mine: Some(false),
            is_watch_only: Some(true),
            is_script: false,
            account: None,
        };
        MockResult::Return(Box::new(futures01::future::ok(result)))
    });

    let address = "";
    let imported = block_on(client.is_address_imported(address)).unwrap();
    assert!(imported);
}

#[test]
fn test_native_is_address_imported_validate_address_false() {
    let client = native_client_for_test();
    NativeClientImpl::validate_address.mock_safe(|_, _| {
        let result = ValidateAddressRes {
            is_valid: false,
            address: "".to_string(),
            script_pub_key: Default::default(),
            seg_id: None,
            is_mine: Some(false),
            is_watch_only: Some(false),
            is_script: false,
            account: None,
        };
        MockResult::Return(Box::new(futures01::future::ok(result)))
    });

    let address = "";
    let imported = block_on(client.is_address_imported(address)).unwrap();
    assert!(!imported);
}

#[test]
fn test_native_is_address_imported_fallback_to_address_info_is_mine() {
    let client = native_client_for_test();
    NativeClientImpl::validate_address.mock_safe(|_, _| {
        let result = ValidateAddressRes {
            is_valid: false,
            address: "".to_string(),
            script_pub_key: Default::default(),
            seg_id: None,
            is_mine: None,
            is_watch_only: None,
            is_script: false,
            account: None,
        };
        MockResult::Return(Box::new(futures01::future::ok(result)))
    });

    NativeClientImpl::get_address_info.mock_safe(|_, _| {
        let result = GetAddressInfoRes {
            is_mine: true,
            is_watch_only: false,
        };
        MockResult::Return(Box::new(futures01::future::ok(result)))
    });

    let address = "";
    let imported = block_on(client.is_address_imported(address)).unwrap();
    assert!(imported);
}

#[test]
fn test_native_is_address_imported_fallback_to_address_info_is_watch_only() {
    let client = native_client_for_test();
    NativeClientImpl::validate_address.mock_safe(|_, _| {
        let result = ValidateAddressRes {
            is_valid: false,
            address: "".to_string(),
            script_pub_key: Default::default(),
            seg_id: None,
            is_mine: None,
            is_watch_only: None,
            is_script: false,
            account: None,
        };
        MockResult::Return(Box::new(futures01::future::ok(result)))
    });

    NativeClientImpl::get_address_info.mock_safe(|_, _| {
        let result = GetAddressInfoRes {
            is_mine: false,
            is_watch_only: true,
        };
        MockResult::Return(Box::new(futures01::future::ok(result)))
    });

    let address = "";
    let imported = block_on(client.is_address_imported(address)).unwrap();
    assert!(imported);
}

#[test]
fn test_native_is_address_imported_fallback_to_address_info_false() {
    let client = native_client_for_test();
    NativeClientImpl::validate_address.mock_safe(|_, _| {
        let result = ValidateAddressRes {
            is_valid: false,
            address: "".to_string(),
            script_pub_key: Default::default(),
            seg_id: None,
            is_mine: None,
            is_watch_only: None,
            is_script: false,
            account: None,
        };
        MockResult::Return(Box::new(futures01::future::ok(result)))
    });

    NativeClientImpl::get_address_info.mock_safe(|_, _| {
        let result = GetAddressInfoRes {
            is_mine: false,
            is_watch_only: false,
        };
        MockResult::Return(Box::new(futures01::future::ok(result)))
    });

    let address = "";
    let imported = block_on(client.is_address_imported(address)).unwrap();
    assert!(!imported);
}

/// Test if the [`NativeClient::find_output_spend`] handle the conflicting transactions correctly.
/// https://github.com/KomodoPlatform/atomicDEX-API/pull/775
#[test]
fn test_find_output_spend_skips_conflicting_transactions() {
    const LIST_SINCE_BLOCK_JSON: &str = r#"{"transactions":[{"involvesWatchonly":true,"account":"","address":"RAsbVN52LC2hEp3UWWSLbV8pJ8CneKjW9F","category":"send","amount":-0.01537462,"vout":0,"fee":-0.00001000,"rawconfirmations":-1,"confirmations":-1,"txid":"220c337006b2581c3da734ef9f1106601e8538ebab823d0dd6719a4d4580fd04","walletconflicts":["a2144bee4eac4b41ab1aed2dd8f854785b3ddebd617d48696dd84e62d129544b"],"time":1607831631,"timereceived":1607831631,"vjoinsplit":[],"size":320},{"involvesWatchonly":true,"account":"","address":"RAsbVN52LC2hEp3UWWSLbV8pJ8CneKjW9F","category":"send","amount":-0.01537462,"vout":0,"fee":-0.00001000,"rawconfirmations":-1,"confirmations":-1,"txid":"6fb83afb1bf309515fa429814bf07552eea951656fdee913f3aa687d513cd720","walletconflicts":["4aad6471f59e5912349cd7679bc029bfbd5da54d34c235d20500249f98f549e4"],"time":1607831556,"timereceived":1607831556,"vjoinsplit":[],"size":320},{"account":"","address":"RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd","category":"receive","amount":0.54623851,"vout":2,"rawconfirmations":1617,"confirmations":1617,"blockhash":"000000000c33a387d73180220a5a8f2fe6081bad9bdfc0dba5a9985abcee8294","blockindex":7,"blocktime":1607957613,"expiryheight":0,"txid":"45e4900a2b330800a356a74ce2a97370596ad3a25e689e3ed5c36e421d12bbf7","walletconflicts":[],"time":1607957175,"timereceived":1607957175,"vjoinsplit":[],"size":567},{"involvesWatchonly":true,"account":"","address":"RT9MpMyucqXiX8bZLimXBnrrn2ofmdGNKd","category":"send","amount":-0.00797200,"vout":0,"fee":-0.00001000,"rawconfirmations":-1,"confirmations":-1,"txid":"bfc99c06d1a060cdbeba05620dc1c6fdb7351eb4c04b7aae578688ca6aeaeafd","walletconflicts":[],"time":1607957792,"timereceived":1607957792,"vjoinsplit":[],"size":286}],"lastblock":"06082d363f78174fd13b126994210d3c3ad9d073ee3983ad59fe8b76e6e3e071"}"#;
    // in the json above this transaction is only one not conflicting
    const NON_CONFLICTING_TXID: &str = "45e4900a2b330800a356a74ce2a97370596ad3a25e689e3ed5c36e421d12bbf7";
    let expected_txid: H256Json = hex::decode(NON_CONFLICTING_TXID).unwrap().as_slice().into();

    NativeClientImpl::get_block_hash.mock_safe(|_, _| {
        // no matter what we return here
        let blockhash: H256Json = hex::decode("000000000c33a387d73180220a5a8f2fe6081bad9bdfc0dba5a9985abcee8294")
            .unwrap()
            .as_slice()
            .into();
        MockResult::Return(Box::new(futures01::future::ok(blockhash)))
    });

    NativeClientImpl::list_since_block.mock_safe(|_, _| {
        let listsinceblockres: ListSinceBlockRes =
            json::from_str(LIST_SINCE_BLOCK_JSON).expect("Json is expected to be valid");
        MockResult::Return(Box::new(futures01::future::ok(listsinceblockres)))
    });

    static mut GET_RAW_TRANSACTION_BYTES_CALLED: usize = 0;
    NativeClientImpl::get_raw_transaction_bytes.mock_safe(move |_, txid| {
        unsafe { GET_RAW_TRANSACTION_BYTES_CALLED += 1 };
        assert_eq!(txid, expected_txid);
        // no matter what we return here
        let bytes: BytesJson = hex::decode("0400008085202f890347d329798b508dc28ec99d8c6f6c7ced860a19a364e1bafe391cab89aeaac731020000006a47304402203ea8b380d0a7e64348869ef7c4c2bfa966fc7b148633003332fa8d0ab0c1bc5602202cc63fabdd2a6578c52d8f4f549069b16505f2ead48edc2b8de299be15aadf9a012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff1d1fd3a6b01710647a7f4a08c6de6075cb8e78d5069fa50f10c4a2a10ded2a95000000006a47304402203868945edc0f6dc2ee43d70a69ee4ec46ca188dc493173ce58924ba9bf6ee7a50220648ff99ce458ca72800758f6a1bd3800cd05ff9c3122f23f3653c25e09d22c79012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff7932150df8b4a1852b8b84b89b0d5322bf74665fb7f76a728369fd6895d3fd48000000006a4730440220127918c6f79c11f7f2376a6f3b750ed4c7103183181ad1218afcb2625ece9599022028c05e88d3a2f97cebd84a718cda33b62b48b18f16278fa8e531fd2155e61ee8012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff0329fd12000000000017a914cafb62e3e8bdb8db3735c39b92743ac6ebc9ef20870000000000000000166a14a7416b070c9bb98f4bafae55616f005a2a30bd6014b40c00000000001976a91450f4f098306f988d8843004689fae28c83ef16e888ac8cc5925f000000000000000000000000000000").unwrap().into();
        MockResult::Return(Box::new(futures01::future::ok(bytes)))
    });
    let client = native_client_for_test();

    // no matter what arguments we will pass to the function because of the mocks above
    let tx: UtxoTx = "0400008085202f89027f57730fcbbc2c72fb18bcc3766a713044831a117bb1cade3ed88644864f7333020000006a47304402206e3737b2fcf078b61b16fa67340cc3e79c5d5e2dc9ffda09608371552a3887450220460a332aa1b8ad8f2de92d319666f70751078b221199951f80265b4f7cef8543012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff42b916a80430b80a77e114445b08cf120735447a524de10742fac8f6a9d4170f000000006a473044022004aa053edafb9d161ea8146e0c21ed1593aa6b9404dd44294bcdf920a1695fd902202365eac15dbcc5e9f83e2eed56a8f2f0e5aded36206f9c3fabc668fd4665fa2d012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff03547b16000000000017a9143e8ad0e2bf573d32cb0b3d3a304d9ebcd0c2023b870000000000000000166a144e2b3c0323ab3c2dc6f86dc5ec0729f11e42f56103970400000000001976a91450f4f098306f988d8843004689fae28c83ef16e888ac89c5925f000000000000000000000000000000".into();
    let vout = 0;
    let from_block = 0;
    let actual = client.find_output_spend(&tx, vout, from_block).wait();
    assert_eq!(actual, Ok(None));
    assert_eq!(unsafe { GET_RAW_TRANSACTION_BYTES_CALLED }, 1);
}

#[test]
fn test_qtum_is_unspent_mature() {
    use crate::utxo::qtum::{QtumBasedCoin, QtumCoin};
    use rpc::v1::types::{ScriptType, SignedTransactionOutput, TransactionOutputScript};

    let mut coin_fields = utxo_coin_fields_for_test(UtxoRpcClientEnum::Native(native_client_for_test()), None);
    // Qtum's mature confirmations is 500 blocks
    coin_fields.mature_confirmations = 500;
    let arc: UtxoArc = coin_fields.into();
    let coin = QtumCoin::from(arc);

    let empty_output = SignedTransactionOutput {
        value: 0.,
        n: 0,
        script: TransactionOutputScript {
            asm: "".into(),
            hex: "".into(),
            req_sigs: 0,
            script_type: ScriptType::NonStandard,
            addresses: vec![],
        },
    };
    let real_output = SignedTransactionOutput {
        value: 117.02430015,
        n: 1,
        script: TransactionOutputScript {
            asm: "03e71b9c152bb233ddfe58f20056715c51b054a1823e0aba108e6f1cea0ceb89c8 OP_CHECKSIG".into(),
            hex: "2103e71b9c152bb233ddfe58f20056715c51b054a1823e0aba108e6f1cea0ceb89c8ac".into(),
            req_sigs: 0,
            script_type: ScriptType::PubKey,
            addresses: vec![],
        },
    };

    let mut tx = RpcTransaction {
        hex: Default::default(),
        txid: "47d983175720ba2a67f36d0e1115a129351a2f340bdde6ecb6d6029e138fe920".into(),
        hash: None,
        size: Default::default(),
        vsize: Default::default(),
        version: 2,
        locktime: 0,
        vin: vec![],
        vout: vec![empty_output, real_output],
        blockhash: "c23882939ff695be36546ea998eb585e962b043396e4d91959477b9796ceb9e1".into(),
        confirmations: 421,
        rawconfirmations: None,
        time: 1590671504,
        blocktime: 1590671504,
        height: None,
    };

    // output is coinbase and has confirmations < QTUM_MATURE_CONFIRMATIONS
    assert!(!coin.is_qtum_unspent_mature(&tx));

    tx.confirmations = 501;
    // output is coinbase but has confirmations > QTUM_MATURE_CONFIRMATIONS
    assert!(coin.is_qtum_unspent_mature(&tx));

    tx.confirmations = 421;
    // remove empty output
    tx.vout.remove(0);
    // output is not coinbase
    assert!(coin.is_qtum_unspent_mature(&tx));
}
