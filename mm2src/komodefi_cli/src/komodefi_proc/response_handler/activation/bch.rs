use anyhow::Result;
use std::io::Write;
use term_table::{row::Row, TableStyle};

use common::{write_safe::io::WriteSafeIO, write_safe_io, writeln_safe_io};

use super::super::formatters::{term_table_blank, writeln_field, ZERO_INDENT};
use super::{format_addr_infos, format_coin_balance, format_token_balances};
use crate::rpc_data::bch::{BchWithTokensActivationResult, SlpInitResult};

pub(in super::super) fn on_enable_bch(writer: &mut dyn Write, response: BchWithTokensActivationResult) -> Result<()> {
    writeln_field(writer, "current_block", response.current_block, ZERO_INDENT);

    if response.bch_addresses_infos.is_empty() {
        writeln_field(writer, "bch_addresses_infos", "none", ZERO_INDENT);
    } else {
        writeln_field(writer, "bch_addresses_infos", "", ZERO_INDENT);
        let addr_infos = format_addr_infos(response.bch_addresses_infos, format_coin_balance);
        writeln_safe_io!(writer, "{}", addr_infos);
    }

    if response.slp_addresses_infos.is_empty() {
        writeln_field(writer, "slp_addresses_infos", "none", ZERO_INDENT);
    } else {
        writeln_field(writer, "slp_addresses_infos", "", ZERO_INDENT);
        let addr_infos = format_addr_infos(response.slp_addresses_infos, format_token_balances);
        writeln_safe_io!(writer, "{}", addr_infos);
    }

    Ok(())
}

pub(in super::super) fn on_enable_slp(writer: &mut dyn Write, response: SlpInitResult) -> Result<()> {
    writeln_field(writer, "platform_coin", response.platform_coin, ZERO_INDENT);
    writeln_field(writer, "token_id", hex::encode(response.token_id.0), ZERO_INDENT);
    writeln_field(
        writer,
        "required_confirmations",
        response.required_confirmations,
        ZERO_INDENT,
    );

    if response.balances.is_empty() {
        writeln_field(writer, "balances(spend:unspend)", "none", ZERO_INDENT);
        return Ok(());
    }
    let mut term_table = term_table_blank(TableStyle::empty(), false, false, false);
    writeln_field(writer, "balances(spend:unspend)", "", ZERO_INDENT);
    for (token, balance) in response.balances {
        term_table.add_row(Row::new(vec![token, format_coin_balance(balance)?]))
    }
    writeln_safe_io!(writer, "{}", term_table.render());
    Ok(())
}
