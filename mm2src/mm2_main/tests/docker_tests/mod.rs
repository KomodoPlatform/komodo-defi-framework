pub mod docker_tests_common;

mod docker_ordermatch_tests;
mod docker_tests_inner;
pub mod qrc20_tests;
mod slp_tests;
mod swap_watcher_tests;
mod swaps_confs_settings_sync_tests;
mod swaps_file_lock_tests;

#[cfg(feature = "enable-solana")] mod solana_tests;

// dummy test helping IDE to recognize this as test module
#[test]
fn dummy() { assert!(true) }
