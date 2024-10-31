//! This module defines types exclusively for the request-response P2P protocol
//! which are separate from other request types such as RPC requests or Gossipsub
//! messages.

pub mod network_info;
pub mod ordermatch;

use serde::{Deserialize, Serialize};

/// Wrapper type for handling request-response P2P requests.
#[derive(Eq, Debug, Deserialize, PartialEq, Serialize)]
pub enum P2PRequest {
    /// Request for order matching.
    Ordermatch(ordermatch::OrdermatchRequest),
    /// Request various information from the target peer.
    PeerInfo(network_info::PeerInfoRequest),
}
