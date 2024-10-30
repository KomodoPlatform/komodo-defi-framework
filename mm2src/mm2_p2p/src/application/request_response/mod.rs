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
    /// Request for network information from the target peer.
    ///
    /// TODO: This should be called `PeerInfoRequest` instead. However, renaming it
    /// will introduce a breaking change in the network and is not worth it. Do this
    /// renaming when there is already a breaking change in the release.
    NetworkInfo(network_info::NetworkInfoRequest),
}
