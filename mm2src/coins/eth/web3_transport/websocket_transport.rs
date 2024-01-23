#![allow(unused)] // TODO: remove this

use crate::eth::web3_transport::Web3SendOut;
use futures::lock::Mutex as AsyncMutex;
use jsonrpc_core::Call;
use std::sync::Arc;
use web3::{RequestId, Transport};

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
    client: Arc<WebsocketTransportRpcClient>,
}

impl Transport for WebsocketTransport {
    type Out = Web3SendOut;

    fn prepare(&self, method: &str, params: Vec<serde_json::Value>) -> (RequestId, Call) { todo!() }

    fn send(&self, id: RequestId, request: Call) -> Self::Out { todo!() }
}
