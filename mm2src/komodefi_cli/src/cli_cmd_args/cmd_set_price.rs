use clap::Args;
use std::mem::take;

use mm2_number::MmNumber;
use mm2_rpc::data::legacy::SetPriceRequest;

use super::parse_mm_number;

#[derive(Args)]
#[command(about = "Place an order on the orderbook. The setprice order is always considered a sell")]
pub(crate) struct SetPriceArgs {
    #[arg(help = "The name of the coin the user desires to sell")]
    base: String,
    #[arg(help = "The name of the coin the user desires to receive")]
    rel: String,
    #[arg(
        value_parser = parse_mm_number,
        help = "The price in rel the user is willing to receive per one unit of the base coin"
    )]
    price: MmNumber,
    #[command(flatten)]
    delegate: SetPriceVolumeGroup,
    #[arg(
        long,
        short,
        value_parser = parse_mm_number,
        help = "The minimum amount of base coin available for the order; it must be less or equal than volume param; \
                the following values must be greater than or equal to the min_trading_vol of the corresponding coin",
    )]
    min_volume: Option<MmNumber>,
    #[arg(
        long,
        short,
        visible_alias = "cancel",
        help = "Cancel all existing orders for the give pair"
    )]
    cancel_prev: bool,
    #[arg(
        long,
        visible_alias = "bc",
        help = "Number of required blockchain confirmations for base coin atomic swap transaction"
    )]
    base_confs: Option<u64>,
    #[arg(
        long,
        visible_alias = "bn",
        help = "Whether dPoW notarization is required for base coin atomic swap transaction"
    )]
    base_nota: Option<bool>,
    #[arg(
        long,
        visible_alias = "rc",
        help = "Number of required blockchain confirmations for rel coin atomic swap transaction"
    )]
    rel_confs: Option<u64>,
    #[arg(
        long,
        visible_alias = "rn",
        help = "Whether dPoW notarization is required for rel coin atomic swap transaction"
    )]
    rel_nota: Option<bool>,
    #[arg(
        long,
        short,
        visible_alias = "save",
        help = "If true, each order's short record history is stored in a local SQLite database table, \
                and when the order is cancelled or fully matched, it's history will be saved as a json file",
        default_value_t = true
    )]
    save_in_history: bool,
}

impl From<&mut SetPriceArgs> for SetPriceRequest {
    fn from(set_price_args: &mut SetPriceArgs) -> Self {
        SetPriceRequest {
            base: take(&mut set_price_args.base),
            rel: take(&mut set_price_args.rel),
            price: take(&mut set_price_args.price),
            max: set_price_args.delegate.max,
            volume: set_price_args
                .delegate
                .volume
                .as_mut()
                .map_or_else(MmNumber::default, take),
            min_volume: take(&mut set_price_args.min_volume),
            cancel_previous: set_price_args.cancel_prev,
            base_confs: take(&mut set_price_args.base_confs),
            base_nota: take(&mut set_price_args.base_nota),
            rel_confs: take(&mut set_price_args.rel_confs),
            rel_nota: take(&mut set_price_args.rel_nota),
            save_in_history: set_price_args.save_in_history,
        }
    }
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct SetPriceVolumeGroup {
    #[arg(
        group = "set-price-volume",
        long,
        short = 'M',
        help = "Use the entire coin balance for the order, taking 0.001 coins into reserve to account for fees",
        default_value_t = false
    )]
    max: bool,
    #[arg(
        group = "set-price-volume",
        long,
        short,
        help = "The maximum amount of base coin available for the order, ignored if max is true; the following values must be greater than or equal to the min_trading_vol of the corresponding coin",
        value_parser = parse_mm_number
    )]
    volume: Option<MmNumber>,
}
