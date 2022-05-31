use crate::{decode_message, encode_message};
use async_trait::async_trait;
use core::iter;
use futures::channel::{mpsc, oneshot};
use futures::io::{AsyncRead, AsyncWrite};
use futures::task::{Context, Poll};
use futures::StreamExt;
use libp2p::core::upgrade::{read_length_prefixed, write_length_prefixed};
use libp2p::request_response::{ProtocolName, ProtocolSupport, RequestId, RequestResponse, RequestResponseCodec,
                               RequestResponseConfig, RequestResponseEvent, RequestResponseMessage, ResponseChannel};
use libp2p::swarm::{NetworkBehaviour, NetworkBehaviourAction, NetworkBehaviourEventProcess, PollParameters};
use libp2p::NetworkBehaviour;
use libp2p::PeerId;
use log::{debug, error, warn};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::time::Duration;
use wasm_timer::{Instant, Interval};

const MAX_BUFFER_SIZE: usize = 1024 * 1024 - 100;

pub type RequestResponseReceiver = mpsc::UnboundedReceiver<(PeerId, PeerRequest, oneshot::Sender<PeerResponse>)>;
pub type RequestResponseSender = mpsc::UnboundedSender<(PeerId, PeerRequest, oneshot::Sender<PeerResponse>)>;

/// Build a request-response network behaviour.
pub fn build_request_response_behaviour() -> RequestResponseBehaviour {
    let config = RequestResponseConfig::default();
    let protocol = iter::once((Protocol::Version1, ProtocolSupport::Full));
    let inner = RequestResponse::new(Codec::default(), protocol, config);

    let (tx, rx) = mpsc::unbounded();
    let pending_requests = HashMap::new();
    let events = VecDeque::new();
    let timeout = Duration::from_secs(10);
    let timeout_interval = Interval::new(Duration::from_secs(1));

    RequestResponseBehaviour {
        inner,
        rx,
        tx,
        pending_requests,
        events,
        timeout,
        timeout_interval,
    }
}

pub enum RequestResponseBehaviourEvent {
    InboundRequest {
        peer_id: PeerId,
        request: PeerRequest,
        response_channel: ResponseChannel<PeerResponse>,
    },
}

struct PendingRequest {
    tx: oneshot::Sender<PeerResponse>,
    initiated_at: Instant,
}

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "RequestResponseBehaviourEvent", event_process = true)]
#[behaviour(poll_method = "poll_event")]
pub struct RequestResponseBehaviour {
    /// The inner RequestResponse network behaviour.
    inner: RequestResponse<Codec<Protocol, PeerRequest, PeerResponse>>,
    #[behaviour(ignore)]
    rx: RequestResponseReceiver,
    #[behaviour(ignore)]
    tx: RequestResponseSender,
    #[behaviour(ignore)]
    pending_requests: HashMap<RequestId, PendingRequest>,
    /// Events that need to be yielded to the outside when polling.
    #[behaviour(ignore)]
    events: VecDeque<RequestResponseBehaviourEvent>,
    /// Timeout for pending requests
    #[behaviour(ignore)]
    timeout: Duration,
    /// Interval for request timeout check
    #[behaviour(ignore)]
    timeout_interval: Interval,
}

impl RequestResponseBehaviour {
    pub fn sender(&self) -> RequestResponseSender { self.tx.clone() }

    pub fn send_response(&mut self, ch: ResponseChannel<PeerResponse>, rs: PeerResponse) -> Result<(), PeerResponse> {
        self.inner.send_response(ch, rs)
    }

    pub fn send_request(
        &mut self,
        peer_id: &PeerId,
        request: PeerRequest,
        response_tx: oneshot::Sender<PeerResponse>,
    ) -> RequestId {
        let request_id = self.inner.send_request(peer_id, request);
        let pending_request = PendingRequest {
            tx: response_tx,
            initiated_at: Instant::now(),
        };
        assert!(self.pending_requests.insert(request_id, pending_request).is_none());
        request_id
    }

