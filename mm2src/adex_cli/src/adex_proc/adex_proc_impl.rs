use anyhow::{bail, Result};
use log::{error, info, warn};
use mm2_rpc::data::legacy::{BalanceResponse, CoinInitResponse, GetEnabledResponse, Mm2RpcResult, MmVersionResponse,
                            OrderbookRequest, OrderbookResponse, SellBuyRequest, SellBuyResponse, Status};
use serde_json::{json, Value as Json};

use super::command::{Command, Dummy, Method};
use super::response_handler::ResponseHandler;
use super::OrderbookConfig;
use crate::activation_scheme_db::get_activation_scheme;
use crate::adex_config::AdexConfig;
use crate::transport::Transport;
use crate::{error_bail, warn_bail};

pub(crate) struct AdexProc<'trp, 'hand, 'cfg, T: Transport, H: ResponseHandler, C: AdexConfig + ?Sized> {
    pub(crate) transport: &'trp T,
    pub(crate) response_handler: &'hand H,
    pub(crate) config: &'cfg C,
}

impl<T: Transport, P: ResponseHandler, C: AdexConfig + 'static> AdexProc<'_, '_, '_, T, P, C> {
    pub(crate) async fn enable(&self, asset: &str) -> Result<()> {
        info!("Enabling asset: {asset}");

        let activation_scheme = get_activation_scheme()?;
        let Some(activation_method) = activation_scheme.get_activation_method(asset) else {
            warn_bail!("Asset is not known: {asset}")
        };

        let command = Command::builder()
            .flatten_data(activation_method)
            .userpass(self.config.rpc_password()?)
            .build();

        match self.transport.send::<_, CoinInitResponse, Json>(command).await {
            Ok(Ok(ref ok)) => self.response_handler.on_enable_response(ok),
            Ok(Err(err)) => self.response_handler.print_response(err),
            Err(err) => error_bail!("Failed to enable asset: {asset}, error: {err:?}"),
        }
    }

    pub(crate) async fn get_balance(&self, asset: &str) -> Result<()> {
        info!("Getting balance, coin: {asset} ...");
        let command = Command::builder()
            .method(Method::GetBalance)
            .flatten_data(json!({ "coin": asset }))
            .userpass(self.config.rpc_password()?)
            .build();

        match self.transport.send::<_, BalanceResponse, Json>(command).await {
            Ok(Ok(balance_response)) => self.response_handler.on_balance_response(&balance_response),
            Ok(Err(err)) => self.response_handler.print_response(err),
            Err(err) => error_bail!("Failed to get balance: {err:?}"),
        }
    }

    pub(crate) async fn get_enabled(&self) -> Result<()> {
        info!("Getting list of enabled coins ...");

        let command = Command::<i32>::builder()
            .method(Method::GetEnabledCoins)
            .userpass(self.config.rpc_password()?)
            .build();

        match self
            .transport
            .send::<_, Mm2RpcResult<GetEnabledResponse>, Json>(command)
            .await
        {
            Ok(Ok(ok)) => self.response_handler.on_get_enabled_response(&ok),
            Ok(Err(err)) => self.response_handler.print_response(err),
            Err(err) => error_bail!("Failed to get enabled coins: {:?}", err),
        }
    }

    pub(crate) async fn get_orderbook(&self, base: &str, rel: &str, orderbook_config: OrderbookConfig) -> Result<()> {
        info!("Getting orderbook, base: {base}, rel: {rel} ...");

        let command = Command::builder()
            .userpass(self.config.rpc_password()?)
            .method(Method::GetOrderbook)
            .flatten_data(OrderbookRequest {
                base: base.to_string(),
                rel: rel.to_string(),
            })
            .build();

        match self.transport.send::<_, OrderbookResponse, Json>(command).await {
            Ok(Ok(ok)) => self
                .response_handler
                .on_orderbook_response(ok, self.config, orderbook_config),
            Ok(Err(err)) => self.response_handler.print_response(err),
            Err(err) => error_bail!("Failed to get orderbook: {err:?}"),
        }
    }

    pub(crate) async fn sell(&self, order: SellBuyRequest) -> Result<()> {
        info!(
            "Selling: {} {} for: {} {} at the price of {} {} per {}",
            order.volume,
            order.base,
            order.volume.clone() * order.price.clone(),
            order.rel,
            order.price,
            order.rel,
            order.base,
        );

        let command = Command::builder()
            .userpass(self.config.rpc_password()?)
            .method(Method::Sell)
            .flatten_data(order)
            .build();

        match self
            .transport
            .send::<_, Mm2RpcResult<SellBuyResponse>, Json>(command)
            .await
        {
            Ok(Ok(ok)) => self.response_handler.on_sell_response(&ok),
            Ok(Err(err)) => self.response_handler.print_response(err),
            Err(err) => error_bail!("Failed to sell: {err:?}"),
        }
    }

    pub(crate) async fn buy(&self, order: SellBuyRequest) -> Result<()> {
        info!(
            "Buying: {} {} with: {} {} at the price of {} {} per {}",
            order.volume,
            order.base,
            order.volume.clone() * order.price.clone(),
            order.rel,
            order.price,
            order.rel,
            order.base,
        );

        let command = Command::builder()
            .userpass(self.config.rpc_password()?)
            .method(Method::Buy)
            .flatten_data(order)
            .build();

        match self
            .transport
            .send::<_, Mm2RpcResult<SellBuyResponse>, Json>(command)
            .await
        {
            Ok(Ok(ok)) => self.response_handler.on_buy_response(&ok),
            Ok(Err(err)) => self.response_handler.print_response(err),
            Err(err) => error_bail!("Failed to buy: {err:?}"),
        }
    }

    pub(crate) async fn send_stop(&self) -> Result<()> {
        info!("Sending stop command");
        let stop_command = Command::<Dummy>::builder()
            .userpass(self.config.rpc_password()?)
            .method(Method::Stop)
            .build();

        match self.transport.send::<_, Mm2RpcResult<Status>, Json>(stop_command).await {
            Ok(Ok(ok)) => self.response_handler.on_stop_response(&ok),
            Ok(Err(error)) => error_bail!("Failed to stop through the API: {error}"),
            _ => bail!(""),
        }
    }

    pub(crate) async fn get_version(self) -> Result<()> {
        info!("Request for mm2 version");
        let version_command = Command::<Dummy>::builder()
            .userpass(self.config.rpc_password()?)
            .method(Method::Version)
            .build();

        match self.transport.send::<_, MmVersionResponse, Json>(version_command).await {
            Ok(Ok(ok)) => self.response_handler.on_version_response(&ok),
            Ok(Err(error)) => error_bail!("Failed get version through the API: {error}"),
            _ => bail!(""),
        }
    }
}
