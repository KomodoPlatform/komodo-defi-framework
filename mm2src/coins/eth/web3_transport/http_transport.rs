use crate::eth::{web3_transport::Web3SendOut, EthCoin, GuiAuthMessages, RpcTransportEventHandler,
                 RpcTransportEventHandlerShared, Web3RpcError};
use common::APPLICATION_JSON;
#[cfg(not(target_arch = "wasm32"))] use futures::FutureExt;
use futures::TryFutureExt;
use http::header::CONTENT_TYPE;
use jsonrpc_core::{Call, Response};
use mm2_net::transport::{GuiAuthValidation, GuiAuthValidationGenerator};
use serde_json::Value as Json;
#[cfg(not(target_arch = "wasm32"))] use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use web3::error::{Error, ErrorKind};
use web3::helpers::{build_request, to_result_from_output, to_string};
use web3::{RequestId, Transport};

#[derive(Serialize, Clone)]
pub struct AuthPayload<'a> {
    #[serde(flatten)]
    pub request: &'a Call,
    pub signed_message: GuiAuthValidation,
}

/// Parse bytes RPC response into `Result`.
/// Implementation copied from Web3 HTTP transport
#[cfg(not(target_arch = "wasm32"))]
fn single_response<T: Deref<Target = [u8]>>(response: T, rpc_url: &str) -> Result<Json, Error> {
    let response = serde_json::from_slice(&response)
        .map_err(|e| Error::from(ErrorKind::InvalidResponse(format!("{}: {}", rpc_url, e))))?;

    match response {
        Response::Single(output) => to_result_from_output(output),
        _ => Err(ErrorKind::InvalidResponse("Expected single, got batch.".into()).into()),
    }
}

#[derive(Clone, Debug)]
pub struct HttpTransport {
    id: Arc<AtomicUsize>,
    nodes: Vec<HttpTransportNode>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
    pub(crate) gui_auth_validation_generator: Option<GuiAuthValidationGenerator>,
}

#[derive(Clone, Debug)]
pub struct HttpTransportNode {
    pub(crate) uri: http::Uri,
    pub(crate) gui_auth: bool,
}

impl HttpTransport {
    #[cfg(test)]
    #[inline]
    pub fn new(nodes: Vec<HttpTransportNode>) -> Self {
        HttpTransport {
            id: Arc::new(AtomicUsize::new(0)),
            nodes,
            event_handlers: Default::default(),
            gui_auth_validation_generator: None,
        }
    }

    #[inline]
    pub fn with_event_handlers(
        nodes: Vec<HttpTransportNode>,
        event_handlers: Vec<RpcTransportEventHandlerShared>,
    ) -> Self {
        HttpTransport {
            id: Arc::new(AtomicUsize::new(0)),
            nodes,
            event_handlers,
            gui_auth_validation_generator: None,
        }
    }

    #[allow(dead_code)]
    pub fn single_node(url: &'static str, gui_auth: bool) -> Self {
        let nodes = vec![HttpTransportNode {
            uri: url.parse().unwrap(),
            gui_auth,
        }];

        HttpTransport {
            id: Arc::new(AtomicUsize::new(0)),
            nodes,
            event_handlers: Default::default(),
            gui_auth_validation_generator: None,
        }
    }
}

impl Transport for HttpTransport {
    type Out = Web3SendOut;

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
                self.gui_auth_validation_generator.clone(),
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
            self.gui_auth_validation_generator.clone(),
        );
        Box::new(Box::pin(fut).compat())
    }
}

