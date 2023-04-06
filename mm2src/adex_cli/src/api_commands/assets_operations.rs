use cli_table::{print_stdout, WithTitle};
use log::{error, info, warn};
use mm2_net::native_http::slurp_post_json;
use response::print_result_as_table;
use serde_json::{json, Value as Json};
use std::ops::Deref;

use super::protocol_data::{CoinPair, Command, GetEnabledResponse, Method};
use super::{get_adex_config, macros};
use crate::activation_scheme::get_activation_scheme;
use crate::adex_config::AdexConfig;
use crate::api_commands::protocol_data::SellData;
use crate::api_commands::{response, Response};
use crate::transport::Transport;

pub struct AdexProc {
    pub transport: Box<dyn Transport + 'static>,
}

impl AdexProc {
    pub async fn enable(&mut self, asset: &str) {
        let activation_scheme = get_activation_scheme();
        let Some(activate_specific_settings) = activation_scheme.get_activation_method(&asset) else {
            warn!("Asset is not known: {asset}");
            return;
        };

        let (rpc_password, rpc_uri) = macros::get_config!();
        let command = Command::builder()
            .flatten_data(activate_specific_settings.clone())
            .userpass(rpc_password)
            .build();

        match self.transport.as_ref().send::<_, Json, Json>(command).await {
            Ok(ok) => {
                let Ok(_) = print_result_as_table(ok);
            },
            Err(Ok(err)) => {
                let Ok(_) = print_result_as_table(err);
            },
            _ => {},
        };
    }
}

pub async fn enable(asset: &str) {
    let activation_scheme = get_activation_scheme();
    let Some(activate_specific_settings) = activation_scheme.get_activation_method(&asset) else {
        warn!("Asset is not known: {asset}");
        return;
    };

    let (rpc_password, rpc_uri) = macros::get_config!();
    let command = Command::builder()
        .flatten_data(activate_specific_settings.clone())
        .userpass(rpc_password)
        .build();

    let command_data = serde_json::to_string(&command).expect("Failed to serialize enable request");
    match slurp_post_json(&rpc_uri, command_data).await {
        Err(error) => {
            error!("Failed to activate: {error}");
        },
        Ok(resp) => resp.process::<Json, Json, _, _>(print_result_as_table, Some(print_result_as_table)),
    };
}

pub async fn get_balance(asset: &str) {
    info!("Getting balance, coin: {asset} ...");
    let (rpc_password, rpc_uri) = macros::get_config!();
    let command = Command::builder()
        .method(Method::GetBalance)
        .flatten_data(json!({ "coin": asset }))
        .userpass(rpc_password)
        .build();
    let command_data = serde_json::to_string(&command).expect("Failed to serialize get_balance request");
    match slurp_post_json(&rpc_uri, command_data).await {
        Err(error) => {
            error!("Failed to get balance: {error}");
        },
        Ok(resp) => resp.process::<Json, Json, _, _>(print_result_as_table, Some(print_result_as_table)),
    };
}

pub async fn get_enabled() {
    info!("Getting list of enabled coins ...");
    let (rpc_password, rpc_uri) = macros::get_config!();
    let command = Command::<i32>::builder()
        .method(Method::GetEnabledCoins)
        .userpass(rpc_password)
        .build();

    let command_data = serde_json::to_string(&command).expect("Failed to serialize get_enabled request");
    match slurp_post_json(&rpc_uri, command_data).await {
        Err(error) => error!("Failed to list enabled coins: {error}"),
        Ok(resp) => resp.process::<GetEnabledResponse, Json, _, _>(print_enabled_response, Some(print_result_as_table)),
    };
}

pub async fn get_orderbook(base: &str, rel: &str) {
    info!("Getting orderbook, base: {base}, rel: {rel} ...");
    let (rpc_password, rpc_uri) = macros::get_config!();
    let command = Command::builder()
        .userpass(rpc_password)
        .method(Method::GetOrderbook)
        .flatten_data(CoinPair::new(base, rel))
        .build();
    let command_data = serde_json::to_string(&command).expect("Failed to serialize get_orderbook request");
    match slurp_post_json(&rpc_uri, command_data).await {
        Err(error) => error!("Failed  to get orderbook: {error}"),
        Ok(resp) => resp.process::<Json, Json, _, _>(print_result_as_table, Some(print_result_as_table)),
    };
}

pub async fn sell(base: &str, rel: &str, volume: f64, price: f64) {
    info!("Sell base: {base}, rel: {rel}, volume: {volume}, price: {price} ...");
    let (rpc_password, rpc_uri) = macros::get_config!();
    let command = Command::builder()
        .userpass(rpc_password)
        .method(Method::Sell)
        .flatten_data(SellData::new(base, rel, volume, price))
        .build();
    let command_data = serde_json::to_string(&command).expect("Failed to serialize get_orderbook request");
    match slurp_post_json(&rpc_uri, command_data).await {
        Err(error) => error!("Failed  to get orderbook: {error}"),
        Ok(resp) => resp.process::<Json, Json, _, _>(print_result_as_table, Some(print_result_as_table)),
    };
}

fn print_enabled_response(response: GetEnabledResponse) -> Result<(), ()> {
    if response.result.is_empty() {
        info!("Enabled coins list is empty");
        return Ok(());
    }
    print_stdout(response.result.with_title()).map_err(|error| error!("Failed to print result: {error}"))?;
    Ok(())
}
