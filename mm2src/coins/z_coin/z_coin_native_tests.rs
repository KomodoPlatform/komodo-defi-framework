use bitcrypto::dhash160;
use common::{block_on, now_sec, one_thousand_u32};
use mm2_core::mm_ctx::MmCtxBuilder;
use mm2_test_helpers::for_tests::zombie_conf;
use std::path::PathBuf;
use std::time::Duration;
use zcash_client_backend::encoding::decode_extended_spending_key;

use super::{z_coin_from_conf_and_params_with_z_key, z_mainnet_constants, Future, PrivKeyBuildPolicy,
            RefundPaymentArgs, SendPaymentArgs, SpendPaymentArgs, SwapOps, ValidateFeeArgs, ValidatePaymentError,
            ZTransaction};
use crate::z_coin::{z_htlc::z_send_dex_fee, ZcoinActivationParams, ZcoinRpcMode};
use crate::{CoinProtocol, SwapTxTypeWithSecretHash};
use crate::{DexFee, DexFeeBurnDestination};
use mm2_number::MmNumber;

#[tokio::test]
async fn zombie_coin_send_and_refund_maker_payment() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let mut conf = zombie_conf();
    let params = default_zcoin_activation_params();
    let pk_data = [1; 32];
    let db_dir = PathBuf::from("./for_tests");
    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();
    let protocol_info = match serde_json::from_value::<CoinProtocol>(conf["protocol"].take()).unwrap() {
        CoinProtocol::ZHTLC(protocol_info) => protocol_info,
        other_protocol => panic!("Failed to get protocol from config: {:?}", other_protocol),
    };

    let coin = z_coin_from_conf_and_params_with_z_key(
        &ctx,
        "ZOMBIE",
        &conf,
        &params,
        PrivKeyBuildPolicy::IguanaPrivKey(pk_data.into()),
        db_dir,
        z_key,
        protocol_info,
    )
    .await
    .unwrap();

    let time_lock = now_sec() - 3600;
    let maker_uniq_data = [3; 32];

    let taker_uniq_data = [5; 32];
    let taker_key_pair = coin.derive_htlc_key_pair(taker_uniq_data.as_slice());
    let taker_pub = taker_key_pair.public();

    let secret_hash = [0; 20];

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
    let tx = coin.send_maker_payment(args).wait().unwrap();
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

#[tokio::test]
async fn zombie_coin_send_and_spend_maker_payment() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let mut conf = zombie_conf();
    let params = default_zcoin_activation_params();
    let pk_data = [1; 32];
    let db_dir = PathBuf::from("./for_tests");
    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();
    let protocol_info = match serde_json::from_value::<CoinProtocol>(conf["protocol"].take()).unwrap() {
        CoinProtocol::ZHTLC(protocol_info) => protocol_info,
        other_protocol => panic!("Failed to get protocol from config: {:?}", other_protocol),
    };

    let coin = z_coin_from_conf_and_params_with_z_key(
        &ctx,
        "ZOMBIE",
        &conf,
        &params,
        PrivKeyBuildPolicy::IguanaPrivKey(pk_data.into()),
        db_dir,
        z_key,
        protocol_info,
    )
    .await
    .unwrap();

    let lock_time = now_sec() - 1000;

    let maker_uniq_data = [3; 32];
    let maker_key_pair = coin.derive_htlc_key_pair(maker_uniq_data.as_slice());
    let maker_pub = maker_key_pair.public();

    let taker_uniq_data = [5; 32];
    let taker_key_pair = coin.derive_htlc_key_pair(taker_uniq_data.as_slice());
    let taker_pub = taker_key_pair.public();

    let secret = [0; 32];
    let secret_hash = dhash160(&secret);

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

    let tx = coin.send_maker_payment(maker_payment_args).wait().unwrap();
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

