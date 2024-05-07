use derive_more::Display;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub(crate) struct VersionStatAddNodeRequest {
    pub(crate) name: String,
    pub(crate) address: String,
    pub(crate) peer_id: String,
}

#[derive(Debug, Deserialize, Display)]
#[serde(tag = "error_type", content = "error_data")]
pub(crate) enum NodeVersionError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Database error: {}", _0)]
    DatabaseError(String),
    #[display(fmt = "Invalid address: {}", _0)]
    InvalidAddress(String),
    #[display(fmt = "Error on parse peer id {}: {}", _0, _1)]
    PeerIdParseError(String, String),
    #[display(fmt = "{} is only supported in native mode", _0)]
    UnsupportedMode(String),
    #[display(fmt = "start_version_stat_collection is already running")]
    AlreadyRunning,
    #[display(fmt = "Version stat collection is currently stopping")]
    CurrentlyStopping,
    #[display(fmt = "start_version_stat_collection is not running")]
    NotRunning,
}

#[derive(Serialize)]
pub(crate) struct VersionStatRemoveNodeRequest {
    pub(crate) name: String,
}

#[derive(Serialize)]
pub(crate) struct VStatStartCollectionRequest {
    pub(crate) interval: f64,
}

#[derive(Serialize)]
pub(crate) struct VStatUpdateCollectionRequest {
    pub(crate) interval: f64,
}
