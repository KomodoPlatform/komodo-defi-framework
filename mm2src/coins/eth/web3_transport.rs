use common::executor::Timer;
use common::wio::slurp_reqʹ;
use futures01::Future;
use futures::compat::Compat;
use futures::future::{select, Either};
use gstuff::binprint;
use http::header::HeaderValue;
use jsonrpc_core::{Call, Response};
use serde_json::{Value as Json};
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
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

#[derive(Debug, Clone)]
pub struct Web3Transport {
    id: Arc<AtomicUsize>,
    uris: Vec<http::Uri>,
}

impl Web3Transport {
    pub fn new(urls: Vec<String>) -> Result<Self, String> {
        let mut uris = vec![];
        for url in urls.iter() {
            uris.push(try_s!(url.parse()));
        }
        Ok(Web3Transport {
            id: Arc::new(AtomicUsize::new(0)),
            uris,
        })
    }
}

impl Transport for Web3Transport {
    type Out = Box<dyn Future<Item=Json, Error=Error> + Send>;

    fn prepare(&self, method: &str, params: Vec<Json>) -> (RequestId, Call) {
        let id = self.id.fetch_add(1, Ordering::AcqRel);
        let request = build_request(id, method, params);

        (id, request)
    }

    fn send(&self, _id: RequestId, request: Call) -> Self::Out {
        Box::new(Compat::new(Box::pin(sendʹ(request, self.uris.clone()))))
    }
}

async fn sendʹ(request: Call, uris: Vec<http::Uri>) -> Result<Json, Error> {
    let mut errors = Vec::new();
    for uri in uris.iter() {
        let request = to_string(&request);
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
        let (status, _headers, body) = match res {Ok(r) => r, Err(err) => {errors.push(err); continue}};
        if !status.is_success() {errors.push(ERRL!("!200: {}, {}", status, binprint(&body, b'.'))); continue}
        return single_response(body)
    }
    Err(ErrorKind::Transport(fomat!(
        "request " [request] " failed: "
        for err in errors {(err)} sep {"; "}
    )).into())
}
