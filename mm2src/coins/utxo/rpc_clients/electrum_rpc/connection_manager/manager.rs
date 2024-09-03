use std::collections::{BTreeMap, HashMap};
use std::iter::FromIterator;
use std::sync::{Arc, Mutex, RwLock, Weak};

use super::super::client::{ElectrumClient, ElectrumClientImpl};
use super::super::connection::{ElectrumConnection, ElectrumConnectionErr, ElectrumConnectionSettings};
use super::super::constants::{BACKGROUND_TASK_WAIT_TIMEOUT, PING_INTERVAL};
use super::connection_context::ConnectionContext;

use crate::utxo::rpc_clients::UtxoRpcClientOps;
use common::executor::abortable_queue::AbortableQueue;
use common::executor::{AbortableSystem, SpawnFuture, Timer};
use common::notifier::{Notifiee, Notifier};
use common::now_ms;
use futures::stream::FuturesUnordered;
use keys::Address;

use futures::compat::Future01CompatExt;
use futures::{FutureExt, StreamExt};
use instant::Instant;

/// A macro to unwrap an option and *execute* some code if the option is None.
macro_rules! unwrap_or_else {
    ($option:expr, $($action:tt)*) => {{
        let Some(some_val) = $option else {
            $($action)*;
        };
        some_val
    }};
}

macro_rules! unwrap_or_continue {
    ($option:expr) => {
        unwrap_or_else!($option, continue)
    };
}

macro_rules! unwrap_or_return {
    ($option:expr, $ret:expr) => {
        unwrap_or_else!($option, return $ret)
    };
    ($option:expr) => {
        unwrap_or_else!($option, return)
    };
}

/// The ID of a connection (and also its priority, lower is better).
type ID = u32;

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

/// The configuration parameter for a connection manager.
#[derive(Debug)]
struct ManagerConfig {
    /// A flag to spawn a ping loop task for active connections.
    spawn_ping: bool,
    /// The minimum number of connections that should be connected at all times.
    min_connected: u32,
    /// The maximum number of connections that can be connected at any given time.
    max_connected: u32,
}

#[derive(Debug)]
/// A connection manager that maintains a set of connections to electrum servers and
/// handles reconnecting, address subscription distribution, etc...
struct ConnectionManagerImpl {
    /// The configuration for the connection manager.
    config: ManagerConfig,
    /// The set of addresses that are currently connected.
    ///
    /// This set's size should satisfy: `min_connected <= maintained_connections.len() <= max_connected`.
    ///
    /// It is actually represented as a sorted map from connection ID (u32, also represents connection priority)
    /// to address so we can easily/cheaply pop low priority connections and add high priority ones.
    maintained_connections: RwLock<BTreeMap<ID, String>>,
    /// A map for server addresses to their corresponding connections.
    connections: HashMap<String, ConnectionContext>,
    /// A weak reference to the electrum client that owns this connection manager.
    /// It is used to send electrum requests during connection establishment (version querying).
    // TODO: This field might not be necessary if [`ElectrumConnection`] object be used to send
    // electrum requests on its own, i.e. implement [`JsonRpcClient`] & [`UtxoRpcClientOps`].
    electrum_client: RwLock<Option<Weak<ElectrumClientImpl>>>,
    /// A notification sender to notify the background task when we have less than `min_connected` connections.
    below_min_connected_notifier: Notifier,
    /// A notification receiver to be used by the background task to receive notifications of when
    /// we have less than `min_connected` maintained connections.
    ///
    /// Wrapped inside a Mutex<Option< to be taken out when the background task is spawned.
    below_min_connected_notifiee: Mutex<Option<Notifiee>>,
}

#[derive(Clone, Debug)]
pub struct ConnectionManager(Arc<ConnectionManagerImpl>);

