use anyhow::Result;
use itertools::Itertools;
use std::io::Write;
use term_table::{row::Row,
                 table_cell::{Alignment, TableCell},
                 TableStyle};

use common::{write_safe::io::WriteSafeIO, write_safe_io, writeln_safe_io};
use mm2_rpc::data::legacy::{FilteringOrder, MakerOrderForRpc, Mm2RpcResult, OrderForRpc, OrdersHistoryResponse,
                            UuidParseError};

use super::formatters::{term_table_blank, write_maker_match};
use crate::komodefi_proc::response_handler::formatters::{format_confirmation_settings, format_datetime_msec,
                                                         format_f64, format_historical_changes, format_ratio,
                                                         get_matches_rows, taker_order_header_row, taker_order_rows,
                                                         COMMON_PRECISION};

pub(crate) struct OrdersHistorySettings {
    pub(crate) takers_detailed: bool,
    pub(crate) makers_detailed: bool,
    pub(crate) warnings: bool,
    pub(crate) all: bool,
}

pub(super) fn on_orders_history(
    writer: &mut dyn Write,
    mut response: Mm2RpcResult<OrdersHistoryResponse>,
    settings: OrdersHistorySettings,
) -> Result<()> {
    macro_rules! write_history_filtered_result {
        ($writer: expr, $rows: ident, $header_fn: ident, $legend: literal) => {
            if $rows.is_empty() {
                writeln_safe_io!($writer, concat!($legend, " not found"));
            } else {
                let mut table = term_table_blank(TableStyle::thin(), false, false, false);
                table.add_row($header_fn());
                table.add_row(Row::new(vec![TableCell::new("")]));
                table.rows.extend($rows.drain(..));
                write_safe_io!(
                    $writer,
                    concat!($legend, "\n{}"),
                    table.render().replace('\0', "")
                )
            }
        };
    }

    if settings.all {
        let mut rows: Vec<Row> = response.result.orders.drain(..).map(order_row).try_collect()?;
        write_history_filtered_result!(writer, rows, order_header_row, "Orders history:");
    }

    let mut maker_rows = vec![];
    let mut taker_rows = vec![];
    if settings.makers_detailed || settings.takers_detailed {
        for order in response.result.details.drain(..) {
            match order {
                OrderForRpc::Maker(order) => maker_rows.append(maker_order_rows(&order)?.as_mut()),
                OrderForRpc::Taker(order) => taker_rows.append(taker_order_rows(&order)?.as_mut()),
            }
        }
    }
    if settings.takers_detailed {
        write_history_filtered_result!(
            writer,
            taker_rows,
            taker_order_header_row,
            "Taker orders history detailed:"
        );
    }
    if settings.makers_detailed {
        write_history_filtered_result!(
            writer,
            maker_rows,
            maker_order_header_row,
            "Maker orders history detailed:"
        );
    }
    if settings.warnings {
        let warnings = response.result.warnings.drain(..);
        let mut rows: Vec<Row> = warnings.map(uuid_parse_error_row).collect();
        write_history_filtered_result!(writer, rows, uuid_parse_error_header_row, "Uuid parse errors:");
    }
    Ok(())
}

fn order_header_row() -> Row<'static> {
    Row::new(vec![
        TableCell::new_with_alignment_and_padding("uuid", 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding("Type", 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding("Action", 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding("Base", 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding("Rel", 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding("Volume", 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding("Price", 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding("Status", 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding("Created", 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding("Updated", 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding("Was taker", 1, Alignment::Left, false),
    ])
}

fn order_row(order: FilteringOrder) -> Result<Row<'static>> {
    Ok(Row::new(vec![
        TableCell::new_with_alignment_and_padding(&order.uuid, 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding(&order.order_type, 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding(&order.initial_action, 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding(&order.base, 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding(&order.rel, 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding(
            format_f64(order.volume, COMMON_PRECISION)?,
            1,
            Alignment::Left,
            false,
        ),
        TableCell::new_with_alignment_and_padding(
            format_f64(order.price, COMMON_PRECISION)?,
            1,
            Alignment::Left,
            false,
        ),
        TableCell::new_with_alignment_and_padding(&order.status, 1, Alignment::Left, false),
        TableCell::new_with_alignment_and_padding(
            format_datetime_msec(order.created_at as u64)?,
            1,
            Alignment::Left,
            false,
        ),
        TableCell::new_with_alignment_and_padding(
            format_datetime_msec(order.last_updated as u64)?,
            1,
            Alignment::Left,
            false,
        ),
        TableCell::new_with_alignment_and_padding(order.was_taker != 0, 1, Alignment::Left, false),
    ]))
}

fn uuid_parse_error_header_row() -> Row<'static> { Row::new(vec![TableCell::new("uuid"), TableCell::new("error")]) }

fn uuid_parse_error_row(uuid_parse_error: UuidParseError) -> Row<'static> {
    Row::new(vec![
        TableCell::new(uuid_parse_error.uuid),
        TableCell::new(uuid_parse_error.warning),
    ])
}

fn maker_order_header_row() -> Row<'static> {
    Row::new(vec![
        TableCell::new("base,rel"),
        TableCell::new("price"),
        TableCell::new("uuid"),
        TableCell::new("created at,\nupdated at"),
        TableCell::new("min base vol,\nmax base vol"),
        TableCell::new("swaps"),
        TableCell::new("conf_settings"),
        TableCell::new("history changes"),
        TableCell::new("orderbook ticker\nbase, rel"),
    ])
}

fn maker_order_rows(order: &MakerOrderForRpc) -> Result<Vec<Row<'static>>> {
    let mut rows = vec![Row::new(vec![
        TableCell::new(format!("{},{}", order.base, order.rel)),
        TableCell::new(format_ratio(&order.price_rat, COMMON_PRECISION)?),
        TableCell::new(order.uuid),
        TableCell::new(format!(
            "{},\n{}",
            format_datetime_msec(order.created_at)?,
            order.updated_at.map_or(Ok("".to_string()), format_datetime_msec)?
        )),
        TableCell::new(format!(
            "{},\n{}",
            format_ratio(&order.min_base_vol_rat, COMMON_PRECISION)?,
            format_ratio(&order.max_base_vol_rat, COMMON_PRECISION)?
        )),
        TableCell::new(if order.started_swaps.is_empty() {
            "empty".to_string()
        } else {
            order.started_swaps.iter().join(",\n")
        }),
        TableCell::new(
            order
                .conf_settings
                .as_ref()
                .map_or_else(|| "none".to_string(), format_confirmation_settings),
        ),
        TableCell::new(order.changes_history.as_ref().map_or_else(
            || "none".to_string(),
            |val| {
                val.iter()
                    .map(|val| format_historical_changes(val, "\n").unwrap_or_else(|_| "error".to_string()))
                    .join(",\n")
            },
        )),
        TableCell::new(format!(
            "{}\n{}",
            order
                .base_orderbook_ticker
                .as_ref()
                .map_or_else(|| "none".to_string(), String::clone),
            order
                .rel_orderbook_ticker
                .as_ref()
                .map_or_else(|| "none".to_string(), String::clone)
        )),
    ])];
    rows.append(get_matches_rows(&order.matches, 10, write_maker_match)?.as_mut());
    Ok(rows)
}
