use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::{Arc, RwLock, Weak};

use super::client::{ElectrumClient, ElectrumClientImpl};
use super::connection::{ElectrumConnection, ElectrumConnectionErr};
use connection_context::ConnectionContext;

use crate::utxo::rpc_clients::UtxoRpcClientOps;
use common::executor::abortable_queue::WeakSpawner;
use common::executor::Timer;
use common::log::warn;
use keys::Address;

use async_trait::async_trait;
use futures::compat::Future01CompatExt;
use instant::Instant;

mod connection_context;
mod multiple;
mod selective;

pub use multiple::ConnectionManagerMultiple;
pub use selective::ConnectionManagerSelective;

/// Trait provides a common interface to get an `ElectrumConnection` from the `ElectrumClient` instance
#[async_trait]
pub trait ConnectionManagerTrait: Debug + Send + Sync {
    /// A copy of the connection manager.
    /// This is a workaround for the non-clonability of the objects/structs implementing this trait.
    fn copy(&self) -> Box<dyn ConnectionManagerTrait>;

    /// Whether to spawn a ping loop or not.
    fn spawn_ping(&self) -> bool;

    /// A reference to the connections hashmap.
    fn connections(&self) -> &HashMap<String, ConnectionContext>;

    /// A reference to the mutex holding our weak client.
    fn weak_client(&self) -> &RwLock<Option<Weak<ElectrumClientImpl>>>;

    /// Used the provided spawner to spawn the manager's background task.
    fn spawn_background_task(&self, spawner: &WeakSpawner);

    /// Used the provided spawner to spawn a ping task (pings active connections periodically).
    fn spawn_ping_task(&self, spawner: &WeakSpawner);

    /// Attempts to upgrades and return the weak client reference we hold. If None is returned this means either
    /// the client was never initialized (initialized will fix this), or the client was dropped, which in this case
    /// the connection manager is no longer usable.
    fn get_client(&self) -> Option<ElectrumClient> {
        self.weak_client()
            .read()
            .unwrap()
            .as_ref()
            .and_then(|weak| weak.upgrade().map(ElectrumClient))
    }

    /// Returns an iterator over all the connections we have, even removed ones.
    fn get_all_connections(&self) -> Vec<Arc<ElectrumConnection>> {
        self.connections()
            .values()
            .map(|connection_ctx| connection_ctx.connection.clone())
            .collect()
    }

    /// Returns a connection by its address.
    fn get_connection(&self, server_address: &str) -> Option<Arc<ElectrumConnection>> {
        self.connections()
            .get(server_address)
            .map(|connection_ctx| connection_ctx.connection.clone())
    }

    /// Initializes the connection manager by connecting the electrum connections.
    /// This must be called and only be called once to have a functioning connection manager.
    fn initialize(&self, weak_client: Weak<ElectrumClientImpl>) -> Result<(), ConnectionManagerErr> {
        // Disallow reusing the same manager with another client.
        if self.weak_client().read().unwrap().is_some() {
            return Err(ConnectionManagerErr::AlreadyInitialized);
        }

        let Some(electrum_client) = weak_client.upgrade() else {
            return Err(ConnectionManagerErr::NoClient);
        };

        // Store the (weak) electrum client.
        *self.weak_client().write().unwrap() = Some(weak_client);

        // Use the client's spawner to spawn the connection manager's background task.
        self.spawn_background_task(&electrum_client.weak_spawner());

        if self.spawn_ping() {
            // Use the client's spawner to spawn the connection manager's ping task.
            self.spawn_ping_task(&electrum_client.weak_spawner());
        }

        Ok(())
    }

    /// Returns all the currently active connections.
    async fn get_active_connections(&self) -> Vec<Arc<ElectrumConnection>>;

    /// Waits until the connection manager is connected to the electrum server.
    async fn wait_till_connected(&self, timeout: f32) -> Result<(), String> {
        let start_time = Instant::now();
        loop {
            if !self.get_active_connections().await.is_empty() {
                return Ok(());
            }
            Timer::sleep(0.5).await;
            if start_time.elapsed().as_secs_f32() > timeout {
                return Err(format!(
                    "Waited for {} seconds but the connection manager is still not connected",
                    timeout
                ));
            }
        }
    }

    /// Returns all the server addresses.
    fn get_all_server_addresses(&self) -> Vec<String> { self.connections().keys().cloned().collect() }

