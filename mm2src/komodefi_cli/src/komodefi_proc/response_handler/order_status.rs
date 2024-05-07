use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::io::Write;
use uuid::Uuid;

use common::log::error;
use common::{write_safe::io::WriteSafeIO, write_safe_io, writeln_safe_io};

use mm2_rpc::data::legacy::{MakerOrderForMyOrdersRpc, OrderStatusResponse, TakerMatchForRpc, TakerOrderForRpc};

use super::formatters::{format_confirmation_settings, format_datetime_msec, format_match_by, format_ratio,
                        write_field_option, write_maker_matches, write_maker_order, write_taker_match, writeln_field,
                        COMMON_INDENT, COMMON_PRECISION};
use super::macros::{write_base_rel, write_confirmation_settings};
use crate::error_anyhow;

pub(super) fn on_order_status(writer: &mut dyn Write, response: OrderStatusResponse) -> Result<()> {
    match response {
        OrderStatusResponse::Maker(maker_status) => write_maker_order_for_my_orders(writer, &maker_status),
        OrderStatusResponse::Taker(taker_status) => write_taker_order(writer, &taker_status),
    }
}

fn write_maker_order_for_my_orders(writer: &mut dyn Write, maker_status: &MakerOrderForMyOrdersRpc) -> Result<()> {
    let order = &maker_status.order;
    write_maker_order(writer, order)?;
    writeln_field(writer, "cancellable", maker_status.cancellable, COMMON_INDENT);
    writeln_field(
        writer,
        "available_amount",
        format_ratio(&maker_status.available_amount, COMMON_PRECISION)?,
        COMMON_INDENT,
    );
    write_maker_matches(writer, &order.matches)?;
    writeln_safe_io!(writer, "");
    Ok(())
}

fn write_taker_order(writer: &mut dyn Write, taker_status: &TakerOrderForRpc) -> Result<()> {
    let req = &taker_status.request;
    writeln_field(writer, "uuid", req.uuid, COMMON_INDENT);
    write_base_rel!(writer, req, COMMON_INDENT);
    writeln_field(writer, "req.action", &req.action, COMMON_INDENT);
    writeln_field(
        writer,
        "req.(sender, dest)",
        format!("{}, {}", req.sender_pubkey, req.dest_pub_key),
        COMMON_INDENT,
    );
    writeln_field(
        writer,
        "req.match_by",
        format_match_by(&req.match_by, ", "),
        COMMON_INDENT,
    );
    write_confirmation_settings!(writer, req, COMMON_INDENT);
    writeln_field(
        writer,
        "created_at",
        format_datetime_msec(taker_status.created_at)?,
        COMMON_INDENT,
    );
    writeln_field(writer, "order_type", &taker_status.order_type, COMMON_INDENT);
    writeln_field(writer, "cancellable", taker_status.cancellable, COMMON_INDENT);
    write_field_option(
        writer,
        "base_ob_ticker",
        taker_status.base_orderbook_ticker.as_ref(),
        COMMON_INDENT,
    );
    write_field_option(
        writer,
        "rel_ob_ticker",
        taker_status.rel_orderbook_ticker.as_ref(),
        COMMON_INDENT,
    );
    write_taker_matches(writer, &taker_status.matches)?;
    Ok(())
}

fn write_taker_matches(writer: &mut dyn Write, matches: &HashMap<Uuid, TakerMatchForRpc>) -> Result<()> {
    if matches.is_empty() {
        return Ok(());
    }
    writeln_field(writer, "matches", "", COMMON_INDENT);
    for (uuid, m) in matches {
        write_taker_match(writer, uuid, m)?;
    }
    Ok(())
}
