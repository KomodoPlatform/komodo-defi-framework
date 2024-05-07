use anyhow::Result;
use std::io::Write;
use term_table::{row::Row, TableStyle};

use common::{write_safe::io::WriteSafeIO, write_safe_io, writeln_safe_io};

use super::super::formatters::{term_table_blank, writeln_field, ZERO_INDENT};
use super::{format_addr_infos, format_coin_balance, format_token_balances};
use crate::rpc_data::eth::{Erc20InitResult, EthWithTokensActivationResult};

pub(in super::super) fn on_enable_erc20(writer: &mut dyn Write, response: Erc20InitResult) -> Result<()> {
    writeln_field(writer, "platform_coin", response.platform_coin, ZERO_INDENT);
    writeln_field(
        writer,
        "token_contract_address",
        response.token_contract_address,
        ZERO_INDENT,
    );
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

pub(in super::super) fn on_enable_eth_with_tokens(
    writer: &mut dyn Write,
    response: EthWithTokensActivationResult,
) -> Result<()> {
    writeln_field(writer, "current_block", response.current_block, ZERO_INDENT);

    if response.eth_addresses_infos.is_empty() {
        writeln_field(writer, "eth_addresses_infos", "none", ZERO_INDENT);
    } else {
        writeln_field(writer, "eth_addresses_infos", "", ZERO_INDENT);
        let addr_infos = format_addr_infos(response.eth_addresses_infos, format_coin_balance);
        writeln_safe_io!(writer, "{}", addr_infos);
    }

    if response.erc20_addresses_infos.is_empty() {
        writeln_field(writer, "erc20_addresses_infos", "none", ZERO_INDENT);
    } else {
        writeln_field(writer, "erc20_addresses_infos", "", ZERO_INDENT);
        let addr_infos = format_addr_infos(response.erc20_addresses_infos, format_token_balances);
        writeln_safe_io!(writer, "{}", addr_infos);
    }

    Ok(())
}
