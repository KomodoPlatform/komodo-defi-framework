use futures::StreamExt;
use futures_ticker::Ticker;
use libp2p::{multiaddr::Protocol,
             request_response::{Behaviour as RequestResponse, Config as RequestResponseConfig,
                                Event as RequestResponseEvent, ProtocolSupport, RequestId, ResponseChannel},
             swarm::{NetworkBehaviour, PollParameters, ToSwarm},
             Multiaddr, PeerId};
use log::{info, warn};
use rand::seq::SliceRandom;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::{HashMap, HashSet, VecDeque},
          iter,
          task::{Context, Poll},
          time::Duration};

use crate::{request_response::Codec, NetworkInfo};

pub type PeerAddresses = HashSet<Multiaddr>;
type PeersExchangeCodec = Codec<PeersExchangeProtocol, PeersExchangeRequest, PeersExchangeResponse>;

const DEFAULT_PEERS_NUM: usize = 20;
const REQUEST_PEERS_INITIAL_DELAY: u64 = 20;
const REQUEST_PEERS_INTERVAL: u64 = 300;
const MAX_PEERS: usize = 100;

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct PeerIdSerde(PeerId);

impl From<PeerId> for PeerIdSerde {
    fn from(peer_id: PeerId) -> PeerIdSerde { PeerIdSerde(peer_id) }
}

impl Serialize for PeerIdSerde {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.clone().to_bytes().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PeerIdSerde {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
        let peer_id = PeerId::from_bytes(&bytes).map_err(|_| serde::de::Error::custom("PeerId::from_bytes error"))?;
        Ok(PeerIdSerde(peer_id))
    }
}

#[derive(Debug, Clone)]
pub enum PeersExchangeProtocol {
    Version1,
}

