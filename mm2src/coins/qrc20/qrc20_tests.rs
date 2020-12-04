use super::*;
use crate::TxFeeDetails;
use bigdecimal::Zero;
use chain::OutPoint;
use common::executor::spawn;
use common::mm_ctx::MmCtxBuilder;
use itertools::Itertools;
use mocktopus::mocking::{MockResult, Mockable};
use std::sync::{Arc, Mutex};

pub fn qrc20_coin_for_test(priv_key: &[u8]) -> (MmArc, Qrc20Coin) {
    let conf = json!({
        "coin":"QRC20",
        "decimals": 8,
        "required_confirmations":0,
        "pubtype":120,
        "p2shtype":50,
        "wiftype":128,
        "segwit":true,
        "mm2":1,
        "mature_confirmations":500,
    });
    let req = json!({
        "method": "electrum",
        "servers": [{"url":"95.217.83.126:10001"}],
        "swap_contract_address": "0xba8b71f3544b93e2f681f996da519a98ace0107a",
    });
    let contract_address = "0xd362e096e873eb7907e205fadc6175c6fec7bc44".into();
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let coin = unwrap!(block_on(qrc20_coin_from_conf_and_request(
        &ctx,
        "QRC20",
        "QTUM",
        &conf,
        &req,
        priv_key,
        contract_address
    )));
    (ctx, coin)
}

fn check_tx_fee(coin: &Qrc20Coin, expected_tx_fee: ActualTxFee) {
    let actual_tx_fee = block_on(coin.get_tx_fee()).unwrap();
    assert_eq!(actual_tx_fee, expected_tx_fee);
}

