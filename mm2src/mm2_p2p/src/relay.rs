use std::collections::{HashMap, HashSet};

use futures_ticker::Ticker;
use libp2p::{gossipsub::{MessageId, TopicHash},
             swarm::{dummy, NetworkBehaviour},
             PeerId};
use void::Void;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerInfo {
    pub peer_id: Option<PeerId>,
    //TODO add this when RFC: Signed Address Records got added to the spec (see pull request
    // https://github.com/libp2p/specs/pull/217)
    //pub signed_peer_record: ?,
}

/// A Control message received by the gossipsub system.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ControlAction {
    /// Node broadcasts known messages per topic - IHave control message.
    IHave {
        /// The topic of the messages.
        topic_hash: TopicHash,
        /// A list of known message ids (peer_id + sequence _number) as a string.
        message_ids: Vec<MessageId>,
    },
    /// The node requests specific message ids (peer_id + sequence _number) - IWant control message.
    IWant {
        /// A list of known message ids (peer_id + sequence _number) as a string.
        message_ids: Vec<MessageId>,
    },
    /// The node has been added to the mesh - Graft control message.
    Graft {
        /// The mesh topic the peer should be added to.
        topic_hash: TopicHash,
    },
    /// The node has been removed from the mesh - Prune control message.
    Prune {
        /// The mesh topic the peer should be removed from.
        topic_hash: TopicHash,
        /// A list of peers to be proposed to the removed peer as peer exchange
        peers: Vec<PeerInfo>,
        /// The backoff time in seconds before we allow to reconnect
        backoff: Option<u64>,
    },
    IAmRelay(bool),
    /// Whether the node included or excluded from other node relays mesh
    IncludedToRelaysMesh {
        included: bool,
        mesh_size: usize,
    },
    MeshSize(usize),
}

pub struct Behaviour {
    /// Pools non-urgent control messages between heartbeats.
    control_pool: HashMap<PeerId, Vec<ControlAction>>,

    /// The peer ids of connected relay nodes
    connected_relays: HashSet<PeerId>,

    /// relays to which we forward the messages. Also tracks the relay mesh size of nodes in mesh.
    relays_mesh: HashMap<PeerId, usize>,

    /// Peers included our node to their relays mesh
    included_to_relays_mesh: HashSet<PeerId>,

    /// Relay mesh maintenance interval stream.
    relay_mesh_maintenance_interval: Ticker,

    /// The relay list which are forcefully kept in relay mesh
    explicit_relay_list: Vec<PeerId>,

    pub am_i_relay: bool,
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = dummy::ConnectionHandler;

    type ToSwarm = libp2p::gossipsub::Event;

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

    fn on_swarm_event(&mut self, event: libp2p::swarm::FromSwarm<Self::ConnectionHandler>) {
        match event {
            libp2p::swarm::FromSwarm::ConnectionEstablished(_) => todo!(),
            libp2p::swarm::FromSwarm::ConnectionClosed(_) => todo!(),
            libp2p::swarm::FromSwarm::AddressChange(_) => todo!(),
            libp2p::swarm::FromSwarm::DialFailure(_) => todo!(),
            libp2p::swarm::FromSwarm::ListenFailure(_) => todo!(),
            libp2p::swarm::FromSwarm::NewListener(_) => todo!(),
            libp2p::swarm::FromSwarm::NewListenAddr(_) => todo!(),
            libp2p::swarm::FromSwarm::ExpiredListenAddr(_) => todo!(),
            libp2p::swarm::FromSwarm::ListenerError(_) => todo!(),
            libp2p::swarm::FromSwarm::ListenerClosed(_) => todo!(),
            libp2p::swarm::FromSwarm::NewExternalAddrCandidate(_) => todo!(),
            libp2p::swarm::FromSwarm::ExternalAddrConfirmed(_) => todo!(),
            libp2p::swarm::FromSwarm::ExternalAddrExpired(_) => todo!(),
        }

        todo!()
    }

    fn on_connection_handler_event(
        &mut self,
        _peer_id: PeerId,
        _connection_id: libp2p::swarm::ConnectionId,
        event: libp2p::swarm::THandlerOutEvent<Self>,
    ) {
        todo!()
    }

    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
        params: &mut impl libp2p::swarm::PollParameters,
    ) -> std::task::Poll<libp2p::swarm::ToSwarm<Self::ToSwarm, libp2p::swarm::THandlerInEvent<Self>>> {
        todo!()
    }
}
