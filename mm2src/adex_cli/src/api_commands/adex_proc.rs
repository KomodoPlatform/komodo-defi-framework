use cli_table::format::{Border, Separator};
use cli_table::{print_stdout, Table, WithTitle};
use log::{error, info, warn};
use mm2_rpc::mm_protocol::VersionResponse;
use serde_json::{json, Value as Json};

use super::protocol_data::{CoinPair, Command, GetEnabledResponse, Method};
use crate::activation_scheme::get_activation_scheme;
use crate::api_commands::printer::Printer;
use crate::api_commands::protocol_data::Dummy;
use crate::api_commands::protocol_data::SellData;
use crate::transport::Transport;

pub struct AdexProc<'a, 'p, T: Transport, P: Printer> {
    pub transport: &'a T,
    pub printer: &'p P,
    pub rpc_password: String,
}

impl<T: Transport, P: Printer> AdexProc<'_, '_, T, P> {
    pub async fn enable(&self, asset: &str) -> Result<(), ()> {
        info!("Enabling asset: {asset}");
        let _ = self.transport.send::<_, i32, Json>(1).await;

        let activation_scheme = get_activation_scheme();
        let Some(activate_specific_settings) = activation_scheme.get_activation_method(&asset) else {
            warn!("Asset is not known: {asset}");
            return Err(());
        };

        let command = Command::builder()
            .flatten_data(activate_specific_settings.clone())
            .userpass(self.rpc_password.clone())
            .build();

        match self.transport.send::<_, Json, Json>(command).await {
            Ok(Ok(ok)) => print_result_as_table(ok),
            Ok(Err(err)) => print_result_as_table(err),
            _ => Err(()),
        }
    }

    pub async fn get_balance(&self, asset: &str) -> Result<(), ()> {
        info!("Getting balance, coin: {asset} ...");
        let command = Command::builder()
            .method(Method::GetBalance)
            .flatten_data(json!({ "coin": asset }))
            .userpass(self.rpc_password.clone())
            .build();

        match self.transport.send::<_, Json, Json>(command).await {
            Ok(Ok(ok)) => print_result_as_table(ok),
            Ok(Err(err)) => print_result_as_table(err),
            _ => Err(()),
        }
    }

    pub async fn get_enabled(&self) -> Result<(), ()> {
        info!("Getting list of enabled coins ...");

        let command = Command::<i32>::builder()
            .method(Method::GetEnabledCoins)
            .userpass(self.rpc_password.clone())
            .build();

        match self.transport.send::<_, GetEnabledResponse, Json>(command).await {
            Ok(Ok(ok)) => Self::print_enabled_response(ok),
            Ok(Err(err)) => print_result_as_table(err),
            _ => Err(()),
        }
    }

    fn print_enabled_response(response: GetEnabledResponse) -> Result<(), ()> {
        if response.result.is_empty() {
            info!("Enabled coins list is empty");
            return Ok(());
        }
        print_stdout(response.result.with_title()).map_err(|error| error!("Failed to print result: {error}"))?;
        Ok(())
    }

    pub async fn get_orderbook(&self, base: &str, rel: &str) -> Result<(), ()> {
        info!("Getting orderbook, base: {base}, rel: {rel} ...");

        let command = Command::builder()
            .userpass(self.rpc_password.clone())
            .method(Method::GetOrderbook)
            .flatten_data(CoinPair::new(base, rel))
            .build();

        match self.transport.send::<_, Json, Json>(command).await {
            Ok(Ok(ok)) => print_result_as_table(ok),
            Ok(Err(err)) => print_result_as_table(err),
            _ => Err(()),
        }
    }

    pub async fn sell(&self, base: &str, rel: &str, volume: f64, price: f64) -> Result<(), ()> {
        info!("Sell base: {base}, rel: {rel}, volume: {volume}, price: {price} ...");
        let command = Command::builder()
            .userpass(self.rpc_password.clone())
            .method(Method::Sell)
            .flatten_data(SellData::new(base, rel, volume, price))
            .build();

        match self.transport.send::<_, Json, Json>(command).await {
            Ok(Ok(ok)) => print_result_as_table(ok),
            Ok(Err(err)) => print_result_as_table(err),
            _ => Err(()),
        }
    }

    pub async fn send_stop(&self) -> Result<(), ()> {
        let stop_command = Command::<Dummy>::builder()
            .userpass(self.rpc_password.clone())
            .method(Method::Stop)
            .build();

        match self.transport.send::<_, Json, Json>(stop_command).await {
            Ok(Ok(ok)) => info!("{ok}"),
            Ok(Err(error)) => {
                error!("Failed to stop through the API: {error}");
                return Err(());
            },
            _ => return Err(()),
        };
        Ok(())
    }

    pub async fn get_version(self) -> Result<(), ()> {
        let version_command = Command::<Dummy>::builder()
            .userpass(self.rpc_password)
            .method(Method::Version)
            .build();

        match self.transport.send::<_, VersionResponse, Json>(version_command).await {
            Ok(Ok(ok)) => self.printer.display_response(ok),
            Ok(Err(error)) => {
                error!("Failed get version through the API: {error}");
                return Err(());
            },
            _ => return Err(()),
        }
    }
}

pub fn print_result_as_table(result: Json) -> Result<(), ()> {
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