#[test]
fn test_withdraw_impl_fee_details() {
    Qrc20Coin::ordered_mature_unspents.mock_safe(|_, _| {
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

    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    let withdraw_req = WithdrawRequest {
        amount: 10.into(),
        to: "qHmJ3KA6ZAjR9wGjpFASn4gtUSeFAqdZgs".into(),
        coin: "QRC20".into(),
        max: false,
        fee: Some(WithdrawFee::Qrc20Gas {
            gas_limit: 2_500_000,
            gas_price: 40,
        }),
    };
    let tx_details = unwrap!(coin.withdraw(withdraw_req).wait());

    let expected: Qrc20FeeDetails = unwrap!(json::from_value(json!({
        "coin": "QTUM",
        // 1000 from satoshi,
        // where decimals = 8,
        //       1000 is fixed fee
        "miner_fee": "0.00001",
        "gas_limit": 2_500_000,
        "gas_price": 40,
        // (gas_limit * gas_price) from satoshi in Qtum
        "total_gas_fee": "1",
    })));
    assert_eq!(tx_details.fee_details, Some(TxFeeDetails::Qrc20(expected)));
}

#[test]
fn test_can_i_spend_other_payment() {
    let miner_fee = 1000;
    ElectrumClient::display_balance.mock_safe(move |_, _, decimal| {
        // one satoshi more than required
        let balance = QRC20_GAS_LIMIT_DEFAULT * QRC20_GAS_PRICE_DEFAULT + miner_fee + 1;
        let balance = big_decimal_from_sat(balance as i64, decimal);
        MockResult::Return(Box::new(futures01::future::ok(balance)))
    });

    let priv_key = [
        192, 240, 176, 226, 14, 170, 226, 96, 107, 47, 166, 243, 154, 48, 28, 243, 18, 144, 240, 1, 79, 103, 178, 42,
        32, 161, 106, 119, 241, 227, 42, 102,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);
    check_tx_fee(&coin, ActualTxFee::Fixed(miner_fee));

    let actual = coin.can_i_spend_other_payment().wait();
    assert_eq!(actual, Ok(()));
}

#[test]
fn test_can_i_spend_other_payment_err() {
    let miner_fee = 1000;
    ElectrumClient::display_balance.mock_safe(move |_, _, decimal| {
        // one satoshi less than required
        let balance = QRC20_GAS_LIMIT_DEFAULT * QRC20_GAS_PRICE_DEFAULT + miner_fee - 1;
        let balance = big_decimal_from_sat(balance as i64, decimal);
        MockResult::Return(Box::new(futures01::future::ok(balance)))
    });

    let priv_key = [
        192, 240, 176, 226, 14, 170, 226, 96, 107, 47, 166, 243, 154, 48, 28, 243, 18, 144, 240, 1, 79, 103, 178, 42,
        32, 161, 106, 119, 241, 227, 42, 102,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);
    check_tx_fee(&coin, ActualTxFee::Fixed(miner_fee));

    let error = coin.can_i_spend_other_payment().wait().err().unwrap();
    log!([error]);
    assert!(error.contains("Base coin balance 0.04000999 is too low to cover gas fee, required 0.04001"));
}

#[test]
#[ignore]
fn test_send_maker_payment() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    let timelock = (now_ms() / 1000) as u32 - 200;
    let taker_pub = hex::decode("022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1a").unwrap();
    let secret_hash = &[1; 20];
    let amount = BigDecimal::from_str("0.2").unwrap();
    let payment = coin
        .send_maker_payment(timelock, &taker_pub, secret_hash, amount)
        .wait()
        .unwrap();
    let tx = match payment {
        TransactionEnum::UtxoTx(tx) => tx,
        _ => panic!("Expected UtxoTx"),
    };

    let tx_hash: H256Json = tx.hash().reversed().into();
    log!([tx_hash]);
    let tx_hex = serialize(&tx);
    log!("tx_hex: "[tx_hex]);

    let confirmations = 1;
    let requires_nota = false;
    let wait_until = (now_ms() / 1000) + 240; // timeout if test takes more than 240 seconds to run
    let check_every = 1;
    unwrap!(coin
        .wait_for_confirmations(&tx_hex, confirmations, requires_nota, wait_until, check_every)
        .wait());
}

#[test]
fn test_validate_maker_payment() {
    // this priv_key corresponds to "taker_passphrase" passphrase
    let priv_key = [
        24, 181, 194, 193, 18, 152, 142, 168, 71, 73, 70, 244, 9, 101, 92, 168, 243, 61, 132, 48, 25, 39, 103, 92, 29,
        17, 11, 29, 113, 235, 48, 70,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    assert_eq!(coin.utxo.my_address, "qUX9FGHubczidVjWPCUWuwCUJWpkAtGCgf".into());

    // tx_hash: 016a59dd2b181b3906b0f0333d5c7561dacb332dc99ac39679a591e523f2c49a
    let payment_tx = hex::decode("010000000194448324c14fc6b78c7a52c59debe3240fc392019dbd6f1457422e3308ce1e75010000006b483045022100800a4956a30a36708536d98e8ea55a3d0983b963af6c924f60241616e2ff056d0220239e622f8ec8f1a0f5ef0fc93ff094a8e6b5aab964a62bed680b17bf6a848aac012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffff020000000000000000e35403a0860101284cc49b415b2a0c692f2ec8ebab181a79e31b7baab30fef0902e57f901c47a342643eeafa6b510000000000000000000000000000000000000000000000000000000001312d00000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc44000000000000000000000000783cf0be521101942da509846ea476e683aad8320101010101010101010101010101010101010101000000000000000000000000000000000000000000000000000000000000000000000000000000005f72ec7514ba8b71f3544b93e2f681f996da519a98ace0107ac201319302000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88ac40ed725f").unwrap();
    let time_lock = 1601367157;
    // pubkey of "cMhHM3PMpMrChygR4bLF7QsTdenhWpFrrmf2UezBG3eeFsz41rtL" passphrase
    let maker_pub = hex::decode("03693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9").unwrap();
    let secret_hash = &[1; 20];
    let amount = BigDecimal::from_str("0.2").unwrap();

    unwrap!(coin
        .validate_maker_payment(&payment_tx, time_lock, &maker_pub, secret_hash, amount.clone())
        .wait());

    let maker_pub_dif = hex::decode("022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1a").unwrap();
    let error = unwrap!(coin
        .validate_maker_payment(&payment_tx, time_lock, &maker_pub_dif, secret_hash, amount.clone())
        .wait()
        .err());
    log!("error: "[error]);
    assert!(
        error.contains("Payment tx was sent from wrong address, expected 0x783cf0be521101942da509846ea476e683aad832")
    );

    let amount_dif = BigDecimal::from_str("0.3").unwrap();
    let error = unwrap!(coin
        .validate_maker_payment(&payment_tx, time_lock, &maker_pub, secret_hash, amount_dif)
        .wait()
        .err());
    log!("error: "[error]);
    assert!(error.contains("Unexpected 'erc20Payment' contract call bytes"));

    let secret_hash_dif = &[2; 20];
    let error = unwrap!(coin
        .validate_maker_payment(&payment_tx, time_lock, &maker_pub, secret_hash_dif, amount.clone())
        .wait()
        .err());
    log!("error: "[error]);
    assert!(error.contains("Payment state is not PAYMENT_STATE_SENT, got 0"));

    let time_lock_dif = 123;
    let error = unwrap!(coin
        .validate_maker_payment(&payment_tx, time_lock_dif, &maker_pub, secret_hash, amount)
        .wait()
        .err());
    log!("error: "[error]);
    assert!(error.contains("Payment state is not PAYMENT_STATE_SENT, got 0"));
}

#[test]
fn test_wait_for_confirmations_excepted() {
    // this priv_key corresponds to "taker_passphrase" passphrase
    let priv_key = [
        24, 181, 194, 193, 18, 152, 142, 168, 71, 73, 70, 244, 9, 101, 92, 168, 243, 61, 132, 48, 25, 39, 103, 92, 29,
        17, 11, 29, 113, 235, 48, 70,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    assert_eq!(coin.utxo.my_address, "qUX9FGHubczidVjWPCUWuwCUJWpkAtGCgf".into());

    // tx_hash: 35e03bc529528a853ee75dde28f27eec8ed7b152b6af7ab6dfa5d55ea46f25ac
    // `approve` contract call excepted only, and `erc20Payment` completed
    let payment_tx = hex::decode("0100000003b1fcca3d7c15bb7f694b4e58b939b8835bce4d535e8441d41855d9910a33372f020000006b48304502210091342b2251d13ae0796f6ebf563bb861883d652cbee9f5606dd5bb875af84039022077a21545ff6ec69c9c4eca35e1f127a450abc4f4e60dd032724d70910d6b2835012102cd7745ea1c03c9a1ebbcdb7ab9ee19d4e4d306f44665295d996db7c38527da6bffffffff874c96188a610850d4cd2c29a7fd20e5b9eb7f6748970792a74ad189405b7d9b020000006a473044022055dc1bf716880764e9bcbe8dd3aea05f634541648ec4f5d224eba93fedc54f8002205e38b6136adc46ef8ca65c0b0e9390837e539cbb19df451e33a90e534c12da4c012102cd7745ea1c03c9a1ebbcdb7ab9ee19d4e4d306f44665295d996db7c38527da6bffffffffd52e234ead3b8a2a4718cb6fee039fa96862063fccf95149fb11f27a52bcc352010000006a4730440220527ce41324e53c99b827d3f34e7078d991abf339f24108b7e677fff1b6cf0ffa0220690fe96d4fb8f1673458bc08615b5119f354f6cd589754855fe1dba5f82653aa012102cd7745ea1c03c9a1ebbcdb7ab9ee19d4e4d306f44665295d996db7c38527da6bffffffff030000000000000000625403a08601012844095ea7b3000000000000000000000000ba8b71f3544b93e2f681f996da519a98ace0107a0000000000000000000000000000000000000000000000000000000001312d0014d362e096e873eb7907e205fadc6175c6fec7bc44c20000000000000000e35403a0860101284cc49b415b2a756dd4fe3852ea4a0378c5e984ebb5e4bfa01eca31785457d1729d5928198ef00000000000000000000000000000000000000000000000000000000001312d00000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc440000000000000000000000000240b898276ad2cc0d2fe6f527e8e31104e7fde30101010101010101010101010101010101010101000000000000000000000000000000000000000000000000000000000000000000000000000000005f686cef14ba8b71f3544b93e2f681f996da519a98ace0107ac21082fb03000000001976a914f36e14131c70e5f15a3f92b1d7e8622a62e570d888acb86d685f").unwrap();

    let confirmations = 1;
    let requires_nota = false;
    let wait_until = (now_ms() / 1000) + 1; // the transaction is mined already
    let check_every = 1;
    unwrap!(coin
        .wait_for_confirmations(&payment_tx, confirmations, requires_nota, wait_until, check_every)
        .wait());

    // tx_hash: ed53b97deb2ad76974c972cb084f6ba63bd9f16c91c4a39106a20c6d14599b2a
    // `erc20Payment` contract call excepted
    let payment_tx = hex::decode("01000000014c1411bac38ca25a2816342b019df81f503e1db75b25c6da618b08484dc2ff49010000006b483045022100da3e90fbcc45a94573c28213b36dc616630e3adfa42a7f16bdf917e8a76b954502206ad0830bb16e5c25466903ae7f749e291586726f1497ae9fc2e709c1b6cd1857012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffff040000000000000000625403a08601012844095ea7b3000000000000000000000000ba8b71f3544b93e2f681f996da519a98ace0107a000000000000000000000000000000000000000000000000000000000000000014d362e096e873eb7907e205fadc6175c6fec7bc44c20000000000000000625403a08601012844095ea7b3000000000000000000000000ba8b71f3544b93e2f681f996da519a98ace0107a000000000000000000000000000000000000000000000000000000000000000a14d362e096e873eb7907e205fadc6175c6fec7bc44c20000000000000000e35403a0860101284cc49b415b2a0a1a8b4af2762154115ced87e2424b3cb940c0181cc3c850523702f1ec298fef0000000000000000000000000000000000000000000000000000000000000064000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc44000000000000000000000000783cf0be521101942da509846ea476e683aad8324b6b2e5444c2639cc0fb7bcea5afba3f3cdce239000000000000000000000000000000000000000000000000000000000000000000000000000000005fa0fffb14ba8b71f3544b93e2f681f996da519a98ace0107ac2493d4a03000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88acae2ea15f").unwrap();
    let error = unwrap!(coin
        .wait_for_confirmations(&payment_tx, confirmations, requires_nota, wait_until, check_every)
        .wait()
        .err());
    log!("error: "[error]);
    assert!(error.contains("Contract call failed with an error: Revert"));

    // tx_hash: aa992c028c07e239dbd2ff32bf67251f026929c644b4d02a469e351cb44abab7
    // `receiverSpend` contract call excepted
    let payment_tx = hex::decode("0100000007077ccb377a68fd6079503f856df4e553e337015f8419cd0f2a949c31db175df7050000006a473044022058097f54be31ae5af197f72e4410b33b22f29fad5b1a1cefb30ee45b3b3477dc02205c1098850fa2f2c1929c27af6261f83abce7682eb769f909dd09e9be5e0bd469012102aa32922f4b05cbc7384dd85b86021c98e4102f5da3df48bc516aa76f8119559affffffffc191895a431db3dccbf4f9d4b8cd8301124343e66275194ad734a77ffe56b95e030000006a4730440220491fed7954c6f43acc7226c337bb16ac71b38df50f55a819441d9b2b9e4a04b502201f95be6941b6619c0ca246e15adb090b82cd908f7c85108a1dcc02eafb7cc725012102aa32922f4b05cbc7384dd85b86021c98e4102f5da3df48bc516aa76f8119559afffffffff678de174fb81d3820df43a2c29945b08df4fb080deb8088ef11b3711c0fe8df020000006a473044022071d9c0ec57ab23360a4f73d0edfc2f67614b56f6d2e54387b39c3de1fa894c7d022030ea65d157784ff68cae9c9acb0dd626205073f478003b1cb1d0d581dcb27b1c012102aa32922f4b05cbc7384dd85b86021c98e4102f5da3df48bc516aa76f8119559affffffffe1ef8740ce51ed3172efea91a5e559b5fe63dc6fede8a9037ad47fbc38560b51040000006a47304402203f056dff0be1f24ed96c72904c9aac3ac964913d0c3228bfab3fa4bef7f22c060220658a121bf8f29d86c18ec1aee4460f363c0704d2f05cc9d7923e978e917f48ca012102aa32922f4b05cbc7384dd85b86021c98e4102f5da3df48bc516aa76f8119559affffffffe825dea61113bbd67dd35cbc9d88890ac222f55bf0201a7f9fb96592e0614d4d080000006b483045022100bb10f195c57c1eed9de3d9d9726484f839e25d83deb54cf2142df37099df6a8d02202a025182caaa5348350b410ee783180e9ce3ccac5e361eb50b162311e9d803f1012102aa32922f4b05cbc7384dd85b86021c98e4102f5da3df48bc516aa76f8119559affffffffe1ef8740ce51ed3172efea91a5e559b5fe63dc6fede8a9037ad47fbc38560b51060000006a47304402205550e0b4e1425f2f7a8645c6fd408ba0603cca5ca408202729041f5eab0b0cd202205c98fc8e91a37960d38f0104e81d3d48f737c4000ef45e2372c84d857455da34012102aa32922f4b05cbc7384dd85b86021c98e4102f5da3df48bc516aa76f8119559affffffffe825dea61113bbd67dd35cbc9d88890ac222f55bf0201a7f9fb96592e0614d4d060000006b483045022100b0d21cbb5d94b4995d9cb81e7440849dbe645416bca6d51bb5450e10753523220220299f105d573cdb785233699b5a9be8f907d9821a74cfd91fb72911a4a6e1bdb8012102aa32922f4b05cbc7384dd85b86021c98e4102f5da3df48bc516aa76f8119559affffffff020000000000000000c35403a0860101284ca402ed292be8b1d4904e8f1924bd7a2eb4d8085214c17af3d8d7574b2740a86b6296d343c00000000000000000000000000000000000000000000000000000000005f5e10028fcc0c5f6d9619d3c1f90af51e891d62333eb748c568f7da2a7734240d37d38000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc44000000000000000000000000d020b63f5a989776516bdc04d426ba118130c00214ba8b71f3544b93e2f681f996da519a98ace0107ac270630800000000001976a914fb7dad7ce97deecf50a4573a2bd7639c79bdc08588aca64aaa5f").unwrap();
    let error = unwrap!(coin
        .wait_for_confirmations(&payment_tx, confirmations, requires_nota, wait_until, check_every)
        .wait()
        .err());
    log!("error: "[error]);
    assert!(error.contains("Contract call failed with an error: Revert"));
}

#[test]
#[ignore]
fn test_taker_spends_maker_payment() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, maker_coin) = qrc20_coin_for_test(&priv_key);

    // priv_key of qUX9FGHubczidVjWPCUWuwCUJWpkAtGCgf
    let priv_key = [
        24, 181, 194, 193, 18, 152, 142, 168, 71, 73, 70, 244, 9, 101, 92, 168, 243, 61, 132, 48, 25, 39, 103, 92, 29,
        17, 11, 29, 113, 235, 48, 70,
    ];
    let (_ctx, taker_coin) = qrc20_coin_for_test(&priv_key);

    let bob_balance = taker_coin.my_balance().wait().unwrap();

    let timelock = (now_ms() / 1000) as u32 - 200;
    // pubkey of "taker_passphrase" passphrase and qUX9FGHubczidVjWPCUWuwCUJWpkAtGCgf address
    let taker_pub = hex::decode("022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1a").unwrap();
    // pubkey of "cMhHM3PMpMrChygR4bLF7QsTdenhWpFrrmf2UezBG3eeFsz41rtL" passphrase
    let maker_pub = hex::decode("03693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9").unwrap();
    let secret = &[1; 32];
    let secret_hash = &*dhash160(secret);
    let amount = BigDecimal::from_str("0.2").unwrap();
    let payment = maker_coin
        .send_maker_payment(timelock, &taker_pub, secret_hash, amount.clone())
        .wait()
        .unwrap();
    let tx = match payment {
        TransactionEnum::UtxoTx(tx) => tx,
        _ => panic!("Expected UtxoTx"),
    };

    let payment_tx_hash: H256Json = tx.hash().reversed().into();
    log!("Maker payment: "[payment_tx_hash]);
    let tx_hex = serialize(&tx);

    let confirmations = 1;
    let requires_nota = false;
    let wait_until = (now_ms() / 1000) + 320; // timeout if test takes more than 320 seconds to run
    let check_every = 1;
    unwrap!(taker_coin
        .wait_for_confirmations(&tx_hex, confirmations, requires_nota, wait_until, check_every)
        .wait());

    unwrap!(taker_coin
        .validate_maker_payment(&tx_hex, timelock, &maker_pub, secret_hash, amount.clone())
        .wait());

    let spend = unwrap!(taker_coin
        .send_taker_spends_maker_payment(&tx_hex, timelock, &maker_pub, secret)
        .wait());
    let spend_tx = match spend {
        TransactionEnum::UtxoTx(tx) => tx,
        _ => panic!("Expected UtxoTx"),
    };

    let spend_tx_hash: H256Json = spend_tx.hash().reversed().into();
    log!("Taker spends tx: "[spend_tx_hash]);
    let spend_tx_hex = serialize(&spend_tx);
    let wait_until = (now_ms() / 1000) + 240; // timeout if test takes more than 240 seconds to run
    unwrap!(taker_coin
        .wait_for_confirmations(&spend_tx_hex, confirmations, requires_nota, wait_until, check_every)
        .wait());

    let bob_new_balance = taker_coin.my_balance().wait().unwrap();
    assert_eq!(bob_balance + amount, bob_new_balance);
}

#[test]
#[ignore]
fn test_maker_spends_taker_payment() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, maker_coin) = qrc20_coin_for_test(&priv_key);

    // priv_key of qUX9FGHubczidVjWPCUWuwCUJWpkAtGCgf
    let priv_key = [
        24, 181, 194, 193, 18, 152, 142, 168, 71, 73, 70, 244, 9, 101, 92, 168, 243, 61, 132, 48, 25, 39, 103, 92, 29,
        17, 11, 29, 113, 235, 48, 70,
    ];
    let (_ctx, taker_coin) = qrc20_coin_for_test(&priv_key);

    let maker_balance = maker_coin.my_balance().wait().unwrap();

    let timelock = (now_ms() / 1000) as u32 - 200;
    // pubkey of "taker_passphrase" passphrase and qUX9FGHubczidVjWPCUWuwCUJWpkAtGCgf address
    let taker_pub = hex::decode("022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1a").unwrap();
    // pubkey of "cMhHM3PMpMrChygR4bLF7QsTdenhWpFrrmf2UezBG3eeFsz41rtL" passphrase
    let maker_pub = hex::decode("03693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9").unwrap();
    let secret = &[1; 32];
    let secret_hash = &*dhash160(secret);
    let amount = BigDecimal::from_str("0.2").unwrap();
    let payment = taker_coin
        .send_taker_payment(timelock, &maker_pub, secret_hash, amount.clone())
        .wait()
        .unwrap();
    let tx = match payment {
        TransactionEnum::UtxoTx(tx) => tx,
        _ => panic!("Expected UtxoTx"),
    };

    let payment_tx_hash: H256Json = tx.hash().reversed().into();
    log!("Maker payment: "[payment_tx_hash]);
    let tx_hex = serialize(&tx);

    let confirmations = 1;
    let requires_nota = false;
    let wait_until = (now_ms() / 1000) + 320; // timeout if test takes more than 320 seconds to run
    let check_every = 1;
    unwrap!(maker_coin
        .wait_for_confirmations(&tx_hex, confirmations, requires_nota, wait_until, check_every)
        .wait());

    unwrap!(maker_coin
        .validate_taker_payment(&tx_hex, timelock, &taker_pub, secret_hash, amount.clone())
        .wait());

    let spend = unwrap!(maker_coin
        .send_maker_spends_taker_payment(&tx_hex, timelock, &taker_pub, secret)
        .wait());
    let spend_tx = match spend {
        TransactionEnum::UtxoTx(tx) => tx,
        _ => panic!("Expected UtxoTx"),
    };

    let spend_tx_hash: H256Json = spend_tx.hash().reversed().into();
    log!("Taker spends tx: "[spend_tx_hash]);
    let spend_tx_hex = serialize(&spend_tx);
    let wait_until = (now_ms() / 1000) + 240; // timeout if test takes more than 240 seconds to run
    unwrap!(maker_coin
        .wait_for_confirmations(&spend_tx_hex, confirmations, requires_nota, wait_until, check_every)
        .wait());

    let maker_new_balance = maker_coin.my_balance().wait().unwrap();
    assert_eq!(maker_balance + amount, maker_new_balance);
}

