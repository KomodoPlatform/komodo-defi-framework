pub mod docker_tests_common;

//@FIXME - Failing tests

#[cfg(feature = "enable-failing-tests")]
mod docker_ordermatch_tests;

#[cfg(feature = "enable-failing-tests")] mod docker_tests_inner;

#[cfg(feature = "enable-failing-tests")] pub mod qrc20_tests;

#[cfg(feature = "enable-failing-tests")] mod slp_tests;

#[cfg(feature = "enable-failing-tests")] mod swap_proto_v2_tests;

#[cfg(feature = "enable-failing-tests")] mod swap_watcher_tests;

#[cfg(feature = "enable-failing-tests")]
mod swaps_confs_settings_sync_tests;

#[cfg(feature = "enable-failing-tests")]
mod swaps_file_lock_tests;

#[cfg(feature = "enable-solana")] mod solana_tests;

// dummy test helping IDE to recognize this as test module
#[test]
#[allow(clippy::assertions_on_constants)]
fn dummy() { assert!(true) }
