use common::log::{debug, error};
use futures::channel::mpsc::UnboundedSender;
use relay_client::error::ClientError;
use relay_client::websocket::{CloseFrame, ConnectionHandler, PublishedMessage};

pub(crate) const MAX_BACKOFF: u64 = 60;

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
