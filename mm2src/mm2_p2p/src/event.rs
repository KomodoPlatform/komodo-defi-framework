use libp2p::{gossipsub::{Event as GossipsubEvent, Message as GossipsubMessage, MessageId, TopicHash},
             PeerId};

use crate::request_response::AdexResponseChannel;

#[derive(Debug)]
pub enum AdexBehaviourEvent {
    /// A message has been received.
    /// Derived from GossipsubEvent.
    Message(PeerId, MessageId, GossipsubMessage),
    /// A remote subscribed to a topic.
    Subscribed {
        /// Remote that has subscribed.
        peer_id: PeerId,
        /// The topic it has subscribed to.
        topic: TopicHash,
    },
    /// A remote unsubscribed from a topic.
    Unsubscribed {
        /// Remote that has unsubscribed.
        peer_id: PeerId,
        /// The topic it has subscribed from.
        topic: TopicHash,
    },
    /// A remote peer sent a request and waits for a response.
    PeerRequest {
        /// Remote that sent this request.
        peer_id: PeerId,
        /// The serialized data.
        request: Vec<u8>,
        /// A channel for sending a response to this request.
        /// The channel is used to identify the peer on the network that is waiting for an answer to this request.
        /// See [`AdexBehaviourCmd::SendResponse`].
        response_channel: AdexResponseChannel,
    },
}

impl From<GossipsubEvent> for AdexBehaviourEvent {
    fn from(event: GossipsubEvent) -> Self {
        match event {
            GossipsubEvent::Message {
                propagation_source,
                message_id,
                message,
            } => AdexBehaviourEvent::Message(propagation_source, message_id, message),
            GossipsubEvent::Subscribed { peer_id, topic } => AdexBehaviourEvent::Subscribed { peer_id, topic },
            GossipsubEvent::Unsubscribed { peer_id, topic } => AdexBehaviourEvent::Unsubscribed { peer_id, topic },
            GossipsubEvent::GossipsubNotSupported { peer_id } => {
                todo!()
            },
        }
    }
}
