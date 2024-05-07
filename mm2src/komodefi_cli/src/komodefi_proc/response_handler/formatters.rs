use anyhow::{anyhow, Result};
use chrono::{TimeZone, Utc};
use itertools::Itertools;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::io::Write;
use term_table::{row::Row, table_cell::TableCell, Table as TermTable, TableStyle};
use uuid::Uuid;

use common::log::error;
use common::{write_safe::io::WriteSafeIO, write_safe_io, writeln_safe_io};
use mm2_number::bigdecimal::{ToPrimitive, Zero};
use mm2_rpc::data::legacy::{HistoricalOrder, MakerMatchForRpc, MakerOrderForRpc, MakerReservedForRpc, MatchBy,
                            OrderConfirmationsSettings, TakerMatchForRpc, TakerOrderForRpc};
use rpc::v1::types::Bytes;

use super::super::SmartFractPrecision;
use super::macros::{write_base_rel, write_confirmation_settings, write_connected};
use super::smart_fraction_fmt::SmartFractionFmt;
use crate::logging::error_anyhow;

pub(super) const COMMON_INDENT: usize = 20;
pub(super) const COMMON_PRECISION: SmartFractPrecision = (2, 5);
const NESTED_INDENT: usize = 26;
pub(super) const ZERO_INDENT: usize = 0;

pub(super) fn on_maker_order_response(writer: &mut dyn Write, order: MakerOrderForRpc) -> Result<()> {
    writeln_field(writer, "Maker order", "", ZERO_INDENT);
    write_maker_order(writer, &order)?;
    write_maker_matches(writer, &order.matches)?;
    writeln_safe_io!(writer, "");
    Ok(())
}

pub(super) fn write_maker_order(writer: &mut dyn Write, order: &MakerOrderForRpc) -> Result<()> {
    writeln_field(writer, "base", &order.base, COMMON_INDENT);
    writeln_field(writer, "rel", &order.rel, COMMON_INDENT);
    writeln_field(
        writer,
        "price",
        format_ratio(&order.price_rat, COMMON_PRECISION)?,
        COMMON_INDENT,
    );
    writeln_field(writer, "uuid", order.uuid, COMMON_INDENT);
    writeln_field(
        writer,
        "created at",
        format_datetime_msec(order.created_at)?,
        COMMON_INDENT,
    );
    if let Some(updated_at) = order.updated_at {
        writeln_field(writer, "updated at", format_datetime_msec(updated_at)?, COMMON_INDENT);
    }
    writeln_field(
        writer,
        "max_base_vol",
        format_ratio(&order.max_base_vol_rat, COMMON_PRECISION)?,
        COMMON_INDENT,
    );
    writeln_field(
        writer,
        "min_base_vol",
        format_ratio(&order.min_base_vol_rat, COMMON_PRECISION)?,
        COMMON_INDENT,
    );
    writeln_field(
        writer,
        "swaps",
        if order.started_swaps.is_empty() {
            "empty".to_string()
        } else {
            order.started_swaps.iter().join(", ")
        },
        COMMON_INDENT,
    );
    if let Some(ref conf_settings) = order.conf_settings {
        writeln_field(
            writer,
            "conf_settings",
            format_confirmation_settings(conf_settings),
            COMMON_INDENT,
        );
    }
    if let Some(ref changes_history) = order.changes_history {
        writeln_field(
            writer,
            "changes_history",
            changes_history
                .iter()
                .map(|val| format_historical_changes(val, ", ").unwrap_or_else(|_| "error".to_string()))
                .join("; "),
            COMMON_INDENT,
        );
    }
    Ok(())
}

pub(super) fn write_maker_matches(writer: &mut dyn Write, matches: &HashMap<Uuid, MakerMatchForRpc>) -> Result<()> {
    if matches.is_empty() {
        return Ok(());
    }
    writeln_field(writer, "matches", "", COMMON_INDENT);
    for (uuid, m) in matches {
        write_maker_match(writer, uuid, m)?;
        writeln_safe_io!(writer, "");
    }
    Ok(())
}

