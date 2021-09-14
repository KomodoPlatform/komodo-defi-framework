use libp2p::{ping::{Ping, PingConfig, PingEvent},
             swarm::{CloseConnection, NetworkBehaviourAction, NetworkBehaviourEventProcess, PollParameters},
             NetworkBehaviour};
use log::error;
use std::{collections::VecDeque,
          num::NonZeroU32,
          task::{Context, Poll}};
use void::Void;

/// Wrapper around libp2p Ping behaviour that forcefully disconnects a peer using NetworkBehaviourAction::DisconnectPeer
/// event.
/// Libp2p has unclear ConnectionHandlers keep alive logic so in some cases even if Ping handler emits Close event the
/// connection is kept active which is undesirable.
#[derive(NetworkBehaviour)]
#[behaviour(out_event = "Void")]
#[behaviour(poll_method = "poll_event")]
pub struct AdexPing {
    ping: Ping,
    #[behaviour(ignore)]
    events: VecDeque<NetworkBehaviourAction<Void, Void>>,
}

impl NetworkBehaviourEventProcess<PingEvent> for AdexPing {
    fn inject_event(&mut self, event: PingEvent) {
        if let Err(e) = event.result {
            error!("Ping error {}. Disconnecting peer {}", e, event.peer);
            self.events.push_back(NetworkBehaviourAction::CloseConnection {
                peer_id: event.peer,
                connection: CloseConnection::All,
            });
        }
    }
}

#[allow(clippy::new_without_default)]
impl AdexPing {
    pub fn new() -> Self {
        AdexPing {
            ping: Ping::new(PingConfig::new().with_max_failures(unsafe { NonZeroU32::new_unchecked(2) })),
            events: VecDeque::new(),
        }
    }

    fn poll_event(
        &mut self,
        _cx: &mut Context,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<Void, Void>> {
        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(event);
        }

        Poll::Pending
    }
}
