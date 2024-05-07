use clap::Args;
use std::mem::take;

use common::serde_derive::Serialize;
use mm2_rpc::data::legacy::OrderbookRequest;

use crate::komodefi_proc::OrderbookSettings;

const ORDERBOOK_BIDS_LIMIT: &str = "20";
const ORDERBOOK_ASKS_LIMIT: &str = "20";

#[derive(Args, Debug, Serialize)]
pub(crate) struct OrderbookArgs {
    #[arg(help = "Base currency of a pair")]
    base: String,
    #[arg(help = "Related currency, also can be called \"quote currency\" according to exchange terms")]
    rel: String,
    #[arg(long, short, help = "Enable `uuid` column")]
    uuids: bool,
    #[arg(long, short, visible_alias = "min", help = "Enable `min_volume` column")]
    min_volume: bool,
    #[arg(long, short = 'M', visible_alias = "max", help = "Enable `max_volume` column")]
    max_volume: bool,
    #[arg(long, short, help = "Enable `public` column")]
    publics: bool,
    #[arg(long, short = 'a', help = "Enable `address` column")]
    address: bool,
    #[arg(long, short = 'A', help = "Enable `age` column")]
    age: bool,
    #[arg(long, short, help = "Enable order confirmation settings column")]
    conf_settings: bool,
    #[arg(long, help = "Orderbook asks count limitation", default_value = ORDERBOOK_ASKS_LIMIT)]
    asks_limit: Option<usize>,
    #[arg(long, help = "Orderbook bids count limitation", default_value = ORDERBOOK_BIDS_LIMIT)]
    bids_limit: Option<usize>,
}

impl From<&mut OrderbookArgs> for OrderbookSettings {
    fn from(value: &mut OrderbookArgs) -> Self {
        OrderbookSettings {
            uuids: value.uuids,
            min_volume: value.min_volume,
            max_volume: value.max_volume,
            publics: value.publics,
            address: value.address,
            age: value.age,
            conf_settings: value.conf_settings,
            asks_limit: value.asks_limit,
            bids_limit: value.bids_limit,
        }
    }
}

impl From<&mut OrderbookArgs> for OrderbookRequest {
    fn from(value: &mut OrderbookArgs) -> Self {
        OrderbookRequest {
            rel: take(&mut value.rel),
            base: take(&mut value.base),
        }
    }
}
