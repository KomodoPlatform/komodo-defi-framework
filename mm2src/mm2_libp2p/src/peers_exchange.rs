use crate::request_response::Codec;
use futures::StreamExt;
use libp2p::swarm::NetworkBehaviour;
use libp2p::{multiaddr::Multiaddr,
             request_response::{handler::RequestProtocol, ProtocolName, ProtocolSupport, RequestResponse,
                                RequestResponseConfig, RequestResponseEvent, RequestResponseMessage},
             swarm::{NetworkBehaviourAction, NetworkBehaviourEventProcess, PollParameters},
             NetworkBehaviour, PeerId};
use log::{error, info};
use rand::{seq::SliceRandom, thread_rng};
use serde::{de::Deserializer, ser::Serializer, Deserialize, Serialize};
use std::collections::HashSet;
use std::{collections::{HashMap, VecDeque},
          iter,
          task::{Context, Poll},
          time::Duration};
use wasm_timer::{Instant, Interval};

pub type PeerAddresses = HashSet<Multiaddr>;

#[derive(Debug, Clone)]
pub enum PeersExchangeProtocol {
    Version1,
}

impl ProtocolName for PeersExchangeProtocol {
    fn protocol_name(&self) -> &[u8] {
        match self {
            PeersExchangeProtocol::Version1 => b"/peers-exchange/1",
        }
    }
}

type PeersExchangeCodec = Codec<PeersExchangeProtocol, PeersExchangeRequest, PeersExchangeResponse>;

const REQUEST_PEERS_INITIAL_DELAY: u64 = 60;
const REQUEST_PEERS_INTERVAL: u64 = 300;
const MAX_PEERS: usize = 100;

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct PeerIdSerde(PeerId);

impl From<PeerId> for PeerIdSerde {
    fn from(peer_id: PeerId) -> PeerIdSerde { PeerIdSerde(peer_id) }
}

impl Serialize for PeerIdSerde {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.clone().into_bytes().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PeerIdSerde {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
        let peer_id = PeerId::from_bytes(bytes).map_err(|_| serde::de::Error::custom("PeerId::from_bytes error"))?;
        Ok(PeerIdSerde(peer_id))
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

/// Behaviour that requests known peers list from other peers at random
#[derive(NetworkBehaviour)]
#[behaviour(poll_method = "poll")]
pub struct PeersExchange {
    request_response: RequestResponse<PeersExchangeCodec>,
    #[behaviour(ignore)]
    known_peers: Vec<PeerId>,
    #[behaviour(ignore)]
    events: VecDeque<NetworkBehaviourAction<RequestProtocol<PeersExchangeCodec>, ()>>,
    #[behaviour(ignore)]
    maintain_peers_interval: Interval,
}

#[allow(clippy::new_without_default)]
impl PeersExchange {
    pub fn new() -> Self {
        let codec = Codec::default();
        let protocol = iter::once((PeersExchangeProtocol::Version1, ProtocolSupport::Full));
        let config = RequestResponseConfig::default();
        let request_response = RequestResponse::new(codec, protocol, config);
        PeersExchange {
            request_response,
            known_peers: Vec::new(),
            events: VecDeque::new(),
            maintain_peers_interval: Interval::new_at(
                Instant::now() + Duration::from_secs(REQUEST_PEERS_INITIAL_DELAY),
                Duration::from_secs(REQUEST_PEERS_INTERVAL),
            ),
        }
    }

    fn get_random_known_peers(&mut self, num: usize) -> HashMap<PeerIdSerde, PeerAddresses> {
        let mut result = HashMap::with_capacity(num);
        let mut rng = thread_rng();
        let peer_ids = self.known_peers.choose_multiple(&mut rng, num).cloned();
        for peer_id in peer_ids {
            let addresses = self.request_response.addresses_of_peer(&peer_id).into_iter().collect();
            result.insert(peer_id.into(), addresses);
        }
        result
    }

