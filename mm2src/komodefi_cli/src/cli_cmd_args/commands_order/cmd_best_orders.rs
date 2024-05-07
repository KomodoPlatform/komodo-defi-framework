use clap::{Args, ValueEnum};
use std::mem::take;

use mm2_number::MmNumber;
use mm2_rpc::data::version2::{BestOrdersAction, BestOrdersByRequest, BestOrdersRequestV2};

use crate::cli_cmd_args::parse_mm_number;

#[derive(Args)]
pub(crate) struct BestOrderArgs {
    #[arg(value_enum, help = "The coin to get best orders")]
    coin: String,
    #[arg(help = "Whether to buy or sell the given coin")]
    action: OrderActionArg,
    #[command(flatten)]
    delegate: BestOrdersByArg,
    #[arg(
        long,
        short = 'o',
        visible_aliases = ["show-origin", "original-tickers", "origin"],
        help = "Whether to show the original tickers if they are configured for the queried coin",
        default_value = "false"
    )]
    pub(crate) show_orig_tickets: bool,
    #[arg(long, short, help = "Exclude orders that is mine", default_value = "false")]
    exclude_mine: bool,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct BestOrdersByArg {
    #[arg(
        long,
        short,
        group = "best-orders",
        value_parser=parse_mm_number,
        help="The returned results will show the best prices for trades that can fill the requested volume"
    )]
    volume: Option<MmNumber>,
    #[arg(
        long,
        short,
        group = "best-orders",
        help = "The returned results will show a list of the best prices"
    )]
    number: Option<usize>,
}

#[derive(Clone, ValueEnum)]
enum OrderActionArg {
    Buy,
    Sell,
}

impl From<&mut BestOrdersByArg> for BestOrdersByRequest {
    fn from(value: &mut BestOrdersByArg) -> Self {
        if let Some(number) = value.number {
            BestOrdersByRequest::Number(number)
        } else if let Some(ref mut volume) = value.volume {
            BestOrdersByRequest::Volume(take(volume))
        } else {
            panic!("Unreachable state during converting BestOrdersCli into BestOrdersByRequest");
        }
    }
}

impl From<&OrderActionArg> for BestOrdersAction {
    fn from(value: &OrderActionArg) -> Self {
        match value {
            OrderActionArg::Buy => BestOrdersAction::Buy,
            OrderActionArg::Sell => BestOrdersAction::Sell,
        }
    }
}

impl From<&mut BestOrderArgs> for BestOrdersRequestV2 {
    fn from(value: &mut BestOrderArgs) -> Self {
        BestOrdersRequestV2 {
            coin: take(&mut value.coin),
            action: (&value.action).into(),
            request_by: (&mut value.delegate).into(),
            exclude_mine: value.exclude_mine,
        }
    }
}
