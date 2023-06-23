use std::collections::HashMap;

use futures::channel::mpsc::{Receiver, Sender};
use futures::{channel::oneshot,
              future::{join_all, poll_fn},
              Future, FutureExt, SinkExt, StreamExt};
use instant::Duration;
use libp2p::floodsub::Floodsub;
use libp2p::gossipsub::{Behaviour as Gossipsub, MessageId};
use libp2p::request_response::ResponseChannel;
use libp2p::PeerId;

use crate::peers::PeersExchange;
use crate::ping::AdexPing;
use crate::request_response::{PeerResponse, RequestResponseBehaviour};
use crate::swarm_runtime::SwarmRuntime;
use crate::{event::AdexBehaviourEvent, peers::PeerAddresses};

pub type AdexCmdTx = Sender<AdexBehaviourCmd>;
pub type AdexEventRx = Receiver<AdexBehaviourEvent>;

pub const PEERS_TOPIC: &str = "PEERS";
const CONNECTED_RELAYS_CHECK_INTERVAL: Duration = Duration::from_secs(30);
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(600);
const ANNOUNCE_INITIAL_DELAY: Duration = Duration::from_secs(60);
const CHANNEL_BUF_SIZE: usize = 1024 * 8;

/// The structure is the same as `PeerResponse`,
/// but is used to prevent `PeerResponse` from being used outside the network implementation.
#[derive(Debug, Eq, PartialEq)]
pub enum AdexResponse {
    Ok { response: Vec<u8> },
    None,
    Err { error: String },
}

impl From<PeerResponse> for AdexResponse {
    fn from(res: PeerResponse) -> Self {
        match res {
            PeerResponse::Ok { res } => AdexResponse::Ok { response: res },
            PeerResponse::None => AdexResponse::None,
            PeerResponse::Err { err } => AdexResponse::Err { error: err },
        }
    }
}

impl From<AdexResponse> for PeerResponse {
    fn from(res: AdexResponse) -> Self {
        match res {
            AdexResponse::Ok { response } => PeerResponse::Ok { res: response },
            AdexResponse::None => PeerResponse::None,
            AdexResponse::Err { error } => PeerResponse::Err { err: error },
        }
    }
}

#[derive(Debug)]
pub struct AdexResponseChannel(ResponseChannel<PeerResponse>);

impl From<ResponseChannel<PeerResponse>> for AdexResponseChannel {
    fn from(res: ResponseChannel<PeerResponse>) -> Self { AdexResponseChannel(res) }
}

impl From<AdexResponseChannel> for ResponseChannel<PeerResponse> {
    fn from(res: AdexResponseChannel) -> Self { res.0 }
}

#[derive(Debug)]
pub enum AdexBehaviourCmd {
    Subscribe {
        /// Subscribe to this topic
        topic: String,
    },
    PublishMsg {
        topics: Vec<String>,
        msg: Vec<u8>,
    },
    PublishMsgFrom {
        topics: Vec<String>,
        msg: Vec<u8>,
        from: PeerId,
    },
    /// Request relays sequential until a response is received.
    RequestAnyRelay {
        req: Vec<u8>,
        response_tx: oneshot::Sender<Option<(PeerId, Vec<u8>)>>,
    },
    /// Request given peers and collect all their responses.
    RequestPeers {
        req: Vec<u8>,
        peers: Vec<String>,
        response_tx: oneshot::Sender<Vec<(PeerId, AdexResponse)>>,
    },
    /// Request relays and collect all their responses.
    RequestRelays {
        req: Vec<u8>,
        response_tx: oneshot::Sender<Vec<(PeerId, AdexResponse)>>,
    },
    /// Send a response using a `response_channel`.
    SendResponse {
        /// Response to a request.
        res: AdexResponse,
        /// Pass the same `response_channel` as that was obtained from [`AdexBehaviourEvent::PeerRequest`].
        response_channel: AdexResponseChannel,
    },
    GetPeersInfo {
        result_tx: oneshot::Sender<HashMap<String, Vec<String>>>,
    },
    GetGossipMesh {
        result_tx: oneshot::Sender<HashMap<String, Vec<String>>>,
    },
    GetGossipPeerTopics {
        result_tx: oneshot::Sender<HashMap<String, Vec<String>>>,
    },
    GetGossipTopicPeers {
        result_tx: oneshot::Sender<HashMap<String, Vec<String>>>,
    },
    GetRelayMesh {
        result_tx: oneshot::Sender<Vec<String>>,
    },
    /// Add a reserved peer to the peer exchange.
    AddReservedPeer {
        peer: PeerId,
        addresses: PeerAddresses,
    },
    PropagateMessage {
        message_id: MessageId,
        propagation_source: PeerId,
    },
}

/// Returns info about connected peers
pub async fn get_peers_info(mut cmd_tx: AdexCmdTx) -> HashMap<String, Vec<String>> {
    let (result_tx, rx) = oneshot::channel();
    let cmd = AdexBehaviourCmd::GetPeersInfo { result_tx };
    cmd_tx.send(cmd).await.expect("Rx should be present");
    rx.await.expect("Tx should be present")
}

/// Returns current gossipsub mesh state
pub async fn get_gossip_mesh(mut cmd_tx: AdexCmdTx) -> HashMap<String, Vec<String>> {
    let (result_tx, rx) = oneshot::channel();
    let cmd = AdexBehaviourCmd::GetGossipMesh { result_tx };
    cmd_tx.send(cmd).await.expect("Rx should be present");
    rx.await.expect("Tx should be present")
}

pub async fn get_gossip_peer_topics(mut cmd_tx: AdexCmdTx) -> HashMap<String, Vec<String>> {
    let (result_tx, rx) = oneshot::channel();
    let cmd = AdexBehaviourCmd::GetGossipPeerTopics { result_tx };
    cmd_tx.send(cmd).await.expect("Rx should be present");
    rx.await.expect("Tx should be present")
}

pub async fn get_gossip_topic_peers(mut cmd_tx: AdexCmdTx) -> HashMap<String, Vec<String>> {
    let (result_tx, rx) = oneshot::channel();
    let cmd = AdexBehaviourCmd::GetGossipTopicPeers { result_tx };
    cmd_tx.send(cmd).await.expect("Rx should be present");
    rx.await.expect("Tx should be present")
}

pub async fn get_relay_mesh(mut cmd_tx: AdexCmdTx) -> Vec<String> {
    let (result_tx, rx) = oneshot::channel();
    let cmd = AdexBehaviourCmd::GetRelayMesh { result_tx };
    cmd_tx.send(cmd).await.expect("Rx should be present");
    rx.await.expect("Tx should be present")
}

pub struct AtomicDexBehaviour {
    gossipsub: Gossipsub,
    floodsub: Floodsub,
    // #[behaviour(ignore)]
    event_tx: Sender<AdexBehaviourEvent>,
    // #[behaviour(ignore)]
    runtime: SwarmRuntime,
    // #[behaviour(ignore)]
    cmd_rx: Receiver<AdexBehaviourCmd>,
    // #[behaviour(ignore)]
    request_response: RequestResponseBehaviour,
    peers_exchange: PeersExchange,
    ping: AdexPing,
    netid: u16,
}
