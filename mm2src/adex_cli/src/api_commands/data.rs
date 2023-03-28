use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[derive(Serialize, Clone, derive_more::Display)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Stop,
    Version,
}

#[derive(Serialize, Clone)]
pub struct Command {
    pub userpass: String,
    pub method: Method,
}

impl Display for Command {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut cmd = self.clone();
        cmd.userpass = "***********".to_string();
        writeln!(f, "{}", serde_json::to_string(&cmd).unwrap_or("Unknown".to_string()))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AdexStatus {
    Success,
    Failure, //TODO: check if it is really failure)
}

#[derive(Serialize, Deserialize)]
pub struct SendStopResponse {
    result: AdexStatus,
}

#[derive(Serialize, Deserialize)]
pub struct VersionResponse {
    #[serde(rename(deserialize = "result", serialize = "result"))]
    version: String,
    datetime: String,
}
