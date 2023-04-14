use cli_table::format::{Border, Justify, Separator};
use cli_table::{print_stdout, Cell, Table, WithTitle};
use itertools::Itertools;
use log::{error, info};
use mm2_number::bigdecimal::ToPrimitive;
use mm2_rpc::mm_protocol::{OrderbookEntryAggregate, OrderbookResponse};
use serde_json::Value as Json;
use std::cmp::Ordering;
use std::fmt::{Debug, Display};

use super::smart_fraction_fmt::SmartFractionFmt;
use super::GetEnabledResponse;
use crate::adex_config::AdexConfig;

type PricePrecision = (usize, usize);
type VolumePrecision = (usize, usize);
type AskBidRowInput<'a, 'b, 'c> = (&'a OrderbookEntryAggregate, &'b VolumePrecision, &'c PricePrecision);

pub trait ResponseHandler {
    fn print_response(&self, response: Json) -> Result<(), ()>;
    fn display_response<T: Display + 'static>(&self, response: &T) -> Result<(), ()>;
    fn debug_response<T: Debug + 'static>(&self, response: &T) -> Result<(), ()>;
    fn on_orderbook_response<Cfg: AdexConfig + 'static>(
        &self,
        orderbook: OrderbookResponse,
        config: &Cfg,
        asks_limit: &Option<usize>,
        bids_limit: &Option<usize>,
    ) -> Result<(), ()>;
    fn on_get_enabled_response(&self, enabled: &GetEnabledResponse) -> Result<(), ()>;
}

pub struct ResponseHandlerImpl {}

impl ResponseHandler for ResponseHandlerImpl {
    fn print_response(&self, result: Json) -> Result<(), ()> { print_result_as_table(result) }

    fn display_response<T: Display + 'static>(&self, result: &T) -> Result<(), ()> {
        info!("{result}");
        Ok(())
    }

    fn debug_response<T: Debug + 'static>(&self, response: &T) -> Result<(), ()> {
        info!("{response:?}");
        Ok(())
    }

    fn on_orderbook_response<Cfg: AdexConfig + 'static>(
        &self,
        orderbook: OrderbookResponse,
        config: &Cfg,
        asks_limit: &Option<usize>,
        bids_limit: &Option<usize>,
    ) -> Result<(), ()> {
        let price_prec = config.orderbook_price_precision();
        let vol_prec = config.orderbook_volume_precision();

        let mut table: Vec<AskBidRow> = vec![];

        if orderbook.asks.is_empty() {
            table.push(AskBidRow::new("", "No asks found"));
        } else {
            let skip = orderbook
                .asks
                .len()
                .checked_sub(asks_limit.unwrap_or(usize::MAX))
                .unwrap_or_default();

            table.extend(
                orderbook
                    .asks
                    .iter()
                    .sorted_by(cmp_asks)
                    .skip(skip)
                    .map(|entry| (entry, vol_prec, price_prec).into()),
            );
        }
        table.push(AskBidRow::new("---------", "---------"));

        if orderbook.bids.is_empty() {
            table.push(AskBidRow::new("", "No bids found"));
        } else {
            table.extend(
                orderbook
                    .bids
                    .iter()
                    .sorted_by(cmp_bids)
                    .take(bids_limit.unwrap_or(usize::MAX))
                    .map(|entry| (entry, vol_prec, price_prec).into()),
            );
        }

        let base_vol_head = "Volume: ".to_string() + &orderbook.base;
        let rel_price_head = "Price: ".to_string() + &orderbook.rel;

        let title = vec![base_vol_head.cell().justify(Justify::Right), rel_price_head.cell()];
        print_stdout(
            table
                .with_title()
                .title(title)
                .border(Border::builder().build())
                .separator(Separator::builder().build()),
        )
        .map_err(|error| error!("Failed to print result: {error}"))
    }

    fn on_get_enabled_response(&self, enabled: &GetEnabledResponse) -> Result<(), ()> {
        if enabled.result.is_empty() {
            info!("Enabled coins list is empty");
            return Ok(());
        }
        print_stdout(enabled.result.with_title()).map_err(|error| error!("Failed to print result: {error}"))
    }
}

fn cmp_bids(left: &&OrderbookEntryAggregate, right: &&OrderbookEntryAggregate) -> Ordering {
    let cmp = left.price.cmp(&right.price).reverse();
    if cmp.is_eq() {
        return left.base_max_volume.cmp(&right.base_max_volume).reverse();
    }
    cmp
}

fn cmp_asks(left: &&OrderbookEntryAggregate, right: &&OrderbookEntryAggregate) -> Ordering {
    let cmp = left.price.cmp(&right.price).reverse();
    if cmp.is_eq() {
        return left.base_max_volume.cmp(&right.base_max_volume);
    }
    cmp
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
struct AskBidRow {
    #[table(justify = "Justify::Right")]
    volume: String,
    price: String,
}

impl AskBidRow {
    fn new(volume: &str, price: &str) -> Self {
        Self {
            volume: volume.into(),
            price: price.into(),
        }
    }
}

impl From<AskBidRowInput<'_, '_, '_>> for AskBidRow {
    fn from(input: AskBidRowInput) -> Self {
        let entry = input.0;
        let vol_prec = input.1;
        let price_prec = input.2;
        AskBidRow {
            volume: SmartFractionFmt::new(vol_prec.0, vol_prec.1, entry.base_max_volume.to_f64().unwrap())
                .unwrap()
                .to_string(),
            price: SmartFractionFmt::new(price_prec.0, price_prec.1, entry.price.to_f64().unwrap())
                .unwrap()
                .to_string(),
        }
    }
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
