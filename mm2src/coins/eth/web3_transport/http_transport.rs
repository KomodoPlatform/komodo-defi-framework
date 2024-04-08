use crate::eth::web3_transport::handle_gui_auth_payload;
use crate::eth::{web3_transport::Web3SendOut, RpcTransportEventHandler, RpcTransportEventHandlerShared, Web3RpcError};
use common::APPLICATION_JSON;
use http::header::CONTENT_TYPE;
use jsonrpc_core::{Call, Response};
use mm2_net::transport::{GuiAuthValidation, GuiAuthValidationGenerator};
use serde_json::Value as Json;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use web3::error::{Error, TransportError};
use web3::helpers::{build_request, to_result_from_output, to_string};
use web3::{RequestId, Transport};

#[derive(Clone, Serialize)]
pub struct AuthPayload<'a> {
    #[serde(flatten)]
    pub request: &'a Call,
    pub signed_message: GuiAuthValidation,
}

/// Deserialize bytes RPC response into `Result`.
/// Implementation copied from Web3 HTTP transport
pub(crate) fn de_rpc_response<T>(response: T, rpc_url: &str) -> Result<Json, Error>
where
    T: Deref<Target = [u8]> + std::fmt::Debug,
{
    let response = serde_json::from_slice(&response).map_err(|e| {
        Error::InvalidResponse(format!(
            "url: {}, Error deserializing response: {}, raw response: {}",
            rpc_url,
            e,
            String::from_utf8_lossy(&response)
        ))
    })?;

    match response {
        Response::Single(output) => to_result_from_output(output),
        _ => Err(Error::InvalidResponse("Expected single, got batch.".into())),
    }
}

#[derive(Clone, Debug)]
pub struct HttpTransport {
    id: Arc<AtomicUsize>,
    pub(crate) last_request_failed: Arc<AtomicBool>,
    node: HttpTransportNode,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
    pub(crate) gui_auth_validation_generator: Option<GuiAuthValidationGenerator>,
}

#[derive(Clone, Debug)]
pub struct HttpTransportNode {
    pub(crate) uri: http::Uri,
    pub(crate) gui_auth: bool,
}

impl HttpTransport {
    #[inline]
    #[cfg(any(test, target_arch = "wasm32"))]
    pub fn new(node: HttpTransportNode) -> Self {
        HttpTransport {
            id: Arc::new(AtomicUsize::new(0)),
            node,
            event_handlers: Default::default(),
            gui_auth_validation_generator: None,
            last_request_failed: Arc::new(AtomicBool::new(false)),
        }
    }

