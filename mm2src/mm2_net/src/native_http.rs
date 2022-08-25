use crate::transport::{SlurpError, SlurpResult};
use common::wio::{drive03, HYPER};
use common::APPLICATION_JSON;
use futures::channel::oneshot::Canceled;
use http::{header, Request};
use hyper::Body;
use mm2_err_handle::prelude::*;

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

/// `http::Error` can appear on an HTTP request [`http::Builder::build`] building.
impl From<http::Error> for SlurpError {
    fn from(e: http::Error) -> Self { SlurpError::InvalidRequest(e.to_string()) }
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
    let req = Request::builder().uri(url).body(Vec::new())?;
    slurp_req(req).await
}

/// Executes a POST request, returning the response status, headers and body.
pub async fn slurp_post_json(url: &str, body: String) -> SlurpResult {
    let request = Request::builder()
        .method("POST")
        .uri(url)
        .header(header::CONTENT_TYPE, APPLICATION_JSON)
        .body(body.into())?;
    slurp_req(request).await
}

#[cfg(test)]
mod tests {
    use crate::native_http::slurp_url;
    use common::block_on;

    #[test]
    fn test_slurp_req() {
        let (status, headers, body) = block_on(slurp_url("https://httpbin.org/get")).unwrap();
        assert!(status.is_success(), "{:?} {:?} {:?}", status, headers, body);
    }
}
