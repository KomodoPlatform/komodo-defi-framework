use crate::api_commands::data::{Method, SendStopResponse, VersionResponse};
use http::{HeaderMap, StatusCode};
use log::{error, info, warn};
use mm2_net::transport::slurp_post_json;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;

use super::adex_cli_conf::AdexCliConf;
use super::data::Command;

#[tokio::main(flavor = "current_thread")]
pub async fn send_stop() {
    let Ok(config) = AdexCliConf::from_config_path() else {
        error!("Failed to send stop");
        return;
    };

    if !config.is_set() {
        warn!("Failed to send stop, configuration is not fully set");
        return;
    }
    let AdexCliConf{ rpc_api_password: Some(rpc_api_password), rpc_api_uri: Some(rpc_api_uri)} = config
        else {
            assert!(false);
            return;
        };
    let stop_command = Command {
        userpass: rpc_api_password,
        method: Method::Stop,
    };
    let data = serde_json::to_string(&stop_command).expect("Failed to serialize stop_command");
    match slurp_post_json(&rpc_api_uri, data).await {
        Err(error) => {
            error!("Failed to stop through the API: {error}");
            return;
        },
        Ok((status, headers, data)) => process_answer::<SendStopResponse>(&status, &headers, &data),
    };
}

fn process_answer<T>(status: &StatusCode, headers: &HeaderMap, data: &[u8])
where
    T: for<'a> Deserialize<'a> + Serialize,
{
    match status {
        &StatusCode::OK => {
            let Ok(adex_reponse) = serde_json::from_slice::<T>(data) else {
                error!("Failed to deseialize adex_response from data");
                return;
            };
            info!("Got response is: {}", serde_json::to_string(&adex_reponse).unwrap());
        },
        _ => {
            warn!("Bad response, status: {status}");
        },
    };
}

#[tokio::main(flavor = "current_thread")]
pub async fn get_version() {
    let Ok(config) = AdexCliConf::from_config_path() else {
        error!("Failed to send stop");
        return;
    };

    if !config.is_set() {
        warn!("Failed to send stop, configuration is not fully set");
        return;
    }
    let AdexCliConf{ rpc_api_password: Some(rpc_api_password), rpc_api_uri: Some(rpc_api_uri)} = config
        else {
            assert!(false);
            return;
        };
    let stop_command = Command {
        userpass: rpc_api_password,
        method: Method::Version,
    };
    let data = serde_json::to_string(&stop_command).expect("Failed to serialize stop_command");
    match slurp_post_json(&rpc_api_uri, data).await {
        Err(error) => {
            error!("Failed to stop through the API: {error}");
            return;
        },
        Ok((status, headers, data)) => process_answer::<VersionResponse>(&status, &headers, &data),
    };
}

#[test]
fn test_stop() {}