pub(super) fn write_maker_match(writer: &mut dyn Write, uuid: &Uuid, m: &MakerMatchForRpc) -> Result<()> {
    let (req, reserved, connect, connected) = (&m.request, &m.reserved, &m.connect, &m.connected);
    writeln_field(writer, "uuid", uuid, NESTED_INDENT);
    writeln_field(writer, "req.uuid", req.uuid, NESTED_INDENT);
    write_base_rel!(writer, req, NESTED_INDENT);
    writeln_field(
        writer,
        "req.match_by",
        format_match_by(&req.match_by, ", "),
        NESTED_INDENT,
    );
    writeln_field(writer, "req.action", &req.action, NESTED_INDENT);
    write_confirmation_settings!(writer, req, NESTED_INDENT);
    writeln_field(
        writer,
        "req.(sender, dest)",
        format!("{},{}", req.sender_pubkey, req.dest_pub_key),
        NESTED_INDENT,
    );
    write_maker_reserved_for_rpc(writer, reserved)?;
    if let Some(ref connected) = connected {
        write_connected!(writer, connected, NESTED_INDENT);
    }
    if let Some(ref connect) = connect {
        write_connected!(writer, connect, NESTED_INDENT);
    }
    writeln_field(
        writer,
        "last_updated",
        format_datetime_msec(m.last_updated)?,
        NESTED_INDENT,
    );
    Ok(())
}

fn write_maker_reserved_for_rpc(writer: &mut dyn Write, reserved: &MakerReservedForRpc) -> Result<()> {
    write_base_rel!(writer, reserved, NESTED_INDENT);
    writeln_field(
        writer,
        "reserved.(taker, maker)",
        format!("{},{}", reserved.taker_order_uuid, reserved.maker_order_uuid),
        NESTED_INDENT,
    );
    writeln_field(
        writer,
        "reserved.(sender, dest)",
        format!("{},{}", reserved.sender_pubkey, reserved.dest_pub_key),
        NESTED_INDENT,
    );
    write_confirmation_settings!(writer, reserved, NESTED_INDENT);
    Ok(())
}

pub(super) fn taker_order_header_row() -> Row<'static> {
    Row::new(vec![
        TableCell::new("action\nbase(vol),rel(vol)"),
        TableCell::new("uuid, sender, dest"),
        TableCell::new("type,created_at\nconfirmation"),
        TableCell::new("match_by"),
        TableCell::new("base,rel\norderbook ticker"),
        TableCell::new("cancellable"),
    ])
}

pub(super) fn taker_order_rows(taker_order: &TakerOrderForRpc) -> Result<Vec<Row<'static>>> {
    let req = &taker_order.request;
    let mut rows = vec![Row::new(vec![
        TableCell::new(format!(
            "{}\n{}({}),{}({})",
            req.action,
            req.base,
            format_ratio(&req.base_amount, COMMON_PRECISION)?,
            req.rel,
            format_ratio(&req.rel_amount, COMMON_PRECISION)?
        )),
        TableCell::new(format!("{}\n{}\n{}", req.uuid, req.sender_pubkey, req.dest_pub_key)),
        TableCell::new(format!(
            "{}\n{}\n{}",
            taker_order.order_type,
            format_datetime_msec(taker_order.created_at)?,
            req.conf_settings
                .as_ref()
                .map_or_else(|| "none".to_string(), format_confirmation_settings),
        )),
        TableCell::new(format_match_by(&req.match_by, "\n")),
        TableCell::new(format!(
            "{}\n{}",
            taker_order
                .base_orderbook_ticker
                .as_ref()
                .map_or_else(|| "none".to_string(), String::clone),
            taker_order
                .rel_orderbook_ticker
                .as_ref()
                .map_or_else(|| "none".to_string(), String::clone)
        )),
        TableCell::new(taker_order.cancellable),
    ])];
    rows.append(get_matches_rows(&taker_order.matches, 6, write_taker_match)?.as_mut());
    Ok(rows)
}

pub(super) fn get_matches_rows<M, F: Fn(&mut dyn Write, &Uuid, &M) -> Result<()>>(
    matches: &HashMap<Uuid, M>,
    collspan: usize,
    write_match: F,
) -> Result<Vec<Row<'static>>> {
    let mut rows = vec![];
    if matches.is_empty() {
        return Ok(rows);
    }
    rows.push(Row::new(vec![TableCell::new_with_col_span("matches", collspan)]));
    for (uuid, m) in matches.iter() {
        let mut buff = vec![];
        write_match(&mut buff as &mut dyn Write, uuid, m)?;
        rows.push(Row::new(vec![TableCell::new_with_col_span(
            String::from_utf8(buff)
                .map_err(|error| error_anyhow!("Failed to get string from taker order matches_str buffer: {error}"))?,
            collspan,
        )]));
    }
    Ok(rows)
}

