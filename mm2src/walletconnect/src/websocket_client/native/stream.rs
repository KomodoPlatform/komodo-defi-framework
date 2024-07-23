use std::{collections::{hash_map::Entry, HashMap},
          pin::Pin,
          task::{Context, Poll}};

use crate::{error::{ClientError, CloseReason, WebsocketClientError},
            HttpRequest, MessageIdGenerator};
use futures_util::{stream::FusedStream, Stream};
use futures_util::{SinkExt, StreamExt};
use relay_rpc::{domain::MessageId,
                rpc::{self, Params, Payload, Response, ServiceRequest, Subscription}};
use tokio::sync::{mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
                  oneshot};
use tokio_tungstenite_wasm::{connect, CloseFrame, Message, WebSocketStream};

use super::{inbound::InboundRequest,
            outbound::{create_request, OutboundRequest, ResponseFuture}};

/// Possible events produced by the [`ClientStream`].
///
/// The events are produced by polling [`ClientStream`] in a loop.
#[derive(Debug)]
pub enum StreamEvent {
    /// Inbound request for receiving a subscription message.
    ///
    /// Currently, [`Subscription`] is the only request that the Relay sends to
    /// the clients.
    InboundSubscriptionRequest(InboundRequest<Subscription>),

    /// Error generated when failed to parse an inbound message, invalid request
    /// type or message ID.
    InboundError(ClientError),

    /// Error generated when failed to write data to the underlying websocket
    /// stream.
    OutboundError(ClientError),

    /// The websocket connection was closed.
    ///
    /// This is the last event that can be produced by the stream.
    ConnectionClosed(Option<CloseFrame<'static>>),
}

/// Lower-level [`FusedStream`] interface for the client connection.
///
/// The stream produces [`StreamEvent`] when polled, and can be used to send RPC
/// requests (see [`ClientStream::send()`] and [`ClientStream::send_raw()`]).
///
/// For a higher-level interface see [`Client`](crate::client::Client). For an
/// example usage of the stream see `client::connection` module.
pub struct ClientStream {
    socket: WebSocketStream,
    socket_ended: bool,
    outbound_tx: UnboundedSender<Message>,
    outbound_rx: UnboundedReceiver<Message>,
    requests: HashMap<MessageId, oneshot::Sender<Result<serde_json::Value, ClientError>>>,
    id_generator: MessageIdGenerator,
    close_frame: Option<CloseFrame<'static>>,
}

/// Opens a connection to the Relay and returns [`ClientStream`] for the
/// connection.
pub async fn open_new_relay_connection_stream(request: HttpRequest<()>) -> Result<ClientStream, WebsocketClientError> {
    let stream = connect(request.uri().to_string())
        .await
        .map_err(|err| WebsocketClientError::TransportError(err.to_string()))?;

    Ok(ClientStream::new(stream))
}

impl ClientStream {
    fn new(socket: WebSocketStream) -> Self {
        let id_generator = MessageIdGenerator::new();
        let (outbound_tx, outbound_rx) = unbounded_channel();
        let requests = HashMap::new();

        Self {
            socket,
            outbound_rx,
            outbound_tx,
            requests,
            close_frame: None,
            id_generator,
            socket_ended: false,
        }
    }

    /// Sends an already serialized [`OutboundRequest`][OutboundRequest] (see
    /// [`create_request()`]).
    pub fn send_raw(&mut self, request: OutboundRequest) {
        let tx = request.tx;
        let id = self.id_generator.next();
        let request = Payload::Request(rpc::Request::new(id, request.params));
        let serialized = serde_json::to_string(&request);

        match serialized {
            Ok(data) => match self.requests.entry(id) {
                Entry::Occupied(_) => {
                    tx.send(Err(ClientError::DuplicateRequestId)).ok();
                },

                Entry::Vacant(entry) => {
                    entry.insert(tx);
                    self.outbound_tx.send(Message::Text(data)).ok();
                },
            },

            Err(err) => {
                tx.send(Err(ClientError::SerdeError(err.to_string()))).ok();
            },
        }
    }

    /// Serialize the request into a generic [`OutboundRequest`] and sends it,
    /// returning a future that resolves with the response.
    pub fn send<T>(&mut self, request: T) -> ResponseFuture<T>
    where
        T: ServiceRequest,
    {
        println!("send");
        let (request, response) = create_request(request);
        println!("send AFTER");
        self.send_raw(request);
        response
    }

    /// Closes the connection.
    pub async fn close(&mut self) -> Result<(), ClientError> {
        self.socket
            .close()
            .await
            .map_err(|err| WebsocketClientError::ClosingFailed(format!("{err:?}")).into())
    }

