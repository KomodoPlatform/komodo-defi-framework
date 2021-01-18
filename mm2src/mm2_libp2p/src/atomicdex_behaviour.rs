use crate::{adex_ping::AdexPing,
            peers_exchange::{PeerAddresses, PeersExchange},
            request_response::{build_request_response_behaviour, PeerRequest, PeerResponse, RequestResponseBehaviour,
                               RequestResponseBehaviourEvent, RequestResponseSender}};
use atomicdex_gossipsub::{Gossipsub, GossipsubConfigBuilder, GossipsubEvent, GossipsubMessage, MessageId, Topic,
                          TopicHash};
use futures::{channel::{mpsc::{channel, Receiver, Sender},
                        oneshot},
              future::{join_all, poll_fn},
              Future, SinkExt, StreamExt};
use lazy_static::lazy_static;
use libp2p::swarm::{IntoProtocolsHandler, NetworkBehaviour, ProtocolsHandler};
use libp2p::{core::{ConnectedPoint, Multiaddr, Transport},
             identity,
             multiaddr::Protocol,
             noise,
             request_response::ResponseChannel,
             swarm::{ExpandedSwarm, NetworkBehaviourEventProcess, Swarm},
             NetworkBehaviour, PeerId};
use libp2p_floodsub::{Floodsub, FloodsubEvent, Topic as FloodsubTopic};
use log::{debug, error, info};
use rand::{seq::SliceRandom, thread_rng};
use std::collections::HashSet;
use std::{collections::hash_map::{DefaultHasher, HashMap},
          hash::{Hash, Hasher},
          iter::{self, FromIterator},
          net::IpAddr,
          pin::Pin,
          str::FromStr,
          task::{Context, Poll},
          time::Duration};
use tokio::runtime::Runtime;
use void::Void;
use wasm_timer::{Instant, Interval};

pub type AdexCmdTx = Sender<AdexBehaviourCmd>;
pub type AdexEventRx = Receiver<AdexBehaviourEvent>;

#[cfg(test)] mod tests;

struct SwarmRuntime(Runtime);

pub const PEERS_TOPIC: &str = "PEERS";
const CONNECTED_RELAYS_CHECK_INTERVAL: Duration = Duration::from_secs(30);
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(600);
const ANNOUNCE_INITIAL_DELAY: Duration = Duration::from_secs(60);
const CHANNEL_BUF_SIZE: usize = 1024 * 8;
const NETID_7777: u16 = 7777;

impl libp2p::core::Executor for &SwarmRuntime {
    fn exec(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) { self.0.spawn(future); }
}

lazy_static! {
    static ref SWARM_RUNTIME: SwarmRuntime = SwarmRuntime(Runtime::new().unwrap());
}

/// Returns info about connected peers
pub async fn get_peers_info(mut cmd_tx: AdexCmdTx) -> HashMap<String, Vec<String>> {
    let (result_tx, rx) = oneshot::channel();
    let cmd = AdexBehaviourCmd::GetPeersInfo { result_tx };
    cmd_tx.send(cmd).await.expect("Rx should be present");
    rx.await.expect("Tx should be present")
}

/// Returns current gossipsub mesh state
pub async fn get_gossip_mesh(mut cmd_tx: AdexCmdTx) -> HashMap<String, Vec<String>> {
    let (result_tx, rx) = oneshot::channel();
    let cmd = AdexBehaviourCmd::GetGossipMesh { result_tx };
    cmd_tx.send(cmd).await.expect("Rx should be present");
    rx.await.expect("Tx should be present")
}

pub async fn get_gossip_peer_topics(mut cmd_tx: AdexCmdTx) -> HashMap<String, Vec<String>> {
    let (result_tx, rx) = oneshot::channel();
    let cmd = AdexBehaviourCmd::GetGossipPeerTopics { result_tx };
    cmd_tx.send(cmd).await.expect("Rx should be present");
    rx.await.expect("Tx should be present")
}

pub async fn get_gossip_topic_peers(mut cmd_tx: AdexCmdTx) -> HashMap<String, Vec<String>> {
    let (result_tx, rx) = oneshot::channel();
    let cmd = AdexBehaviourCmd::GetGossipTopicPeers { result_tx };
    cmd_tx.send(cmd).await.expect("Rx should be present");
    rx.await.expect("Tx should be present")
}

