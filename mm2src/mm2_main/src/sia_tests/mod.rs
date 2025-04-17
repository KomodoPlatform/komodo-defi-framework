// #[cfg(feature = "run-sia-functional-tests")]
mod docker_functional_tests;

/// This module is a temporary hack to allow grouping the relevant tests together via `cargo test` commands.
/// See doc comment inside short_locktime_tests.rs for more details.
// #[cfg(feature = "run-sia-functional-tests-short-locktime")]
mod short_locktime_tests;

pub(crate) mod utils;
