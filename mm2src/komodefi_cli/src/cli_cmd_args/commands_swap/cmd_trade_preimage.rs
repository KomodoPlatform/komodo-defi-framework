use clap::{Args, ValueEnum};
use std::mem::take;

use mm2_number::MmNumber;

use crate::cli_cmd_args::parse_mm_number;
use crate::rpc_data::{TradePreimageMethod, TradePreimageRequest};

#[derive(Args)]
pub(crate) struct TradePreimageArgs {
    #[arg(help = "Base currency of the request")]
    base: String,
    #[arg(help = "Rel currency of the request")]
    rel: String,
    #[arg(
        value_enum,
        name = "METHOD",
        help = "Price in rel the user is willing to pay per one unit of the base coin"
    )]
    swap_method: TradePreimageMethodArg,
    #[arg(
        value_parser = parse_mm_number,
        help = "Price in rel the user is willing to pay per one unit of the base coin"
    )]
    price: MmNumber,
    #[command(flatten)]
    volume: TradePreimageVol,
}

impl From<&mut TradePreimageArgs> for TradePreimageRequest {
    fn from(value: &mut TradePreimageArgs) -> Self {
        TradePreimageRequest {
            base: take(&mut value.base),
            rel: take(&mut value.rel),
            swap_method: match value.swap_method {
                TradePreimageMethodArg::SetPrice => TradePreimageMethod::SetPrice,
                TradePreimageMethodArg::Sell => TradePreimageMethod::Sell,
                TradePreimageMethodArg::Buy => TradePreimageMethod::Buy,
            },
            price: take(&mut value.price),
            volume: value.volume.volume.take(),
            max: value.volume.max,
        }
    }
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct TradePreimageVol {
    #[arg(
        long,
        short,
        group = "trade-preimage-vol",
        value_parser=parse_mm_number,
        help="Amount the user is willing to trade; ignored if max = true and swap_method = setprice, otherwise, it must be set"
    )]
    volume: Option<MmNumber>,
    #[arg(
        long,
        short,
        group = "trade-preimage-vol",
        help = "Whether to return the maximum available volume for setprice method; must not be set or false if swap_method is buy or sell"
    )]
    max: bool,
}

#[derive(Clone, ValueEnum)]
enum TradePreimageMethodArg {
    SetPrice,
    Buy,
    Sell,
}
