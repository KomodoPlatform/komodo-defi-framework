#![allow(unused)] // TODO: remove this

use crate::eth::RpcTransportEventHandlerShared;
use crate::{eth::web3_transport::Web3SendOut, rpc_command::lightning::nodes};
use common::log;
use futures::lock::Mutex as AsyncMutex;
use jsonrpc_core::Call;
use mm2_net::transport::GuiAuthValidationGenerator;
use std::sync::{atomic::{AtomicUsize, Ordering},
                Arc};
use web3::error::Error;
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
}

impl WebsocketTransport {
    pub fn with_event_handlers(
        nodes: Vec<WebsocketTransportNode>,
        event_handlers: Vec<RpcTransportEventHandlerShared>,
    ) -> Self {
        let client_impl = WebsocketTransportRpcClientImpl { nodes };

        WebsocketTransport {
            client: Arc::new(WebsocketTransportRpcClient(AsyncMutex::new(client_impl))),
            event_handlers,
            request_id: Arc::new(AtomicUsize::new(0)),
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

            // TODO
        }
    }

    async fn stop_connection_loop(&self) { todo!() }

    async fn rpc_send_and_receive(&self) { todo!() }
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
