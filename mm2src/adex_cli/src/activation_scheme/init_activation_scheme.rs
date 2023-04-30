use crate::activation_scheme;
use crate::activation_scheme::COIN_ACTIVATION_SOURCE;
use common::log::{error, info};
use mm2_net::transport::slurp_url;
use std::fs::OpenOptions;
use std::io::Write;

pub const ACTIVATION_SCHEME_FILE: &str = "activation_scheme.json";

pub async fn init_activation_scheme() -> Result<(), ()> {
    info!("Start getting activation_scheme from");
    let config_path = activation_scheme::get_activation_scheme_path()?;

    let mut writer = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(config_path)
        .map_err(|error| error!("Faile to open activation_scheme file to write: {error}"))?;

    let activation_scheme = get_activation_scheme_data().await?;
    writer
        .write_all(&activation_scheme)
        .map_err(|error| error!("Failed to write activation_scheme: {error}"))
}

async fn get_activation_scheme_data() -> Result<Vec<u8>, ()> {
    info!("Download activation_scheme from: {COIN_ACTIVATION_SOURCE}");
    let (_status_code, _headers, data) = slurp_url(COIN_ACTIVATION_SOURCE).await.map_err(|error| {
        error!("Failed to get activation_scheme from: {COIN_ACTIVATION_SOURCE}, error: {error}");
    })?;

    Ok(data)
}