impl ConnectionManager {
    pub fn try_new(
        servers: Vec<ElectrumConnectionSettings>,
        spawn_ping: bool,
        (min_connected, max_connected): (u32, u32),
        abortable_system: &AbortableQueue,
    ) -> Result<Self, String> {
        let mut connections = HashMap::with_capacity(servers.len());
        // Priority is assumed to be the order of the servers in the list as they appear.
        for (priority, connection_settings) in servers.into_iter().enumerate() {
            let subsystem = abortable_system.create_subsystem().map_err(|e| {
                ERRL!(
                    "Failed to create abortable subsystem for connection: {}, error: {:?}",
                    connection_settings.url,
                    e
                )
            })?;
            let connection = ElectrumConnection::new(connection_settings, subsystem);
            connections.insert(
                connection.address().to_string(),
                ConnectionContext::new(connection, priority as u32),
            );
        }

        if min_connected == 0 {
            ERRL!("min_connected should be greater than 0");
        }
        if min_connected > max_connected {
            ERRL!(
                "min_connected ({}) must be <= max_connected ({})",
                min_connected,
                max_connected
            );
        }

        let (notifier, notifiee) = Notifier::new();
        Ok(ConnectionManager(Arc::new(ConnectionManagerImpl {
            config: ManagerConfig {
                spawn_ping,
                min_connected,
                max_connected,
            },
            connections,
            maintained_connections: RwLock::new(BTreeMap::new()),
            electrum_client: RwLock::new(None),
            below_min_connected_notifier: notifier,
            below_min_connected_notifiee: Mutex::new(Some(notifiee)),
        })))
    }

    /// Initializes the connection manager by connecting the electrum connections.
    /// This must be called and only be called once to have a functioning connection manager.
    pub fn initialize(&self, weak_client: Weak<ElectrumClientImpl>) -> Result<(), ConnectionManagerErr> {
        // Disallow reusing the same manager with another client.
        if self.weak_client().read().unwrap().is_some() {
            return Err(ConnectionManagerErr::AlreadyInitialized);
        }

        let electrum_client = unwrap_or_return!(weak_client.upgrade(), Err(ConnectionManagerErr::NoClient));

        // Store the (weak) electrum client.
        *self.weak_client().write().unwrap() = Some(weak_client);

        // Use the client's spawner to spawn the connection manager's background task.
        electrum_client.weak_spawner().spawn(self.clone().background_task());

        if self.config().spawn_ping {
            // Use the client's spawner to spawn the connection manager's ping task.
            electrum_client.weak_spawner().spawn(self.clone().ping_task());
        }

        Ok(())
    }

    // Abstractions over the accesses of the inner fields of the connection manager.
    #[inline]
    fn config(&self) -> &ManagerConfig { &self.0.config }

    #[inline]
    fn connections(&self) -> &HashMap<String, ConnectionContext> { &self.0.connections }

    #[inline]
    fn weak_client(&self) -> &RwLock<Option<Weak<ElectrumClientImpl>>> { &self.0.electrum_client }

    #[inline]
    fn maintained_connections(&self) -> &RwLock<BTreeMap<ID, String>> { &self.0.maintained_connections }

    #[inline]
    fn notify_below_min_connected(&self) { self.0.below_min_connected_notifier.notify().ok(); }

    #[inline]
    fn extract_below_min_connected_notifiee(&self) -> Option<Notifiee> {
        self.0.below_min_connected_notifiee.lock().unwrap().take()
    }

    /// Attempts to upgrades and return the weak client reference we hold. If None is returned this means either
    /// the client was never initialized (initializing will fix this), or the client was dropped, which in this case
    /// the connection manager is no longer usable.
    fn get_client(&self) -> Option<ElectrumClient> {
        self.weak_client()
            .read()
            .unwrap()
            .as_ref()
            .and_then(|weak| weak.upgrade().map(ElectrumClient))
    }

    /// Returns all the server addresses.
    pub fn get_all_server_addresses(&self) -> Vec<String> { self.connections().keys().cloned().collect() }

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

    /// Retrieve a specific electrum connection by its address.
    /// The connection will be forcibly established if it's disconnected.
    pub async fn get_connection_by_address(
        &self,
        server_address: &str,
        force_connect: bool,
    ) -> Result<Arc<ElectrumConnection>, ConnectionManagerErr> {
        let connection = self
            .get_connection(server_address)
            .ok_or(ConnectionManagerErr::UnknownAddress)?;

        if force_connect {
            let client = unwrap_or_return!(self.get_client(), Err(ConnectionManagerErr::NoClient));
            // Make sure the connection is connected.
            ElectrumConnection::establish_connection_loop(connection.clone(), client)
                .await
                .map_err(ConnectionManagerErr::ConnectingError)?;
        }

        Ok(connection)
    }

