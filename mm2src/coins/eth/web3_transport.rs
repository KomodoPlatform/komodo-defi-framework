use super::{RpcTransportEventHandler, RpcTransportEventHandlerShared};
#[cfg(not(target_arch = "wasm32"))] use futures::FutureExt;
use futures::TryFutureExt;
use futures01::{Future, Poll};
use jsonrpc_core::{Call, Response};
use mm2_net::transport::GuiAuthValidation;
use serde_json::Value as Json;
#[cfg(not(target_arch = "wasm32"))] use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use web3::api::Namespace;
use web3::error::{Error, ErrorKind};
use web3::helpers::{self, build_request, to_result_from_output, to_string, CallFuture};
use web3::types::{BlockNumber, U256};
use web3::{RequestId, Transport};

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

/// Parse bytes RPC response into `Result`.
/// Implementation copied from Web3 HTTP transport
#[cfg(not(target_arch = "wasm32"))]
fn single_response<T: Deref<Target = [u8]>>(response: T, rpc_url: &str) -> Result<Json, Error> {
    let response = serde_json::from_slice(&*response)
        .map_err(|e| Error::from(ErrorKind::InvalidResponse(format!("{}: {}", rpc_url, e))))?;

    match response {
        Response::Single(output) => to_result_from_output(output),
        _ => Err(ErrorKind::InvalidResponse("Expected single, got batch.".into()).into()),
    }
}

#[derive(Clone, Debug)]
pub struct Web3Transport {
    id: Arc<AtomicUsize>,
    nodes: Vec<Web3TransportNode>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
    pub(crate) gui_auth_validation: Option<GuiAuthValidation>,
}

#[derive(Clone, Debug)]
pub struct Web3TransportNode {
    uri: http::Uri,
    gui_auth: bool,
}

impl Web3Transport {
    #[allow(dead_code)]
    pub fn new(urls: Vec<String>, gui_auth: bool) -> Result<Self, String> {
        let mut nodes = vec![];
        for url in urls.iter() {
            let uri = try_s!(url.parse());
            nodes.push(Web3TransportNode { uri, gui_auth });
        }
        Ok(Web3Transport {
            id: Arc::new(AtomicUsize::new(0)),
            nodes,
            event_handlers: Default::default(),
            gui_auth_validation: None,
        })
    }

    pub fn with_event_handlers(
        urls: Vec<String>,
        event_handlers: Vec<RpcTransportEventHandlerShared>,
        gui_auth: bool,
    ) -> Result<Self, String> {
        let mut nodes = vec![];
        for url in urls.iter() {
            let uri = try_s!(url.parse());
            nodes.push(Web3TransportNode { uri, gui_auth });
        }
        Ok(Web3Transport {
            id: Arc::new(AtomicUsize::new(0)),
            nodes,
            event_handlers,
            gui_auth_validation: None,
        })
    }
}

struct SendFuture<T>(T);

impl<T: Future> Future for SendFuture<T> {
    type Item = T::Item;

    type Error = T::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> { self.0.poll() }
}

unsafe impl<T> Send for SendFuture<T> where T: Send {}
unsafe impl<T> Sync for SendFuture<T> {}

impl Transport for Web3Transport {
    type Out = Box<dyn Future<Item = Json, Error = Error> + Send>;

