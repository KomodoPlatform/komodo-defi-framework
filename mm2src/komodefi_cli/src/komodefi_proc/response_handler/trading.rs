use anyhow::Result;
use mm2_rpc::data::legacy::MinTradingVolResponse;
use std::io::Write;

use crate::komodefi_proc::response_handler::formatters::{term_table_blank, writeln_field, ZERO_INDENT};
use common::{write_safe::io::WriteSafeIO, write_safe_io, writeln_safe_io};
use mm2_number::BigRational;
use term_table::row::Row;
use term_table::TableStyle;

use super::formatters::{format_ratio, write_field_option, COMMON_PRECISION};
use crate::rpc_data::{MakerPreimage, MaxTakerVolResponse, TakerPreimage, TotalTradeFeeResponse, TradeFeeResponse,
                      TradePreimageResponse};

pub(super) fn on_min_trading_vol(writer: &mut dyn Write, response: MinTradingVolResponse) -> Result<()> {
    writeln_field(writer, "coin", response.coin, 0);
    writeln_field(
        writer,
        "volume",
        format_ratio(&response.volume.min_trading_vol, COMMON_PRECISION)?,
        0,
    );
    Ok(())
}

pub(super) fn on_max_taker_vol(writer: &mut dyn Write, response: MaxTakerVolResponse) -> Result<()> {
    writeln_field(writer, "coin", response.coin, 0);
    writeln_field(
        writer,
        "result",
        format_ratio(&BigRational::from(response.result), COMMON_PRECISION)?,
        0,
    );
    Ok(())
}

pub(super) fn on_trade_preimage(writer: &mut dyn Write, response: TradePreimageResponse) -> Result<()> {
    match response {
        TradePreimageResponse::TakerPreimage(taker_preimage) => write_taker_preimage(writer, taker_preimage),
        TradePreimageResponse::MakerPreimage(maker_preimage) => write_maker_preimage(writer, maker_preimage),
    }
}

fn write_taker_preimage(writer: &mut dyn Write, preimage: TakerPreimage) -> Result<()> {
    writeln_field(
        writer,
        "base_coin_fee",
        format_trade_coin_fee(preimage.base_coin_fee)?,
        ZERO_INDENT,
    );
    writeln_field(
        writer,
        "rel_coin_fee",
        format_trade_coin_fee(preimage.rel_coin_fee)?,
        ZERO_INDENT,
    );
    writeln_field(
        writer,
        "taker_fee",
        format_trade_coin_fee(preimage.taker_fee)?,
        ZERO_INDENT,
    );
    writeln_field(
        writer,
        "fee_to_send_taker_fee",
        format_trade_coin_fee(preimage.fee_to_send_taker_fee)?,
        ZERO_INDENT,
    );
    writeln_field(writer, "total_fee", "", ZERO_INDENT);
    write_total_trade_fee(writer, preimage.total_fees)
}

fn write_maker_preimage(writer: &mut dyn Write, preimage: MakerPreimage) -> Result<()> {
    writeln_field(
        writer,
        "base_coin_fee",
        format_trade_coin_fee(preimage.base_coin_fee)?,
        ZERO_INDENT,
    );
    writeln_field(
        writer,
        "rel_coin_fee",
        format_trade_coin_fee(preimage.rel_coin_fee)?,
        ZERO_INDENT,
    );
    write_field_option(
        writer,
        "volume",
        preimage
            .volume
            .map(|v| format_ratio(&v.volume, COMMON_PRECISION).unwrap_or_else(|_| "error".to_string())),
        ZERO_INDENT,
    );
    writeln_field(writer, "total_fee", "", ZERO_INDENT);
    write_total_trade_fee(writer, preimage.total_fees)
}

fn format_trade_coin_fee(trade_coin_fee: TradeFeeResponse) -> Result<String> {
    Ok(format!(
        "coin: {}, amount: {}, paid_from_trading_vol: {}",
        trade_coin_fee.coin,
        format_ratio(&trade_coin_fee.amount.amount, COMMON_PRECISION)?,
        trade_coin_fee.paid_from_trading_vol
    ))
}

fn write_total_trade_fee(writer: &mut dyn Write, total_fee: Vec<TotalTradeFeeResponse>) -> Result<()> {
    if total_fee.is_empty() {
        writeln_field(writer, "total_fee", "empty", ZERO_INDENT);
        return Ok(());
    }

    let mut term_table = term_table_blank(TableStyle::thin(), false, false, false);
    term_table.add_row(Row::new(vec!["coin", "amount", "required_balance"]));
    for fee in total_fee {
        term_table.add_row(Row::new(vec![
            fee.coin,
            format_ratio(&fee.amount.amount, COMMON_PRECISION)?,
            format_ratio(&fee.required_balance.required_balance, COMMON_PRECISION)?,
        ]));
    }
    writeln_safe_io!(writer, "{}", term_table.render());
    Ok(())
}
