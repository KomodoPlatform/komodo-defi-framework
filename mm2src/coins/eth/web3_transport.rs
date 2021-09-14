use super::{RpcTransportEventHandler, RpcTransportEventHandlerShared};
#[cfg(not(target_arch = "wasm32"))] use futures::FutureExt;
use futures::TryFutureExt;
use futures01::{Future, Poll};
use jsonrpc_core::{Call, Response};
use serde_json::Value as Json;
#[cfg(not(target_arch = "wasm32"))] use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use web3::error::{Error, ErrorKind};
use web3::helpers::{build_request, to_result_from_output, to_string};
use web3::{RequestId, Transport};

/// Parse bytes RPC response into `Result`.
/// Implementation copied from Web3 HTTP transport
#[cfg(not(target_arch = "wasm32"))]
fn single_response<T: Deref<Target = [u8]>>(response: T) -> Result<Json, Error> {
    let response =
        serde_json::from_slice(&*response).map_err(|e| Error::from(ErrorKind::InvalidResponse(format!("{}", e))))?;

    match response {
        Response::Single(output) => to_result_from_output(output),
        _ => Err(ErrorKind::InvalidResponse("Expected single, got batch.".into()).into()),
    }
}

#[derive(Clone, Debug)]
pub struct Web3Transport {
    id: Arc<AtomicUsize>,
    uris: Vec<http::Uri>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
}

impl Web3Transport {
    #[allow(dead_code)]
    pub fn new(urls: Vec<String>) -> Result<Self, String> {
        let mut uris = vec![];
        for url in urls.iter() {
            uris.push(try_s!(url.parse()));
        }
        Ok(Web3Transport {
            id: Arc::new(AtomicUsize::new(0)),
            uris,
            event_handlers: Default::default(),
        })
    }

    pub fn with_event_handlers(
        urls: Vec<String>,
        event_handlers: Vec<RpcTransportEventHandlerShared>,
    ) -> Result<Self, String> {
        let mut uris = vec![];
        for url in urls.iter() {
            uris.push(try_s!(url.parse()));
        }
        Ok(Web3Transport {
            id: Arc::new(AtomicUsize::new(0)),
            uris,
            event_handlers,
        })
    }
}

struct SendFuture<T>(T);

impl<T: Future> Future for SendFuture<T> {
    type Item = T::Item;

    type Error = T::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> { self.0.poll() }
}

unsafe impl<T> Send for SendFuture<T> {}
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
            send_request(request, self.uris.clone(), self.event_handlers.clone())
                .boxed()
                .compat(),
        )
    }

    #[cfg(target_arch = "wasm32")]
    fn send(&self, _id: RequestId, request: Call) -> Self::Out {
        let fut = send_request(request, self.uris.clone(), self.event_handlers.clone());
        Box::new(SendFuture(Box::pin(fut).compat()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn send_request(
    request: Call,
    uris: Vec<http::Uri>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> Result<Json, Error> {
    use common::executor::Timer;
    use common::wio::slurp_reqʹ;
    use futures::future::{select, Either};
    use gstuff::binprint;
    use http::header::HeaderValue;

    let mut errors = Vec::new();
    for uri in uris.iter() {
        let request = to_string(&request);
        event_handlers.on_outgoing_request(request.as_bytes());

        let mut req = http::Request::new(request.clone().into_bytes());
        *req.method_mut() = http::Method::POST;
        *req.uri_mut() = uri.clone();
        req.headers_mut()
            .insert(http::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let timeout = Timer::sleep(60.);
        let req = Box::pin(slurp_reqʹ(req));
        let rc = select(req, timeout).await;
        let res = match rc {
            Either::Left((r, _t)) => r,
            Either::Right((_t, _r)) => {
                errors.push(ERRL!("timeout"));
                continue;
            },
        };

        let (status, _headers, body) = match res {
            Ok(r) => r,
            Err(err) => {
                errors.push(err);
                continue;
            },
        };

        event_handlers.on_incoming_response(&body);

        if !status.is_success() {
            errors.push(ERRL!("!200: {}, {}", status, binprint(&body, b'.')));
            continue;
        }

        return single_response(body);
    }
    Err(ErrorKind::Transport(fomat!(
        "request " [request] " failed: "
        for err in errors {(err)} sep {"; "}
    ))
    .into())
}

#[cfg(target_arch = "wasm32")]
async fn send_request(
    request: Call,
    uris: Vec<http::Uri>,
    event_handlers: Vec<RpcTransportEventHandlerShared>,
) -> Result<Json, Error> {
    let request_payload = to_string(&request);

    let mut transport_errors = Vec::new();
    for uri in uris {
        match send_request_once(request_payload.clone(), &uri, &event_handlers).await {
            Ok(response_json) => return Ok(response_json),
            Err(Error(ErrorKind::Transport(e), _)) => {
                transport_errors.push(e.to_string());
            },
            Err(e) => return Err(e),
        }
    }

    Err(ErrorKind::Transport(fomat!(
        "request " [request] " failed: "
        for err in transport_errors {(err)} sep {"; "}
    ))
    .into())
}

#[cfg(target_arch = "wasm32")]
async fn send_request_once(
    request_payload: String,
    uri: &http::Uri,
    event_handlers: &Vec<RpcTransportEventHandlerShared>,
) -> Result<Json, Error> {
    use common::wasm_http::FetchRequest;

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
