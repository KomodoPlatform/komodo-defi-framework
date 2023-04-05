use cli_table::Table;
use derive_more::Display;
use log::error;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[derive(Serialize, Clone)]
pub(crate) struct Command<T>
where
    T: Serialize,
{
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    pub userpass: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<Method>,
}

impl<T> Command<T>
where
    T: Serialize,
{
    pub fn builder() -> CommandBuilder<T> { CommandBuilder::new() }
}

pub(crate) struct CommandBuilder<T> {
    userpass: Option<String>,
    method: Option<Method>,
    data: Option<T>,
}

impl<T> CommandBuilder<T>
where
    T: Serialize,
{
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

    pub fn flatten_data(&mut self, flatten_data: T) -> &mut Self {
        self.data = Some(flatten_data);
        self
    }

    pub fn build(&mut self) -> Command<T> {
        Command {
            userpass: self
                .userpass
                .take()
                .ok_or_else(|| error!("Build command failed, no userpass"))
                .expect("Unexpected error during building api command"),
            method: self.method.take(),
            data: self.data.take(),
        }
    }
}

impl<T: Serialize + Clone> Display for Command<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut cmd: Self = self.clone();
        cmd.userpass = "***********".to_string();
        writeln!(
            f,
            "{}",
            serde_json::to_string(&cmd).unwrap_or_else(|_| "Unknown".to_string())
        )
    }
}

#[derive(Serialize, Clone, Copy, derive_more::Display)]
pub(crate) struct Dummy {}

#[derive(Serialize, Clone, derive_more::Display)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Method {
    Stop,
    Version,
    #[serde(rename = "my_balance")]
    Balance,
    #[serde(rename = "get_enabled_coins")]
    GetEnabledCoins,
    #[serde(rename = "orderbook")]
    GetOrderbook,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct GetOrderbook {
    base: String,
    rel: String,
}

impl GetOrderbook {
    pub fn new(base: &str, rel: &str) -> Self {
        Self {
            base: base.to_string(),
            rel: rel.to_string(),
        }
    }
}

impl Display for GetOrderbook {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result { writeln!(f, "") }
}

#[derive(Serialize, Deserialize, Display)]
#[serde(rename_all = "lowercase")]
enum StopStatus {
    Success,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SendStopResponse {
    result: StopStatus,
}

impl Display for SendStopResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result { writeln!(f, "Status: {}", self.result) }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct VersionResponse {
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
pub(crate) struct BalanceCommand {
    result: StopStatus,
}

#[derive(Deserialize, Table)]
pub(crate) struct GetEnabledResult {
    #[table(title = "Ticker")]
    ticker: String,
    #[table(title = "Address")]
    address: String,
}

#[derive(Deserialize)]
pub(crate) struct GetEnabledResponse {
    pub result: Vec<GetEnabledResult>,
}
