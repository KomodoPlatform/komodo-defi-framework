use futures::channel::mpsc::Receiver;
use libp2p::floodsub::FloodsubEvent;
use libp2p::request_response::Event as RequestResponseEvent;
use libp2p::{gossipsub::{Event as GossipsubEvent, Message as GossipsubMessage, MessageId, TopicHash},
             PeerId};

use crate::behaviour::AdexResponseChannel;
use crate::peers_exchange::{PeersExchangeRequest, PeersExchangeResponse};
use crate::request_response::RequestResponseBehaviourEvent;

impl From<GossipsubEvent> for AdexBehaviourEvent {
    fn from(event: GossipsubEvent) -> Self { AdexBehaviourEvent::Gossipsub(event) }
}

#[derive(Debug)]
pub enum AdexBehaviourEvent {
    Gossipsub(GossipsubEvent),
    Floodsub(FloodsubEvent),
    PeersExchange(libp2p::request_response::Event<PeersExchangeRequest, PeersExchangeResponse>),
    Ping(libp2p::ping::Event),
    RequestResponse(RequestResponseBehaviourEvent),
}
