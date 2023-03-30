use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use derive_more::Display;

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

#[derive(Serialize, Deserialize, Display)]
#[serde(rename_all = "lowercase")]
enum StopStatus {
    Success
}

#[derive(Serialize, Deserialize)]
pub struct SendStopResponse {
    result: StopStatus,
}

impl Display for SendStopResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Status: {}", self.result)
    }
}

#[derive(Serialize, Deserialize)]
pub struct VersionResponse {
    #[serde(rename(deserialize = "result", serialize = "result"))]
    version: String,
    datetime: String,
}

impl Display for VersionResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Version: {}", self.version)?;
        writeln!(f, "Datetime: {}", self.datetime)
    }
}
