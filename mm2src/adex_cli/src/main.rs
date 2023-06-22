#[cfg(not(target_arch = "wasm32"))] mod activation_scheme_db;
#[cfg(not(any(test, target_arch = "wasm32")))] mod adex_app;
#[cfg(not(target_arch = "wasm32"))] mod adex_config;
#[cfg(not(target_arch = "wasm32"))] mod adex_proc;
#[cfg(not(target_arch = "wasm32"))] mod cli;
#[cfg(not(target_arch = "wasm32"))] mod helpers;
mod logging;
#[cfg(not(target_arch = "wasm32"))] mod scenarios;
#[cfg(all(not(target_arch = "wasm32"), test))] mod tests;
#[cfg(not(target_arch = "wasm32"))] mod transport;

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(any(test, target_arch = "wasm32")))]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    logging::init_logging();
    let Ok(app) = adex_app::AdexApp::new() else { return; };

    app.execute().await;
}
