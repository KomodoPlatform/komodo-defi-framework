use crate::{MmCoin, RpcTransportEventHandlerShared};
use common::executor::{AbortSettings, SpawnAbortable};
use ethereum_types::U256;
use futures::future::BoxFuture;
use jsonrpc_core::Call;
use mm2_core::mm_ctx::MmArc;
#[cfg(target_arch = "wasm32")] use mm2_metamask::MetamaskResult;
use mm2_net::transport::GuiAuthValidationGenerator;
use serde_json::Value as Json;
use serde_json::Value;
use web3::api::Namespace;
use web3::helpers::{self, CallFuture};
use web3::types::BlockNumber;
use web3::{Error, RequestId, Transport};

pub(crate) mod http_transport;
#[cfg(target_arch = "wasm32")] pub(crate) mod metamask_transport;
pub(crate) mod websocket_transport;

type Web3SendOut = BoxFuture<'static, Result<Json, Error>>;

#[derive(Clone, Debug)]
pub(crate) enum Web3Transport {
    Http(http_transport::HttpTransport),
    Websocket(websocket_transport::WebsocketTransport),
    #[cfg(target_arch = "wasm32")]
    Metamask(metamask_transport::MetamaskTransport),
}

impl Web3Transport {
    pub fn new_http(
        nodes: Vec<http_transport::HttpTransportNode>,
        event_handlers: Vec<RpcTransportEventHandlerShared>,
    ) -> Web3Transport {
        http_transport::HttpTransport::with_event_handlers(nodes, event_handlers).into()
    }

    pub fn new_websocket(
        ctx: &MmArc,
        nodes: Vec<websocket_transport::WebsocketTransportNode>,
        event_handlers: Vec<RpcTransportEventHandlerShared>,
    ) -> Web3Transport {
        let transport = websocket_transport::WebsocketTransport::with_event_handlers(nodes, event_handlers);

        // TODO: Don't do this here
        let fut = transport.clone().start_connection_loop();
        let settings = AbortSettings::info_on_abort("TODO".to_string());
        ctx.spawner().spawn_with_settings(fut, settings);

        transport.into()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new_metamask(
        eth_config: metamask_transport::MetamaskEthConfig,
        event_handlers: Vec<RpcTransportEventHandlerShared>,
    ) -> MetamaskResult<Web3Transport> {
        Ok(metamask_transport::MetamaskTransport::detect(eth_config, event_handlers)?.into())
    }

    #[cfg(test)]
    pub fn with_nodes(nodes: Vec<http_transport::HttpTransportNode>) -> Web3Transport {
        http_transport::HttpTransport::new(nodes).into()
    }

    #[allow(dead_code)]
    pub fn single_node(url: &'static str, gui_auth: bool) -> Self {
        http_transport::HttpTransport::single_node(url, gui_auth).into()
    }

    pub fn gui_auth_validation_generator_as_mut(&mut self) -> Option<&mut GuiAuthValidationGenerator> {
        match self {
            Web3Transport::Http(http) => http.gui_auth_validation_generator.as_mut(),
            // TODO
            Web3Transport::Websocket(_) => None,
            #[cfg(target_arch = "wasm32")]
            Web3Transport::Metamask(_) => None,
        }
    }
}

impl Transport for Web3Transport {
    type Out = Web3SendOut;

    fn prepare(&self, method: &str, params: Vec<Value>) -> (RequestId, Call) {
        match self {
            Web3Transport::Http(http) => http.prepare(method, params),
            Web3Transport::Websocket(websocket) => websocket.prepare(method, params),
            #[cfg(target_arch = "wasm32")]
            Web3Transport::Metamask(metamask) => metamask.prepare(method, params),
        }
    }

    fn send(&self, id: RequestId, request: Call) -> Self::Out {
        match self {
            Web3Transport::Http(http) => http.send(id, request),
            Web3Transport::Websocket(websocket) => websocket.send(id, request),
            #[cfg(target_arch = "wasm32")]
            Web3Transport::Metamask(metamask) => metamask.send(id, request),
        }
    }
}

impl From<http_transport::HttpTransport> for Web3Transport {
    fn from(http: http_transport::HttpTransport) -> Self { Web3Transport::Http(http) }
}

impl From<websocket_transport::WebsocketTransport> for Web3Transport {
    fn from(websocket: websocket_transport::WebsocketTransport) -> Self { Web3Transport::Websocket(websocket) }
}

#[cfg(target_arch = "wasm32")]
impl From<metamask_transport::MetamaskTransport> for Web3Transport {
    fn from(metamask: metamask_transport::MetamaskTransport) -> Self { Web3Transport::Metamask(metamask) }
}

/// eth_feeHistory support is missing even in the latest rust-web3
/// It's the custom namespace implementing it
#[derive(Debug, Clone)]
pub struct EthFeeHistoryNamespace<T> {
    transport: T,
}

impl<T: Transport> Namespace<T> for EthFeeHistoryNamespace<T> {
    fn new(transport: T) -> Self
    where
        Self: Sized,
    {
        Self { transport }
    }

    fn transport(&self) -> &T { &self.transport }
}

#[derive(Debug, Deserialize)]
pub struct FeeHistoryResult {
    #[serde(rename = "oldestBlock")]
    pub oldest_block: U256,
    #[serde(rename = "baseFeePerGas")]
    pub base_fee_per_gas: Vec<U256>,
}

impl<T: Transport> EthFeeHistoryNamespace<T> {
    pub fn eth_fee_history(
        &self,
        count: U256,
        block: BlockNumber,
        reward_percentiles: &[f64],
    ) -> CallFuture<FeeHistoryResult, T::Out> {
        let count = helpers::serialize(&count);
        let block = helpers::serialize(&block);
        let reward_percentiles = helpers::serialize(&reward_percentiles);
        let params = vec![count, block, reward_percentiles];
        CallFuture::new(self.transport.execute("eth_feeHistory", params))
    }
}