#[tokio::test]
async fn zombie_coin_send_dex_fee() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let mut conf = zombie_conf();
    let params = default_zcoin_activation_params();
    let priv_key = PrivKeyBuildPolicy::IguanaPrivKey([1; 32].into());
    let db_dir = PathBuf::from("./for_tests");
    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();
    let protocol_info = match serde_json::from_value::<CoinProtocol>(conf["protocol"].take()).unwrap() {
        CoinProtocol::ZHTLC(protocol_info) => protocol_info,
        other_protocol => panic!("Failed to get protocol from config: {:?}", other_protocol),
    };

    let coin =
        z_coin_from_conf_and_params_with_z_key(&ctx, "ZOMBIE", &conf, &params, priv_key, db_dir, z_key, protocol_info)
            .await
            .unwrap();

    let dex_fee = DexFee::WithBurn {
        fee_amount: "0.0075".into(),
        burn_amount: "0.0025".into(),
        burn_destination: DexFeeBurnDestination::PreBurnAccount,
    };
    let tx = z_send_dex_fee(&coin, dex_fee, &[1; 16]).await.unwrap();
    log!("dex fee tx {}", tx.txid());
}

#[tokio::test]
async fn zombie_coin_send_standard_dex_fee() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let mut conf = zombie_conf();
    let params = default_zcoin_activation_params();
    let priv_key = PrivKeyBuildPolicy::IguanaPrivKey([1; 32].into());
    let db_dir = PathBuf::from("./for_tests");
    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();
    let protocol_info = match serde_json::from_value::<CoinProtocol>(conf["protocol"].take()).unwrap() {
        CoinProtocol::ZHTLC(protocol_info) => protocol_info,
        other_protocol => panic!("Failed to get protocol from config: {:?}", other_protocol),
    };

    let coin =
        z_coin_from_conf_and_params_with_z_key(&ctx, "ZOMBIE", &conf, &params, priv_key, db_dir, z_key, protocol_info)
            .await
            .unwrap();

    let dex_fee = DexFee::Standard("0.01".into());
    let tx = z_send_dex_fee(&coin, dex_fee, &[1; 16]).await.unwrap();
    log!("dex fee tx {}", tx.txid());
}