pub(super) fn write_taker_match(writer: &mut dyn Write, uuid: &Uuid, m: &TakerMatchForRpc) -> Result<()> {
    let (reserved, connect, connected) = (&m.reserved, &m.connect, &m.connected);
    writeln_field(writer, "uuid", uuid, NESTED_INDENT);
    write_maker_reserved_for_rpc(writer, reserved)?;
    let last_updated = if m.last_updated.is_zero() {
        "none".to_string()
    } else {
        format_datetime_msec(m.last_updated)?
    };
    writeln_field(writer, "last_updated", last_updated, NESTED_INDENT);
    write_connected!(writer, connect, NESTED_INDENT);
    if let Some(ref connected) = connected {
        write_connected!(writer, connected, NESTED_INDENT);
    }
    Ok(())
}

pub(super) fn format_historical_changes(historical_order: &HistoricalOrder, delimiter: &str) -> Result<String> {
    let mut result = vec![];

    if let Some(ref min_base_vol) = historical_order.min_base_vol {
        result.push(format!(
            "min_base_vol: {}",
            format_ratio(min_base_vol, COMMON_PRECISION)?,
        ))
    }
    if let Some(ref max_base_vol) = historical_order.max_base_vol {
        result.push(format!(
            "max_base_vol: {}",
            format_ratio(max_base_vol, COMMON_PRECISION)?,
        ))
    }
    if let Some(ref price) = historical_order.price {
        result.push(format!("price: {}", format_ratio(price, COMMON_PRECISION)?));
    }
    if let Some(updated_at) = historical_order.updated_at {
        result.push(format!("updated_at: {}", format_datetime_msec(updated_at)?));
    }
    if let Some(ref conf_settings) = historical_order.conf_settings {
        result.push(format!(
            "conf_settings: {}",
            format_confirmation_settings(conf_settings),
        ));
    }
    Ok(result.join(delimiter))
}

pub(super) fn term_table_blank(
    style: TableStyle,
    sep_row: bool,
    bottom_border: bool,
    top_border: bool,
) -> TermTable<'static> {
    let mut term_table = TermTable::new();
    term_table.style = style;
    term_table.separate_rows = sep_row;
    term_table.has_bottom_boarder = bottom_border;
    term_table.has_top_boarder = top_border;
    term_table
}

pub(super) fn format_match_by(match_by: &MatchBy, delimiter: &str) -> String {
    match match_by {
        MatchBy::Any => "Any".to_string(),
        MatchBy::Orders(orders) => orders.iter().sorted().join(delimiter),
        MatchBy::Pubkeys(pubkeys) => pubkeys.iter().sorted().join(delimiter),
    }
}

pub(super) fn format_datetime_msec(datetime_msec: u64) -> Result<String> { format_datetime_sec(datetime_msec / 1000) }

pub(super) fn format_datetime_sec(datetime: u64) -> Result<String> {
    let datetime = Utc
        .timestamp_opt(datetime as i64, 0)
        .single()
        .ok_or_else(|| error_anyhow!("Failed to get datetime formatted datetime"))?;
    Ok(format!("{}", datetime.format("%y-%m-%d %H:%M:%S")))
}

pub(super) fn format_ratio<T: ToPrimitive + Debug>(rational: &T, precision: SmartFractPrecision) -> Result<String> {
    format_f64(
        rational
            .to_f64()
            .ok_or_else(|| error_anyhow!("Failed to cast rational to f64: {rational:?}"))?,
        precision,
    )
}

pub(super) fn format_f64(rational: f64, fract_precision: SmartFractPrecision) -> Result<String> {
    Ok(SmartFractionFmt::new(&fract_precision, rational)
        .map_err(|_| error_anyhow!("Failed to create smart_fraction_fmt"))?
        .to_string())
}

pub(super) fn format_confirmation_settings(settings: &OrderConfirmationsSettings) -> String {
    format!(
        "{},{}:{},{}",
        settings.base_confs, settings.base_nota, settings.rel_confs, settings.rel_nota
    )
}

pub(super) fn write_sequence<T: Display, I: ExactSizeIterator<Item = T> + Itertools>(
    writer: &mut dyn Write,
    header: &str,
    mut iter: I,
    indent: usize,
) {
    writeln_field(
        writer,
        header,
        if iter.len() == 0 {
            "none".to_string()
        } else {
            iter.join(", ")
        },
        indent,
    )
}

pub(super) fn writeln_field<H: Display, T: Display>(writer: &mut dyn Write, header: H, value: T, indent: usize) {
    writeln_safe_io!(writer, "{:>width$}: {}", header, value, width = indent)
}

pub(super) fn write_field_option<H: Display, T: Display>(
    writer: &mut dyn Write,
    header: H,
    value: Option<T>,
    indent: usize,
) {
    if let Some(ref value) = value {
        writeln_safe_io!(writer, "{:>width$}: {}", header, value, width = indent)
    }
}

pub(super) fn format_bytes(bytes: Bytes) -> String { hex::encode(bytes.as_slice()) }
