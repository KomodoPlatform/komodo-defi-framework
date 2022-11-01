use super::*;
use common::executor::{spawn_abortable, SpawnFuture, Timer};
use common::log::LogState;
use derive_more::Display;
use lightning::chain::Access;
use lightning::ln::msgs::NetAddress;
use lightning::ln::peer_handler::{IgnoringMessageHandler, MessageHandler, SimpleArcPeerManager};
use lightning::routing::gossip;
use lightning_net_tokio::SocketDescriptor;
use mm2_net::ip_addr::fetch_external_ip;
use rand::RngCore;
use secp256k1v22::{PublicKey, SecretKey};
use std::net::{IpAddr, Ipv4Addr};
use tokio::net::TcpListener;

const TRY_RECONNECTING_TO_NODE_INTERVAL: f64 = 60.;
const BROADCAST_NODE_ANNOUNCEMENT_INTERVAL: u64 = 600;

pub type NetworkGossip = gossip::P2PGossipSync<Arc<NetworkGraph>, Arc<dyn Access + Send + Sync>, Arc<LogState>>;

pub type PeerManager =
    SimpleArcPeerManager<SocketDescriptor, ChainMonitor, Platform, Platform, dyn Access + Send + Sync, LogState>;

#[derive(Display)]
pub enum ConnectToNodeRes {
    #[display(fmt = "Already connected to node: {}@{}", pubkey, node_addr)]
    AlreadyConnected { pubkey: PublicKey, node_addr: SocketAddr },
    #[display(fmt = "Connected successfully to node : {}@{}", pubkey, node_addr)]
    ConnectedSuccessfully { pubkey: PublicKey, node_addr: SocketAddr },
}

#[derive(Display)]
pub enum ConnectionError {
    #[display(fmt = "Handshake error: {}", _0)]
    HandshakeErr(String),
    #[display(fmt = "Timeout error: {}", _0)]
    TimeOut(String),
}

pub async fn connect_to_ln_node(
    pubkey: PublicKey,
    node_addr: SocketAddr,
    peer_manager: Arc<PeerManager>,
) -> Result<ConnectToNodeRes, ConnectionError> {
    let peer_manager_ref = peer_manager.clone();
    let peer_node_ids = async_blocking(move || peer_manager_ref.get_peer_node_ids()).await;
    if peer_node_ids.contains(&pubkey) {
        return Ok(ConnectToNodeRes::AlreadyConnected { pubkey, node_addr });
    }

    let mut connection_closed_future =
        match lightning_net_tokio::connect_outbound(Arc::clone(&peer_manager), pubkey, node_addr).await {
            Some(fut) => Box::pin(fut),
            None => {
                return Err(ConnectionError::TimeOut(format!(
                    "Failed to connect to node: {}",
                    pubkey
                )))
            },
        };

    loop {
        // Make sure the connection is still established.
        match futures::poll!(&mut connection_closed_future) {
            std::task::Poll::Ready(_) => {
                return Err(ConnectionError::HandshakeErr(format!(
                    "Node {} disconnected before finishing the handshake",
                    pubkey
                )));
            },
            std::task::Poll::Pending => {},
        }

        let peer_manager = peer_manager.clone();
        let peer_node_ids = async_blocking(move || peer_manager.get_peer_node_ids()).await;
        if peer_node_ids.contains(&pubkey) {
            break;
        }

        // Wait for the handshake to complete
        Timer::sleep_ms(10).await;
    }

    Ok(ConnectToNodeRes::ConnectedSuccessfully { pubkey, node_addr })
}

pub async fn connect_to_ln_nodes_loop(open_channels_nodes: NodesAddressesMapShared, peer_manager: Arc<PeerManager>) {
    loop {
        let open_channels_nodes = open_channels_nodes.lock().clone();
        for (pubkey, node_addr) in open_channels_nodes {
            let peer_manager = peer_manager.clone();
            match connect_to_ln_node(pubkey, node_addr, peer_manager.clone()).await {
                Ok(res) => {
                    if let ConnectToNodeRes::ConnectedSuccessfully { .. } = res {
                        log::info!("{}", res.to_string());
                    }
                },
                Err(e) => log::error!("{}", e.to_string()),
            }
        }

        Timer::sleep(TRY_RECONNECTING_TO_NODE_INTERVAL).await;
    }
}

