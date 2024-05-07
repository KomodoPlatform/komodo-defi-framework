use clap::{Args, Subcommand};
use std::mem::take;

use mm2_rpc::data::legacy::{SetRequiredConfRequest, SetRequiredNotaRequest};

use crate::rpc_data::DisableCoinRequest;

#[derive(Subcommand)]
pub(crate) enum CoinCommands {
    #[command(about = "Put a coin to the trading index")]
    Enable(EnableArgs),
    #[command(about = "Deactivates enabled coin and also cancels all active orders that use the given coin.")]
    Disable(DisableCoinArgs),
    #[command(visible_alias = "enabled", about = "List activated coins")]
    GetEnabled,
    #[command(
        visible_alias = "set-conf",
        about = "Set the number of confirmations to wait for the given coin"
    )]
    SetRequiredConf(SetRequiredConfArgs),
    #[command(
        visible_alias = "set-nota",
        about = "Whether to wait for a dPoW notarization of the given atomic swap transactions"
    )]
    SetRequiredNota(SetRequiredNotaArgs),
    #[command(
        visible_alias = "to-kick",
        about = "Return the coins that should be activated to continue the interrupted swaps"
    )]
    CoinsToKickStart,
}

#[derive(Args)]
pub(crate) struct EnableArgs {
    #[arg(help = "Coin to be included into the trading index")]
    pub(crate) coin: String,
    #[arg(
        long,
        short = 'k',
        visible_aliases = ["track", "keep", "progress"],
        default_value_t = 0,
        help = "Whether to keep progress on task based commands"
    )]
    pub(crate) keep_progress: u64,
    #[arg(
        long,
        default_value_t = false,
        visible_alias = "history",
        short = 'H',
        help = "Whether to save tx history for the coin"
    )]
    pub(crate) tx_history: bool,
}

#[derive(Args)]
pub(crate) struct DisableCoinArgs {
    #[arg(name = "COIN", help = "Coin to disable")]
    coin: String,
}

impl From<&mut DisableCoinArgs> for DisableCoinRequest {
    fn from(value: &mut DisableCoinArgs) -> Self {
        DisableCoinRequest {
            coin: take(&mut value.coin),
        }
    }
}

#[derive(Args)]
pub(crate) struct SetRequiredConfArgs {
    #[arg(help = "Ticker of the given coin")]
    pub(crate) coin: String,
    #[arg(visible_alias = "conf", help = "Number of confirmations to require")]
    pub(crate) confirmations: u64,
}

impl From<&mut SetRequiredConfArgs> for SetRequiredConfRequest {
    fn from(value: &mut SetRequiredConfArgs) -> Self {
        SetRequiredConfRequest {
            coin: take(&mut value.coin),
            confirmations: value.confirmations,
        }
    }
}

#[derive(Args)]
pub(crate) struct SetRequiredNotaArgs {
    #[arg(help = "Ticker of the given coin")]
    pub(crate) coin: String,
    #[arg(
        long,
        short = 'n',
        visible_alias = "requires-nota",
        help = "Whether the node should wait for dPoW notarization of atomic swap transactions"
    )]
    pub(crate) requires_notarization: bool,
}

impl From<&mut SetRequiredNotaArgs> for SetRequiredNotaRequest {
    fn from(value: &mut SetRequiredNotaArgs) -> Self {
        SetRequiredNotaRequest {
            coin: take(&mut value.coin),
            requires_notarization: value.requires_notarization,
        }
    }
}
