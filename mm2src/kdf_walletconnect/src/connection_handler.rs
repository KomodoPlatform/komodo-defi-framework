use futures::StreamExt;
use std::sync::Arc;

use common::{executor::Timer,
             log::{error, info}};
use futures::channel::mpsc::UnboundedSender;
use relay_client::{error::ClientError,
                   websocket::{CloseFrame, ConnectionHandler, PublishedMessage}};

use crate::WalletConnectCtx;

pub struct Handler {
    name: &'static str,
    msg_sender: UnboundedSender<PublishedMessage>,
    conn_live_sender: UnboundedSender<()>,
}

impl Handler {
    pub fn new(
        name: &'static str,
        msg_sender: UnboundedSender<PublishedMessage>,
        conn_live_sender: UnboundedSender<()>,
    ) -> Self {
        Self {
            name,
            msg_sender,
            conn_live_sender,
        }
    }
}

impl ConnectionHandler for Handler {
    fn connected(&mut self) {
        info!("\n[{}] connection open", self.name);
    }

    fn disconnected(&mut self, frame: Option<CloseFrame<'static>>) {
        info!("\n[{}] connection closed: frame={frame:?}", self.name);

        if let Err(e) = self.conn_live_sender.start_send(()) {
            info!("\n[{}] failed to send to the receiver: {e}", self.name);
        }
    }

    fn message_received(&mut self, message: PublishedMessage) {
        info!(
            "\n[{}] inbound message: message_id={} topic={} tag={} message={}",
            self.name, message.message_id, message.topic, message.tag, message.message,
        );

        if let Err(e) = self.msg_sender.start_send(message) {
            info!("\n[{}] failed to send to the receiver: {e}", self.name);
        }
    }

    fn inbound_error(&mut self, error: ClientError) {
        info!("\n[{}] inbound error: {error}", self.name);
    }

    fn outbound_error(&mut self, error: ClientError) {
        info!("\n[{}] outbound error: {error}", self.name);
    }
}

const INITIAL_RETRY_SECS: f64 = 5.0;
const RETRY_INCREMENT: f64 = 5.0;
const RECONNECT_DELAY: f64 = 5.0;

pub(crate) async fn maintain_client_connection(this: Arc<WalletConnectCtx>) {
    initial_connection(&this).await;
    handle_disconnections(&this).await;
}

async fn initial_connection(this: &WalletConnectCtx) {
    let mut retry_count = 0;
    let mut retry_secs = INITIAL_RETRY_SECS;

    while let Err(err) = this.connect_client().await {
        retry_count += 1;
        error!(
            "Error during initial connection attempt {}: {:?}. Retrying in {retry_secs} seconds...",
            retry_count, err
        );
        Timer::sleep(retry_secs).await;
        retry_secs += RETRY_INCREMENT;
    }

    info!("Successfully connected to client after {} attempt(s).", retry_count + 1);

    // load session from storage
    if let Err(err) = this.load_session_from_storage().await {
        error!("Unable to load session from storage: {err:?}");
    };
}

async fn handle_disconnections(this: &WalletConnectCtx) {
    let mut recv = this.connection_live_handler.lock().await;

    while let Some(_msg) = recv.next().await {
        info!("Connection disconnected. Attempting to reconnect...");
        reconnect(this).await;
        resubscribe_to_topics(this).await;
        info!("Reconnection process complete.");
    }
}

async fn reconnect(this: &WalletConnectCtx) {
    let mut retry_count = 0;

    while let Err(err) = this.connect_client().await {
        retry_count += 1;
        error!(
            "Error while reconnecting to client (attempt {}): {:?}. Retrying in {} seconds...",
            retry_count, err, RECONNECT_DELAY
        );
        Timer::sleep(RECONNECT_DELAY).await;
    }

    info!(
        "Successfully reconnected to client after {} attempt(s).",
        retry_count + 1
    );
}

async fn resubscribe_to_topics(this: &WalletConnectCtx) {
    let subs = this.subscriptions.lock().await;
    for topic in &*subs {
        match this.client.subscribe(topic.clone()).await {
            Ok(_) => info!("Successfully reconnected to topic: {:?}", topic),
            Err(err) => error!("Failed to subscribe to topic: {:?}. Error: {:?}", topic, err),
        }
    }
}
