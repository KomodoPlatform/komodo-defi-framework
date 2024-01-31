#![allow(unused)] // TODO: remove this

use crate::eth::web3_transport::Web3SendOut;
use crate::eth::RpcTransportEventHandlerShared;
use common::executor::Timer;
use common::log;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot;
use futures::lock::Mutex as AsyncMutex;
use futures_ticker::Ticker;
use futures_util::{FutureExt, SinkExt, StreamExt};
use instant::Duration;
use jsonrpc_core::Call;
use mm2_net::transport::GuiAuthValidationGenerator;
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::{atomic::{AtomicUsize, Ordering},
                Arc};
use web3::error::Error;
use web3::helpers::to_string;
use web3::{helpers::build_request, RequestId, Transport};

#[derive(Clone, Debug)]
pub struct WebsocketTransportNode {
    pub(crate) uri: http::Uri,
    pub(crate) gui_auth: bool,
}

#[derive(Debug)]
struct WebsocketTransportRpcClient(AsyncMutex<WebsocketTransportRpcClientImpl>);

#[derive(Debug)]
struct WebsocketTransportRpcClientImpl {
    nodes: Vec<WebsocketTransportNode>,
}

#[derive(Clone, Debug)]
pub struct WebsocketTransport {
    request_id: Arc<AtomicUsize>,
    client: Arc<WebsocketTransportRpcClient>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
    // TODO: explain why this is used and how safe it is
    responses: SafeMapPtr,
    request_handler: RequestHandler,
}

#[derive(Clone, Debug)]
struct RequestHandler {
    tx: UnboundedSender<WsRequest>,
    rx: Arc<AsyncMutex<UnboundedReceiver<WsRequest>>>,
}

#[derive(Debug)]
struct WsRequest {
    request: Call,
    request_id: RequestId,
    response_notifier: oneshot::Sender<()>,
}

#[derive(Clone, Debug)]
struct SafeMapPtr(*mut HashMap<RequestId, serde_json::Value>);

unsafe impl Send for SafeMapPtr {}
unsafe impl Sync for SafeMapPtr {}

impl WebsocketTransport {
    pub fn with_event_handlers(
        nodes: Vec<WebsocketTransportNode>,
        event_handlers: Vec<RpcTransportEventHandlerShared>,
    ) -> Self {
        let client_impl = WebsocketTransportRpcClientImpl { nodes };

        let (req_tx, req_rx) = futures::channel::mpsc::unbounded();

        let mut hashmap = HashMap::default();

        WebsocketTransport {
            client: Arc::new(WebsocketTransportRpcClient(AsyncMutex::new(client_impl))),
            event_handlers,
            responses: SafeMapPtr(Box::into_raw(hashmap.into())),
            request_id: Arc::new(AtomicUsize::new(1)),
            request_handler: RequestHandler {
                tx: req_tx,
                rx: Arc::new(AsyncMutex::new(req_rx)),
            },
        }
    }

    pub async fn start_connection_loop(self) {
        // TODO: clear disconnected channels every 30s or so.
        let mut response_map: HashMap<RequestId, oneshot::Sender<()>> = HashMap::new();

        for node in (*self.client.0.lock().await).nodes.clone() {
            let mut wsocket = match tokio_tungstenite_wasm::connect(node.uri.to_string()).await {
                Ok(ws) => ws,
                Err(e) => {
                    log::error!("{e}");
                    continue;
                },
            };

            // id list of awaiting requests
            let mut keepalive_interval = Ticker::new(Duration::from_secs(10));
            let mut req_rx = self.request_handler.rx.lock().await;

            loop {
                futures_util::select! {
                    _ = keepalive_interval.next().fuse() => {
                        const SIMPLE_REQUEST: &str = r#"{"jsonrpc":"2.0","method":"net_version","params":[],"id": 0 }"#;

                        if let Err(e) = wsocket.send(tokio_tungstenite_wasm::Message::Text(SIMPLE_REQUEST.to_string())).await {
                            log::error!("{e}");
                            // TODO: try couple times at least before continue
                            continue;
                        }
                    }

                    request = req_rx.next().fuse() => {
                        if let Some( WsRequest { request_id, request, response_notifier }) = request {
                            let serialized_request = to_string(&request);
                            response_map.insert(request_id, response_notifier);
                            if let Err(e) = wsocket.send(tokio_tungstenite_wasm::Message::Text(serialized_request)).await {
                                log::error!("{e}");
                                let _ = response_map.remove(&request_id);
                                // TODO: try couple times at least before continue
                                continue;
                            }
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

                                         if let Some(notifier) = response_map.remove(&request_id) {
                                             let response_map = unsafe { &mut *self.responses.0 };
                                             let _ = response_map.insert(request_id, inc_event.get("result").expect("TODO").clone());
                                             notifier.send(()).expect("TODO");
                                         }
                                     }
                                 }
                             },
                             Some(Ok(tokio_tungstenite_wasm::Message::Binary(_))) => todo!(),
                             Some(Ok(tokio_tungstenite_wasm::Message::Close(_))) => break,
                             Some(Err(e)) => {
                                log::error!("{e}");
                                break;
                             },
                             None => continue,
                        }
                    }
                }
            }
        }
    }

    async fn stop_connection(self) { todo!() }
}

async fn rpc_send_and_receive(
    transport: WebsocketTransport,
    request: Call,
    request_id: RequestId,
) -> Result<serde_json::Value, Error> {
    let mut tx = transport.request_handler.tx.clone();

    let (notification_sender, notification_receiver) = futures::channel::oneshot::channel::<()>();

    tx.send(WsRequest {
        request_id,
        request,
        response_notifier: notification_sender,
    })
    .await
    .expect("TODO");

    if let Ok(_ping) = notification_receiver.await {
        let response_map = unsafe { &mut *transport.responses.0 };
        if let Some(response) = response_map.remove(&request_id) {
            return Ok(response.clone());
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
        Box::pin(rpc_send_and_receive(self.clone(), request, id))
    }
}