    fn prepare(&self, method: &str, params: Vec<Json>) -> (RequestId, Call) {
        let id = self.id.fetch_add(1, Ordering::AcqRel);
        let request = build_request(id, method, params);

        (id, request)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn send(&self, _id: RequestId, request: Call) -> Self::Out {
        Box::new(
            send_request(
                request,
                self.nodes.clone(),
                self.event_handlers.clone(),
                self.gui_auth_validation.clone(),
            )
            .boxed()
            .compat(),
        )
    }

    #[cfg(target_arch = "wasm32")]
    fn send(&self, _id: RequestId, request: Call) -> Self::Out {
        let fut = send_request(
            request,
            self.nodes.clone(),
            self.event_handlers.clone(),
            self.gui_auth_validation.clone(),
        );
        Box::new(SendFuture(Box::pin(fut).compat()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn send_request(
    request: Call,
    nodes: Vec<Web3TransportNode>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
    gui_auth_validation: Option<GuiAuthValidation>,
) -> Result<Json, Error> {
    use common::executor::Timer;
    use common::log::warn;
    use futures::future::{select, Either};
    use gstuff::binprint;
    use http::header::HeaderValue;
    use mm2_net::transport::slurp_req;

    const REQUEST_TIMEOUT_S: f64 = 60.;

    let mut errors = Vec::new();

    let serialized_request = to_string(&request);

    for node in nodes.iter() {
        let mut serialized_request = serialized_request.clone();
        if node.gui_auth {
            if let Some(gui_auth_validation) = gui_auth_validation.clone() {
                let mut json_payload = serde_json::to_value(&serialized_request)?;
                json_payload["signed_message"] = json!(gui_auth_validation);
                common::drop_mutability!(json_payload);
                serialized_request = json_payload.to_string();
            } else {
                errors.push(ERRL!("GuiAuthValidation is not provided for {:?} node", node,));
                continue;
            }
        }
        common::drop_mutability!(serialized_request);

        event_handlers.on_outgoing_request(serialized_request.as_bytes());

        let mut req = http::Request::new(serialized_request.clone().into_bytes());
        *req.method_mut() = http::Method::POST;
        *req.uri_mut() = node.uri.clone();
        req.headers_mut()
            .insert(http::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let timeout = Timer::sleep(REQUEST_TIMEOUT_S);
        let req = Box::pin(slurp_req(req));
        let rc = select(req, timeout).await;
        let res = match rc {
            Either::Left((r, _t)) => r,
            Either::Right((_t, _r)) => {
                let error = ERRL!(
                    "Error requesting '{}': {}s timeout expired",
                    node.uri,
                    REQUEST_TIMEOUT_S
                );
                warn!("{}", error);
                errors.push(error);
                continue;
            },
        };

        let (status, _headers, body) = match res {
            Ok(r) => r,
            Err(err) => {
                errors.push(err.to_string());
                continue;
            },
        };

        event_handlers.on_incoming_response(&body);

        if !status.is_success() {
            errors.push(ERRL!(
                "Server '{:?}' response !200: {}, {}",
                node,
                status,
                binprint(&body, b'.')
            ));
            continue;
        }

        return single_response(body, &node.uri.to_string());
    }

    Err(request_failed_error(&request, &errors))
}

#[cfg(target_arch = "wasm32")]
async fn send_request(
    request: Call,
    nodes: Vec<Web3TransportNode>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
    gui_auth_validation: Option<GuiAuthValidation>,
) -> Result<Json, Error> {
    let serialized_request = to_string(&request);

    let mut transport_errors = Vec::new();
    for node in nodes {
        let mut serialized_request = serialized_request.clone();
        if node.gui_auth {
            if let Some(gui_auth_validation) = gui_auth_validation.clone() {
                let mut json_payload = serde_json::to_value(&serialized_request)?;
                json_payload["signed_message"] = json!(gui_auth_validation);
                common::drop_mutability!(json_payload);
                serialized_request = json_payload.to_string();
            } else {
                transport_errors.push(ERRL!("GuiAuthValidation is not provided for {:?} node", node,));
                continue;
            }
        }
        common::drop_mutability!(serialized_request);

        match send_request_once(serialized_request.clone(), &node.uri, &event_handlers).await {
            Ok(response_json) => return Ok(response_json),
            Err(Error(ErrorKind::Transport(e), _)) => {
                transport_errors.push(e.to_string());
            },
            Err(e) => return Err(e),
        }
    }

    Err(request_failed_error(&request, &transport_errors))
}

#[cfg(target_arch = "wasm32")]
async fn send_request_once(
    request_payload: String,
    uri: &http::Uri,
    event_handlers: &Vec<RpcTransportEventHandlerShared>,
) -> Result<Json, Error> {
    use mm2_net::wasm_http::FetchRequest;

    macro_rules! try_or {
        ($exp:expr, $errkind:ident) => {
            match $exp {
                Ok(x) => x,
                Err(e) => return Err(Error::from(ErrorKind::$errkind(ERRL!("{:?}", e)))),
            }
        };
    }

    // account for outgoing traffic
    event_handlers.on_outgoing_request(request_payload.as_bytes());

    let result = FetchRequest::post(&uri.to_string())
        .cors()
        .body_utf8(request_payload)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .request_str()
        .await;
    let (status_code, response_str) = try_or!(result, Transport);
    if !status_code.is_success() {
        return Err(Error::from(ErrorKind::Transport(ERRL!(
            "!200: {}, {}",
            status_code,
            response_str
        ))));
    }

    // account for incoming traffic
    event_handlers.on_incoming_response(response_str.as_bytes());

    let response: Response = try_or!(serde_json::from_str(&response_str), InvalidResponse);
    match response {
        Response::Single(output) => to_result_from_output(output),
        Response::Batch(_) => Err(Error::from(ErrorKind::InvalidResponse(
            "Expected single, got batch.".to_owned(),
        ))),
    }
}

fn request_failed_error(request: &Call, errors: &[String]) -> Error {
    let errors = errors.join("; ");
    let error = format!("request {:?} failed: {}", request, errors);
    Error::from(ErrorKind::Transport(error))
}
