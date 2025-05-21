//! Native tests for zcoin
//!
//! To run zcoin tests in this source you need `--features zhtlc-native-tests`
//! ZOMBIE chain must be running for zcoin tests:
//! komodod -ac_name=ZOMBIE -ac_supply=0 -ac_reward=25600000000 -ac_halving=388885 -ac_private=1 -ac_sapling=1 -testnode=1 -addnode=65.21.51.116 -addnode=116.203.120.163 -addnode=168.119.236.239 -addnode=65.109.1.121 -addnode=159.69.125.84 -addnode=159.69.10.44
//! Also check the test z_key (spending key) has balance:
//! `komodo-cli -ac_name=ZOMBIE z_getbalance zs10hvyxf3ajm82e4gvxem3zjlf9xf3yxhjww9fvz3mfqza9zwumvluzy735e29c3x5aj2nu0ua6n0`
//! If no balance, you may mine some transparent coins and send to the test z_key.
//! When tests are run for the first time (or have not been run for a long) synching to fill ZOMBIE_wallet.db is started which may take hours.
//! So it is recommended to run prepare_zombie_sapling_cache to sync ZOMBIE_wallet.db before running zcoin tests:
//! cargo test -p coins --features zhtlc-native-tests -- --nocapture prepare_zombie_sapling_cache
//! If you did not run prepare_zombie_sapling_cache waiting for ZOMBIE_wallet.db sync will be done in the first call to ZCoin::gen_tx.
//! In tests, for ZOMBIE_wallet.db to be filled, another database ZOMBIE_cache.db is created in memory,
//! so if db sync in tests is cancelled and restarted this would cause restarting of building ZOMBIE_cache.db in memory
//!
//! Note that during the ZOMBIE_wallet.db sync an error may be reported:
//! 'error trying to connect: tcp connect error: Can't assign requested address (os error 49)'.
//! Also during the sync other apps like ssh or komodo-cli may return same error or even crash. TODO: fix this problem, maybe it is due to too much load on TCP stack
//! Errors like `No one seems interested in SyncStatus: send failed because channel is full` in the debug log may be ignored (means that update status is temporarily not watched)
//!
//! To monitor sync status in logs you may add logging support into the beginning of prepare_zombie_sapling_cache test (or other tests):
//! common::log::UnifiedLoggerBuilder::default().init();
//! and run cargo test with var RUST_LOG=debug

use bitcrypto::dhash160;
use common::{block_on, now_sec};
use mm2_core::mm_ctx::MmCtxBuilder;
use mm2_test_helpers::for_tests::zombie_conf;
use std::time::Duration;
use zcash_client_backend::encoding::decode_extended_spending_key;

use super::{z_coin_from_conf_and_params_with_z_key, z_mainnet_constants, PrivKeyBuildPolicy, RefundPaymentArgs,
            SendPaymentArgs, SpendPaymentArgs, SwapOps, ValidateFeeArgs, ValidatePaymentError, ZTransaction};
use crate::z_coin::{z_htlc::z_send_dex_fee, ZcoinActivationParams, ZcoinRpcMode};
use crate::{CoinProtocol, SwapTxTypeWithSecretHash};
use crate::{DexFee, DexFeeBurnDestination};
use mm2_number::MmNumber;

