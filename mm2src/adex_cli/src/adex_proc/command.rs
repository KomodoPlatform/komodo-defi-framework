use derive_more::Display;
use serde::Serialize;

#[derive(Serialize, Clone)]
pub(super) struct Command<T>
where
    T: Serialize + Sized,
{
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    flatten_data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<Method>,
    #[serde(skip_serializing_if = "Option::is_none")]
    userpass: Option<String>,
}

#[derive(Serialize, Clone, Display)]
#[serde(rename_all = "lowercase")]
pub(super) enum Method {
    Stop,
    Version,
    #[serde(rename = "my_balance")]
    GetBalance,
    #[serde(rename = "get_enabled_coins")]
    GetEnabledCoins,
    #[serde(rename = "orderbook")]
    GetOrderbook,
    Sell,
    Buy,
}

#[derive(Serialize, Clone, Copy, Display)]
pub(super) struct Dummy {}

impl<T> Command<T>
where
    T: Serialize + Sized,
{
    pub(super) fn builder() -> CommandBuilder<T> { CommandBuilder::new() }
}

pub(super) struct CommandBuilder<T> {
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

    pub(super) fn userpass(&mut self, userpass: String) -> &mut Self {
        self.userpass = Some(userpass);
        self
    }

    pub(super) fn method(&mut self, method: Method) -> &mut Self {
        self.method = Some(method);
        self
    }

    pub(super) fn flatten_data(&mut self, flatten_data: T) -> &mut Self {
        self.flatten_data = Some(flatten_data);
        self
    }

    pub(super) fn build(&mut self) -> Command<T> {
        Command {
            userpass: self.userpass.take(),
            method: self.method.take(),
            flatten_data: self.flatten_data.take(),
        }
    }
}

impl<T: Serialize + Clone> std::fmt::Display for Command<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut cmd: Self = self.clone();
        cmd.userpass = self.userpass.as_ref().map(|_| "***********".to_string());
        writeln!(
            f,
            "{}",
            serde_json::to_string(&cmd).unwrap_or_else(|_| "Unknown".to_string())
        )
    }
}