#[test]
#[ignore]
fn test_maker_refunds_payment() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    let expected_balance = unwrap!(coin.my_balance().wait());

    let timelock = (now_ms() / 1000) as u32 - 200;
    // pubkey of "taker_passphrase" passphrase and qUX9FGHubczidVjWPCUWuwCUJWpkAtGCgf address
    let taker_pub = hex::decode("022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1a").unwrap();
    let secret_hash = &[1; 20];
    let amount = BigDecimal::from_str("0.2").unwrap();
    let payment = coin
        .send_maker_payment(timelock, &taker_pub, secret_hash, amount.clone())
        .wait()
        .unwrap();
    let tx = match payment {
        TransactionEnum::UtxoTx(tx) => tx,
        _ => panic!("Expected UtxoTx"),
    };

    let payment_tx_hash: H256Json = tx.hash().reversed().into();
    log!("Maker payment: "[payment_tx_hash]);
    let tx_hex = serialize(&tx);

    let confirmations = 1;
    let requires_nota = false;
    let wait_until = (now_ms() / 1000) + 320; // timeout if test takes more than 320 seconds to run
    let check_every = 1;
    unwrap!(coin
        .wait_for_confirmations(&tx_hex, confirmations, requires_nota, wait_until, check_every)
        .wait());

    let balance_after_payment = unwrap!(coin.my_balance().wait());
    assert_eq!(expected_balance.clone() - amount, balance_after_payment);

    let refund = unwrap!(coin
        .send_maker_refunds_payment(&tx_hex, timelock, &taker_pub, secret_hash)
        .wait());
    let refund_tx = match refund {
        TransactionEnum::UtxoTx(tx) => tx,
        _ => panic!("Expected UtxoTx"),
    };

    let refund_tx_hash: H256Json = refund_tx.hash().reversed().into();
    log!("Taker spends tx: "[refund_tx_hash]);
    let refund_tx_hex = serialize(&refund_tx);
    let wait_until = (now_ms() / 1000) + 240; // timeout if test takes more than 240 seconds to run
    unwrap!(coin
        .wait_for_confirmations(&refund_tx_hex, confirmations, requires_nota, wait_until, check_every)
        .wait());

    let balance_after_refund = unwrap!(coin.my_balance().wait());
    assert_eq!(expected_balance, balance_after_refund);
}