    /// Returns a list of active/maintained connections.
    pub async fn get_active_connections(&self) -> Vec<Arc<ElectrumConnection>> {
        self.maintained_connections()
            .read()
            .unwrap()
            .iter()
            .filter_map(|(_id, address)| self.get_connection(address))
            .collect()
    }

    /// Waits until the connection manager is connected to the electrum server.
    pub async fn wait_till_connected(&self, timeout: f32) -> Result<(), String> {
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

    /// Returns a boolean value indicating whether the connections pool is empty (true)
    /// or not (false).
    pub async fn is_connections_pool_empty(&self) -> bool {
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
    pub async fn add_subscriptions(&self, addresses: &HashMap<String, Address>) {
        for (scripthash, address) in addresses.iter() {
            // For a single address/scripthash, keep trying to subscribe it until we succeed.
            'single_address_sub: loop {
                let client = unwrap_or_return!(self.get_client());
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
                        let connection_ctx = unwrap_or_continue!(self.connections().get(connection.address()));
                        connection_ctx.add_sub(address.clone());
                        break 'single_address_sub;
                    }
                }
            }
        }
    }

    // Handles the connection event.
    pub fn on_connected(&self, server_address: &str) {
        let connection_ctx = unwrap_or_return!(self.connections().get(server_address));

        // Reset the suspend time & disconnection time.
        connection_ctx.connected();
    }

    // Handles the disconnection event from an Electrum server.
    pub fn on_disconnected(&self, server_address: &str) {
        let connection_ctx = unwrap_or_return!(self.connections().get(server_address));

        if self
            .maintained_connections()
            .read()
            .unwrap()
            .contains_key(&connection_ctx.id)
        {
            // If the connection was maintained, remove it from the maintained connections.
            let mut maintained_connections = self.maintained_connections().write().unwrap();
            maintained_connections.remove(&connection_ctx.id);
            // And notify the background task if we fell below the `min_connected` threshold.
            if (maintained_connections.len() as u32) < self.config().min_connected {
                self.notify_below_min_connected()
            }
        }

        let abandoned_subs = connection_ctx.disconnected();
        // Re-subscribe the abandoned addresses using the client.
        let client = unwrap_or_return!(self.get_client());
        client.subscribe_addresses(abandoned_subs).ok();
    }

    /// Remove a connection from the connection manager by its address.
    // TODO(feat): Add the ability to add a connection during runtime.
    pub async fn remove_connection(
        &self,
        server_address: &str,
    ) -> Result<Arc<ElectrumConnection>, ConnectionManagerErr> {
        let connection = self
            .get_connection(server_address)
            .ok_or(ConnectionManagerErr::UnknownAddress)?;
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

    /// A forever-lived task that pings active/maintained connections periodically.
    async fn ping_task(self) {
        loop {
            let client = unwrap_or_return!(self.get_client());
            // This will ping all the active/maintained connections, which will keep these connections alive.
            client.server_ping().compat().await.ok();
            Timer::sleep(PING_INTERVAL).await;
        }
    }

    /// A forever-lived task that does the house keeping tasks of the connection manager:
    ///     - Maintaining the right number of active connections.
    ///     - Establishing new connections if needed.
    ///     - Replacing low priority connections with high priority ones periodically.
    ///     - etc...
    async fn background_task(self) {
        // Take out the min_connected notifiee from the manager.
        let mut min_connected_notification = unwrap_or_return!(self.extract_below_min_connected_notifiee());
        loop {
            // Get the candidate connections that we will consider maintaining.
            let (will_never_get_min_connected, candidate_connections, currently_connected) = {
                let maintained_connections = self.maintained_connections().read().unwrap();
                let currently_connected = maintained_connections.len() as u32;
                // The number of connections we need to add as maintained to reach the `min_connected` threshold.
                let connections_needed = self.config().min_connected.saturating_sub(currently_connected);
                // The connections that we can consider (all connections - candidate connections).
                let all_candidate_connections: Vec<_> = self
                    .connections()
                    .iter()
                    .filter_map(|(_, conn_ctx)| {
                        (!maintained_connections.contains_key(&conn_ctx.id)).then(|| conn_ctx.connection.clone())
                    })
                    .collect();
                drop(maintained_connections);
                // The candidate connections from above, but further filtered by whether they are suspended or not.
                let non_suspended_candidate_connections: Vec<_> = all_candidate_connections
                    .iter()
                    .filter(|connection| {
                        self.connections()
                            .get(connection.address())
                            .map_or(false, |conn_ctx| now_ms() > conn_ctx.suspended_till())
                    })
                    .cloned()
                    .collect();
                // Decide which candidate connections to consider (all or only non-suspended).
                if connections_needed > non_suspended_candidate_connections.len() as u32 {
                    if connections_needed > all_candidate_connections.len() as u32 {
                        // Not enough connections to cover the `min_connected` threshold.
                        // This means we will never be able to maintain `min_connected` active connections.
                        (true, all_candidate_connections, currently_connected)
                    } else {
                        // If we consider all candidate connection (but some are suspended), we can cover the needed connections.
                        // We will consider the suspended ones since if we don't we will stay below `min_connected` threshold.
                        (false, all_candidate_connections, currently_connected)
                    }
                } else {
                    // Non suspended candidates are enough to cover the needed connections.
                    (false, non_suspended_candidate_connections, currently_connected)
                }
            };

            // Establish the connections to the selected candidates and alter the maintained connections set accordingly.
            {
                let client = unwrap_or_return!(self.get_client());
                // This is the maximum connections we can have being established concurrently.
                let allowed_concurrency =
                    self.config().max_connected.saturating_sub(currently_connected).max(1) as usize;
                let connection_loop_chunks = candidate_connections.chunks(allowed_concurrency).map(|chunk| {
                    FuturesUnordered::from_iter(chunk.iter().map(|connection| {
                        let address = connection.address().to_string();
                        ElectrumConnection::establish_connection_loop(connection.clone(), client.clone())
                            .map(|res| res.map(|_| address))
                    }))
                });
                // Pick each chunk of connections and establish them concurrently.
                for mut connection_loops in connection_loop_chunks {
                    while let Some(result) = connection_loops.next().await {
                        let Ok(address) = result else { continue; };
                        let conn_ctx = unwrap_or_continue!(self.connections().get(&address));
                        let maintained_connections = self.maintained_connections().read().unwrap();
                        let maintained_connections_size = maintained_connections.len() as u32;
                        let lowest_priority_connection_id =
                            *maintained_connections.keys().next_back().unwrap_or(&u32::MAX);
                        // NOTE: Must drop to avoid deadlock with the write lock below.
                        drop(maintained_connections);
                        // We don't write-lock the maintained connections unless we know we will add this connection.
                        // That is, we can add it because we didn't hit the `max_connected` threshold,
                        // or we can add it because it is of a higher priority than the lowest priority connection.
                        if maintained_connections_size < self.config().max_connected
                            || conn_ctx.id < lowest_priority_connection_id
                        {
                            let mut maintained_connections = self.maintained_connections().write().unwrap();
                            maintained_connections.insert(conn_ctx.id, address);
                            // If we have reached the `max_connected` threshold then remove the lowest priority connection.
                            if !maintained_connections_size < self.config().max_connected {
                                maintained_connections.remove(&lowest_priority_connection_id);
                            }
                        }
                    }
                }
            }

            // Only sleep if we successfully acquired the minimum number of connections,
            // or if we know we can never maintain `min_connected` connections; there is no point of infinite non-wait looping then.
            if self.maintained_connections().read().unwrap().len() as u32 >= self.config().min_connected
                || will_never_get_min_connected
            {
                // Wait for a timeout or a below `min_connected` notification before doing another round of house keeping.
                futures::select! {
                    _ = Timer::sleep(BACKGROUND_TASK_WAIT_TIMEOUT).fuse() => (),
                    _ = min_connected_notification.wait().fuse() => (),
                }
            }
        }
    }
}