pub async fn get_relay_mesh(mut cmd_tx: AdexCmdTx) -> Vec<String> {
    let (result_tx, rx) = oneshot::channel();
    let cmd = AdexBehaviourCmd::GetRelayMesh { result_tx };
    cmd_tx.send(cmd).await.expect("Rx should be present");
    rx.await.expect("Tx should be present")
}

#[derive(Debug)]
pub struct AdexResponseChannel(ResponseChannel<PeerResponse>);

impl From<ResponseChannel<PeerResponse>> for AdexResponseChannel {
    fn from(res: ResponseChannel<PeerResponse>) -> Self { AdexResponseChannel(res) }
}

impl From<AdexResponseChannel> for ResponseChannel<PeerResponse> {
    fn from(res: AdexResponseChannel) -> Self { res.0 }
}

#[derive(Debug)]
pub enum AdexBehaviourCmd {
    Subscribe {
        /// Subscribe to this topic
        topic: String,
    },
    PublishMsg {
        topics: Vec<String>,
        msg: Vec<u8>,
    },
    /// Request relays sequential until a response is received.
    RequestAnyRelay {
        req: Vec<u8>,
        response_tx: oneshot::Sender<Option<(PeerId, Vec<u8>)>>,
    },
    /// Request given peers and collect all their responses.
    RequestPeers {
        req: Vec<u8>,
        peers: Vec<String>,
        response_tx: oneshot::Sender<Vec<(PeerId, AdexResponse)>>,
    },
    /// Request relays and collect all their responses.
    RequestRelays {
        req: Vec<u8>,
        response_tx: oneshot::Sender<Vec<(PeerId, AdexResponse)>>,
    },
    /// Send a response using a `response_channel`.
    SendResponse {
        /// Response to a request.
        res: AdexResponse,
        /// Pass the same `response_channel` as that was obtained from [`AdexBehaviourEvent::PeerRequest`].
        response_channel: AdexResponseChannel,
    },
    GetPeersInfo {
        result_tx: oneshot::Sender<HashMap<String, Vec<String>>>,
    },
    GetGossipMesh {
        result_tx: oneshot::Sender<HashMap<String, Vec<String>>>,
    },
    GetGossipPeerTopics {
        result_tx: oneshot::Sender<HashMap<String, Vec<String>>>,
    },
    GetGossipTopicPeers {
        result_tx: oneshot::Sender<HashMap<String, Vec<String>>>,
    },
    GetRelayMesh {
        result_tx: oneshot::Sender<Vec<String>>,
    },
    PropagateMessage {
        message_id: MessageId,
        propagation_source: PeerId,
    },
}

/// The structure is the same as `PeerResponse`,
/// but is used to prevent `PeerResponse` from being used outside the network implementation.
#[derive(Debug, Eq, PartialEq)]
pub enum AdexResponse {
    Ok { response: Vec<u8> },
    None,
    Err { error: String },
}

impl From<PeerResponse> for AdexResponse {
    fn from(res: PeerResponse) -> Self {
        match res {
            PeerResponse::Ok { res } => AdexResponse::Ok { response: res },
            PeerResponse::None => AdexResponse::None,
            PeerResponse::Err { err } => AdexResponse::Err { error: err },
        }
    }
}

impl From<AdexResponse> for PeerResponse {
    fn from(res: AdexResponse) -> Self {
        match res {
            AdexResponse::Ok { response } => PeerResponse::Ok { res: response },
            AdexResponse::None => PeerResponse::None,
            AdexResponse::Err { error } => PeerResponse::Err { err: error },
        }
    }
}

/// The structure consists of GossipsubEvent and RequestResponse events.
/// It is used to prevent the network events from being used outside the network implementation.
#[derive(Debug)]
pub enum AdexBehaviourEvent {
    /// A message has been received.
    /// Derived from GossipsubEvent.
    Message(PeerId, MessageId, GossipsubMessage),
    /// A remote subscribed to a topic.
    Subscribed {
        /// Remote that has subscribed.
        peer_id: PeerId,
        /// The topic it has subscribed to.
        topic: TopicHash,
    },
    /// A remote unsubscribed from a topic.
    Unsubscribed {
        /// Remote that has unsubscribed.
        peer_id: PeerId,
        /// The topic it has subscribed from.
        topic: TopicHash,
    },
    /// A remote peer sent a request and waits for a response.
    PeerRequest {
        /// Remote that sent this request.
        peer_id: PeerId,
        /// The serialized data.
        request: Vec<u8>,
        /// A channel for sending a response to this request.
        /// The channel is used to identify the peer on the network that is waiting for an answer to this request.
        /// See [`AdexBehaviourCmd::SendResponse`].
        response_channel: AdexResponseChannel,
    },
}

