use anyhow::{anyhow, bail, Result};
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
use crate::{error_anyhow, error_bail, warn_anyhow, warn_bail};

pub(crate) struct AdexProc<'trp, 'hand, 'cfg, T: Transport, H: ResponseHandler, C: AdexConfig + ?Sized> {
    pub(crate) transport: Option<&'trp T>,
    pub(crate) response_handler: &'hand H,
    pub(crate) config: &'cfg C,
}

macro_rules! request_legacy {
    ($request: ident, $response_ty: ty, $self: ident, $handle_method: ident$ (, $opt:expr)*) => {{
        let transport = $self.transport.ok_or_else(|| warn_anyhow!( concat!("Failed to send: `", stringify!($request), "`, transport is not available")))?;
        match transport.send::<_, $response_ty, Json>($request).await {
            Ok(Ok(ok)) => $self.response_handler.$handle_method(&ok, $($opt),*),
            Ok(Err(error)) => $self.response_handler.print_response(error),
            Err(error) => error_bail!(
                concat!("Failed to send: `", stringify!($request), "`, error: {}"),
                error
            ),
        }
    }};
}

impl<T: Transport, P: ResponseHandler, C: AdexConfig + 'static> AdexProc<'_, '_, '_, T, P, C> {
    pub(crate) async fn enable(&self, asset: &str) -> Result<()> {
        info!("Enabling asset: {asset}");

        let activation_scheme = get_activation_scheme()?;
        let Some(activation_method) = activation_scheme.get_activation_method(asset) else {
            warn_bail!("Asset is not known: {asset}")
        };

        let enable = Command::builder()
            .flatten_data(activation_method)
            .userpass(self.get_rpc_password()?)
            .build();

        request_legacy!(enable, CoinInitResponse, self, on_enable_response)
    }

    pub(crate) async fn get_balance(&self, asset: &str) -> Result<()> {
        info!("Getting balance, coin: {asset} ...");
        let get_balance = Command::builder()
            .method(Method::GetBalance)
            .flatten_data(json!({ "coin": asset }))
            .userpass(self.get_rpc_password()?)
            .build();
        request_legacy!(get_balance, BalanceResponse, self, on_balance_response)
    }

    pub(crate) async fn get_enabled(&self) -> Result<()> {
        info!("Getting list of enabled coins ...");

        let get_enabled = Command::<i32>::builder()
            .method(Method::GetEnabledCoins)
            .userpass(self.get_rpc_password()?)
            .build();
        request_legacy!(
            get_enabled,
            Mm2RpcResult<GetEnabledResponse>,
            self,
            on_get_enabled_response
        )
    }

    pub(crate) async fn get_orderbook(&self, base: &str, rel: &str, orderbook_config: OrderbookConfig) -> Result<()> {
        info!("Getting orderbook, base: {base}, rel: {rel} ...");

        let get_orderbook = Command::builder()
            .method(Method::GetOrderbook)
            .flatten_data(OrderbookRequest {
                base: base.to_string(),
                rel: rel.to_string(),
            })
            .build();

        request_legacy!(
            get_orderbook,
            OrderbookResponse,
            self,
            on_orderbook_response,
            self.config,
            orderbook_config
        )
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

        let sell = Command::builder()
            .userpass(self.get_rpc_password()?)
            .method(Method::Sell)
            .flatten_data(order)
            .build();
        request_legacy!(sell, Mm2RpcResult<SellBuyResponse>, self, on_sell_response)
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

        let buy = Command::builder()
            .userpass(self.get_rpc_password()?)
            .method(Method::Buy)
            .flatten_data(order)
            .build();
        request_legacy!(buy, Mm2RpcResult<SellBuyResponse>, self, on_buy_response)
    }

    pub(crate) async fn send_stop(&self) -> Result<()> {
        info!("Sending stop command");
        let stop_command = Command::<Dummy>::builder()
            .userpass(self.get_rpc_password()?)
            .method(Method::Stop)
            .build();
        request_legacy!(stop_command, Mm2RpcResult<Status>, self, on_stop_response)
    }

    pub(crate) async fn get_version(self) -> Result<()> {
        info!("Request for mm2 version");
        let get_version = Command::<Dummy>::builder()
            .userpass(self.get_rpc_password()?)
            .method(Method::Version)
            .build();
        request_legacy!(get_version, MmVersionResponse, self, on_version_response)
    }

    fn get_rpc_password(&self) -> Result<String> {
        self.config
            .rpc_password()
            .ok_or_else(|| error_anyhow!("Failed to get rpc_password, not set"))
    }
}
