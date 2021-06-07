use super::*;
use crate::z_coin::z_htlc::z_send_dex_fee;
use common::block_on;
use common::mm_ctx::MmCtxBuilder;
use common::now_ms;
use zcash_client_backend::encoding::decode_extended_spending_key;

#[test]
fn zombie_coin_send_and_refund_maker_payment() {
    let conf = json!({
        "coin": "ZOMBIE",
        "asset": "ZOMBIE",
        "fname": "ZOMBIE (TESTCOIN)",
        "txversion": 4,
        "overwintered": 1,
        "mm2": 1,
    });
    let req = json!({
        "method": "enable",
        "coin": "ZOMBIE"
    });

    let ctx = MmCtxBuilder::default().into_mm_arc();
    let priv_key = [1; 32];
    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();

    let coin = block_on(z_coin_from_conf_and_request_with_z_key(
        &ctx, "ZOMBIE", &conf, &req, &priv_key, z_key,
    ))
    .unwrap();

    let lock_time = (now_ms() / 1000) as u32 - 3600;
    let taker_pub = coin.utxo_arc.key_pair.public();
    let secret_hash = [0; 20];
    let tx = coin
        .send_maker_payment(lock_time, &*taker_pub, &secret_hash, "0.01".parse().unwrap(), &None)
        .wait()
        .unwrap();
    println!("swap tx {}", hex::encode(&tx.tx_hash().0));

    let refund_tx = coin
        .send_maker_refunds_payment(&tx.tx_hex(), lock_time, &*taker_pub, &secret_hash, &None)
        .wait()
        .unwrap();
    println!("refund tx {}", hex::encode(&refund_tx.tx_hash().0));
}

#[test]
fn zombie_coin_send_and_spend_maker_payment() {
    let conf = json!({
        "coin": "ZOMBIE",
        "asset": "ZOMBIE",
        "fname": "ZOMBIE (TESTCOIN)",
        "txversion": 4,
        "overwintered": 1,
        "mm2": 1,
    });
    let req = json!({
        "method": "enable",
        "coin": "ZOMBIE"
    });

    let ctx = MmCtxBuilder::default().into_mm_arc();
    let priv_key = [1; 32];
    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();

    let coin = block_on(z_coin_from_conf_and_request_with_z_key(
        &ctx, "ZOMBIE", &conf, &req, &priv_key, z_key,
    ))
    .unwrap();

    let lock_time = (now_ms() / 1000) as u32 - 1000;
    let taker_pub = coin.utxo_arc.key_pair.public();
    let secret = [0; 32];
    let secret_hash = dhash160(&secret);
    let tx = coin
        .send_maker_payment(lock_time, &*taker_pub, &*secret_hash, "0.01".parse().unwrap(), &None)
        .wait()
        .unwrap();
    println!("swap tx {}", hex::encode(&tx.tx_hash().0));

    let maker_pub = taker_pub;
    let spend_tx = coin
        .send_taker_spends_maker_payment(&tx.tx_hex(), lock_time, &*maker_pub, &secret, &None)
        .wait()
        .unwrap();
    println!("spend tx {}", hex::encode(&spend_tx.tx_hash().0));
}

#[test]
fn zombie_coin_send_and_refund_dex_fee() {
    let conf = json!({
        "coin": "ZOMBIE",
        "asset": "ZOMBIE",
        "fname": "ZOMBIE (TESTCOIN)",
        "txversion": 4,
        "overwintered": 1,
        "mm2": 1,
    });
    let req = json!({
        "method": "enable",
        "coin": "ZOMBIE"
    });

    let ctx = MmCtxBuilder::default().into_mm_arc();
    let priv_key = [1; 32];
    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();

    let coin = block_on(z_coin_from_conf_and_request_with_z_key(
        &ctx, "ZOMBIE", &conf, &req, &priv_key, z_key,
    ))
    .unwrap();

    let lock_time = (now_ms() / 1000) as u32 - 1000;
    let watcher_pub = coin.utxo_arc.key_pair.public();
    let (tx, redeem_script) = block_on(z_send_dex_fee(&coin, lock_time, watcher_pub, "0.01".parse().unwrap())).unwrap();
    println!("dex fee tx {}", hex::encode(&*tx.hash().reversed()));

    let script_data = ScriptBuilder::default().push_opcode(Opcode::OP_1).into_script();
    let refund = block_on(z_p2sh_spend(
        &coin,
        tx,
        lock_time,
        SEQUENCE_FINAL - 1,
        redeem_script,
        script_data,
    ))
    .unwrap();
    println!("dex fee refund tx {}", hex::encode(&*refund.hash().reversed()));
}

#[test]
fn zombie_coin_send_and_spend_dex_fee() {
    let conf = json!({
        "coin": "ZOMBIE",
        "asset": "ZOMBIE",
        "fname": "ZOMBIE (TESTCOIN)",
        "txversion": 4,
        "overwintered": 1,
        "mm2": 1,
    });
    let req = json!({
        "method": "enable",
        "coin": "ZOMBIE"
    });

    let ctx = MmCtxBuilder::default().into_mm_arc();
    let priv_key = [1; 32];
    let z_key = decode_extended_spending_key(z_mainnet_constants::HRP_SAPLING_EXTENDED_SPENDING_KEY, "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();

    let coin = block_on(z_coin_from_conf_and_request_with_z_key(
        &ctx, "ZOMBIE", &conf, &req, &priv_key, z_key,
    ))
    .unwrap();

    let lock_time = (now_ms() / 1000) as u32 - 1000;
    let watcher_pub = coin.utxo_arc.key_pair.public();
    let (tx, redeem_script) = block_on(z_send_dex_fee(&coin, lock_time, watcher_pub, "0.01".parse().unwrap())).unwrap();
    println!("dex fee tx {}", hex::encode(&*tx.hash().reversed()));

    let script_data = ScriptBuilder::default().push_opcode(Opcode::OP_0).into_script();
    let spend = block_on(z_p2sh_spend(
        &coin,
        tx,
        lock_time,
        SEQUENCE_FINAL,
        redeem_script,
        script_data,
    ))
    .unwrap();
    println!("dex fee spend tx {}", hex::encode(&*spend.hash().reversed()));
}
