use inquire::Password;
use log::{error, info, warn};

use crate::adex_config::AdexConfigImpl;

pub fn get_config() {
    let Ok(adex_cfg) = AdexConfigImpl::from_config_path() else { return; };
    info!("adex config: {}", adex_cfg)
}

pub fn set_config(set_password: bool, rpc_api_uri: Option<String>) {
    let mut adex_cfg = AdexConfigImpl::from_config_path().unwrap_or_else(|()| AdexConfigImpl::default());
    let mut is_changes_happened = false;
    if set_password {
        let rpc_password = Password::new("Enter RPC API password:")
            .prompt()
            .map(|value| {
                is_changes_happened = true;
                value
            })
            .map_err(|error| error!("Failed to get rpc_api_password: {error}"))
            .ok();
        adex_cfg.set_rpc_password(rpc_password);
    }
    if rpc_api_uri.is_some() {
        adex_cfg.set_rpc_uri(rpc_api_uri);
        is_changes_happened = true;
    }

    if is_changes_happened && adex_cfg.write_to_config_path().is_ok() {
        info!("Configuration has been set");
    } else {
        warn!("Nothing changed");
    }
}
