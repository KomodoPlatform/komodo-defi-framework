use inquire::Password;
use log::{error, info, warn};
use mm2_net::transport::slurp_post_json;
use serde_json::Value as Json;

use super::protocol_data::{Dummy, Method, StopResponse, VersionResponse};
use super::{get_adex_config, macros, protocol_data::Command};
use crate::adex_config::AdexConfig;
use crate::api_commands::Response;

pub async fn send_stop() {
    let (rpc_password, rpc_uri) = macros::get_config!();
    let stop_command = Command::<Dummy>::builder()
        .userpass(rpc_password)
        .method(Method::Stop)
        .build();
    let data = serde_json::to_string(&stop_command).expect("Failed to serialize stop_command");
    match slurp_post_json(&rpc_uri, data).await {
        Err(error) => error!("Failed to stop through the API: {error}"),
        Ok(resp) => resp.process::<StopResponse, Json, _, _>(|res| Ok(info!("{res}")), Some(|_| Ok(()))),
    };
}

pub async fn get_version() {
    let (rpc_password, rpc_uri) = macros::get_config!();
    let version_command = Command::<Dummy>::builder()
        .userpass(rpc_password)
        .method(Method::Version)
        .build();
    let data = serde_json::to_string(&version_command).expect("Failed to serialize stop_command");
    match slurp_post_json(&rpc_uri, data).await {
        Err(error) => error!("Failed to stop through the API: {error}"),
        Ok(resp) => resp.process::<VersionResponse, Json, _, _>(|r| Ok(info!("{r}")), Some(|_| Ok(()))),
    };
}

pub fn get_config() {
    let Ok(adex_cfg) = AdexConfig::from_config_path() else { return; };
    info!("adex config: {}", adex_cfg)
}

pub fn set_config(set_password: bool, rpc_api_uri: Option<String>) {
    let mut adex_cfg = AdexConfig::from_config_path().unwrap_or_else(|()| AdexConfig::new());
    let mut is_changes_happened = false;
    if set_password {
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

    if is_changes_happened && adex_cfg.write_to_config_path().is_ok() {
        info!("Configuration has been set");
    } else {
        warn!("Nothing changed");
    }
}
