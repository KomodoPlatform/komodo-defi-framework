//! This module offers a transport layer for managing request-response style communication
//! with Ethereum nodes using websockets in a wait and lock-free manner (with unsafe raw-pointers).
//! In comparison to HTTP transport, this approach proves to be much quicker (low-latency) and consumes
//! less bandwidth. This efficiency is achieved by avoiding the handling of TCP handshakes (connection reusability)
//! for each request.

use super::handle_gui_auth_payload;
use super::http_transport::de_rpc_response;
use crate::eth::web3_transport::Web3SendOut;
use crate::eth::{EthCoin, RpcTransportEventHandlerShared};
use crate::{MmCoin, RpcTransportEventHandler};
use common::executor::{AbortSettings, SpawnAbortable, Timer};
use common::expirable_map::ExpirableMap;
use common::log;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot;
use futures::lock::Mutex as AsyncMutex;
use futures_ticker::Ticker;
use futures_util::{FutureExt, SinkExt, StreamExt};
use instant::Duration;
use jsonrpc_core::Call;
use mm2_net::transport::GuiAuthValidationGenerator;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{atomic::{AtomicUsize, Ordering},
                Arc};
use tokio_tungstenite_wasm::WebSocketStream;
use web3::error::{Error, TransportError};
use web3::helpers::to_string;
use web3::{helpers::build_request, RequestId, Transport};

const REQUEST_TIMEOUT_AS_SEC: u64 = 10;

#[derive(Clone, Debug)]
pub(crate) struct WebsocketTransportNode {
    pub(crate) uri: http::Uri,
    pub(crate) gui_auth: bool,
}

#[derive(Clone, Debug)]
pub struct WebsocketTransport {
    request_id: Arc<AtomicUsize>,
    pub(crate) last_request_failed: Arc<AtomicBool>,
    node: WebsocketTransportNode,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
    pub(crate) gui_auth_validation_generator: Option<GuiAuthValidationGenerator>,
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
    serialized_request: String,
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
struct SafeMapPtr(*mut HashMap<RequestId, Vec<u8>>);

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
            gui_auth_validation_generator: None,
            last_request_failed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) async fn start_connection_loop(self) {
        let _guard = self.connection_guard.lock().await;

        const MAX_ATTEMPTS: u32 = 3;
        const SLEEP_DURATION: f64 = 1.;
        const SIMPLE_REQUEST: &str = r#"{"jsonrpc":"2.0","method":"net_version","params":[],"id": 0 }"#;
        const KEEPALIVE_DURATION: Duration = Duration::from_secs(10);

        async fn attempt_to_establish_socket_connection(
            address: String,
            max_attempts: u32,
            sleep_duration_on_failure: f64,
        ) -> tokio_tungstenite_wasm::Result<WebSocketStream> {
            loop {
                let mut attempts = 0;

                match tokio_tungstenite_wasm::connect(address.clone()).await {
                    Ok(ws) => return Ok(ws),
                    Err(e) => {
                        attempts += 1;
                        if attempts > max_attempts {
                            return Err(e);
                        }

                        Timer::sleep(sleep_duration_on_failure).await;
                        continue;
                    },
                };
            }
        }

        // List of awaiting requests
        let mut response_notifiers: ExpirableMap<RequestId, oneshot::Sender<()>> = ExpirableMap::default();

        let mut wsocket =
            match attempt_to_establish_socket_connection(self.node.uri.to_string(), MAX_ATTEMPTS, SLEEP_DURATION).await
            {
                Ok(ws) => ws,
                Err(e) => {
                    log::error!("Connection could not established for {}. Error {e}", self.node.uri);
                    return;
                },
            };

        let mut keepalive_interval = Ticker::new(KEEPALIVE_DURATION);
        let mut req_rx = self.controller_channel.rx.lock().await;

        loop {
            futures_util::select! {
                // KEEPALIVE HANDLING
                _ = keepalive_interval.next().fuse() => {
                    // Drop expired response notifier channels
                    response_notifiers.clear_expired_entries();

                    let mut should_continue = Default::default();
                    for _ in 0..MAX_ATTEMPTS {
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

                        Timer::sleep(SLEEP_DURATION).await;
                    }

                    if should_continue {
                        continue;
                    }
                }

                // SEND REQUESTS
                request = req_rx.next().fuse() => {
                    match request {
                        Some(ControllerMessage::Request(WsRequest { request_id, serialized_request, response_notifier })) => {
                            response_notifiers.insert(request_id, response_notifier, Duration::from_secs(REQUEST_TIMEOUT_AS_SEC));

                            let mut should_continue = Default::default();
                            for _ in 0..MAX_ATTEMPTS {
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

                                Timer::sleep(SLEEP_DURATION).await;
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

                // HANDLE RESPONSES SENT FROM USER
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
                                         let mut res_bytes: Vec<u8> = Vec::new();
                                         if serde_json::to_writer(&mut res_bytes, &inc_event).is_ok() {
                                             let response_map = unsafe { &mut *self.responses.0 };
                                             let _ = response_map.insert(request_id, res_bytes);

                                             notifier.send(()).expect("receiver channel must be alive");
                                         }

                                     }
                                 }
                             }
                         },
                         Some(Ok(tokio_tungstenite_wasm::Message::Binary(_))) => continue,
                         Some(Ok(tokio_tungstenite_wasm::Message::Close(_))) => break,
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
    transport.last_request_failed.store(false, Ordering::SeqCst);
    let mut serialized_request = to_string(&request);

    if transport.node.gui_auth {
        match handle_gui_auth_payload(&transport.gui_auth_validation_generator, &request) {
            Ok(r) => serialized_request = r,
            Err(e) => {
                transport.last_request_failed.store(true, Ordering::SeqCst);
                return Err(Error::Transport(TransportError::Message(format!(
                    "Couldn't generate signed message payload for {:?}. Error: {e}",
                    request
                ))));
            },
        };
    }

    let mut tx = transport.controller_channel.tx.lock().await;

    let (notification_sender, notification_receiver) = futures::channel::oneshot::channel::<()>();

    event_handlers.on_outgoing_request(serialized_request.as_bytes());

    tx.send(ControllerMessage::Request(WsRequest {
        request_id,
        serialized_request,
        response_notifier: notification_sender,
    }))
    .await
    .expect("receiver channel must be alive");

    if let Ok(_ping) = notification_receiver.await {
        let response_map = unsafe { &mut *transport.responses.0 };
        if let Some(response) = response_map.remove(&request_id) {
            event_handlers.on_incoming_response(&response);

            return de_rpc_response(response, &transport.node.uri.to_string());
        }
    };

    transport.last_request_failed.store(true, Ordering::SeqCst);

    Err(Error::Transport(TransportError::Message(format!(
        "Sending {:?} failed.",
        request
    ))))
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
