use crate::api_commands::protocol_data::{Method, SendStopResponse, VersionResponse};
use inquire::Password;
use log::{error, info, warn};
use mm2_net::transport::slurp_post_json;

use super::{get_adex_config, macros, process_answer, protocol_data::Command};
use crate::adex_config::AdexConfig;

pub async fn send_stop() {
    let (rpc_password, rpc_uri) = macros::get_config!();
    let stop_command = Command::new().userpass(rpc_password).method(Method::Stop).build();
    let data = serde_json::to_string(&stop_command).expect("Failed to serialize stop_command");
    match slurp_post_json(&rpc_uri, data).await {
        Err(error) => {
            error!("Failed to stop through the API: {error}");
            return;
        },
        Ok((status, headers, data)) => {
            process_answer::<SendStopResponse, _>(&status, &headers, &data, |result| info!("{result}"), None)
        },
    };
}

pub async fn get_version() {
    let (rpc_password, rpc_uri) = macros::get_config!();
    let version_command = Command::new().userpass(rpc_password).method(Method::Version).build();
    let data = serde_json::to_string(&version_command).expect("Failed to serialize stop_command");
    match slurp_post_json(&rpc_uri, data).await {
        Err(error) => {
            error!("Failed to stop through the API: {error}");
            return;
        },
        Ok((status, headers, data)) => {
            process_answer::<VersionResponse, _>(&status, &headers, &data, |result| info!("{result}"), None)
        },
    };
}

pub fn get_config() {
    let Ok(adex_cfg) = AdexConfig::from_config_path() else { return; };
    info!("adex config: {}", adex_cfg)
}

pub fn set_config(set_password: bool, rpc_api_uri: Option<String>) {
    let mut adex_cfg = AdexConfig::from_config_path().unwrap_or_else(|()| AdexConfig::new());
    let mut is_changes_happened = false;
    if set_password == true {
        adex_cfg.rpc_password = Password::new("Enter RPC API password:")
            .prompt()
            .map(|value| {
                is_changes_happened = true;
                value
            })
            .map_err(|error| error!("Failed to get rpc_api_password: {error}"))
            .ok();
    }
    if rpc_api_uri.is_some() {
        adex_cfg.rpc_uri = rpc_api_uri;
        is_changes_happened = true;
    }

    if is_changes_happened == true && adex_cfg.write_to_config_path().is_ok() {
        info!("Configuration has been set");
    } else {
        warn!("Nothing changed");
    }
}