#[test]
#[ignore]
fn test_taker_refunds_payment() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    let expected_balance = unwrap!(coin.my_balance().wait());

    let timelock = (now_ms() / 1000) as u32 - 200;
    // pubkey of "taker_passphrase" passphrase and qUX9FGHubczidVjWPCUWuwCUJWpkAtGCgf address
    let maker_pub = hex::decode("022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1a").unwrap();
    let secret_hash = &[1; 20];
    let amount = BigDecimal::from_str("0.2").unwrap();
    let payment = coin
        .send_taker_payment(timelock, &maker_pub, secret_hash, amount.clone())
        .wait()
        .unwrap();
    let tx = match payment {
        TransactionEnum::UtxoTx(tx) => tx,
        _ => panic!("Expected UtxoTx"),
    };

    let payment_tx_hash: H256Json = tx.hash().reversed().into();
    log!("Maker payment: "[payment_tx_hash]);
    let tx_hex = serialize(&tx);

    let confirmations = 1;
    let requires_nota = false;
    let wait_until = (now_ms() / 1000) + 320; // timeout if test takes more than 320 seconds to run
    let check_every = 1;
    unwrap!(coin
        .wait_for_confirmations(&tx_hex, confirmations, requires_nota, wait_until, check_every)
        .wait());

    let balance_after_payment = unwrap!(coin.my_balance().wait());
    assert_eq!(expected_balance.clone() - amount, balance_after_payment);

    let refund = unwrap!(coin
        .send_taker_refunds_payment(&tx_hex, timelock, &maker_pub, secret_hash)
        .wait());
    let refund_tx = match refund {
        TransactionEnum::UtxoTx(tx) => tx,
        _ => panic!("Expected UtxoTx"),
    };

    let refund_tx_hash: H256Json = refund_tx.hash().reversed().into();
    log!("Taker spends tx: "[refund_tx_hash]);
    let refund_tx_hex = serialize(&refund_tx);
    let wait_until = (now_ms() / 1000) + 240; // timeout if test takes more than 240 seconds to run
    unwrap!(coin
        .wait_for_confirmations(&refund_tx_hex, confirmations, requires_nota, wait_until, check_every)
        .wait());

    let balance_after_refund = unwrap!(coin.my_balance().wait());
    assert_eq!(expected_balance, balance_after_refund);
}

#[test]
fn test_check_if_my_payment_sent() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    let time_lock = 1601367157;
    // pubkey of "cMhHM3PMpMrChygR4bLF7QsTdenhWpFrrmf2UezBG3eeFsz41rtL" passphrase
    let maker_pub = hex::decode("03693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9").unwrap();
    let secret_hash = &[1; 20];
    // search from b22ee034e860d89af6e76e54bb7f8efb69d833a8670e61c60e5dfdfaa27db371 transaction
    let search_from_block = 686125;

    // tx_hash: 016a59dd2b181b3906b0f0333d5c7561dacb332dc99ac39679a591e523f2c49a
    let expected_tx = TransactionEnum::UtxoTx("010000000194448324c14fc6b78c7a52c59debe3240fc392019dbd6f1457422e3308ce1e75010000006b483045022100800a4956a30a36708536d98e8ea55a3d0983b963af6c924f60241616e2ff056d0220239e622f8ec8f1a0f5ef0fc93ff094a8e6b5aab964a62bed680b17bf6a848aac012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffff020000000000000000e35403a0860101284cc49b415b2a0c692f2ec8ebab181a79e31b7baab30fef0902e57f901c47a342643eeafa6b510000000000000000000000000000000000000000000000000000000001312d00000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc44000000000000000000000000783cf0be521101942da509846ea476e683aad8320101010101010101010101010101010101010101000000000000000000000000000000000000000000000000000000000000000000000000000000005f72ec7514ba8b71f3544b93e2f681f996da519a98ace0107ac201319302000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88ac40ed725f".into());
    let tx = unwrap!(coin
        .check_if_my_payment_sent(time_lock, &maker_pub, secret_hash, search_from_block)
        .wait());
    assert_eq!(tx, Some(expected_tx));

    let time_lock_dif = 1601367156;
    let tx = unwrap!(coin
        .check_if_my_payment_sent(time_lock_dif, &maker_pub, secret_hash, search_from_block)
        .wait());
    assert_eq!(tx, None);
}

#[test]
fn test_send_taker_fee() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    let fee_addr_pub_key = hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06").unwrap();
    let amount = BigDecimal::from_str("0.01").unwrap();
    let tx = unwrap!(coin.send_taker_fee(&fee_addr_pub_key, amount.clone()).wait());
    let tx_hash: H256Json = match tx {
        TransactionEnum::UtxoTx(ref tx) => tx.hash().reversed().into(),
        _ => panic!("Expected UtxoTx"),
    };
    log!("Fee tx "[tx_hash]);

    let result = coin.validate_fee(&tx, &fee_addr_pub_key, &amount).wait();
    assert_eq!(result, Ok(()));
}

#[test]
fn test_validate_fee() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    // QRC20 transfer tx "f97d3a43dbea0993f1b7a6a299377d4ee164c84935a1eb7d835f70c9429e6a1d"
    let tx = TransactionEnum::UtxoTx("010000000160fd74b5714172f285db2b36f0b391cd6883e7291441631c8b18f165b0a4635d020000006a47304402205d409e141111adbc4f185ae856997730de935ac30a0d2b1ccb5a6c4903db8171022024fc59bbcfdbba283556d7eeee4832167301dc8e8ad9739b7865f67b9676b226012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffff020000000000000000625403a08601012844a9059cbb000000000000000000000000ca1e04745e8ca0c60d8c5881531d51bec470743f00000000000000000000000000000000000000000000000000000000000f424014d362e096e873eb7907e205fadc6175c6fec7bc44c200ada205000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88acfe967d5f".into());

    let fee_addr = hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc06").unwrap();
    let amount = BigDecimal::from_str("0.01").unwrap();

    let result = coin.validate_fee(&tx, &fee_addr, &amount).wait();
    assert_eq!(result, Ok(()));

    let fee_addr_dif = hex::decode("03bc2c7ba671bae4a6fc835244c9762b41647b9827d4780a89a949b984a8ddcc05").unwrap();
    let err = coin
        .validate_fee(&tx, &fee_addr_dif, &amount)
        .wait()
        .err()
        .expect("Expected an error");
    log!("error: "[err]);
    assert!(err.contains("QRC20 Fee tx was sent to wrong address"));

    let amount_dif = BigDecimal::from_str("0.02").unwrap();
    let err = coin
        .validate_fee(&tx, &fee_addr, &amount_dif)
        .wait()
        .err()
        .expect("Expected an error");
    log!("error: "[err]);
    assert!(err.contains("QRC20 Fee tx value 1000000 is less than expected 2000000"));

    // QTUM tx "8a51f0ffd45f34974de50f07c5bf2f0949da4e88433f8f75191953a442cf9310"
    let tx = TransactionEnum::UtxoTx("020000000113640281c9332caeddd02a8dd0d784809e1ad87bda3c972d89d5ae41f5494b85010000006a47304402207c5c904a93310b8672f4ecdbab356b65dd869a426e92f1064a567be7ccfc61ff02203e4173b9467127f7de4682513a21efb5980e66dbed4da91dff46534b8e77c7ef012102baefe72b3591de2070c0da3853226b00f082d72daa417688b61cb18c1d543d1afeffffff020001b2c4000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88acbc4dd20c2f0000001976a9144208fa7be80dcf972f767194ad365950495064a488ac76e70800".into());
    let err = coin
        .validate_fee(&tx, &fee_addr, &amount)
        .wait()
        .err()
        .expect("Expected an error");
    log!("error: "[err]);
    assert!(err.contains("Expected 'transfer' contract call"));
}