    #[inline]
    pub fn with_event_handlers(node: HttpTransportNode, event_handlers: Vec<RpcTransportEventHandlerShared>) -> Self {
        HttpTransport {
            id: Arc::new(AtomicUsize::new(0)),
            node,
            event_handlers,
            gui_auth_validation_generator: None,
            last_request_failed: Arc::new(AtomicBool::new(false)),
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
    fn send(&self, _id: RequestId, request: Call) -> Self::Out { Box::pin(send_request(request, self.clone())) }

    #[cfg(target_arch = "wasm32")]
    fn send(&self, _id: RequestId, request: Call) -> Self::Out { Box::pin(send_request(request, self.clone())) }
}

#[cfg(not(target_arch = "wasm32"))]
async fn send_request(request: Call, transport: HttpTransport) -> Result<Json, Error> {
    use common::executor::Timer;
    use common::log::warn;
    use futures::future::{select, Either};
    use gstuff::binprint;
    use http::header::HeaderValue;
    use mm2_net::transport::slurp_req;

    const REQUEST_TIMEOUT_S: f64 = 20.;

    let mut serialized_request = to_string(&request);

    if transport.node.gui_auth {
        match handle_gui_auth_payload(&transport.gui_auth_validation_generator, &request) {
            Ok(r) => serialized_request = r,
            Err(e) => {
                return Err(request_failed_error(request, e));
            },
        };
    }

    transport
        .event_handlers
        .on_outgoing_request(serialized_request.as_bytes());

    let mut req = http::Request::new(serialized_request.into_bytes());
    *req.method_mut() = http::Method::POST;
    *req.uri_mut() = transport.node.uri.clone();
    req.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(APPLICATION_JSON));
    let timeout = Timer::sleep(REQUEST_TIMEOUT_S);
    let req = Box::pin(slurp_req(req));
    let rc = select(req, timeout).await;
    let res = match rc {
        Either::Left((r, _t)) => r,
        Either::Right((_t, _r)) => {
            let (method, id) = match &request {
                Call::MethodCall(m) => (m.method.clone(), m.id.clone()),
                Call::Notification(n) => (n.method.clone(), jsonrpc_core::Id::Null),
                Call::Invalid { id } => ("Invalid call".to_string(), id.clone()),
            };
            let error = format!(
                "Error requesting '{}': {}s timeout expired, method: '{}', id: {:?}",
                transport.node.uri, REQUEST_TIMEOUT_S, method, id
            );
            warn!("{}", error);
            return Err(request_failed_error(request, Web3RpcError::Transport(error)));
        },
    };

    let (status, _headers, body) = match res {
        Ok(r) => r,
        Err(err) => {
            return Err(request_failed_error(request, Web3RpcError::Transport(err.to_string())));
        },
    };

    transport.event_handlers.on_incoming_response(&body);

    if !status.is_success() {
        return Err(request_failed_error(
            request,
            Web3RpcError::Transport(format!(
                "Server: '{}', response !200: {}, {}",
                transport.node.uri,
                status,
                binprint(&body, b'.')
            )),
        ));
    }

    let res = match de_rpc_response(body, &transport.node.uri.to_string()) {
        Ok(r) => r,
        Err(err) => {
            return Err(request_failed_error(
                request,
                Web3RpcError::InvalidResponse(format!("Server: '{}', error: {}", transport.node.uri, err)),
            ));
        },
    };

    Ok(res)
}

#[cfg(target_arch = "wasm32")]
async fn send_request(request: Call, transport: HttpTransport) -> Result<Json, Error> {
    let mut serialized_request = to_string(&request);

    if transport.node.gui_auth {
        match handle_gui_auth_payload(&transport.gui_auth_validation_generator, &request) {
            Ok(r) => serialized_request = r,
            Err(e) => {
                return Err(request_failed_error(
                    request,
                    Web3RpcError::Transport(format!("Server: '{}', error: {}", transport.node.uri, e)),
                ));
            },
        };
    }

    match send_request_once(serialized_request, &transport.node.uri, &transport.event_handlers).await {
        Ok(response_json) => Ok(response_json),
        Err(Error::Transport(e)) => Err(request_failed_error(
            request,
            Web3RpcError::Transport(format!("Server: '{}', error: {}", transport.node.uri, e)),
        )),
        Err(e) => Err(request_failed_error(
            request,
            Web3RpcError::InvalidResponse(format!("Server: '{}', error: {}", transport.node.uri, e)),
        )),
    }
}

#[cfg(target_arch = "wasm32")]
async fn send_request_once(
    request_payload: String,
    uri: &http::Uri,
    event_handlers: &Vec<RpcTransportEventHandlerShared>,
) -> Result<Json, Error> {
    use http::header::ACCEPT;
    use mm2_net::wasm::http::FetchRequest;

    // account for outgoing traffic
    event_handlers.on_outgoing_request(request_payload.as_bytes());

    let (status_code, response_str) = FetchRequest::post(&uri.to_string())
        .cors()
        .body_utf8(request_payload)
        .header(ACCEPT.as_str(), APPLICATION_JSON)
        .header(CONTENT_TYPE.as_str(), APPLICATION_JSON)
        .request_str()
        .await
        .map_err(|e| Error::Transport(TransportError::Message(ERRL!("{:?}", e))))?;

    if !status_code.is_success() {
        let err = ERRL!("!200: {}, {}", status_code, response_str);
        return Err(Error::Transport(TransportError::Message(err)));
    }

    // account for incoming traffic
    event_handlers.on_incoming_response(response_str.as_bytes());

    let response: Response = serde_json::from_str(&response_str).map_err(|e| {
        Error::InvalidResponse(format!(
            "Error deserializing response: {}, raw response: {:?}",
            e, response_str
        ))
    })?;
    match response {
        Response::Single(output) => to_result_from_output(output),
        Response::Batch(_) => Err(Error::InvalidResponse("Expected single, got batch.".to_owned())),
    }
}

fn request_failed_error(request: Call, error: Web3RpcError) -> Error {
    let error = format!("request {:?} failed: {}", request, error);
    Error::Transport(TransportError::Message(error))
}
