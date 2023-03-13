use mm2_test_helpers::for_tests::get_passphrase;

pub mod docker_tests_common;
pub mod qrc20_tests;

// mod docker_ordermatch_tests;
// mod docker_tests_inner;
// mod slp_tests;
// mod swap_watcher_tests;
// mod swaps_confs_settings_sync_tests;
// mod swaps_file_lock_tests;

// #[cfg(not(feature = "disable-solana-tests"))] mod solana_tests;

// dummy test helping IDE to recognize this as test module
#[test]
fn dummy() { assert!(true) }

#[test]
fn dump() {
    for (key, value) in std::env::vars() {
        println!("{key}: {value}");
    }

    let bob_passphrase = get_passphrase(&".env.seed", "BOB_PASSPHRASE").unwrap();
    dbg!(bob_passphrase.as_bytes());
    let alice_passphrase = get_passphrase(&".env.client", "ALICE_PASSPHRASE").unwrap();
    dbg!(alice_passphrase.as_bytes());

    assert!(false);
}