fn native_zcoin_activation_params() -> ZcoinActivationParams {
    ZcoinActivationParams {
        mode: ZcoinRpcMode::Native,
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn zombie_coin_send_and_refund_maker_payment() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let mut conf = zombie_conf();
    let params = native_zcoin_activation_params();
    let pk_data = [1; 32];
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
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let mut conf = zombie_conf();
    let params = native_zcoin_activation_params();
    let pk_data = [1; 32];
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
async fn zombie_coin_send_dex_fee() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let mut conf = zombie_conf();
    let params = native_zcoin_activation_params();
    let priv_key = PrivKeyBuildPolicy::IguanaPrivKey([1; 32].into());
    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();
    let protocol_info = match serde_json::from_value::<CoinProtocol>(conf["protocol"].take()).unwrap() {
        CoinProtocol::ZHTLC(protocol_info) => protocol_info,
        other_protocol => panic!("Failed to get protocol from config: {:?}", other_protocol),
    };

    let coin = z_coin_from_conf_and_params_with_z_key(&ctx, "ZOMBIE", &conf, &params, priv_key, z_key, protocol_info)
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

#[tokio::test(flavor = "multi_thread")]
async fn zombie_coin_send_standard_dex_fee() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let mut conf = zombie_conf();
    let params = native_zcoin_activation_params();
    let priv_key = PrivKeyBuildPolicy::IguanaPrivKey([1; 32].into());
    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();
    let protocol_info = match serde_json::from_value::<CoinProtocol>(conf["protocol"].take()).unwrap() {
        CoinProtocol::ZHTLC(protocol_info) => protocol_info,
        other_protocol => panic!("Failed to get protocol from config: {:?}", other_protocol),
    };

    let coin = z_coin_from_conf_and_params_with_z_key(&ctx, "ZOMBIE", &conf, &params, priv_key, z_key, protocol_info)
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
    let params = native_zcoin_activation_params();
    let priv_key = PrivKeyBuildPolicy::IguanaPrivKey([1; 32].into());
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
        z_key,
        protocol_info,
    ))
    .unwrap();

    while !block_on(coin.is_sapling_state_synced()) {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn zombie_coin_validate_dex_fee() {
    let ctx = MmCtxBuilder::default().into_mm_arc();
    let mut conf = zombie_conf();
    let params = native_zcoin_activation_params();
    let priv_key = PrivKeyBuildPolicy::IguanaPrivKey([1; 32].into());
    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();
    let protocol_info = match serde_json::from_value::<CoinProtocol>(conf["protocol"].take()).unwrap() {
        CoinProtocol::ZHTLC(protocol_info) => protocol_info,
        other_protocol => panic!("Failed to get protocol from config: {:?}", other_protocol),
    };

    let coin = z_coin_from_conf_and_params_with_z_key(&ctx, "ZOMBIE", &conf, &params, priv_key, z_key, protocol_info)
        .await
        .unwrap();

    // https://zombie.explorer.lordofthechains.com/tx/9390a26810342151f48f455b09e5d087a5429cbba08f2381b02c43b76f813e29
    let tx_hex = "0400008085202f8900000000000001030c00e8030000000000000169e7017fbd969be53da2c1b8812002baaf59ce98b230a9c1001397ba7f4db8676bd77e8ea644b67067d1f996d8d81c279961343f00a10095bccbddc341c98539287c900cf969688ddc574786e0e34bd6d3ec2ffaab5e2d472848781b116906669786c14c5c608b20dc23c9566fd46861f6a258b5ffc6de73495b56f4823e098c8664eab895d5cd31c013428ae2cbe940dc236ca40465ea2b912ce6c36555b2affb1f38b99b28dc593d865b0b948d567f9315df666d2e65e666d829b9823154bae0410bd885582b4a8a6eb4b9ae214b59ffd9b1167b7cd48f48a11cbd67c08f4e01ed4fd78fc91d0c9e70baa4f25761ef6c78cd7268b307aaa6ece2b443937eb4beac2c8843279a8879adbe0b381e65d0b674f2feeb54b78f80b377f66baab72c4cf9f10dde48f343c001df91a1a6d252ad8eca26eea0fdee49ad7024b505e55b4e082e94616794ddd7c2b852594b4b7af2292f0aa9e34f38322f548f1a21c015e92dbfd239ce18144f3b8045e9efa3de6b4c6b338f01d0adeb26a088a3c8c00503b67b2980b7663e97541e2944e4ad3588554966b6a930d2dc01d9fc7f8a846583fcf3b721f979705eff5bb9bb1fb0cad9ad941ceb3f581710efd8c50713a53751a0a196322ef8618bf1e097383666e91b5133ba81645d2b542181476eba2326cd02fb29a9f09edc46ea04b32ed9243597318d23b955a2570d78cbfb46cc26c1807eddd1de4785b6e752f859f7e25fc67f9e8a00feafac6fd7781eb72a663d9b80c10e9c387abc4d41294b3573785fd53bc56ccac2edf5c7bbb99cb3bcf87161fa893d2e1aabfee75754767cef07a12e44bb707720e727e585a258356cc797ecee8263c0f61cfc8ffa0360c758f1348ac44c186e12ce0f4faad43b4638abd4a0bc9fd4a6fa4352c20cc771241f95c26f1671ca95c8f4a63a8318dc43299f54e8a899df78ccfd3112a0d5ea637847dd2e3b05be8c0658dd0d7d814473fa5369957c00e84df600df23faaee5faa17b9ededad4731e5e9c1099dfddf5264756800dcfcad4b006b736d1d47c59a019acde4dc22249fc40846b77b43294e32a21db745e1bec790324c3d505edc79388a6e44b02841b26306ed48cfce1e941642c30792315016dba03797c8e4e279eec5b78aad602620471f24c25aea3aaa57509aa9eef2057f11bc95bad708918f2f0df74ac179d7dffc772b2c603dd89e7aea0e8f94f1a8bab4a4fba10bf05c88fbe4b021b3faff3d558e32e4bc20be4bed62d653674ce697390e098e590a3e354cb4a1e703474de8aab30cd76cf7e237f2e66bf486c4fc6c22028764e95adf7d8fa018f44b51ae6acfa3bf80f14c45c06623b916d79649abe0a2b229f96e60e421f6e734160da37f01e915cf73d1cacd1eb7f06c26c33b4d8e4dde264f3cfe84bada0601d1c03aa31c5938750ca0b852f3177883cae9f285d582a4eb38c05f8ef6e5cff5be0745e1ec66e20752bfd5bd5a1590fa280ace3e9786e0022e7ae3c48bcca14e9c5513bc8b57e15820a685f8348159862be0579a35d8ac9d1abaf36d9274c7e750fd9ad265c0d8f08c95ed9ce69eef3a55aef05f2d5d601f80f472689f3428e4f0095829a459813d5dace7e6137a752ae5567982e67b2092afeba99561fbe4e716f67bd1b4e8de1f376dec30eed27371bcc42d7de2ea0f4288054618e9afa002a2d1996b7a70a9683229f28bab811b67629dad527f325c0f12e19d92bac51e5924f27048fa118673b52b296b3642ec946d9915ded0ae84e1a2236da65f672bdad75a22cc0ea751c07e56d2ec22caa41afc98ec6b37a8c1b6a5378a81f2cdb2228f4efb8d7f35c0086a955e1b04bd09bd7e056c949fab1805f733a8b2061adad0c2b7fae33d21363de911e517b21a1539dfa1b3cbb1ea0dbfa3ffff23bbac01183f852de41e798fca5a278b711893175aeaded90873574d8de30b360f39ea239492c630eda4a811d3bb7a125054d5ca74bb6698aeea1a417ad19415ca0e5ca36abc2f96725986f73bcbe3113e391010d08f58f05979c7cef26ff92506c5d1eb2a2f6f5689e9a39957f0723bef3262f5190de996234d4f00b73ed74d78fdf1e6bf31161e16bd083bc6fbddc4eba85c17067e15f08019e5ed943de8e23a974d516abc641e85e641b03779816c30b3449a16b142417c1ff93ab7fa8f96a175e9ef73b3f06ac76788c27889d426efa78d5b8ce35be4591902f7766fe579a0aa28229235a920d26264c09625dea807f619a040f08931d6e1fe57ff0c48ea476be93a16d1fc8de3617984eeebcf14b63c839b41f8f9305402d1288c8e481a4fa5c3302bb1f83e3f0dc8ff9550f9bacb44bccb58f3de152abef5d578afed1c29dc89495b9e54a0c6d00f1dba45a2cf68c9512d9a9ff0b2531e58e47428a99cb246ca23f867b660dc71785b57407cc292f735634c602409792c4640831809f1f1e51903273b623aa0ae0cdd335c7b9db360b0bceb0d15f2313e1944800f30f82ed5bb07cfa1c4740c2bf2806539a4afac1f79d779b923ad8dc2493ebb2d2fce9aea58a009d64e7d1b71ca6893b076e41f7e88a4b51b5402e3fa6c60fa65a686adea229f0164318c9fa1b6d2d2218e5ada710daffecb6b7dd8bf7447658795c4c7a0ad710c4f02fd19017a0575f9467600cdca019793f2f49d197dbfc937828e5790b90929e5ca16037ec79734b64feec36b36c220a2979c45dd51e24c9fb21d8634471aac20c6f179f90c0d61c7b3d89826d146b157bedd8f6b66f6edfabfe04b49f2f2d999fc2e578a440bafd524c82ae614dc8017e379cf926e042f4fbd6f0628fde52de18d764ba8385b77569eda30d5a3617fb0a0c7fd26c821308c3ae98498d33b974cb318a04af3ea3fbcb13fc62fc952aaef095423da9ec7bdc7b77adbd403931189ddc98fe19a06711415b40a9a68812bb7c5453b7b2377910c7b89c99b379e038a7940487c0fd2405456ee55ab6ead3ef25a8a5b1abcae479c24f5e6869057e0bdabcdf352b4a64a3e385171a6e14c8102b2a187034e21705e3a457167fe0dc0d63d6e8d489c9a18c9d84b541504d36b086c2c63cc1a34c0080122c5d60ca33ab60289d16f21e1ded753607267c2093b1c587b89da9df65584fbe3ff9eb7f91d64e33912b8e91adc27191d22f8e835be6bb24546f21488f7abcb29339c34058d4f4093096144b17b8ab76a346275b7e7c80bca59d20e0bb482bb2a9cc3c9515cc1b5be17348c65c73e9fb1ed77d423c509f7cff0e355a34d080d310f3b848dbc209bbba6b6b109fb8d9556dca0fab086e197327ab423d5d762b68961244d8d22c30a8a3a116770bb15b5a0a347091a843b68d6a8e0f1c79f12523a7561c1233cd44db90f6cd3c1ce5fc13f8382177b5522aae028379269b71ae2a42f41dff7374ed7e83c89566f57297b82478b04359a2c199ce8f842112b7450cc1e2e2e394cda4c67e0b2302e21f6af997607ceefd067f77be8900bb3ecb3e30782477aa76861b286b9ddc9e36fcebb50f04f9516e02da31e6219bb5bcb81ee673d95be14c1bd2be4909556d6dbca0365292c582dedcafcc60b255ab7bcd9d977a4139f394ca1da81040e784fd8e7534f230bc5201e7f1db47eadc30f37609d5bbaba624157d98d65029bbab766b6c23c3049a32b894c0cfcb40913ba1cd2d5acda7d2acc920fd01c36f28fc6b7ffd01a37b17fc3235d0dbe9b8098530bed6894b288604b8689f4aafc22cdf211fb95ef5c90cae62a250234e6f790e9a15012acac88305dc4f91fd564a9ab8bb27c057ec5dd46fe952a7be557caea9b7b1d6118aa42df79b8c207e2bae6c34d67dc32b4360ad20b3e609e9caeb7f432ad51cfce139f2d4eb9ed219f4323acd5685e0e0409939eb662175a83fa083f500516dbcb091a3448cb24c3198c8fc547fbda3cb0894edeceef7ccb4ad746aa06f4038b63ab4095a9c390656520561ba3763b1057b3af7cb548342a2bfc2ab725b01b12a7adfc30d7d9632acafd2595cde406b8637a911b7c86f7b09b11f58acec3f1a1bd7cf6853331b48d7907ed699d91fbdbcab8001e3d8d3a26b491b6e2d98c5e149847a07a2b7faa1f567cd4bc9c83ad553339632f3dcacb890c5222656b3349ddd5c8eacaa490ac0b2b38f8a26da9ce7789f5601769a7f10b93125cb93b589bda4ddb4e8795817b60cc149af7c0699b2bbbf655f2f5ec170d6af51213e8c725e699d181923ecf10c6f1069f46e6bc89c7a29d2ebe133b5c0c4b67826a93add7d4824e60b4c5f0cee358abedb50c54a59e95185d7a80081f2dddba5c7c7c637b2dfe8575ddaa71306a2725c9ec17b8e4e1f271a442f6798cc21bbd55c2d69819ddde37a8e8d6a812c41a3e58719b7c96e9375155c4a873ed698ad37144ef32e3fe41cce9c48bbe31441dbbeec7b97734769063d6d04cd8d4963f09f7101bf57cb97a83452cc5de873c5ac0ce001c471c9fcd3275d90a118dd4c25a525d9fb358ff85104b98136850786b387fa17cc1a1d128bc5f7c365ec7920ea677e4c8023071a958647d9fbd27e29d7d099b4dfbbac086ac2af00407fd12092ef1f4847bf8988d839e49a6b5b42482c3dde77022ace66e1ca15b46f2df88d053c1bc3623110b3be74b08749eba6d22f87a44cf7cc1997e7e45d0e";
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
    let err = coin.validate_fee(validate_fee_args).await.unwrap_err().into_inner();
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
    let err = coin.validate_fee(validate_fee_args).await.unwrap_err().into_inner();
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
    let err = coin.validate_fee(validate_fee_args).await.unwrap_err().into_inner();
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
    coin.validate_fee(validate_fee_args).await.unwrap();

    // Test old standard dex fee with no burn output
    // TODO: disable when the upgrade transition period ends

    // https://zombie.explorer.lordofthechains.com/tx/9eb7fc697b280499df33e5838af6e67540d436fd8f565f47a7f03e6013e8342c
    let tx_2_hex = "0400008085202f8900000000000006030c00e80300000000000001c167d6e78e09dfbac2973bfd8acac75fc603f6ffb481377e3ec790f1cc812a8a3979ecfb8a0c7c3a966d90675261568550f9363f9384a21390d7f58bde6f7b03270d88e1fa61d739c27d7f585c9bbc81a3d522fbb88fe8dc8567e27a048d475ce14fdfd11455fd54c577538438decbf6954f1ffba86c78896178ce514c5f1762a7de9e83552533eb4c558c4f9950b1806f266b25d6437f5aac08048d6f48100d49ecb2253e85c3b555a7cd84c9628ae58e5d68ddad61e69edfcdc0fa12170dd80340c417bff9e1711bf6e9728a6a52c42598d7ffd00c35679b1555cab075e54b134901d02ca9b07bb20c5719b2728faa020fb844c183c2ae649034a5476c4d129c3f97cd00a87be1ca7e73d027188cdab57fbb34b5addb7432f51454299b8cf47b389f98bad8abd42d82a2f8c2d11312e39272d44409540bcfa4c6b445e8e6dc63cc2fd5db1448875adb055ea8665c863bd07bf3aa8eb210f638287789957c96c54819061ee215eb7ba7b6048591a57f097a3e5da06b6359325d830d5b74c20c025996a113e4bb9fd2c853b7360d4961396cd99c23a13de972097eede3a955a5d5d8c8695a7290581a248fc03ea87606e71564d8e8fb00ebb8d5c10fc8fefe1660171524264060d15363fc2dc0ac0ab21fcbae1dc53786873cb9e8716f3ada651e79c3306ad49adeeb354213cc37499e217fa1c0f219e85bd22cf493f5e76f053543dd3b36bd180b1dcf17f781e35d6955c33c06426a885138f1e21b78ee87a27624f33b6567bfa6a0fe43e2d623578f6917d300a408c4dd48683213ffad453de1003e120fbfa74a6db4628af9d446e26492fde67bf52d034fcaf2b9b959472404fd631ef599815c6f190807b75f638e134148a5813424ba6cf59cf86ce515a14b95f7b8f80b1aa1b3cbbc091fa2a686277a9cc613e48b2c227aed7b4b093ac8b12a238bc99f9983c8bac21bb0f897eada35bf0e01b1436cf6d44b959595bdcdfd4676e28b500b9ad6b8a5825c3d3c0c38a4a5a2c3ded205584439621eaa7ee639b09aca1f533bb4892b29d761d94887fa78f605b9b8f5b3ab44ea578d9329bd78d7a6ae903f1960e16007a924be79ab31ea6ed7466485488b5c71eb02d6b99f345f2f61cb3cd994045c502d19f615233b3ebb263981de26674de082d384cc04c09a309567780f7f24298847fc2dff5f22082074684aa9efa260b8aaf4357bff2e9d32f8918b16876051b5459136dcc8788aba7b2ead435c3bf662f9f1acddd4a8a71b593e99ed50e158028946195ee991666bf88f4cf4d30a04c877ce8a9e6d224aed662e85a32f5cb9029a3dd4ba663b6f6314ef58fbce623171946d01d1ff456f90131159e5209cb41329061a0dd8a5fc35576108681e783fb173f67dda33134a9b1f07494a1d6273810fd77a25c92f7444d6226738d5c7161b7b198be069ac65d50a22d728292e95d1859e0c646db62aa3f401e55026a551b1edfe8fd5eca8e4c6836bd09429b5e22f64a09db4c6935b6febcbac6430f66dc0280c9be046133795f1f59ec32cbf4511749984f7b2ba131588f86f82322901ee7d709550ecadb5b915d5cfb2e950d2a8c5eda57da49d2ac9562b851f81e70a32178989e83807f04a6324cf7320a26a91b41e31a06c706431794ffb8b9ce5f3d853fb9106c8a98ea3b2948356948bfbbd63eb30e3cb68d7e373df80221d1b1211c717afe8b7b0b46a3208859254d9ae3517b8e031f413178c0fd408e76ccdc580a9a19edf4b3c70c273f4c8c626fad225e5aeee890c65328437b8bf316066e54a4741d8ac8ab9b5555f09b89b79165f9aa08a59be8f10c121b1b425bd5e3a64b6e4db3e1cacb00a5867fd05b454b75ff1eb8560770f21af7680107560a2209373d2999eb21bed2a10bafe1eaf5a31c18e69c63cce9b8c6cddcfc1088f956bcf3c9adeb77ef0589ab6405f0a9ba5650819a48fb42597fcd2f4ad67bdc89870d82eaa0d8dbd298a59ff552576dedb539834de725638e0f68307d4ac203d8e2e4649e31abc4e8748251c8fb6df3459300d1badfc19ad4d2f680f466b02680bb3e5a13c0c8a5db3665bc9fc2093c4d38acb176754db556ebd1663c23f284bec95279957b112131f8aa09af15ff26eebea3215c96b9df43c9fc9134d9db4e588aff293f3084db13e1d92bc33ca07a1b534b4a4e5fcbf098be7d26f9312db7f9d6b160318a4562c3c3b0c87688c59f402e0032242324339ef33713bf39c2110e7eb155bf926888385fe4b18bf3ef13dc2601b76def3d763f5b2ddea363f7e3697112194fb6332be96540a53a86e1e34fd70429dcfc39c5e2f68fa72e0045fe4ef12b965f0827c5bee9cd4f0c9b4cf6468316384fe33df5703c7742f9b409b9a508e94faa8be3c27ad75d21f85ee31753c96deb909221befd62bae084885c890d89f775dc0eee940ffbcad0aa65c08a71d09e234ad150e82610ba03deb608d44e9019d8579f9e9351daa6f3bcbbc8ec170c8b700bcb495c333b32136721f6417a3f3b12500641eb7af9e5813fafd27794a7b2476320fde18f3019302d49d77c3536af214e6c8357a36029a37a07011d1cdbe0db3fe7443a6908f5d3b6e08d61f33bad2a0bfbc9db86022d4f91b0ba6ef1b5ec30f0187f4c540eeb117c4d3d78659e46540df4b9301c6fce031d7e438abeb13a747be6ce9c0a33a2bd6f6092d0a26d5ba138bb6f2c3113ea6cff868853dacfc5df0433049a59d2b365e9a87ee6a6203e52121d60bc709feb1c1a30e95fbc600f648dfa5fadc8cf324a4c5d91e1f80501661aa51a518b381933932a1367e4369e07943f291012f5a9394692d9984fc2dc55c0ec4fe3d18a4a0b9f9d7c9d3f57b2e2a0c31f08f17ffe7355fec963b8ae364ed8cff046aa8220dc813f2dc78405069c707afadb77cfc8d64803a25eab7ebc74c738b41f9b3f2d881f1e2b77d37f38c1b5991daf5c911c04947891909f9c3e50e1314884207f0ea99d9310c9cfe93fea53fb57c93efbd412702e283e61196b9158de774333893b51c768ae48ec086e47b105d0b21357bd14f85b9f145fbfd63c0e998d6e54900915c8ffaf1234fa910ede3035e5e47ee9b22559459d0ea2b0f3242c5ec2782d09a7b477b560b1ecfd14d82f24600334d2c85dc2def0f457ea199e266c52fb9a596de02da05a9df8e4731cf941e1ada11c66d0954742745d5ef1b36dc7628614ed28ba9358ab38c2d007aa90147906270ab35ae26fa3473ec5881f8e6ed04c592a403386c4061becc70b5735531f8d249abb079317f43f111de58c6678e62a6d2dc83193acef928c906";
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
    let err = coin.validate_fee(validate_fee_args).await.unwrap_err().into_inner();
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
    coin.validate_fee(validate_fee_args).await.unwrap();
}
