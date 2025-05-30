use bitcrypto::dhash160;
use coins::z_coin::{z_coin_from_conf_and_params_with_docker, z_send_dex_fee, ZCoin, ZcoinActivationParams,
                    ZcoinRpcMode};
use coins::DexFeeBurnDestination;
use coins::{coin_errors::ValidatePaymentError, CoinProtocol, DexFee, PrivKeyBuildPolicy, RefundPaymentArgs,
            SendPaymentArgs, SpendPaymentArgs, SwapOps, SwapTxTypeWithSecretHash, ValidateFeeArgs};
use common::{now_sec, Future01CompatExt};
use lazy_static::lazy_static;
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};
use mm2_number::{BigDecimal, MmNumber};
use mm2_test_helpers::for_tests::zombie_conf_for_docker;
use rustc_hex::ToHex;
use tempfile::TempDir;
use tokio::sync::Mutex;
use coins::MarketCoinOps;
use core::time::Duration;
use std::str::FromStr;
use coins::z_coin::{ZOutput, DEX_FEE_OVK};
use zcash_client_backend::encoding::{decode_payment_address, encode_extended_spending_key};
use zcash_primitives::transaction::components::Amount;
use zcash_primitives::memo::MemoBytes;
use zcash_primitives::zip32::ExtendedSpendingKey;
use zcash_primitives::constants::mainnet as z_mainnet_constants;

use std::collections::HashSet;
use coins::utxo::UtxoCommonOps;
use rpc::v1::types::H256;

use crypto::privkey::key_pair_from_seed;

// https://github.com/KomodoPlatform/librustzcash/blob/4e030a0f44cc17f100bf5f019563be25c5b8755f/zcash_client_backend/src/data_api/wallet.rs#L72-L73
lazy_static! {
    /// For secret....fe
    static ref GEN_TX_LOCK_MUTEX: Mutex<()> = Mutex::new(());
    /// For secret....we
    static ref GEN_TX_LOCK_MUTEX_ADDR2: Mutex<()> = Mutex::new(());
    /// This `TempDir` is created once on first use and cleaned up when the process exits.
    static ref TEMP_DIR: Mutex<TempDir> = Mutex::new(TempDir::new().unwrap());
}

