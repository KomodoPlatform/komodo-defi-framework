use derive_more::Display;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Display)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Success,
}

#[derive(Serialize, Deserialize)]
pub struct MmVersionResponse {
    pub result: String,
    pub datetime: String,
}
