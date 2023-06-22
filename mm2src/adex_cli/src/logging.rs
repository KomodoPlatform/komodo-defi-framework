#[cfg(not(any(test, target_arch = "wasm32")))]
use log::LevelFilter;
#[cfg(not(any(test, target_arch = "wasm32")))]
use std::io::Write;

#[cfg(not(any(test, target_arch = "wasm32")))]
pub(super) fn init_logging() {
    let mut builder = env_logger::builder();
    let level = std::env::var("RUST_LOG")
        .map(|s| s.parse().expect("Failed to parse RUST_LOG"))
        .unwrap_or(LevelFilter::Info);
    builder
        .filter_level(level)
        .format(|buf, record| writeln!(buf, "{}", record.args()));
    builder.init();
}

#[macro_export]
macro_rules! error_anyhow { ($($arg: expr),*) => { { error!($($arg),*); anyhow!("") } } }
#[macro_export]
macro_rules! error_bail { ($($arg: expr),*) => { { error!($($arg),*); bail!("") } } }
#[macro_export]
macro_rules! warn_bail { ($($arg: expr),*) => { { warn!($($arg),*); bail!("") } } }

pub(super) use {error_anyhow, error_bail, warn_bail};
