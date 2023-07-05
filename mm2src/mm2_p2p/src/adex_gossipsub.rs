use libp2p::{gossipsub::{Behaviour as Gossipsub, Event as GossipsubEvent, MessageId, Topic},
             swarm::{ConnectionClosed, ConnectionHandler, FromSwarm, NetworkBehaviour},
             PeerId};
use std::collections::{HashMap, HashSet};

pub struct AtomicDexGossipsub {
    gossipsub: Gossipsub,
    /// relays to which we forward the messages. also tracks the relay mesh size of nodes in mesh.
    relays_mesh: HashMap<PeerId, usize>,
    /// The peer ids of connected relay nodes
    connected_relays: HashSet<PeerId>,
}

impl NetworkBehaviour for AtomicDexGossipsub {
    type ConnectionHandler = <Gossipsub as NetworkBehaviour>::ConnectionHandler;
    type ToSwarm = GossipsubEvent;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: libp2p::swarm::ConnectionId,
        peer: PeerId,
        local_addr: &libp2p::Multiaddr,
        remote_addr: &libp2p::Multiaddr,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        self.gossipsub
            .handle_established_inbound_connection(connection_id, peer, local_addr, remote_addr)
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: libp2p::swarm::ConnectionId,
        peer: PeerId,
        addr: &libp2p::Multiaddr,
        role_override: libp2p::core::Endpoint,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        self.gossipsub
            .handle_established_outbound_connection(connection_id, peer, addr, role_override)
    }

    fn on_swarm_event(&mut self, event: libp2p::swarm::FromSwarm<Self::ConnectionHandler>) {
        match event {
            libp2p::swarm::FromSwarm::ConnectionEstablished(_) => {},
            FromSwarm::ConnectionClosed(ref cc) => {
                self.relays_mesh.remove(&cc.peer_id);
            },
            libp2p::swarm::FromSwarm::AddressChange(_) => {},
            libp2p::swarm::FromSwarm::DialFailure(_) => {},
            libp2p::swarm::FromSwarm::ListenFailure(_) => {},
            libp2p::swarm::FromSwarm::NewListener(_) => {},
            libp2p::swarm::FromSwarm::NewListenAddr(_) => {},
            libp2p::swarm::FromSwarm::ExpiredListenAddr(_) => {},
            libp2p::swarm::FromSwarm::ListenerError(_) => {},
            libp2p::swarm::FromSwarm::ListenerClosed(_) => {},
            libp2p::swarm::FromSwarm::NewExternalAddrCandidate(_) => {},
            libp2p::swarm::FromSwarm::ExternalAddrConfirmed(_) => {},
            libp2p::swarm::FromSwarm::ExternalAddrExpired(_) => {},
        }

        self.gossipsub.on_swarm_event(event)
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: libp2p::swarm::ConnectionId,
        event: libp2p::swarm::THandlerOutEvent<Self>,
    ) {
        self.gossipsub
            .on_connection_handler_event(peer_id, connection_id, event)
    }

    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
        params: &mut impl libp2p::swarm::PollParameters,
    ) -> std::task::Poll<libp2p::swarm::ToSwarm<Self::ToSwarm, libp2p::swarm::THandlerInEvent<Self>>> {
        self.gossipsub.poll(cx, params)
    }
}
