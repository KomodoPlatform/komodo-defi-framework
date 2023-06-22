use libp2p::ping::{Behaviour, Config, Event};
use libp2p::swarm::{NetworkBehaviour, PollParameters, ToSwarm};
use std::{collections::VecDeque,
          task::{Context, Poll}};
use void::Void;

pub struct AdexPing {
    ping: Behaviour,
    events: VecDeque<ToSwarm<Void, <Self as NetworkBehaviour>::ConnectionHandler>>,
}

impl NetworkBehaviour for AdexPing {
    type ConnectionHandler = <Behaviour as NetworkBehaviour>::ConnectionHandler;

    type ToSwarm = Void;

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: libp2p::swarm::ConnectionId,
        peer: libp2p::PeerId,
        local_addr: &libp2p::Multiaddr,
        remote_addr: &libp2p::Multiaddr,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        todo!()
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: libp2p::swarm::ConnectionId,
        peer: libp2p::PeerId,
        addr: &libp2p::Multiaddr,
        role_override: libp2p::core::Endpoint,
    ) -> Result<libp2p::swarm::THandler<Self>, libp2p::swarm::ConnectionDenied> {
        todo!()
    }

    fn on_swarm_event(&mut self, event: libp2p::swarm::FromSwarm<Self::ConnectionHandler>) { todo!() }

    fn on_connection_handler_event(
        &mut self,
        _peer_id: libp2p::PeerId,
        _connection_id: libp2p::swarm::ConnectionId,
        _event: libp2p::swarm::THandlerOutEvent<Self>,
    ) {
        todo!()
    }

    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
        params: &mut impl PollParameters,
    ) -> std::task::Poll<ToSwarm<Self::ToSwarm, libp2p::swarm::THandlerInEvent<Self>>> {
        todo!()
    }
}

impl From<Event> for AdexPing {
    fn from(value: Event) -> Self { todo!() }
}

#[allow(clippy::new_without_default)]
impl AdexPing {
    pub fn new() -> Self {
        AdexPing {
            ping: Behaviour::new(Config::new()),
            events: VecDeque::new(),
        }
    }

    fn poll_event(
        &mut self,
        _cx: &mut Context,
        _params: &mut impl PollParameters,
    ) -> Poll<ToSwarm<Void, <Self as NetworkBehaviour>::ConnectionHandler>> {
        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(event);
        }

        Poll::Pending
    }
}
