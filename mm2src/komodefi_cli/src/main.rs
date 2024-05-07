#[cfg(not(target_arch = "wasm32"))] mod activation_scheme_db;
#[cfg(not(any(test, target_arch = "wasm32")))] mod app;
#[cfg(not(target_arch = "wasm32"))] mod cli;
#[cfg(not(target_arch = "wasm32"))] mod cli_cmd_args;
#[cfg(not(target_arch = "wasm32"))] mod config;
#[cfg(not(target_arch = "wasm32"))] mod helpers;
#[cfg(not(target_arch = "wasm32"))] mod komodefi_proc;
mod logging;
#[cfg(not(target_arch = "wasm32"))] mod rpc_data;
#[cfg(not(target_arch = "wasm32"))] mod scenarios;
#[cfg(all(not(target_arch = "wasm32"), test))] mod tests;
#[cfg(not(target_arch = "wasm32"))] mod transport;

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(any(test, target_arch = "wasm32")))]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    logging::init_logging();
    let app = app::KomodefiApp::new();
    app.execute().await;
}
