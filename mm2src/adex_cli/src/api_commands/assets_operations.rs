use cli_table::format::{Border, Separator};
use cli_table::{print_stdout, Table};
use log::{error, warn};
use mm2_net::native_http::slurp_post_json;
use serde_json::{json, Value as Json};

use super::protocol_data::{Command, Method};
use super::{get_adex_config, macros, process_answer};
use crate::activation_scheme::get_activation_scheme;
use crate::adex_config::AdexConfig;

pub async fn activate(asset: String) {
    let activation_scheme = get_activation_scheme();
    let Some(activate_specific_settings) = activation_scheme.get_activation_method(&asset) else {
        warn!("Asset is not known: {asset}");
        return;
    };

    let (rpc_password, rpc_uri) = macros::get_config!();
    let command = Command::new()
        .flatten_data(activate_specific_settings.clone())
        .userpass(rpc_password)
        .build();

    let command_data = serde_json::to_string(&command).expect("Failed to serialize activate command");
    match slurp_post_json(&rpc_uri, command_data).await {
        Err(error) => {
            error!("Failed to activate: {error}");
            return;
        },
        Ok((status, headers, data)) => process_answer::<Json, _>(
            &status,
            &headers,
            &data,
            print_result_as_talbe,
            Some(print_result_as_talbe),
        ),
    };
}

fn print_result_as_talbe(result: Json) {
    let Some(object) = result.as_object() else {
        error!("Failed to cast result as object");
        return;
    };
    let data: Vec<SimpleCliTable> = object.iter().map(SimpleCliTable::from_pair).collect();
    let data = data
        .table()
        .border(Border::builder().build())
        .separator(Separator::builder().build());
    if let Err(error) = print_stdout(data) {
        error!("Failed to print result: {error}");
    };
}

pub async fn balance(asset: String) {
    let (rpc_password, rpc_uri) = macros::get_config!();
    let command = Command::new()
        .method(Method::Balance)
        .flatten_data(json!({ "coin": asset }))
        .userpass(rpc_password)
        .build();
    let command_data = serde_json::to_string(&command).expect("Failed to serialize balance command");
    match slurp_post_json(&rpc_uri, command_data).await {
        Err(error) => {
            error!("Failed to get balance: {error}");
            return;
        },
        Ok((status, headers, data)) => process_answer::<Json, _>(
            &status,
            &headers,
            &data,
            print_result_as_talbe,
            Some(print_result_as_talbe),
        ),
    };
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
