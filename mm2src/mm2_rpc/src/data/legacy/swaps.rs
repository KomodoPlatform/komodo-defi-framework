use derive_more::Display;
use serde::{Deserialize, Serialize};
use skip_serializing_none::skip_serializing_none;

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
pub struct MySwapsFilter {
    pub my_coin: Option<String>,
    pub other_coin: Option<String>,
    pub from_timestamp: Option<u64>,
    pub to_timestamp: Option<u64>,
}

#[derive(Debug, Deserialize, Display, PartialEq, Serialize)]
pub enum RecoveredSwapAction {
    RefundedMyPayment,
    SpentOtherPayment,
}
