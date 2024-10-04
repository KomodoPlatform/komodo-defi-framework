use super::connection_manager::ConnectionManager;
use super::constants::BLOCKCHAIN_SCRIPTHASH_SUB_ID;

use crate::utxo::ScripthashNotification;
use crate::RpcTransportEventHandler;
use common::jsonrpc_client::JsonRpcRequest;
use common::log::{error, warn};

use futures::channel::mpsc::UnboundedSender;
use serde_json::{self as json, Value as Json};

/// An `RpcTransportEventHandler` that forwards `ScripthashNotification`s to trigger balance updates.
///
/// This handler hooks in `on_incoming_response` and looks for an electrum script hash notification to forward it.
pub struct ElectrumScriptHashNotificationBridge {
    pub scripthash_notification_sender: UnboundedSender<ScripthashNotification>,
}

impl RpcTransportEventHandler for ElectrumScriptHashNotificationBridge {
    fn debug_info(&self) -> String { "ElectrumScriptHashNotificationBridge".into() }

    fn on_incoming_response(&self, data: &[u8]) {
        if let Ok(raw_json) = json::from_slice::<Json>(data) {
            // Try to parse the notification. A notification is sent as a JSON-RPC request.
            if let Ok(notification) = json::from_value::<JsonRpcRequest>(raw_json) {
                // Only care about `BLOCKCHAIN_SCRIPTHASH_SUB_ID` notifications.
                if notification.method.as_str() == BLOCKCHAIN_SCRIPTHASH_SUB_ID {
                    if let Some(scripthash) = notification.params.first().and_then(|s| s.as_str()) {
                        if let Err(e) = self
                            .scripthash_notification_sender
                            .unbounded_send(ScripthashNotification::Triggered(scripthash.to_string()))
                        {
                            error!("Failed sending script hash message. {e:?}");
                        }
                    } else {
                        warn!("Notification must contain the script hash value, got: {notification:?}");
                    }
                };
            }
        }
    }

    fn on_connected(&self, _address: &str) -> Result<(), String> { Ok(()) }

    fn on_disconnected(&self, _address: &str) -> Result<(), String> { Ok(()) }

    fn on_outgoing_request(&self, _data: &[u8]) {}
}

/// An `RpcTransportEventHandler` that notifies the `ConnectionManager` upon connections and  disconnections.
///
/// When a connection is connected or disconnected, this event handler will notify the `ConnectionManager`
/// to handle the the event.
pub struct ElectrumConnectionManagerNotifier {
    pub connection_manager: ConnectionManager,
}

impl RpcTransportEventHandler for ElectrumConnectionManagerNotifier {
    fn debug_info(&self) -> String { "ElectrumConnectionManagerNotifier".into() }

    fn on_connected(&self, address: &str) -> Result<(), String> {
        self.connection_manager.on_connected(address);
        Ok(())
    }

    fn on_disconnected(&self, address: &str) -> Result<(), String> {
        self.connection_manager.on_disconnected(address);
        Ok(())
    }

    fn on_incoming_response(&self, _data: &[u8]) {}

    fn on_outgoing_request(&self, _data: &[u8]) {}
}
