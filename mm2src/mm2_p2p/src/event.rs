use libp2p::floodsub::FloodsubEvent;

use crate::peers_exchange::{PeersExchangeRequest, PeersExchangeResponse};
use crate::request_response::RequestResponseBehaviourEvent;

#[derive(Debug)]
pub enum AdexBehaviourEvent {
    Gossipsub(libp2p::gossipsub::Event),
    Floodsub(FloodsubEvent),
    PeersExchange(libp2p::request_response::Event<PeersExchangeRequest, PeersExchangeResponse>),
    Ping(libp2p::ping::Event),
    RequestResponse(RequestResponseBehaviourEvent),
}
