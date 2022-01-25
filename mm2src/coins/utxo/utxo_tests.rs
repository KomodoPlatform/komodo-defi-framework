use super::*;
use crate::coin_balance::{CheckHDAccountBalanceParams, CheckHDAccountBalanceResponse, HDAccountBalanceParams,
                          HDAccountBalanceResponse, HDAddressBalance, HDWalletBalanceRpcOps};
use crate::utxo::qtum::{qtum_coin_with_priv_key, QtumCoin, QtumDelegationOps, QtumDelegationRequest};
use crate::utxo::rpc_clients::{BlockHashOrHeight, ElectrumClient, ElectrumClientImpl, GetAddressInfoRes,
                               ListSinceBlockRes, ListTransactionsItem, NativeClient, NativeClientImpl, NetworkInfo,
                               UtxoRpcClientOps, ValidateAddressRes, VerboseBlock};
use crate::utxo::utxo_builder::{UtxoArcWithIguanaPrivKeyBuilder, UtxoCoinBuilderCommonOps};
use crate::utxo::utxo_common::UtxoTxBuilder;
use crate::utxo::utxo_standard::{utxo_standard_coin_with_priv_key, UtxoStandardCoin};
#[cfg(not(target_arch = "wasm32"))] use crate::WithdrawFee;
use crate::{Bip44Chain, CoinBalance, StakingInfosDetails, SwapOps, TradePreimageValue, TxFeeDetails};
use bigdecimal::{BigDecimal, Signed};
use chain::OutPoint;
use common::executor::Timer;
use common::mm_ctx::MmCtxBuilder;
use common::privkey::key_pair_from_seed;
use common::{block_on, now_ms, OrdRange, PagingOptionsEnum, DEX_FEE_ADDR_RAW_PUBKEY};
use crypto::RpcDerivationPath;
use futures::future::join_all;
use futures::TryFutureExt;
use mocktopus::mocking::*;
use rpc::v1::types::H256 as H256Json;
use serialization::{deserialize, CoinVariant};
use std::num::NonZeroUsize;

const TEST_COIN_NAME: &'static str = "RICK";
// Made-up hrp for rick to test p2wpkh script
const TEST_COIN_HRP: &'static str = "rck";
const RICK_ELECTRUM_ADDRS: &[&'static str] = &[
    "electrum1.cipig.net:10017",
    "electrum2.cipig.net:10017",
    "electrum3.cipig.net:10017",
];
const TEST_COIN_DECIMALS: u8 = 8;

pub fn electrum_client_for_test(servers: &[&str]) -> ElectrumClient {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let servers: Vec<_> = servers.iter().map(|server| json!({ "url": server })).collect();
    let req = json!({
        "method": "electrum",
        "servers": servers,
    });
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let builder =
        UtxoArcWithIguanaPrivKeyBuilder::new(&ctx, TEST_COIN_NAME, &Json::Null, &params, &[], UtxoStandardCoin::from);
    let args = ElectrumBuilderArgs {
        spawn_ping: false,
        negotiate_version: true,
        collect_metrics: false,
    };

    let servers = servers.into_iter().map(|s| json::from_value(s).unwrap()).collect();
    block_on(builder.electrum_client(args, servers)).unwrap()
}

/// Returned client won't work by default, requires some mocks to be usable
#[cfg(not(target_arch = "wasm32"))]
fn native_client_for_test() -> NativeClient { NativeClient(Arc::new(NativeClientImpl::default())) }

fn utxo_coin_fields_for_test(
    rpc_client: UtxoRpcClientEnum,
    force_seed: Option<&str>,
    is_segwit_coin: bool,
) -> UtxoCoinFields {
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
        hash: key_pair.public().address_hash().into(),
        t_addr_prefix: 0,
        checksum_type,
        hrp: None,
        addr_format: UtxoAddressFormat::Standard,
    };
    let my_script_pubkey = Builder::build_p2pkh(&my_address.hash).to_bytes();

    let priv_key_policy = PrivKeyPolicy::KeyPair(key_pair);
    let derivation_method = DerivationMethod::Iguana(my_address);

    let bech32_hrp = if is_segwit_coin {
        Some(TEST_COIN_HRP.to_string())
    } else {
        None
    };

    UtxoCoinFields {
        conf: UtxoCoinConf {
            is_pos: false,
            requires_notarization: false.into(),
            overwintered: true,
            segwit: true,
            tx_version: 4,
            default_address_format: UtxoAddressFormat::Standard,
            asset_chain: true,
            p2sh_addr_prefix: 85,
            p2sh_t_addr_prefix: 0,
            pub_addr_prefix: 60,
            pub_t_addr_prefix: 0,
            bech32_hrp,
            ticker: TEST_COIN_NAME.into(),
            wif_prefix: 0,
            tx_fee_volatility_percent: DEFAULT_DYNAMIC_FEE_VOLATILITY_PERCENT,
            version_group_id: 0x892f2085,
            consensus_branch_id: 0x76b809bb,
            zcash: true,
            checksum_type,
            fork_id: 0,
            signature_version: SignatureVersion::Base,
            required_confirmations: 1.into(),
            force_min_relay_fee: false,
            mtp_block_count: NonZeroU64::new(11).unwrap(),
            estimate_fee_mode: None,
            mature_confirmations: MATURE_CONFIRMATIONS_DEFAULT,
            estimate_fee_blocks: 1,
            lightning: false,
            network: None,
            trezor_coin: None,
        },
        decimals: TEST_COIN_DECIMALS,
        dust_amount: UTXO_DUST_AMOUNT,
        tx_fee: TxFee::FixedPerKb(1000),
        rpc_client,
        priv_key_policy,
        derivation_method,
        history_sync_state: Mutex::new(HistorySyncState::NotEnabled),
        tx_cache_directory: None,
        recently_spent_outpoints: AsyncMutex::new(RecentlySpentOutPoints::new(my_script_pubkey)),
        tx_hash_algo: TxHashAlgo::DSHA256,
        check_utxo_maturity: false,
    }
}

fn utxo_coin_from_fields(coin: UtxoCoinFields) -> UtxoStandardCoin {
    let arc: UtxoArc = coin.into();
    arc.into()
}

fn utxo_coin_for_test(
    rpc_client: UtxoRpcClientEnum,
    force_seed: Option<&str>,
    is_segwit_coin: bool,
) -> UtxoStandardCoin {
    utxo_coin_from_fields(utxo_coin_fields_for_test(rpc_client, force_seed, is_segwit_coin))
}

#[test]
fn test_extract_secret() {
    let client = electrum_client_for_test(RICK_ELECTRUM_ADDRS);
    let coin = utxo_coin_for_test(client.into(), None, false);

    let tx_hex = hex::decode("0100000001de7aa8d29524906b2b54ee2e0281f3607f75662cbc9080df81d1047b78e21dbc00000000d7473044022079b6c50820040b1fbbe9251ced32ab334d33830f6f8d0bf0a40c7f1336b67d5b0220142ccf723ddabb34e542ed65c395abc1fbf5b6c3e730396f15d25c49b668a1a401209da937e5609680cb30bff4a7661364ca1d1851c2506fa80c443f00a3d3bf7365004c6b6304f62b0e5cb175210270e75970bb20029b3879ec76c4acd320a8d0589e003636264d01a7d566504bfbac6782012088a9142fb610d856c19fd57f2d0cffe8dff689074b3d8a882103f368228456c940ac113e53dad5c104cf209f2f102a409207269383b6ab9b03deac68ffffffff01d0dc9800000000001976a9146d9d2b554d768232320587df75c4338ecc8bf37d88ac40280e5c").unwrap();
    let expected_secret = hex::decode("9da937e5609680cb30bff4a7661364ca1d1851c2506fa80c443f00a3d3bf7365").unwrap();
    let secret_hash = &*dhash160(&expected_secret);
    let secret = coin.extract_secret(secret_hash, &tx_hex).unwrap();
    assert_eq!(secret, expected_secret);
}

#[test]
fn test_generate_transaction() {
    let client = electrum_client_for_test(RICK_ELECTRUM_ADDRS);
    let coin = utxo_coin_for_test(client.into(), None, false);
    let unspents = vec![UnspentInfo {
        value: 10000000000,
        outpoint: OutPoint::default(),
        height: Default::default(),
    }];

    let outputs = vec![TransactionOutput {
        script_pubkey: vec![].into(),
        value: 999,
    }];

    let builder = UtxoTxBuilder::new(&coin)
        .add_available_inputs(unspents)
        .add_outputs(outputs);
    let generated = block_on(builder.build());
    // must not allow to use output with value < dust
    generated.unwrap_err();

    let unspents = vec![UnspentInfo {
        value: 100000,
        outpoint: OutPoint::default(),
        height: Default::default(),
    }];

    let outputs = vec![TransactionOutput {
        script_pubkey: vec![].into(),
        value: 98001,
    }];

    let builder = UtxoTxBuilder::new(&coin)
        .add_available_inputs(unspents)
        .add_outputs(outputs);
    let generated = block_on(builder.build()).unwrap();
    // the change that is less than dust must be included to miner fee
    // so no extra outputs should appear in generated transaction
    assert_eq!(generated.0.outputs.len(), 1);

    assert_eq!(generated.1.fee_amount, 1000);
    assert_eq!(generated.1.unused_change, Some(999));
    assert_eq!(generated.1.received_by_me, 0);
    assert_eq!(generated.1.spent_by_me, 100000);

    let unspents = vec![UnspentInfo {
        value: 100000,
        outpoint: OutPoint::default(),
        height: Default::default(),
    }];

    let outputs = vec![TransactionOutput {
        script_pubkey: Builder::build_p2pkh(&coin.as_ref().derivation_method.unwrap_iguana().hash).to_bytes(),
        value: 100000,
    }];

    // test that fee is properly deducted from output amount equal to input amount (max withdraw case)
    let builder = UtxoTxBuilder::new(&coin)
        .add_available_inputs(unspents)
        .add_outputs(outputs)
        .with_fee_policy(FeePolicy::DeductFromOutput(0));

    let generated = block_on(builder.build()).unwrap();
    assert_eq!(generated.0.outputs.len(), 1);

    assert_eq!(generated.1.fee_amount, 1000);
    assert_eq!(generated.1.unused_change, None);
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
    let builder = UtxoTxBuilder::new(&coin)
        .add_available_inputs(unspents)
        .add_outputs(outputs);

    block_on(builder.build()).unwrap_err();
}