    fn poll_event(
        &mut self,
        cx: &mut Context,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<RequestResponseBehaviourEvent, <Self as NetworkBehaviour>::ConnectionHandler>>
    {
        // poll the `rx`
        match self.rx.poll_next_unpin(cx) {
            // received a request, forward it through the network and put to the `pending_requests`
            Poll::Ready(Some((peer_id, request, response_tx))) => {
                let _request_id = self.send_request(&peer_id, request, response_tx);
            },
            // the channel was closed
            Poll::Ready(None) => panic!("request-response channel has been closed"),
            Poll::Pending => (),
        }

        if let Some(event) = self.events.pop_front() {
            // forward a pending event to the top
            return Poll::Ready(NetworkBehaviourAction::GenerateEvent(event));
        }

        while let Poll::Ready(Some(())) = self.timeout_interval.poll_next_unpin(cx) {
            let now = Instant::now();
            let timeout = self.timeout;
            self.pending_requests.retain(|request_id, pending_request| {
                let retain = now.duration_since(pending_request.initiated_at) < timeout;
                if !retain {
                    warn!("Request {} timed out", request_id);
                }
                retain
            });
        }

        Poll::Pending
    }

    fn process_request(
        &mut self,
        peer_id: PeerId,
        request: PeerRequest,
        response_channel: ResponseChannel<PeerResponse>,
    ) {
        self.events.push_back(RequestResponseBehaviourEvent::InboundRequest {
            peer_id,
            request,
            response_channel,
        })
    }

    fn process_response(&mut self, request_id: RequestId, response: PeerResponse) {
        match self.pending_requests.remove(&request_id) {
            Some(pending) => {
                if let Err(e) = pending.tx.send(response) {
                    error!("{:?}. Request {:?} is not processed", e, request_id);
                }
            },
            _ => error!("Received unknown request {:?}", request_id),
        }
    }
}

impl NetworkBehaviourEventProcess<RequestResponseEvent<PeerRequest, PeerResponse>> for RequestResponseBehaviour {
    fn inject_event(&mut self, event: RequestResponseEvent<PeerRequest, PeerResponse>) {
        let (peer_id, message) = match event {
            RequestResponseEvent::Message { peer, message } => (peer, message),
            RequestResponseEvent::InboundFailure { error, .. } => {
                error!("Error on receive a request: {:?}", error);
                return;
            },
            RequestResponseEvent::OutboundFailure {
                peer,
                request_id,
                error,
            } => {
                error!("Error on send request {:?} to peer {:?}: {:?}", request_id, peer, error);
                let err_response = PeerResponse::Err {
                    err: format!("{:?}", error),
                };
                self.process_response(request_id, err_response);
                return;
            },
            RequestResponseEvent::ResponseSent { .. } => return,
        };

        match message {
            RequestResponseMessage::Request { request, channel, .. } => {
                debug!("Received a request from {:?} peer", peer_id);
                self.process_request(peer_id, request, channel)
            },
            RequestResponseMessage::Response { request_id, response } => {
                debug!(
                    "Received a response to the {:?} request from peer {:?}",
                    request_id, peer_id
                );
                self.process_response(request_id, response)
            },
        }
    }
}

#[derive(Clone)]
pub struct Codec<Proto, Req, Res> {
    phantom: std::marker::PhantomData<(Proto, Req, Res)>,
}

impl<Proto, Req, Res> Default for Codec<Proto, Req, Res> {
    fn default() -> Self {
        Codec {
            phantom: Default::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Protocol {
    Version1,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PeerRequest {
    pub req: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum PeerResponse {
    Ok { res: Vec<u8> },
    None,
    Err { err: String },
}

macro_rules! try_io {
    ($e: expr) => {
        match $e {
            Ok(ok) => ok,
            Err(err) => return Err(io::Error::new(io::ErrorKind::InvalidData, err)),
        }
    };
}

impl ProtocolName for Protocol {
    fn protocol_name(&self) -> &[u8] {
        match self {
            Protocol::Version1 => b"/request-response/1",
        }
    }
}

#[async_trait]
impl<
        Proto: Clone + ProtocolName + Send + Sync,
        Req: DeserializeOwned + Serialize + Send + Sync,
        Res: DeserializeOwned + Serialize + Send + Sync,
    > RequestResponseCodec for Codec<Proto, Req, Res>
{
    type Protocol = Proto;
    type Request = Req;
    type Response = Res;

    async fn read_request<T>(&mut self, _protocol: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_to_end(io).await
    }

    async fn read_response<T>(&mut self, _protocol: &Self::Protocol, io: &mut T) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_to_end(io).await
    }

    async fn write_request<T>(&mut self, _protocol: &Self::Protocol, io: &mut T, req: Self::Request) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_all(io, &req).await
    }

    async fn write_response<T>(&mut self, _protocol: &Self::Protocol, io: &mut T, res: Self::Response) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_all(io, &res).await
    }
}

async fn read_to_end<T, M>(io: &mut T) -> io::Result<M>
where
    T: AsyncRead + Unpin + Send,
    M: DeserializeOwned,
{
    match read_length_prefixed(io, MAX_BUFFER_SIZE).await {
        Ok(data) => Ok(try_io!(decode_message(&data))),
        Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
    }
}

async fn write_all<T, M>(io: &mut T, msg: &M) -> io::Result<()>
where
    T: AsyncWrite + Unpin + Send,
    M: Serialize,
{
    let data = try_io!(encode_message(msg));
    if data.len() > MAX_BUFFER_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Try to send data size over maximum",
        ));
    }
    write_length_prefixed(io, data).await
}