// TODO: add TOR address option
fn netaddress_from_ipaddr(addr: IpAddr, port: u16) -> Vec<NetAddress> {
    if addr == Ipv4Addr::new(0, 0, 0, 0) || addr == Ipv4Addr::new(127, 0, 0, 1) {
        return Vec::new();
    }
    let mut addresses = Vec::new();
    let address = match addr {
        IpAddr::V4(addr) => NetAddress::IPv4 {
            addr: u32::from(addr).to_be_bytes(),
            port,
        },
        IpAddr::V6(addr) => NetAddress::IPv6 {
            addr: u128::from(addr).to_be_bytes(),
            port,
        },
    };
    addresses.push(address);
    addresses
}

pub async fn ln_node_announcement_loop(
    channel_manager: Arc<ChannelManager>,
    node_name: [u8; 32],
    node_color: [u8; 3],
    port: u16,
) {
    loop {
        // Right now if the node is behind NAT the external ip is fetched on every loop
        // If the node does not announce a public IP, it will not be displayed on the network graph,
        // and other nodes will not be able to open a channel with it. But it can open channels with other nodes.
        let addresses = match fetch_external_ip().await {
            Ok(ip) => {
                log::debug!("Fetch real IP successfully: {}:{}", ip, port);
                netaddress_from_ipaddr(ip, port)
            },
            Err(e) => {
                log::error!("Error while fetching external ip for node announcement: {}", e);
                Timer::sleep(BROADCAST_NODE_ANNOUNCEMENT_INTERVAL as f64).await;
                continue;
            },
        };
        let channel_manager = channel_manager.clone();
        async_blocking(move || channel_manager.broadcast_node_announcement(node_color, node_name, addresses)).await;

        Timer::sleep(BROADCAST_NODE_ANNOUNCEMENT_INTERVAL as f64).await;
    }
}

async fn ln_p2p_loop(peer_manager: Arc<PeerManager>, listener: TcpListener) {
    // This container consists of abort handlers of the spawned inbound connections.
    // They will be dropped once `LightningCoin` is dropped because `ln_p2p_loop` is spawned
    // via [`Platform::abortable_system`].
    let mut spawned = Vec::new();
    loop {
        let peer_mgr = peer_manager.clone();
        let tcp_stream = match listener.accept().await {
            Ok((stream, addr)) => {
                log::debug!("New incoming lightning connection from node address: {}", addr);
                stream
            },
            Err(e) => {
                log::error!("Error on accepting lightning connection: {}", e);
                continue;
            },
        };
        if let Ok(stream) = tcp_stream.into_std() {
            spawned.push(spawn_abortable(async move {
                // Todo: There is 2 spawns inside setup_inbound that might be the cause of not releasing the port when deactivating the coin
                lightning_net_tokio::setup_inbound(peer_mgr.clone(), stream).await;
            }));
        };
    }
}

pub async fn init_peer_manager(
    ctx: MmArc,
    platform: &Arc<Platform>,
    listening_port: u16,
    channel_manager: Arc<ChannelManager>,
    gossip_sync: Arc<NetworkGossip>,
    node_secret: SecretKey,
    logger: Arc<LogState>,
) -> EnableLightningResult<Arc<PeerManager>> {
    // The set (possibly empty) of socket addresses on which this node accepts incoming connections.
    // If the user wishes to preserve privacy, addresses should likely contain only Tor Onion addresses.
    let listening_addr = myipaddr(ctx).await.map_to_mm(EnableLightningError::InvalidAddress)?;
    // If the listening port is used start_lightning should return an error early
    // Todo: when deactivating coin the address should be unbinded, listener seems to not be dropped for some reason, should also add a unit test for this after fixing it
    let listener = TcpListener::bind(format!("{}:{}", listening_addr, listening_port))
        .await
        .map_to_mm(|e| EnableLightningError::IOError(e.to_string()))?;

    // ephemeral_random_data is used to derive per-connection ephemeral keys
    let mut ephemeral_bytes = [0; 32];
    rand::thread_rng().fill_bytes(&mut ephemeral_bytes);
    let lightning_msg_handler = MessageHandler {
        chan_handler: channel_manager,
        route_handler: gossip_sync,
    };

    // IgnoringMessageHandler is used as custom message types (experimental and application-specific messages) is not needed
    let peer_manager: Arc<PeerManager> = Arc::new(PeerManager::new(
        lightning_msg_handler,
        node_secret,
        &ephemeral_bytes,
        logger,
        Arc::new(IgnoringMessageHandler {}),
    ));

    // Initialize p2p networking
    platform.spawner().spawn(ln_p2p_loop(peer_manager.clone(), listener));
    Ok(peer_manager)
}
