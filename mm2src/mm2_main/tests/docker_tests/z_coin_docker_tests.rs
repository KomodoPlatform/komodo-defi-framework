use crate::common::Future01CompatExt;
use bitcrypto::dhash160;
use coins::z_coin::{z_coin_from_conf_and_params_with_docker, z_send_dex_fee, ZCoin, ZcoinActivationParams,
                    ZcoinRpcMode};
use coins::{coin_errors::ValidatePaymentError, CoinProtocol, DexFee, MarketCoinOps, PrivKeyBuildPolicy,
            RefundPaymentArgs, SendPaymentArgs, SpendPaymentArgs, SwapOps, SwapTxTypeWithSecretHash, ValidateFeeArgs};
use common::{executor::Timer, now_sec};
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};
use mm2_number::MmNumber;
use mm2_test_helpers::for_tests::{new_mm2_temp_folder_path, zombie_conf};
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};

/// Build asset `ZCoin` from ticker and spendingkey str without filling the balance.
pub async fn z_coin_from_spending_key(spending_key: &str) -> (MmArc, ZCoin) {
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let mut conf = zombie_conf();
    let params = ZcoinActivationParams {
        mode: ZcoinRpcMode::Native,
        ..Default::default()
    };
    let pk_data = [1; 32];
    let salt: String = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(4)
        .map(char::from)
        .collect();
    let db_folder = new_mm2_temp_folder_path(None).join(format!("ZOMBIE_DB_{}", salt));
    std::fs::create_dir_all(&db_folder).unwrap();
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
        db_folder,
        protocol_info,
        spending_key,
    )
    .await
    .unwrap();

    (ctx, coin)
}

#[tokio::test(flavor = "multi_thread")]
async fn zombie_coin_send_and_refund_maker_payment() {
    let (_ctx, coin) = z_coin_from_spending_key("secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").await;
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
}

#[tokio::test(flavor = "multi_thread")]
async fn zombie_coin_send_and_spend_maker_payment() {
    let (_ctx, coin) = z_coin_from_spending_key("secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").await;

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
}

#[tokio::test(flavor = "multi_thread")]
async fn prepare_zombie_sapling_cache() {
    let (_ctx, coin) = z_coin_from_spending_key("secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").await;

    while !coin.is_sapling_state_synced().await {
        Timer::sleep(1.0).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn zombie_coin_send_dex_fee() {
    let (_ctx, coin) = z_coin_from_spending_key("secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").await;

    let tx = z_send_dex_fee(&coin, "0.01".parse().unwrap(), &[1; 16]).await.unwrap();
    log!("dex fee tx {}", tx.txid());
}

// TODO: fix test
#[ignore]
#[tokio::test(flavor = "multi_thread")]
async fn zombie_coin_validate_dex_fee() {
    let (_ctx, coin) = z_coin_from_spending_key("secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").await;

    // let balance = coin.my_balance().compat().await;

    let tx = z_send_dex_fee(&coin, "0.01".parse().unwrap(), &[1; 16]).await.unwrap();
    log!("dex fee tx {}", tx.txid());
    let tx = tx.into();

    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx,
        expected_sender: &[],
        fee_addr: &[],
        dex_fee: &DexFee::Standard(MmNumber::from("0.001")),
        min_block_number: 10,
        uuid: &[1; 16],
    };
    // Invalid amount should return an error
    let err = coin.validate_fee(validate_fee_args).await.unwrap_err().into_inner();
    match err {
        ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("Dex fee has invalid amount")),
        _ => panic!("Expected `WrongPaymentTx`: {:?}", err),
    }

    // Invalid memo should return an error
    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx,
        expected_sender: &[],
        fee_addr: &[],
        dex_fee: &DexFee::Standard(MmNumber::from("0.01")),
        min_block_number: 10,
        uuid: &[2; 16],
    };
    let err = coin.validate_fee(validate_fee_args).await.unwrap_err().into_inner();
    match err {
        ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("Dex fee has invalid memo")),
        _ => panic!("Expected `WrongPaymentTx`: {:?}", err),
    }

    // Confirmed before min block
    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx,
        expected_sender: &[],
        fee_addr: &[],
        dex_fee: &DexFee::Standard(MmNumber::from("0.01")),
        min_block_number: 20000,
        uuid: &[1; 16],
    };
    let err = coin.validate_fee(validate_fee_args).await.unwrap_err().into_inner();
    match err {
        ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("confirmed before min block")),
        _ => panic!("Expected `WrongPaymentTx`: {:?}", err),
    }

    // Success validation
    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx,
        expected_sender: &[],
        fee_addr: &[],
        dex_fee: &DexFee::Standard(MmNumber::from("0.01")),
        min_block_number: 10,
        uuid: &[1; 16],
    };
    coin.validate_fee(validate_fee_args).await.unwrap();
}
