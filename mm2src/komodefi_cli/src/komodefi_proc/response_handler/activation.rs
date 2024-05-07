#[path = "activation/bch.rs"] pub(crate) mod bch;
#[path = "activation/eth.rs"] pub(crate) mod eth;
#[path = "activation/tendermint.rs"] pub(crate) mod tendermint;
#[path = "activation/z_coin.rs"] pub(crate) mod z_coin;

pub(super) use bch::{on_enable_bch, on_enable_slp};
pub(super) use eth::{on_enable_erc20, on_enable_eth_with_tokens};
pub(super) use tendermint::{on_enable_tendermint, on_enable_tendermint_token};
pub(super) use z_coin::on_enable_zcoin;

use anyhow::{anyhow, Result};
use itertools::Itertools;
use std::collections::HashMap;
use std::io::Write;
use term_table::{row::Row, TableStyle};

use common::log::error;

use super::formatters::{format_ratio, term_table_blank, write_sequence, writeln_field, COMMON_PRECISION, ZERO_INDENT};
use crate::error_anyhow;
use crate::rpc_data::activation::{CoinAddressInfo, CoinBalance, TokenBalances};
use crate::rpc_data::{CoinsToKickstartResponse, DisableCoinFailed, DisableCoinResponse, DisableCoinSuccess,
                      SetRequiredConfResponse, SetRequiredNotaResponse};

pub(super) fn on_disable_coin(writer: &mut dyn Write, response: DisableCoinResponse) {
    match response {
        DisableCoinResponse::Success(mm2_rpc_result) => write_disable_success(writer, mm2_rpc_result.result),
        DisableCoinResponse::Failed(disable_failed) => write_disable_failed(writer, disable_failed),
    }
}

pub(super) fn on_set_confirmations(writer: &mut dyn Write, response: SetRequiredConfResponse) {
    writeln_field(writer, "coin", response.coin, ZERO_INDENT);
    writeln_field(writer, "confirmations", response.confirmations, ZERO_INDENT);
}

pub(super) fn on_set_notarization(writer: &mut dyn Write, response: SetRequiredNotaResponse) {
    writeln_field(writer, "coin", response.coin, ZERO_INDENT);
    writeln_field(
        writer,
        "requires_notarization",
        response.requires_notarization,
        ZERO_INDENT,
    );
}

pub(super) fn on_coins_to_kickstart(writer: &mut dyn Write, coins: CoinsToKickstartResponse) {
    write_sequence(writer, "coins", coins.iter(), ZERO_INDENT);
}

fn write_disable_success(writer: &mut dyn Write, disable_success: DisableCoinSuccess) {
    writeln_field(writer, "coin", disable_success.coin, ZERO_INDENT);
    let cancelled_orders = disable_success.cancelled_orders.iter();
    write_sequence(writer, "cancelled_orders", cancelled_orders, ZERO_INDENT);
    writeln_field(writer, "passivized", disable_success.passivized, ZERO_INDENT);
}

fn write_disable_failed(writer: &mut dyn Write, disable_failed: DisableCoinFailed) {
    writeln_field(writer, "error", disable_failed.error, ZERO_INDENT);

    let active_swaps = disable_failed.active_swaps.iter();
    write_sequence(writer, "active_swaps", active_swaps, ZERO_INDENT);
    let orders_matching = disable_failed.orders.matching.iter();
    write_sequence(writer, "orders_matching", orders_matching, ZERO_INDENT);
    let orders_cancelled = disable_failed.orders.cancelled.iter();
    write_sequence(writer, "orders_matching", orders_cancelled, ZERO_INDENT);
}

fn format_coin_balance(balance: CoinBalance) -> Result<String> {
    Ok(format!(
        "{}:{}",
        format_ratio(&balance.spendable, COMMON_PRECISION)?,
        format_ratio(&balance.unspendable, COMMON_PRECISION)?
    ))
}

fn format_token_balances(balances: TokenBalances) -> Result<String> {
    if balances.is_empty() {
        return Ok("{}".to_string());
    }
    let mut buff: Vec<u8> = vec![];
    let writer: &mut dyn Write = &mut buff;
    for (token, balance) in balances {
        writeln_field(writer, token, format_coin_balance(balance)?, ZERO_INDENT);
    }
    String::from_utf8(buff).map_err(|error| error_anyhow!("Failed to format token_balances: {}", error))
}

fn format_addr_infos<T, F: FnOnce(T) -> Result<String> + Copy>(
    addr: HashMap<String, CoinAddressInfo<T>>,
    format_balance: F,
) -> String {
    let mut term_table = term_table_blank(TableStyle::thin(), false, false, false);
    term_table.add_row(Row::new(["address, pubkey", "method", "balance(sp,unsp)", "tickers"]));
    for (address, info) in addr {
        term_table.add_row(Row::new(vec![
            format!("{}\n{}", address, info.pubkey),
            info.derivation_method.to_string(),
            format_option(
                info.balances
                    .map(format_balance)
                    .map(|result| result.unwrap_or_else(|_| "error".to_string())),
            ),
            format_option(info.tickers.map(|tickers| tickers.iter().join("\n"))),
        ]))
    }
    term_table.render()
}

fn format_option<T: ToString>(value: Option<T>) -> String {
    let Some(value) = value else {
        return  "none".to_string();
    };
    value.to_string()
}
