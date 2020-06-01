use futures01::{Future, Poll};
use common::executor::Timer;
use common::wio::slurp_reqʹ;
use futures::compat::{Compat};
use futures::future::{select, Either};
use gstuff::binprint;
use http::header::HeaderValue;
use jsonrpc_core::{Call, Response};
use serde_json::{Value as Json};
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use super::{RpcTransportEventHandler, RpcTransportEventHandlerShared};
use web3::{RequestId, Transport};
use web3::error::{Error, ErrorKind};
use web3::helpers::{build_request, to_result_from_output, to_string};

/// Parse bytes RPC response into `Result`.
/// Implementation copied from Web3 HTTP transport
fn single_response<T: Deref<Target = [u8]>>(response: T) -> Result<Json, Error> {
    let response = serde_json::from_slice(&*response).map_err(|e| Error::from(ErrorKind::InvalidResponse(format!("{}", e))))?;

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

    pub fn with_event_handlers(urls: Vec<String>, event_handlers: Vec<RpcTransportEventHandlerShared>) -> Result<Self, String> {
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

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0.poll()
    }
}

unsafe impl<T> Send for SendFuture<T> {}
unsafe impl<T> Sync for SendFuture<T> {}

impl Transport for Web3Transport {
    type Out = Box<dyn Future<Item=Json, Error=Error> + Send>;

    fn prepare(&self, method: &str, params: Vec<Json>) -> (RequestId, Call) {
        let id = self.id.fetch_add(1, Ordering::AcqRel);
        let request = build_request(id, method, params);

        (id, request)
    }

    #[cfg(not(feature="w-bindgen"))]
    fn send(&self, _id: RequestId, request: Call) -> Self::Out {
        Box::new(Compat::new(Box::pin(sendʹ(request, self.uris.clone(), self.event_handlers.clone()))))
    }

    #[cfg(feature="w-bindgen")]
    fn send(&self, _id: RequestId, request: Call) -> Self::Out {
        use js_sys;
        use js_sys::Promise;
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;
        use wasm_bindgen_futures::JsFuture;
        use web_sys::{Request, RequestInit, RequestMode, Response as JsResponse};

        let body = to_string(&request);
        self.event_handlers.on_outgoing_request(body.as_bytes());

        let mut opts = RequestInit::new();
        opts.method("POST");
        opts.mode(RequestMode::Cors);
        opts.body(Some(&JsValue::from_str(&body)));

        let request = Request::new_with_str_and_init(
            "http://195.201.0.6:8565",
            &opts,
        ).unwrap();

        request
            .headers()
            .set("Accept", "application/json")
            .unwrap();

        request
            .headers()
            .set("Content-Type", "application/json")
            .unwrap();

        let window = web_sys::window().unwrap();
        let request_promise = window.fetch_with_request(&request);
        use web_sys::console;

        let future = JsFuture::from(request_promise);
        let event_handlers = self.event_handlers.clone();
        let res = async move {
            let resp_value = future.await.unwrap();
            assert!(resp_value.is_instance_of::<JsResponse>());
            let resp: JsResponse = resp_value.dyn_into().unwrap();
            let json_value = JsFuture::from(resp.json().unwrap()).await.unwrap();
            let response: Json = json_value.into_serde().unwrap();

            let response = serde_json::to_vec(&response).unwrap();
            event_handlers.on_incoming_response(&response);

            single_response(response)
        };
        Box::new(SendFuture(Box::pin(res).compat()))
    }
}

async fn sendʹ(request: Call, uris: Vec<http::Uri>, event_handlers: Vec<RpcTransportEventHandlerShared>) -> Result<Json, Error> {
    let mut errors = Vec::new();
    for uri in uris.iter() {
        let request = to_string(&request);
        event_handlers.on_outgoing_request(request.as_bytes());

        let mut req = http::Request::new(request.clone().into_bytes());
        *req.method_mut() = http::Method::POST;
        *req.uri_mut() = uri.clone();
        req.headers_mut().insert(http::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let timeout = Timer::sleep(60.);
        let req = Box::pin(slurp_reqʹ(req));
        let rc = select(req, timeout).await;
        let res = match rc {
            Either::Left((r, _t)) => r,
            Either::Right((_t, _r)) => {errors.push(ERRL!("timeout")); continue}
        };

        let (status, _headers, body) = match res {
            Ok(r) => r,
            Err(err) => {
                errors.push(err);
                continue
            }
        };

        event_handlers.on_incoming_response(&body);

        if !status.is_success() {
            errors.push(ERRL!("!200: {}, {}", status, binprint(&body, b'.')));
            continue
        }

        return single_response(body)
    }
    Err(ErrorKind::Transport(fomat!(
        "request " [request] " failed: "
        for err in errors {(err)} sep {"; "}
    )).into())
}