#[test]
fn test_addresses_from_script() {
    let client = electrum_client_for_test(RICK_ELECTRUM_ADDRS);
    let coin = utxo_coin_for_test(client.into(), None, false);
    // P2PKH
    let script: Script = "76a91405aab5342166f8594baf17a7d9bef5d56744332788ac".into();
    let expected_addr: Vec<Address> = vec!["R9o9xTocqr6CeEDGDH6mEYpwLoMz6jNjMW".into()];
    let actual_addr = coin.addresses_from_script(&script).unwrap();
    assert_eq!(expected_addr, actual_addr);

    // P2SH
    let script: Script = "a914e71a6120653ebd526e0f9d7a29cde5969db362d487".into();
    let expected_addr: Vec<Address> = vec!["bZoEPR7DjTqSDiQTeRFNDJuQPTRY2335LD".into()];
    let actual_addr = coin.addresses_from_script(&script).unwrap();
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
#[cfg(not(target_arch = "wasm32"))]
fn test_wait_for_payment_spend_timeout_native() {
    let client = NativeClientImpl::default();

    static mut OUTPUT_SPEND_CALLED: bool = false;
    NativeClient::find_output_spend.mock_safe(|_, _, _, _, _| {
        unsafe { OUTPUT_SPEND_CALLED = true };
        MockResult::Return(Box::new(futures01::future::ok(None)))
    });
    let client = UtxoRpcClientEnum::Native(NativeClient(Arc::new(client)));
    let coin = utxo_coin_for_test(client, None, false);
    let transaction = hex::decode("01000000000102fff7f7881a8099afa6940d42d1e7f6362bec38171ea3edf433541db4e4ad969f00000000494830450221008b9d1dc26ba6a9cb62127b02742fa9d754cd3bebf337f7a55d114c8e5cdd30be022040529b194ba3f9281a99f2b1c0a19c0489bc22ede944ccf4ecbab4cc618ef3ed01eeffffffef51e1b804cc89d182d279655c3aa89e815b1b309fe287d9b2b55d57b90ec68a0100000000ffffffff02202cb206000000001976a9148280b37df378db99f66f85c95a783a76ac7a6d5988ac9093510d000000001976a9143bde42dbee7e4dbe6a21b2d50ce2f0167faa815988ac000247304402203609e17b84f6a7d30c80bfa610b5b4542f32a8a0d5447a12fb1366d7f01cc44a0220573a954c4518331561406f90300e8f3358f51928d43c212a8caed02de67eebee0121025476c2e83188368da1ff3e292e7acafcdb3566bb0ad253f62fc70f07aeee635711000000")
        .unwrap();
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
    ElectrumClient::find_output_spend.mock_safe(|_, _, _, _, _| {
        unsafe { OUTPUT_SPEND_CALLED = true };
        MockResult::Return(Box::new(futures01::future::ok(None)))
    });

    let client = ElectrumClientImpl::new(TEST_COIN_NAME.into(), Default::default());
    let client = UtxoRpcClientEnum::Electrum(ElectrumClient(Arc::new(client)));
    let coin = utxo_coin_for_test(client, None, false);
    let transaction = hex::decode("01000000000102fff7f7881a8099afa6940d42d1e7f6362bec38171ea3edf433541db4e4ad969f00000000494830450221008b9d1dc26ba6a9cb62127b02742fa9d754cd3bebf337f7a55d114c8e5cdd30be022040529b194ba3f9281a99f2b1c0a19c0489bc22ede944ccf4ecbab4cc618ef3ed01eeffffffef51e1b804cc89d182d279655c3aa89e815b1b309fe287d9b2b55d57b90ec68a0100000000ffffffff02202cb206000000001976a9148280b37df378db99f66f85c95a783a76ac7a6d5988ac9093510d000000001976a9143bde42dbee7e4dbe6a21b2d50ce2f0167faa815988ac000247304402203609e17b84f6a7d30c80bfa610b5b4542f32a8a0d5447a12fb1366d7f01cc44a0220573a954c4518331561406f90300e8f3358f51928d43c212a8caed02de67eebee0121025476c2e83188368da1ff3e292e7acafcdb3566bb0ad253f62fc70f07aeee635711000000")
        .unwrap();
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
    let client = electrum_client_for_test(RICK_ELECTRUM_ADDRS);
    let coin = utxo_coin_for_test(
        client.into(),
        Some("spice describe gravity federal blast come thank unfair canal monkey style afraid"),
        false,
    );

    // raw tx bytes of https://rick.kmd.dev/tx/ba881ecca15b5d4593f14f25debbcdfe25f101fd2e9cf8d0b5d92d19813d4424
    let payment_tx_bytes = hex::decode("0400008085202f8902e115acc1b9e26a82f8403c9f81785445cc1285093b63b6246cf45aabac5e0865000000006b483045022100ca578f2d6bae02f839f71619e2ced54538a18d7aa92bd95dcd86ac26479ec9f802206552b6c33b533dd6fc8985415a501ebec89d1f5c59d0c923d1de5280e9827858012102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ffffffffb0721bf69163f7a5033fb3d18ba5768621d8c1347ebaa2fddab0d1f63978ea78020000006b483045022100a3309f99167982e97644dbb5cd7279b86630b35fc34855e843f2c5c0cafdc66d02202a8c3257c44e832476b2e2a723dad1bb4ec1903519502a49b936c155cae382ee012102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ffffffff0300e1f5050000000017a91443fde927a77b3c1d104b78155dc389078c4571b0870000000000000000166a14b8bcb07f6344b42ab04250c86a6e8b75d3fdbbc64b8cd736000000001976a91405aab5342166f8594baf17a7d9bef5d56744332788acba0ce35e000000000000000000000000000000")
        .unwrap();

    // raw tx bytes of https://rick.kmd.dev/tx/cea8028f93f7556ce0ef96f14b8b5d88ef2cd29f428df5936e02e71ca5b0c795
    let spend_tx_bytes = hex::decode("0400008085202f890124443d81192dd9b5d0f89c2efd01f125fecdbbde254ff193455d5ba1cc1e88ba00000000d74730440220519d3eed69815a16357ff07bf453b227654dc85b27ffc22a77abe077302833ec02205c27f439ddc542d332504112871ecac310ea710b99e1922f48eb179c045e44ee01200000000000000000000000000000000000000000000000000000000000000000004c6b6304a9e5e25eb1752102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ac6782012088a914b8bcb07f6344b42ab04250c86a6e8b75d3fdbbc6882102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ac68ffffffff0118ddf505000000001976a91405aab5342166f8594baf17a7d9bef5d56744332788acbffee25e000000000000000000000000000000")
        .unwrap();
    let spend_tx = TransactionEnum::UtxoTx(deserialize(spend_tx_bytes.as_slice()).unwrap());

    let found = block_on(coin.search_for_swap_tx_spend_my(
        1591928233,
        &*coin.my_public_key().unwrap(),
        &*dhash160(&secret),
        &payment_tx_bytes,
        0,
        &None,
    ))
    .unwrap()
    .unwrap();
    assert_eq!(FoundSwapTxSpend::Spent(spend_tx), found);
}

#[test]
fn test_search_for_swap_tx_spend_electrum_was_refunded() {
    let secret = [0; 20];
    let client = electrum_client_for_test(RICK_ELECTRUM_ADDRS);
    let coin = utxo_coin_for_test(
        client.into(),
        Some("spice describe gravity federal blast come thank unfair canal monkey style afraid"),
        false,
    );

    // raw tx bytes of https://rick.kmd.dev/tx/78ea7839f6d1b0dafda2ba7e34c1d8218676a58bd1b33f03a5f76391f61b72b0
    let payment_tx_bytes = hex::decode("0400008085202f8902bf17bf7d1daace52e08f732a6b8771743ca4b1cb765a187e72fd091a0aabfd52000000006a47304402203eaaa3c4da101240f80f9c5e9de716a22b1ec6d66080de6a0cca32011cd77223022040d9082b6242d6acf9a1a8e658779e1c655d708379862f235e8ba7b8ca4e69c6012102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ffffffffff023ca13c0e9e085dd13f481f193e8a3e8fd609020936e98b5587342d994f4d020000006b483045022100c0ba56adb8de923975052312467347d83238bd8d480ce66e8b709a7997373994022048507bcac921fdb2302fa5224ce86e41b7efc1a2e20ae63aa738dfa99b7be826012102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ffffffff0300e1f5050000000017a9141ee6d4c38a3c078eab87ad1a5e4b00f21259b10d870000000000000000166a1400000000000000000000000000000000000000001b94d736000000001976a91405aab5342166f8594baf17a7d9bef5d56744332788ac2d08e35e000000000000000000000000000000")
        .unwrap();

    // raw tx bytes of https://rick.kmd.dev/tx/65085eacab5af46c24b6633b098512cc455478819f3c40f8826ae2b9c1ac15e1
    let refund_tx_bytes = hex::decode("0400008085202f8901b0721bf69163f7a5033fb3d18ba5768621d8c1347ebaa2fddab0d1f63978ea7800000000b6473044022052e06c1abf639148229a3991fdc6da15fe51c97577f4fda351d9c606c7cf53670220780186132d67d354564cae710a77d94b6bb07dcbd7162a13bebee261ffc0963601514c6b63041dfae25eb1752102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ac6782012088a9140000000000000000000000000000000000000000882102031d4256c4bc9f99ac88bf3dba21773132281f65f9bf23a59928bce08961e2f3ac68feffffff0118ddf505000000001976a91405aab5342166f8594baf17a7d9bef5d56744332788ace6fae25e000000000000000000000000000000")
        .unwrap();
    let refund_tx = TransactionEnum::UtxoTx(deserialize(refund_tx_bytes.as_slice()).unwrap());

    let found = block_on(coin.search_for_swap_tx_spend_my(
        1591933469,
        coin.as_ref().priv_key_policy.key_pair_or_err().unwrap().public(),
        &secret,
        &payment_tx_bytes,
        0,
        &None,
    ))
    .unwrap()
    .unwrap();
    assert_eq!(FoundSwapTxSpend::Refunded(refund_tx), found);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_impl_set_fixed_fee() {
    UtxoStandardCoin::list_unspent_ordered.mock_safe(|coin, _| {
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None, false);

    let withdraw_req = WithdrawRequest {
        amount: 1u64.into(),
        from: None,
        to: "RQq6fWoy8aGGMLjvRfMY5mBNVm2RQxJyLa".to_string(),
        coin: TEST_COIN_NAME.into(),
        max: false,
        fee: Some(WithdrawFee::UtxoFixed {
            amount: "0.1".parse().unwrap(),
        }),
    };
    let expected = Some(
        UtxoFeeDetails {
            coin: Some(TEST_COIN_NAME.into()),
            amount: "0.1".parse().unwrap(),
        }
        .into(),
    );
    let tx_details = coin.withdraw(withdraw_req).wait().unwrap();
    assert_eq!(expected, tx_details.fee_details);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_impl_sat_per_kb_fee() {
    UtxoStandardCoin::list_unspent_ordered.mock_safe(|coin, _| {
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None, false);

    let withdraw_req = WithdrawRequest {
        amount: 1u64.into(),
        from: None,
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
            coin: Some(TEST_COIN_NAME.into()),
            amount: "0.0245".parse().unwrap(),
        }
        .into(),
    );
    let tx_details = coin.withdraw(withdraw_req).wait().unwrap();
    assert_eq!(expected, tx_details.fee_details);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_impl_sat_per_kb_fee_amount_equal_to_max() {
    UtxoStandardCoin::list_unspent_ordered.mock_safe(|coin, _| {
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None, false);

    let withdraw_req = WithdrawRequest {
        amount: "9.9789".parse().unwrap(),
        from: None,
        to: "RQq6fWoy8aGGMLjvRfMY5mBNVm2RQxJyLa".to_string(),
        coin: TEST_COIN_NAME.into(),
        max: false,
        fee: Some(WithdrawFee::UtxoPerKbyte {
            amount: "0.1".parse().unwrap(),
        }),
    };
    let tx_details = coin.withdraw(withdraw_req).wait().unwrap();
    // The resulting transaction size might be 210 or 211 bytes depending on signature size
    // MM2 always expects the worst case during fee calculation
    // 0.1 * 211 / 1000 = 0.0211
    let expected_fee = Some(
        UtxoFeeDetails {
            coin: Some(TEST_COIN_NAME.into()),
            amount: "0.0211".parse().unwrap(),
        }
        .into(),
    );
    assert_eq!(expected_fee, tx_details.fee_details);
    let expected_balance_change = BigDecimal::from(-10i32);
    assert_eq!(expected_balance_change, tx_details.my_balance_change);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_impl_sat_per_kb_fee_amount_equal_to_max_dust_included_to_fee() {
    UtxoStandardCoin::list_unspent_ordered.mock_safe(|coin, _| {
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None, false);

    let withdraw_req = WithdrawRequest {
        amount: "9.9789".parse().unwrap(),
        from: None,
        to: "RQq6fWoy8aGGMLjvRfMY5mBNVm2RQxJyLa".to_string(),
        coin: TEST_COIN_NAME.into(),
        max: false,
        fee: Some(WithdrawFee::UtxoPerKbyte {
            amount: "0.09999999".parse().unwrap(),
        }),
    };
    let tx_details = coin.withdraw(withdraw_req).wait().unwrap();
    // The resulting transaction size might be 210 or 211 bytes depending on signature size
    // MM2 always expects the worst case during fee calculation
    // 0.1 * 211 / 1000 = 0.0211
    let expected_fee = Some(
        UtxoFeeDetails {
            coin: Some(TEST_COIN_NAME.into()),
            amount: "0.0211".parse().unwrap(),
        }
        .into(),
    );
    assert_eq!(expected_fee, tx_details.fee_details);
    let expected_balance_change = BigDecimal::from(-10i32);
    assert_eq!(expected_balance_change, tx_details.my_balance_change);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_impl_sat_per_kb_fee_amount_over_max() {
    UtxoStandardCoin::list_unspent_ordered.mock_safe(|coin, _| {
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None, false);

    let withdraw_req = WithdrawRequest {
        amount: "9.97939455".parse().unwrap(),
        from: None,
        to: "RQq6fWoy8aGGMLjvRfMY5mBNVm2RQxJyLa".to_string(),
        coin: TEST_COIN_NAME.into(),
        max: false,
        fee: Some(WithdrawFee::UtxoPerKbyte {
            amount: "0.1".parse().unwrap(),
        }),
    };
    coin.withdraw(withdraw_req).wait().unwrap_err();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_impl_sat_per_kb_fee_max() {
    UtxoStandardCoin::list_unspent_ordered.mock_safe(|coin, _| {
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None, false);

    let withdraw_req = WithdrawRequest {
        amount: 0u64.into(),
        from: None,
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
            coin: Some(TEST_COIN_NAME.into()),
            amount: "0.0211".parse().unwrap(),
        }
        .into(),
    );
    let tx_details = coin.withdraw(withdraw_req).wait().unwrap();
    assert_eq!(expected, tx_details.fee_details);
}

#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_kmd_rewards_impl(
    tx_hash: &'static str,
    tx_hex: &'static str,
    verbose_serialized: &str,
    current_mtp: u32,
    expected_rewards: Option<BigDecimal>,
) {
    let verbose: RpcTransaction = json::from_str(verbose_serialized).unwrap();
    let unspent_height = verbose.height;
    UtxoStandardCoin::list_unspent_ordered.mock_safe(move |coin, _| {
        let tx: UtxoTx = tx_hex.into();
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: tx.hash(),
                index: 0,
            },
            value: tx.outputs[0].value,
            height: unspent_height,
        }];
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });
    UtxoStandardCoin::get_current_mtp
        .mock_safe(move |_fields| MockResult::Return(Box::pin(futures::future::ok(current_mtp))));
    NativeClient::get_verbose_transaction.mock_safe(move |_coin, txid| {
        let expected: H256Json = hex::decode(tx_hash).unwrap().as_slice().into();
        assert_eq!(*txid, expected);
        MockResult::Return(Box::new(futures01::future::ok(verbose.clone())))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let mut fields = utxo_coin_fields_for_test(UtxoRpcClientEnum::Native(client), None, false);
    fields.conf.ticker = "KMD".to_owned();
    let coin = utxo_coin_from_fields(fields);

    let withdraw_req = WithdrawRequest {
        amount: BigDecimal::from_str("0.00001").unwrap(),
        from: None,
        to: "RQq6fWoy8aGGMLjvRfMY5mBNVm2RQxJyLa".to_string(),
        coin: "KMD".to_owned(),
        max: false,
        fee: None,
    };
    let expected_fee = TxFeeDetails::Utxo(UtxoFeeDetails {
        coin: Some("KMD".into()),
        amount: "0.00001".parse().unwrap(),
    });
    let tx_details = coin.withdraw(withdraw_req).wait().unwrap();
    assert_eq!(tx_details.fee_details, Some(expected_fee));

    let expected_rewards = expected_rewards.map(|amount| KmdRewardsDetails {
        amount,
        claimed_by_me: true,
    });
    assert_eq!(tx_details.kmd_rewards, expected_rewards);
}

/// https://kmdexplorer.io/tx/535ffa3387d3fca14f4a4d373daf7edf00e463982755afce89bc8c48d8168024
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_kmd_rewards() {
    const TX_HASH: &str = "535ffa3387d3fca14f4a4d373daf7edf00e463982755afce89bc8c48d8168024";
    const TX_HEX: &str = "0400008085202f8901afcadb73880bc1c9e7ce96b8274c2e2a4547415e649f425f98791685be009b73020000006b483045022100b8fbb77efea482b656ad16fc53c5a01d289054c2e429bf1d7bab16c3e822a83602200b87368a95c046b2ce6d0d092185138a3f234a7eb0d7f8227b196ef32358b93f012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff01dd15c293000000001976a91483762a373935ca241d557dfce89171d582b486de88ac99fe9960000000000000000000000000000000";
    const VERBOSE_SERIALIZED: &str = r#"{"hex":"0400008085202f8901afcadb73880bc1c9e7ce96b8274c2e2a4547415e649f425f98791685be009b73020000006b483045022100b8fbb77efea482b656ad16fc53c5a01d289054c2e429bf1d7bab16c3e822a83602200b87368a95c046b2ce6d0d092185138a3f234a7eb0d7f8227b196ef32358b93f012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff01dd15c293000000001976a91483762a373935ca241d557dfce89171d582b486de88ac99fe9960000000000000000000000000000000","txid":"535ffa3387d3fca14f4a4d373daf7edf00e463982755afce89bc8c48d8168024","hash":null,"size":null,"vsize":null,"version":4,"locktime":1620704921,"vin":[{"txid":"739b00be851679985f429f645e4147452a2e4c27b896cee7c9c10b8873dbcaaf","vout":2,"scriptSig":{"asm":"3045022100b8fbb77efea482b656ad16fc53c5a01d289054c2e429bf1d7bab16c3e822a83602200b87368a95c046b2ce6d0d092185138a3f234a7eb0d7f8227b196ef32358b93f[ALL] 03b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd","hex":"483045022100b8fbb77efea482b656ad16fc53c5a01d289054c2e429bf1d7bab16c3e822a83602200b87368a95c046b2ce6d0d092185138a3f234a7eb0d7f8227b196ef32358b93f012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58addd"},"sequence":4294967295,"txinwitness":null}],"vout":[{"value":24.78970333,"n":0,"scriptPubKey":{"asm":"OP_DUP OP_HASH160 83762a373935ca241d557dfce89171d582b486de OP_EQUALVERIFY OP_CHECKSIG","hex":"76a91483762a373935ca241d557dfce89171d582b486de88ac","reqSigs":1,"type":"pubkeyhash","addresses":["RMGJ9tRST45RnwEKHPGgBLuY3moSYP7Mhk"]}}],"blockhash":"0b438a8e50afddb38fb1c7be4536ffc7f7723b76bbc5edf7c28f2c17924dbdfa","confirmations":33186,"rawconfirmations":33186,"time":1620705483,"blocktime":1620705483,"height":2387532}"#;
    const CURRENT_MTP: u32 = 1622724281;

    let expected_rewards = BigDecimal::from_str("0.07895295").unwrap();
    test_withdraw_kmd_rewards_impl(TX_HASH, TX_HEX, VERBOSE_SERIALIZED, CURRENT_MTP, Some(expected_rewards));
}

/// If the ticker is `KMD` AND no rewards were accrued due to a value less than 10 or for any other reasons,
/// then `TransactionDetails::kmd_rewards` has to be `Some(0)`, not `None`.
/// https://kmdexplorer.io/tx/8c43e5a0402648faa5d0ae3550137544507ab1553425fa1b6f481a66a53f7a2d
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_kmd_rewards_zero() {
    const TX_HASH: &str = "8c43e5a0402648faa5d0ae3550137544507ab1553425fa1b6f481a66a53f7a2d";
    const TX_HEX: &str = "0400008085202f8901c3651b6fb9ddf372e7a9d4d829c27eeea6cdfaab4f2e6e3527905c2a14f3702b010000006a47304402206819b3e51f076841ed5946bc9a48b9d75024b60abd8e854bfe50cbdfae8a268e022001a3648d2a4b33a761090676e4a8c676ee67cb602f29fef74ea5bbb8b516a178012103832b54342019dd5ecc08f1143757fbcf4ac6c8696653d456a84b40f34653c9a8ffffffff0200e1f505000000001976a91483762a373935ca241d557dfce89171d582b486de88ac60040c35000000001976a9142b33504039790fde428e4ab084aa1baf6aee209288acb0edd45f000000000000000000000000000000";
    const VERBOSE_SERIALIZED: &str = r#"{"hex":"0400008085202f8901c3651b6fb9ddf372e7a9d4d829c27eeea6cdfaab4f2e6e3527905c2a14f3702b010000006a47304402206819b3e51f076841ed5946bc9a48b9d75024b60abd8e854bfe50cbdfae8a268e022001a3648d2a4b33a761090676e4a8c676ee67cb602f29fef74ea5bbb8b516a178012103832b54342019dd5ecc08f1143757fbcf4ac6c8696653d456a84b40f34653c9a8ffffffff0200e1f505000000001976a91483762a373935ca241d557dfce89171d582b486de88ac60040c35000000001976a9142b33504039790fde428e4ab084aa1baf6aee209288acb0edd45f000000000000000000000000000000","txid":"8c43e5a0402648faa5d0ae3550137544507ab1553425fa1b6f481a66a53f7a2d","hash":null,"size":null,"vsize":null,"version":4,"locktime":1607790000,"vin":[{"txid":"2b70f3142a5c9027356e2e4fabfacda6ee7ec229d8d4a9e772f3ddb96f1b65c3","vout":1,"scriptSig":{"asm":"304402206819b3e51f076841ed5946bc9a48b9d75024b60abd8e854bfe50cbdfae8a268e022001a3648d2a4b33a761090676e4a8c676ee67cb602f29fef74ea5bbb8b516a178[ALL] 03832b54342019dd5ecc08f1143757fbcf4ac6c8696653d456a84b40f34653c9a8","hex":"47304402206819b3e51f076841ed5946bc9a48b9d75024b60abd8e854bfe50cbdfae8a268e022001a3648d2a4b33a761090676e4a8c676ee67cb602f29fef74ea5bbb8b516a178012103832b54342019dd5ecc08f1143757fbcf4ac6c8696653d456a84b40f34653c9a8"},"sequence":4294967295,"txinwitness":null}],"vout":[{"value":1.0,"n":0,"scriptPubKey":{"asm":"OP_DUP OP_HASH160 83762a373935ca241d557dfce89171d582b486de OP_EQUALVERIFY OP_CHECKSIG","hex":"76a91483762a373935ca241d557dfce89171d582b486de88ac","reqSigs":1,"type":"pubkeyhash","addresses":["RMGJ9tRST45RnwEKHPGgBLuY3moSYP7Mhk"]}},{"value":8.8998,"n":1,"scriptPubKey":{"asm":"OP_DUP OP_HASH160 2b33504039790fde428e4ab084aa1baf6aee2092 OP_EQUALVERIFY OP_CHECKSIG","hex":"76a9142b33504039790fde428e4ab084aa1baf6aee209288ac","reqSigs":1,"type":"pubkeyhash","addresses":["RDDcc63q27t6k95LrysuDwtwrxuAXqNiXe"]}}],"blockhash":"0000000054ed9fc7a4316430659e127eac5776ebc2d2382db0cb9be3eb970d7b","confirmations":243859,"rawconfirmations":243859,"time":1607790977,"blocktime":1607790977,"height":2177114}"#;
    const CURRENT_MTP: u32 = 1622724281;

    let expected_rewards = BigDecimal::from(0);
    test_withdraw_kmd_rewards_impl(TX_HASH, TX_HEX, VERBOSE_SERIALIZED, CURRENT_MTP, Some(expected_rewards));
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_rick_rewards_none() {
    // https://rick.explorer.dexstats.info/tx/7181400be323acc6b5f3164240e6c4601ff4c252f40ce7649f87e81634330209
    const TX_HEX: &str = "0400008085202f8901df8119c507aa61d32332cd246dbfeb3818a4f96e76492454c1fbba5aa097977e000000004847304402205a7e229ea6929c97fd6dde254c19e4eb890a90353249721701ae7a1c477d99c402206a8b7c5bf42b5095585731d6b4c589ce557f63c20aed69ff242eca22ecfcdc7a01feffffff02d04d1bffbc050000232102afdbba3e3c90db5f0f4064118f79cf308f926c68afd64ea7afc930975663e4c4ac402dd913000000001976a9143e17014eca06281ee600adffa34b4afb0922a22288ac2bdab86035a00e000000000000000000000000";

    UtxoStandardCoin::list_unspent_ordered.mock_safe(move |coin, _| {
        let tx: UtxoTx = TX_HEX.into();
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: tx.hash(),
                index: 0,
            },
            value: tx.outputs[0].value,
            height: Some(1431628),
        }];
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None, false);

    let withdraw_req = WithdrawRequest {
        amount: BigDecimal::from_str("0.00001").unwrap(),
        from: None,
        to: "RQq6fWoy8aGGMLjvRfMY5mBNVm2RQxJyLa".to_string(),
        coin: "RICK".to_owned(),
        max: false,
        fee: None,
    };
    let expected_fee = TxFeeDetails::Utxo(UtxoFeeDetails {
        coin: Some(TEST_COIN_NAME.into()),
        amount: "0.00001".parse().unwrap(),
    });
    let tx_details = coin.withdraw(withdraw_req).wait().unwrap();
    assert_eq!(tx_details.fee_details, Some(expected_fee));
    assert_eq!(tx_details.kmd_rewards, None);
}

#[test]
fn test_list_mature_unspents_ordered_without_tx_cache() {
    let client = electrum_client_for_test(RICK_ELECTRUM_ADDRS);
    let coin = utxo_coin_for_test(
        client.into(),
        Some("spice describe gravity federal blast come thank unfair canal monkey style afraid"),
        false,
    );
    assert!(coin.as_ref().tx_cache_directory.is_none());
    assert_ne!(
        coin.my_spendable_balance().wait().unwrap(),
        0.into(),
        "The test address doesn't have unspent outputs"
    );
    let (unspents, _) =
        block_on(coin.list_mature_unspent_ordered(&Address::from("R9o9xTocqr6CeEDGDH6mEYpwLoMz6jNjMW"))).unwrap();
    assert!(!unspents.is_empty());
}

#[test]
fn test_utxo_lock() {
    // send several transactions concurrently to check that they are not using same inputs
    let client = electrum_client_for_test(RICK_ELECTRUM_ADDRS);
    let coin = utxo_coin_for_test(client.into(), None, false);
    let output = TransactionOutput {
        value: 1000000,
        script_pubkey: Builder::build_p2pkh(&coin.as_ref().derivation_method.unwrap_iguana().hash).to_bytes(),
    };
    let mut futures = vec![];
    for _ in 0..5 {
        futures.push(send_outputs_from_my_address_impl(coin.clone(), vec![output.clone()]));
    }
    let results = block_on(join_all(futures));
    for result in results {
        result.unwrap();
    }
}

#[test]
fn list_since_block_btc_serde() {
    // https://github.com/KomodoPlatform/atomicDEX-API/issues/563
    let input = r#"{"lastblock":"000000000000000000066f896cca2a6c667ca85fff28ed6731d64e3c39ecb119","removed":[],"transactions":[{"abandoned":false,"address":"1Q3kQ1jsB2VyH83PJT1NXJqEaEcR6Yuknn","amount":-0.01788867,"bip125-replaceable":"no","blockhash":"0000000000000000000db4be4c2df08790e1027326832cc90889554bbebc69b7","blockindex":437,"blocktime":1572174214,"category":"send","confirmations":197,"fee":-0.00012924,"involvesWatchonly":true,"time":1572173721,"timereceived":1572173721,"txid":"29606e6780c69a39767b56dc758e6af31ced5232491ad62dcf25275684cb7701","vout":0,"walletconflicts":[]},{"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":0.1995,"bip125-replaceable":"no","blockhash":"0000000000000000000e75b33bbb27e6af2fc3898108c93c03c293fd72a86c6f","blockindex":157,"blocktime":1572179171,"category":"receive","confirmations":190,"label":"","time":1572178251,"timereceived":1572178251,"txid":"da651c6addc8da7c4b2bec21d43022852a93a9f2882a827704b318eb2966b82e","vout":19,"walletconflicts":[]},{"abandoned":false,"address":"14RXkMTyH4NyK48DbhTQyMBoMb2UkbBEPr","amount":-0.0208,"bip125-replaceable":"no","blockhash":"0000000000000000000611bfe0b3f7612239264459f4f6e7169f8d1a67e1b08f","blockindex":286,"blocktime":1572189657,"category":"send","confirmations":178,"fee":-0.0002,"involvesWatchonly":true,"time":1572189100,"timereceived":1572189100,"txid":"8d10920ce70aeb6c7e61c8d47f3cd903fb69946edd08d8907472a90761965943","vout":0,"walletconflicts":[]},{"abandoned":false,"address":"361JVximBAqkLZERT7XB1rykgLePEHAP7B","amount":-0.01801791,"bip125-replaceable":"no","blockhash":"00000000000000000011e9293c1f07f9711e677389ac101b93116d239ac38c33","blockindex":274,"blocktime":1572173649,"category":"send","confirmations":198,"fee":-0.0000965,"involvesWatchonly":true,"label":"361JVximBAqkLZERT7XB1rykgLePEHAP7B","time":1572173458,"timereceived":1572173458,"txid":"7983cae1afeb7fe58e020878aaedea0fee15be9319bc49c81f3b9ad466782950","vout":0,"walletconflicts":[]},{"abandoned":false,"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":-0.0003447,"bip125-replaceable":"no","blockhash":"00000000000000000011e9293c1f07f9711e677389ac101b93116d239ac38c33","blockindex":274,"blocktime":1572173649,"category":"send","confirmations":198,"fee":-0.0000965,"label":"","time":1572173458,"timereceived":1572173458,"txid":"7983cae1afeb7fe58e020878aaedea0fee15be9319bc49c81f3b9ad466782950","vout":1,"walletconflicts":[]},{"address":"361JVximBAqkLZERT7XB1rykgLePEHAP7B","amount":0.01801791,"bip125-replaceable":"no","blockhash":"00000000000000000011e9293c1f07f9711e677389ac101b93116d239ac38c33","blockindex":274,"blocktime":1572173649,"category":"receive","confirmations":198,"involvesWatchonly":true,"label":"361JVximBAqkLZERT7XB1rykgLePEHAP7B","time":1572173458,"timereceived":1572173458,"txid":"7983cae1afeb7fe58e020878aaedea0fee15be9319bc49c81f3b9ad466782950","vout":0,"walletconflicts":[]},{"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":0.0003447,"bip125-replaceable":"no","blockhash":"00000000000000000011e9293c1f07f9711e677389ac101b93116d239ac38c33","blockindex":274,"blocktime":1572173649,"category":"receive","confirmations":198,"label":"","time":1572173458,"timereceived":1572173458,"txid":"7983cae1afeb7fe58e020878aaedea0fee15be9319bc49c81f3b9ad466782950","vout":1,"walletconflicts":[]},{"abandoned":false,"address":"3B3q1GTLQQ7Fspo6ATy3cd3tg5yu97hkve","amount":-0.021,"bip125-replaceable":"no","blockhash":"0000000000000000000debf11962f89e2ae08f8ff75803b0da6170af6c5c346b","blockindex":2618,"blocktime":1572188894,"category":"send","confirmations":179,"fee":-0.00016026,"involvesWatchonly":true,"label":"3B3q1GTLQQ7Fspo6ATy3cd3tg5yu97hkve","time":1572186009,"timereceived":1572186009,"txid":"54b159ac3a656bbaaf3bf0263b8deafad03b376ec0c2e9c715d0cf1caaf3495e","vout":0,"walletconflicts":[]},{"abandoned":false,"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":-0.17868444,"bip125-replaceable":"no","blockhash":"0000000000000000000debf11962f89e2ae08f8ff75803b0da6170af6c5c346b","blockindex":2618,"blocktime":1572188894,"category":"send","confirmations":179,"fee":-0.00016026,"label":"","time":1572186009,"timereceived":1572186009,"txid":"54b159ac3a656bbaaf3bf0263b8deafad03b376ec0c2e9c715d0cf1caaf3495e","vout":1,"walletconflicts":[]},{"address":"3B3q1GTLQQ7Fspo6ATy3cd3tg5yu97hkve","amount":0.021,"bip125-replaceable":"no","blockhash":"0000000000000000000debf11962f89e2ae08f8ff75803b0da6170af6c5c346b","blockindex":2618,"blocktime":1572188894,"category":"receive","confirmations":179,"involvesWatchonly":true,"label":"3B3q1GTLQQ7Fspo6ATy3cd3tg5yu97hkve","time":1572186009,"timereceived":1572186009,"txid":"54b159ac3a656bbaaf3bf0263b8deafad03b376ec0c2e9c715d0cf1caaf3495e","vout":0,"walletconflicts":[]},{"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":0.17868444,"bip125-replaceable":"no","blockhash":"0000000000000000000debf11962f89e2ae08f8ff75803b0da6170af6c5c346b","blockindex":2618,"blocktime":1572188894,"category":"receive","confirmations":179,"label":"","time":1572186009,"timereceived":1572186009,"txid":"54b159ac3a656bbaaf3bf0263b8deafad03b376ec0c2e9c715d0cf1caaf3495e","vout":1,"walletconflicts":[]},{"abandoned":false,"address":"3AC6k1Y54knEdkgWjX3TjmWGjDHtJCNZZY","amount":-0.17822795,"bip125-replaceable":"no","blockhash":"00000000000000000009a60478f29f4910e29224ea5ed63d77321ac8c624ec45","blockindex":2377,"blocktime":1572190637,"category":"send","confirmations":177,"fee":-0.00009985,"involvesWatchonly":true,"label":"3AC6k1Y54knEdkgWjX3TjmWGjDHtJCNZZY","time":1572189626,"timereceived":1572189626,"txid":"eabc01e45db89ea8cf623f8e22847e4023c69bed3c7d396d573b89dec3fe17a7","vout":0,"walletconflicts":[]},{"abandoned":false,"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":-0.00035664,"bip125-replaceable":"no","blockhash":"00000000000000000009a60478f29f4910e29224ea5ed63d77321ac8c624ec45","blockindex":2377,"blocktime":1572190637,"category":"send","confirmations":177,"fee":-0.00009985,"label":"","time":1572189626,"timereceived":1572189626,"txid":"eabc01e45db89ea8cf623f8e22847e4023c69bed3c7d396d573b89dec3fe17a7","vout":1,"walletconflicts":[]},{"address":"3AC6k1Y54knEdkgWjX3TjmWGjDHtJCNZZY","amount":0.17822795,"bip125-replaceable":"no","blockhash":"00000000000000000009a60478f29f4910e29224ea5ed63d77321ac8c624ec45","blockindex":2377,"blocktime":1572190637,"category":"receive","confirmations":177,"involvesWatchonly":true,"label":"3AC6k1Y54knEdkgWjX3TjmWGjDHtJCNZZY","time":1572189626,"timereceived":1572189626,"txid":"eabc01e45db89ea8cf623f8e22847e4023c69bed3c7d396d573b89dec3fe17a7","vout":0,"walletconflicts":[]},{"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":0.00035664,"bip125-replaceable":"no","blockhash":"00000000000000000009a60478f29f4910e29224ea5ed63d77321ac8c624ec45","blockindex":2377,"blocktime":1572190637,"category":"receive","confirmations":177,"label":"","time":1572189626,"timereceived":1572189626,"txid":"eabc01e45db89ea8cf623f8e22847e4023c69bed3c7d396d573b89dec3fe17a7","vout":1,"walletconflicts":[]},{"abandoned":false,"address":"1Q3kQ1jsB2VyH83PJT1NXJqEaEcR6Yuknn","amount":-0.17809412,"bip125-replaceable":"no","blockhash":"000000000000000000125e17a9540ac901d70e92e987d59a1cf87ca36ebca830","blockindex":1680,"blocktime":1572191122,"category":"send","confirmations":176,"fee":-0.00013383,"involvesWatchonly":true,"time":1572190821,"timereceived":1572190821,"txid":"d3579f7be169ea8fd1358d0eda85bad31ce8080a6020dcd224eac8a663dc9bf7","vout":0,"walletconflicts":[]},{"abandoned":false,"address":"326VCyLKV1w4SxeYs81jQU1SC11njcL1eG","amount":-0.039676,"bip125-replaceable":"no","blockhash":"0000000000000000000d61630db06ed5d3054a39bf71a706efeaa9e86866b9d4","blockindex":2193,"blocktime":1572053656,"category":"send","confirmations":380,"fee":-0.00005653,"involvesWatchonly":true,"label":"326VCyLKV1w4SxeYs81jQU1SC11njcL1eG","time":1572052431,"timereceived":1572052431,"txid":"37b57fb36312e21ec7d069a55ab9bffc6abc7fe3731ed38502c5329025a9edf9","vout":0,"walletconflicts":[]},{"abandoned":false,"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":-0.01845911,"bip125-replaceable":"no","blockhash":"0000000000000000000d61630db06ed5d3054a39bf71a706efeaa9e86866b9d4","blockindex":2193,"blocktime":1572053656,"category":"send","confirmations":380,"fee":-0.00005653,"label":"","time":1572052431,"timereceived":1572052431,"txid":"37b57fb36312e21ec7d069a55ab9bffc6abc7fe3731ed38502c5329025a9edf9","vout":1,"walletconflicts":[]},{"address":"326VCyLKV1w4SxeYs81jQU1SC11njcL1eG","amount":0.039676,"bip125-replaceable":"no","blockhash":"0000000000000000000d61630db06ed5d3054a39bf71a706efeaa9e86866b9d4","blockindex":2193,"blocktime":1572053656,"category":"receive","confirmations":380,"involvesWatchonly":true,"label":"326VCyLKV1w4SxeYs81jQU1SC11njcL1eG","time":1572052431,"timereceived":1572052431,"txid":"37b57fb36312e21ec7d069a55ab9bffc6abc7fe3731ed38502c5329025a9edf9","vout":0,"walletconflicts":[]},{"address":"1JsAjr6d21j9T8EMsYnQ6GXf1mM523JAv1","amount":0.01845911,"bip125-replaceable":"no","blockhash":"0000000000000000000d61630db06ed5d3054a39bf71a706efeaa9e86866b9d4","blockindex":2193,"blocktime":1572053656,"category":"receive","confirmations":380,"label":"","time":1572052431,"timereceived":1572052431,"txid":"37b57fb36312e21ec7d069a55ab9bffc6abc7fe3731ed38502c5329025a9edf9","vout":1,"walletconflicts":[]}]}"#;
    let _res: ListSinceBlockRes = json::from_str(input).unwrap();
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
        false,
    );

    let fut = async move {
        // hash of coinbase transaction https://morty.explorer.dexstats.info/tx/b59b093ed97c1798f2a88ee3375a0c11d0822b6e4468478777f899891abd34a5
        let hash = hex::decode("b59b093ed97c1798f2a88ee3375a0c11d0822b6e4468478777f899891abd34a5").unwrap();

        let mut input_transactions = HistoryUtxoTxMap::new();
        let tx_details = coin.tx_details_by_hash(&hash, &mut input_transactions).await.unwrap();
        assert!(tx_details.from.is_empty());
    };

    block_on(fut);
}

#[test]
fn test_electrum_rpc_client_error() {
    let client = electrum_client_for_test(&["electrum1.cipig.net:10060"]);

    let empty_hash = H256Json::default();
    let err = client.get_verbose_transaction(&empty_hash).wait().unwrap_err();

    // use the static string instead because the actual error message cannot be obtain
    // by serde_json serialization
    let expected = r#"JsonRpcError { client_info: "coin: RICK", request: JsonRpcRequest { jsonrpc: "2.0", id: "1", method: "blockchain.transaction.get", params: [String("0000000000000000000000000000000000000000000000000000000000000000"), Bool(true)] }, error: Response(electrum1.cipig.net:10060, Object({"code": Number(2), "message": String("daemon error: DaemonError({'code': -5, 'message': 'No such mempool or blockchain transaction. Use gettransaction for wallet transactions.'})")})) }"#;
    let actual = format!("{}", err);

    assert!(actual.contains(expected));
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
#[cfg(not(target_arch = "wasm32"))]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/617
fn test_generate_transaction_relay_fee_is_used_when_dynamic_fee_is_lower() {
    let client = NativeClientImpl::default();

    static mut GET_RELAY_FEE_CALLED: bool = false;
    NativeClient::get_relay_fee.mock_safe(|_| {
        unsafe { GET_RELAY_FEE_CALLED = true };
        MockResult::Return(Box::new(futures01::future::ok("1.0".parse().unwrap())))
    });
    let client = UtxoRpcClientEnum::Native(NativeClient(Arc::new(client)));
    let mut coin = utxo_coin_fields_for_test(client, None, false);
    coin.conf.force_min_relay_fee = true;
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

    let builder = UtxoTxBuilder::new(&coin)
        .add_available_inputs(unspents)
        .add_outputs(outputs)
        .with_fee(ActualTxFee::Dynamic(100));

    let generated = block_on(builder.build()).unwrap();
    assert_eq!(generated.0.outputs.len(), 1);

    // generated transaction fee must be equal to relay fee if calculated dynamic fee is lower than relay
    assert_eq!(generated.1.fee_amount, 100000000);
    assert_eq!(generated.1.unused_change, None);
    assert_eq!(generated.1.received_by_me, 0);
    assert_eq!(generated.1.spent_by_me, 1000000000);
    assert!(unsafe { GET_RELAY_FEE_CALLED });
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/1037
fn test_generate_transaction_relay_fee_is_used_when_dynamic_fee_is_lower_and_deduct_from_output() {
    let client = NativeClientImpl::default();

    static mut GET_RELAY_FEE_CALLED: bool = false;
    NativeClient::get_relay_fee.mock_safe(|_| {
        unsafe { GET_RELAY_FEE_CALLED = true };
        MockResult::Return(Box::new(futures01::future::ok("1.0".parse().unwrap())))
    });
    let client = UtxoRpcClientEnum::Native(NativeClient(Arc::new(client)));
    let mut coin = utxo_coin_fields_for_test(client, None, false);
    coin.conf.force_min_relay_fee = true;
    let coin = utxo_coin_from_fields(coin);
    let unspents = vec![UnspentInfo {
        value: 1000000000,
        outpoint: OutPoint::default(),
        height: Default::default(),
    }];

    let outputs = vec![TransactionOutput {
        script_pubkey: vec![].into(),
        value: 1000000000,
    }];

    let tx_builder = UtxoTxBuilder::new(&coin)
        .add_available_inputs(unspents)
        .add_outputs(outputs)
        .with_fee_policy(FeePolicy::DeductFromOutput(0))
        .with_fee(ActualTxFee::Dynamic(100));

    let generated = block_on(tx_builder.build()).unwrap();
    assert_eq!(generated.0.outputs.len(), 1);
    // `output (= 10.0) - fee_amount (= 1.0)`
    assert_eq!(generated.0.outputs[0].value, 900000000);

    // generated transaction fee must be equal to relay fee if calculated dynamic fee is lower than relay
    assert_eq!(generated.1.fee_amount, 100000000);
    assert_eq!(generated.1.unused_change, None);
    assert_eq!(generated.1.received_by_me, 0);
    assert_eq!(generated.1.spent_by_me, 1000000000);
    assert!(unsafe { GET_RELAY_FEE_CALLED });
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/617
fn test_generate_tx_fee_is_correct_when_dynamic_fee_is_larger_than_relay() {
    let client = NativeClientImpl::default();

    static mut GET_RELAY_FEE_CALLED: bool = false;
    NativeClient::get_relay_fee.mock_safe(|_| {
        unsafe { GET_RELAY_FEE_CALLED = true };
        MockResult::Return(Box::new(futures01::future::ok("0.00001".parse().unwrap())))
    });
    let client = UtxoRpcClientEnum::Native(NativeClient(Arc::new(client)));
    let mut coin = utxo_coin_fields_for_test(client, None, false);
    coin.conf.force_min_relay_fee = true;
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

    let builder = UtxoTxBuilder::new(&coin)
        .add_available_inputs(unspents)
        .add_outputs(outputs)
        .with_fee(ActualTxFee::Dynamic(1000));

    let generated = block_on(builder.build()).unwrap();

    assert_eq!(generated.0.outputs.len(), 2);
    assert_eq!(generated.0.inputs.len(), 20);

    // resulting signed transaction size would be 3032 bytes so fee is 3032 sat
    assert_eq!(generated.1.fee_amount, 3032);
    assert_eq!(generated.1.unused_change, None);
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
        .get_median_time_past(1773390, KMD_MTP_BLOCK_COUNT, CoinVariant::Standard)
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

    let mtp = client
        .get_median_time_past(632858, KMD_MTP_BLOCK_COUNT, CoinVariant::Standard)
        .wait()
        .unwrap();
    assert_eq!(1591173041, mtp);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_get_median_time_past_from_native_has_median_in_get_block() {
    let client = native_client_for_test();
    NativeClientImpl::get_block_hash.mock_safe(|_, block_num| {
        assert_eq!(block_num, 632858);
        MockResult::Return(Box::new(futures01::future::ok(
            "00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3".into(),
        )))
    });

    NativeClientImpl::get_block.mock_safe(|_, block_hash| {
        assert_eq!(block_hash, "00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3".into());
        let block_data_str = r#"{"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632858,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591174568,"mediantime":1591173041,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"}"#;
        let block_data = json::from_str(block_data_str).unwrap();
        MockResult::Return(
            Box::new(futures01::future::ok(block_data))
        )
    });

    let mtp = client
        .get_median_time_past(632858, KMD_MTP_BLOCK_COUNT, CoinVariant::Standard)
        .wait()
        .unwrap();
    assert_eq!(1591173041, mtp);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_get_median_time_past_from_native_does_not_have_median_in_get_block() {
    use std::collections::HashMap;

    let blocks_json_str = r#"
    [
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632858,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173090,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e4","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632857,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173080,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e5","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632856,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173070,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e6","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632855,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173058,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e7","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632854,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173050,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e8","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632853,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173041,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695e9","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632852,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173040,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695f0","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632851,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173039,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695f1","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632850,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173038,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695f2","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632849,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173037,"nonce":"1594651477","bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"},
        {"hash":"00000000000000000002eb7892b8fdfd7b8e0f089e5cdf96436de680b7e695f3","confirmations":1,"strippedsize":833287,"size":1493229,"weight":3993090,"height":632848,"version":549453824,"versionHex":"20c00000","merkleroot":"7e20760d227465d2a84fbb2617b2962f77364daa66f06b48d1010fa27923b940","tx":[],"time":1591173030,"nonce":1594651477,"bits":"171297f6","difficulty":15138043247082.88,"chainwork":"00000000000000000000000000000000000000000fff2e35384d3c16f53adda4","nTx":1601,"previousblockhash":"00000000000000000009a54084d9f4eafa3ca07af646ff8fa9031d0ac72a92aa"}
    ]
    "#;

    let blocks: Vec<VerboseBlock> = json::from_str(blocks_json_str).unwrap();
    let mut block_hashes: HashMap<_, _> = blocks
        .iter()
        .map(|block| (block.height.unwrap() as u64, block.hash.clone()))
        .collect();
    let mut blocks: HashMap<_, _> = blocks.into_iter().map(|block| (block.hash.clone(), block)).collect();
    let client = native_client_for_test();

    NativeClientImpl::get_block_hash.mock_safe(move |_, block_num| {
        let hash = block_hashes.remove(&block_num).unwrap();
        MockResult::Return(Box::new(futures01::future::ok(hash)))
    });

    NativeClientImpl::get_block.mock_safe(move |_, block_hash| {
        let block = blocks.remove(&block_hash).unwrap();
        MockResult::Return(Box::new(futures01::future::ok(block)))
    });

    let mtp = client
        .get_median_time_past(632858, KMD_MTP_BLOCK_COUNT, CoinVariant::Standard)
        .wait()
        .unwrap();
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
         "servers": [
             {"url":"electroncash.de:50003"},
             {"url":"tbch.loping.net:60001"},
             {"url":"blackie.c3-soft.com:60001"},
             {"url":"bch0.kister.net:51001"},
             {"url":"testnet.imaginary.cash:50001"}
         ],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();

    let coin = block_on(utxo_standard_coin_with_priv_key(
        &ctx, "BCH", &conf, &params, &[1u8; 32],
    ))
    .unwrap();

    let hash = hex::decode("0f2f6e0c8f440c641895023782783426c3aca1acc78d7c0db7751995e8aa5751").unwrap();
    let fut = async {
        let mut input_transactions = HistoryUtxoTxMap::new();
        let tx_details = coin.tx_details_by_hash(&hash, &mut input_transactions).await.unwrap();
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
         "servers": [
             {"url":"electroncash.de:50003"},
             {"url":"tbch.loping.net:60001"},
             {"url":"blackie.c3-soft.com:60001"},
             {"url":"bch0.kister.net:51001"},
             {"url":"testnet.imaginary.cash:50001"}
         ],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();

    let coin = block_on(utxo_standard_coin_with_priv_key(
        &ctx, "BCH", &conf, &params, &[1u8; 32],
    ))
    .unwrap();

    // other error on parse
    let error = coin
        .address_from_str("bitcoincash:000000000000000000000000000000000000000000")
        .err()
        .unwrap();
    assert!(error.contains("Invalid address: bitcoincash:000000000000000000000000000000000000000000"));
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
         "servers": [
             {"url":"electroncash.de:50003"},
             {"url":"tbch.loping.net:60001"},
             {"url":"blackie.c3-soft.com:60001"},
             {"url":"bch0.kister.net:51001"},
             {"url":"testnet.imaginary.cash:50001"}
         ],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();

    let coin = block_on(utxo_standard_coin_with_priv_key(
        &ctx, "BCH", &conf, &params, &[1u8; 32],
    ))
    .unwrap();

    let error = coin
        .address_from_str("bitcoincash:qzxqqt9lh4feptf0mplnk58gnajfepzwcq9f2rxk55")
        .err()
        .unwrap();
    assert!(error.contains("Legacy address format activated for BCH, but CashAddress format used instead"));

    // other error on parse
    let error = coin
        .address_from_str("0000000000000000000000000000000000")
        .err()
        .unwrap();
    assert!(error.contains("Invalid address: 0000000000000000000000000000000000"));
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
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let error = block_on(utxo_standard_coin_with_priv_key(
        &ctx, "RICK", &conf, &params, &[1u8; 32],
    ))
    .err()
    .unwrap();
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
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(utxo_standard_coin_with_priv_key(
        &ctx,
        "RICK",
        &conf,
        &params,
        &*key_pair.private().secret,
    ))
    .unwrap();

    let output = TransactionOutput {
        value: 1000000,
        script_pubkey: Builder::build_p2pkh(&coin.as_ref().derivation_method.unwrap_iguana().hash).to_bytes(),
    };
    let mut futures = vec![];
    for _ in 0..5 {
        futures.push(send_outputs_from_my_address_impl(coin.clone(), vec![output.clone()]));
    }
    let results = block_on(join_all(futures));
    for result in results {
        result.unwrap();
    }
}

#[test]
fn test_one_unavailable_electrum_proto_version() {
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
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();

    let coin = block_on(utxo_standard_coin_with_priv_key(
        &ctx, "BTC", &conf, &params, &[1u8; 32],
    ))
    .unwrap();

    block_on(async { Timer::sleep(0.5).await });

    assert!(coin.as_ref().rpc_client.get_block_count().wait().is_ok());
}

#[test]
fn test_qtum_unspendable_balance_failed_once() {
    let mut unspents = vec![
        // spendable balance (69.0) > balance (68.0)
        vec![
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 5000000000,
                height: Default::default(),
            },
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 1900000000,
                height: Default::default(),
            },
        ],
        // spendable balance (68.0) == balance (68.0)
        vec![
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 5000000000,
                height: Default::default(),
            },
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 1800000000,
                height: Default::default(),
            },
        ],
    ];
    QtumCoin::list_mature_unspent_ordered.mock_safe(move |coin, _| {
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = unspents.pop().unwrap();
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let conf = json!({"coin":"tQTUM","rpcport":13889,"pubtype":120,"p2shtype":110});
    let req = json!({
        "method": "electrum",
        "servers": [{"url":"electrum1.cipig.net:10071"}, {"url":"electrum2.cipig.net:10071"}, {"url":"electrum3.cipig.net:10071"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();

    let priv_key = [
        184, 199, 116, 240, 113, 222, 8, 199, 253, 143, 98, 185, 127, 26, 87, 38, 246, 206, 159, 27, 207, 20, 27, 112,
        184, 102, 137, 37, 78, 214, 113, 78,
    ];

    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(qtum_coin_with_priv_key(&ctx, "tQTUM", &conf, &params, &priv_key)).unwrap();

    let CoinBalance { spendable, unspendable } = coin.my_balance().wait().unwrap();
    let expected_spendable = BigDecimal::from(68);
    let expected_unspendable = BigDecimal::from(0);
    assert_eq!(spendable, expected_spendable);
    assert_eq!(unspendable, expected_unspendable);
}

#[test]
fn test_qtum_generate_pod() {
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let conf = json!({"coin":"tQTUM","rpcport":13889,"pubtype":120,"p2shtype":110});
    let req = json!({
        "method": "electrum",
        "servers": [{"url":"electrum1.cipig.net:10071"}, {"url":"electrum2.cipig.net:10071"}, {"url":"electrum3.cipig.net:10071"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();

    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(qtum_coin_with_priv_key(&ctx, "tQTUM", &conf, &params, &priv_key)).unwrap();
    let expected_res = "20086d757b34c01deacfef97a391f8ed2ca761c72a08d5000adc3d187b1007aca86a03bc5131b1f99b66873a12b51f8603213cdc1aa74c05ca5d48fe164b82152b";
    let address = Address::from_str("qcyBHeSct7Wr4mAw18iuQ1zW5mMFYmtmBE").unwrap();
    let res = coin.generate_pod(address.hash).unwrap();
    assert_eq!(expected_res, res.to_string());
}

#[test]
fn test_qtum_add_delegation() {
    let keypair = key_pair_from_seed("asthma turtle lizard tone genuine tube hunt valley soap cloth urge alpha amazing frost faculty cycle mammal leaf normal bright topple avoid pulse buffalo").unwrap();
    let conf = json!({"coin":"tQTUM","rpcport":13889,"pubtype":120,"p2shtype":110, "mature_confirmations":1});
    let req = json!({
        "method": "electrum",
        "servers": [{"url":"electrum1.cipig.net:10071"}, {"url":"electrum2.cipig.net:10071"}, {"url":"electrum3.cipig.net:10071"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(qtum_coin_with_priv_key(
        &ctx,
        "tQTUM",
        &conf,
        &params,
        keypair.private().secret.as_slice(),
    ))
    .unwrap();
    let address = Address::from_str("qcyBHeSct7Wr4mAw18iuQ1zW5mMFYmtmBE").unwrap();
    let request = QtumDelegationRequest {
        address: address.to_string(),
        fee: Some(10),
    };
    let res = coin.add_delegation(request).wait().unwrap();
    // Eligible for delegation
    assert_eq!(res.my_balance_change.is_negative(), true);
    assert_eq!(res.total_amount, res.spent_by_me);
    assert!(res.spent_by_me > res.received_by_me);

    let request = QtumDelegationRequest {
        address: "fake_address".to_string(),
        fee: Some(10),
    };
    let res = coin.add_delegation(request).wait();
    // Wrong address
    assert_eq!(res.is_err(), true);
}

#[test]
fn test_qtum_add_delegation_on_already_delegating() {
    let keypair = key_pair_from_seed("federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron").unwrap();
    let conf = json!({"coin":"tQTUM","rpcport":13889,"pubtype":120,"p2shtype":110, "mature_confirmations":1});
    let req = json!({
        "method": "electrum",
        "servers": [{"url":"electrum1.cipig.net:10071"}, {"url":"electrum2.cipig.net:10071"}, {"url":"electrum3.cipig.net:10071"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(qtum_coin_with_priv_key(
        &ctx,
        "tQTUM",
        &conf,
        &params,
        keypair.private().secret.as_slice(),
    ))
    .unwrap();
    let address = Address::from_str("qcyBHeSct7Wr4mAw18iuQ1zW5mMFYmtmBE").unwrap();
    let request = QtumDelegationRequest {
        address: address.to_string(),
        fee: Some(10),
    };
    let res = coin.add_delegation(request).wait();
    // Already Delegating
    assert_eq!(res.is_err(), true);
}

#[test]
fn test_qtum_get_delegation_infos() {
    let keypair =
        key_pair_from_seed("federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron").unwrap();
    let conf = json!({"coin":"tQTUM","rpcport":13889,"pubtype":120,"p2shtype":110, "mature_confirmations":1});
    let req = json!({
        "method": "electrum",
        "servers": [{"url":"electrum1.cipig.net:10071"}, {"url":"electrum2.cipig.net:10071"}, {"url":"electrum3.cipig.net:10071"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();

    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(qtum_coin_with_priv_key(
        &ctx,
        "tQTUM",
        &conf,
        &params,
        keypair.private().secret.as_slice(),
    ))
    .unwrap();
    let staking_infos = coin.get_delegation_infos().wait().unwrap();
    match staking_infos.staking_infos_details {
        StakingInfosDetails::Qtum(staking_details) => {
            assert_eq!(staking_details.am_i_staking, true);
            assert_eq!(staking_details.staker.unwrap(), "qcyBHeSct7Wr4mAw18iuQ1zW5mMFYmtmBE");
            // Will return false for segwit.
            assert_eq!(staking_details.is_staking_supported, true);
        },
    };
}

#[test]
fn test_qtum_remove_delegation() {
    let keypair = key_pair_from_seed("federal stay trigger hour exist success game vapor become comfort action phone bright ill target wild nasty crumble dune close rare fabric hen iron").unwrap();
    let conf = json!({"coin":"tQTUM","rpcport":13889,"pubtype":120,"p2shtype":110, "mature_confirmations":1});
    let req = json!({
        "method": "electrum",
        "servers": [{"url":"electrum1.cipig.net:10071"}, {"url":"electrum2.cipig.net:10071"}, {"url":"electrum3.cipig.net:10071"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(qtum_coin_with_priv_key(
        &ctx,
        "tQTUM",
        &conf,
        &params,
        keypair.private().secret.as_slice(),
    ))
    .unwrap();
    let res = coin.remove_delegation().wait();
    assert_eq!(res.is_err(), false);
}

#[test]
fn test_qtum_unspendable_balance_failed() {
    QtumCoin::list_mature_unspent_ordered.mock_safe(move |coin, _| {
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        // spendable balance (69.0) > balance (68.0)
        let unspents = vec![
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 5000000000,
                height: Default::default(),
            },
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 1900000000,
                height: Default::default(),
            },
        ];
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let conf = json!({"coin":"tQTUM","rpcport":13889,"pubtype":120,"p2shtype":110});
    let req = json!({
        "method": "electrum",
        "servers": [{"url":"electrum1.cipig.net:10071"}, {"url":"electrum2.cipig.net:10071"}, {"url":"electrum3.cipig.net:10071"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();

    let priv_key = [
        184, 199, 116, 240, 113, 222, 8, 199, 253, 143, 98, 185, 127, 26, 87, 38, 246, 206, 159, 27, 207, 20, 27, 112,
        184, 102, 137, 37, 78, 214, 113, 78,
    ];

    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(qtum_coin_with_priv_key(&ctx, "tQTUM", &conf, &params, &priv_key)).unwrap();

    let error = coin.my_balance().wait().err().unwrap();
    log!("error: "[error]);
    let expected_error = BalanceError::Internal("Spendable balance 69 greater than total balance 68".to_owned());
    assert_eq!(error.get_inner(), &expected_error);
}

#[test]
fn test_qtum_my_balance() {
    QtumCoin::list_mature_unspent_ordered.mock_safe(move |coin, _| {
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        // spendable balance (66.0) < balance (68.0), then unspendable balance is expected to be (2.0)
        let unspents = vec![
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 5000000000,
                height: Default::default(),
            },
            UnspentInfo {
                outpoint: OutPoint {
                    hash: 1.into(),
                    index: 0,
                },
                value: 1600000000,
                height: Default::default(),
            },
        ];
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let conf = json!({"coin":"tQTUM","rpcport":13889,"pubtype":120,"p2shtype":110});
    let req = json!({
        "method": "electrum",
        "servers": [{"url":"electrum1.cipig.net:10071"}, {"url":"electrum2.cipig.net:10071"}, {"url":"electrum3.cipig.net:10071"}],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();

    let priv_key = [
        184, 199, 116, 240, 113, 222, 8, 199, 253, 143, 98, 185, 127, 26, 87, 38, 246, 206, 159, 27, 207, 20, 27, 112,
        184, 102, 137, 37, 78, 214, 113, 78,
    ];

    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(qtum_coin_with_priv_key(&ctx, "tQTUM", &conf, &params, &priv_key)).unwrap();

    let CoinBalance { spendable, unspendable } = coin.my_balance().wait().unwrap();
    let expected_spendable = BigDecimal::from(66);
    let expected_unspendable = BigDecimal::from(2);
    assert_eq!(spendable, expected_spendable);
    assert_eq!(unspendable, expected_unspendable);
}

#[test]
fn test_qtum_my_balance_with_check_utxo_maturity_false() {
    const DISPLAY_BALANCE: u64 = 68;
    ElectrumClient::display_balance.mock_safe(move |_, _, _| {
        MockResult::Return(Box::new(futures01::future::ok(BigDecimal::from(DISPLAY_BALANCE))))
    });
    QtumCoin::list_all_unspent_ordered.mock_safe(move |_, _| {
        panic!("'QtumCoin::list_all_unspent_ordered' is not expected to be called when `check_utxo_maturity` is false")
    });

    let conf = json!({"coin":"tQTUM","rpcport":13889,"pubtype":120,"p2shtype":110});
    let req = json!({
        "method": "electrum",
        "servers": [{"url":"electrum1.cipig.net:10071"}, {"url":"electrum2.cipig.net:10071"}, {"url":"electrum3.cipig.net:10071"}],
        "check_utxo_maturity": false,
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();

    let priv_key = [
        184, 199, 116, 240, 113, 222, 8, 199, 253, 143, 98, 185, 127, 26, 87, 38, 246, 206, 159, 27, 207, 20, 27, 112,
        184, 102, 137, 37, 78, 214, 113, 78,
    ];

    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();
    let coin = block_on(qtum_coin_with_priv_key(&ctx, "tQTUM", &conf, &params, &priv_key)).unwrap();

    let CoinBalance { spendable, unspendable } = coin.my_balance().wait().unwrap();
    let expected_spendable = BigDecimal::from(DISPLAY_BALANCE);
    let expected_unspendable = BigDecimal::from(0);
    assert_eq!(spendable, expected_spendable);
    assert_eq!(unspendable, expected_unspendable);
}

fn test_list_mature_unspents_ordered_from_cache_impl(
    unspent_height: Option<u64>,
    cached_height: Option<u64>,
    cached_confs: u32,
    block_count: u64,
    expected_height: Option<u64>,
    expected_confs: u32,
) {
    const TX_HASH: &str = "0a0fda88364b960000f445351fe7678317a1e0c80584de0413377ede00ba696f";
    let tx_hash: H256Json = hex::decode(TX_HASH).unwrap().as_slice().into();
    let client = electrum_client_for_test(RICK_ELECTRUM_ADDRS);
    let mut verbose = client.get_verbose_transaction(&tx_hash).wait().unwrap();
    verbose.confirmations = cached_confs;
    verbose.height = cached_height;

    // prepare mocks
    ElectrumClient::list_unspent.mock_safe(move |_, _, _| {
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: H256::from_reversed_str(TX_HASH),
                index: 0,
            },
            value: 1000000000,
            height: unspent_height,
        }];
        MockResult::Return(Box::new(futures01::future::ok(unspents)))
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
    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Electrum(client), None, false);
    let (unspents, _) =
        block_on(coin.list_mature_unspent_ordered(&Address::from("R9o9xTocqr6CeEDGDH6mEYpwLoMz6jNjMW")))
            .expect("Expected an empty unspent list");
    // unspents should be empty because `is_unspent_mature()` always returns false
    assert!(unspents.is_empty());
    assert!(unsafe { IS_UNSPENT_MATURE_CALLED == true });
}

#[test]
fn test_list_mature_unspents_ordered_from_cache() {
    let unspent_height = None;
    let cached_height = None;
    let cached_confs = 0;
    let block_count = 1000;
    let expected_height = None; // is unknown
    let expected_confs = 0; // is not changed because height is unknown
    test_list_mature_unspents_ordered_from_cache_impl(
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
    test_list_mature_unspents_ordered_from_cache_impl(
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
    test_list_mature_unspents_ordered_from_cache_impl(
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
    test_list_mature_unspents_ordered_from_cache_impl(
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
    test_list_mature_unspents_ordered_from_cache_impl(
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
    test_list_mature_unspents_ordered_from_cache_impl(
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
    test_list_mature_unspents_ordered_from_cache_impl(
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
    test_list_mature_unspents_ordered_from_cache_impl(
        unspent_height,
        cached_height,
        cached_confs,
        block_count,
        expected_height,
        expected_confs,
    );
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_native_client_unspents_filtered_using_tx_cache_single_tx_in_cache() {
    let client = native_client_for_test();
    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client), None, false);

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
#[cfg(not(target_arch = "wasm32"))]
fn test_native_client_unspents_filtered_using_tx_cache_single_several_chained_txs_in_cache() {
    let client = native_client_for_test();
    let coin = utxo_coin_fields_for_test(UtxoRpcClientEnum::Native(client), None, false);

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
#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
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
        assert_eq!(*txid, expected_txid);
        // no matter what we return here
        let bytes: BytesJson = hex::decode("0400008085202f890347d329798b508dc28ec99d8c6f6c7ced860a19a364e1bafe391cab89aeaac731020000006a47304402203ea8b380d0a7e64348869ef7c4c2bfa966fc7b148633003332fa8d0ab0c1bc5602202cc63fabdd2a6578c52d8f4f549069b16505f2ead48edc2b8de299be15aadf9a012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff1d1fd3a6b01710647a7f4a08c6de6075cb8e78d5069fa50f10c4a2a10ded2a95000000006a47304402203868945edc0f6dc2ee43d70a69ee4ec46ca188dc493173ce58924ba9bf6ee7a50220648ff99ce458ca72800758f6a1bd3800cd05ff9c3122f23f3653c25e09d22c79012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff7932150df8b4a1852b8b84b89b0d5322bf74665fb7f76a728369fd6895d3fd48000000006a4730440220127918c6f79c11f7f2376a6f3b750ed4c7103183181ad1218afcb2625ece9599022028c05e88d3a2f97cebd84a718cda33b62b48b18f16278fa8e531fd2155e61ee8012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff0329fd12000000000017a914cafb62e3e8bdb8db3735c39b92743ac6ebc9ef20870000000000000000166a14a7416b070c9bb98f4bafae55616f005a2a30bd6014b40c00000000001976a91450f4f098306f988d8843004689fae28c83ef16e888ac8cc5925f000000000000000000000000000000").unwrap().into();
        MockResult::Return(Box::new(futures01::future::ok(bytes)))
    });
    let client = native_client_for_test();

    // no matter what arguments we will pass to the function because of the mocks above
    let tx: UtxoTx = "0400008085202f89027f57730fcbbc2c72fb18bcc3766a713044831a117bb1cade3ed88644864f7333020000006a47304402206e3737b2fcf078b61b16fa67340cc3e79c5d5e2dc9ffda09608371552a3887450220460a332aa1b8ad8f2de92d319666f70751078b221199951f80265b4f7cef8543012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff42b916a80430b80a77e114445b08cf120735447a524de10742fac8f6a9d4170f000000006a473044022004aa053edafb9d161ea8146e0c21ed1593aa6b9404dd44294bcdf920a1695fd902202365eac15dbcc5e9f83e2eed56a8f2f0e5aded36206f9c3fabc668fd4665fa2d012102d8c948c6af848c588517288168faa397d6ba3ea924596d03d1d84f224b5123c2ffffffff03547b16000000000017a9143e8ad0e2bf573d32cb0b3d3a304d9ebcd0c2023b870000000000000000166a144e2b3c0323ab3c2dc6f86dc5ec0729f11e42f56103970400000000001976a91450f4f098306f988d8843004689fae28c83ef16e888ac89c5925f000000000000000000000000000000".into();
    let vout = 0;
    let from_block = 0;
    let actual = client
        .find_output_spend(
            tx.hash(),
            &tx.outputs[vout].script_pubkey,
            vout,
            BlockHashOrHeight::Height(from_block),
        )
        .wait();
    assert_eq!(actual, Ok(None));
    assert_eq!(unsafe { GET_RAW_TRANSACTION_BYTES_CALLED }, 1);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_qtum_is_unspent_mature() {
    use crate::utxo::qtum::QtumBasedCoin;
    use rpc::v1::types::{ScriptType, SignedTransactionOutput, TransactionOutputScript};

    let mut coin_fields = utxo_coin_fields_for_test(UtxoRpcClientEnum::Native(native_client_for_test()), None, false);
    // Qtum's mature confirmations is 500 blocks
    coin_fields.conf.mature_confirmations = 500;
    let arc: UtxoArc = coin_fields.into();
    let coin = QtumCoin::from(arc);

    let empty_output = SignedTransactionOutput {
        value: Some(0.),
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
        value: Some(117.02430015),
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

#[test]
#[ignore]
// TODO it fails at least when fee is 2055837 sat per kbyte, need to investigate
fn test_get_sender_trade_fee_dynamic_tx_fee() {
    let rpc_client = electrum_client_for_test(&["electrum1.cipig.net:10071"]);
    let mut coin_fields = utxo_coin_fields_for_test(
        UtxoRpcClientEnum::Electrum(rpc_client),
        Some("bob passphrase max taker vol with dynamic trade fee"),
        false,
    );
    coin_fields.tx_fee = TxFee::Dynamic(EstimateFeeMethod::Standard);
    let coin = utxo_coin_from_fields(coin_fields);
    let my_balance = coin.my_spendable_balance().wait().expect("!my_balance");
    let expected_balance = BigDecimal::from_str("2.22222").expect("!BigDecimal::from_str");
    assert_eq!(my_balance, expected_balance);

    let fee1 = coin
        .get_sender_trade_fee(
            TradePreimageValue::UpperBound(my_balance.clone()),
            FeeApproxStage::WithoutApprox,
        )
        .wait()
        .expect("!get_sender_trade_fee");

    let value_without_fee = &my_balance - &fee1.amount.to_decimal();
    log!("value_without_fee "(value_without_fee));
    let fee2 = coin
        .get_sender_trade_fee(
            TradePreimageValue::Exact(value_without_fee),
            FeeApproxStage::WithoutApprox,
        )
        .wait()
        .expect("!get_sender_trade_fee");
    assert_eq!(fee1, fee2);

    // `2.21934443` value was obtained as a result of executing the `max_taker_vol` RPC call for this wallet
    let max_taker_vol = BigDecimal::from_str("2.21934443").expect("!BigDecimal::from_str");
    let fee3 = coin
        .get_sender_trade_fee(TradePreimageValue::Exact(max_taker_vol), FeeApproxStage::WithoutApprox)
        .wait()
        .expect("!get_sender_trade_fee");
    assert_eq!(fee1, fee3);
}

#[test]
fn test_validate_fee_wrong_sender() {
    let rpc_client = electrum_client_for_test(&[
        "electrum1.cipig.net:10018",
        "electrum2.cipig.net:10018",
        "electrum3.cipig.net:10018",
    ]);
    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Electrum(rpc_client), None, false);
    // https://morty.explorer.dexstats.info/tx/fe4b0e1c4537e22f2956b5b74513fc936ebd87ada21513e850899cb07a45d475
    let tx_bytes = hex::decode("0400008085202f890199cc492c24cc617731d13cff0ef22e7b0c277a64e7368a615b46214424a1c894020000006a473044022071edae37cf518e98db3f7637b9073a7a980b957b0c7b871415dbb4898ec3ebdc022031b402a6b98e64ffdf752266449ca979a9f70144dba77ed7a6a25bfab11648f6012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0202290200000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac8a96e70b000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac8afb2c60000000000000000000000000000000").unwrap();
    let taker_fee_tx = coin.tx_enum_from_bytes(&tx_bytes).unwrap();
    let amount: BigDecimal = "0.0014157".parse().unwrap();
    let validate_err = coin
        .validate_fee(
            &taker_fee_tx,
            &*DEX_FEE_ADDR_RAW_PUBKEY,
            &*DEX_FEE_ADDR_RAW_PUBKEY,
            &amount,
            0,
            &[],
        )
        .wait()
        .unwrap_err();
    assert!(validate_err.contains("was sent from wrong address"));
}

#[test]
fn test_validate_fee_min_block() {
    let rpc_client = electrum_client_for_test(&[
        "electrum1.cipig.net:10018",
        "electrum2.cipig.net:10018",
        "electrum3.cipig.net:10018",
    ]);
    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Electrum(rpc_client), None, false);
    // https://morty.explorer.dexstats.info/tx/fe4b0e1c4537e22f2956b5b74513fc936ebd87ada21513e850899cb07a45d475
    let tx_bytes = hex::decode("0400008085202f890199cc492c24cc617731d13cff0ef22e7b0c277a64e7368a615b46214424a1c894020000006a473044022071edae37cf518e98db3f7637b9073a7a980b957b0c7b871415dbb4898ec3ebdc022031b402a6b98e64ffdf752266449ca979a9f70144dba77ed7a6a25bfab11648f6012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0202290200000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac8a96e70b000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac8afb2c60000000000000000000000000000000").unwrap();
    let taker_fee_tx = coin.tx_enum_from_bytes(&tx_bytes).unwrap();
    let amount: BigDecimal = "0.0014157".parse().unwrap();
    let sender_pub = hex::decode("03ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fa").unwrap();
    let validate_err = coin
        .validate_fee(
            &taker_fee_tx,
            &sender_pub,
            &*DEX_FEE_ADDR_RAW_PUBKEY,
            &amount,
            810329,
            &[],
        )
        .wait()
        .unwrap_err();
    assert!(validate_err.contains("confirmed before min_block"));
}

#[test]
// https://github.com/KomodoPlatform/atomicDEX-API/issues/857
fn test_validate_fee_bch_70_bytes_signature() {
    let rpc_client = electrum_client_for_test(&[
        "electrum1.cipig.net:10055",
        "electrum2.cipig.net:10055",
        "electrum3.cipig.net:10055",
    ]);
    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Electrum(rpc_client), None, false);
    // https://blockchair.com/bitcoin-cash/transaction/ccee05a6b5bbc6f50d2a65a5a3a04690d3e2d81082ad57d3ab471189f53dd70d
    let tx_bytes = hex::decode("0100000002cae89775f264e50f14238be86a7184b7f77bfe26f54067b794c546ec5eb9c91a020000006b483045022100d6ed080f722a0637a37552382f462230cc438984bc564bdb4b7094f06cfa38fa022062304a52602df1fbb3bebac4f56e1632ad456f62d9031f4983f07e546c8ec4d8412102ae7dc4ef1b49aadeff79cfad56664105f4d114e1716bc4f930cb27dbd309e521ffffffff11f386a6fe8f0431cb84f549b59be00f05e78f4a8a926c5e023a0d5f9112e8200000000069463043021f17eb93ed20a6f2cd357eabb41a4ec6329000ddc6d5b42ecbe642c5d41b206a022026bc4920c4ce3af751283574baa8e4a3efd4dad0d8fe6ba3ddf5d75628d36fda412102ae7dc4ef1b49aadeff79cfad56664105f4d114e1716bc4f930cb27dbd309e521ffffffff0210270000000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac57481c00000000001976a914bac11ce4cd2b1df2769c470d09b54f86df737e3c88ac035b4a60").unwrap();
    let taker_fee_tx = coin.tx_enum_from_bytes(&tx_bytes).unwrap();
    let amount: BigDecimal = "0.0001".parse().unwrap();
    let sender_pub = hex::decode("02ae7dc4ef1b49aadeff79cfad56664105f4d114e1716bc4f930cb27dbd309e521").unwrap();
    coin.validate_fee(&taker_fee_tx, &sender_pub, &*DEX_FEE_ADDR_RAW_PUBKEY, &amount, 0, &[])
        .wait()
        .unwrap();
}

#[test]
fn firo_verbose_block_deserialize() {
    let json = json!({
       "hash":"e21ea157b142270ba479a0aeb5571144b2a06f66a693c20675c624a6f211de0a",
       "confirmations":1,
       "strippedsize":234913,
       "size":234913,
       "weight":234913,
       "height":348355,
       "version":536875008,
       "versionHex":"20001000",
       "merkleroot":"b7fa3ce26f5b493397302c260905ca6f8c9ade56cab7cb314dc6f8a1d4c69245",
       "tx":[
          "166d2e6c6b8e1f29192737be5b0df79f7ccb286a898a3bf7253aa091e1002756",
          "f0bcbf10f2aa20d6891c14fdf64eb336df2d4466ebbc6bd5349c61478be77bd3",
          "0305f0fed2286b4504907bd2588dec5205f0807f11d003489b6748437728b6dc",
          "17f69f35b125de65e140de9bffe873702a4550379fb0ae4fe371f703c739e268",
          "ca60309ee4f846f607295aabcea2d0680ca23a7fbb8699ad1b597255ad6c5a73",
          "5aec101f7b2452d293c1a1c3889861bc8e96081f3ecd328859bc005c14d2737e",
          "bd9a8a2fdbad3db6c38e6472fd2e50d452a98553c8a105cb10afc85b5eaadee0",
          "0a52a67bf6ca3784f81b828616cda6bdca314402cded278d98f94b546784a58d",
          "55e6f918b2e7af2886499919b1c4a2ba341180934a4691a1a7166d6dadfcf8b9",
          "7a2d8b10b3bfc3037ee884699ca4770d96575b2d39179801d760d1c86377ff58",
          "ded160f1ec3e978daa2d8adb0b611223946db1c1155522cf9f0796e6f6c081fe"
       ],
       "cbTx":{
          "version":2,
          "height":348355,
          "merkleRootMNList":"5bd9041001ba65e1aea7a8d3982bb7fc2a8a561a1898d4e176a2cc4d242107b0",
          "merkleRootQuorums":"bfe0f35ec169f3b96eb66097138e70d1e52a66a2fc31a057df6298bbbc790fce"
       },
       "time":1614002775,
       "mediantime":1614001062,
       "nonce":43516489,
       "bits":"1b6d4183",
       "difficulty":599.8302783653238,
       "chainwork":"000000000000000000000000000000000000000000000000bb39407cfc6d253a",
       "previousblockhash":"71b81ff345f062e5c6eacbda63f64295590667a8d72428e4e71011675fe531e1",
       "chainlock":true
    });
    let _block: VerboseBlock = json::from_value(json).unwrap();
}

#[test]
fn firo_lelantus_tx() {
    // https://explorer.firo.org/tx/06ed4b75010edcf404a315be70903473f44050c978bc37fbcee90e0b49114ba8
    let tx_hash = "06ed4b75010edcf404a315be70903473f44050c978bc37fbcee90e0b49114ba8".into();
    let electrum = electrum_client_for_test(&[
        "electrumx01.firo.org:50001",
        "electrumx02.firo.org:50001",
        "electrumx03.firo.org:50001",
    ]);
    let _tx = electrum.get_verbose_transaction(&tx_hash).wait().unwrap();
}

#[test]
fn firo_lelantus_tx_details() {
    // https://explorer.firo.org/tx/06ed4b75010edcf404a315be70903473f44050c978bc37fbcee90e0b49114ba8
    let electrum = electrum_client_for_test(&[
        "electrumx01.firo.org:50001",
        "electrumx02.firo.org:50001",
        "electrumx03.firo.org:50001",
    ]);
    let coin = utxo_coin_for_test(electrum.into(), None, false);
    let mut map = HashMap::new();

    let tx_hash = hex::decode("ad812911f5cba3eab7c193b6cd7020ea02fb5c25634ae64959c3171a6bd5a74d").unwrap();
    let tx_details = block_on(coin.tx_details_by_hash(&tx_hash, &mut map)).unwrap();

    let expected_fee = TxFeeDetails::Utxo(UtxoFeeDetails {
        coin: Some(TEST_COIN_NAME.into()),
        amount: "0.00003793".parse().unwrap(),
    });
    assert_eq!(Some(expected_fee), tx_details.fee_details);

    let tx_hash = hex::decode("06ed4b75010edcf404a315be70903473f44050c978bc37fbcee90e0b49114ba8").unwrap();
    let tx_details = block_on(coin.tx_details_by_hash(&tx_hash, &mut map)).unwrap();

    let expected_fee = TxFeeDetails::Utxo(UtxoFeeDetails {
        coin: Some(TEST_COIN_NAME.into()),
        amount: "0.00045778".parse().unwrap(),
    });
    assert_eq!(Some(expected_fee), tx_details.fee_details);
}

#[test]
fn test_generate_tx_doge_fee() {
    // A tx below 1kb is always 0,01 doge fee per kb.
    let config = json!({
        "coin": "DOGE",
        "name": "dogecoin",
        "fname": "Dogecoin",
        "rpcport": 22555,
        "pubtype": 30,
        "p2shtype": 22,
        "wiftype": 158,
        "txfee": 1000000,
        "force_min_relay_fee": true,
        "mm2": 1,
        "required_confirmations": 2,
        "avg_blocktime": 1,
        "protocol": {
            "type": "UTXO"
        }
    });
    let request = json!({
        "method": "electrum",
        "coin": "DOGE",
        "servers": [{"url": "electrum1.cipig.net:10060"},{"url": "electrum2.cipig.net:10060"},{"url": "electrum3.cipig.net:10060"}],
    });
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&request).unwrap();

    let doge = block_on(utxo_standard_coin_with_priv_key(
        &ctx, "DOGE", &config, &params, &[1; 32],
    ))
    .unwrap();

    let unspents = vec![UnspentInfo {
        outpoint: Default::default(),
        value: 1000000000000,
        height: None,
    }];
    let outputs = vec![TransactionOutput {
        value: 100000000,
        script_pubkey: vec![0; 26].into(),
    }];
    let builder = UtxoTxBuilder::new(&doge)
        .add_available_inputs(unspents)
        .add_outputs(outputs);
    let (_, data) = block_on(builder.build()).unwrap();
    let expected_fee = 1000000;
    assert_eq!(expected_fee, data.fee_amount);

    let unspents = vec![UnspentInfo {
        outpoint: Default::default(),
        value: 1000000000000,
        height: None,
    }];
    let outputs = vec![
        TransactionOutput {
            value: 100000000,
            script_pubkey: vec![0; 26].into(),
        }
        .clone();
        40
    ];

    let builder = UtxoTxBuilder::new(&doge)
        .add_available_inputs(unspents)
        .add_outputs(outputs);
    let (_, data) = block_on(builder.build()).unwrap();
    let expected_fee = 2000000;
    assert_eq!(expected_fee, data.fee_amount);

    let unspents = vec![UnspentInfo {
        outpoint: Default::default(),
        value: 1000000000000,
        height: None,
    }];
    let outputs = vec![
        TransactionOutput {
            value: 100000000,
            script_pubkey: vec![0; 26].into(),
        }
        .clone();
        60
    ];

    let builder = UtxoTxBuilder::new(&doge)
        .add_available_inputs(unspents)
        .add_outputs(outputs);
    let (_, data) = block_on(builder.build()).unwrap();
    let expected_fee = 3000000;
    assert_eq!(expected_fee, data.fee_amount);
}

#[test]
fn doge_mtp() {
    let electrum = electrum_client_for_test(&[
        "electrum1.cipig.net:10060",
        "electrum2.cipig.net:10060",
        "electrum3.cipig.net:10060",
    ]);
    let mtp = electrum
        .get_median_time_past(3631820, NonZeroU64::new(11).unwrap(), CoinVariant::Standard)
        .wait()
        .unwrap();
    assert_eq!(mtp, 1614849084);
}

#[test]
fn firo_mtp() {
    let electrum = electrum_client_for_test(&[
        "electrumx01.firo.org:50001",
        "electrumx02.firo.org:50001",
        "electrumx03.firo.org:50001",
    ]);
    let mtp = electrum
        .get_median_time_past(356730, NonZeroU64::new(11).unwrap(), CoinVariant::Standard)
        .wait()
        .unwrap();
    assert_eq!(mtp, 1616492629);
}

#[test]
fn verus_mtp() {
    let electrum = electrum_client_for_test(&["el0.verus.io:17485", "el1.verus.io:17485", "el2.verus.io:17485"]);
    let mtp = electrum
        .get_median_time_past(1480113, NonZeroU64::new(11).unwrap(), CoinVariant::Standard)
        .wait()
        .unwrap();
    assert_eq!(mtp, 1618579909);
}

#[test]
fn sys_mtp() {
    let electrum = electrum_client_for_test(&[
        "electrum1.cipig.net:10064",
        "electrum2.cipig.net:10064",
        "electrum3.cipig.net:10064",
    ]);
    let mtp = electrum
        .get_median_time_past(1006678, NonZeroU64::new(11).unwrap(), CoinVariant::Standard)
        .wait()
        .unwrap();
    assert_eq!(mtp, 1620019628);
}

#[test]
fn btc_mtp() {
    let electrum = electrum_client_for_test(&[
        "electrum1.cipig.net:10000",
        "electrum2.cipig.net:10000",
        "electrum3.cipig.net:10000",
    ]);
    let mtp = electrum
        .get_median_time_past(681659, NonZeroU64::new(11).unwrap(), CoinVariant::Standard)
        .wait()
        .unwrap();
    assert_eq!(mtp, 1620019527);
}

#[test]
fn rvn_mtp() {
    let electrum = electrum_client_for_test(&[
        "electrum1.cipig.net:10051",
        "electrum2.cipig.net:10051",
        "electrum3.cipig.net:10051",
    ]);
    let mtp = electrum
        .get_median_time_past(1968120, NonZeroU64::new(11).unwrap(), CoinVariant::Standard)
        .wait()
        .unwrap();
    assert_eq!(mtp, 1633946264);
}

#[test]
fn qtum_mtp() {
    let electrum = electrum_client_for_test(&[
        "electrum1.cipig.net:10050",
        "electrum2.cipig.net:10050",
        "electrum3.cipig.net:10050",
    ]);
    let mtp = electrum
        .get_median_time_past(681659, NonZeroU64::new(11).unwrap(), CoinVariant::Qtum)
        .wait()
        .unwrap();
    assert_eq!(mtp, 1598854128);
}

#[test]
fn zer_mtp() {
    let electrum = electrum_client_for_test(&[
        "electrum1.cipig.net:10065",
        "electrum2.cipig.net:10065",
        "electrum3.cipig.net:10065",
    ]);
    let mtp = electrum
        .get_median_time_past(1130915, NonZeroU64::new(11).unwrap(), CoinVariant::Standard)
        .wait()
        .unwrap();
    assert_eq!(mtp, 1623240214);
}

#[test]
#[ignore]
fn test_tx_details_kmd_rewards() {
    let electrum = electrum_client_for_test(&[
        "electrum1.cipig.net:10001",
        "electrum2.cipig.net:10001",
        "electrum3.cipig.net:10001",
    ]);
    let mut fields = utxo_coin_fields_for_test(electrum.into(), None, false);
    fields.conf.ticker = "KMD".to_owned();
    fields.derivation_method = DerivationMethod::Iguana(Address::from("RMGJ9tRST45RnwEKHPGgBLuY3moSYP7Mhk"));
    let coin = utxo_coin_from_fields(fields);

    let mut input_transactions = HistoryUtxoTxMap::new();
    let hash = hex::decode("535ffa3387d3fca14f4a4d373daf7edf00e463982755afce89bc8c48d8168024").unwrap();
    let tx_details = block_on(coin.tx_details_by_hash(&hash, &mut input_transactions)).expect("!tx_details_by_hash");

    let expected_fee = TxFeeDetails::Utxo(UtxoFeeDetails {
        coin: Some("KMD".into()),
        amount: BigDecimal::from_str("0.00001").unwrap(),
    });
    assert_eq!(tx_details.fee_details, Some(expected_fee));

    let expected_kmd_rewards = KmdRewardsDetails {
        amount: BigDecimal::from_str("0.10431954").unwrap(),
        claimed_by_me: true,
    };
    assert_eq!(tx_details.kmd_rewards, Some(expected_kmd_rewards));
}

/// If the ticker is `KMD` AND no rewards were accrued due to a value less than 10 or for any other reasons,
/// then `TransactionDetails::kmd_rewards` has to be `Some(0)`, not `None`.
/// https://kmdexplorer.io/tx/f09e8894959e74c1e727ffa5a753a30bf2dc6d5d677cc1f24b7ee5bb64e32c7d
#[test]
#[ignore]
#[cfg(not(target_arch = "wasm32"))]
fn test_tx_details_kmd_rewards_claimed_by_other() {
    let electrum = electrum_client_for_test(&[
        "electrum1.cipig.net:10001",
        "electrum2.cipig.net:10001",
        "electrum3.cipig.net:10001",
    ]);
    let mut fields = utxo_coin_fields_for_test(electrum.into(), None, false);
    fields.conf.ticker = "KMD".to_owned();
    fields.derivation_method = DerivationMethod::Iguana(Address::from("RMGJ9tRST45RnwEKHPGgBLuY3moSYP7Mhk"));
    let coin = utxo_coin_from_fields(fields);

    let mut input_transactions = HistoryUtxoTxMap::new();
    let hash = hex::decode("f09e8894959e74c1e727ffa5a753a30bf2dc6d5d677cc1f24b7ee5bb64e32c7d").unwrap();
    let tx_details = block_on(coin.tx_details_by_hash(&hash, &mut input_transactions)).expect("!tx_details_by_hash");

    let expected_fee = TxFeeDetails::Utxo(UtxoFeeDetails {
        coin: Some("KMD".into()),
        amount: BigDecimal::from_str("0.00001").unwrap(),
    });
    assert_eq!(tx_details.fee_details, Some(expected_fee));

    let expected_kmd_rewards = KmdRewardsDetails {
        amount: BigDecimal::from_str("0.00022428").unwrap(),
        claimed_by_me: false,
    };
    assert_eq!(tx_details.kmd_rewards, Some(expected_kmd_rewards));
}

#[test]
fn test_tx_details_bch_no_rewards() {
    let electrum = electrum_client_for_test(&[
        "electroncash.de:50003",
        "tbch.loping.net:60001",
        "blackie.c3-soft.com:60001",
        "bch0.kister.net:51001",
        "testnet.imaginary.cash:50001",
    ]);
    let coin = utxo_coin_for_test(electrum.into(), None, false);

    let mut input_transactions = HistoryUtxoTxMap::new();
    let hash = hex::decode("eb13d926f15cbb896e0bcc7a1a77a4ec63504e57a1524c13a7a9b80f43ecb05c").unwrap();
    let tx_details = block_on(coin.tx_details_by_hash(&hash, &mut input_transactions)).expect("!tx_details_by_hash");

    let expected_fee = TxFeeDetails::Utxo(UtxoFeeDetails {
        coin: Some(TEST_COIN_NAME.into()),
        amount: BigDecimal::from_str("0.00000452").unwrap(),
    });
    assert_eq!(tx_details.fee_details, Some(expected_fee));
    assert_eq!(tx_details.kmd_rewards, None);
}

#[test]
fn test_update_kmd_rewards() {
    // 535ffa3387d3fca14f4a4d373daf7edf00e463982755afce89bc8c48d8168024
    const OUTDATED_TX_DETAILS: &str = r#"{"tx_hex":"0400008085202f8901afcadb73880bc1c9e7ce96b8274c2e2a4547415e649f425f98791685be009b73020000006b483045022100b8fbb77efea482b656ad16fc53c5a01d289054c2e429bf1d7bab16c3e822a83602200b87368a95c046b2ce6d0d092185138a3f234a7eb0d7f8227b196ef32358b93f012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff01dd15c293000000001976a91483762a373935ca241d557dfce89171d582b486de88ac99fe9960000000000000000000000000000000","tx_hash":"535ffa3387d3fca14f4a4d373daf7edf00e463982755afce89bc8c48d8168024","from":["RMGJ9tRST45RnwEKHPGgBLuY3moSYP7Mhk"],"to":["RMGJ9tRST45RnwEKHPGgBLuY3moSYP7Mhk"],"total_amount":"24.68539379","spent_by_me":"24.68539379","received_by_me":"24.78970333","my_balance_change":"0.10430954","block_height":2387532,"timestamp":1620705483,"fee_details":{"type":"Utxo","amount":"-0.10430954"},"coin":"KMD","internal_id":"535ffa3387d3fca14f4a4d373daf7edf00e463982755afce89bc8c48d8168024"}"#;

    let electrum = electrum_client_for_test(&[
        "electrum1.cipig.net:10001",
        "electrum2.cipig.net:10001",
        "electrum3.cipig.net:10001",
    ]);
    let mut fields = utxo_coin_fields_for_test(electrum.into(), None, false);
    fields.conf.ticker = "KMD".to_owned();
    fields.derivation_method = DerivationMethod::Iguana(Address::from("RMGJ9tRST45RnwEKHPGgBLuY3moSYP7Mhk"));
    let coin = utxo_coin_from_fields(fields);

    let mut input_transactions = HistoryUtxoTxMap::default();
    let mut tx_details: TransactionDetails = json::from_str(OUTDATED_TX_DETAILS).unwrap();
    block_on(coin.update_kmd_rewards(&mut tx_details, &mut input_transactions)).expect("!update_kmd_rewards");

    let expected_rewards = KmdRewardsDetails {
        amount: BigDecimal::from_str("0.10431954").unwrap(),
        claimed_by_me: true,
    };
    assert_eq!(tx_details.kmd_rewards, Some(expected_rewards));

    let expected_fee_details = TxFeeDetails::Utxo(UtxoFeeDetails {
        coin: Some("KMD".into()),
        amount: BigDecimal::from_str("0.00001").unwrap(),
    });
    assert_eq!(tx_details.fee_details, Some(expected_fee_details));
}

#[test]
fn test_update_kmd_rewards_claimed_not_by_me() {
    // The custom 535ffa3387d3fca14f4a4d373daf7edf00e463982755afce89bc8c48d8168024 transaction with the additional 'from' address.
    const OUTDATED_TX_DETAILS: &str = r#"{"tx_hex":"0400008085202f8901afcadb73880bc1c9e7ce96b8274c2e2a4547415e649f425f98791685be009b73020000006b483045022100b8fbb77efea482b656ad16fc53c5a01d289054c2e429bf1d7bab16c3e822a83602200b87368a95c046b2ce6d0d092185138a3f234a7eb0d7f8227b196ef32358b93f012103b1e544ce2d860219bc91314b5483421a553a7b33044659eff0be9214ed58adddffffffff01dd15c293000000001976a91483762a373935ca241d557dfce89171d582b486de88ac99fe9960000000000000000000000000000000","tx_hash":"535ffa3387d3fca14f4a4d373daf7edf00e463982755afce89bc8c48d8168024","from":["RMGJ9tRST45RnwEKHPGgBLuY3moSYP7Mhk", "RMDc4fvQeekJwrXxuaw1R2b7CTPEuVguMP"],"to":["RMGJ9tRST45RnwEKHPGgBLuY3moSYP7Mhk"],"total_amount":"24.68539379","spent_by_me":"24.68539379","received_by_me":"24.78970333","my_balance_change":"0.10430954","block_height":2387532,"timestamp":1620705483,"fee_details":{"type":"Utxo","amount":"-0.10430954"},"coin":"KMD","internal_id":"535ffa3387d3fca14f4a4d373daf7edf00e463982755afce89bc8c48d8168024"}"#;

    let electrum = electrum_client_for_test(&[
        "electrum1.cipig.net:10001",
        "electrum2.cipig.net:10001",
        "electrum3.cipig.net:10001",
    ]);
    let mut fields = utxo_coin_fields_for_test(electrum.into(), None, false);
    fields.conf.ticker = "KMD".to_owned();
    fields.derivation_method = DerivationMethod::Iguana(Address::from("RMGJ9tRST45RnwEKHPGgBLuY3moSYP7Mhk"));
    let coin = utxo_coin_from_fields(fields);

    let mut input_transactions = HistoryUtxoTxMap::default();
    let mut tx_details: TransactionDetails = json::from_str(OUTDATED_TX_DETAILS).unwrap();
    block_on(coin.update_kmd_rewards(&mut tx_details, &mut input_transactions)).expect("!update_kmd_rewards");

    let expected_rewards = KmdRewardsDetails {
        amount: BigDecimal::from_str("0.10431954").unwrap(),
        claimed_by_me: false,
    };
    assert_eq!(tx_details.kmd_rewards, Some(expected_rewards));

    let expected_fee_details = TxFeeDetails::Utxo(UtxoFeeDetails {
        coin: Some("KMD".into()),
        amount: BigDecimal::from_str("0.00001").unwrap(),
    });
    assert_eq!(tx_details.fee_details, Some(expected_fee_details));
}

/// https://github.com/KomodoPlatform/atomicDEX-API/issues/966
#[test]
fn test_parse_tx_with_huge_locktime() {
    let verbose = r#"{"hex":"0400008085202f89010c03a2b3d8f97139a623f0759224c657513752b705b5c689a256d52b8f8279f200000000d8483045022100fa07821f4739890fa3518c73ecb4917f4a8e7a1c7a803a0d0aea28f991f14f84022041ac557507d6c9786128828c7b2fca7d5c345ba57c8050e3edb29be0c1e5d2660120bdb3d550a68dfaeebe4c416e5750d20d27617bbfb29756843d605a0570ae787b004c6b63046576ba60b17521039ef1b42c635c32440099910bbe1c5e8b0c9373274c3f21cf1003750fc88d3499ac6782012088a914a4f9f1009dcb778bf1c26052258284b32c9075098821031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ac68ffffffff014ddbf305000000001976a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88acf5b98899000000000000000000000000000000","txid":"3b666753b77e28da8a4d858339825315f32516cc147fa743329c7248bd0c6902","overwintered":true,"version":4,"versiongroupid":"892f2085","locktime":2575874549,"expiryheight":0,"vin":[{"txid":"f279828f2bd556a289c6b505b752375157c6249275f023a63971f9d8b3a2030c","vout":0,"scriptSig":{"asm":"3045022100fa07821f4739890fa3518c73ecb4917f4a8e7a1c7a803a0d0aea28f991f14f84022041ac557507d6c9786128828c7b2fca7d5c345ba57c8050e3edb29be0c1e5d266[ALL]bdb3d550a68dfaeebe4c416e5750d20d27617bbfb29756843d605a0570ae787b063046576ba60b17521039ef1b42c635c32440099910bbe1c5e8b0c9373274c3f21cf1003750fc88d3499ac6782012088a914a4f9f1009dcb778bf1c26052258284b32c9075098821031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ac68","hex":"483045022100fa07821f4739890fa3518c73ecb4917f4a8e7a1c7a803a0d0aea28f991f14f84022041ac557507d6c9786128828c7b2fca7d5c345ba57c8050e3edb29be0c1e5d2660120bdb3d550a68dfaeebe4c416e5750d20d27617bbfb29756843d605a0570ae787b004c6b63046576ba60b17521039ef1b42c635c32440099910bbe1c5e8b0c9373274c3f21cf1003750fc88d3499ac6782012088a914a4f9f1009dcb778bf1c26052258284b32c9075098821031bb83b58ec130e28e0a6d5d2acf2eb01b0d3f1670e021d47d31db8a858219da8ac68"},"sequence":4294967295}],"vout":[{"value":0.99867469,"valueZat":99867469,"valueSat":99867469,"n":0,"scriptPubKey":{"asm":"OP_DUPOP_HASH160c3f710deb7320b0efa6edb14e3ebeeb9155fa90dOP_EQUALVERIFYOP_CHECKSIG","hex":"76a914c3f710deb7320b0efa6edb14e3ebeeb9155fa90d88ac","reqSigs":1,"type":"pubkeyhash","addresses":["t1bjmkBWkzLWk3mHFoybXE5daGRY9pk1fxF"]}}],"vjoinsplit":[],"valueBalance":0.0,"valueBalanceZat":0,"vShieldedSpend":[],"vShieldedOutput":[],"blockhash":"0000077e33e838d9967427018a6e7049d8619ae556acb3e80c070990e90b67fc","height":1127478,"confirmations":2197,"time":1622825622,"blocktime":1622825622}"#;
    let verbose_tx: RpcTransaction = json::from_str(verbose).expect("!json::from_str");
    let _: UtxoTx = deserialize(verbose_tx.hex.as_slice()).unwrap();
}

#[test]
fn tbch_electroncash_verbose_tx() {
    let verbose = r#"{"blockhash":"00000000000d93dbc9c6e95c37044d584be959d24e514533b3a82f0f61dddc03","blocktime":1626262632,"confirmations":3708,"hash":"e64531613f909647651ac3f8fd72f3e6f72ac6e01c5a1d923884a10476f56a7f","height":1456230,"hex":"0100000002ebc10f58f220ec1bad5d634684ae649aa7bdd2f9c9081d36e5384e579caa95c2020000006a4730440220639ac218f572520c7d8addae74be6bfdefa9c86bc91474b6dedd7e117d232085022015a92f45f9ae5cee08c188e01fc614b77c461a41733649a55abfcc3e7ca207444121036879df230663db4cd083c8eeb0f293f46abc460ad3c299b0089b72e6d472202cffffffffebc10f58f220ec1bad5d634684ae649aa7bdd2f9c9081d36e5384e579caa95c2030000006a47304402204c27a2c04df44f34bd71ec69cc0a24291a96f265217473affb3c3fce2dbd937202202c2ad2e6cfaac3901c807d9b048ccb2b5e7b0dbd922f2066e637f6bbf459313a4121036879df230663db4cd083c8eeb0f293f46abc460ad3c299b0089b72e6d472202cffffffff040000000000000000406a04534c500001010453454e4420bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb70800000000000003e808000000000000f5fee80300000000000017a9146569d9a853a1934c642223a9432f18c3b3f2a64b87e8030000000000001976a9148cfffc2409d063437d6aa8b75a009b9ba51b71fc88ac67a84601000000001976a9148cfffc2409d063437d6aa8b75a009b9ba51b71fc88ac87caee60","locktime":1626262151,"size":477,"time":1626262632,"txid":"e64531613f909647651ac3f8fd72f3e6f72ac6e01c5a1d923884a10476f56a7f","version":1,"vin":[{"coinbase":null,"scriptSig":{"asm":"OP_PUSHBYTES_71 30440220639ac218f572520c7d8addae74be6bfdefa9c86bc91474b6dedd7e117d232085022015a92f45f9ae5cee08c188e01fc614b77c461a41733649a55abfcc3e7ca2074441 OP_PUSHBYTES_33 036879df230663db4cd083c8eeb0f293f46abc460ad3c299b0089b72e6d472202c","hex":"4730440220639ac218f572520c7d8addae74be6bfdefa9c86bc91474b6dedd7e117d232085022015a92f45f9ae5cee08c188e01fc614b77c461a41733649a55abfcc3e7ca207444121036879df230663db4cd083c8eeb0f293f46abc460ad3c299b0089b72e6d472202c"},"sequence":4294967295,"txid":"c295aa9c574e38e5361d08c9f9d2bda79a64ae8446635dad1bec20f2580fc1eb","vout":2},{"coinbase":null,"scriptSig":{"asm":"OP_PUSHBYTES_71 304402204c27a2c04df44f34bd71ec69cc0a24291a96f265217473affb3c3fce2dbd937202202c2ad2e6cfaac3901c807d9b048ccb2b5e7b0dbd922f2066e637f6bbf459313a41 OP_PUSHBYTES_33 036879df230663db4cd083c8eeb0f293f46abc460ad3c299b0089b72e6d472202c","hex":"47304402204c27a2c04df44f34bd71ec69cc0a24291a96f265217473affb3c3fce2dbd937202202c2ad2e6cfaac3901c807d9b048ccb2b5e7b0dbd922f2066e637f6bbf459313a4121036879df230663db4cd083c8eeb0f293f46abc460ad3c299b0089b72e6d472202c"},"sequence":4294967295,"txid":"c295aa9c574e38e5361d08c9f9d2bda79a64ae8446635dad1bec20f2580fc1eb","vout":3}],"vout":[{"n":0,"scriptPubKey":{"addresses":[],"asm":"OP_RETURN OP_PUSHBYTES_4 534c5000 OP_PUSHBYTES_1 01 OP_PUSHBYTES_4 53454e44 OP_PUSHBYTES_32 bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb7 OP_PUSHBYTES_8 00000000000003e8 OP_PUSHBYTES_8 000000000000f5fe","hex":"6a04534c500001010453454e4420bb309e48930671582bea508f9a1d9b491e49b69be3d6f372dc08da2ac6e90eb70800000000000003e808000000000000f5fe","type":"nulldata"},"value_coin":0.0,"value_satoshi":0},{"n":1,"scriptPubKey":{"addresses":["bchtest:ppjknkdg2wsexnryyg36jse0rrpm8u4xfv9hwa0rgl"],"asm":"OP_HASH160 OP_PUSHBYTES_20 6569d9a853a1934c642223a9432f18c3b3f2a64b OP_EQUAL","hex":"a9146569d9a853a1934c642223a9432f18c3b3f2a64b87","type":"scripthash"},"value_coin":0.00001,"value_satoshi":1000},{"n":2,"scriptPubKey":{"addresses":["bchtest:qzx0llpyp8gxxsmad25twksqnwd62xm3lsnnczzt66"],"asm":"OP_DUP OP_HASH160 OP_PUSHBYTES_20 8cfffc2409d063437d6aa8b75a009b9ba51b71fc OP_EQUALVERIFY OP_CHECKSIG","hex":"76a9148cfffc2409d063437d6aa8b75a009b9ba51b71fc88ac","type":"pubkeyhash"},"value_coin":0.00001,"value_satoshi":1000},{"n":3,"scriptPubKey":{"addresses":["bchtest:qzx0llpyp8gxxsmad25twksqnwd62xm3lsnnczzt66"],"asm":"OP_DUP OP_HASH160 OP_PUSHBYTES_20 8cfffc2409d063437d6aa8b75a009b9ba51b71fc OP_EQUALVERIFY OP_CHECKSIG","hex":"76a9148cfffc2409d063437d6aa8b75a009b9ba51b71fc88ac","type":"pubkeyhash"},"value_coin":0.21407847,"value_satoshi":21407847}]}"#;
    let _: RpcTransaction = json::from_str(verbose).expect("!json::from_str");
}

#[test]
fn tbch_electroncash_verbose_tx_unconfirmed() {
    let verbose = r#"{"blockhash":null,"blocktime":null,"confirmations":null,"hash":"e5c9ec5013fca3a62fdf880d1a98f1096a00d20ceaeb6a4cb88ddbea6f1e185a","height":null,"hex":"01000000017f6af57604a18438921d5a1ce0c62af7e6f372fdf8c31a654796903f613145e6030000006b483045022100c335dd0f22e047b806a9d84e02b70aab609093e960888f6f1878e605a173e3da02201c274ce4983d8e519a47c4bd17aeca897b084954ce7a9d77033100e06aa999304121036879df230663db4cd083c8eeb0f293f46abc460ad3c299b0089b72e6d472202cffffffff0280969800000000001976a914eed5d3ad264ffc68fc0a6454e1696a30d8f405be88acbe0dae00000000001976a9148cfffc2409d063437d6aa8b75a009b9ba51b71fc88ac7a361261","locktime":1628583546,"size":226,"time":null,"txid":"e5c9ec5013fca3a62fdf880d1a98f1096a00d20ceaeb6a4cb88ddbea6f1e185a","version":1,"vin":[{"coinbase":null,"scriptSig":{"asm":"OP_PUSHBYTES_72 3045022100c335dd0f22e047b806a9d84e02b70aab609093e960888f6f1878e605a173e3da02201c274ce4983d8e519a47c4bd17aeca897b084954ce7a9d77033100e06aa9993041 OP_PUSHBYTES_33 036879df230663db4cd083c8eeb0f293f46abc460ad3c299b0089b72e6d472202c","hex":"483045022100c335dd0f22e047b806a9d84e02b70aab609093e960888f6f1878e605a173e3da02201c274ce4983d8e519a47c4bd17aeca897b084954ce7a9d77033100e06aa999304121036879df230663db4cd083c8eeb0f293f46abc460ad3c299b0089b72e6d472202c"},"sequence":4294967295,"txid":"e64531613f909647651ac3f8fd72f3e6f72ac6e01c5a1d923884a10476f56a7f","vout":3}],"vout":[{"n":0,"scriptPubKey":{"addresses":["bchtest:qrhdt5adye8lc68upfj9fctfdgcd3aq9hctf8ft6md"],"asm":"OP_DUP OP_HASH160 OP_PUSHBYTES_20 eed5d3ad264ffc68fc0a6454e1696a30d8f405be OP_EQUALVERIFY OP_CHECKSIG","hex":"76a914eed5d3ad264ffc68fc0a6454e1696a30d8f405be88ac","type":"pubkeyhash"},"value_coin":0.1,"value_satoshi":10000000},{"n":1,"scriptPubKey":{"addresses":["bchtest:qzx0llpyp8gxxsmad25twksqnwd62xm3lsnnczzt66"],"asm":"OP_DUP OP_HASH160 OP_PUSHBYTES_20 8cfffc2409d063437d6aa8b75a009b9ba51b71fc OP_EQUALVERIFY OP_CHECKSIG","hex":"76a9148cfffc2409d063437d6aa8b75a009b9ba51b71fc88ac","type":"pubkeyhash"},"value_coin":0.11406782,"value_satoshi":11406782}]}"#;
    let _: RpcTransaction = json::from_str(verbose).expect("!json::from_str");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_to_p2pkh() {
    UtxoStandardCoin::list_unspent_ordered.mock_safe(|coin, _| {
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client.clone()), None, false);

    // Create a p2pkh address for the test coin
    let p2pkh_address = Address {
        prefix: coin.as_ref().conf.pub_addr_prefix,
        hash: coin.as_ref().derivation_method.unwrap_iguana().hash.clone(),
        t_addr_prefix: coin.as_ref().conf.pub_t_addr_prefix,
        checksum_type: coin.as_ref().derivation_method.unwrap_iguana().checksum_type,
        hrp: coin.as_ref().conf.bech32_hrp.clone(),
        addr_format: UtxoAddressFormat::Standard,
    };

    let withdraw_req = WithdrawRequest {
        amount: 1.into(),
        from: None,
        to: p2pkh_address.to_string(),
        coin: TEST_COIN_NAME.into(),
        max: false,
        fee: None,
    };
    let tx_details = coin.withdraw(withdraw_req).wait().unwrap();
    let transaction: UtxoTx = deserialize(tx_details.tx_hex.as_slice()).unwrap();
    let output_script: Script = transaction.outputs[0].script_pubkey.clone().into();

    let expected_script = Builder::build_p2pkh(&p2pkh_address.hash);

    assert_eq!(output_script, expected_script);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_to_p2sh() {
    UtxoStandardCoin::list_unspent_ordered.mock_safe(|coin, _| {
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client.clone()), None, false);

    // Create a p2sh address for the test coin
    let p2sh_address = Address {
        prefix: coin.as_ref().conf.p2sh_addr_prefix,
        hash: coin.as_ref().derivation_method.unwrap_iguana().hash.clone(),
        t_addr_prefix: coin.as_ref().conf.p2sh_t_addr_prefix,
        checksum_type: coin.as_ref().derivation_method.unwrap_iguana().checksum_type,
        hrp: coin.as_ref().conf.bech32_hrp.clone(),
        addr_format: UtxoAddressFormat::Standard,
    };

    let withdraw_req = WithdrawRequest {
        amount: 1.into(),
        from: None,
        to: p2sh_address.to_string(),
        coin: TEST_COIN_NAME.into(),
        max: false,
        fee: None,
    };
    let tx_details = coin.withdraw(withdraw_req).wait().unwrap();
    let transaction: UtxoTx = deserialize(tx_details.tx_hex.as_slice()).unwrap();
    let output_script: Script = transaction.outputs[0].script_pubkey.clone().into();

    let expected_script = Builder::build_p2sh(&p2sh_address.hash);

    assert_eq!(output_script, expected_script);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_withdraw_to_p2wpkh() {
    UtxoStandardCoin::list_unspent_ordered.mock_safe(|coin, _| {
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = vec![UnspentInfo {
            outpoint: OutPoint {
                hash: 1.into(),
                index: 0,
            },
            value: 1000000000,
            height: Default::default(),
        }];
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));

    let coin = utxo_coin_for_test(UtxoRpcClientEnum::Native(client.clone()), None, true);

    // Create a p2wpkh address for the test coin
    let p2wpkh_address = Address {
        prefix: coin.as_ref().conf.pub_addr_prefix,
        hash: coin.as_ref().derivation_method.unwrap_iguana().hash.clone(),
        t_addr_prefix: coin.as_ref().conf.pub_t_addr_prefix,
        checksum_type: coin.as_ref().derivation_method.unwrap_iguana().checksum_type,
        hrp: coin.as_ref().conf.bech32_hrp.clone(),
        addr_format: UtxoAddressFormat::Segwit,
    };

    let withdraw_req = WithdrawRequest {
        amount: 1.into(),
        from: None,
        to: p2wpkh_address.to_string(),
        coin: TEST_COIN_NAME.into(),
        max: false,
        fee: None,
    };
    let tx_details = coin.withdraw(withdraw_req).wait().unwrap();
    let transaction: UtxoTx = deserialize(tx_details.tx_hex.as_slice()).unwrap();
    let output_script: Script = transaction.outputs[0].script_pubkey.clone().into();

    let expected_script = Builder::build_witness_script(&p2wpkh_address.hash);

    assert_eq!(output_script, expected_script);
}

/// `UtxoStandardCoin` has to check UTXO maturity if `check_utxo_maturity` is `true`.
/// https://github.com/KomodoPlatform/atomicDEX-API/issues/1181
#[test]
fn test_utxo_standard_with_check_utxo_maturity_true() {
    static mut LIST_MATURE_UNSPENT_ORDERED_CALLED: bool = false;

    UtxoStandardCoin::list_mature_unspent_ordered.mock_safe(|coin, _| {
        unsafe { LIST_MATURE_UNSPENT_ORDERED_CALLED = true };
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = Vec::new();
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let conf = json!({"coin":"RICK","asset":"RICK","rpcport":25435,"txversion":4,"overwintered":1,"mm2":1,"protocol":{"type":"UTXO"}});
    let req = json!({
         "method": "electrum",
         "servers": [
             {"url":"electrum1.cipig.net:10017"},
             {"url":"electrum2.cipig.net:10017"},
             {"url":"electrum3.cipig.net:10017"},
         ],
        "check_utxo_maturity": true,
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();

    let coin = block_on(utxo_standard_coin_with_priv_key(
        &ctx, "RICK", &conf, &params, &[1u8; 32],
    ))
    .unwrap();

    let address = Address::from("R9o9xTocqr6CeEDGDH6mEYpwLoMz6jNjMW");
    // Don't use `block_on` here because it's used within a mock of [`UtxoStandardCoin::list_mature_unspent_ordered`].
    coin.list_unspent_ordered(&address).compat().wait().unwrap();
    assert!(unsafe { LIST_MATURE_UNSPENT_ORDERED_CALLED });
}

/// `UtxoStandardCoin` hasn't to check UTXO maturity if `check_utxo_maturity` is not set.
/// https://github.com/KomodoPlatform/atomicDEX-API/issues/1181
#[test]
fn test_utxo_standard_without_check_utxo_maturity() {
    static mut LIST_ALL_UNSPENT_ORDERED_CALLED: bool = false;

    UtxoStandardCoin::list_all_unspent_ordered.mock_safe(|coin, _| {
        unsafe { LIST_ALL_UNSPENT_ORDERED_CALLED = true };
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = Vec::new();
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    UtxoStandardCoin::list_mature_unspent_ordered.mock_safe(|_, _| {
        panic!("'UtxoStandardCoin::list_mature_unspent_ordered' is not expected to be called when `check_utxo_maturity` is not set")
    });

    let conf = json!({"coin":"RICK","asset":"RICK","rpcport":25435,"txversion":4,"overwintered":1,"mm2":1,"protocol":{"type":"UTXO"}});
    let req = json!({
         "method": "electrum",
         "servers": [
             {"url":"electrum1.cipig.net:10017"},
             {"url":"electrum2.cipig.net:10017"},
             {"url":"electrum3.cipig.net:10017"},
         ]
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();

    let coin = block_on(utxo_standard_coin_with_priv_key(
        &ctx, "RICK", &conf, &params, &[1u8; 32],
    ))
    .unwrap();

    let address = Address::from("R9o9xTocqr6CeEDGDH6mEYpwLoMz6jNjMW");
    // Don't use `block_on` here because it's used within a mock of [`UtxoStandardCoin::list_mature_unspent_ordered`].
    coin.list_unspent_ordered(&address).compat().wait().unwrap();
    assert!(unsafe { LIST_ALL_UNSPENT_ORDERED_CALLED });
}

/// `QtumCoin` has to check UTXO maturity if `check_utxo_maturity` is not set.
/// https://github.com/KomodoPlatform/atomicDEX-API/issues/1181
#[test]
fn test_qtum_without_check_utxo_maturity() {
    static mut LIST_MATURE_UNSPENT_ORDERED_CALLED: bool = false;

    QtumCoin::list_mature_unspent_ordered.mock_safe(|coin, _| {
        unsafe { LIST_MATURE_UNSPENT_ORDERED_CALLED = true };
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = Vec::new();
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });

    let conf = json!({"coin":"tQTUM","rpcport":13889,"pubtype":120,"p2shtype":110});
    let req = json!({
        "method": "electrum",
        "servers": [
            {"url":"electrum1.cipig.net:10071"},
            {"url":"electrum2.cipig.net:10071"},
            {"url":"electrum3.cipig.net:10071"},
        ],
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();

    let coin = block_on(qtum_coin_with_priv_key(&ctx, "QTUM", &conf, &params, &[1u8; 32])).unwrap();

    let address = Address::from("qcyBHeSct7Wr4mAw18iuQ1zW5mMFYmtmBE");
    // Don't use `block_on` here because it's used within a mock of [`QtumCoin::list_mature_unspent_ordered`].
    coin.list_unspent_ordered(&address).compat().wait().unwrap();
    assert!(unsafe { LIST_MATURE_UNSPENT_ORDERED_CALLED });
}

/// `QtumCoin` hasn't to check UTXO maturity if `check_utxo_maturity` is `false`.
/// https://github.com/KomodoPlatform/atomicDEX-API/issues/1181
#[test]
fn test_qtum_with_check_utxo_maturity_false() {
    static mut LIST_ALL_UNSPENT_ORDERED_CALLED: bool = false;

    QtumCoin::list_all_unspent_ordered.mock_safe(|coin, _| {
        unsafe { LIST_ALL_UNSPENT_ORDERED_CALLED = true };
        let cache = block_on(coin.as_ref().recently_spent_outpoints.lock());
        let unspents = Vec::new();
        MockResult::Return(Box::pin(futures::future::ok((unspents, cache))))
    });
    QtumCoin::list_mature_unspent_ordered.mock_safe(|_, _| {
        panic!(
            "'QtumCoin::list_mature_unspent_ordered' is not expected to be called when `check_utxo_maturity` is false"
        )
    });

    let conf = json!({"coin":"tQTUM","rpcport":13889,"pubtype":120,"p2shtype":110});
    let req = json!({
        "method": "electrum",
        "servers": [
            {"url":"electrum1.cipig.net:10071"},
            {"url":"electrum2.cipig.net:10071"},
            {"url":"electrum3.cipig.net:10071"},
        ],
        "check_utxo_maturity": false,
    });

    let ctx = MmCtxBuilder::new().into_mm_arc();
    let params = UtxoActivationParams::from_legacy_req(&req).unwrap();

    let coin = block_on(qtum_coin_with_priv_key(&ctx, "QTUM", &conf, &params, &[1u8; 32])).unwrap();

    let address = Address::from("qcyBHeSct7Wr4mAw18iuQ1zW5mMFYmtmBE");
    // Don't use `block_on` here because it's used within a mock of [`QtumCoin::list_mature_unspent_ordered`].
    coin.list_unspent_ordered(&address).compat().wait().unwrap();
    assert!(unsafe { LIST_ALL_UNSPENT_ORDERED_CALLED });
}

#[test]
fn test_hd_account_balance_rpc() {
    let mut addresses_map: HashMap<String, u64> = HashMap::new();
    let mut balances_by_der_path: HashMap<String, HDAddressBalance> = HashMap::new();

    macro_rules! known_address {
        ($der_path:literal, $address:literal, $chain:expr, balance = $balance:literal) => {
            addresses_map.insert($address.to_string(), $balance);
            balances_by_der_path.insert($der_path.to_string(), HDAddressBalance {
                address: $address.to_string(),
                derivation_path: RpcDerivationPath(DerivationPath::from_str($der_path).unwrap()),
                chain: $chain,
                balance: CoinBalance::new(BigDecimal::from($balance)),
            })
        };
    }

    macro_rules! get_balances {
        ($($der_paths:literal),*) => {
            [$($der_paths),*].iter().map(|der_path| balances_by_der_path.get(*der_path).unwrap().clone()).collect()
        };
    }

    #[rustfmt::skip]
    {
        // Account#0, external addresses.
        known_address!("m/44'/141'/0'/0/0", "RRqF4cYniMwYs66S4QDUUZ4GJQFQF69rBE", Bip44Chain::External, balance = 0);
        known_address!("m/44'/141'/0'/0/1", "RSVLsjXc9LJ8fm9Jq7gXjeubfja3bbgSDf", Bip44Chain::External, balance = 0);
        known_address!("m/44'/141'/0'/0/2", "RSSZjtgfnLzvqF4cZQJJEpN5gvK3pWmd3h", Bip44Chain::External, balance = 0);
        known_address!("m/44'/141'/0'/0/3", "RU1gRFXWXNx7uPRAEJ7wdZAW1RZ4TE6Vv1", Bip44Chain::External, balance = 98);
        known_address!("m/44'/141'/0'/0/4", "RUkEvRzb7mtwfVeKiSFEbYupLkcvU5KJBw", Bip44Chain::External, balance = 1);
        known_address!("m/44'/141'/0'/0/5", "RP8deqVfjBbkvxbGbsQ2EGdamMaP1wxizR", Bip44Chain::External, balance = 0);
        known_address!("m/44'/141'/0'/0/6", "RSvKMMegKGP5e2EanH7fnD4yNsxdJvLAmL", Bip44Chain::External, balance = 32);

        // Account#0, internal addresses.
        known_address!("m/44'/141'/0'/1/0", "RLZxcZSYtKe74JZd1hBAmmD9PNHZqb72oL", Bip44Chain::Internal, balance = 13);
        known_address!("m/44'/141'/0'/1/1", "RPj9JXUVnewWwVpxZDeqGB25qVqz5qJzwP", Bip44Chain::Internal, balance = 44);
        known_address!("m/44'/141'/0'/1/2", "RSYdSLRYWuzBson2GDbWBa632q2PmFnCaH", Bip44Chain::Internal, balance = 10);

        // Account#1, internal addresses.
        known_address!("m/44'/141'/1'/1/0", "RGo7sYzivPtzv8aRQ4A6vRJDxoqkRRBRhZ", Bip44Chain::Internal, balance = 0);
    }

    NativeClient::display_balance.mock_safe(move |_, address: Address, _| {
        let address = address.to_string();
        let balance = addresses_map
            .remove(&address)
            .expect(&format!("Unexpected address: {}", address));
        MockResult::Return(Box::new(futures01::future::ok(BigDecimal::from(balance))))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));
    let mut fields = utxo_coin_fields_for_test(UtxoRpcClientEnum::Native(client), None, false);
    let accounts = PaMutex::new(vec![
        UtxoHDAccount {
            account_id: 0,
            extended_pubkey: Secp256k1ExtendedPublicKey::from_str("xpub6DEHSksajpRPM59RPw7Eg6PKdU7E2ehxJWtYdrfQ6JFmMGBsrR6jA78ANCLgzKYm4s5UqQ4ydLEYPbh3TRVvn5oAZVtWfi4qJLMntpZ8uGJ").unwrap(),
            account_derivation_path: DerivationPath::from_str("m/44'/141'/0'").unwrap(),
            external_addresses_number: 7,
            internal_addresses_number: 3,
        },
        UtxoHDAccount {
            account_id: 1,
            extended_pubkey: Secp256k1ExtendedPublicKey::from_str("xpub6DEHSksajpRPQq2FdGT6JoieiQZUpTZ3WZn8fcuLJhFVmtCpXbuXxp5aPzaokwcLV2V9LE55Dwt8JYkpuMv7jXKwmyD28WbHYjBH2zhbW2p").unwrap(),
            account_derivation_path: DerivationPath::from_str("m/44'/141'/1'").unwrap(),
            external_addresses_number: 0,
            internal_addresses_number: 1,
        },
    ]);
    fields.derivation_method = DerivationMethod::HDWallet(UtxoHDWallet {
        address_format: UtxoAddressFormat::Standard,
        derivation_path: DerivationPath::from_str("m/44'/141'").unwrap(),
        accounts,
        gap_limit: 3,
    });
    let coin = utxo_coin_from_fields(fields);

    // Request a balance of Account#0, external addresses, 1st page

    let params = HDAccountBalanceParams {
        account_index: 0,
        chain: Bip44Chain::External,
        limit: 3,
        paging_options: PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap()),
    };
    let actual = block_on(coin.hd_account_balance_rpc(params)).expect("!hd_account_balance_rpc");
    let expected = HDAccountBalanceResponse {
        account_index: 0,
        derivation_path: DerivationPath::from_str("m/44'/141'/0'").unwrap().into(),
        addresses: get_balances!("m/44'/141'/0'/0/0", "m/44'/141'/0'/0/1", "m/44'/141'/0'/0/2"),
        limit: 3,
        skipped: 0,
        total: 7,
        total_pages: 3,
        paging_options: PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap()),
    };
    assert_eq!(actual, expected);

    // Request a balance of Account#0, external addresses, 2nd page

    let params = HDAccountBalanceParams {
        account_index: 0,
        chain: Bip44Chain::External,
        limit: 3,
        paging_options: PagingOptionsEnum::PageNumber(NonZeroUsize::new(2).unwrap()),
    };
    let actual = block_on(coin.hd_account_balance_rpc(params)).expect("!hd_account_balance_rpc");
    let expected = HDAccountBalanceResponse {
        account_index: 0,
        derivation_path: DerivationPath::from_str("m/44'/141'/0'").unwrap().into(),
        addresses: get_balances!("m/44'/141'/0'/0/3", "m/44'/141'/0'/0/4", "m/44'/141'/0'/0/5"),
        limit: 3,
        skipped: 3,
        total: 7,
        total_pages: 3,
        paging_options: PagingOptionsEnum::PageNumber(NonZeroUsize::new(2).unwrap()),
    };
    assert_eq!(actual, expected);

    // Request a balance of Account#0, external addresses, 3rd page

    let params = HDAccountBalanceParams {
        account_index: 0,
        chain: Bip44Chain::External,
        limit: 3,
        paging_options: PagingOptionsEnum::PageNumber(NonZeroUsize::new(3).unwrap()),
    };
    let actual = block_on(coin.hd_account_balance_rpc(params)).expect("!hd_account_balance_rpc");
    let expected = HDAccountBalanceResponse {
        account_index: 0,
        derivation_path: DerivationPath::from_str("m/44'/141'/0'").unwrap().into(),
        addresses: get_balances!("m/44'/141'/0'/0/6"),
        limit: 3,
        skipped: 6,
        total: 7,
        total_pages: 3,
        paging_options: PagingOptionsEnum::PageNumber(NonZeroUsize::new(3).unwrap()),
    };
    assert_eq!(actual, expected);

    // Request a balance of Account#0, external addresses, page 4 (out of bound)

    let params = HDAccountBalanceParams {
        account_index: 0,
        chain: Bip44Chain::External,
        limit: 3,
        paging_options: PagingOptionsEnum::PageNumber(NonZeroUsize::new(4).unwrap()),
    };
    let actual = block_on(coin.hd_account_balance_rpc(params)).expect("!hd_account_balance_rpc");
    let expected = HDAccountBalanceResponse {
        account_index: 0,
        derivation_path: DerivationPath::from_str("m/44'/141'/0'").unwrap().into(),
        addresses: Vec::new(),
        limit: 3,
        skipped: 7,
        total: 7,
        total_pages: 3,
        paging_options: PagingOptionsEnum::PageNumber(NonZeroUsize::new(4).unwrap()),
    };
    assert_eq!(actual, expected);

    // Request a balance of Account#0, internal addresses, starting from idx=1

    let params = HDAccountBalanceParams {
        account_index: 0,
        chain: Bip44Chain::Internal,
        limit: 3,
        paging_options: PagingOptionsEnum::FromId(1),
    };
    let actual = block_on(coin.hd_account_balance_rpc(params)).expect("!hd_account_balance_rpc");
    let expected = HDAccountBalanceResponse {
        account_index: 0,
        derivation_path: DerivationPath::from_str("m/44'/141'/0'").unwrap().into(),
        addresses: get_balances!("m/44'/141'/0'/1/1", "m/44'/141'/0'/1/2"),
        limit: 3,
        skipped: 1,
        total: 3,
        total_pages: 1,
        paging_options: PagingOptionsEnum::FromId(1),
    };
    assert_eq!(actual, expected);

    // Request a balance of Account#1, external addresses, page 1 (out of bound)

    let params = HDAccountBalanceParams {
        account_index: 1,
        chain: Bip44Chain::External,
        limit: 3,
        paging_options: PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap()),
    };
    let actual = block_on(coin.hd_account_balance_rpc(params)).expect("!hd_account_balance_rpc");
    let expected = HDAccountBalanceResponse {
        account_index: 1,
        derivation_path: DerivationPath::from_str("m/44'/141'/1'").unwrap().into(),
        addresses: Vec::new(),
        limit: 3,
        skipped: 0,
        total: 0,
        total_pages: 0,
        paging_options: PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap()),
    };
    assert_eq!(actual, expected);

    // Request a balance of Account#1, external addresses, page 1

    let params = HDAccountBalanceParams {
        account_index: 1,
        chain: Bip44Chain::Internal,
        limit: 3,
        paging_options: PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap()),
    };
    let actual = block_on(coin.hd_account_balance_rpc(params)).expect("!hd_account_balance_rpc");
    let expected = HDAccountBalanceResponse {
        account_index: 1,
        derivation_path: DerivationPath::from_str("m/44'/141'/1'").unwrap().into(),
        addresses: get_balances!("m/44'/141'/1'/1/0"),
        limit: 3,
        skipped: 0,
        total: 1,
        total_pages: 1,
        paging_options: PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap()),
    };
    assert_eq!(actual, expected);

    // Request a balance of Account#1, external addresses, starting from idx=1 (out of bound)

    let params = HDAccountBalanceParams {
        account_index: 1,
        chain: Bip44Chain::Internal,
        limit: 3,
        paging_options: PagingOptionsEnum::FromId(1),
    };
    let actual = block_on(coin.hd_account_balance_rpc(params)).expect("!hd_account_balance_rpc");
    let expected = HDAccountBalanceResponse {
        account_index: 1,
        derivation_path: DerivationPath::from_str("m/44'/141'/1'").unwrap().into(),
        addresses: Vec::new(),
        limit: 3,
        skipped: 1,
        total: 1,
        total_pages: 1,
        paging_options: PagingOptionsEnum::FromId(1),
    };
    assert_eq!(actual, expected);
}

#[test]
fn test_check_hd_wallet_balance() {
    let mut checking_addresses: HashMap<String, Option<u64>> = HashMap::new();
    let mut non_empty_addresses: Vec<String> = Vec::new();
    let mut balances_by_der_path: HashMap<String, HDAddressBalance> = HashMap::new();

    macro_rules! new_address {
        ($der_path:literal, $address:literal, $chain:expr, balance = $balance:expr) => {{
            let balance = $balance;
            checking_addresses.insert($address.to_string(), balance);
            balances_by_der_path.insert($der_path.to_string(), HDAddressBalance {
                address: $address.to_string(),
                derivation_path: RpcDerivationPath(DerivationPath::from_str($der_path).unwrap()),
                chain: $chain,
                balance: CoinBalance::new(BigDecimal::from(balance.unwrap_or(0))),
            });
            if balance.is_some() {
                non_empty_addresses.push($address.to_string());
            }
        }};
    }

    macro_rules! unused_address {
        ($_der_path:literal, $address:literal) => {
            checking_addresses.insert($address.to_string(), None)
        };
    }

    macro_rules! get_balances {
        ($($der_paths:literal),*) => {
            [$($der_paths),*].iter().map(|der_path| balances_by_der_path.get(*der_path).unwrap().clone()).collect()
        };
    }

    // Please note that the order of the `known` and `new` addresses is important.
    #[rustfmt::skip]
    {
        // Account#0, external addresses.
        new_address!("m/44'/141'/0'/0/3", "RU1gRFXWXNx7uPRAEJ7wdZAW1RZ4TE6Vv1", Bip44Chain::External, balance = Some(98));
        unused_address!("m/44'/141'/0'/0/4", "RUkEvRzb7mtwfVeKiSFEbYupLkcvU5KJBw");
        unused_address!("m/44'/141'/0'/0/5", "RP8deqVfjBbkvxbGbsQ2EGdamMaP1wxizR");
        unused_address!("m/44'/141'/0'/0/6", "RSvKMMegKGP5e2EanH7fnD4yNsxdJvLAmL"); // Stop searching for a non-empty address (gap_limit = 3).

        // Account#0, internal addresses.
        new_address!("m/44'/141'/0'/1/1", "RPj9JXUVnewWwVpxZDeqGB25qVqz5qJzwP", Bip44Chain::Internal, balance = Some(98));
        new_address!("m/44'/141'/0'/1/2", "RSYdSLRYWuzBson2GDbWBa632q2PmFnCaH", Bip44Chain::Internal, balance = None);
        new_address!("m/44'/141'/0'/1/3", "RQstQeTUEZLh6c3YWJDkeVTTQoZUsfvNCr", Bip44Chain::Internal, balance = Some(14));
        unused_address!("m/44'/141'/0'/1/4", "RT54m6pfj9scqwSLmYdfbmPcrpxnWGAe9J");
        unused_address!("m/44'/141'/0'/1/5", "RYWfEFxqA6zya9c891Dj7vxiDojCmuWR9T");
        unused_address!("m/44'/141'/0'/1/6", "RSkY6twW8knTcn6wGACUAG9crJHcuQ2kEH"); // Stop searching for a non-empty address (gap_limit = 3).

        // Account#1, external addresses.
        new_address!("m/44'/141'/1'/0/0", "RBQFLwJ88gVcnfkYvJETeTAB6AAYLow12K", Bip44Chain::External, balance = Some(9));
        new_address!("m/44'/141'/1'/0/1", "RCyy77sRWFa2oiFPpyimeTQfenM1aRoiZs", Bip44Chain::External, balance = Some(7));
        new_address!("m/44'/141'/1'/0/2", "RDnNa3pQmisfi42KiTZrfYfuxkLC91PoTJ", Bip44Chain::External, balance = None);
        new_address!("m/44'/141'/1'/0/3", "RQRGgXcGJz93CoAfQJoLgBz2r9HtJYMX3Z", Bip44Chain::External, balance = None);
        new_address!("m/44'/141'/1'/0/4", "RM6cqSFCFZ4J1LngLzqKkwo2ouipbDZUbm", Bip44Chain::External, balance = Some(11));
        unused_address!("m/44'/141'/1'/0/5", "RX2fGBZjNZMNdNcnc5QBRXvmsXTvadvTPN");
        unused_address!("m/44'/141'/1'/0/6", "RJJ7muUETyp59vxVXna9KAZ9uQ1QSqmcjE");
        unused_address!("m/44'/141'/1'/0/7", "RYJ6vbhxFre5yChCMiJJFNTTBhAQbKM9AY"); // Stop searching for a non-empty address (gap_limit = 3).

        // Account#1, internal addresses.
        unused_address!("m/44'/141'/1'/0/2", "RCjRDibDAXKYpVYSUeJXrbTzZ1UEKYAwJa");
        unused_address!("m/44'/141'/1'/0/3", "REs1NRzg8XjwN3v8Jp1wQUAyQb3TzeT8EB");
        unused_address!("m/44'/141'/1'/0/4", "RS4UZtkwZ8eYaTL1xodXgFNryJoTbPJYE5"); // Stop searching for a non-empty address (gap_limit = 3).
    }

    NativeClient::display_balance.mock_safe(move |_, address: Address, _| {
        let address = address.to_string();
        let balance = checking_addresses
            .remove(&address)
            .expect(&format!("Unexpected address: {}", address))
            .expect(&format!(
                "'{}' address is empty. 'NativeClient::display_balance' must not be called for this address",
                address
            ));
        MockResult::Return(Box::new(futures01::future::ok(BigDecimal::from(balance))))
    });

    NativeClient::list_all_transactions.mock_safe(move |_, _| {
        let tx_history = non_empty_addresses
            .clone()
            .into_iter()
            .map(|address| ListTransactionsItem {
                address,
                ..ListTransactionsItem::default()
            })
            .collect();
        MockResult::Return(Box::new(futures01::future::ok(tx_history)))
    });

    let client = NativeClient(Arc::new(NativeClientImpl::default()));
    let mut fields = utxo_coin_fields_for_test(UtxoRpcClientEnum::Native(client), None, false);
    let accounts = PaMutex::new(vec![
        UtxoHDAccount {
            account_id: 0,
            extended_pubkey: Secp256k1ExtendedPublicKey::from_str("xpub6DEHSksajpRPM59RPw7Eg6PKdU7E2ehxJWtYdrfQ6JFmMGBsrR6jA78ANCLgzKYm4s5UqQ4ydLEYPbh3TRVvn5oAZVtWfi4qJLMntpZ8uGJ").unwrap(),
            account_derivation_path: DerivationPath::from_str("m/44'/141'/0'").unwrap(),
            external_addresses_number: 3,
            internal_addresses_number: 1,
        },
        UtxoHDAccount {
            account_id: 1,
            extended_pubkey: Secp256k1ExtendedPublicKey::from_str("xpub6DEHSksajpRPQq2FdGT6JoieiQZUpTZ3WZn8fcuLJhFVmtCpXbuXxp5aPzaokwcLV2V9LE55Dwt8JYkpuMv7jXKwmyD28WbHYjBH2zhbW2p").unwrap(),
            account_derivation_path: DerivationPath::from_str("m/44'/141'/1'").unwrap(),
            external_addresses_number: 0,
            internal_addresses_number: 2,
        },
    ]);
    fields.derivation_method = DerivationMethod::HDWallet(UtxoHDWallet {
        address_format: UtxoAddressFormat::Standard,
        derivation_path: DerivationPath::from_str("m/44'/141'").unwrap(),
        accounts,
        gap_limit: 3,
    });
    let coin = utxo_coin_from_fields(fields);

    // Check balance of Account#0

    let params = CheckHDAccountBalanceParams {
        account_index: 0,
        gap_limit: Some(3),
    };
    let actual = block_on(coin.check_hd_account_balance_rpc(params)).expect("!hd_account_balance_rpc");
    let expected = CheckHDAccountBalanceResponse {
        account_index: 0,
        derivation_path: DerivationPath::from_str("m/44'/141'/0'").unwrap().into(),
        new_addresses: get_balances!(
            "m/44'/141'/0'/0/3",
            "m/44'/141'/0'/1/1",
            "m/44'/141'/0'/1/2",
            "m/44'/141'/0'/1/3"
        ),
    };
    assert_eq!(actual, expected);

    // Check balance of Account#1

    let params = CheckHDAccountBalanceParams {
        account_index: 1,
        gap_limit: None,
    };
    let actual = block_on(coin.check_hd_account_balance_rpc(params)).expect("!hd_account_balance_rpc");
    let expected = CheckHDAccountBalanceResponse {
        account_index: 1,
        derivation_path: DerivationPath::from_str("m/44'/141'/1'").unwrap().into(),
        new_addresses: get_balances!(
            "m/44'/141'/1'/0/0",
            "m/44'/141'/1'/0/1",
            "m/44'/141'/1'/0/2",
            "m/44'/141'/1'/0/3",
            "m/44'/141'/1'/0/4"
        ),
    };
    assert_eq!(actual, expected);

    let accounts = match coin.as_ref().derivation_method {
        DerivationMethod::HDWallet(UtxoHDWallet { ref accounts, .. }) => accounts.lock().clone(),
        _ => unreachable!(),
    };
    assert_eq!(accounts[0].external_addresses_number, 4);
    assert_eq!(accounts[0].internal_addresses_number, 4);
    assert_eq!(accounts[1].external_addresses_number, 5);
    assert_eq!(accounts[1].internal_addresses_number, 2);
}
