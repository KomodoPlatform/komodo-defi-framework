pub mod connection;
pub mod fetch;
pub mod inbound;
pub mod outbound;
pub mod stream;

use crate::common::executor::SpawnAbortable;
use crate::error::{ClientError, ServiceErrorExt};
use crate::{convert_subscription_result, ConnectionOptions};

use common::executor::{spawn_abortable, AbortOnDropHandle, AbortSettings};
use connection::connection_event_loop;
use connection::ConnectionControl;
use fetch::FetchMessageStream;
use futures::SinkExt;
use inbound::InboundRequest;
use mm2_core::mm_ctx::{MmArc, MmCtx};
use outbound::create_request;
use outbound::{EmptyResponseFuture, OutboundRequest, ResponseFuture};
use relay_rpc::domain::{MessageId, SubscriptionId, Topic};
use relay_rpc::rpc::{self, BatchFetchMessages, BatchReceiveMessages, BatchSubscribe, BatchSubscribeBlocking,
                     BatchUnsubscribe, FetchMessages, Publish, Receipt, Subscribe, SubscribeBlocking, Subscription,
                     SubscriptionError, Unsubscribe};
use std::borrow::BorrowMut;
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;
use tokio_tungstenite_wasm::CloseFrame;

type SubscriptionResult<T> = Result<T, ServiceErrorExt<SubscriptionError>>;

/// The message received from a subscription.
#[derive(Debug, Clone)]
pub struct PublishedMessage {
    pub message_id: MessageId,
    pub subscription_id: SubscriptionId,
    pub topic: Topic,
    pub message: Arc<str>,
    pub tag: u32,
    pub published_at: chrono::DateTime<chrono::Utc>,
    pub received_at: chrono::DateTime<chrono::Utc>,
}

impl From<&InboundRequest<Subscription>> for PublishedMessage {
    fn from(request: &InboundRequest<Subscription>) -> Self {
        let now = chrono::Utc::now();
        let Subscription {
            id: subscription_id,
            data,
        } = request.data();

        Self {
            message_id: request.id(),
            subscription_id: subscription_id.clone(),
            topic: data.topic.clone(),
            message: data.message.clone(),
            tag: data.tag,
            published_at: now,
            received_at: now,
        }
    }
}

/// Handlers for the RPC stream events.
pub trait ConnectionHandler: Send + 'static {
    /// Called when a connection to the Relay is established.
    fn connected(&mut self) {}
    /// Called when the Relay connection is closed.
    fn disconnected(&mut self, _frame: Option<CloseFrame<'static>>) {}
    /// Called when a message is received from the Relay.
    fn message_received(&mut self, message: PublishedMessage);
    /// Called when an inbound error occurs, such as data deserialization
    /// failure, or an unknown response message ID.
    fn inbound_error(&mut self, _error: ClientError) {}
    /// Called when an outbound error occurs, i.e. failed to write to the
    /// websocket stream.
    fn outbound_error(&mut self, _error: ClientError) {}
}

/// The Relay WebSocket RPC client.
///
/// This provides the high-level access to all of the available RPC methods. For
/// a lower-level RPC stream see [`ClientStream`](crate::client::ClientStream).
#[derive(Clone)]
pub struct Client(Arc<ClientImpl>);
impl Deref for Client {
    type Target = Arc<ClientImpl>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl DerefMut for Client {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.0 }
}

pub struct ClientImpl {
    control_tx: UnboundedSender<ConnectionControl>,
    handle: AbortOnDropHandle,
}

impl Client {
    pub fn new(ctx: &MmArc, handler: impl ConnectionHandler) -> Self {
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let settings = AbortSettings::info_on_abort("connection loop stopped for".to_owned());
        let abort = spawn_abortable(connection_event_loop(control_rx, handler));
        Self(Arc::new(ClientImpl {
            control_tx,
            handle: abort,
        }))
    }

    /// Publishes a message over the network on given topic.
    pub fn publish(
        &self,
        topic: Topic,
        message: impl Into<Arc<str>>,
        tag: u32,
        ttl: Duration,
        prompt: bool,
    ) -> EmptyResponseFuture<Publish> {
        let (request, response) = create_request(Publish {
            topic,
            message: message.into(),
            ttl_secs: ttl.as_secs() as u32,
            tag,
            prompt,
        });
        self.request(request);

        EmptyResponseFuture::new(response)
    }

    /// Subscribes on topic to receive messages. The request is resolved
    /// optimistically as soon as the relay receives it.
    pub fn subscribe(&self, topic: Topic) -> ResponseFuture<Subscribe> {
        let (request, response) = create_request(Subscribe { topic });
        self.request(request);

        response
    }

