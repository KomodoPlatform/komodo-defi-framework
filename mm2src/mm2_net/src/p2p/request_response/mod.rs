//! Types to use with p2p request-response protocol.

use serde::{Deserialize, Serialize};

pub mod network_info;
pub mod ordermatch;

#[derive(Eq, Debug, Deserialize, PartialEq, Serialize)]
pub enum P2PRequest {
    Ordermatch(ordermatch::OrdermatchRequest),
    NetworkInfo(network_info::NetworkInfoRequest),
}