/// Use to create ZOMBIE_wallet.db
#[test]
fn prepare_zombie_sapling_cache() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let mut conf = zombie_conf();
    let params = default_zcoin_activation_params();
    let priv_key = PrivKeyBuildPolicy::IguanaPrivKey([1; 32].into());
    let db_dir = PathBuf::from("./for_tests");
    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();
    let protocol_info = match serde_json::from_value::<CoinProtocol>(conf["protocol"].take()).unwrap() {
        CoinProtocol::ZHTLC(protocol_info) => protocol_info,
        other_protocol => panic!("Failed to get protocol from config: {:?}", other_protocol),
    };

    let coin = block_on(z_coin_from_conf_and_params_with_z_key(
        &ctx,
        "ZOMBIE",
        &conf,
        &params,
        priv_key,
        db_dir,
        z_key,
        protocol_info,
    ))
    .unwrap();

    while !block_on(coin.is_sapling_state_synced()) {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[tokio::test]
async fn zombie_coin_validate_dex_fee() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let mut conf = zombie_conf();
    let params = default_zcoin_activation_params();
    let priv_key = PrivKeyBuildPolicy::IguanaPrivKey([1; 32].into());
    let db_dir = PathBuf::from("./for_tests");
    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();
    let protocol_info = match serde_json::from_value::<CoinProtocol>(conf["protocol"].take()).unwrap() {
        CoinProtocol::ZHTLC(protocol_info) => protocol_info,
        other_protocol => panic!("Failed to get protocol from config: {:?}", other_protocol),
    };

    let coin =
        z_coin_from_conf_and_params_with_z_key(&ctx, "ZOMBIE", &conf, &params, priv_key, db_dir, z_key, protocol_info)
            .await
            .unwrap();

    // https://zombie.explorer.lordofthechains.com/tx/78b24c5093cb5de007bd8b34f4ec5c26e88862199dd7b38fc3e0bf60c3c620a9
    let tx_hex = "0400008085202f890000000000005efc0600e80300000000000001745e8e55063d50f286b2d8665b860fb4d489acdd41fa41da6eccbbe871f4854ee4f12460119a7b54c792136c29daf0a848aeafd89630f38926f1c666366bc02433523951dd53294409067bed250213f583d610cb141afe73113f83a2098e616befd22deba3d11cb4246e5a3cb35dfec42466bf4aafc691026c1e560d990354ab9855303c2a70003662a101ecf35b5adbf8bf003832bec5e4941822c619e04402da405bedac41a95a47929ed30ee16e57825b34c711b611872ce946b9621eb6498ae40d7604311a5436a4f2da5c12ef83dee790470dff088cb88277a29a259b8d04e145cd8ad80f2fb6c4720d194d83427b6b78f915b070e91bbcbf84a5d4dec4757cdb19037d4b76307452153608e015b915a7b29f2c306e70650d1e891a0f6469860dba690b1659f6ec38bdc2d5ed23f12c1d9eb78dc62632fbddfa7aaf164b24572e381f28e33259d9078dd3bc896e22d0701efc68f7dc4487c392d994a062fedce5abcd0c230dfbd8c4e7c2e6b4784a4313593acf76d9e791db65c3b3270e036be3fd8dd5bc597fb5f3f48602e7219807060e27209d56b24f17d5a26da5d7307d18780194def01212a511f8470589584278bffd82ffff015e5de4d1a3c01e482995d1cece7efacfbfb8d7780abae18f88ae9c5789e54c6248c00bb1caa4a16145e8d08e9f90f93d5b12aac8e7fec7dce114bf77ad8a0b927ead9f7ea46dfd23a52111478ead12cc7a54e118ac51f3624f8e5923778f6274ca361247965f42332ae30c1f498e7acb4b89bf508802127cf0952bf2a31fc6b6aafda6f477ab98d01a01c912b4e0d0ca9c535c2e731e4f58c36066e1c631cc703773fa204d5db897d6a8da2c0af846979984a9e8bbf0113b94b233a7cfb7b878ecd4794bbb99a3d8cafba4204c2633501b8215adb1100aae3c21486ca126b913326fc6867306e50414391333ef2c1e1432ba7315b1dd0d744854f4e7ff7b742b5861b8cb59927183284d2db3f38aef52b244e391b059fc1057938f4c7081c3fd208b78714f3ae235db3f4db431c7fbcc446469f811e387f82fd6008e4b6a1b0bbd4760d1e9ad3111884cf83820058015ddfbd89588816f1b1016b27d41b40faf7f94d53e9925edf2e4dcec23554ee21a9ef3bfd185498b16c6b7b4a7c79f46e256ba498e465468304bf8827432ffbf93bd988f90e77d9bcc72ef729aac729ef25e8eeaabb6d21c4bf320c04158572085cf6293ddfb183a9936e9205ea1dc8175777c66bdd762f52b62a554033e338b2f2ea9511ce7a0448b046ecb03258d95d4743c0c80b9dbae49f69250b7021368d3855526ae0440d55388653912f2f86ddf8128697c1c9155a16bd4e5632c8c35b650b37aeb61d57f0cf1606459ae4472f4c4a721c5e1b78144557f7ca641cc2c04c9760e9d27d7730231f0e2d2f8fd118292fbe420dd69cdf95ead77f01791f9be4e91e1379a27b9c042ac8a25978e9cf77fd9b63c443ac218ec16a1fd96e208f1558746eb92704618ea787b33c3731c146c5a485d2398e41b25742c54b70d081850b812d07a145d191023852f74bb40e00656f9a8e25bef21e6436d65f05354c84369d05c7e4197832cbe460d80fc7b4d202c7a6e4646730c372f4e0b179225d033d19571a0c090823863c88148c8b0b9a68177a5f24ffc698a4d7c0a9756ec06d9b5ab53be48ce2a9ff744951afe354c271f9ff2d11bda37eccaefc79f28b9aef3cf3e4107fb9e18b9cf9a3d0d810b713ed2648b7787b98c592a6be3505c0d053ebb5786bb90dc3d6f3bd6d0c5292f86bfad170e3b4b56921bd897cbaee97d58d7ce4a891dbad972a7f968d72224c7acb89a25f8b828b4e2e3cc7f7fbc8b902667120870ed0ad7c2e5347d2b7d8a863fe89bb5e1573f5d8dd2e26e1ffc07cc8f95b5640e670941ba9ee83c6c1287c8ba0dc73d9beab0d726a4db82eff96c8c932ff1d3f24206faef182f784328bf9e554e3ee4bb38857c8ae43ecee421f13ab113e6a561ef78738bcda577671a3dee7ce4be41cd37562f64c0616c6640dec87452cd30ebe3316f6d40be064392f0e620be3371f74838432e08a757aa0d157634bcb2d3d23ded907aef74780f7eed1becdcd05ddc91b8f7ece2504c2a5fd261c67a6200e3efaa39893ab5778243b8d70f83806d54654fb8307bb23f7edef133e6f6eb8bdf01ba307be546568f4f0447e0e582b9ad908ce6a04f4453817ca2fc542a8adb7512260e1012c20e4ec425b7e2806becf14ac84594800f76056c3d2fd4edc498442b43909ef3b007c4db189691b9726c17ea5e9a9ef262890252e193440abdd7e2901134bee4de58d4866a7dc69a302ac953c318cdad842a5a4c7c1341f772519ad350e53f23b41996ab0d3c1edecc0bee7f86b9add873bdfa4134ec9993782d5fd0299a17de6fb774b55f45a0c7051e365b30e2e23ca39b405c9f6b58927fbf2ccca777d888439a8ef43f99a64cedf321522c38283b2cefde744bc176a0505ec23d810bc4474b0a9dfa2aca0032608559559fb978fe180e727d68a73234519ea545867307b2d9c3bbff9070dc7d5117de318aee94fe7a909f846a46a31a0caf101bf5625dde52708f5b2189097f5525ba0c44234bc4705bc24fcbb44d43d09dc74a389cc4ea32cb82bf3578abe3943182c1e8acbc7bbe4d2e151a5481e28e34b0621bb85d1bd7a2778fdada261172059d305538fd4ab908cf29743fc3fcefcc5774872c18b70c77b38a9343ac331174aa0b5266c7ba68d082fe550b0cecf4d4b99529c492e8220fb0d87160b33431bdeb40393f45bdfc9c5d781f718c9e4a598834c74dfcb86aa7ac5f1ae7c88052c420c5ba65e1f09015d0b2c420d87fa10ba802c3cd05b7f35c0daac0098aa9865fa0a6ca4da10f4cec3fe62f2b50b1f77fabdb16d6f779f08ea30e746b8c29388f41c70ef33e309ffb4d08e51f27893b3f85ca57d1c7d3f73bf14c76e03d1ad215db84c3b6ded38d84bffe4c79951aa0b689a10b395ba5a834f0766cca5e9110419d944f691df22ea46e370b2931e3ce8be06caefedeaf6cc6095c2cd654200d92861c80137ca55f7cc5939c31af177426e2dcba53e75202b7e087c32679ca38832bdf535bea854302ff6262296a0b3ceb1b5d6d38902e8be64728f4224ab8aefc20cc2e5a9be85e319e546cc7d9649048ffebb16bb4ffb2a1e7ccb50b5e12283ee7a68d48f37f84fafb3ad0d2420d4cd1796bb5fe9ff55734b61847b6f99094563b27b780910a00c0f968f6465639e9362e9122b41948083c7b7a9815094e6a780052fb1b2f20b0af5b53868b2bb209e5589aefc6b376b7edd1200546bfc51dfd3bda62c9ea8eb79f1b75cab4d67d0cdad6bba2d14daa00019735b37c17a71b83f905d7e78cde70632111f69864361598bb83c78c75e857bb39fddbaee6c299cdffcd1244b20fc07fbe880c35ab926f1504d7a93431ee942efcbd6e3b86da2cb4f0c03d51bfadec4ca15564d54a1b9cf6fe57cc5754a00d75a5b5eb2f5b107d22bed8819a8ac28bc77b8b21aa40ad65decd368c690b174c05d5eb5dfd4eba825779c767807f8e45708b12b6e38eafea6efd3a0d64e740d0a9117d2e82ee94bc92929a5d87dbfcc6e7e3ff1be6d54f0236c90e8c918833c7ebeadc7430417b88769c1dd06dca5e53a635cc67a9b46d3dfa4505c51a63739a6523f8c18ec2b9ccb4f36c6f3b532cc27b15038ccb62a20cfaae6707f9c972ed826792ed2e41c2a58187b87780989f3b3f29efa1054e08593304d5ea5cb267dac61131874c4caa34bd6cd2a22569e6eb9ccebfc249750d96b6354c233fdc292005f04f7823c3b87420020c36829c6959ae338185f97074697eef433f319f396bb486b69520fdf91f110040f3babf1cf4a5e88c09cd2ebb9685bf2e532776165daed9abfb7a7066a78541467a7c119de71a29414459beb0a97bdebac9f8ec9bbdf17346ddf79fc55cdbab93d32b2db46519e6495721f8bcf512245fcec8c00fe9a428d5f2fe83a8b7a63d2bec01c62ba635901066328d10f93e9737d986c7137bc8459fdfa69d6fa41e1029f117402df57070b3c3e1993be762d75c7ed39a0d435c71dcec6255d3cca5eca300782c0e795b90da33a202bf00020fad8b254ca0410dc4bb49fb28005b79796ece98b7c0aba1dead4a1e8ae010dc233fac9c5d614cc3550dca31e46bc4e51997c435846c01ade5a9b3f09379690e1fbc85df24fa3ca17c8274daee7db84043b8f671a81d2413ccc8c1bde2e2cd4aa7bef8b86d8221b6602e6ec2b996058b7db0207f29f150386847a9625d66288db397addb96b701a0522091a9ab28c164f28cc1d92ba740fd46bee92b0bcd1b1de18603c6767e73158e47ab34e2624ec06b6fba66b09353893add65be3e60112d22a8c10b1c46f89b979c649b686b18db83164d351d48e23c95cefcd1247fafd7a5f6b8c346a20ed1b95af1051fea86618eb7569e7e5765b6d0cd1c4d7d77f0483da78680de98ca8b3a13c2717e1904af02b07ec1eb23d339adcce34c4a14e1fa9ea86195eb0b0faac8a7241d9ec7bfbe0e006cf002aca2ff875a18541405f73d5c439417a53711bf2639a1d7debd16de9b327e8c56a009b7561a223629c1ef2e13c384b928347a92b873ec988fae2a8de004";
    let tx_bytes = hex::decode(tx_hex).unwrap();
    let tx = ZTransaction::read(tx_bytes.as_slice()).unwrap();
    let tx = tx.into();

    let expected_fee = DexFee::WithBurn {
        fee_amount: "0.0075".into(),
        burn_amount: "0.0025".into(),
        burn_destination: DexFeeBurnDestination::PreBurnAccount,
    };

    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx,
        expected_sender: &[],
        dex_fee: &DexFee::Standard(MmNumber::from("0.001")),
        min_block_number: 12000,
        uuid: &[1; 16],
    };
    // Invalid amount should return an error
    let err = coin.validate_fee(validate_fee_args).wait().unwrap_err().into_inner();
    match err {
        ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("invalid amount")),
        _ => panic!("Expected `WrongPaymentTx`: {:?}", err),
    }

    // Invalid memo should return an error
    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx,
        expected_sender: &[],
        dex_fee: &expected_fee,
        min_block_number: 12000,
        uuid: &[2; 16],
    };
    let err = coin.validate_fee(validate_fee_args).wait().unwrap_err().into_inner();
    match err {
        ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("invalid memo")),
        _ => panic!("Expected `WrongPaymentTx`: {:?}", err),
    }

    /* Fix realtime min_block_number to run this test:
    // Confirmed before min block
    let min_block_number = 451208;
    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx,
        expected_sender: &[],
        dex_fee: &expected_fee,
        min_block_number: ,
        uuid: &[1; 16],
    };
    let err = coin.validate_fee(validate_fee_args).wait().unwrap_err().into_inner();
    match err {
        ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("confirmed before min block")),
        _ => panic!("Expected `WrongPaymentTx`: {:?}", err),
    } */

    // Success validation
    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx,
        expected_sender: &[],
        dex_fee: &expected_fee,
        min_block_number: 12000,
        uuid: &[1; 16],
    };
    coin.validate_fee(validate_fee_args).wait().unwrap();

    // Test old standard dex fee with no burn output
    // TODO: disable when the upgrade transition period ends

    // https://zombie.explorer.lordofthechains.com/tx/1d7db42155ed1ace74ca00dbf21ac4108f2d25baf83accf9fffd3c4d1d4239b6
    let tx_2_hex = "0400008085202f8900000000000034fc0600e80300000000000001a18aa4c3884d0d1a2123cba24bed5882c1f97fccfc7c14e2276504554e046f268e028917a6bda8a84bb4659da995048ea8f33aaab14954a7a67072993930662a15372c4dcf6e33684e35611b2e69fbee3c318519af6c2b0d8fbf035ada9fcf0145d3f14afa06481c9cb1cf0ac5a8b5b85a6753de2e1892ee5fe17f13599e00e0a4d59591030d007e43b9604da70037a07b8563a241dcb31efe0379996b5fec1b6894af965704d175032c3b0cfd311662b156900aadb76db5d9631d81350cc399e66138c9a51258a7f10922d0a2a2c3530b63486ee79e83ab1013701e9317f1cf12d4d03f5062932f5c8a50b40eb8768f3cd917cd75acba59b4b35c9ca9fa01c74b18eed47e614f06ff93b9451cb0547288a7b6689464d856f29cbba463434ff584ac982ab9e30be34d78c5ce43e35fb218cf3a3fbbcc0be787c2fe8bc81fbbc4b4bf3a9a91d74c7702cab77f7292fea24d6ef17b63f29ec73053e7b5c2c40b3719c1a9a7d0070b1bddf712aedf8b6165e29cc4b71ddce73dc4752b4551566a0102c8e8391fbcd124d1e2b1cca28c3fd2da2c7fd36cd3134e3e7cbb9323a600ea06256d5ce0da0efc8bfe90ab15406c763d50de9fe7daf6aa6ff90db6fc6236414447177f7b8ff792322b70bb2ef11eb400ab9609f7099ef393232b4bc31bc76485e80d24e8961e03ef542dafbf2ebc4138de6ab18e9afd468c31a35f7cdd8822f18ca7404795e95296eb31baeb8305faa05f0c8c08f48b5ff071c7f5479a2181d33abe8c6c172988cca1fe24921664b0ce3caab56ae8548bc542e867990c6ebe49483939cb85702d58cbe40fe52e34bc57c91795625f869430009bfff50cd96e7a548dac852700751a4160ebe71a5639abf6e4efa7e8fd1946bea7d7dab3ce28921f1371d3ecd8540429a36f2175f40ff96c0f72a3b18429e83a8cb3259db864044eb0a6a481233108109cc6c9c798976d2abdc6cb62252b18262b05736fc557e214de4259c5c856c3c2d022b18d0782135b387e9ade580a432c1a8fec86deadc287f09101583b3e30c55024c72fc78bb8ce453541b34d6bf0252d675d48be06e84c41125f5110c36e0cbd113931bf40838b926e6611e60bc6741c1779fec015da4547f71e42af65e1814b39179e062a095765061e56778ab4ba0892a4b3160045672b249fc5350174b195fa323f87c781a5ec71af9f17c4208ce9172717eb9e187c9005d2e8db184d95eb6efd252804159db9f2a1c227b516ee7ebc958a593892b927f958a35d8ebe8edd20acf62920adb4344a673beb4ec350ce1cb3c7bf7c5008ef857cb4e1c32040710b304863d6de29017717e90748c37f4182b30a5175b991bdc8358982659c1944e5391e344f60ab6870d9f75e407345f8bf89cfc7ecf38b8a021fd747e4f58c7117c240c6043759cffe6ec30417b6bad46ac6638bd62f68995f3f24293c0ddfbe2c5bf50b1d1e9c1c07a000f4eb3bada725bc9867551b93b5df49aa0dcc2890d3f36cd3d5315606e460ac2843497b1f1fef34ec6064bc6453997fdc1e197181b35090c1b53701a5b3ba8fc923a8704982786d0ba7593f0f256f8788a65cdd8e7e0681632a0832f6b3d01ea4b9c159ff87595d25da0ff3a6bda1dc1f66091d0509e6ce636205713bf9fe8607091a5d69a3c0f545a68499206f84a1b96231227419cb2898c110dcf7c818a27912171416394bef4bd8b9f1616d91a5e018e484fcf9a685c50a75472d0a393215faf302795042bfb15d4f30e7ef7c12af9a50deb729a3d93441c482af6928f1037fa3464d73b501b592a7e8f065c7c4a4c0f37ca09dca67f39aa153dd6c72ce086895865b4ab6ec8bb89a59c2f4497b955d32deba77d9c4e9f8eebe0f5a8f808da89d77f451cb09908d506fe7e79d230f66a8d5b5f0ccc53ff074862b64b64291221ccd41745ffaf8e18ab98c090505e908534772d10f8f586e6170c6d4b20004c83957fe8ccb72a1485cc9af649cad6a05e58092310ad054367b1b7fe45066e91db277cbcf5bd26842a766676bdb56b30ac53b44fdc30aeb00a81317796c2ce0d88f59a46be27086570d8c620661f16c04bfa0124bcc3c978f6745072c15d5d912a93c24509bdbc3aba2caddfcef001dde162ebb89c7f433122ea6d093ff3b22bcd8a2564d69f09c02ea0310549af862c7338d287b94223a55d3c56884cfbfc4bf4b9161848e4e571c500f3d1a7e1d0954d2d86481804b4c8ab795346ec31b9fb139b7d8af4673c0069f1030ecb3f183ab8d86d67866fe0d4778bc96e05120b41046c3ec643097d62cfaa3b1c29aac340056d080c034cb1d56ba90cb8164d21645703281906058901e4c131c261ec7d7e2c29f2000387cb989cd6c70401c24b96449c0f3f25d63caceab4d9722e5fe4444f7a964ba4b37260ef93c45492516cb1e4901d4c75d2e0feb012cbe1751350c929e5f1c8dcafbecea6e966a57516e6158ec0385c49b591816b8d588b4f006742f36e5de5af2a0193368c712395c59effd3e01b923d3a825567a125ee5169ad19160c905cee9cc0ccec5611899572a8132a21be704a95a574941418e7ef8b6e40236434c61972dc0bbfbb84aa7317d7570e98206d190d319c7dedee24779f89802bd80165f398f8d2cd555b12dc58c4c98b7ba40ea80197532f214e15e4bcecc7531e69ea8a93988ba5c1011600dda0bcab460d70af3abd26540497d1a7e2787fdf4916f034cee53866d7a83397677487ee87f7889e707b0c59104340fbe99d36f624e7e7c39bacf113d2f98ba3bd54d1ed53d58aac6fdca42d06020b29b4959cd1e14db6351f611f50b6f9251123a1ac640864b36522568bc4e515590331a7e3d7c22d24133bcd3b7e40e44f8535e1576b51d64e8a4e6e4add358fd0481cfa35f3b45fa0bbf0cd8bd45cde4445f1d10733ea43051aefca672ef0428654ab346dfda2e8ec28cc92bfbd5001bc3e34dd91abcb18460ceba223c4a60e40a53f808316f41142541ece5cd8727269664f8eb28358893b564a071d1240aa8edd615f6d8c31ddc4132b5273d3183c04736cdc02fabb49fc284be83507c2c131d90700ef7335e00702d8c6b569007073251e179eb76439f7c117680a97939c077c37967c022df61621ab28e37b4fed1e252caf48ce0cf80a55b4a07d91620ed96d55768caa6f33e7288407f394d655313c0b5f821cc460fd89c9c1482d51c5f98c4c4f6da2e45004e35050b6f3d25150177c44a7f17d1b0e954bbe4c8083dc993f8ddef7459c284dd117f58d925dc70e580fce4dd27b98bd43533c824849001c94518ed9010bc01";
    let tx_2_bytes = hex::decode(tx_2_hex).unwrap();
    let tx_2 = ZTransaction::read(tx_2_bytes.as_slice()).unwrap();
    let tx_2 = tx_2.into();

    // Success validation
    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx_2,
        expected_sender: &[],
        dex_fee: &DexFee::Standard("0.00999999".into()),
        min_block_number: 12000,
        uuid: &[1; 16],
    };
    let err = coin.validate_fee(validate_fee_args).wait().unwrap_err().into_inner();
    match err {
        ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("invalid amount")),
        _ => panic!("Expected `WrongPaymentTx`: {:?}", err),
    }

    // Success validation
    let expected_std_fee = DexFee::Standard("0.01".into());
    let validate_fee_args = ValidateFeeArgs {
        fee_tx: &tx_2,
        expected_sender: &[],
        dex_fee: &expected_std_fee,
        min_block_number: 12000,
        uuid: &[1; 16],
    };
    coin.validate_fee(validate_fee_args).wait().unwrap();
}

fn default_zcoin_activation_params() -> ZcoinActivationParams {
    ZcoinActivationParams {
        mode: ZcoinRpcMode::Native,
        required_confirmations: None,
        requires_notarization: None,
        zcash_params_path: None,
        scan_blocks_per_iteration: one_thousand_u32(),
        scan_interval_ms: 0,
        account: 0,
    }
}
