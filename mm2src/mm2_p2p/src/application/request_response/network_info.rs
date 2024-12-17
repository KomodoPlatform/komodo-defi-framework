use serde::{Deserialize, Serialize};

/// Wraps the different types of network information requests for the P2P request-response
/// protocol.
#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum NetworkInfoRequest {
    /// Get MM2 version of nodes added to stats collection
    GetMm2Version,
    /// Get UTC timestamp in seconds from the target peer
    GetPeerUtcTimestamp,
}
