use anyhow::{anyhow, Result};
use common::log::{error, info};
use http::StatusCode;
use itertools::Itertools;
use mm2_net::transport::slurp_url;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use crate::adex_config::AdexConfigImpl;
use crate::error_anyhow;

const ACTIVATION_SCHEME_FILE: &str = "activation_scheme.json";
const COIN_ACTIVATION_SOURCE: &str = "https://stats.kmd.io/api/table/coin_activation/";

pub(crate) async fn init_activation_scheme() -> Result<()> {
    let config_path = get_activation_scheme_path()?;
    info!("Start getting activation_scheme from: {config_path:?}");

    let mut writer = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(config_path)
        .map_err(|error| error_anyhow!("Failed to open activation_scheme file to write: {error}"))?;

    let activation_scheme = get_activation_scheme_data().await?;
    writer
        .write_all(&activation_scheme)
        .map_err(|error| error_anyhow!("Failed to write activation_scheme: {error}"))
}

pub(super) fn get_activation_scheme_path() -> Result<PathBuf> {
    let mut config_path = AdexConfigImpl::get_config_dir()?;
    config_path.push(ACTIVATION_SCHEME_FILE);
    Ok(config_path)
}

async fn get_activation_scheme_data() -> Result<Vec<u8>> {
    info!("Download activation_scheme from: {COIN_ACTIVATION_SOURCE}");
    match slurp_url(COIN_ACTIVATION_SOURCE).await {
        Ok((StatusCode::OK, _, data)) => Ok(data),
        Ok((status_code, headers, data)) => Err(error_anyhow!(
            "Failed to get activation scheme from: {COIN_ACTIVATION_SOURCE}, bad status: {status_code}, headers: {}, data: {}",
            headers.iter().map(|(k, v)| format!("{k}: {v:?}")).join(", "),
            String::from_utf8_lossy(&data)
        )),
        Err(error) => Err(error_anyhow!(
            "Failed to get activation_scheme from: {COIN_ACTIVATION_SOURCE}, error: {error}"
        )),
    }
}