/// Generates a signed message and inserts it into request
/// payload if gui_auth is activated. Returns false on errors.
fn handle_gui_auth_payload_if_activated(
    gui_auth_validation_generator: &Option<GuiAuthValidationGenerator>,
    node: &HttpTransportNode,
    request: &Call,
) -> Result<Option<String>, Web3RpcError> {
    if !node.gui_auth {
        return Ok(None);
    }

    let generator = match gui_auth_validation_generator.clone() {
        Some(gen) => gen,
        None => {
            return Err(Web3RpcError::Internal(format!(
                "GuiAuthValidationGenerator is not provided for {:?} node",
                node
            )));
        },
    };

    let signed_message = match EthCoin::generate_gui_auth_signed_validation(generator) {
        Ok(t) => t,
        Err(e) => {
            return Err(Web3RpcError::Internal(format!(
                "GuiAuth signed message generation failed for {:?} node, error: {:?}",
                node, e
            )));
        },
    };

    let auth_request = AuthPayload {
        request,
        signed_message,
    };

    Ok(Some(to_string(&auth_request)))
}

#[cfg(not(target_arch = "wasm32"))]
async fn send_request(
    request: Call,
    nodes: Vec<HttpTransportNode>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
    gui_auth_validation_generator: Option<GuiAuthValidationGenerator>,
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
        let serialized_request =
            match handle_gui_auth_payload_if_activated(&gui_auth_validation_generator, node, &request) {
                Ok(Some(r)) => r,
                Ok(None) => serialized_request.clone(),
                Err(e) => {
                    errors.push(e);
                    continue;
                },
            };

        event_handlers.on_outgoing_request(serialized_request.as_bytes());

        let mut req = http::Request::new(serialized_request.clone().into_bytes());
        *req.method_mut() = http::Method::POST;
        *req.uri_mut() = node.uri.clone();
        req.headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static(APPLICATION_JSON));
        let timeout = Timer::sleep(REQUEST_TIMEOUT_S);
        let req = Box::pin(slurp_req(req));
        let rc = select(req, timeout).await;
        let res = match rc {
            Either::Left((r, _t)) => r,
            Either::Right((_t, _r)) => {
                let error = format!(
                    "Error requesting '{}': {}s timeout expired",
                    node.uri, REQUEST_TIMEOUT_S
                );
                warn!("{}", error);
                errors.push(Web3RpcError::Transport(error));
                continue;
            },
        };

        let (status, _headers, body) = match res {
            Ok(r) => r,
            Err(err) => {
                errors.push(Web3RpcError::Transport(err.to_string()));
                continue;
            },
        };

        event_handlers.on_incoming_response(&body);

        if !status.is_success() {
            errors.push(Web3RpcError::Transport(format!(
                "Server '{:?}' response !200: {}, {}",
                node,
                status,
                binprint(&body, b'.')
            )));
            continue;
        }

        return single_response(body, &node.uri.to_string());
    }

    Err(request_failed_error(&request, &errors))
}

#[cfg(target_arch = "wasm32")]
async fn send_request(
    request: Call,
    nodes: Vec<HttpTransportNode>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
    gui_auth_validation_generator: Option<GuiAuthValidationGenerator>,
) -> Result<Json, Error> {
    let serialized_request = to_string(&request);

    let mut transport_errors = Vec::new();
    for node in nodes.iter() {
        let serialized_request =
            match handle_gui_auth_payload_if_activated(&gui_auth_validation_generator, node, &request) {
                Ok(Some(r)) => r,
                Ok(None) => serialized_request.clone(),
                Err(e) => {
                    transport_errors.push(e);
                    continue;
                },
            };

        match send_request_once(serialized_request.clone(), &node.uri, &event_handlers).await {
            Ok(response_json) => return Ok(response_json),
            Err(Error(ErrorKind::Transport(e), _)) => {
                transport_errors.push(Web3RpcError::Transport(e));
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
    use http::header::ACCEPT;
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
        .header(ACCEPT.as_str(), APPLICATION_JSON)
        .header(CONTENT_TYPE.as_str(), APPLICATION_JSON)
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

fn request_failed_error(request: &Call, errors: &[Web3RpcError]) -> Error {
    let errors: String = errors.iter().map(|e| format!("{:?}; ", e)).collect();
    let error = format!("request {:?} failed: {}", request, errors);
    Error::from(ErrorKind::Transport(error))
}