impl AsRef<str> for PeersExchangeProtocol {
    fn as_ref(&self) -> &str {
        match self {
            PeersExchangeProtocol::Version1 => "/peers-exchange/1",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum PeersExchangeRequest {
    GetKnownPeers { num: usize },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum PeersExchangeResponse {
    KnownPeers { peers: HashMap<PeerIdSerde, PeerAddresses> },
}

pub struct PeersExchange {
    request_response: RequestResponse<PeersExchangeCodec>,
    known_peers: Vec<PeerId>,
    reserved_peers: Vec<PeerId>,
    events: VecDeque<ToSwarm<(), <Self as NetworkBehaviour>::ConnectionHandler>>,
    maintain_peers_interval: Ticker,
    network_info: NetworkInfo,
}

impl NetworkBehaviour for PeersExchange {
    type ConnectionHandler = <RequestResponse<PeersExchangeCodec> as NetworkBehaviour>::ConnectionHandler;

    type ToSwarm = ();

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: libp2p::swarm::ConnectionId,
        peer: PeerId,
        local_addr: &libp2p::Multiaddr,
        remote_addr: &libp2p::Multiaddr,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        todo!()
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: libp2p::swarm::ConnectionId,
        peer: PeerId,
        addr: &libp2p::Multiaddr,
        role_override: libp2p::core::Endpoint,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        todo!()
    }

    fn on_swarm_event(&mut self, event: libp2p::swarm::FromSwarm<Self::ConnectionHandler>) { todo!() }

    fn on_connection_handler_event(
        &mut self,
        _peer_id: PeerId,
        _connection_id: libp2p::swarm::ConnectionId,
        _event: libp2p::swarm::THandlerOutEvent<Self>,
    ) {
        todo!()
    }

    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
        params: &mut impl libp2p::swarm::PollParameters,
    ) -> std::task::Poll<ToSwarm<Self::ToSwarm, libp2p::swarm::THandlerInEvent<Self>>> {
        todo!()
    }
}

#[allow(clippy::new_without_default)]
impl PeersExchange {
    pub fn new(network_info: NetworkInfo) -> Self {
        let protocol = iter::once((PeersExchangeProtocol::Version1, ProtocolSupport::Full));
        let config = RequestResponseConfig::default();
        let request_response = RequestResponse::new(protocol, config);
        PeersExchange {
            request_response,
            known_peers: Vec::new(),
            reserved_peers: Vec::new(),
            events: VecDeque::new(),
            maintain_peers_interval: Ticker::new_with_next(
                Duration::from_secs(REQUEST_PEERS_INTERVAL),
                Duration::from_secs(REQUEST_PEERS_INITIAL_DELAY),
            ),
            network_info,
        }
    }

    fn addresses_of_peer(&self, peer: &PeerId) -> Vec<Multiaddr> {
        let mut addresses = Vec::new();
        if let Some(connections) = self.request_response.connected.get(peer) {
            addresses.extend(connections.iter().filter_map(|c| c.address.clone()))
        }
        if let Some(more) = self.request_response.addresses.get(peer) {
            addresses.extend(more.into_iter().cloned());
        }
        addresses
    }

    fn get_random_known_peers(&mut self, num: usize) -> HashMap<PeerIdSerde, PeerAddresses> {
        let mut result = HashMap::with_capacity(num);
        let mut rng = rand::thread_rng();
        let peer_ids = self
            .known_peers
            .clone()
            .into_iter()
            .filter(|peer| !self.addresses_of_peer(peer).is_empty())
            .collect::<Vec<_>>();

        let peer_ids = peer_ids.choose_multiple(&mut rng, num);
        for peer_id in peer_ids {
            let addresses = self.addresses_of_peer(peer_id).into_iter().collect();
            result.insert((*peer_id).into(), addresses);
        }
        result
    }

    fn forget_peer(&mut self, peer: &PeerId) {
        self.known_peers.retain(|known_peer| known_peer != peer);
        self.forget_peer_addresses(peer);
    }

    fn forget_peer_addresses(&mut self, peer: &PeerId) {
        for address in self.addresses_of_peer(peer) {
            if !self.is_reserved_peer(peer) {
                self.request_response.remove_address(peer, &address);
            }
        }
    }

    pub fn add_peer_addresses_to_known_peers(&mut self, peer: &PeerId, addresses: PeerAddresses) {
        for address in addresses.iter() {
            if !self.validate_global_multiaddr(address) {
                warn!("Attempt adding a not valid address of the peer '{}': {}", peer, address);
                return;
            }
        }
        if !self.known_peers.contains(peer) && !addresses.is_empty() {
            self.known_peers.push(*peer);
        }
        let already_known = self.addresses_of_peer(peer);
        for address in addresses {
            if !already_known.contains(&address) {
                self.request_response.add_address(peer, address);
            }
        }
    }

    pub fn add_peer_addresses_to_reserved_peers(&mut self, peer: &PeerId, addresses: PeerAddresses) {
        for address in addresses.iter() {
            if !self.validate_global_multiaddr(address) {
                return;
            }
        }

        if !self.reserved_peers.contains(peer) && !addresses.is_empty() {
            self.reserved_peers.push(*peer);
        }

        let already_reserved = self.addresses_of_peer(peer);
        for address in addresses {
            if !already_reserved.contains(&address) {
                self.request_response.add_address(peer, address);
            }
        }
    }

    fn maintain_known_peers(&mut self) {
        if self.known_peers.len() > MAX_PEERS {
            let mut rng = rand::thread_rng();
            let to_remove_num = self.known_peers.len() - MAX_PEERS;
            self.known_peers.shuffle(&mut rng);
            let removed_peers: Vec<_> = self.known_peers.drain(..to_remove_num).collect();
            for peer in removed_peers {
                self.forget_peer_addresses(&peer);
            }
        }
        self.request_known_peers_from_random_peer();
    }

    fn request_known_peers_from_random_peer(&mut self) {
        let mut rng = rand::thread_rng();
        if let Some(from_peer) = self.known_peers.choose(&mut rng) {
            info!("Try to request {} peers from peer {}", DEFAULT_PEERS_NUM, from_peer);
            let request = PeersExchangeRequest::GetKnownPeers { num: DEFAULT_PEERS_NUM };
            self.request_response.send_request(from_peer, request);
        }
    }

    pub fn get_random_peers(
        &mut self,
        num: usize,
        mut filter: impl FnMut(&PeerId) -> bool,
    ) -> HashMap<PeerId, PeerAddresses> {
        let mut result = HashMap::with_capacity(num);
        let mut rng = rand::thread_rng();
        let peer_ids = self.known_peers.iter().filter(|peer| filter(peer)).collect::<Vec<_>>();

        for peer_id in peer_ids.choose_multiple(&mut rng, num) {
            let addresses = self.addresses_of_peer(peer_id).into_iter().collect();
            result.insert(**peer_id, addresses);
        }

        result
    }

    pub fn is_known_peer(&self, peer: &PeerId) -> bool { self.known_peers.contains(peer) }

    pub fn is_reserved_peer(&self, peer: &PeerId) -> bool { self.reserved_peers.contains(peer) }

    pub fn add_known_peer(&mut self, peer: PeerId) {
        if !self.is_known_peer(&peer) {
            self.known_peers.push(peer)
        }
    }

    fn validate_global_multiaddr(&self, address: &Multiaddr) -> bool {
        let network_ports = match self.network_info {
            NetworkInfo::Distributed { network_ports } => network_ports,
            NetworkInfo::InMemory => panic!("PeersExchange must not be used with in-memory network"),
        };

        let mut components = address.iter();
        match components.next() {
            Some(Protocol::Ip4(addr)) => {
                if !addr.is_global() {
                    return false;
                }
            },
            _ => return false,
        }

        match components.next() {
            Some(Protocol::Tcp(port)) => {
                // currently, `NetworkPorts::ws` is not supported by `PeersExchange`
                if port != network_ports.tcp {
                    return false;
                }
            },
            _ => return false,
        }

        true
    }

    fn validate_get_known_peers_response(&self, response: &HashMap<PeerIdSerde, PeerAddresses>) -> bool {
        if response.is_empty() {
            return false;
        }

        if response.len() > DEFAULT_PEERS_NUM {
            return false;
        }

        for addresses in response.values() {
            if addresses.is_empty() {
                return false;
            }

            for address in addresses {
                if !self.validate_global_multiaddr(address) {
                    warn!("Received a not valid address: {}", address);
                    return false;
                }
            }
        }
        true
    }

    fn poll(
        &mut self,
        cx: &mut Context,
        _params: &mut impl PollParameters,
    ) -> Poll<ToSwarm<(), <Self as NetworkBehaviour>::ConnectionHandler>> {
        while let Poll::Ready(Some(_)) = self.maintain_peers_interval.poll_next_unpin(cx) {
            self.maintain_known_peers();
        }

        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(event);
        }

        Poll::Pending
    }
}
