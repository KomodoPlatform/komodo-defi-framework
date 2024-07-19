use std::collections::HashMap;

use crate::{error::{ClientError, WebsocketClientError},
            HttpRequest, MessageIdGenerator};
use futures_channel::mpsc::unbounded;
use relay_rpc::{domain::MessageId, rpc::Subscription};
use tokio::{net::TcpStream,
            sync::{mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
                   oneshot}};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::{inbound::InboundRequest, outbound::OutboundRequest};

pub type SocketStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Opens a connection to the Relay and returns [`ClientStream`] for the
/// connection.
pub async fn create_stream(request: HttpRequest<()>) -> Result<ClientStream, WebsocketClientError> {
    let (socket, _) = connect_async(request)
        .await
        .map_err(|_| WebsocketClientError::ConnectionFailed)?;

    Ok(ClientStream::new(socket))
}

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
    socket: SocketStream,
    outbound_tx: UnboundedSender<Message>,
    outbound_rx: UnboundedReceiver<Message>,
    requests: HashMap<MessageId, oneshot::Sender<Result<serde_json::Value, ClientError>>>,
    id_generator: MessageIdGenerator,
    close_frame: Option<CloseFrame<'static>>,
}

impl ClientStream {
    fn new(socket: SocketStream) -> Self {
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
        }
    }

    fn send_raw(&self, req: OutboundRequest) {
        let tx = req.tx;
        todo!()
    }
}
