use log::{error, info, warn};
use mm2_rpc::mm_protocol::{OrderbookResponse, VersionResponse};
use serde_json::{json, Value as Json};

use super::protocol_data::{CoinPair, Command, GetEnabledResponse, Method};
use crate::activation_scheme::get_activation_scheme;
use crate::adex_config::AdexConfig;
use crate::api_commands::protocol_data::Dummy;
use crate::api_commands::protocol_data::SellData;
use crate::api_commands::response_handler;
use crate::api_commands::response_handler::ResponseHandler;
use crate::transport::Transport;

pub struct AdexProc<'trp, 'hand, 'cfg, T: Transport, H: ResponseHandler, C: AdexConfig + ?Sized> {
    pub transport: &'trp T,
    pub response_handler: &'hand H,
    pub config: &'cfg C,
}

impl<T: Transport, P: ResponseHandler, C: AdexConfig + 'static> AdexProc<'_, '_, '_, T, P, C> {
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
            .userpass(self.config.rpc_password())
            .build();

        match self.transport.send::<_, Json, Json>(command).await {
            Ok(Ok(ok)) => response_handler::print_result_as_table(ok),
            Ok(Err(err)) => response_handler::print_result_as_table(err),
            _ => Err(()),
        }
    }

    pub async fn get_balance(&self, asset: &str) -> Result<(), ()> {
        info!("Getting balance, coin: {asset} ...");
        let command = Command::builder()
            .method(Method::GetBalance)
            .flatten_data(json!({ "coin": asset }))
            .userpass(self.config.rpc_password())
            .build();

        match self.transport.send::<_, Json, Json>(command).await {
            Ok(Ok(ok)) => response_handler::print_result_as_table(ok),
            Ok(Err(err)) => response_handler::print_result_as_table(err),
            _ => Err(()),
        }
    }

    pub async fn get_enabled(&self) -> Result<(), ()> {
        info!("Getting list of enabled coins ...");

        let command = Command::<i32>::builder()
            .method(Method::GetEnabledCoins)
            .userpass(self.config.rpc_password())
            .build();

        match self.transport.send::<_, GetEnabledResponse, Json>(command).await {
            Ok(Ok(ok)) => self.response_handler.on_get_enabled_response(&ok),
            Ok(Err(err)) => response_handler::print_result_as_table(err),
            _ => Err(()),
        }
    }

    pub async fn get_orderbook(
        &self,
        base: &str,
        rel: &str,
        asks_limit: &Option<usize>,
        bids_limit: &Option<usize>,
    ) -> Result<(), ()> {
        info!("Getting orderbook, base: {base}, rel: {rel} ...");

        let command = Command::builder()
            .userpass(self.config.rpc_password())
            .method(Method::GetOrderbook)
            .flatten_data(CoinPair::new(base, rel))
            .build();

        match self.transport.send::<_, OrderbookResponse, Json>(command).await {
            Ok(Ok(ok)) => self
                .response_handler
                .on_orderbook_response(ok, self.config, asks_limit, bids_limit),
            Ok(Err(err)) => response_handler::print_result_as_table(err),
            _ => Err(()),
        }
    }

    pub async fn sell(&self, base: &str, rel: &str, volume: f64, price: f64) -> Result<(), ()> {
        info!("Sell base: {base}, rel: {rel}, volume: {volume}, price: {price} ...");
        let command = Command::builder()
            .userpass(self.config.rpc_password())
            .method(Method::Sell)
            .flatten_data(SellData::new(base, rel, volume, price))
            .build();

        match self.transport.send::<_, Json, Json>(command).await {
            Ok(Ok(ok)) => response_handler::print_result_as_table(ok),
            Ok(Err(err)) => response_handler::print_result_as_table(err),
            _ => Err(()),
        }
    }

    pub async fn send_stop(&self) -> Result<(), ()> {
        let stop_command = Command::<Dummy>::builder()
            .userpass(self.config.rpc_password())
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
            .userpass(self.config.rpc_password())
            .method(Method::Version)
            .build();

        match self.transport.send::<_, VersionResponse, Json>(version_command).await {
            Ok(Ok(ok)) => self.response_handler.display_response(&ok),
            Ok(Err(error)) => {
                error!("Failed get version through the API: {error}");
                return Err(());
            },
            _ => return Err(()),
        }
    }
}
