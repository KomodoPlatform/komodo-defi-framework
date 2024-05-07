use anyhow::Result;
use itertools::Itertools;
use std::io::Write;
use term_table::{row::Row, TableStyle};

use common::{write_safe::io::WriteSafeIO, write_safe_io, writeln_safe_io};

use super::super::formatters::{term_table_blank, write_field_option, writeln_field, ZERO_INDENT};
use super::{format_coin_balance, format_token_balances};
use crate::rpc_data::tendermint::{TendermintActivationResult, TendermintTokenInitResult};

pub(in super::super) fn on_enable_tendermint(
    writer: &mut dyn Write,
    response: TendermintActivationResult,
) -> Result<()> {
    writeln_field(writer, "ticker", response.ticker, ZERO_INDENT);
    writeln_field(writer, "address", response.address, ZERO_INDENT);
    writeln_field(writer, "current_block", response.current_block, ZERO_INDENT);
    write_field_option(
        writer,
        "balance",
        response
            .balance
            .map(format_coin_balance)
            .map(|result| result.unwrap_or_else(|_| "error".to_string())),
        ZERO_INDENT,
    );

    write_field_option(
        writer,
        "token_balances",
        response
            .tokens_balances
            .map(format_token_balances)
            .map(|result| result.unwrap_or_else(|_| "error".to_string())),
        ZERO_INDENT,
    );

    write_field_option(
        writer,
        "tokens_tickers",
        response.tokens_tickers.map(|tickers| tickers.iter().join(", ")),
        ZERO_INDENT,
    );
    Ok(())
}

pub(in super::super) fn on_enable_tendermint_token(
    writer: &mut dyn Write,
    response: TendermintTokenInitResult,
) -> Result<()> {
    writeln_field(writer, "platform_coin", response.platform_coin, ZERO_INDENT);

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
