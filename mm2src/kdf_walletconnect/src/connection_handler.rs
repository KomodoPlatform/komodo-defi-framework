use crate::storage::WalletConnectStorageOps;
use crate::WalletConnectCtxImpl;

use common::executor::Timer;
use common::log::{debug, error, info};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::StreamExt;
use relay_client::error::ClientError;
use relay_client::websocket::{CloseFrame, ConnectionHandler, PublishedMessage};
use std::sync::Arc;

const INITIAL_RETRY_SECS: f64 = 5.0;
const MAX_BACKOFF: u64 = 60;
const RETRY_INCREMENT: f64 = 5.0;

pub struct Handler {
    name: &'static str,
    msg_sender: UnboundedSender<PublishedMessage>,
    conn_live_sender: UnboundedSender<Option<String>>,
}

impl Handler {
    pub fn new(
        name: &'static str,
        msg_sender: UnboundedSender<PublishedMessage>,
        conn_live_sender: UnboundedSender<Option<String>>,
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
        debug!("[{}] connection to WalletConnect relay server successful", self.name);
    }

    fn disconnected(&mut self, frame: Option<CloseFrame<'static>>) {
        debug!("[{}] connection closed: frame={frame:?}", self.name);

        if let Err(e) = self.conn_live_sender.unbounded_send(frame.map(|f| f.to_string())) {
            error!("[{}] failed to send to the receiver: {e}", self.name);
        }
    }

    fn message_received(&mut self, message: PublishedMessage) {
        debug!(
            "[{}] inbound message: message_id={} topic={} tag={} message={}",
            self.name, message.message_id, message.topic, message.tag, message.message,
        );

        if let Err(e) = self.msg_sender.unbounded_send(message) {
            error!("[{}] failed to send to the receiver: {e}", self.name);
        }
    }

    fn inbound_error(&mut self, error: ClientError) {
        debug!("[{}] inbound error: {error}", self.name);
        if let Err(e) = self.conn_live_sender.unbounded_send(Some(error.to_string())) {
            error!("[{}] failed to send to the receiver: {e}", self.name);
        }
    }

    fn outbound_error(&mut self, error: ClientError) {
        debug!("[{}] outbound error: {error}", self.name);
        if let Err(e) = self.conn_live_sender.unbounded_send(Some(error.to_string())) {
            error!("[{}] failed to send to the receiver: {e}", self.name);
        }
    }
}

/// Establishes initial connection to WalletConnect relay server with linear retry mechanism.
/// Uses increasing delay between retry attempts starting from INITIAL_RETRY_SECS.
/// After successful connection, attempts to restore previous session state from storage.
pub(crate) async fn spawn_connection_initialization(
    wc: Arc<WalletConnectCtxImpl>,
    connection_live_rx: UnboundedReceiver<Option<String>>,
) {
    info!("Initializing WalletConnect connection");
    let mut retry_count = 0;
    let mut retry_secs = INITIAL_RETRY_SECS;

    while let Err(err) = wc.connect_client().await {
        retry_count += 1;
        error!(
            "Error during initial connection attempt {}: {:?}. Retrying in {retry_secs} seconds...",
            retry_count, err
        );
        Timer::sleep(retry_secs).await;
        retry_secs += RETRY_INCREMENT;
    }

    // Initialize storage
    if let Err(err) = wc.session_manager.storage().init().await {
        error!("Unable to initialize WalletConnect persistent storage: {err:?}. Only inmemory storage will be utilized for this Session.");
    };

    // load session from storage
    if let Err(err) = wc.load_session_from_storage().await {
        error!("Unable to load session from storage: {err:?}");
    };

    // Spawn session disconnection watcher.
    handle_disconnections(&wc, connection_live_rx).await;
}

/// Handles unexpected disconnections from WalletConnect relay server.
/// Implements exponential backoff retry mechanism for reconnection attempts.
/// After successful reconnection, resubscribes to previous topics to restore full functionality.
pub(crate) async fn handle_disconnections(
    this: &WalletConnectCtxImpl,
    mut connection_live_rx: UnboundedReceiver<Option<String>>,
) {
    let mut backoff = 1;

    while let Some(msg) = connection_live_rx.next().await {
        info!("WalletConnect disconnected with message: {msg:?}. Attempting to reconnect...");

        loop {
            match this.reconnect_and_subscribe().await {
                Ok(_) => {
                    info!("Reconnection process complete.");
                    backoff = 1;
                    break;
                },
                Err(e) => {
                    info!("Reconnection attempt failed: {:?}. Retrying in {:?}...", e, backoff);
                    Timer::sleep(backoff as f64).await;
                    backoff = std::cmp::min(backoff * 2, MAX_BACKOFF);
                },
            }
        }
    }
}
