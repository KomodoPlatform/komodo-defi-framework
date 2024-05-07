use clap::{Args, Subcommand};
use std::mem::take;
use uuid::Uuid;

use mm2_rpc::data::legacy::{CancelAllOrdersRequest, CancelBy, CancelOrderRequest};

#[derive(Subcommand)]
pub(crate) enum CancelSubcommand {
    #[command(short_flag = 'o', about = "Cancels given order by uuid")]
    Order(CancelOrderArgs),
    #[command(short_flag = 'a', about = "Cancels all orders of current node")]
    All,
    #[command(
        short_flag = 'p',
        visible_alias = "pair",
        about = "Cancels all orders of specific pair"
    )]
    ByPair(CancelByPairArgs),
    #[command(
        short_flag = 'c',
        visible_alias = "coin",
        about = "Cancels all orders using the coin ticker as base or rel"
    )]
    ByCoin(CancelByCoinArgs),
}

#[derive(Args)]
pub(crate) struct CancelOrderArgs {
    #[arg(help = "Order identifier")]
    uuid: Uuid,
}

impl From<&mut CancelOrderArgs> for CancelOrderRequest {
    fn from(value: &mut CancelOrderArgs) -> Self {
        CancelOrderRequest {
            uuid: take(&mut value.uuid),
        }
    }
}

#[derive(Args)]
pub(crate) struct CancelByPairArgs {
    #[arg(help = "Base coin of the pair")]
    base: String,
    #[arg(help = "Rel coin of the pair")]
    rel: String,
}

impl From<&mut CancelByPairArgs> for CancelAllOrdersRequest {
    fn from(value: &mut CancelByPairArgs) -> Self {
        CancelAllOrdersRequest {
            cancel_by: CancelBy::Pair {
                base: take(&mut value.base),
                rel: take(&mut value.rel),
            },
        }
    }
}

#[derive(Args)]
pub(crate) struct CancelByCoinArgs {
    #[arg(help = "Order is cancelled if it uses ticker as base or rel")]
    ticker: String,
}

impl From<&mut CancelByCoinArgs> for CancelAllOrdersRequest {
    fn from(value: &mut CancelByCoinArgs) -> Self {
        CancelAllOrdersRequest {
            cancel_by: CancelBy::Coin {
                ticker: take(&mut value.ticker),
            },
        }
    }
}
