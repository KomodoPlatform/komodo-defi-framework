use rpc::v1::types::H256 as H256Json;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct MmVersionResponse {
    pub result: String,
    pub datetime: String,
}

#[derive(Default, Serialize)]
#[serde(tag = "method", rename = "stop")]
pub struct StopRequest {}

#[derive(Default, Serialize)]
#[serde(tag = "method", rename = "version")]
pub struct VersionRequest {}

#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename = "ban_pubkey")]
pub struct BanPubkeysRequest {
    pub pubkey: H256Json,
    pub reason: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum UnbanPubkeysRequest {
    All,
    Few(Vec<H256Json>),
}