#[test]
#[ignore]
fn test_search_for_swap_tx_spend() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    let other_pub = &[0]; //ignored
    let search_from_block = 693000;

    // taker spent maker payment - d3f5dab4d54c14b3d7ed8c7f5c8cc7f47ccf45ce589fdc7cd5140a3c1c3df6e1
    let expected = Ok(Some(FoundSwapTxSpend::Spent(TransactionEnum::UtxoTx("01000000033f56ecafafc8602fde083ba868d1192d6649b8433e42e1a2d79ba007ea4f7abb010000006b48304502210093404e90e40d22730013035d31c404c875646dcf2fad9aa298348558b6d65ba60220297d045eac5617c1a3eddb71d4bca9772841afa3c4c9d6c68d8d2d42ee6de3950121022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1affffffff9cac7fe90d597922a1d92e05306c2215628e7ea6d5b855bfb4289c2944f4c73a030000006b483045022100b987da58c2c0c40ce5b6ef2a59e8124ed4ef7a8b3e60c7fb631139280019bc93022069649bcde6fe4dd5df9462a1fcae40598488d6af8c324cd083f5c08afd9568be0121022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1affffffff70b9870f2b0c65d220a839acecebf80f5b44c3ca4c982fa2fdc5552c037f5610010000006a473044022071b34dd3ebb72d29ca24f3fa0fc96571c815668d3b185dd45cc46a7222b6843f02206c39c030e618d411d4124f7b3e7ca1dd5436775bd8083a85712d123d933a51300121022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1affffffff020000000000000000c35403a0860101284ca402ed292b806a1835a1b514ad643f2acdb5c8db6b6a9714accff3275ea0d79a3f23be8fd00000000000000000000000000000000000000000000000000000000001312d000101010101010101010101010101010101010101010101010101010101010101000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc440000000000000000000000009e032d4b0090a11dc40fe6c47601499a35d55fbb14ba8b71f3544b93e2f681f996da519a98ace0107ac2c02288d4010000001976a914783cf0be521101942da509846ea476e683aad83288ac0f047f5f".into()))));
    // maker sent payment - c8112c75be039100c30d71293571f081e189540818ef8e2903ff75d2d556b446
    let tx_hex = hex::decode("0100000001e6b256dd9d390be2ccd8eddaf67a40d1994a983845fb223c102ce8e58eca2b48010000006b4830450221008e8e793ad00ed1d45f4546b9e7b9dc8305d61c384e126c24e7945bd0056df099022077f033cf16535f0d3627548196cd3868d904ca6ccac9d80d56f1f70df6589915012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffff020000000000000000e35403a0860101284cc49b415b2a806a1835a1b514ad643f2acdb5c8db6b6a9714accff3275ea0d79a3f23be8fd00000000000000000000000000000000000000000000000000000000001312d00000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc44000000000000000000000000783cf0be521101942da509846ea476e683aad8324b6b2e5444c2639cc0fb7bcea5afba3f3cdce239000000000000000000000000000000000000000000000000000000000000000000000000000000005f7f02c014ba8b71f3544b93e2f681f996da519a98ace0107ac27046a001000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88ac8b037f5f").unwrap();
    let timelock = 1602159296;
    let secret = &[1; 32];
    let secret_hash = &*dhash160(secret);
    let actual = coin.search_for_swap_tx_spend_my(timelock, other_pub, secret_hash, &tx_hex, search_from_block);
    assert_eq!(actual, expected);

    // maker refunded payment his - df41079d58a13320590476e648d37007459366b0fbfce8d0b72fae502e39cc01
    let expected = Ok(Some(FoundSwapTxSpend::Refunded(TransactionEnum::UtxoTx("010000000191999480813e0284212d08a16b32146e7d32315feaf6489cd3aa696b54e5ce71010000006a4730440220282a32f05a4802caee065ee8d2b08a9b366c26b16d9afb068b3259aa54107b0e0220039c7697620e91096d566ddb6056ad347c395584114f790a2a727db86789c576012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffff020000000000000000c35403a0860101284ca446fc0294796332096ae329d7aa84c52f036bbeb9dd4b872c8d2021ccb8775e23f56a422e0000000000000000000000000000000000000000000000000000000001312d000101010101010101010101010101010101010101000000000000000000000000000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc44000000000000000000000000783cf0be521101942da509846ea476e683aad83214ba8b71f3544b93e2f681f996da519a98ace0107ac2d012ac00000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88ac97067f5f".into()))));
    // maker sent payment - 71cee5546b69aad39c48f6ea5f31327d6e14326ba1082d2184023e8180949991
    let tx_hex = hex::decode("0100000001422dd62a9405fbda1f0e01ed45917cd908a68258a5f5530a1f53c4cd173bc82b010000006a47304402201c2c3b789a651143a657217b5b459027b68a78545a5036e03f90bacbc4cfd8b1022055200a3da6b208dc8763471a87d869d6b045f1dd38f855b0fda0b526f23f88ea012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffff020000000000000000e35403a0860101284cc49b415b2a796332096ae329d7aa84c52f036bbeb9dd4b872c8d2021ccb8775e23f56a422e0000000000000000000000000000000000000000000000000000000001312d00000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc44000000000000000000000000783cf0be521101942da509846ea476e683aad8320101010101010101010101010101010101010101000000000000000000000000000000000000000000000000000000000000000000000000000000005f7f059f14ba8b71f3544b93e2f681f996da519a98ace0107ac2b81fe900000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88ac6a067f5f").unwrap();
    let timelock = 1602160031;
    let secret_hash = &[1; 20];
    let actual = coin.search_for_swap_tx_spend_my(timelock, other_pub, secret_hash, &tx_hex, search_from_block);
    assert_eq!(actual, expected);

    // maker payment hasn't been spent or refunded yet
    let expected = Ok(None);
    // maker sent payment 9fae1771bb542f9860d845091109a6a951f95fc277faebe3ec6ab3e8df9e58b6
    let tx_hex = hex::decode("010000000101cc392e50ae2fb7d0e8fcfbb06693450770d348e67604592033a1589d0741df010000006b483045022100935cf73d2b01a694f4383eb844d5e93e041496b13e6bdf1f7a8f3bb8dd83b50002204952184584460cc1ab979895ec4850ea9e26a7308d231376fc21c133c7eeaf08012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffff020000000000000000e35403a0860101284cc49b415b2a4357ff815e6657ea5b4cf992475e29940b3a4cda9b589d5e5061bb06c1f5bf5a0000000000000000000000000000000000000000000000000000000001312d00000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc44000000000000000000000000783cf0be521101942da509846ea476e683aad8320101010101010101010101010101010101010101000000000000000000000000000000000000000000000000000000000000000000000000000000005f7f066014ba8b71f3544b93e2f681f996da519a98ace0107ac2e8056f00000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88ac2b077f5f").unwrap();
    let timelock = 1602160224;
    let secret_hash = &[1; 20];
    let actual = coin.search_for_swap_tx_spend_my(timelock, other_pub, secret_hash, &tx_hex, search_from_block);
    assert_eq!(actual, expected);
}

