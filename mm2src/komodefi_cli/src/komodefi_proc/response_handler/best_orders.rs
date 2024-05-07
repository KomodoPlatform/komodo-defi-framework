use anyhow::Result;
use itertools::Itertools;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use term_table::{row::Row,
                 table_cell::{Alignment, TableCell},
                 TableStyle};

use common::{write_safe::io::WriteSafeIO, write_safe_io};
use mm2_rpc::data::version2::BestOrdersV2Response;

use super::formatters::{format_confirmation_settings, format_ratio, term_table_blank, COMMON_PRECISION};

pub(super) fn on_best_orders(
    writer: &mut dyn Write,
    response: BestOrdersV2Response,
    show_orig_tickers: bool,
) -> Result<()> {
    let mut term_table = term_table_blank(TableStyle::thin(), false, false, false);
    term_table.add_row(best_orders_header_row());

    for (coin, data) in response.orders.iter().sorted_by_key(|p| p.0) {
        let coin = if show_orig_tickers {
            get_original_ticker(coin, &response.original_tickers)
        } else {
            coin.clone()
        };
        term_table.add_row(Row::new(vec![TableCell::new_with_alignment(coin, 7, Alignment::Left)]));
        for order in data.iter().sorted_by_key(|o| o.uuid) {
            term_table.add_row(Row::new(vec![
                TableCell::new(if order.is_mine { "*" } else { "" }),
                TableCell::new(format_ratio(&order.price.rational, COMMON_PRECISION)?),
                TableCell::new(order.uuid),
                TableCell::new(format!(
                    "{}:{}",
                    format_ratio(&order.base_min_volume.rational, COMMON_PRECISION)?,
                    format_ratio(&order.base_max_volume.rational, COMMON_PRECISION)?
                )),
                TableCell::new(format!(
                    "{}:{}",
                    format_ratio(&order.rel_min_volume.rational, COMMON_PRECISION)?,
                    format_ratio(&order.rel_max_volume.rational, COMMON_PRECISION)?
                )),
                TableCell::new(&order.address),
                TableCell::new(
                    &order
                        .conf_settings
                        .as_ref()
                        .map_or_else(|| "none".to_string(), format_confirmation_settings),
                ),
            ]));
        }
    }
    write_safe_io!(writer, "{}", term_table.render());

    Ok(())
}

fn best_orders_header_row() -> Row<'static> {
    Row::new(vec![
        TableCell::new(""),
        TableCell::new("Price"),
        TableCell::new("Uuid"),
        TableCell::new("Base vol(min:max)"),
        TableCell::new("Rel vol(min:max)"),
        TableCell::new("Address"),
        TableCell::new("Confirmation"),
    ])
}

fn get_original_ticker(coin: &String, original_tickers: &HashMap<String, HashSet<String>>) -> String {
    original_tickers
        .get(coin)
        .map_or_else(|| coin.clone(), |set| set.iter().join(", "))
}