    #[allow(unused)]
    fn forget_peer(&mut self, peer: &PeerId) {
        self.known_peers.retain(|known_peer| known_peer != peer);
        self.forget_peer_addresses(peer);
    }

    fn forget_peer_addresses(&mut self, peer: &PeerId) {
        for address in self.request_response.addresses_of_peer(&peer) {
            self.request_response.remove_address(&peer, &address);
        }
    }

    pub fn add_peer_addresses(&mut self, peer: &PeerId, addresses: PeerAddresses) {
        if !self.known_peers.contains(&peer) && !addresses.is_empty() {
            self.known_peers.push(peer.clone());
        }
        let already_known = self.request_response.addresses_of_peer(peer);
        for address in addresses {
            if !already_known.contains(&address) {
                self.request_response.add_address(&peer, address);
            }
        }
    }

    fn maintain_known_peers(&mut self) {
        if self.known_peers.len() > MAX_PEERS {
            let mut rng = thread_rng();
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
        const DEFAULT_PEERS_NUM: usize = 20;
        let mut rng = thread_rng();
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
        let mut rng = thread_rng();
        let peer_ids = self.known_peers.iter().filter(|peer| filter(*peer)).collect::<Vec<_>>();
        for peer_id in peer_ids.choose_multiple(&mut rng, num) {
            let addresses = self.request_response.addresses_of_peer(*peer_id).into_iter().collect();
            result.insert((*peer_id).clone(), addresses);
        }
        result
    }

    pub fn is_known_peer(&self, peer: &PeerId) -> bool { self.known_peers.contains(peer) }

    pub fn add_known_peer(&mut self, peer: PeerId) {
        if !self.is_known_peer(&peer) {
            self.known_peers.push(peer)
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<RequestProtocol<PeersExchangeCodec>, ()>> {
        while let Poll::Ready(Some(())) = self.maintain_peers_interval.poll_next_unpin(cx) {
            self.maintain_known_peers();
        }

        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(event);
        }

        Poll::Pending
    }
}

impl NetworkBehaviourEventProcess<RequestResponseEvent<PeersExchangeRequest, PeersExchangeResponse>> for PeersExchange {
    fn inject_event(&mut self, event: RequestResponseEvent<PeersExchangeRequest, PeersExchangeResponse>) {
        match event {
            RequestResponseEvent::Message { message, .. } => match message {
                RequestResponseMessage::Request { request, channel, .. } => match request {
                    PeersExchangeRequest::GetKnownPeers { num } => {
                        let response = PeersExchangeResponse::KnownPeers {
                            peers: self.get_random_known_peers(num),
                        };
                        self.request_response.send_response(channel, response);
                    },
                },
                RequestResponseMessage::Response { response, .. } => match response {
                    PeersExchangeResponse::KnownPeers { peers } => {
                        info!("Got peers {:?}", peers);
                        peers.into_iter().for_each(|(peer, addresses)| {
                            self.add_peer_addresses(&peer.0, addresses);
                        });
                    },
                },
            },
            RequestResponseEvent::OutboundFailure {
                peer,
                request_id,
                error,
            } => {
                error!(
                    "Outbound failure {:?} while requesting {:?} to peer {}",
                    error, request_id, peer
                );
            },
            RequestResponseEvent::InboundFailure { peer, error, .. } => {
                error!(
                    "Inbound failure {:?} while processing request from peer {}",
                    error, peer
                );
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::peers_exchange::PeerIdSerde;
    use crate::PeerId;

    #[test]
    fn test_peer_id_serde() {
        let peer_id = PeerIdSerde(PeerId::random());
        let serialized = rmp_serde::to_vec(&peer_id).unwrap();
        let deserialized: PeerIdSerde = rmp_serde::from_read_ref(&serialized).unwrap();
        assert_eq!(peer_id.0, deserialized.0);
    }
}
