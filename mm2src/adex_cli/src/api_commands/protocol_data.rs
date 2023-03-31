use derive_more::Display;
use log::error;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::fmt::{Display, Formatter};

#[derive(Serialize, Clone, derive_more::Display)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Stop,
    Version,
    #[serde(rename = "my_balance")]
    Balance,
}

#[derive(Serialize, Clone)]
pub struct Command {
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub data: Option<Json>,
    pub userpass: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<Method>,
}

impl Command {
    pub fn new() -> CommandBuilder { CommandBuilder::new() }
}

pub struct CommandBuilder {
    userpass: Option<String>,
    method: Option<Method>,
    data: Option<Json>,
}

impl CommandBuilder {
    fn new() -> Self {
        CommandBuilder {
            userpass: None,
            method: None,
            data: None,
        }
    }

    pub fn userpass(&mut self, userpass: String) -> &mut Self {
        self.userpass = Some(userpass);
        self
    }

    pub fn method(&mut self, method: Method) -> &mut Self {
        self.method = Some(method);
        self
    }

    pub fn flatten_data(&mut self, data: Json) -> &mut Self {
        self.data = Some(data);
        self
    }

    pub fn build(&mut self) -> Command {
        let command = Command {
            userpass: self
                .userpass
                .take()
                .ok_or_else(|| error!("Build command failed, no userpass"))
                .expect("Unexpected error during building api command"),
            method: self.method.take(),
            data: self.data.take(),
        };
        command
    }
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
    Success,
}

#[derive(Serialize, Deserialize)]
pub struct SendStopResponse {
    result: StopStatus,
}

impl Display for SendStopResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result { writeln!(f, "Status: {}", self.result) }
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

#[derive(Serialize, Deserialize, Display)]
pub struct BalanceCommand {
    result: StopStatus,
}
