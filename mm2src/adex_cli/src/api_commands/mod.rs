mod assets_operations;
mod protocol_data;
mod response;
mod service_operations;

use log::{error, warn};

use crate::adex_config::AdexConfig;
use response::Response;

pub use assets_operations::{enable, get_balance, get_enabled, get_orderbook};
pub use service_operations::{get_config, get_version, send_stop, set_config};

mod macros {

    macro_rules! get_config {
        () => {
            match get_adex_config() {
                Err(_) => {
                    return;
                },
                Ok(AdexConfig {
                    rpc_password,
                    rpc_uri,
                }) => (rpc_password.unwrap(), rpc_uri.unwrap()),
            }
        };
    }

    pub(crate) use get_config;
}

fn get_adex_config() -> Result<AdexConfig, ()> {
    let config = AdexConfig::from_config_path().map_err(|_| error!("Failed to get adex_config"))?;
    if !config.is_set() {
        warn!("Failed to process, adex_config is not fully set");
        return Err(());
    }
    Ok(config)
}
