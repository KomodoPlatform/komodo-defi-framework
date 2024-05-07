use chrono::{DateTime, Utc};
use clap::{Args, ValueEnum};
use derive_more::Display;

use mm2_number::MmNumber;
use mm2_rpc::data::legacy::OrdersHistoryRequest;

use crate::cli_cmd_args::{parse_datetime, parse_mm_number};
use crate::komodefi_proc;

#[derive(Clone, Display, ValueEnum)]
enum OrderTypeFilter {
    Taker,
    Maker,
}

#[derive(Clone, Display, ValueEnum)]
enum InitialActionFilter {
    Sell,
    Buy,
}

#[derive(Clone, Display, ValueEnum)]
enum StatusFilter {
    Created,
    Updated,
    Fulfilled,
    InsuficcientBalance,
    Cancelled,
    TimedOut,
}

#[derive(Args)]
pub(crate) struct OrdersHistoryArgs {
    #[command(flatten)]
    settings: OrdersHistorySettings,
    #[arg(long = "type", value_enum, help = "Return only orders that match the type")]
    order_type: Option<OrderTypeFilter>,
    #[arg(
        long = "action",
        value_enum,
        help = "Return only orders that match the initial action. Note that maker order initial_action is considered \"Sell\""
    )]
    initial_action: Option<InitialActionFilter>,
    #[arg(long, help = "Return only orders that match the order.base")]
    base: Option<String>,
    #[arg(long, help = "Return only orders that match the order.rel")]
    rel: Option<String>,
    #[arg(long, value_parser = parse_mm_number, help = "Return only orders whose price is more or equal the from_price")]
    from_price: Option<MmNumber>,
    #[arg(long, value_parser = parse_mm_number, help = "Return only orders whose price is less or equal the to_price")]
    to_price: Option<MmNumber>,
    #[arg(long, value_parser = parse_mm_number, help = "Return only orders whose volume is more or equal the from_volume")]
    from_volume: Option<MmNumber>,
    #[arg(long, value_parser = parse_mm_number, help = "Return only orders whose volume is less or equal the to_volume")]
    to_volume: Option<MmNumber>,
    #[arg(
        long,
        value_parser = parse_datetime,
        help = "Return only orders that match the order.created_at >= from_dt. Datetime fmt: \"%y-%m-%dT%H:%M:%S\""
    )]
    from_dt: Option<DateTime<Utc>>,
    #[arg(
        long,
        value_parser = parse_datetime,
        help = "Return only orders that match the order.created_at <= to_dt. Datetime fmt: \"%y-%m-%dT%H:%M:%S\""
    )]
    to_dt: Option<DateTime<Utc>>,
    #[arg(
        long,
        help = "Return only GoodTillCancelled orders that got converted from taker to maker"
    )]
    was_taker: bool,
    #[arg(long, value_enum, help = "Return only orders that match the status")]
    status: Option<StatusFilter>,
}

impl From<&mut OrdersHistoryArgs> for OrdersHistoryRequest {
    fn from(value: &mut OrdersHistoryArgs) -> Self {
        OrdersHistoryRequest {
            order_type: value.order_type.as_ref().map(OrderTypeFilter::to_string),
            initial_action: value.initial_action.as_ref().map(InitialActionFilter::to_string),
            base: value.base.take(),
            rel: value.rel.take(),
            from_price: value.from_price.take(),
            to_price: value.to_price.take(),
            from_volume: value.from_volume.take(),
            to_volume: value.to_volume.take(),
            from_timestamp: value.from_dt.map(|dt| dt.timestamp() as u64),
            to_timestamp: value.to_dt.map(|dt| dt.timestamp() as u64),
            was_taker: Some(value.was_taker),
            status: value.status.as_ref().map(StatusFilter::to_string),
            include_details: value.settings.takers || value.settings.makers,
        }
    }
}

#[derive(Args, Clone)]
#[group(required = true, multiple = true)]
struct OrdersHistorySettings {
    #[arg(
        long,
        short,
        default_value_t = false,
        help = "Whether to show taker orders detailed history"
    )]
    takers: bool,
    #[arg(
        long,
        short,
        default_value_t = false,
        help = "Whether to show maker orders detailed history"
    )]
    makers: bool,
    #[arg(long, short, default_value_t = false, help = "Whether to show warnings")]
    warnings: bool,
    #[arg(long, short, default_value_t = false, help = "Whether to show common history data")]
    all: bool,
}

impl From<&OrdersHistorySettings> for komodefi_proc::OrdersHistorySettings {
    fn from(value: &OrdersHistorySettings) -> Self {
        komodefi_proc::OrdersHistorySettings {
            takers_detailed: value.takers,
            makers_detailed: value.makers,
            warnings: value.warnings,
            all: value.all,
        }
    }
}

impl From<&mut OrdersHistoryArgs> for komodefi_proc::OrdersHistorySettings {
    fn from(value: &mut OrdersHistoryArgs) -> Self { komodefi_proc::OrdersHistorySettings::from(&value.settings) }
}
