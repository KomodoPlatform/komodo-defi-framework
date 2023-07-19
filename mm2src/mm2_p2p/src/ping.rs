use libp2p::ping::{Behaviour, Config};
use libp2p::swarm::{CloseConnection, NetworkBehaviour, PollParameters, ToSwarm};
use log::error;
use std::{collections::VecDeque, task::Poll};
use void::Void;

pub struct AdexPing {
    ping: Behaviour,
    events: VecDeque<ToSwarm<<Behaviour as NetworkBehaviour>::ToSwarm, Void>>,
}

impl NetworkBehaviour for AdexPing {
    type ConnectionHandler = <Behaviour as NetworkBehaviour>::ConnectionHandler;
    type ToSwarm = <Behaviour as NetworkBehaviour>::ToSwarm;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: libp2p::swarm::ConnectionId,
        peer: libp2p::PeerId,
        local_addr: &libp2p::Multiaddr,
        remote_addr: &libp2p::Multiaddr,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        self.ping
            .handle_established_inbound_connection(connection_id, peer, local_addr, remote_addr)
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: libp2p::swarm::ConnectionId,
        peer: libp2p::PeerId,
        addr: &libp2p::Multiaddr,
        role_override: libp2p::core::Endpoint,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        self.ping
            .handle_established_outbound_connection(connection_id, peer, addr, role_override)
    }

    fn on_swarm_event(&mut self, event: libp2p::swarm::FromSwarm<Self::ConnectionHandler>) {
        self.ping.on_swarm_event(event)
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: libp2p::PeerId,
        connection_id: libp2p::swarm::ConnectionId,
        event: libp2p::swarm::THandlerOutEvent<Self>,
    ) {
        if let Err(e) = &event {
            error!("Ping error {}. Disconnecting peer {}", e, peer_id);
            self.events.push_back(ToSwarm::CloseConnection {
                peer_id,
                connection: CloseConnection::All,
            });
        }

        self.ping.on_connection_handler_event(peer_id, connection_id, event)
    }

    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
        params: &mut impl PollParameters,
    ) -> std::task::Poll<ToSwarm<Self::ToSwarm, libp2p::swarm::THandlerInEvent<Self>>> {
        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(event);
        }

        self.ping.poll(cx, params)
    }
}

#[allow(clippy::new_without_default)]
impl AdexPing {
    pub fn new() -> Self {
        AdexPing {
            ping: Behaviour::new(Config::new()),
            events: VecDeque::new(),
        }
    }
}
