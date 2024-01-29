#![allow(unused)] // TODO: remove this

use crate::eth::web3_transport::Web3SendOut;
use crate::eth::RpcTransportEventHandlerShared;
use common::log;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::lock::Mutex as AsyncMutex;
use futures_ticker::Ticker;
use futures_util::{FutureExt, SinkExt, StreamExt};
use instant::Duration;
use jsonrpc_core::Call;
use mm2_net::transport::GuiAuthValidationGenerator;
use std::collections::HashSet;
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
    request_handler: RequestHandler,
    response_handler: ResponseHandler,
}

#[derive(Clone, Debug)]
struct RequestHandler {
    tx: UnboundedSender<Call>,
    rx: Arc<AsyncMutex<UnboundedReceiver<Call>>>,
}

#[derive(Clone, Debug)]
struct ResponseHandler {
    tx: UnboundedSender<serde_json::Value>,
    rx: Arc<AsyncMutex<UnboundedReceiver<serde_json::Value>>>,
}

impl WebsocketTransport {
    pub fn with_event_handlers(
        nodes: Vec<WebsocketTransportNode>,
        event_handlers: Vec<RpcTransportEventHandlerShared>,
    ) -> Self {
        let client_impl = WebsocketTransportRpcClientImpl { nodes };

        let (req_tx, req_rx) = futures::channel::mpsc::unbounded();
        let (res_tx, res_rx) = futures::channel::mpsc::unbounded();

        WebsocketTransport {
            client: Arc::new(WebsocketTransportRpcClient(AsyncMutex::new(client_impl))),
            event_handlers,
            request_id: Arc::new(AtomicUsize::new(0)),
            request_handler: RequestHandler {
                tx: req_tx,
                rx: Arc::new(AsyncMutex::new(req_rx)),
            },
            response_handler: ResponseHandler {
                tx: res_tx,
                rx: Arc::new(AsyncMutex::new(res_rx)),
            },
        }
    }

    async fn start_connection_loop(&self) {
        for node in (*self.client.0.lock().await).nodes.clone() {
            let mut wsocket = match tokio_tungstenite_wasm::connect(node.uri.to_string()).await {
                Ok(ws) => ws,
                Err(e) => {
                    log::error!("{e}");
                    continue;
                },
            };

            // id list of awaiting requests
            let mut awaiting_requests: HashSet<usize> = HashSet::default();
            let mut keepalive_interval = Ticker::new(Duration::from_secs(10));
            let mut req_rx = self.request_handler.rx.lock().await;
            let mut res_tx = self.response_handler.tx.clone();

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
                        if let Some(request) = request {
                            let serialized_request = to_string(&request);
                            if let Err(e) = wsocket.send(tokio_tungstenite_wasm::Message::Text(serialized_request)).await {
                                log::error!("{e}");
                                // TODO: try couple times at least before continue
                                continue;
                            }
                        }
                    }

                    message = wsocket.next().fuse() => {
                        match message {
                             Some(Ok(tokio_tungstenite_wasm::Message::Text(inc_event))) => {
                                 if let Ok(inc_event) = serde_json::from_str::<serde_json::Value>(&inc_event) {
                                     if let Some(id) = inc_event.get("id") {
                                         res_tx.send(inc_event.clone()).await.expect("TODO");
                                         awaiting_requests.remove(&(id.as_u64().unwrap_or_default() as usize));
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

    // TODO: implement timeouts
    async fn rpc_send_and_receive(&self, request: Call, request_id: usize) -> Result<serde_json::Value, Error> {
        let mut tx = self.request_handler.tx.clone();
        let mut rx = self.response_handler.rx.lock().await;
        tx.send(request).await.expect("TODO");

        while let Some(response) = rx.next().await {
            if let Some(id) = response.get("id") {
                if id.as_u64().unwrap_or_default() as usize == request_id {
                    return Ok(response);
                }
            }
        }

        Err(Error::Internal)
    }
}

impl Transport for WebsocketTransport {
    type Out = Web3SendOut;

    fn prepare(&self, method: &str, params: Vec<serde_json::Value>) -> (RequestId, Call) {
        let request_id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = build_request(request_id, method, params);

        (request_id, request)
    }

    fn send(&self, id: RequestId, request: Call) -> Self::Out {
        // send this request to another thread that continiously handles ws connection as a
        // background task; filter the responses with the given ID and return its reponse data.
        todo!()
    }
}

async fn send_request(
    request: Call,
    client: Arc<WebsocketTransportRpcClient>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
    gui_auth_validation_generator: Option<GuiAuthValidationGenerator>,
) -> Result<serde_json::Value, Error> {
    todo!()
}
