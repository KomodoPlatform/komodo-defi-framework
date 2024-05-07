#[path = "commands_swap/cmd_trade_preimage.rs"]
mod cmd_trade_preimage;

use chrono::{DateTime, Utc};
pub(crate) use cmd_trade_preimage::TradePreimageArgs;

use clap::{Args, Subcommand};
use uuid::Uuid;

use mm2_rpc::data::legacy::MySwapsFilter;

use super::parse_datetime;
use crate::rpc_data::{MyRecentSwapsRequest, Params, RecoverFundsOfSwapRequest};

#[derive(Subcommand)]
pub(crate) enum SwapCommands {
    #[command(
        short_flag = 'a',
        visible_alias = "active",
        about = "Get all the swaps that are currently running"
    )]
    ActiveSwaps(ActiveSwapsArgs),
    #[command(
        short_flag = 's',
        visible_alias = "status",
        about = "Return the data of an atomic swap"
    )]
    MySwapStatus(MySwapStatusArgs),
    #[command(
        short_flag = 'r',
        visible_alias = "recent",
        about = "Return the data of the most recent atomic swaps by filter"
    )]
    MyRecentSwaps(MyRecentSwapsArgs),
    #[command(
        short_flag = 'R',
        visible_aliases = ["recover", "recover-funds", "refund"],
        about = "Reclaim the user funds from the swap-payment address, if possible"
    )]
    RecoverFundsOfSwap(RecoverFundsOfSwapArgs),
    #[command(about = "Return the minimum required volume for buy/sell/setprice methods for the selected coin")]
    MinTradingVol { coin: String },
    #[command(
        about = "Returns the maximum available volume for buy/sell methods for the given coin. \
                 The result should be used as is for sell method or divided by price for buy method."
    )]
    MaxTakerVol { coin: String },
    #[command(
        visible_alias = "preimage",
        about = "Return the approximate fee amounts that are paid per the whole swap"
    )]
    TradePreimage(TradePreimageArgs),
}

#[derive(Args, Debug)]
pub(crate) struct ActiveSwapsArgs {
    #[arg(
        long,
        short = 's',
        default_value_t = false,
        help = "Whether to include swap statuses in response; defaults to false"
    )]
    pub(crate) include_status: bool,
}

#[derive(Args, Debug)]
pub(crate) struct MySwapStatusArgs {
    #[arg(help = "Uuid of swap, typically received from the buy/sell call")]
    pub(crate) uuid: Uuid,
}

#[derive(Args, Debug)]
pub(crate) struct MyRecentSwapsArgs {
    #[arg(
        long,
        short = 'l',
        default_value_t = 10,
        help = "Limits the number of returned swaps"
    )]
    pub(crate) limit: usize,
    #[arg(
        long,
        short = 'u',
        help = "Skip records until this uuid, skipping the from_uuid as well"
    )]
    pub(crate) from_uuid: Option<Uuid>,
    #[arg(
        long,
        visible_alias = "page",
        short = 'p',
        help = "Return swaps from the given page; This param will be ignored if from_uuid is set"
    )]
    pub(crate) page_number: Option<usize>,
    #[arg(
        long,
        short = 'm',
        visible_alias = "mine",
        help = "Return only swaps that match the swap.my_coin = request.my_coin condition"
    )]
    pub(crate) my_coin: Option<String>,
    #[arg(
        long,
        short = 'o',
        visible_alias = "other",
        help = "Return only swaps that match the swap.other_coin = request.other_coin condition"
    )]
    pub(crate) other_coin: Option<String>,
    #[arg(
        long,
        short = 't',
        value_parser = parse_datetime,
        help = "Return only swaps that match the swap.started_at >= request.from_timestamp condition. Datetime fmt: \"%y-%m-%dT%H:%M:%S\""
    )]
    pub(crate) from_timestamp: Option<DateTime<Utc>>,
    #[arg(
        long,
        short = 'T',
        value_parser = parse_datetime,
        help = "Return only swaps that match the swap.started_at < request.to_timestamp condition. Datetime fmt: \"%y-%m-%dT%H:%M:%S\""
    )]
    pub(crate) to_timestamp: Option<DateTime<Utc>>,
}

impl From<&mut MyRecentSwapsArgs> for MyRecentSwapsRequest {
    fn from(value: &mut MyRecentSwapsArgs) -> Self {
        MyRecentSwapsRequest {
            limit: value.limit,
            from_uuid: value.from_uuid.take(),
            page_number: value.page_number.take(),
            filter: MySwapsFilter {
                my_coin: value.my_coin.take(),
                other_coin: value.other_coin.take(),
                from_timestamp: value.from_timestamp.map(|dt| dt.timestamp() as u64),
                to_timestamp: value.to_timestamp.map(|dt| dt.timestamp() as u64),
            },
        }
    }
}

#[derive(Args, Debug)]
pub(crate) struct RecoverFundsOfSwapArgs {
    #[arg(help = "Uuid of the swap to recover the funds")]
    pub(crate) uuid: Uuid,
}

impl From<&mut RecoverFundsOfSwapArgs> for RecoverFundsOfSwapRequest {
    fn from(value: &mut RecoverFundsOfSwapArgs) -> Self {
        RecoverFundsOfSwapRequest {
            params: Params { uuid: value.uuid },
        }
    }
}