    /// Retrieve a specific electrum connection by its address.
    /// The connection will be forcibly established if it's disconnected.
    async fn get_connection_by_address(
        &self,
        server_address: &str,
        force_connect: bool,
    ) -> Result<Arc<ElectrumConnection>, ConnectionManagerErr> {
        let connection = self
            .get_connection(server_address)
            .ok_or_else(|| ConnectionManagerErr::UnknownAddress)?;

        if force_connect {
            // Force connect the connection if it's not connected yet.
            if !connection.is_connected().await {
                if let Some(client) = self.get_client() {
                    ElectrumConnection::establish_connection_loop(connection.clone(), client)
                        .await
                        .map_err(ConnectionManagerErr::ConnectingError)?;
                } else {
                    return Err(ConnectionManagerErr::NoClient);
                }
            }
        }

        Ok(connection)
    }

    /// Returns a boolean value indicating whether the connections pool is empty (true)
    /// or not (false).
    async fn is_connections_pool_empty(&self) -> bool {
        // Since we don't remove the connections, but just set them as irrecoverable, we need
        // to check if all the connections are irrecoverable/not usable.
        for connection in self.get_all_connections() {
            if connection.usable().await {
                return false;
            }
        }
        true
    }

    /// Subscribe the list of addresses to our active connections.
    ///
    /// There is a bit of indirection here. We register the abandoned addresses on `on_disconnected` with
    /// the client to queue them for `utxo_balance_events` which in turn calls this method back to re-subscribe
    /// the abandoned addresses. We could have instead directly re-subscribed the addresses here in the connection
    /// manager without sending them to `utxo_balance_events`. However, we don't do that so that `utxo_balance_events`
    /// knows about all the added addresses. If it doesn't know about them, it won't be able to retrieve the triggered
    /// address when its script hash is notified.
    async fn add_subscriptions(&self, addresses: &HashMap<String, Address>) {
        for (scripthash, address) in addresses.iter() {
            'outer: loop {
                let Some(client) = self.get_client() else {
                    // The manager is either not initialized or the client is dropped.
                    // If the client is dropped, the manager is no longer usable.
                    return;
                };
                let connections = self.get_active_connections().await;
                if connections.is_empty() {
                    // If there are no active connections, wait for a connection to be established.
                    Timer::sleep(1.).await;
                    continue;
                }
                // Try to subscribe the address to any connection we have.
                for connection in connections {
                    if client
                        .blockchain_scripthash_subscribe_using(connection.address(), scripthash.clone())
                        .compat()
                        .await
                        .is_ok()
                    {
                        if let Some(connection_ctx) = self.connections().get(connection.address()) {
                            connection_ctx.add_sub(address.clone());
                            // Address subscribed successfully, move to the next one;
                            break 'outer;
                        }
                    }
                }
            }
        }
    }

    // Handles the connection event.
    fn on_connected(&self, server_address: &str) {
        let Some(connection_ctx) = self.connections().get(server_address) else {
            warn!("No connection found for address: {server_address}");
            return;
        };

        // Reset the suspend time & disconnection time.
        connection_ctx.connected();
    }

    // Handles the disconnection event from an Electrum server.
    fn on_disconnected(&self, server_address: &str);

    /// Remove a connection from the connection manager by its address.
    // TODO(feat): Add the ability to add a connection during runtime.
    async fn remove_connection(&self, server_address: &str) -> Result<Arc<ElectrumConnection>, ConnectionManagerErr> {
        let connection = self
            .get_connection(server_address)
            .ok_or_else(|| ConnectionManagerErr::UnknownAddress)?;
        // Make sure this connection is disconnected.
        connection
            // Note that setting the error as irrecoverable renders the connection unusable.
            // This is how we (virtually) remove the connection right now. We never delete the
            // connection though. This is done for now to avoid guarding the connection map with a mutex.
            .disconnect(Some(ElectrumConnectionErr::Irrecoverable(
                "Forcefully disconnected & removed".to_string(),
            )))
            .await;
        // Run the on-disconnection hook.
        self.on_disconnected(connection.address());
        Ok(connection.clone())
    }
}

#[derive(Debug, Display)]
pub enum ConnectionManagerErr {
    #[display(fmt = "Unknown server address")]
    UnknownAddress,
    #[display(fmt = "Failed to connect to the server due to {:?}", _0)]
    ConnectingError(ElectrumConnectionErr),
    #[display(fmt = "No client found, connection manager isn't initialized properly")]
    NoClient,
    #[display(fmt = "Connection manager is already initialized")]
    AlreadyInitialized,
}