impl From<GossipsubEvent> for AdexBehaviourEvent {
    fn from(event: GossipsubEvent) -> Self {
        match event {
            GossipsubEvent::Message(peer_id, message_id, gossipsub_message) => {
                AdexBehaviourEvent::Message(peer_id, message_id, gossipsub_message)
            },
            GossipsubEvent::Subscribed { peer_id, topic } => AdexBehaviourEvent::Subscribed { peer_id, topic },
            GossipsubEvent::Unsubscribed { peer_id, topic } => AdexBehaviourEvent::Unsubscribed { peer_id, topic },
        }
    }
}

/// AtomicDEX libp2p Network behaviour implementation
#[derive(NetworkBehaviour)]
pub struct AtomicDexBehaviour {
    #[behaviour(ignore)]
    event_tx: Sender<AdexBehaviourEvent>,
    #[behaviour(ignore)]
    spawn_fn: fn(Box<dyn Future<Output = ()> + Send + Unpin + 'static>) -> (),
    #[behaviour(ignore)]
    cmd_rx: Receiver<AdexBehaviourCmd>,
    #[behaviour(ignore)]
    netid: u16,
    floodsub: Floodsub,
    gossipsub: Gossipsub,
    request_response: RequestResponseBehaviour,
    peers_exchange: PeersExchange,
    ping: AdexPing,
}

impl AtomicDexBehaviour {
    fn notify_on_adex_event(&mut self, event: AdexBehaviourEvent) {
        if let Err(e) = self.event_tx.try_send(event) {
            error!("notify_on_adex_event error {}", e);
        }
    }