    fn parse_inbound(&mut self, result: Result<Message, WebsocketClientError>) -> Option<StreamEvent> {
        match result {
            Ok(message) => match &message {
                Message::Binary(_) | Message::Text(_) => {
                    let payload: Payload = match serde_json::from_slice(&message.into_data()) {
                        Ok(payload) => payload,

                        Err(err) => return Some(StreamEvent::InboundError(ClientError::SerdeError(err.to_string()))),
                    };

                    match payload {
                        Payload::Request(request) => {
                            let id = request.id;

                            let event = match request.params {
                                Params::Subscription(data) => StreamEvent::InboundSubscriptionRequest(
                                    InboundRequest::new(id, data, self.outbound_tx.clone()),
                                ),

                                _ => StreamEvent::InboundError(ClientError::InvalidRequestType),
                            };

                            Some(event)
                        },

                        Payload::Response(response) => {
                            let id = response.id();

                            if id.is_zero() {
                                return match response {
                                    Response::Error(response) => {
                                        Some(StreamEvent::InboundError(ClientError::from(response.error)))
                                    },

                                    Response::Success(_) => {
                                        Some(StreamEvent::InboundError(ClientError::InvalidResponseId))
                                    },
                                };
                            }

                            if let Some(tx) = self.requests.remove(&id) {
                                let result = match response {
                                    Response::Error(response) => Err(ClientError::from(response.error)),

                                    Response::Success(response) => Ok(response.result),
                                };

                                tx.send(result).ok();

                                // Perform compaction if required.
                                if self.requests.len() * 3 < self.requests.capacity() {
                                    self.requests.shrink_to_fit();
                                }

                                None
                            } else {
                                Some(StreamEvent::InboundError(ClientError::InvalidResponseId))
                            }
                        },
                    }
                },

                Message::Close(frame) => {
                    self.close_frame = frame.clone();
                    Some(StreamEvent::ConnectionClosed(frame.clone()))
                },

                _ => None,
            },

            Err(error) => Some(StreamEvent::InboundError(error.into())),
        }
    }

    fn poll_write(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), WebsocketClientError>> {
        let mut should_flush = false;

        loop {
            // `poll_ready() needs to be called before each `start_send()` to make sure the
            // sink is ready to accept more data.
            match self.socket.poll_ready_unpin(cx) {
                // The sink is ready to accept more data.
                Poll::Ready(Ok(())) => {
                    if let Poll::Ready(Some(next_message)) = self.outbound_rx.poll_recv(cx) {
                        if let Err(err) = self.socket.start_send_unpin(next_message) {
                            return Poll::Ready(Err(WebsocketClientError::TransportError(err.to_string())));
                        }

                        should_flush = true;
                    } else if should_flush {
                        // We've sent out some messages, now we need to flush.
                        return self
                            .socket
                            .poll_flush_unpin(cx)
                            .map_err(|err| WebsocketClientError::TransportError(err.to_string()));
                    } else {
                        return Poll::Pending;
                    }
                },

                Poll::Ready(Err(err)) => {
                    return Poll::Ready(Err(WebsocketClientError::TransportError(err.to_string())))
                },

                // The sink is not ready.
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl Stream for ClientStream {
    type Item = StreamEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.is_terminated() {
            return Poll::Ready(None);
        }

        while let Poll::Ready(data) = self.socket.poll_next_unpin(cx) {
            match data {
                Some(result) => {
                    if let Some(event) =
                        self.parse_inbound(result.map_err(|err| WebsocketClientError::TransportError(err.to_string())))
                    {
                        return Poll::Ready(Some(event));
                    }
                },

                None => {
                    self.socket_ended = true;
                    return Poll::Ready(Some(StreamEvent::ConnectionClosed(self.close_frame.clone())));
                },
            }
        }

        match self.poll_write(cx) {
            Poll::Ready(Err(error)) => {
                self.socket_ended = true;
                Poll::Ready(Some(StreamEvent::OutboundError(
                    WebsocketClientError::TransportError(error.to_string()).into(),
                )))
            },

            _ => Poll::Pending,
        }
    }
}

impl FusedStream for ClientStream {
    fn is_terminated(&self) -> bool { self.socket_ended }
}

impl Drop for ClientStream {
    fn drop(&mut self) {
        let reason = CloseReason(self.close_frame.take());

        for (_, tx) in self.requests.drain() {
            tx.send(Err(
                WebsocketClientError::ConnectionClosed(reason.clone().to_string()).into()
            ))
            .ok();
        }
    }
}
