use clap::Args;
use std::mem::take;
use uuid::Uuid;

use mm2_rpc::data::legacy::OrderStatusRequest;

#[derive(Args)]
pub(crate) struct OrderStatusArgs {
    uuid: Uuid,
}

impl From<&mut OrderStatusArgs> for OrderStatusRequest {
    fn from(value: &mut OrderStatusArgs) -> Self {
        OrderStatusRequest {
            uuid: take(&mut value.uuid),
        }
    }
}