#[test]
#[ignore]
fn test_wait_for_tx_spend() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, maker_coin) = qrc20_coin_for_test(&priv_key);

    // priv_key of qUX9FGHubczidVjWPCUWuwCUJWpkAtGCgf
    let priv_key = [
        24, 181, 194, 193, 18, 152, 142, 168, 71, 73, 70, 244, 9, 101, 92, 168, 243, 61, 132, 48, 25, 39, 103, 92, 29,
        17, 11, 29, 113, 235, 48, 70,
    ];
    let (_ctx, taker_coin) = qrc20_coin_for_test(&priv_key);

    let from_block = maker_coin.current_block().wait().unwrap();

    let timelock = (now_ms() / 1000) as u32 - 200;
    // pubkey of "taker_passphrase" passphrase and qUX9FGHubczidVjWPCUWuwCUJWpkAtGCgf address
    let taker_pub = hex::decode("022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1a").unwrap();
    // pubkey of "cMhHM3PMpMrChygR4bLF7QsTdenhWpFrrmf2UezBG3eeFsz41rtL" passphrase
    let maker_pub = hex::decode("03693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9").unwrap();
    let secret = &[1; 32];
    let secret_hash = &*dhash160(secret);
    let amount = BigDecimal::from_str("0.2").unwrap();
    let payment = maker_coin
        .send_maker_payment(timelock, &taker_pub, secret_hash, amount.clone())
        .wait()
        .unwrap();
    let tx = match payment {
        TransactionEnum::UtxoTx(tx) => tx,
        _ => panic!("Expected UtxoTx"),
    };

    let payment_tx_hash: H256Json = tx.hash().reversed().into();
    log!("Maker payment: "[payment_tx_hash]);
    let tx_hex = serialize(&tx);

    let confirmations = 1;
    let requires_nota = false;
    let wait_until = (now_ms() / 1000) + 320; // timeout if test takes more than 320 seconds to run
    let check_every = 1;
    unwrap!(taker_coin
        .wait_for_confirmations(&tx_hex, confirmations, requires_nota, wait_until, check_every)
        .wait());

    unwrap!(taker_coin
        .validate_maker_payment(&tx_hex, timelock, &maker_pub, secret_hash, amount.clone())
        .wait());

    // first try to check if the wait_for_tx_spend() returns an error correctly
    let wait_until = (now_ms() / 1000) + 11;
    let err = maker_coin
        .wait_for_tx_spend(&tx_hex, wait_until, from_block)
        .wait()
        .expect_err("Expected 'Waited too long' error");
    log!("error: "[err]);
    assert!(err.contains("Waited too long"));

    // also spends the maker payment and try to check if the wait_for_tx_spend() returns the correct tx
    let spend_tx: Arc<Mutex<Option<UtxoTx>>> = Arc::new(Mutex::new(None));

    let tx_hex_c = tx_hex.clone();
    let spend_tx_c = spend_tx.clone();
    let fut = async move {
        Timer::sleep(11.).await;

        let spend = unwrap!(
            taker_coin
                .send_taker_spends_maker_payment(&tx_hex_c, timelock, &maker_pub, secret)
                .compat()
                .await
        );
        let mut lock = spend_tx_c.lock().unwrap();
        match spend {
            TransactionEnum::UtxoTx(tx) => *lock = Some(tx),
            _ => panic!("Expected UtxoTx"),
        }
    };

    spawn(fut);

    let wait_until = (now_ms() / 1000) + 320;
    let found = unwrap!(maker_coin.wait_for_tx_spend(&tx_hex, wait_until, from_block).wait());

    let spend_tx = match spend_tx.lock().unwrap().as_ref() {
        Some(tx) => tx.clone(),
        None => panic!(),
    };

    match found {
        TransactionEnum::UtxoTx(tx) => assert_eq!(tx, spend_tx),
        _ => panic!("Unexpected Transaction type"),
    }
}

#[test]
fn test_wait_for_tx_spend_malicious() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    // f94d79f89e9ec785db40bb8bb8dca9bc01b7761429618d4c843bbebbc31836b7
    // the transaction has two outputs:
    //   1 - with an invalid secret (this case should be processed correctly)
    //   2 - correct spend tx
    let expected_tx: UtxoTx = "01000000022bc8299981ec0cea664cdf9df4f8306396a02e2067d6ac2d3770b34646d2bc2a010000006b483045022100eb13ef2d99ac1cd9984045c2365654b115dd8a7815b7fbf8e2a257f0b93d1592022060d648e73118c843e97f75fafc94e5ff6da70ec8ba36ae255f8c96e2626af6260121022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1affffffffd92a0a10ac6d144b36033916f67ae79889f40f35096629a5cd87be1a08f40ee7010000006b48304502210080cdad5c4770dfbeb760e215494c63cc30da843b8505e75e7bf9e8dad18568000220234c0b11c41bfbcdd50046c69059976aedabe17657fe43d809af71e9635678e20121022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1affffffff030000000000000000c35403a0860101284ca402ed292b8620ad3b72361a5aeba5dffd333fb64750089d935a1ec974d6a91ef4f24ff6ba0000000000000000000000000000000000000000000000000000000001312d000202020202020202020202020202020202020202020202020202020202020202000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc440000000000000000000000009e032d4b0090a11dc40fe6c47601499a35d55fbb14ba8b71f3544b93e2f681f996da519a98ace0107ac20000000000000000c35403a0860101284ca402ed292b8620ad3b72361a5aeba5dffd333fb64750089d935a1ec974d6a91ef4f24ff6ba0000000000000000000000000000000000000000000000000000000001312d000101010101010101010101010101010101010101010101010101010101010101000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc440000000000000000000000009e032d4b0090a11dc40fe6c47601499a35d55fbb14ba8b71f3544b93e2f681f996da519a98ace0107ac2b8ea82d3010000001976a914783cf0be521101942da509846ea476e683aad83288ac735d855f".into();

    // 15fd8f71be6b2678b021e1300e67fa99574a2ad877df08276ac275728ac12304
    let payment_tx = hex::decode("01000000016601daa208531d20532c460d0c86b74a275f4a126bbffcf4eafdf33835af2859010000006a47304402205825657548bc1b5acf3f4bb2f89635a02b04f3228cd08126e63c5834888e7ac402207ca05fa0a629a31908a97a508e15076e925f8e621b155312b7526a6666b06a76012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffff020000000000000000e35403a0860101284cc49b415b2a8620ad3b72361a5aeba5dffd333fb64750089d935a1ec974d6a91ef4f24ff6ba0000000000000000000000000000000000000000000000000000000001312d00000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc44000000000000000000000000783cf0be521101942da509846ea476e683aad8324b6b2e5444c2639cc0fb7bcea5afba3f3cdce239000000000000000000000000000000000000000000000000000000000000000000000000000000005f855c7614ba8b71f3544b93e2f681f996da519a98ace0107ac2203de400000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88ac415d855f").unwrap();
    let wait_until = (now_ms() / 1000) + 1;
    let from_block = 696245;
    let found = unwrap!(coin.wait_for_tx_spend(&payment_tx, wait_until, from_block).wait());

    let spend_tx = match found {
        TransactionEnum::UtxoTx(tx) => tx,
        _ => panic!("Unexpected Transaction type"),
    };

    assert_eq!(spend_tx, expected_tx);
}

#[test]
fn test_extract_secret() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    let expected_secret = &[1; 32];
    let secret_hash = &*dhash160(expected_secret);

    // taker spent maker payment - d3f5dab4d54c14b3d7ed8c7f5c8cc7f47ccf45ce589fdc7cd5140a3c1c3df6e1
    let tx_hex = hex::decode("01000000033f56ecafafc8602fde083ba868d1192d6649b8433e42e1a2d79ba007ea4f7abb010000006b48304502210093404e90e40d22730013035d31c404c875646dcf2fad9aa298348558b6d65ba60220297d045eac5617c1a3eddb71d4bca9772841afa3c4c9d6c68d8d2d42ee6de3950121022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1affffffff9cac7fe90d597922a1d92e05306c2215628e7ea6d5b855bfb4289c2944f4c73a030000006b483045022100b987da58c2c0c40ce5b6ef2a59e8124ed4ef7a8b3e60c7fb631139280019bc93022069649bcde6fe4dd5df9462a1fcae40598488d6af8c324cd083f5c08afd9568be0121022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1affffffff70b9870f2b0c65d220a839acecebf80f5b44c3ca4c982fa2fdc5552c037f5610010000006a473044022071b34dd3ebb72d29ca24f3fa0fc96571c815668d3b185dd45cc46a7222b6843f02206c39c030e618d411d4124f7b3e7ca1dd5436775bd8083a85712d123d933a51300121022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1affffffff020000000000000000c35403a0860101284ca402ed292b806a1835a1b514ad643f2acdb5c8db6b6a9714accff3275ea0d79a3f23be8fd00000000000000000000000000000000000000000000000000000000001312d000101010101010101010101010101010101010101010101010101010101010101000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc440000000000000000000000009e032d4b0090a11dc40fe6c47601499a35d55fbb14ba8b71f3544b93e2f681f996da519a98ace0107ac2c02288d4010000001976a914783cf0be521101942da509846ea476e683aad83288ac0f047f5f").unwrap();
    let secret = unwrap!(coin.extract_secret(secret_hash, &tx_hex));

    assert_eq!(secret, expected_secret);
}

