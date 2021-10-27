use crate::mm_error::prelude::*;
use crate::transport::{SlurpError, SlurpResult};
use crate::wio::{drive03, HYPER};
use futures::channel::oneshot::Canceled;
use http::Request;
use hyper::Body;

impl From<Canceled> for SlurpError {
    fn from(_: Canceled) -> Self { SlurpError::Internal("Spawned Slurp future has been canceled".to_owned()) }
}

impl SlurpError {
    fn from_hyper_error(e: hyper::Error, uri: String) -> SlurpError {
        let error = e.to_string();
        if e.is_parse() || e.is_parse_status() || e.is_parse_too_large() {
            SlurpError::ErrorDeserializing { uri, error }
        } else if e.is_user() {
            SlurpError::InvalidRequest(error)
        } else if e.is_timeout() {
            SlurpError::Timeout { uri, error }
        } else {
            SlurpError::Transport { uri, error }
        }
    }
}

/// Executes a Hyper request, returning the response status, headers and body.
pub async fn slurp_req(request: Request<Vec<u8>>) -> SlurpResult {
    let uri = request.uri().to_string();
    let (head, body) = request.into_parts();
    let request = Request::from_parts(head, Body::from(body));

    let request_f = HYPER.request(request);
    let response = drive03(request_f)
        .await?
        .map_to_mm(|e| SlurpError::from_hyper_error(e, uri.clone()))?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body();
    let output = hyper::body::to_bytes(body)
        .await
        .map_to_mm(|e| SlurpError::from_hyper_error(e, uri.clone()))?;
    Ok((status, headers, output.to_vec()))
}

/// Executes a GET request, returning the response status, headers and body.
pub async fn slurp_url(url: &str) -> SlurpResult {
    let req = Request::builder()
        .uri(url)
        .body(Vec::new())
        .map_to_mm(|e| SlurpError::InvalidRequest(e.to_string()))?;
    slurp_req(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;

    #[test]
    fn test_slurp_req() {
        let (status, headers, body) = block_on(slurp_url("https://httpbin.org/get")).unwrap();
        assert!(status.is_success(), "{:?} {:?} {:?}", status, headers, body);
    }
}
