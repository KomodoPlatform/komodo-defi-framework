//! This module offers a transport layer for managing request-response style communication
//! with Ethereum nodes using websockets in a wait and lock-free manner. In comparison to
//! HTTP transport, this approach proves to be much quicker (low-latency) and consumes less
//! bandwidth. This efficiency is achieved by avoiding the handling of TCP
//! handshakes (connection reusability) for each request.

use crate::eth::web3_transport::Web3SendOut;
use crate::eth::{EthCoin, RpcTransportEventHandlerShared};
use crate::{MmCoin, RpcTransportEventHandler};
use common::executor::{AbortSettings, SpawnAbortable};
use common::expirable_map::ExpirableMap;
use common::log;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot;
use futures::lock::Mutex as AsyncMutex;
use futures_ticker::Ticker;
use futures_util::{FutureExt, SinkExt, StreamExt};
use instant::Duration;
use jsonrpc_core::Call;
use std::collections::HashMap;
use std::sync::{atomic::{AtomicUsize, Ordering},
                Arc};
use web3::error::Error;
use web3::helpers::to_string;
use web3::{helpers::build_request, RequestId, Transport};

const REQUEST_TIMEOUT_AS_SEC: u64 = 10;

#[derive(Clone, Debug)]
pub(crate) struct WebsocketTransportNode {
    pub(crate) uri: http::Uri,
    // TODO: We need to support this mechanism on the komodo-defi-proxy
    #[allow(dead_code)]
    pub(crate) gui_auth: bool,
}

#[derive(Clone, Debug)]
pub struct WebsocketTransport {
    request_id: Arc<AtomicUsize>,
    node: WebsocketTransportNode,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
    responses: Arc<SafeMapPtr>,
    controller_channel: Arc<ControllerChannel>,
    connection_guard: Arc<AsyncMutex<()>>,
}

#[derive(Debug)]
struct ControllerChannel {
    tx: Arc<AsyncMutex<UnboundedSender<ControllerMessage>>>,
    rx: Arc<AsyncMutex<UnboundedReceiver<ControllerMessage>>>,
}

enum ControllerMessage {
    Request(WsRequest),
    Close,
}

#[derive(Debug)]
struct WsRequest {
    request: Call,
    request_id: RequestId,
    response_notifier: oneshot::Sender<()>,
}

/// A wrapper type for raw pointers used as a mutable Send & Sync HashMap reference.
///
/// Safety notes:
///
/// The implemented algorithm for socket request-response is already thread-safe,
/// so we don't care about race conditions.
///
/// As for deallocations, see the `Drop` implementation below.
#[derive(Debug)]
struct SafeMapPtr(*mut HashMap<RequestId, serde_json::Value>);

impl Drop for SafeMapPtr {
    fn drop(&mut self) {
        // Let the compiler do the job.
        let _ = unsafe { Box::from_raw(self.0) };
    }
}

unsafe impl Send for SafeMapPtr {}
unsafe impl Sync for SafeMapPtr {}

impl WebsocketTransport {
    pub(crate) fn with_event_handlers(
        node: WebsocketTransportNode,
        event_handlers: Vec<RpcTransportEventHandlerShared>,
    ) -> Self {
        let (req_tx, req_rx) = futures::channel::mpsc::unbounded();

        WebsocketTransport {
            node,
            event_handlers,
            responses: Arc::new(SafeMapPtr(Box::into_raw(Default::default()))),
            request_id: Arc::new(AtomicUsize::new(1)),
            controller_channel: ControllerChannel {
                tx: Arc::new(AsyncMutex::new(req_tx)),
                rx: Arc::new(AsyncMutex::new(req_rx)),
            }
            .into(),
            connection_guard: Arc::new(AsyncMutex::new(())),
        }
    }

