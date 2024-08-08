use ethereum_types::U256;
use futures::future::BoxFuture;
use jsonrpc_core::Call;
#[cfg(target_arch = "wasm32")] use mm2_metamask::MetamaskResult;
use mm2_net::transport::{KomodefiProxyAuthValidation, ProxyAuthValidationGenerator};
use serde_json::Value as Json;
use serde_json::Value;
use std::sync::atomic::Ordering;
use web3::helpers::to_string;
use web3::{Error, RequestId, Transport};

use self::http_transport::QuicknodePayload;
use super::{EthCoin, KomodoDefiAuthMessages, Web3RpcError};
use crate::RpcTransportEventHandlerShared;

pub(crate) mod http_transport;
#[cfg(target_arch = "wasm32")] pub(crate) mod metamask_transport;
pub(crate) mod websocket_transport;

pub(crate) type Web3SendOut = BoxFuture<'static, Result<Json, Error>>;

/// The transport layer for interacting with a Web3 provider.
#[derive(Clone, Debug)]
pub enum Web3Transport {
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

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub fn new_http(node: http_transport::HttpTransportNode) -> Web3Transport {
        http_transport::HttpTransport::new(node).into()
    }

    pub fn proxy_auth_validation_generator_as_mut(&mut self) -> Option<&mut ProxyAuthValidationGenerator> {
        match self {
            Web3Transport::Http(http) => http.proxy_auth_validation_generator.as_mut(),
            Web3Transport::Websocket(websocket) => websocket.proxy_auth_validation_generator.as_mut(),
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

#[derive(Debug, Deserialize)]
pub struct FeeHistoryResult {
    #[serde(rename = "oldestBlock")]
    pub oldest_block: U256,
    #[serde(rename = "baseFeePerGas")]
    pub base_fee_per_gas: Vec<U256>,
    #[serde(rename = "gasUsedRatio")]
    pub gas_used_ratio: Vec<f64>,
    #[serde(rename = "reward")]
    pub priority_rewards: Option<Vec<Vec<U256>>>,
}

/// Generates a Quicknode payload JSON string by inserting a signed message into the request payload.
pub(super) fn handle_quicknode_payload(
    proxy_auth_validation_generator: &Option<ProxyAuthValidationGenerator>,
    request: &Call,
) -> Result<String, Web3RpcError> {
    let signed_message = generate_signed_message(proxy_auth_validation_generator)?;

    let auth_request = QuicknodePayload {
        request,
        signed_message,
    };

    Ok(to_string(&auth_request))
}

/// Generates a signed message JSON string if proxy authentication is enabled.
pub(crate) fn generate_auth_header(
    proxy_auth_validation_generator: &Option<ProxyAuthValidationGenerator>,
    gui_auth: bool,
) -> Result<Option<String>, Web3RpcError> {
    if !gui_auth {
        return Ok(None);
    }
    let signed_message = generate_signed_message(proxy_auth_validation_generator)?;
    Ok(Some(serde_json::to_string(&signed_message)?))
}

/// Generates a signed message using the provided ProxyAuthValidationGenerator
fn generate_signed_message(
    proxy_auth_validation_generator: &Option<ProxyAuthValidationGenerator>,
) -> Result<KomodefiProxyAuthValidation, Web3RpcError> {
    let generator = proxy_auth_validation_generator
        .clone()
        .ok_or_else(|| Web3RpcError::Internal("ProxyAuthValidationGenerator is not provided".to_string()))?;

    let signed_message = EthCoin::generate_proxy_auth_signed_validation(generator).map_err(|e| {
        Web3RpcError::Internal(format!(
            "KomodefiProxyAuthValidation signed message generation failed. Error: {:?}",
            e
        ))
    })?;
    Ok(signed_message)
}
