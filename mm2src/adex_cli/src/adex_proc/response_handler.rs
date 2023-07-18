#[path = "response_handler/orderbook.rs"] mod orderbook;
#[path = "response_handler/smart_fraction_fmt.rs"]
mod smart_fraction_fmt;

pub(crate) use smart_fraction_fmt::SmartFractPrecision;

use anyhow::{anyhow, Result};
use itertools::Itertools;
use log::{error, info};
use mm2_rpc::data::legacy::{BalanceResponse, CoinInitResponse, GetEnabledResponse, Mm2RpcResult, MmVersionResponse,
                            OrderbookResponse, SellBuyResponse, Status};
use serde_json::Value as Json;
use std::cell::RefCell;
use std::fmt::Debug;
use std::io::Write;

use super::OrderbookConfig;
use crate::adex_config::AdexConfig;
use crate::error_anyhow;
use common::{write_safe::io::WriteSafeIO, write_safe_io, writeln_safe_io};

pub(crate) trait ResponseHandler {
    fn print_response(&self, response: Json) -> Result<()>;
    fn debug_response<T: Debug + 'static>(&self, response: &T) -> Result<()>;
    fn on_orderbook_response<Cfg: AdexConfig + 'static>(
        &self,
        orderbook: &OrderbookResponse,
        config: &Cfg,
        orderbook_config: OrderbookConfig,
    ) -> Result<()>;
    fn on_get_enabled_response(&self, enabled: &Mm2RpcResult<GetEnabledResponse>) -> Result<()>;
    fn on_version_response(&self, response: &MmVersionResponse) -> Result<()>;
    fn on_enable_response(&self, response: &CoinInitResponse) -> Result<()>;
    fn on_balance_response(&self, response: &BalanceResponse) -> Result<()>;
    fn on_sell_response(&self, response: &Mm2RpcResult<SellBuyResponse>) -> Result<()>;
    fn on_buy_response(&self, response: &Mm2RpcResult<SellBuyResponse>) -> Result<()>;
    fn on_stop_response(&self, response: &Mm2RpcResult<Status>) -> Result<()>;
}

pub(crate) struct ResponseHandlerImpl<'a> {
    pub(crate) writer: RefCell<&'a mut dyn Write>,
}

impl ResponseHandler for ResponseHandlerImpl<'_> {
    fn print_response(&self, result: Json) -> Result<()> {
        let object = result
            .as_object()
            .ok_or_else(|| error_anyhow!("Failed to cast result as object"))?;

        object
            .iter()
            .map(SimpleCliTable::from_pair)
            .for_each(|value| writeln_safe_io!(self.writer.borrow_mut(), "{}: {:?}", value.key, value.value));
        Ok(())
    }

    fn debug_response<T: Debug + 'static>(&self, response: &T) -> Result<()> {
        info!("{response:?}");
        Ok(())
    }

    fn on_orderbook_response<Cfg: AdexConfig + 'static>(
        &self,
        orderbook: &OrderbookResponse,
        config: &Cfg,
        orderbook_config: OrderbookConfig,
    ) -> Result<()> {
        let mut writer = self.writer.borrow_mut();

        let base_vol_head = format!("Volume: {}", orderbook.base);
        let rel_price_head = format!("Price: {}", orderbook.rel);
        writeln_safe_io!(
            writer,
            "{}",
            orderbook::AskBidRow::new(
                base_vol_head.as_str(),
                rel_price_head.as_str(),
                "Uuid",
                "Min volume",
                "Max volume",
                "Age(sec.)",
                "Public",
                "Address",
                "Order conf (bc,bn:rc,rn)",
                &orderbook_config
            )
        );

        let price_prec = config.orderbook_price_precision();
        let vol_prec = config.orderbook_volume_precision();

        if orderbook.asks.is_empty() {
            writeln_safe_io!(
                writer,
                "{}",
                orderbook::AskBidRow::new("", "No asks found", "", "", "", "", "", "", "", &orderbook_config)
            );
        } else {
            let skip = orderbook
                .asks
                .len()
                .checked_sub(orderbook_config.asks_limit.unwrap_or(usize::MAX))
                .unwrap_or_default();

            orderbook
                .asks
                .iter()
                .sorted_by(orderbook::cmp_asks)
                .skip(skip)
                .map(|entry| orderbook::AskBidRow::from_orderbook_entry(entry, vol_prec, price_prec, &orderbook_config))
                .for_each(|row: orderbook::AskBidRow| writeln_safe_io!(writer, "{}", row));
        }
        writeln_safe_io!(writer, "{}", orderbook::AskBidRow::new_delimiter(&orderbook_config));

        if orderbook.bids.is_empty() {
            writeln_safe_io!(
                writer,
                "{}",
                orderbook::AskBidRow::new("", "No bids found", "", "", "", "", "", "", "", &orderbook_config)
            );
        } else {
            orderbook
                .bids
                .iter()
                .sorted_by(orderbook::cmp_bids)
                .take(orderbook_config.bids_limit.unwrap_or(usize::MAX))
                .map(|entry| orderbook::AskBidRow::from_orderbook_entry(entry, vol_prec, price_prec, &orderbook_config))
                .for_each(|row: orderbook::AskBidRow| writeln_safe_io!(writer, "{}", row));
        }
        Ok(())
    }

    fn on_get_enabled_response(&self, enabled: &Mm2RpcResult<GetEnabledResponse>) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        writeln_safe_io!(writer, "{:8} {}", "Ticker", "Address");
        for row in &enabled.result {
            writeln_safe_io!(writer, "{:8} {}", row.ticker, row.address);
        }
        Ok(())
    }

    fn on_version_response(&self, response: &MmVersionResponse) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        writeln_safe_io!(writer, "Version: {}", response.result);
        writeln_safe_io!(writer, "Datetime: {}", response.datetime);
        Ok(())
    }

    fn on_enable_response(&self, response: &CoinInitResponse) -> Result<()> {
        let mut writer = self.writer.borrow_mut();
        writeln_safe_io!(
            writer,
            "coin: {}\naddress: {}\nbalance: {}\nunspendable_balance: {}\nrequired_confirmations: {}\nrequires_notarization: {}",
            response.coin,
            response.address,
            response.balance,
            response.unspendable_balance,
            response.required_confirmations,
            if response.requires_notarization { "Yes" } else { "No" }
        );
        if let Some(mature_confirmations) = response.mature_confirmations {
            writeln_safe_io!(writer, "mature_confirmations: {}", mature_confirmations);
        }
        Ok(())
    }

    fn on_balance_response(&self, response: &BalanceResponse) -> Result<()> {
        writeln_safe_io!(
            self.writer.borrow_mut(),
            "coin: {}\nbalance: {}\nunspendable: {}\naddress: {}",
            response.coin,
            response.balance,
            response.unspendable_balance,
            response.address
        );
        Ok(())
    }

    fn on_sell_response(&self, response: &Mm2RpcResult<SellBuyResponse>) -> Result<()> {
        writeln_safe_io!(self.writer.borrow_mut(), "Order uuid: {}", response.request.uuid);
        Ok(())
    }

    fn on_buy_response(&self, response: &Mm2RpcResult<SellBuyResponse>) -> Result<()> {
        writeln_safe_io!(self.writer.borrow_mut(), "Buy order uuid: {}", response.request.uuid);
        Ok(())
    }

    fn on_stop_response(&self, response: &Mm2RpcResult<Status>) -> Result<()> {
        writeln_safe_io!(self.writer.borrow_mut(), "Service stopped: {}", response.result);
        Ok(())
    }
}

struct SimpleCliTable<'a> {
    key: &'a String,
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
