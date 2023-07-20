use libp2p::{core::ConnectedPoint,
             swarm::{derive_prelude::ConnectionEstablished, dummy, ConnectionClosed, ConnectionId, FromSwarm,
                     NetworkBehaviour},
             Multiaddr, PeerId};
use smallvec::SmallVec;
use std::{collections::HashMap, task::Poll};
use void::Void;

/// The ID of an inbound or outbound request.
///
/// Note: [`RequestId`]'s uniqueness is only guaranteed between two
/// inbound and likewise between two outbound requests. There is no
/// uniqueness guarantee in a set of both inbound and outbound
/// [`RequestId`]s nor in a set of inbound or outbound requests
/// originating from different [`Behaviour`]'s.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct RequestId(u64);

/// Internal information tracked for an established connection.
pub struct Connection {
    pub id: ConnectionId,
    pub address: Option<Multiaddr>,
}

/// A request/response protocol for some message codec.
#[derive(Default)]
pub struct Behaviour {
    /// The currently connected peers, their pending outbound and inbound responses and their known,
    /// reachable addresses, if any.
    pub connected: HashMap<PeerId, SmallVec<[Connection; 2]>>,
    /// Externally managed addresses via `add_address` and `remove_address`.
    pub addresses: HashMap<PeerId, SmallVec<[Multiaddr; 6]>>,
}

impl Behaviour {
    /// Adds a known address for a peer that can be used for
    /// dialing attempts by the `Swarm`, i.e. is returned
    /// by [`NetworkBehaviour::handle_pending_outbound_connection`].
    ///
    /// Addresses added in this way are only removed by `remove_address`.
    pub fn add_address(&mut self, peer: &PeerId, address: Multiaddr) {
        self.addresses.entry(*peer).or_default().push(address);
    }

    /// Removes an address of a peer previously added via `add_address`.
    pub fn remove_address(&mut self, peer: &PeerId, address: &Multiaddr) {
        let mut last = false;
        if let Some(addresses) = self.addresses.get_mut(peer) {
            addresses.retain(|a| a != address);
            last = addresses.is_empty();
        }
        if last {
            self.addresses.remove(peer);
        }
    }

    /// Checks whether a peer is currently connected.
    pub fn is_connected(&self, peer: &PeerId) -> bool {
        if let Some(connections) = self.connected.get(peer) {
            !connections.is_empty()
        } else {
            false
        }
    }

    /// Addresses that this behaviour is aware of for this specific peer, and that may allow
    /// reaching the peer.
    ///
    /// The addresses will be tried in the order returned by this function, which means that they
    /// should be ordered by decreasing likelihood of reachability. In other words, the first
    /// address should be the most likely to be reachable.
    pub fn addresses_of_peer(&self, peer: &PeerId) -> Vec<Multiaddr> {
        let mut addresses = Vec::new();
        if let Some(connections) = self.connected.get(peer) {
            addresses.extend(connections.iter().filter_map(|c| c.address.clone()))
        }
        if let Some(more) = self.addresses.get(peer) {
            addresses.extend(more.into_iter().cloned());
        }
        addresses
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = dummy::ConnectionHandler;

    type ToSwarm = Void;

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        Ok(dummy::ConnectionHandler)
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _addr: &Multiaddr,
        _role_override: libp2p::core::Endpoint,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        Ok(dummy::ConnectionHandler)
    }

    fn on_swarm_event(&mut self, event: libp2p::swarm::FromSwarm<Self::ConnectionHandler>) {
        match event {
            FromSwarm::ConnectionClosed(cc) => {
                let ConnectionClosed {
                    peer_id,
                    connection_id,
                    remaining_established,
                    ..
                } = cc;

                let connections = self
                    .connected
                    .get_mut(&peer_id)
                    .expect("Expected some established connection to peer before closing.");

                connections
                    .iter()
                    .position(|c| c.id == connection_id)
                    .map(|p: usize| connections.remove(p))
                    .expect("Expected connection to be established before closing.");

                debug_assert_eq!(connections.is_empty(), remaining_established == 0);
                if connections.is_empty() {
                    self.connected.remove(&peer_id);
                }
            },
            FromSwarm::ConnectionEstablished(ce) => {
                let ConnectionEstablished {
                    peer_id,
                    connection_id,
                    endpoint,
                    ..
                } = ce;

                let address = match endpoint {
                    ConnectedPoint::Dialer { address, .. } => Some(address.clone()),
                    ConnectedPoint::Listener { .. } => None,
                };

                self.connected.entry(peer_id).or_default().push(Connection {
                    id: connection_id,
                    address,
                });
            },
            FromSwarm::AddressChange(_) => {},
            FromSwarm::DialFailure(_) => {},
            FromSwarm::ListenFailure(_) => {},
            FromSwarm::NewListener(_) => {},
            FromSwarm::NewListenAddr(_) => {},
            FromSwarm::ExpiredListenAddr(_) => {},
            FromSwarm::ListenerError(_) => {},
            FromSwarm::ListenerClosed(_) => {},
            FromSwarm::NewExternalAddrCandidate(_) => {},
            FromSwarm::ExternalAddrExpired(_) => {},
            FromSwarm::ExternalAddrConfirmed(_) => {},
        }
    }

    fn on_connection_handler_event(
        &mut self,
        _peer_id: PeerId,
        _connection_id: ConnectionId,
        event: libp2p::swarm::THandlerOutEvent<Self>,
    ) {
        void::unreachable(event)
    }

    fn poll(
        &mut self,
        _cx: &mut std::task::Context<'_>,
        _params: &mut impl libp2p::swarm::PollParameters,
    ) -> std::task::Poll<libp2p::swarm::ToSwarm<Self::ToSwarm, libp2p::swarm::THandlerInEvent<Self>>> {
        Poll::Pending
    }
}
