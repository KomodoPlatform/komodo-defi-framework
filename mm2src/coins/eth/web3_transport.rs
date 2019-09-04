use common::StringError;
use common::wio::slurp_req;
use common::custom_futures::select_ok_sequential;
use futures01::Future;
use futures_timer::{FutureExt};
use http::header::HeaderValue;
use jsonrpc_core::{Call, Response};
use serde_json::{Value as Json};
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
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
        let mut futures = vec![];
        for uri in self.uris.iter() {
            let request = to_string(&request);
            let mut req = http::Request::new(Vec::from(request.clone()));
            *req.method_mut() = http::Method::POST;
            *req.uri_mut() = uri.clone();
            req.headers_mut().insert(http::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
            let fut = slurp_req(req)
                .map_err(|e| StringError(e))
                .timeout(Duration::from_secs(60))
                .map_err(move |e| {
                    log!("Error " (e.0) " on request " (request));
                    ERRL!("{}", e.0)
                });
            futures.push(fut);
        }

        Box::new(select_ok_sequential(futures)
            .map_err(|errs| ErrorKind::Transport(ERRL!("{:?}", errs)).into())
            .and_then(|(_, _, body)| single_response(body))
        )
    }
}