#[test]
fn test_extract_secret_malicious() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    // f94d79f89e9ec785db40bb8bb8dca9bc01b7761429618d4c843bbebbc31836b7
    // the transaction has two outputs:
    //   1 - with an invalid secret (this case should be processed correctly)
    //   2 - correct spend tx
    let spend_tx = hex::decode("01000000022bc8299981ec0cea664cdf9df4f8306396a02e2067d6ac2d3770b34646d2bc2a010000006b483045022100eb13ef2d99ac1cd9984045c2365654b115dd8a7815b7fbf8e2a257f0b93d1592022060d648e73118c843e97f75fafc94e5ff6da70ec8ba36ae255f8c96e2626af6260121022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1affffffffd92a0a10ac6d144b36033916f67ae79889f40f35096629a5cd87be1a08f40ee7010000006b48304502210080cdad5c4770dfbeb760e215494c63cc30da843b8505e75e7bf9e8dad18568000220234c0b11c41bfbcdd50046c69059976aedabe17657fe43d809af71e9635678e20121022b00078841f37b5d30a6a1defb82b3af4d4e2d24dd4204d41f0c9ce1e875de1affffffff030000000000000000c35403a0860101284ca402ed292b8620ad3b72361a5aeba5dffd333fb64750089d935a1ec974d6a91ef4f24ff6ba0000000000000000000000000000000000000000000000000000000001312d000202020202020202020202020202020202020202020202020202020202020202000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc440000000000000000000000009e032d4b0090a11dc40fe6c47601499a35d55fbb14ba8b71f3544b93e2f681f996da519a98ace0107ac20000000000000000c35403a0860101284ca402ed292b8620ad3b72361a5aeba5dffd333fb64750089d935a1ec974d6a91ef4f24ff6ba0000000000000000000000000000000000000000000000000000000001312d000101010101010101010101010101010101010101010101010101010101010101000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc440000000000000000000000009e032d4b0090a11dc40fe6c47601499a35d55fbb14ba8b71f3544b93e2f681f996da519a98ace0107ac2b8ea82d3010000001976a914783cf0be521101942da509846ea476e683aad83288ac735d855f").unwrap();
    let expected_secret = &[1; 32];
    let secret_hash = &*dhash160(expected_secret);
    let actual = coin.extract_secret(secret_hash, &spend_tx);
    assert_eq!(actual, Ok(expected_secret.to_vec()));
}

#[test]
fn test_generate_token_transfer_script_pubkey() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    let gas_limit = 2_500_000;
    let gas_price = 40;

    // sample QRC20 transfer from https://testnet.qtum.info/tx/51e9cec885d7eb26271f8b1434c000f6cf07aad47671268fc8d36cee9d48f6de
    // the script is a script_pubkey of one of the transaction output
    let expected_script: Script = "5403a02526012844a9059cbb0000000000000000000000000240b898276ad2cc0d2fe6f527e8e31104e7fde3000000000000000000000000000000000000000000000000000000003b9aca0014d362e096e873eb7907e205fadc6175c6fec7bc44c2".into();
    let expected = ContractCallOutput {
        value: 0,
        script_pubkey: expected_script.to_bytes(),
        gas_limit,
        gas_price,
    };

    let to_addr: UtxoAddress = "qHmJ3KA6ZAjR9wGjpFASn4gtUSeFAqdZgs".into();
    let to_addr = qrc20_addr_from_utxo_addr(to_addr);
    let amount: U256 = 1000000000.into();
    let actual = coin
        .transfer_output(to_addr.clone(), amount, gas_limit, gas_price)
        .unwrap();
    assert_eq!(expected, actual);

    assert!(coin
        .transfer_output(
            to_addr.clone(),
            amount,
            0, // gas_limit cannot be zero
            gas_price,
        )
        .is_err());

    assert!(coin
        .transfer_output(
            to_addr.clone(),
            amount,
            gas_limit,
            0, // gas_price cannot be zero
        )
        .is_err());
}

#[test]
fn test_transfer_details_by_hash() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);
    let tx_hash_bytes = hex::decode("85ede12ccc12fb1709c4d9e403e96c0c394b0916f2f6098d41d8dfa00013fcdb").unwrap();
    let tx_hash: H256Json = tx_hash_bytes.as_slice().into();
    let tx_hex:BytesJson = hex::decode("0100000001426d27fde82e12e1ce84e73ca41e2a30420f4c94aaa37b30d4c5b8b4f762c042040000006a473044022032665891693ee732571cefaa6d322ec5114c78259f2adbe03a0d7e6b65fbf40d022035c9319ca41e5423e09a8a613ac749a20b8f5ad6ba4ad6bb60e4a020b085d009012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffff050000000000000000625403a08601012844095ea7b30000000000000000000000001549128bbfb33b997949b4105b6a6371c998e212000000000000000000000000000000000000000000000000000000000000000014d362e096e873eb7907e205fadc6175c6fec7bc44c20000000000000000625403a08601012844095ea7b30000000000000000000000001549128bbfb33b997949b4105b6a6371c998e21200000000000000000000000000000000000000000000000000000000000927c014d362e096e873eb7907e205fadc6175c6fec7bc44c20000000000000000835403a0860101284c640c565ae300000000000000000000000000000000000000000000000000000000000493e0000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc440000000000000000000000000000000000000000000000000000000000000000141549128bbfb33b997949b4105b6a6371c998e212c20000000000000000835403a0860101284c640c565ae300000000000000000000000000000000000000000000000000000000000493e0000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc440000000000000000000000000000000000000000000000000000000000000001141549128bbfb33b997949b4105b6a6371c998e212c231754b04000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88acf7cd8b5f").unwrap().into();

    let details = unwrap!(block_on(coin.transfer_details_by_hash(tx_hash)));
    let mut it = details.into_iter().sorted_by(|(id_x, _), (id_y, _)| id_x.cmp(&id_y));

    let expected_fee_details = |total_gas_fee: &str| -> TxFeeDetails {
        let fee = Qrc20FeeDetails {
            coin: "QTUM".into(),
            miner_fee: BigDecimal::from_str("0.15806792").unwrap(),
            gas_limit: 100000,
            gas_price: 40,
            total_gas_fee: BigDecimal::from_str(total_gas_fee).unwrap(),
        };
        TxFeeDetails::Qrc20(fee)
    };

    // qKVvtDqpnFGDxsDzck5jmLwdnD2jRH6aM8 is UTXO representation of 1549128bbfb33b997949b4105b6a6371c998e212 contract address

    let (_id, actual) = it.next().unwrap();
    let expected = TransactionDetails {
        tx_hex: tx_hex.clone(),
        tx_hash: tx_hash_bytes.clone().into(),
        from: vec!["qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG".into()],
        to: vec!["qKVvtDqpnFGDxsDzck5jmLwdnD2jRH6aM8".into()],
        total_amount: BigDecimal::from_str("0.003").unwrap(),
        spent_by_me: BigDecimal::from_str("0.003").unwrap(),
        received_by_me: BigDecimal::zero(),
        my_balance_change: BigDecimal::from_str("-0.003").unwrap(),
        block_height: 699545,
        timestamp: 1602997840,
        fee_details: Some(expected_fee_details("0.00059074")),
        coin: "QRC20".into(),
        internal_id: hex::decode(
            "85ede12ccc12fb1709c4d9e403e96c0c394b0916f2f6098d41d8dfa00013fcdb00000000000000020000000000000000",
        )
        .unwrap()
        .into(),
    };
    assert_eq!(actual, expected);

    let (_id, actual) = it.next().unwrap();
    let expected = TransactionDetails {
        tx_hex: tx_hex.clone(),
        tx_hash: tx_hash_bytes.clone().into(),
        from: vec!["qKVvtDqpnFGDxsDzck5jmLwdnD2jRH6aM8".into()],
        to: vec!["qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG".into()],
        total_amount: BigDecimal::from_str("0.00295").unwrap(),
        spent_by_me: BigDecimal::zero(),
        received_by_me: BigDecimal::from_str("0.00295").unwrap(),
        my_balance_change: BigDecimal::from_str("0.00295").unwrap(),
        block_height: 699545,
        timestamp: 1602997840,
        fee_details: Some(expected_fee_details("0.00059074")),
        coin: "QRC20".into(),
        internal_id: hex::decode(
            "85ede12ccc12fb1709c4d9e403e96c0c394b0916f2f6098d41d8dfa00013fcdb00000000000000020000000000000001",
        )
        .unwrap()
        .into(),
    };
    assert_eq!(actual, expected);

    let (_id, actual) = it.next().unwrap();
    let expected = TransactionDetails {
        tx_hex: tx_hex.clone(),
        tx_hash: tx_hash_bytes.clone().into(),
        from: vec!["qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG".into()],
        to: vec!["qKVvtDqpnFGDxsDzck5jmLwdnD2jRH6aM8".into()],
        total_amount: BigDecimal::from_str("0.003").unwrap(),
        spent_by_me: BigDecimal::from_str("0.003").unwrap(),
        received_by_me: BigDecimal::zero(),
        my_balance_change: BigDecimal::from_str("-0.003").unwrap(),
        block_height: 699545,
        timestamp: 1602997840,
        fee_details: Some(expected_fee_details("0.00059118")),
        coin: "QRC20".into(),
        internal_id: hex::decode(
            "85ede12ccc12fb1709c4d9e403e96c0c394b0916f2f6098d41d8dfa00013fcdb00000000000000030000000000000000",
        )
        .unwrap()
        .into(),
    };
    assert_eq!(actual, expected);

    let (_id, actual) = it.next().unwrap();
    let expected = TransactionDetails {
        tx_hex: tx_hex.clone(),
        tx_hash: tx_hash_bytes.clone().into(),
        from: vec!["qKVvtDqpnFGDxsDzck5jmLwdnD2jRH6aM8".into()],
        to: vec!["qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG".into()],
        total_amount: BigDecimal::from_str("0.00295").unwrap(),
        spent_by_me: BigDecimal::zero(),
        received_by_me: BigDecimal::from_str("0.00295").unwrap(),
        my_balance_change: BigDecimal::from_str("0.00295").unwrap(),
        block_height: 699545,
        timestamp: 1602997840,
        fee_details: Some(expected_fee_details("0.00059118")),
        coin: "QRC20".into(),
        internal_id: hex::decode(
            "85ede12ccc12fb1709c4d9e403e96c0c394b0916f2f6098d41d8dfa00013fcdb00000000000000030000000000000001",
        )
        .unwrap()
        .into(),
    };
    assert_eq!(actual, expected);

    let (_id, actual) = it.next().unwrap();
    let expected = TransactionDetails {
        tx_hex: tx_hex.clone(),
        tx_hash: tx_hash_bytes.clone().into(),
        from: vec!["qKVvtDqpnFGDxsDzck5jmLwdnD2jRH6aM8".into()],
        to: vec!["qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG".into()],
        total_amount: BigDecimal::from_str("0.00005000").unwrap(),
        spent_by_me: BigDecimal::zero(),
        received_by_me: BigDecimal::from_str("0.00005000").unwrap(),
        my_balance_change: BigDecimal::from_str("0.00005000").unwrap(),
        block_height: 699545,
        timestamp: 1602997840,
        fee_details: Some(expected_fee_details("0.00059118")),
        coin: "QRC20".into(),
        internal_id: hex::decode(
            "85ede12ccc12fb1709c4d9e403e96c0c394b0916f2f6098d41d8dfa00013fcdb00000000000000030000000000000002",
        )
        .unwrap()
        .into(),
    };
    assert_eq!(actual, expected);
    assert!(it.next().is_none());
}