    pub(crate) async fn start_connection_loop(self) {
        let _guard = self.connection_guard.lock().await;

        let mut response_notifiers: ExpirableMap<RequestId, oneshot::Sender<()>> = ExpirableMap::default();

        loop {
            let mut wsocket = match tokio_tungstenite_wasm::connect(self.node.uri.to_string()).await {
                Ok(ws) => ws,
                Err(e) => {
                    log::error!("{e}");
                    continue;
                },
            };

            // List of awaiting requests IDs
            let mut keepalive_interval = Ticker::new(Duration::from_secs(10));
            let mut req_rx = self.controller_channel.rx.lock().await;

            loop {
                futures_util::select! {
                    _ = keepalive_interval.next().fuse() => {
                        // Drop expired response notifier channels
                        response_notifiers.clear_expired_entries();

                        const SIMPLE_REQUEST: &str = r#"{"jsonrpc":"2.0","method":"net_version","params":[],"id": 0 }"#;

                        let mut should_continue = Default::default();
                        for _ in 0..3 {
                            match wsocket.send(tokio_tungstenite_wasm::Message::Text(SIMPLE_REQUEST.to_string())).await {
                                Ok(_) => {
                                    should_continue = false;
                                    break;
                                },
                                Err(e) => {
                                    log::error!("{e}");
                                    should_continue = true;
                                }
                            };
                        }

                        if should_continue {
                            continue;
                        }
                    }

                    request = req_rx.next().fuse() => {
                        match request {
                            Some(ControllerMessage::Request(WsRequest { request_id, request, response_notifier })) => {
                                let serialized_request = to_string(&request);
                                response_notifiers.insert(request_id, response_notifier, Duration::from_secs(REQUEST_TIMEOUT_AS_SEC));

                                let mut should_continue = Default::default();
                                for _ in 0..3 {
                                    match wsocket.send(tokio_tungstenite_wasm::Message::Text(serialized_request.clone())).await {
                                        Ok(_) => {
                                            should_continue = false;
                                            break;
                                        },
                                        Err(e) => {
                                            log::error!("{e}");
                                            should_continue = true;
                                        }
                                    }
                                }

                                if should_continue {
                                    let _ = response_notifiers.remove(&request_id);
                                    continue;
                                }
                            },
                            Some(ControllerMessage::Close) => {
                                break;
                            },
                            _ => {},
                        }
                    }

                    message = wsocket.next().fuse() => {
                        match message {
                             Some(Ok(tokio_tungstenite_wasm::Message::Text(inc_event))) => {
                                 if let Ok(inc_event) = serde_json::from_str::<serde_json::Value>(&inc_event) {
                                     if !inc_event.is_object() {
                                         continue;
                                     }

                                     if let Some(id) = inc_event.get("id") {
                                         let request_id = id.as_u64().unwrap_or_default() as usize;

                                         if let Some(notifier) = response_notifiers.remove(&request_id) {
                                             let response_map = unsafe { &mut *self.responses.0 };
                                             let _ = response_map.insert(request_id, inc_event.get("result").expect("TODO").clone());

                                             notifier.send(()).expect("receiver channel must be alive");
                                         }
                                     }
                                 }
                             },
                             Some(Ok(tokio_tungstenite_wasm::Message::Binary(_))) => continue,
                             Some(Ok(tokio_tungstenite_wasm::Message::Close(_))) => return,
                             Some(Err(e)) => {
                                log::error!("{e}");
                                return;
                             },
                             None => continue,
                        }
                    }
                }
            }
        }
    }

    pub(crate) async fn stop_connection_loop(&self) {
        let mut tx = self.controller_channel.tx.lock().await;
        tx.send(ControllerMessage::Close)
            .await
            .expect("receiver channel must be alive");

        let response_map = unsafe { &mut *self.responses.0 };
        response_map.clear();
    }

    pub(crate) fn maybe_spawn_connection_loop(&self, coin: EthCoin) {
        // if we can acquire the lock here, it means connection loop is not alive
        if self.connection_guard.try_lock().is_some() {
            let fut = self.clone().start_connection_loop();
            let settings = AbortSettings::info_on_abort(format!("connection loop stopped for {:?}", self.node.uri));
            coin.spawner().spawn_with_settings(fut, settings);
        }
    }
}

async fn send_request(
    transport: WebsocketTransport,
    request: Call,
    request_id: RequestId,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> Result<serde_json::Value, Error> {
    let mut tx = transport.controller_channel.tx.lock().await;

    let (notification_sender, notification_receiver) = futures::channel::oneshot::channel::<()>();

    let serialized_request = to_string(&request);
    event_handlers.on_outgoing_request(serialized_request.as_bytes());

    tx.send(ControllerMessage::Request(WsRequest {
        request_id,
        request,
        response_notifier: notification_sender,
    }))
    .await
    .expect("receiver channel must be alive");

    if let Ok(_ping) = notification_receiver.await {
        let response_map = unsafe { &mut *transport.responses.0 };
        if let Some(response) = response_map.remove(&request_id) {
            let mut res_bytes: Vec<u8> = Vec::new();
            if serde_json::to_writer(&mut res_bytes, &response).is_ok() {
                event_handlers.on_incoming_response(&res_bytes);
            }

            return Ok(response);
        }
    };

    Err(Error::Internal)
}

impl Transport for WebsocketTransport {
    type Out = Web3SendOut;

    fn prepare(&self, method: &str, params: Vec<serde_json::Value>) -> (RequestId, Call) {
        let request_id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = build_request(request_id, method, params);

        (request_id, request)
    }

    fn send(&self, id: RequestId, request: Call) -> Self::Out {
        Box::pin(send_request(self.clone(), request, id, self.event_handlers.clone()))
    }
}