/// Build asset `ZCoin` from ticker and spending_key.
pub async fn z_coin_from_spending_key<'a>(pk_data: &[u8; 32], spending_key: &str, path: &'a str) -> (MmArc, ZCoin) {
    let tmp = TEMP_DIR.lock().await;
    let db_path = tmp.path().join(format!("ZOMBIE_DB_{path}"));
    std::fs::create_dir_all(&db_path).unwrap();
    let ctx = MmCtxBuilder::new().with_conf(json!({ "dbdir": db_path})).into_mm_arc();

    let mut conf = zombie_conf_for_docker();
    let params = ZcoinActivationParams {
        mode: ZcoinRpcMode::Native,
        ..Default::default()
    };
    // let pk_data = [1; 32];

    let protocol_info = match serde_json::from_value::<CoinProtocol>(conf["protocol"].take()).unwrap() {
        CoinProtocol::ZHTLC(protocol_info) => protocol_info,
        other_protocol => panic!("Failed to get protocol from config: {:?}", other_protocol),
    };

    let coin = z_coin_from_conf_and_params_with_docker(
        &ctx,
        "ZOMBIE",
        &conf,
        &params,
        PrivKeyBuildPolicy::IguanaPrivKey(pk_data.into()),
        protocol_info,
        spending_key,
    )
    .await
    .unwrap();

    (ctx, coin)
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_zombie_sapling_cache() {
    let _lock = GEN_TX_LOCK_MUTEX.lock().await;
    let (_ctx, coin) = z_coin_from_spending_key(&[1; 32], "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe", "fe").await;
    assert!(coin.is_sapling_state_synced().await);
    drop(_lock)
}

#[tokio::test(flavor = "current_thread")]
async fn zombie_coin_send_and_refund_maker_payment() {
    let _lock = GEN_TX_LOCK_MUTEX.lock().await;
    let (_ctx, coin) = z_coin_from_spending_key(&[1; 32], "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe", "fe").await;

    assert!(coin.is_sapling_state_synced().await);

    let time_lock = now_sec() - 3600;
    let secret_hash = [0; 20];
    let maker_uniq_data = [3; 32];
    let taker_uniq_data = [5; 32];
    let taker_key_pair = coin.derive_htlc_key_pair(taker_uniq_data.as_slice());
    let taker_pub = taker_key_pair.public();

    let args = SendPaymentArgs {
        time_lock_duration: 0,
        time_lock,
        other_pubkey: taker_pub,
        secret_hash: &secret_hash,
        amount: "0.01".parse().unwrap(),
        swap_contract_address: &None,
        swap_unique_data: maker_uniq_data.as_slice(),
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let tx = coin.send_maker_payment(args).await.unwrap();
    log!("swap tx {}", hex::encode(tx.tx_hash_as_bytes().0));

    let refund_args = RefundPaymentArgs {
        payment_tx: &tx.tx_hex(),
        time_lock,
        other_pubkey: taker_pub,
        tx_type_with_secret_hash: SwapTxTypeWithSecretHash::TakerOrMakerPayment {
            maker_secret_hash: &secret_hash,
        },
        swap_contract_address: &None,
        swap_unique_data: maker_uniq_data.as_slice(),
        watcher_reward: false,
    };
    let refund_tx = coin.send_maker_refunds_payment(refund_args).await.unwrap();
    log!("refund tx {}", hex::encode(refund_tx.tx_hash_as_bytes().0));
    drop(_lock);
}

#[tokio::test(flavor = "current_thread")]
async fn zombie_coin_send_and_spend_maker_payment() {
    let _lock = GEN_TX_LOCK_MUTEX_ADDR2.lock().await;
    let (_ctx, coin) = z_coin_from_spending_key(&[1; 32], "secret-extended-key-main1qvqstxphqyqqpqqnh3hstqpdjzkpadeed6u7fz230jmm2mxl0aacrtu9vt7a7rmr2w5az5u79d24t0rudak3newknrz5l0m3dsd8m4dffqh5xwyldc5qwz8pnalrnhlxdzf900x83jazc52y25e9hvyd4kepaze6nlcvk8sd8a4qjh3e9j5d6730t7ctzhhrhp0zljjtwuptadnksxf8a8y5axwdhass5pjaxg0hzhg7z25rx0rll7a6txywl32s6cda0s5kexr03uqdtelwe", "we").await;

    assert!(coin.is_sapling_state_synced().await);

    let lock_time = now_sec() - 1000;
    let secret = [0; 32];
    let secret_hash = dhash160(&secret);

    let maker_uniq_data = [3; 32];
    let maker_key_pair = coin.derive_htlc_key_pair(maker_uniq_data.as_slice());
    let maker_pub = maker_key_pair.public();

    let taker_uniq_data = [5; 32];
    let taker_key_pair = coin.derive_htlc_key_pair(taker_uniq_data.as_slice());
    let taker_pub = taker_key_pair.public();

    let maker_payment_args = SendPaymentArgs {
        time_lock_duration: 0,
        time_lock: lock_time,
        other_pubkey: taker_pub,
        secret_hash: secret_hash.as_slice(),
        amount: "0.01".parse().unwrap(),
        swap_contract_address: &None,
        swap_unique_data: maker_uniq_data.as_slice(),
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };

    let tx = coin.send_maker_payment(maker_payment_args).await.unwrap();
    log!("swap tx {}", hex::encode(tx.tx_hash_as_bytes().0));
    let spends_payment_args = SpendPaymentArgs {
        other_payment_tx: &tx.tx_hex(),
        time_lock: lock_time,
        other_pubkey: maker_pub,
        secret: &secret,
        secret_hash: secret_hash.as_slice(),
        swap_contract_address: &None,
        swap_unique_data: taker_uniq_data.as_slice(),
        watcher_reward: false,
    };
    let spend_tx = coin.send_taker_spends_maker_payment(spends_payment_args).await.unwrap();
    log!("spend tx {}", hex::encode(spend_tx.tx_hash_as_bytes().0));
    drop(_lock);
}

#[tokio::test(flavor = "current_thread")]
async fn zombie_coin_send_standard_dex_fee_and_payment() {
    let _lock = GEN_TX_LOCK_MUTEX_ADDR2.lock().await;

    let (_ctx, coin) = z_coin_from_spending_key(&[1; 32], "secret-extended-key-main1qvqstxphqyqqpqqnh3hstqpdjzkpadeed6u7fz230jmm2mxl0aacrtu9vt7a7rmr2w5az5u79d24t0rudak3newknrz5l0m3dsd8m4dffqh5xwyldc5qwz8pnalrnhlxdzf900x83jazc52y25e9hvyd4kepaze6nlcvk8sd8a4qjh3e9j5d6730t7ctzhhrhp0zljjtwuptadnksxf8a8y5axwdhass5pjaxg0hzhg7z25rx0rll7a6txywl32s6cda0s5kexr03uqdtelwe", "we").await;
    assert!(coin.is_sapling_state_synced().await);

    // wait until 1 coin mined
    println!("waiting for some mined coins...");
    loop {
        let my_balance = coin.my_balance().compat().await.unwrap();
        //println!("my_balance before withdraw {:?}", my_balance);
        if my_balance.spendable > BigDecimal::from_str("1.0").unwrap() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }

    let to_addr = decode_payment_address(
        &coin.z_fields.consensus_params.hrp_sapling_payment_address,
        "zs182ht30wnnnr8jjhj2j9v5dkx3qsknnr5r00jfwk2nczdtqy7w0v836kyy840kv2r8xle5gcl549",
    )
    .expect("valid z-address")
    .expect("valid z-address");

    let z_out = ZOutput {
        to_addr,
        amount: Amount::from_u64(100000000).unwrap(),
        viewing_key: Some(DEX_FEE_OVK),
        memo: Some(MemoBytes::from_bytes(&[0]).expect("memo from_bytes")),
    };

    // top up another address
    let tx = coin.send_outputs(vec![], vec![z_out]).await.unwrap();
    println!("send_outputs txid {}", tx.txid().to_string());

    drop(coin);

    // use another address to have exact amount on it (avoid filling it with coinbases)
    let seed = "spice describe gravity federal blast come thank unfair canal monkey style afraid";
    let secp_keypair = key_pair_from_seed(seed).unwrap();
    let z_spending_key = ExtendedSpendingKey::master(&*secp_keypair.private().secret);
    println!("second z_spending_key={:?}", z_spending_key);
    let encoded = encode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, &z_spending_key);

    let (_ctx, coin) = z_coin_from_spending_key(&secp_keypair.private().secret, &encoded, "we2").await;

    println!("waiting for non zero balance on secind address...");
    loop {
        let my_balance = coin.my_balance().compat().await.unwrap();
        //println!("my_balance before z_send_dex_fee {:?}", my_balance);
        if my_balance.spendable > BigDecimal::from_str("0").unwrap() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }

    println!("my_balance before dex fee {:?}", coin.my_balance().compat().await.unwrap());
    let fee_tx = z_send_dex_fee(&coin, DexFee::Standard("0.01".into()), &[1; 16])
        .await
        .unwrap();
    println!("z_send_dex_fee txid {}", fee_tx.txid().to_string());
    println!("my_balance after z_send_dex_fee {:?}", coin.my_balance().compat().await.unwrap());
    //log!("dex fee tx {}", tx.txid());

    // this loop, waiting for all locked notes unlocked, makes test workable
    // but without it, the test is not stable: waiting infinitely in sending the payment tx code
    /*loop {
        println!("my_balance after dex fee {:?}", coin.my_balance().compat().await.unwrap());
        let locked_notes = coin.z_fields.locked_notes_db.load_all_notes().await.unwrap();
        println!("locked_notes={:?}", locked_notes);
        if locked_notes.is_empty() {
            break;
        }
        //let z_txid = H256::from_str(&fee_tx.txid().to_string()).unwrap().reversed();
        let z_txid = H256::from_str(&fee_tx.txid().to_string()).unwrap();
        let mut z_txids = HashSet::new();
        z_txids.insert(z_txid);

        match coin.get_verbose_transactions_from_cache_or_rpc(z_txids).compat().await {
            Ok(txns) => {
                if let Some(tx) = txns.get(&z_txid) {
                    println!("fee_tx height {:?}", tx.clone().into_inner().height);
                } else {
                    println!("fee_tx not found in cache");
                }
            },
            Err(e) => {
                println!("get_verbose_transactions_from_cache_or_rpc error {:?}", e);
            },
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }*/


    // try to send payment of 0.08 while the change note is locked by the dexfee tx
    let lock_time = now_sec() - 1000;
    let secret = [0; 32];
    let secret_hash = dhash160(&secret);

    let maker_uniq_data = [3; 32];
    // let maker_key_pair = coin.derive_htlc_key_pair(maker_uniq_data.as_slice());
    // let maker_pub = maker_key_pair.public();

    let taker_uniq_data = [5; 32];
    let taker_key_pair = coin.derive_htlc_key_pair(taker_uniq_data.as_slice());
    let taker_pub = taker_key_pair.public();

    let maker_payment_args = SendPaymentArgs {
        time_lock_duration: 0,
        time_lock: lock_time,
        other_pubkey: taker_pub,
        secret_hash: secret_hash.as_slice(),
        amount: "0.08".parse().unwrap(),
        swap_contract_address: &None,
        swap_unique_data: maker_uniq_data.as_slice(),
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };

    let swap_tx = coin.send_maker_payment(maker_payment_args).await.unwrap();
    println!("send_maker_payment txid {}", swap_tx.tx_hash_as_bytes().to_hex::<String>());
    //log!("swap tx {}", hex::encode(tx.tx_hash_as_bytes().0));
    println!("my_balance after send_maker_payment {:?}", coin.my_balance().compat().await.unwrap());


    drop(_lock)
}

#[tokio::test(flavor = "current_thread")]
async fn zombie_coin_send_dex_fee() {
    let _lock = GEN_TX_LOCK_MUTEX_ADDR2.lock().await;
    let (_ctx, coin) = z_coin_from_spending_key(&[1; 32], "secret-extended-key-main1qvqstxphqyqqpqqnh3hstqpdjzkpadeed6u7fz230jmm2mxl0aacrtu9vt7a7rmr2w5az5u79d24t0rudak3newknrz5l0m3dsd8m4dffqh5xwyldc5qwz8pnalrnhlxdzf900x83jazc52y25e9hvyd4kepaze6nlcvk8sd8a4qjh3e9j5d6730t7ctzhhrhp0zljjtwuptadnksxf8a8y5axwdhass5pjaxg0hzhg7z25rx0rll7a6txywl32s6cda0s5kexr03uqdtelwe", "we").await;

    assert!(coin.is_sapling_state_synced().await);

    let dex_fee = DexFee::WithBurn {
        fee_amount: "0.0075".into(),
        burn_amount: "0.0025".into(),
        burn_destination: DexFeeBurnDestination::PreBurnAccount,
    };
    let tx = z_send_dex_fee(&coin, dex_fee, &[1; 16]).await.unwrap();
    log!("dex fee tx {}", tx.txid());
    drop(_lock);
}

#[tokio::test(flavor = "current_thread")]
async fn zombie_coin_validate_dex_fee() {
    let _lock = GEN_TX_LOCK_MUTEX.lock().await;
    let (_ctx, coin) = z_coin_from_spending_key(&[1; 32], "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe", "fe").await;

    assert!(coin.is_sapling_state_synced().await);

    let tx = z_send_dex_fee(
        &coin,
        DexFee::WithBurn {
            fee_amount: "0.0075".into(),
            burn_amount: "0.0025".into(),
            burn_destination: DexFeeBurnDestination::PreBurnAccount,
        },
        &[1; 16],
    )
    .await
    .unwrap();
    log!("dex fee tx {}", tx.txid());
    let tx = tx.into();

    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx,
        expected_sender: &[],
        dex_fee: &DexFee::Standard(MmNumber::from("0.001")),
        min_block_number: 12000,
        uuid: &[1; 16],
    };
    // Invalid amount should return an error
    let err = coin.validate_fee(validate_fee_args).await.unwrap_err().into_inner();
    match err {
        ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("invalid amount")),
        _ => panic!("Expected `WrongPaymentTx`: {:?}", err),
    }

    // Invalid memo should return an error
    let expected_fee = DexFee::WithBurn {
        fee_amount: "0.0075".into(),
        burn_amount: "0.0025".into(),
        burn_destination: DexFeeBurnDestination::PreBurnAccount,
    };

    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx,
        expected_sender: &[],
        dex_fee: &expected_fee,
        min_block_number: 12000,
        uuid: &[2; 16],
    };

    let err = coin.validate_fee(validate_fee_args).await.unwrap_err().into_inner();
    match err {
        ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("invalid memo")),
        _ => panic!("Expected `WrongPaymentTx`: {:?}", err),
    }

    // Success validation
    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx,
        expected_sender: &[],
        dex_fee: &expected_fee,
        min_block_number: 12000,
        uuid: &[1; 16],
    };
    coin.validate_fee(validate_fee_args).await.unwrap();

    // Test old standard dex fee with no burn output
    // TODO: disable when the upgrade transition period ends
    let tx_2 = z_send_dex_fee(&coin, DexFee::Standard("0.00879999".into()), &[1; 16])
        .await
        .unwrap();
    log!("dex fee tx {}", tx_2.txid());
    let tx_2 = tx_2.into();

    // Success validation
    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx_2,
        expected_sender: &[],
        dex_fee: &DexFee::Standard("0.00999999".into()),
        min_block_number: 12000,
        uuid: &[1; 16],
    };
    let err = coin.validate_fee(validate_fee_args).await.unwrap_err().into_inner();
    match err {
        ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("invalid amount")),
        _ => panic!("Expected `WrongPaymentTx`: {:?}", err),
    }

    // Success validation
    let expected_std_fee = DexFee::Standard("0.00879999".into());
    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx_2,
        expected_sender: &[],
        dex_fee: &expected_std_fee,
        min_block_number: 12000,
        uuid: &[1; 16],
    };
    coin.validate_fee(validate_fee_args).await.unwrap();
    drop(_lock)
}
