use std::collections::HashMap;
use std::sync::{Arc, RwLock, Weak};

use super::super::client::ElectrumClientImpl;
use super::super::connection::{ElectrumConnection, ElectrumConnectionSettings};
use super::super::constants::PING_INTERVAL;
use super::connection_context::ConnectionContext;
use super::ConnectionManagerTrait;

use common::executor::abortable_queue::{AbortableQueue, WeakSpawner};
use common::executor::{AbortableSystem, SpawnFuture, Timer};
use common::log::warn;
use common::now_ms;

use async_trait::async_trait;
use futures::compat::Future01CompatExt;

#[derive(Debug)]
pub struct ConnectionManagerMultiple {
    /// A flag to spawn a ping loop task for active connections.
    spawn_ping: bool,
    /// A map for server addresses to their corresponding connections.
    connections: HashMap<String, ConnectionContext>,
    /// A weak reference to the electrum client that owns this connection manager.
    /// It is used to send electrum requests during connection establishment (version querying).
    // TODO: This field might not be necessary if [`ElectrumConnection`] object be used to send
    // electrum requests on its own, i.e. implement [`JsonRpcClient`] & [`UtxoRpcClientOps`].
    electrum_client: RwLock<Option<Weak<ElectrumClientImpl>>>,
}

impl ConnectionManagerMultiple {
    pub fn try_new_arc(
        servers: Vec<ElectrumConnectionSettings>,
        spawn_ping: bool,
        abortable_system: &AbortableQueue,
    ) -> Result<Arc<Self>, String> {
        let mut connections = HashMap::with_capacity(servers.len());
        for connection_settings in servers {
            let subsystem = abortable_system.create_subsystem().map_err(|e| {
                ERRL!(
                    "Failed to create abortable subsystem for connection: {}, error: {:?}",
                    connection_settings.url,
                    e
                )
            })?;

            let connection = ElectrumConnection::new(connection_settings, subsystem);
            connections.insert(connection.address().to_string(), ConnectionContext::new(connection));
        }

        Ok(Arc::new(ConnectionManagerMultiple {
            spawn_ping,
            connections,
            electrum_client: RwLock::new(None),
        }))
    }
}

#[async_trait]
impl ConnectionManagerTrait for Arc<ConnectionManagerMultiple> {
    fn copy(&self) -> Box<dyn ConnectionManagerTrait> { Box::new(self.clone()) }

    fn spawn_ping(&self) -> bool { self.spawn_ping }

    fn connections(&self) -> &HashMap<String, ConnectionContext> { &self.connections }

    fn weak_client(&self) -> &RwLock<Option<Weak<ElectrumClientImpl>>> { &self.electrum_client }

    async fn get_active_connections(&self) -> Vec<Arc<ElectrumConnection>> {
        let mut connections = vec![];
        for connection in self.get_all_connections() {
            if connection.is_connected().await {
                connections.push(connection);
            }
        }
        connections
    }

    fn on_disconnected(&self, server_address: &str) {
        let Some(connection_ctx) = self.connections.get(server_address) else {
            warn!("No connection found for address: {server_address}");
            return;
        };
        let abandoned_subs = connection_ctx.disconnected();
        // Re-subscribe the abandoned addresses using the client.
        if let Some(client) = self.get_client() {
            client.subscribe_addresses(abandoned_subs).ok();
        };
    }

    fn spawn_ping_task(&self, spawner: &WeakSpawner) {
        let manager = self.clone();
        let task = async move {
            loop {
                let Some(client) = manager.get_client() else { break };
                // This will ping all the active connections, which will keep these connections alive.
                client.server_ping().compat().await.ok();
                Timer::sleep(PING_INTERVAL).await;
            }
        };
        spawner.spawn(task);
    }

    fn spawn_background_task(&self, spawner: &WeakSpawner) {
        let manager = self.clone();
        let task = async move {
            loop {
                let Some(client) = manager.get_client() else { return };
                // If no connection is active, just fire up all the connections we have, we shouldn't operate
                // with no connections at all.
                if manager.get_active_connections().await.is_empty() {
                    for connection in manager.get_all_connections() {
                        ElectrumConnection::establish_connection_loop(connection, client.clone())
                            .await
                            .ok();
                    }
                    continue;
                }
                // Check for any disconnected connection that are due to being connected.
                for connection in manager.get_all_connections() {
                    if !connection.is_connected().await {
                        let Some(connection_ctx) = manager.connections.get(connection.address()) else { continue };
                        // Try to connect the server if it's time to wake it up.
                        if now_ms() >= connection_ctx.suspend_until() {
                            ElectrumConnection::establish_connection_loop(connection, client.clone())
                                .await
                                .ok();
                        }
                    }
                }
                drop(client);

                // Sleep for 5 seconds before checking again.
                Timer::sleep(5.).await;
            }
        };
        spawner.spawn(task);
    }
}
