use cli_table::Table;
use derive_more::Display;
use log::error;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[derive(Serialize, Clone)]
pub struct Command<T>
where
    T: Serialize + Sized,
{
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub flatten_data: Option<T>,
    pub userpass: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<Method>,
}

impl<T> Command<T>
where
    T: Serialize + Sized,
{
    pub fn builder() -> CommandBuilder<T> { CommandBuilder::new() }
}

pub struct CommandBuilder<T> {
    userpass: Option<String>,
    method: Option<Method>,
    flatten_data: Option<T>,
}

impl<T> CommandBuilder<T>
where
    T: Serialize,
{
    fn new() -> Self {
        CommandBuilder {
            userpass: None,
            method: None,
            flatten_data: None,
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
        self.flatten_data = Some(flatten_data);
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
            flatten_data: self.flatten_data.take(),
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
pub struct Dummy {}

#[derive(Serialize, Clone, derive_more::Display)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Stop,
    Version,
    #[serde(rename = "my_balance")]
    GetBalance,
    #[serde(rename = "get_enabled_coins")]
    GetEnabledCoins,
    #[serde(rename = "orderbook")]
    GetOrderbook,
    Sell,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CoinPair {
    base: String,
    rel: String,
}

impl CoinPair {
    pub fn new(base: &str, rel: &str) -> Self {
        Self {
            base: base.to_string(),
            rel: rel.to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Display)]
#[serde(rename_all = "lowercase")]
enum StopStatus {
    Success,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SellData {
    base: String,
    rel: String,
    volume: f64,
    price: f64,
}

impl SellData {
    pub fn new(base: &str, rel: &str, volume: f64, price: f64) -> Self {
        Self {
            base: base.to_string(),
            rel: rel.to_string(),
            volume,
            price,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct StopResponse {
    result: StopStatus,
}

impl Display for StopResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result { writeln!(f, "Status: {}", self.result) }
}

#[derive(Deserialize, Table)]
pub struct GetEnabledResult {
    #[table(title = "Ticker")]
    ticker: String,
    #[table(title = "Address")]
    address: String,
}

#[derive(Deserialize)]
pub struct GetEnabledResponse {
    pub result: Vec<GetEnabledResult>,
}

// pub struct Order {
//     pub coin: String,
//     pub address: String,
//     pub price: String,
//     pub
// }
