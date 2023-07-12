use anyhow::{anyhow, Result};
use common::log::{error, info};
use mm2_net::transport::slurp_url;

use crate::helpers::rewrite_data_file;
use crate::logging::error_anyhow;

const FULL_COIN_SET_ADDRESS: &str = "https://raw.githubusercontent.com/KomodoPlatform/coins/master/coins";

pub(crate) async fn init_coins(coins_file: &str) -> Result<()> {
    info!("Getting coin set from: {FULL_COIN_SET_ADDRESS}");
    let (_status_code, _headers, coins_data) = slurp_url(FULL_COIN_SET_ADDRESS)
        .await
        .map_err(|error| error_anyhow!("Failed to get coin set from: {FULL_COIN_SET_ADDRESS}, error: {error}"))?;

    rewrite_data_file(coins_data, coins_file)?;
    info!("Got coins data, written into: {coins_file}");
    Ok(())
}
