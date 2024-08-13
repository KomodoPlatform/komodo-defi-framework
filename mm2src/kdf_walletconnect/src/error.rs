use derive_more::Display;
use enum_derives::EnumFromStringify;
use relay_rpc::rpc::{self, ServiceError, SubscriptionError};
use tokio_tungstenite_wasm::CloseFrame;

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
}

#[derive(Debug, Display, EnumFromStringify)]
pub enum ClientError {
    ChannelClosed,
    #[display(fmt = "RPC error: code={code} data={data:?} message={message}")]
    Rpc {
        code: i32,
        message: String,
        data: Option<String>,
    },
    #[display(fmt = "Invalid error response")]
    InvalidErrorResponse,
    #[display(fmt = "Invalid request type")]
    InvalidRequestType,
    DeserializationError(String),
    SerializationError(String),
    #[from_stringify("WebsocketClientError")]
    WebsocketClientError(String),
    #[from_stringify("RequestBuildError")]
    RequestBuilderError(String),
    #[display(fmt = "Duplicate request ID")]
    DuplicateRequestId,
    #[display(fmt = "Invalid response ID")]
    InvalidResponseId,
}

impl From<rpc::ErrorData> for ClientError {
    fn from(err: rpc::ErrorData) -> Self {
        Self::Rpc {
            code: err.code,
            message: err.message,
            data: err.data,
        }
    }
}

#[derive(Debug, Display, EnumFromStringify)]
pub enum WebsocketClientError {
    #[display(fmt = "Connection closed: {_0}")]
    ConnectionClosed(String),
    #[display(fmt = "Connection Error: {_0}")]
    TransportError(String),
    #[display(fmt = "Not connected")]
    NotConnected,
    ClosingFailed(String),
}

#[derive(Debug, Display, EnumFromStringify)]
pub enum ServiceErrorExt<T> {
    Client(ClientError),
    Response(rpc::Error<T>),
}

impl<T: ServiceError> From<rpc::Error<T>> for ServiceErrorExt<T> {
    fn from(err: rpc::Error<T>) -> Self { Self::Response(err) }
}

impl<T> From<SubscriptionError> for ServiceErrorExt<T> {
    fn from(_: SubscriptionError) -> Self { Self::Response(rpc::Error::TooManyRequests) }
}

impl<T: ServiceError> From<ClientError> for ServiceErrorExt<T> {
    fn from(err: ClientError) -> Self {
        match err {
            ClientError::Rpc { code, message, data } => {
                let err = rpc::ErrorData { code, message, data };

                match rpc::Error::try_from(err) {
                    Ok(err) => ServiceErrorExt::Response(err),
                    Err(_) => ServiceErrorExt::Client(ClientError::InvalidErrorResponse),
                }
            },

            _ => ServiceErrorExt::Client(err),
        }
    }
}

/// Wrapper around the websocket [`CloseFrame`] providing info about the
/// connection closing reason.
#[derive(Debug, Clone)]
pub struct CloseReason(pub Option<CloseFrame<'static>>);

impl std::fmt::Display for CloseReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(frame) = &self.0 {
            frame.fmt(f)
        } else {
            f.write_str("<close frame unavailable>")
        }
    }
}