    /// Subscribes on topic to receive messages. The request is resolved only
    /// when fully processed by the relay.
    /// Note: This function is experimental and will likely be removed in the
    /// future.
    pub fn subscribe_blocking(&self, topic: Topic) -> ResponseFuture<SubscribeBlocking> {
        let (request, response) = create_request(SubscribeBlocking { topic });
        self.request(request);

        response
    }

    /// Unsubscribes from a topic.
    pub fn unsubscribe(&self, topic: Topic) -> EmptyResponseFuture<Unsubscribe> {
        let (request, response) = create_request(Unsubscribe { topic });
        self.request(request);

        EmptyResponseFuture::new(response)
    }

    /// Fetch mailbox messages for a specific topic.
    pub fn fetch(&self, topic: Topic) -> ResponseFuture<FetchMessages> {
        let (request, response) = create_request(FetchMessages { topic });
        self.request(request);

        response
    }

    /// Fetch mailbox messages for a specific topic. Returns a [`Stream`].
    pub fn fetch_stream(&self, topics: impl Into<Vec<Topic>>) -> FetchMessageStream {
        FetchMessageStream::new(self.clone(), topics.into())
    }

    /// Subscribes on multiple topics to receive messages. The request is
    /// resolved optimistically as soon as the relay receives it.
    pub fn batch_subscribe(&self, topics: impl Into<Vec<Topic>>) -> ResponseFuture<BatchSubscribe> {
        let (request, response) = create_request(BatchSubscribe { topics: topics.into() });

        self.request(request);

        response
    }

    /// Subscribes on multiple topics to receive messages. The request is
    /// resolved only when fully processed by the relay.
    /// Note: This function is experimental and will likely be removed in the
    /// future.
    pub fn batch_subscribe_blocking(
        &self,
        topics: impl Into<Vec<Topic>>,
    ) -> impl Future<Output = SubscriptionResult<Vec<SubscriptionResult<SubscriptionId>>>> {
        let (request, response) = create_request(BatchSubscribeBlocking { topics: topics.into() });
        self.request(request);

        async move {
            Ok(response
                .await?
                .into_iter()
                .map(convert_subscription_result)
                .map(|result| result.map_err(|err| ServiceErrorExt::Response(rpc::Error::TooManyRequests)))
                .collect())
        }
    }

    /// Unsubscribes from multiple topics.
    pub fn batch_unsubscribe(
        &self,
        subscriptions: impl Into<Vec<Unsubscribe>>,
    ) -> EmptyResponseFuture<BatchUnsubscribe> {
        let (request, response) = create_request(BatchUnsubscribe {
            subscriptions: subscriptions.into(),
        });
        self.request(request);

        EmptyResponseFuture::new(response)
    }

    /// Fetch mailbox messages for multiple topics.
    pub fn batch_fetch(&self, topics: impl Into<Vec<Topic>>) -> ResponseFuture<BatchFetchMessages> {
        let (request, response) = create_request(BatchFetchMessages { topics: topics.into() });
        self.request(request);

        response
    }

    /// Acknowledge receipt of messages from a subscribed client.
    pub fn batch_receive(&self, receipts: impl Into<Vec<Receipt>>) -> ResponseFuture<BatchReceiveMessages> {
        let (request, response) = create_request(BatchReceiveMessages {
            receipts: receipts.into(),
        });
        self.request(request);

        response
    }

    /// Opens a connection to the Relay.
    pub async fn connect(&self, opts: &ConnectionOptions) -> Result<(), ClientError> {
        let (tx, rx) = oneshot::channel();
        let request = opts.as_ws_request()?;

        if self.control_tx.send(ConnectionControl::Connect { request, tx }).is_ok() {
            rx.await.map_err(|err| ClientError::ChannelClosed)?
        } else {
            Err(ClientError::ChannelClosed)
        }
    }

    /// Closes the Relay connection.
    pub async fn disconnect(&self) -> Result<(), ClientError> {
        let (tx, rx) = oneshot::channel();

        if self.control_tx.send(ConnectionControl::Disconnect { tx }).is_ok() {
            rx.await.map_err(|_| ClientError::ChannelClosed)?
        } else {
            Err(ClientError::ChannelClosed)
        }
    }

    pub(crate) fn request(&self, request: OutboundRequest) {
        if let Err(err) = self.control_tx.send(ConnectionControl::OutboundRequest(request)) {
            let ConnectionControl::OutboundRequest(request) = err.0 else {
                unreachable!();
            };

            request.tx.send(Err(ClientError::ChannelClosed)).ok();
        }
    }
}
