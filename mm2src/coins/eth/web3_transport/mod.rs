use ethereum_types::U256;
use futures::future::BoxFuture;
use jsonrpc_core::Call;
#[cfg(target_arch = "wasm32")] use mm2_metamask::MetamaskResult;
use mm2_net::transport::GuiAuthValidationGenerator;
use serde_json::Value as Json;
use serde_json::Value;
use std::sync::atomic::Ordering;
use web3::api::Namespace;
use web3::helpers::{self, to_string, CallFuture};
use web3::types::BlockNumber;
use web3::{Error, RequestId, Transport};

use self::http_transport::AuthPayload;
use super::{EthCoin, GuiAuthMessages, Web3RpcError};
use crate::RpcTransportEventHandlerShared;

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
    pub fn new_http_with_event_handlers(
        node: http_transport::HttpTransportNode,
        event_handlers: Vec<RpcTransportEventHandlerShared>,
    ) -> Web3Transport {
        http_transport::HttpTransport::with_event_handlers(node, event_handlers).into()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new_metamask_with_event_handlers(
        eth_config: metamask_transport::MetamaskEthConfig,
        event_handlers: Vec<RpcTransportEventHandlerShared>,
    ) -> MetamaskResult<Web3Transport> {
        Ok(metamask_transport::MetamaskTransport::detect(eth_config, event_handlers)?.into())
    }

    pub fn is_last_request_failed(&self) -> bool {
        match self {
            Web3Transport::Http(http) => http.last_request_failed.load(Ordering::SeqCst),
            Web3Transport::Websocket(websocket) => websocket.last_request_failed.load(Ordering::SeqCst),
            #[cfg(target_arch = "wasm32")]
            Web3Transport::Metamask(metamask) => metamask.last_request_failed.load(Ordering::SeqCst),
        }
    }

    fn set_last_request_failed(&self, val: bool) {
        match self {
            Web3Transport::Http(http) => http.last_request_failed.store(val, Ordering::SeqCst),
            Web3Transport::Websocket(websocket) => websocket.last_request_failed.store(val, Ordering::SeqCst),
            #[cfg(target_arch = "wasm32")]
            Web3Transport::Metamask(metamask) => metamask.last_request_failed.store(val, Ordering::SeqCst),
        }
    }

    #[cfg(any(test, target_arch = "wasm32"))]
    pub fn new_http(node: http_transport::HttpTransportNode) -> Web3Transport {
        http_transport::HttpTransport::new(node).into()
    }

    pub fn gui_auth_validation_generator_as_mut(&mut self) -> Option<&mut GuiAuthValidationGenerator> {
        match self {
            Web3Transport::Http(http) => http.gui_auth_validation_generator.as_mut(),
            Web3Transport::Websocket(websocket) => websocket.gui_auth_validation_generator.as_mut(),
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
        let selfi = self.clone();
        let fut = async move {
            let result = match &selfi {
                Web3Transport::Http(http) => http.send(id, request),
                Web3Transport::Websocket(websocket) => websocket.send(id, request),
                #[cfg(target_arch = "wasm32")]
                Web3Transport::Metamask(metamask) => metamask.send(id, request),
            }
            .await;

            selfi.set_last_request_failed(result.is_err());

            result
        };

        Box::pin(fut)
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

/// Generates a signed message and inserts it into the request payload.
pub(super) fn handle_gui_auth_payload(
    gui_auth_validation_generator: &Option<GuiAuthValidationGenerator>,
    request: &Call,
) -> Result<String, Web3RpcError> {
    let generator = match gui_auth_validation_generator.clone() {
        Some(gen) => gen,
        None => {
            return Err(Web3RpcError::Internal(
                "GuiAuthValidationGenerator is not provided for".to_string(),
            ));
        },
    };

    let signed_message = match EthCoin::generate_gui_auth_signed_validation(generator) {
        Ok(t) => t,
        Err(e) => {
            return Err(Web3RpcError::Internal(format!(
                "GuiAuth signed message generation failed. Error: {:?}",
                e
            )));
        },
    };

    let auth_request = AuthPayload {
        request,
        signed_message,
    };

    Ok(to_string(&auth_request))
}