#[test]
fn test_get_trade_fee() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);
    // check if the coin's tx fee is required
    let expected_tx_fee = 1000;
    check_tx_fee(&coin, ActualTxFee::Fixed(expected_tx_fee));

    let actual_trade_fee = coin.get_trade_fee().wait().unwrap();
    let expected_trade_fee_amount = big_decimal_from_sat(
        (QRC20_SWAP_GAS_REQUIRED * QRC20_GAS_PRICE_DEFAULT + expected_tx_fee) as i64,
        coin.utxo.decimals,
    );
    let expected = TradeFee {
        coin: "QTUM".into(),
        amount: expected_trade_fee_amount.into(),
    };
    assert_eq!(actual_trade_fee, expected);
}

#[test]
fn test_qrc20_coin_from_conf_without_decimals() {
    // priv_key of qXxsj5RtciAby9T7m98AgAATL4zTi4UwDG
    let priv_key = [
        3, 98, 177, 3, 108, 39, 234, 144, 131, 178, 103, 103, 127, 80, 230, 166, 53, 68, 147, 215, 42, 216, 144, 72,
        172, 110, 180, 13, 123, 179, 10, 49,
    ];
    let conf = json!({
        "coin":"QRC20",
        "required_confirmations":0,
        "pubtype":120,
        "p2shtype":50,
        "wiftype":128,
        "segwit":true,
        "mm2":1,
        "mature_confirmations":500,
    });
    let req = json!({
        "method": "electrum",
        "servers": [{"url":"95.217.83.126:10001"}],
        "swap_contract_address": "0xba8b71f3544b93e2f681f996da519a98ace0107a",
    });
    // 0459c999c3edf05e73c83f3fbae9f0f020919f91 has 12 decimals instead of standard 8
    let contract_address = "0x0459c999c3edf05e73c83f3fbae9f0f020919f91".into();
    let ctx = MmCtxBuilder::new().into_mm_arc();
    let coin = unwrap!(block_on(qrc20_coin_from_conf_and_request(
        &ctx,
        "QRC20",
        "QTUM",
        &conf,
        &req,
        &priv_key,
        contract_address
    )));

    assert_eq!(coin.utxo.decimals, 12);
    assert_eq!(coin.decimals(), 12);
}

/// Test [`Qrc20Coin::validate_maker_payment`] and [`Qrc20Coin::erc20_payment_details_from_tx`]
/// with malicious maker payment.
///
/// Maker could send a payment to another malicious swap contract with the same `erc20Payment` function.
/// He could pass the correct arguments and this malicious swap contract could emit a `Transfer` event with the correct topics.
///
/// Example of malicious `erc20Payment` function:
///
/// ```solidity
/// function erc20Payment(
///     bytes32 _id,
///     uint256 _amount,
///     address _tokenAddress,
///     address _receiver,
///     bytes20 _secretHash,
///     uint64 _lockTime
/// ) external payable {
///     require(_receiver != address(0) && _amount > 0 && payments[_id].state == PaymentState.Uninitialized);
///     bytes20 paymentHash = ripemd160(abi.encodePacked(
///         _receiver,
///         msg.sender,
///         _secretHash,
///         _tokenAddress,
///         _amount
///     ));
///     payments[_id] = Payment(
///         paymentHash,
///         _lockTime,
///         PaymentState.PaymentSent
///     );
///     // actual swap contract address 0xba8b71f3544b93e2f681f996da519a98ace0107a in Mixed-case address format
///     address swapContract = 0xbA8B71f3544b93E2f681f996da519A98aCE0107A;
///     IERC20 token = IERC20(_tokenAddress);
///     // transfer some little amounts from the sender to actual swap contract to emit a `Transfer` event with the correct topics
///     require(token.transferFrom(msg.sender, swapContract, 1000));
///     emit PaymentSent(_id);
/// }
/// ```
///
/// In the function above maker spent only 1000 amount, but value is 100000000 in the arguments.
/// Note maker initialized payment with the corresponding swap_id in
/// b61ef2f9911d075e80a3623444cce8fd948932f66c9148283860d46e9af4f2c8 tx.
#[test]
fn test_validate_maker_payment_malicious() {
    // priv_key of qUX9FGHubczidVjWPCUWuwCUJWpkAtGCgf
    let priv_key = [
        24, 181, 194, 193, 18, 152, 142, 168, 71, 73, 70, 244, 9, 101, 92, 168, 243, 61, 132, 48, 25, 39, 103, 92, 29,
        17, 11, 29, 113, 235, 48, 70,
    ];
    let (_ctx, coin) = qrc20_coin_for_test(&priv_key);

    // Malicious tx 81540dc6abe59cf1e301a97a7e1c9b66d5f475da916faa3f0ef7ea896c0b3e5a
    let payment_tx = hex::decode("01000000010144e2b8b5e6da0666faf1db95075653ef49e2acaa8924e1ec595f6b89a6f715050000006a4730440220415adec5e24148db8e9654a6beda4b1af8aded596ab1cd8667af32187853e8f5022007a91d44ee13046194aafc07ca46ec44f770e75b41187acaa4e38e17d4eccb5d012103693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9ffffffff030000000000000000625403a08601012844095ea7b300000000000000000000000085a4df739bbb2d247746bea611d5d365204725830000000000000000000000000000000000000000000000000000000005f5e10014d362e096e873eb7907e205fadc6175c6fec7bc44c20000000000000000e35403a0860101284cc49b415b2a0a1a8b4af2762154115ced87e2424b3cb940c0181cc3c850523702f1ec298fef0000000000000000000000000000000000000000000000000000000005f5e100000000000000000000000000d362e096e873eb7907e205fadc6175c6fec7bc44000000000000000000000000783cf0be521101942da509846ea476e683aad8324b6b2e5444c2639cc0fb7bcea5afba3f3cdce239000000000000000000000000000000000000000000000000000000000000000000000000000000005fa0fffb1485a4df739bbb2d247746bea611d5d36520472583c208535c01000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88acc700a15f").unwrap();
    let time_lock = 1604386811;
    let maker_pub = hex::decode("03693bff1b39e8b5a306810023c29b95397eb395530b106b1820ea235fd81d9ce9").unwrap();
    let secret = &[1; 32];
    let secret_hash = &*dhash160(secret);
    let amount = BigDecimal::from_str("1").unwrap();
    let error = coin
        .validate_maker_payment(&payment_tx, time_lock, &maker_pub, secret_hash, amount)
        .wait()
        .err()
        .expect("'erc20Payment' was called from another swap contract, expected an error");
    log!("error: "(error));
    assert!(error.contains("Unexpected amount 1000 in 'Transfer' event, expected 100000000"));
}