    fn spawn(&self, fut: impl Future<Output = ()> + Send + 'static) { (self.spawn_fn)(Box::new(Box::pin(fut))) }

    fn process_cmd(&mut self, cmd: AdexBehaviourCmd) {
        match cmd {
            AdexBehaviourCmd::Subscribe { topic } => {
                let topic = Topic::new(topic);
                self.gossipsub.subscribe(topic);
            },
            AdexBehaviourCmd::PublishMsg { topics, msg } => {
                self.gossipsub.publish_many(topics.into_iter().map(Topic::new), msg);
            },
            AdexBehaviourCmd::RequestAnyRelay { req, response_tx } => {
                let relays = self.gossipsub.get_relay_mesh();
                // spawn the `request_any_peer` future
                let future = request_any_peer(relays, req, self.request_response.sender(), response_tx);
                self.spawn(future);
            },
            AdexBehaviourCmd::RequestPeers {
                req,
                peers,
                response_tx,
            } => {
                let peers = peers
                    .into_iter()
                    .filter_map(|peer| match peer.parse() {
                        Ok(p) => Some(p),
                        Err(e) => {
                            error!("Error on parse peer id {:?}: {:?}", peer, e);
                            None
                        },
                    })
                    .collect();
                let future = request_peers(peers, req, self.request_response.sender(), response_tx);
                self.spawn(future);
            },
            AdexBehaviourCmd::RequestRelays { req, response_tx } => {
                let relays = self.gossipsub.get_relay_mesh();
                // spawn the `request_peers` future
                let future = request_peers(relays, req, self.request_response.sender(), response_tx);
                self.spawn(future);
            },
            AdexBehaviourCmd::SendResponse { res, response_channel } => {
                self.request_response.send_response(response_channel.into(), res.into());
            },
            AdexBehaviourCmd::GetPeersInfo { result_tx } => {
                let result = self
                    .gossipsub
                    .get_peers_connections()
                    .into_iter()
                    .map(|(peer_id, connected_points)| {
                        let peer_id = peer_id.to_base58();
                        let connected_points = connected_points
                            .into_iter()
                            .map(|point| match point {
                                ConnectedPoint::Dialer { address } => address.to_string(),
                                ConnectedPoint::Listener { send_back_addr, .. } => send_back_addr.to_string(),
                            })
                            .collect();
                        (peer_id, connected_points)
                    })
                    .collect();
                if result_tx.send(result).is_err() {
                    debug!("Result rx is dropped");
                }
            },
            AdexBehaviourCmd::GetGossipMesh { result_tx } => {
                let result = self
                    .gossipsub
                    .get_mesh()
                    .iter()
                    .map(|(topic, peers)| {
                        let topic = topic.to_string();
                        let peers = peers.iter().map(|peer| peer.to_string()).collect();
                        (topic, peers)
                    })
                    .collect();
                if result_tx.send(result).is_err() {
                    debug!("Result rx is dropped");
                }
            },
            AdexBehaviourCmd::GetGossipPeerTopics { result_tx } => {
                let result = self
                    .gossipsub
                    .get_all_peer_topics()
                    .iter()
                    .map(|(peer, topics)| {
                        let peer = peer.to_string();
                        let topics = topics.iter().map(|topic| topic.to_string()).collect();
                        (peer, topics)
                    })
                    .collect();
                if result_tx.send(result).is_err() {
                    error!("Result rx is dropped");
                }
            },
            AdexBehaviourCmd::GetGossipTopicPeers { result_tx } => {
                let result = self
                    .gossipsub
                    .get_all_topic_peers()
                    .iter()
                    .map(|(topic, peers)| {
                        let topic = topic.to_string();
                        let peers = peers.iter().map(|peer| peer.to_string()).collect();
                        (topic, peers)
                    })
                    .collect();
                if result_tx.send(result).is_err() {
                    error!("Result rx is dropped");
                }
            },
            AdexBehaviourCmd::GetRelayMesh { result_tx } => {
                let result = self
                    .gossipsub
                    .get_relay_mesh()
                    .into_iter()
                    .map(|peer| peer.to_string())
                    .collect();
                if result_tx.send(result).is_err() {
                    error!("Result rx is dropped");
                }
            },
            AdexBehaviourCmd::PropagateMessage {
                message_id,
                propagation_source,
            } => {
                self.gossipsub.propagate_message(&message_id, &propagation_source);
            },
        }
    }

    fn announce_listeners(&mut self, listeners: PeerAddresses) {
        let serialized = rmp_serde::to_vec(&listeners).expect("PeerAddresses serialization should never fail");
        self.floodsub.publish(FloodsubTopic::new(PEERS_TOPIC), serialized);
    }

    pub fn connected_relays_len(&self) -> usize { self.gossipsub.connected_relays_len() }

    pub fn relay_mesh_len(&self) -> usize { self.gossipsub.relay_mesh_len() }

    pub fn received_messages_in_period(&self) -> (Duration, usize) { self.gossipsub.get_received_messages_in_period() }

    pub fn connected_peers_len(&self) -> usize { self.gossipsub.get_num_peers() }
}

impl NetworkBehaviourEventProcess<GossipsubEvent> for AtomicDexBehaviour {
    fn inject_event(&mut self, event: GossipsubEvent) { self.notify_on_adex_event(event.into()); }
}

impl NetworkBehaviourEventProcess<FloodsubEvent> for AtomicDexBehaviour {
    fn inject_event(&mut self, event: FloodsubEvent) {
        // do not process peer announce on 7777 temporary
        if self.netid != NETID_7777 {
            if let FloodsubEvent::Message(message) = &event {
                for topic in &message.topics {
                    if topic == &FloodsubTopic::new(PEERS_TOPIC) {
                        let addresses: PeerAddresses = match rmp_serde::from_read_ref(&message.data) {
                            Ok(a) => a,
                            Err(_) => return,
                        };
                        self.peers_exchange.add_peer_addresses(&message.source, addresses);
                    }
                }
            }
        }
    }
}

impl NetworkBehaviourEventProcess<Void> for AtomicDexBehaviour {
    fn inject_event(&mut self, _event: Void) {}
}

impl NetworkBehaviourEventProcess<()> for AtomicDexBehaviour {
    fn inject_event(&mut self, _event: ()) {}
}

impl NetworkBehaviourEventProcess<RequestResponseBehaviourEvent> for AtomicDexBehaviour {
    fn inject_event(&mut self, event: RequestResponseBehaviourEvent) {
        match event {
            RequestResponseBehaviourEvent::InboundRequest {
                peer_id,
                request,
                response_channel,
            } => {
                let event = AdexBehaviourEvent::PeerRequest {
                    peer_id,
                    request: request.req,
                    response_channel: response_channel.into(),
                };
                // forward the event to the AdexBehaviourCmd handler
                self.notify_on_adex_event(event);
            },
        }
    }
}

/// Custom types mapping the complex associated types of AtomicDexBehaviour to the ExpandedSwarm
type AdexSwarmHandler = <<AtomicDexBehaviour as NetworkBehaviour>::ProtocolsHandler as IntoProtocolsHandler>::Handler;
type AtomicDexSwarm = ExpandedSwarm<
    AtomicDexBehaviour,
    <AdexSwarmHandler as ProtocolsHandler>::InEvent,
    <AdexSwarmHandler as ProtocolsHandler>::OutEvent,
    <AtomicDexBehaviour as NetworkBehaviour>::ProtocolsHandler,
>;

fn maintain_connection_to_relays(swarm: &mut AtomicDexSwarm, bootstrap_addresses: &[Multiaddr]) {
    let connected_relays = swarm.gossipsub.connected_relays();
    let mesh_n_low = swarm.gossipsub.get_config().mesh_n_low;
    let mesh_n = swarm.gossipsub.get_config().mesh_n;
    // allow 2 * mesh_n_high connections to other nodes
    let max_n = swarm.gossipsub.get_config().mesh_n_high * 2;
    if connected_relays.len() < mesh_n_low {
        let to_connect_num = mesh_n - connected_relays.len();
        let to_connect = swarm
            .peers_exchange
            .get_random_peers(to_connect_num, |peer| !connected_relays.contains(peer));

        // choose some random bootstrap addresses to connect if peers exchange returned not enough peers
        if to_connect.len() < to_connect_num {
            let connect_bootstrap_num = to_connect_num - to_connect.len();
            for addr in bootstrap_addresses
                .iter()
                .filter(|addr| !swarm.gossipsub.is_connected_to_addr(addr))
                .collect::<Vec<_>>()
                .choose_multiple(&mut thread_rng(), connect_bootstrap_num)
            {
                if let Err(e) = libp2p::Swarm::dial_addr(swarm, (*addr).clone()) {
                    error!("Bootstrap addr {} dial error {}", addr, e);
                }
            }
        }
        for (peer, addresses) in to_connect {
            for addr in addresses {
                if swarm.gossipsub.is_connected_to_addr(&addr) {
                    continue;
                }
                if let Err(e) = libp2p::Swarm::dial_addr(swarm, addr.clone()) {
                    error!("Peer {} address {} dial error {}", peer, addr, e);
                }
            }
        }
    }

    if connected_relays.len() > max_n {
        let mut rng = thread_rng();
        let to_disconnect_num = connected_relays.len() - max_n;
        let relays_mesh = swarm.gossipsub.get_relay_mesh();
        let not_in_mesh: Vec<_> = connected_relays
            .iter()
            .filter(|peer| !relays_mesh.contains(peer))
            .collect();
        for peer in not_in_mesh.choose_multiple(&mut rng, to_disconnect_num) {
            info!("Disconnecting peer {}", peer);
            if Swarm::disconnect_peer_id(swarm, (*peer).clone()).is_err() {
                error!("Peer {} disconnect error", peer);
            }
        }
    }

    for relay in connected_relays {
        if !swarm.peers_exchange.is_known_peer(&relay) {
            swarm.peers_exchange.add_known_peer(relay);
        }
    }
}

fn announce_my_addresses(swarm: &mut AtomicDexSwarm) {
    let global_listeners: PeerAddresses = Swarm::listeners(&swarm)
        .filter(|listener| {
            for protocol in listener.iter() {
                if let Protocol::Ip4(ip) = protocol {
                    return ip.is_global();
                }
            }
            false
        })
        .take(1)
        .cloned()
        .collect();
    if !global_listeners.is_empty() {
        swarm.announce_listeners(global_listeners);
    }
}

const ALL_NETID_7777_SEEDNODES: &[(&str, &str)] = &[
    (
        "12D3KooWEsuiKcQaBaKEzuMtT6uFjs89P1E8MK3wGRZbeuCbCw6P",
        "168.119.236.241",
    ),
    (
        "12D3KooWKxavLCJVrQ5Gk1kd9m6cohctGQBmiKPS9XQFoXEoyGmS",
        "168.119.236.249",
    ),
    (
        "12D3KooWAToxtunEBWCoAHjefSv74Nsmxranw8juy3eKEdrQyGRF",
        "168.119.236.240",
    ),
    (
        "12D3KooWSmEi8ypaVzFA1AGde2RjxNW5Pvxw3qa2fVe48PjNs63R",
        "168.119.236.239",
    ),
    (
        "12D3KooWHKkHiNhZtKceQehHhPqwU5W1jXpoVBgS1qst899GjvTm",
        "168.119.236.251",
    ),
    ("12D3KooWMrjLmrv8hNgAoVf1RfumfjyPStzd4nv5XL47zN4ZKisb", "168.119.237.8"),
    (
        "12D3KooWL6yrrNACb7t7RPyTEPxKmq8jtrcbkcNd6H5G2hK7bXaL",
        "168.119.236.233",
    ),
    (
        "12D3KooWHBeCnJdzNk51G4mLnao9cDsjuqiMTEo5wMFXrd25bd1F",
        "168.119.236.243",
    ),
    (
        "12D3KooW9soGyPfX6kcyh3uVXNHq1y2dPmQNt2veKgdLXkBiCVKq",
        "168.119.236.246",
    ),
    ("12D3KooWPR2RoPi19vQtLugjCdvVmCcGLP2iXAzbDfP3tp81ZL4d", "168.119.237.13"),
    ("12D3KooWKu8pMTgteWacwFjN7zRWWHb3bctyTvHU3xx5x4x6qDYY", "195.201.91.96"),
    ("12D3KooWJWBnkVsVNjiqUEPjLyHpiSmQVAJ5t6qt1Txv5ctJi9Xd", "195.201.91.53"),
    (
        "12D3KooWGrUpCAbkxhPRioNs64sbUmPmpEcou6hYfrqQvxfWDEuf",
        "168.119.174.126",
    ),
    ("12D3KooWEaZpH61H4yuQkaNG5AsyGdpBhKRppaLdAY52a774ab5u", "46.4.78.11"),
    ("12D3KooWAd5gPXwX7eDvKWwkr2FZGfoJceKDCA53SHmTFFVkrN7Q", "46.4.87.18"),
];

/// Creates and spawns new AdexBehaviour Swarm returning:
/// 1. tx to send control commands
/// 2. rx emitting gossip events to processing side
/// 3. our peer_id
#[allow(clippy::too_many_arguments)]
pub fn start_gossipsub(
    ip: IpAddr,
    port: u16,
    netid: u16,
    force_key: Option<[u8; 32]>,
    spawn_fn: fn(Box<dyn Future<Output = ()> + Send + Unpin + 'static>) -> (),
    to_dial: Vec<String>,
    i_am_relay: bool,
    on_poll: impl Fn(&AtomicDexSwarm) + Send + 'static,
) -> (Sender<AdexBehaviourCmd>, AdexEventRx, PeerId) {
    let local_key = match force_key {
        Some(mut key) => {
            let secret = identity::ed25519::SecretKey::from_bytes(&mut key).expect("Secret length is 32 bytes");
            let keypair = identity::ed25519::Keypair::from(secret);
            identity::Keypair::Ed25519(keypair)
        },
        None => identity::Keypair::generate_ed25519(),
    };
    let local_peer_id = PeerId::from(local_key.public());
    info!("Local peer id: {:?}", local_peer_id);

    // Set up an encrypted TCP Transport over the Mplex protocol
    let transport = {
        let tcp = libp2p::tcp::TokioTcpConfig::new().nodelay(true);
        let transport = libp2p::dns::DnsConfig::new(tcp).unwrap();
        let trans_clone = transport.clone();
        transport.or_transport(libp2p::websocket::WsConfig::new(trans_clone))
    };

    let noise_keys = noise::Keypair::<noise::X25519Spec>::new()
        .into_authentic(&local_key)
        .expect("Signing libp2p-noise static DH keypair failed.");

    let transport = transport
        .upgrade(libp2p::core::upgrade::Version::V1)
        .authenticate(noise::NoiseConfig::xx(noise_keys).into_authenticated())
        .multiplex(libp2p::mplex::MplexConfig::default())
        .map(|(peer, muxer), _| (peer, libp2p::core::muxing::StreamMuxerBox::new(muxer)))
        .timeout(std::time::Duration::from_secs(20));

    let (cmd_tx, cmd_rx) = channel(CHANNEL_BUF_SIZE);
    let (event_tx, event_rx) = channel(CHANNEL_BUF_SIZE);

    let bootstrap: Vec<Multiaddr> = to_dial
        .into_iter()
        .map(|addr| parse_relay_address(addr, port))
        .collect();

    let (mesh_n_low, mesh_n, mesh_n_high) = if i_am_relay { (3, 6, 8) } else { (2, 3, 4) };

    // Create a Swarm to manage peers and events
    let mut swarm = {
        // to set default parameters for gossipsub use:
        // let gossipsub_config = gossipsub::GossipsubConfig::default();

        // To content-address message, we can take the hash of message and use it as an ID.
        let message_id_fn = |message: &GossipsubMessage| {
            let mut s = DefaultHasher::new();
            message.data.hash(&mut s);
            message.sequence_number.hash(&mut s);
            MessageId(s.finish().to_string())
        };

        // set custom gossipsub
        let gossipsub_config = GossipsubConfigBuilder::new()
            .message_id_fn(message_id_fn)
            .i_am_relay(i_am_relay)
            .mesh_n_low(mesh_n_low)
            .mesh_n(mesh_n)
            .mesh_n_high(mesh_n_high)
            .manual_propagation()
            .max_transmit_size(1024 * 1024 - 100)
            .build();
        // build a gossipsub network behaviour
        let mut gossipsub = Gossipsub::new(local_peer_id.clone(), gossipsub_config);

        let floodsub = Floodsub::new(local_peer_id.clone(), netid != NETID_7777);

        let mut peers_exchange = PeersExchange::new(port);
        if netid == NETID_7777 {
            for (peer_id, address) in ALL_NETID_7777_SEEDNODES {
                let peer_id = PeerId::from_str(peer_id).expect("valid peer id");
                let multiaddr = parse_relay_address((*address).to_owned(), port);
                peers_exchange.add_peer_addresses(&peer_id, HashSet::from_iter(iter::once(multiaddr)));
                gossipsub.add_explicit_relay(peer_id);
            }
        }

        // build a request-response network behaviour
        let request_response = build_request_response_behaviour();

        // use default ping config with 15s interval, 20s timeout and 1 max failure
        let ping = AdexPing::new();

        let adex_behavior = AtomicDexBehaviour {
            event_tx,
            spawn_fn,
            cmd_rx,
            gossipsub,
            floodsub,
            request_response,
            peers_exchange,
            ping,
            netid,
        };
        libp2p::swarm::SwarmBuilder::new(transport, adex_behavior, local_peer_id.clone())
            .executor(Box::new(&*SWARM_RUNTIME))
            .build()
    };
    swarm.floodsub.subscribe(FloodsubTopic::new(PEERS_TOPIC.to_owned()));
    let addr = format!("/ip4/{}/tcp/{}", ip, port);
    if i_am_relay {
        libp2p::Swarm::listen_on(&mut swarm, addr.parse().unwrap()).unwrap();
    }
    for relay in bootstrap.choose_multiple(&mut thread_rng(), mesh_n) {
        match libp2p::Swarm::dial_addr(&mut swarm, relay.clone()) {
            Ok(_) => info!("Dialed {}", relay),
            Err(e) => error!("Dial {:?} failed: {:?}", relay, e),
        }
    }

    let mut check_connected_relays_interval = Interval::new_at(
        Instant::now() + CONNECTED_RELAYS_CHECK_INTERVAL,
        CONNECTED_RELAYS_CHECK_INTERVAL,
    );
    let mut announce_interval = Interval::new_at(Instant::now() + ANNOUNCE_INITIAL_DELAY, ANNOUNCE_INTERVAL);
    let polling_fut = poll_fn(move |cx: &mut Context| {
        loop {
            match swarm.cmd_rx.poll_next_unpin(cx) {
                Poll::Ready(Some(cmd)) => swarm.process_cmd(cmd),
                Poll::Ready(None) => return Poll::Ready(()),
                Poll::Pending => break,
            }
        }

        loop {
            match swarm.poll_next_unpin(cx) {
                Poll::Ready(Some(event)) => debug!("Swarm event {:?}", event),
                Poll::Ready(None) => return Poll::Ready(()),
                Poll::Pending => break,
            }
        }

        if swarm.gossipsub.is_relay() {
            while let Poll::Ready(Some(())) = announce_interval.poll_next_unpin(cx) {
                announce_my_addresses(&mut swarm);
            }
        }

        while let Poll::Ready(Some(())) = check_connected_relays_interval.poll_next_unpin(cx) {
            maintain_connection_to_relays(&mut swarm, &bootstrap);
        }
        on_poll(&swarm);
        Poll::Pending
    });

    SWARM_RUNTIME.0.spawn(polling_fut);

    (cmd_tx, event_rx, local_peer_id)
}

/// If te `addr` is in the "/ip4/{addr}/tcp/{port}" format then parse the `addr` immediately to the `Multiaddr`,
/// else construct the "/ip4/{addr}/tcp/{port}" from `addr` and `port` values.
#[cfg(test)]
fn parse_relay_address(addr: String, port: u16) -> Multiaddr {
    if addr.contains("/ip4/") && addr.contains("/tcp/") {
        addr.parse().unwrap()
    } else {
        format!("/ip4/{}/tcp/{}", addr, port).parse().unwrap()
    }
}

/// The addr is expected to be an IP of the relay
#[cfg(not(test))]
fn parse_relay_address(addr: String, port: u16) -> Multiaddr { format!("/ip4/{}/tcp/{}", addr, port).parse().unwrap() }

/// Request the peers sequential until a `PeerResponse::Ok()` will not be received.
async fn request_any_peer(
    peers: Vec<PeerId>,
    request_data: Vec<u8>,
    request_response_tx: RequestResponseSender,
    response_tx: oneshot::Sender<Option<(PeerId, Vec<u8>)>>,
) {
    debug!("start request_any_peer loop: peers {}", peers.len());
    for peer in peers {
        match request_one_peer(peer.clone(), request_data.clone(), request_response_tx.clone()).await {
            PeerResponse::Ok { res } => {
                debug!("Received a response from peer {:?}, stop the request loop", peer);
                if response_tx.send(Some((peer, res))).is_err() {
                    error!("Response oneshot channel was closed");
                }
                return;
            },
            PeerResponse::None => {
                debug!("Received None from peer {:?}, request next peer", peer);
            },
            PeerResponse::Err { err } => {
                error!("Error on request {:?} peer: {:?}. Request next peer", peer, err);
            },
        };
    }

    debug!("None of the peers responded to the request");
    if response_tx.send(None).is_err() {
        error!("Response oneshot channel was closed");
    };
}

/// Request the peers and collect all their responses.
async fn request_peers(
    peers: Vec<PeerId>,
    request_data: Vec<u8>,
    request_response_tx: RequestResponseSender,
    response_tx: oneshot::Sender<Vec<(PeerId, AdexResponse)>>,
) {
    debug!("start request_any_peer loop: peers {}", peers.len());
    let mut futures = Vec::with_capacity(peers.len());
    for peer in peers {
        let request_data = request_data.clone();
        let request_response_tx = request_response_tx.clone();
        futures.push(async move {
            let response = request_one_peer(peer.clone(), request_data, request_response_tx).await;
            (peer.clone(), response)
        })
    }

    let responses = join_all(futures)
        .await
        .into_iter()
        .map(|(peer_id, res)| {
            let res: AdexResponse = res.into();
            (peer_id, res)
        })
        .collect();

    if response_tx.send(responses).is_err() {
        error!("Response oneshot channel was closed");
    };
}

async fn request_one_peer(peer: PeerId, req: Vec<u8>, mut request_response_tx: RequestResponseSender) -> PeerResponse {
    // Use the internal receiver to receive a response to this request.
    let (internal_response_tx, internal_response_rx) = oneshot::channel();
    let request = PeerRequest { req };
    request_response_tx
        .send((peer.clone(), request, internal_response_tx))
        .await
        .unwrap();

    match internal_response_rx.await {
        Ok(response) => response,
        Err(e) => PeerResponse::Err {
            err: format!("Error on request the peer {:?}: \"{:?}\". Request next peer", peer, e),
        },
    }
}
