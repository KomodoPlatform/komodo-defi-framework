use cli_table::format::{Border, Separator};
use cli_table::{print_stdout, Table, WithTitle};
use log::{error, info, warn};
use mm2_net::native_http::slurp_post_json;
use serde_json::{json, Value as Json};

use super::protocol_data::{CoinPair, Command, GetEnabledResponse, Method};
use super::{get_adex_config, macros};
use crate::activation_scheme::get_activation_scheme;
use crate::adex_config::AdexConfig;
use crate::api_commands::Response;

pub async fn activate(asset: &str) {
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

    let command_data = serde_json::to_string(&command).expect("Failed to serialize activate command");
    match slurp_post_json(&rpc_uri, command_data).await {
        Err(error) => {
            error!("Failed to activate: {error}");
        },
        Ok(resp) => resp.process::<Json, Json, _, _>(print_result_as_talbe, Some(print_result_as_talbe)),
    };
}

pub async fn balance(asset: &str) {
    let (rpc_password, rpc_uri) = macros::get_config!();
    let command = Command::builder()
        .method(Method::GetBalance)
        .flatten_data(json!({ "coin": asset }))
        .userpass(rpc_password)
        .build();
    let command_data = serde_json::to_string(&command).expect("Failed to serialize balance command");
    match slurp_post_json(&rpc_uri, command_data).await {
        Err(error) => {
            error!("Failed to get balance: {error}");
        },
        Ok(resp) => resp.process::<Json, Json, _, _>(print_result_as_talbe, Some(print_result_as_talbe)),
    };
}

pub async fn get_enabled() {
    let (rpc_password, rpc_uri) = macros::get_config!();
    let command = Command::<i32>::builder()
        .method(Method::GetEnabledCoins)
        .userpass(rpc_password)
        .build();

    let command_data = serde_json::to_string(&command).expect("Failed to serialize list activated command");
    match slurp_post_json(&rpc_uri, command_data).await {
        Err(error) => {
            error!("Failed to list activated: {error}");
        },
        Ok(resp) => {
            resp.process::<GetEnabledResponse, Json, _, _>(print_enabled_coins_result, Some(print_result_as_talbe))
        },
    };
}

pub async fn get_orderbook(base: &str, rel: &str) {
    let (rpc_password, _) = macros::get_config!();
    let _command = Command::builder()
        .userpass(rpc_password)
        .method(Method::GetOrderbook)
        .flatten_data(CoinPair::new(base, rel))
        .build();
}

fn print_result_as_talbe(result: Json) -> Result<(), ()> {
    let object = result
        .as_object()
        .ok_or_else(|| error!("Failed to cast result as object"))?;

    let data: Vec<SimpleCliTable> = object.iter().map(SimpleCliTable::from_pair).collect();
    let data = data
        .table()
        .border(Border::builder().build())
        .separator(Separator::builder().build());
    print_stdout(data).map_err(|error| error!("Failed to print result: {error}"))
}

fn print_enabled_coins_result(response: GetEnabledResponse) -> Result<(), ()> {
    if response.result.is_empty() {
        info!("Enabled coins list is empty");
        return Ok(());
    }
    print_stdout(response.result.with_title()).map_err(|error| error!("Failed to print result: {error}"))?;
    Ok(())
}

#[derive(Table)]
struct SimpleCliTable<'a> {
    #[table(title = "Parameter")]
    key: &'a String,
    #[table(title = "Value")]
    value: &'a Json,
}

impl<'a> SimpleCliTable<'a> {
    fn from_pair(pair: (&'a String, &'a Json)) -> Self {
        SimpleCliTable {
            key: pair.0,
            value: pair.1,
        }
    }
}
