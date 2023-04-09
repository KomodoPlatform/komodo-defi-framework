use crate::adex_config::AdexConfig;
use crate::api_commands::AdexProc;
use crate::transport::SlurpTransport;
use ::log::{error, warn};

#[cfg(not(target_arch = "wasm32"))] mod activation_scheme;
#[cfg(not(target_arch = "wasm32"))] mod adex_config;
#[cfg(not(target_arch = "wasm32"))] mod api_commands;
#[cfg(not(target_arch = "wasm32"))] mod cli;
#[cfg(not(target_arch = "wasm32"))] mod data;
#[cfg(not(target_arch = "wasm32"))] mod helpers;
#[cfg(not(target_arch = "wasm32"))] mod log;
#[cfg(not(target_arch = "wasm32"))] mod scenarios;
#[cfg(not(target_arch = "wasm32"))] mod transport;

#[cfg(target_arch = "wasm32")]
fn main() {}

fn get_adex_config() -> Result<AdexConfig, ()> {
    let config = AdexConfig::from_config_path().map_err(|_| error!("Failed to get adex_config"))?;
    match config {
        config @ AdexConfig {
            rpc_password: Some(_),
            rpc_uri: Some(_),
        } => Ok(config),
        _ => {
            warn!("Failed to process, adex_config is not fully set");
            Err(())
        },
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    log::init_logging();

    let config = get_adex_config().expect("");
    let proc = AdexProc {
        transport: Box::new(SlurpTransport {
            uri: config.rpc_uri.unwrap().to_string(),
        }),
        rpc_password: config.rpc_password.unwrap().to_string(),
    };
    let _ = cli::Cli::execute(&proc).await;
}
