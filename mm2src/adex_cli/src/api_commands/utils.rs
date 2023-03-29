use std::fmt::Display;
use crate::api_commands::api_data::{Method, SendStopResponse, VersionResponse};
use http::{HeaderMap, StatusCode};
use inquire::Password;
use log::{error, info, warn};
use mm2_net::transport::slurp_post_json;
use serde::{Deserialize, Serialize};
use std::fs;

use super::adex_cli_conf::AdexCliConf;
use super::api_data::Command;

#[macro_export]
macro_rules! get_config {
    () => {
        match get_adex_config() {
            Err(_) => {
                return;
            },
            Ok(AdexCliConf{rpc_password, rpc_uri}) => (rpc_password.unwrap(), rpc_uri.unwrap()),
        }
    };
}

#[tokio::main(flavor = "current_thread")]
pub async fn send_stop() {
    let (rpc_password, rpc_uri) = get_config!();
    let stop_command = Command {
        userpass: rpc_password,
        method: Method::Stop,
    };
    let data = serde_json::to_string(&stop_command).expect("Failed to serialize stop_command");
    match slurp_post_json(&rpc_uri, data).await {
        Err(error) => {
            error!("Failed to stop through the API: {error}");
            return;
        },
        Ok((status, headers, data)) => process_answer::<SendStopResponse>(&status, &headers, &data),
    };
}

fn get_adex_config() -> Result<AdexCliConf, ()> {
    let config = AdexCliConf::from_config_path().map_err(|_| error!("Failed to send stop"))?;
    info!("Config: {config:?}");
    if config.is_set() == false {
        warn!("Failed to send stop, configuration is not fully set");
        return Err(());
    }

    Ok(config)
}

fn process_answer<T>(status: &StatusCode, _headers: &HeaderMap, data: &[u8])
where
    T: for<'a> Deserialize<'a> + Serialize + Display,
{
    match status {
        &StatusCode::OK => {
            let Ok(adex_reponse) = serde_json::from_slice::<T>(data) else {
                error!("Failed to deseialize adex_response from data");
                return;
            };
            info!("Got response:\n{}", adex_reponse);
        },
        _ => {
            warn!("Bad http status: {status}");
        },
    };
}

#[tokio::main(flavor = "current_thread")]
pub async fn get_version() {
    let (rpc_password, rpc_uri) = get_config!();
    let version_command = Command {
        userpass: rpc_password,
        method: Method::Version,
    };
    let data = serde_json::to_string(&version_command).expect("Failed to serialize stop_command");
    match slurp_post_json(&rpc_uri, data).await {
        Err(error) => {
            error!("Failed to stop through the API: {error}");
            return;
        },
        Ok((status, headers, data)) => process_answer::<VersionResponse>(&status, &headers, &data),
    };
}

pub fn get_config() {
    let Ok(adex_cfg) = AdexCliConf::from_config_path() else { return; };
    info!("adex config: {}", adex_cfg)
}

pub fn set_config(set_password: bool, rpc_api_uri: Option<String>) {
    match AdexCliConf::get_config_dir() {
        Ok(ref config_dir) => {
            if let Err(error) = fs::create_dir_all(config_dir) {
                error!("Failed to create config_dir: {config_dir:?}, error: {error}");
                return;
            };
        },
        Err(_) => return,
    }

    let mut adex_cfg = AdexCliConf::from_config_path().unwrap_or_else(|()| AdexCliConf::new());
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
