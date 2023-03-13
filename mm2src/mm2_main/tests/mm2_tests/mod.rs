mod bch_and_slp_tests;
mod best_orders_tests;
mod eth_tests;
mod iris_swap;
mod lightning_tests;
mod lp_bot_tests;
mod mm2_tests_inner;
mod orderbook_sync_tests;
mod tendermint_ibc_asset_tests;
mod tendermint_tests;
mod z_coin_tests;

// dummy test helping IDE to recognize this as test module
#[test]
fn dummy() { assert!(true) }

#[test]
fn dump() {
    println!("BOB_PASSPHRASE_LINUX: {:?}", std::env::var("BOB_PASSPHRASE_LINUX"));
    println!("BOB_USERPASS_LINUX: {:?}", std::env::var("BOB_USERPASS_LINUX"));
    println!("ALICE_PASSPHRASE_LINUX: {:?}", std::env::var("ALICE_PASSPHRASE_LINUX"));
    println!("ALICE_USERPASS_LINUX: {:?}", std::env::var("ALICE_USERPASS_LINUX"));

    println!("BOB_PASSPHRASE_MAC: {:?}", std::env::var("BOB_PASSPHRASE_MAC"));
    println!("BOB_USERPASS_MAC: {:?}", std::env::var("BOB_USERPASS_MAC"));
    println!("ALICE_PASSPHRASE_MAC: {:?}", std::env::var("ALICE_PASSPHRASE_MAC"));
    println!("ALICE_USERPASS_MAC: {:?}", std::env::var("ALICE_USERPASS_MAC"));

    println!("BOB_PASSPHRASE_WIN: {:?}", std::env::var("BOB_PASSPHRASE_WIN"));
    println!("BOB_USERPASS_WIN: {:?}", std::env::var("BOB_USERPASS_WIN"));
    println!("ALICE_PASSPHRASE_WIN: {:?}", std::env::var("ALICE_PASSPHRASE_WIN"));
    println!("ALICE_USERPASS_WIN: {:?}", std::env::var("ALICE_USERPASS_WIN"));

    println!("TELEGRAM_API_KEY: {:?}", std::env::var("TELEGRAM_API_KEY"));

    assert!(false);
}
