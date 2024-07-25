use super::outbound::OutboundRequest;
use super::stream::{open_new_relay_connection_stream, ClientStream, StreamEvent};

use crate::client::{ConnectionHandler, PublishedMessage};
use crate::error::{ClientError, WebsocketClientError};
use crate::HttpRequest;

use futures::SinkExt;
use futures_util::stream::{FusedStream, Stream, StreamExt};
use futures_util::FutureExt;
use http::request;
use std::{f32::consts::PI, task::Poll};
use tokio::select;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;

pub(crate) type TxSender = oneshot::Sender<Result<(), ClientError>>;

pub(crate) enum ConnectionControl {
    Connect { request: HttpRequest<()>, tx: TxSender },
    Disconnect { tx: TxSender },
    OutboundRequest(OutboundRequest),
}

pub(crate) async fn connection_event_loop<T>(mut control_rx: UnboundedReceiver<ConnectionControl>, mut handler: T)
where
    T: ConnectionHandler,
{
    let mut connection = Connection::new();
    loop {
        select! {
            event = control_rx.recv() => {
                match event {
                    Some(event) => {
                        match event {
                            ConnectionControl::Connect {request, tx} => {
                                let result = connection.connect(&request).await;
                                if result.is_ok(){
                                    handler.connected();
                                }
                                tx.send(result);
                            },
                            ConnectionControl::Disconnect{tx} => {
                                tx.send(connection.disconnect().await);
                            },
                            ConnectionControl::OutboundRequest(request) => {
                                connection.request(request).await;
                            }
                        }
                    },
                    None => {
                        connection.disconnect().await.ok();
                        handler.disconnected(None);
                        break;
                    }
                }
            },
            event =  connection.select_next_some() => {
                match event {
                    StreamEvent::InboundSubscriptionRequest(request) => {
                        handler.message_received(PublishedMessage::from(&request));
                        request.respond(Ok(true)).ok();
                    }

                    StreamEvent::InboundError(error) => {
                        handler.inbound_error(error);
                    }

                    StreamEvent::OutboundError(error) => {
                        handler.outbound_error(error);
                    }

                    StreamEvent::ConnectionClosed(frame) => {
                        handler.disconnected(frame);
                        connection.reset().await;
                    }
                }
            }
        }
    }
}

pub(crate) struct Connection(Option<ClientStream>);
impl Connection {
    fn new() -> Self { Self(None) }

    async fn connect(&mut self, request: &HttpRequest<()>) -> Result<(), ClientError> {
        if let Some(mut stream) = self.0.take() {
            stream.close().await?;
        }

        self.0 = Some(open_new_relay_connection_stream(request).await?);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), ClientError> {
        let stream = self.0.take();
        if let Some(mut stream) = stream {
            stream.close().await?;
        }

        Err(WebsocketClientError::TransportError("Error while closing connection".to_owned()).into())
    }

    async fn request(&mut self, request: OutboundRequest) {
        match self.0.as_mut() {
            Some(stream) => stream.send_raw(request),
            None => {
                request.tx.send(Err(WebsocketClientError::NotConnected.into())).ok();
            },
        }
    }

    async fn reset(&mut self) { self.0 = None }
}

impl Stream for Connection {
    type Item = StreamEvent;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if let Some(stream) = &mut self.0 {
            if stream.is_terminated() {
                self.0 = None;
                Poll::Pending
            } else {
                stream.poll_next_unpin(cx)
            }
        } else {
            Poll::Pending
        }
    }
}

impl FusedStream for Connection {
    fn is_terminated(&self) -> bool { false }
}
