use common::log::{error, info};
use derive_more::Display;
use mm2_net::transport::slurp_url;

use super::helpers::rewrite_data_file;

#[derive(Clone, Copy, Debug, Display)]
pub enum CoinSet {
    Empty,
    Full,
}

#[tokio::main(flavor = "current_thread")]
pub async fn init_coins(coins_file: &str) -> Result<(), ()> {
    const FULL_COIN_SET_ADDRESS: &str = "https://raw.githubusercontent.com/KomodoPlatform/coins/master/coins";
    const EMPTY_COIN_SET_DATA: &[u8] = b"[]\n";
    let coin_set = inquire_coin_set(coins_file)?;
    info!("Start getting mm2 coins");
    let coins_data = match coin_set {
        CoinSet::Empty => Vec::<u8>::from(EMPTY_COIN_SET_DATA),
        CoinSet::Full => {
            info!("Getting coin set from: {FULL_COIN_SET_ADDRESS}");
            let (_status_code, _headers, data) = slurp_url(FULL_COIN_SET_ADDRESS).await.map_err(|error| {
                error!("Failed to get coin set from: {FULL_COIN_SET_ADDRESS}, error: {error}");
            })?;
            data
        },
    };

    rewrite_data_file(coins_data, coins_file)?;
    info!("Got coins data, written into: {coins_file}");
    Ok(())
}

fn inquire_coin_set(coins_file: &str) -> Result<CoinSet, ()> {
    inquire::Select::new(
        format!("Select one of predefined coin sets to save into: {coins_file}").as_str(),
        vec![CoinSet::Empty, CoinSet::Full],
    )
    .with_help_message("Information about the currencies: their ticker symbols, names, ports, addresses, etc.")
    .prompt()
    .map_err(|error| {
        error!("Failed to select coin_set: {error}");
    })
}
