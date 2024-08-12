use crate::error::ClientError;

use relay_rpc::domain::MessageId;
use relay_rpc::rpc::{self, ErrorResponse, Payload, Response, ServiceRequest, SuccessfulResponse};
use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite_wasm::Message;

/// The lower-level inbound RPC request.
///
/// Provides access to the request payload (via [`InboundRequest::data()`]) and
/// the response channel (via [`InboundRequest::respond()`]).
///
/// Currently, the only inbound RPC request the client can receive is
/// [`Subscription`][relay_rpc::rpc::Subscription].
#[derive(Debug)]
pub struct InboundRequest<T> {
    id: MessageId,
    tx: UnboundedSender<Message>,
    data: T,
}

impl<T> InboundRequest<T>
where
    T: ServiceRequest,
{
    pub(super) fn new(id: MessageId, data: T, tx: UnboundedSender<Message>) -> Self { Self { id, tx, data } }

    /// Returns the request payload reference.
    pub fn data(&self) -> &T { &self.data }

    pub fn id(&self) -> MessageId { self.id }

    /// Sends the response back to the Relay. The value is a
    /// `Result<T::Response, T::Error>` (see [`RequestPayload`] trait for
    /// details).
    ///
    /// Returns an error if the response can't be serialized, or if the
    /// underlying channel is closed.
    pub fn respond(self, response: Result<T::Response, T::Error>) -> Result<(), ClientError> {
        let response = match response {
            Ok(data) => Response::Success(SuccessfulResponse::new(self.id, serde_json::to_value(data)?)),

            Err(err) => Response::Error(ErrorResponse::new(self.id, rpc::Error::Handler(err))),
        };

        let message = Message::Text(serde_json::to_string(&Payload::Response(response))?);
        self.tx.send(message).map_err(|_| ClientError::ChannelClosed)
    }
}
