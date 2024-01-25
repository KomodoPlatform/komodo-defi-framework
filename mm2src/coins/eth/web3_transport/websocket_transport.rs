#![allow(unused)] // TODO: remove this

use crate::eth::web3_transport::Web3SendOut;
use crate::eth::RpcTransportEventHandlerShared;
use futures::lock::Mutex as AsyncMutex;
use jsonrpc_core::Call;
use std::sync::{atomic::{AtomicUsize, Ordering},
                Arc};
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
