use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum NetworkInfoRequest {
    /// Get MM2 version of nodes added to stats collection
    GetMm2Version,
}
