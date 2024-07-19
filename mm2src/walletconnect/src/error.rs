use derive_more::Display;
use enum_derives::EnumFromStringify;

/// Errors generated while parsing
/// [`ConnectionOptions`][crate::ConnectionOptions] and creating an HTTP request
/// for the websocket connection.
#[derive(Debug, Display, EnumFromStringify)]
pub enum RequestBuildError {
    #[from_stringify("serde_qs::Error")]
    #[display(fmt = "Failed to serialize connection query: {_0}")]
    Query(String),
    #[display(fmt = "Failed to add request headers")]
    Headers,
    #[from_stringify("url::ParseError")]
    #[display(fmt = "Failed to parse connection URL: {_0}")]
    Url(String),
    #[display(fmt = "Failed to create websocket request: {_0}")]
    WebsocketClient(String),
}

#[derive(Debug, Display, EnumFromStringify)]
pub enum ClientError {
    ChannelClosed,
    Deserialization(serde_json::Error),
    Serialization(serde_json::Error),
}

#[derive(Debug, Display, EnumFromStringify)]
pub enum WebsocketClientError {}
