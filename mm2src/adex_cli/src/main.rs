#[cfg(not(target_arch = "wasm32"))] mod activation_scheme;
#[cfg(not(target_arch = "wasm32"))] mod adex_config;
#[cfg(not(target_arch = "wasm32"))] mod api_commands;
#[cfg(not(target_arch = "wasm32"))] mod cli;
#[cfg(not(target_arch = "wasm32"))] mod data;
#[cfg(not(target_arch = "wasm32"))] mod helpers;
#[cfg(not(target_arch = "wasm32"))] mod log;
#[cfg(not(target_arch = "wasm32"))] mod scenarios;

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    log::init_logging();
    cli::process_cli().await;
}
