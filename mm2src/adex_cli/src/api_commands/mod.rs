mod assets_operations;
mod protocol_data;
mod service_operations;

use crate::adex_config::AdexConfig;
pub use assets_operations::{activate, balance};
use http::{HeaderMap, StatusCode};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
pub use service_operations::{get_config, get_version, send_stop, set_config};
use std::fmt::Display;

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
    if config.is_set() == false {
        warn!("Failed to process, adex_config is not fully set");
        return Err(());
    }
    Ok(config)
}

fn process_answer<T, F>(status: &StatusCode, _headers: &HeaderMap, data: &[u8], if_ok: F, if_err: Option<F>)
where
    T: for<'a> Deserialize<'a> + Serialize + Display,
    F: Fn(T),
{
    match status {
        &StatusCode::OK => match serde_json::from_slice::<T>(data) {
            Ok(resp_data) => if_ok(resp_data),
            Err(error) => error!("Failed to deserialize adex_response from data: {data:?}, error: {error}"),
        },
        &StatusCode::INTERNAL_SERVER_ERROR => match serde_json::from_slice::<T>(data) {
            Ok(resp_data) if if_err.is_none() => info!("{}", resp_data),
            Ok(resp_data) if if_err.is_some() => if_err.unwrap()(resp_data),
            Ok(_) => assert!(false, "unreachable code"),
            Err(error) => error!("Failed to deserialize adex_response from data: {data:?}, error: {error}"),
        },
        _ => {
            warn!("Bad http status: {status}, data: {data:?}");
        },
    };
}
