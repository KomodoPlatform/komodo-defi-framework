mod cmd_sell_buy;
mod cmd_set_config;
mod cmd_set_price;
mod cmd_task;
mod cmd_update_maker_order;

mod commands_cancel;
mod commands_coin;
mod commands_message;
mod commands_mm2;
mod commands_network;
mod commands_order;
mod commands_swap;
mod commands_utility;
mod commands_version_stat;
mod commands_wallet;

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use std::str::FromStr;

use mm2_number::bigdecimal::ParseBigDecimalError;
use mm2_number::{BigDecimal, MmNumber};

pub(crate) mod prelude {
    pub(crate) use super::cmd_sell_buy::{BuyOrderArgs, SellOrderArgs};
    pub(crate) use super::cmd_set_config::SetConfigArgs;
    pub(crate) use super::cmd_set_price::SetPriceArgs;
    pub(crate) use super::cmd_task::{TaskSubcommand, TaskSubcommandCancel, TaskSubcommandStatus};
    pub(crate) use super::cmd_update_maker_order::UpdateMakerOrderArgs;
    pub(crate) use super::commands_cancel::CancelSubcommand;
    pub(crate) use super::commands_coin::CoinCommands;
    pub(crate) use super::commands_message::MessageCommands;
    pub(crate) use super::commands_mm2::Mm2Commands;
    pub(crate) use super::commands_network::NetworkCommands;
    pub(crate) use super::commands_order::OrderCommands;
    pub(crate) use super::commands_swap::SwapCommands;
    pub(crate) use super::commands_utility::UtilityCommands;
    pub(crate) use super::commands_version_stat::VersionStatCommands;
    pub(crate) use super::commands_wallet::{TxHistoryArgs, WalletCommands};
}

fn parse_mm_number(value: &str) -> Result<MmNumber, ParseBigDecimalError> {
    let decimal: BigDecimal = BigDecimal::from_str(value)?;
    Ok(MmNumber::from(decimal))
}

fn parse_datetime(value: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    Utc.datetime_from_str(value, "%y-%m-%dT%H:%M:%S")
}
