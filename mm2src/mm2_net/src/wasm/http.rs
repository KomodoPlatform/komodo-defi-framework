use crate::grpc_web::PostGrpcWebErr;
use crate::transport::{SlurpError, SlurpResult};
use crate::wasm::body_stream::ResponseBody;

use common::executor::spawn_local;
use common::{stringify_js_error, APPLICATION_GRPC_WEB_PROTO, APPLICATION_JSON, X_GRPC_WEB};
use futures::channel::oneshot;
use futures_util::Future;
use http::header::{ACCEPT, CONTENT_TYPE};
use http::response::Builder;
use http::{HeaderMap, Request, Response, StatusCode};
use js_sys::Array;
use js_sys::Uint8Array;
use mm2_err_handle::prelude::*;
use std::collections::HashMap;
use std::{pin::Pin,
          task::{Context, Poll}};
use tonic::body::BoxBody;
use tonic::codegen::Body;
use tower_service::Service;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request as JsRequest, RequestInit, RequestMode, Response as JsResponse};

/// The result containing either a pair of (HTTP status code, body) or a stringified error.
pub type FetchResult<T> = Result<(StatusCode, T), MmError<SlurpError>>;

/// Executes a GET request, returning the response status, headers and body.
/// Please note the return header map is empty, because `wasm_bindgen` doesn't provide the way to extract all headers.
pub async fn slurp_url(url: &str) -> SlurpResult {
    FetchRequest::get(url)
        .request_str()
        .await
        .map(|(status_code, response)| (status_code, HeaderMap::new(), response.into_bytes()))
}

/// Executes a GET request with additional headers.
/// Returning the response status, headers and body.
/// Please note the return header map is empty, because `wasm_bindgen` doesn't provide the way to extract all headers.
pub async fn slurp_url_with_headers(url: &str, headers: Vec<(&str, &str)>) -> SlurpResult {
    FetchRequest::get(url)
        .headers(headers)
        .request_str()
        .await
        .map(|(status_code, response)| (status_code, HeaderMap::new(), response.into_bytes()))
}

/// Executes a POST request, returning the response status, headers and body.
/// Please note the return header map is empty, because `wasm_bindgen` doesn't provide the way to extract all headers.
pub async fn slurp_post_json(url: &str, body: String) -> SlurpResult {
    FetchRequest::post(url)
        .header(CONTENT_TYPE.as_str(), APPLICATION_JSON)
        .body_utf8(body)
        .request_str()
        .await
        .map(|(status_code, response)| (status_code, HeaderMap::new(), response.into_bytes()))
}

/// Sets the response headers and extracts the content type.
///
/// This function takes a `Builder` for a response and a `JsResponse` from which it extracts
/// the headers and the content type.
fn set_response_headers_and_content_type(
    mut result: Builder,
    response: &JsResponse,
) -> Result<(Builder, Option<String>), MmError<SlurpError>> {
    let headers = response.headers();

    let header_iter =
        js_sys::try_iter(headers.as_ref()).map_to_mm(|err| SlurpError::InvalidRequest(format!("{err:?}")))?;

    let mut content_type = None;

    if let Some(header_iter) = header_iter {
        for header in header_iter {
            let header = header.map_to_mm(|err| SlurpError::InvalidRequest(format!("{err:?}")))?;
            let pair: Array = header.into();

            let header_name = pair.get(0).as_string();
            let header_value = pair.get(1).as_string();

            match (header_name, header_value) {
                (Some(header_name), Some(header_value)) => {
                    if header_name == CONTENT_TYPE.as_str() {
                        content_type = Some(header_value.clone());
                    }

                    result = result.header(header_name, header_value);
                },
                _ => continue,
            }
        }
    }

    Ok((result, content_type))
}

pub struct FetchRequest {
    uri: String,
    method: FetchMethod,
    headers: HashMap<String, String>,
    body: Option<RequestBody>,
    mode: Option<RequestMode>,
}

impl FetchRequest {
    pub fn get(uri: &str) -> FetchRequest {
        FetchRequest {
            uri: uri.to_owned(),
            method: FetchMethod::Get,
            headers: HashMap::new(),
            body: None,
            mode: None,
        }
    }

    pub fn post(uri: &str) -> FetchRequest {
        FetchRequest {
            uri: uri.to_owned(),
            method: FetchMethod::Post,
            headers: HashMap::new(),
            body: None,
            mode: None,
        }
    }

    pub fn body_utf8(mut self, body: String) -> FetchRequest {
        self.body = Some(RequestBody::Utf8(body));
        self
    }

    pub fn body_bytes(mut self, body: Vec<u8>) -> FetchRequest {
        self.body = Some(RequestBody::Bytes(body));
        self
    }

    /// Set the mode to [`RequestMode::Cors`].
    /// The request is no-cors by default.
    pub fn cors(mut self) -> FetchRequest {
        self.mode = Some(RequestMode::Cors);
        self
    }

    pub fn header(mut self, key: &str, val: &str) -> FetchRequest {
        self.headers.insert(key.to_owned(), val.to_owned());
        self
    }

    pub fn headers(mut self, headers: Vec<(&str, &str)>) -> FetchRequest {
        for (key, value) in headers {
            self.headers.insert(key.to_owned(), value.to_owned());
        }
        self
    }

    pub async fn request_str(self) -> FetchResult<String> {
        let (tx, rx) = oneshot::channel();
        Self::spawn_fetch_str(self, tx);
        match rx.await {
            Ok(res) => res,
            Err(_e) => MmError::err(SlurpError::Internal("Spawned future has been canceled".to_owned())),
        }
    }

    pub async fn request_array(self) -> FetchResult<Vec<u8>> {
        let (tx, rx) = oneshot::channel();
        Self::spawn_fetch_array(self, tx);
        match rx.await {
            Ok(res) => res,
            Err(_e) => MmError::err(SlurpError::Internal("Spawned future has been canceled".to_owned())),
        }
    }

    pub async fn request_stream_response(self) -> FetchResult<Response<ResponseBody>> {
        let (tx, rx) = oneshot::channel();
        Self::spawn_fetch_stream_response(self, tx);
        match rx.await {
            Ok(res) => res,
            Err(_e) => MmError::err(SlurpError::Internal("Spawned future has been canceled".to_owned())),
        }
    }

    fn spawn_fetch_str(request: Self, tx: oneshot::Sender<FetchResult<String>>) {
        let fut = async move {
            let result = Self::fetch_str(request).await;
            tx.send(result).ok();
        };

        // The spawned future doesn't capture shared pointers,
        // so we can use `spawn_local` here.
        spawn_local(fut);
    }

    fn spawn_fetch_array(request: Self, tx: oneshot::Sender<FetchResult<Vec<u8>>>) {
        let fut = async move {
            let result = Self::fetch_array(request).await;
            tx.send(result).ok();
        };

        // The spawned future doesn't capture shared pointers,
        // so we can use `spawn_local` here.
        spawn_local(fut);
    }

    fn spawn_fetch_stream_response(request: Self, tx: oneshot::Sender<FetchResult<Response<ResponseBody>>>) {
        let fut = async move {
            let result = Self::fetch_and_stream_response(request).await;
            tx.send(result).ok();
        };

        // The spawned future doesn't capture shared pointers,
        // so we can use `spawn_local` here.
        spawn_local(fut);
    }

    async fn fetch(request: Self) -> FetchResult<JsResponse> {
        let window = web_sys::window().expect("!window");
        let uri = request.uri;

        let mut req_init = RequestInit::new();
        req_init.method(request.method.as_str());
        req_init.body(request.body.map(RequestBody::into_js_value).as_ref());

        if let Some(mode) = request.mode {
            req_init.mode(mode);
        }

        let js_request = JsRequest::new_with_str_and_init(&uri, &req_init)
            .map_to_mm(|e| SlurpError::Internal(stringify_js_error(&e)))?;
        for (hkey, hval) in request.headers {
            js_request
                .headers()
                .set(&hkey, &hval)
                .map_to_mm(|e| SlurpError::Internal(stringify_js_error(&e)))?;
        }

        let request_promise = window.fetch_with_request(&js_request);

        let future = JsFuture::from(request_promise);
        let resp_value = future.await.map_to_mm(|e| SlurpError::Transport {
            uri: uri.clone(),
            error: stringify_js_error(&e),
        })?;
        let js_response: JsResponse = match resp_value.dyn_into() {
            Ok(res) => res,
            Err(origin_val) => {
                let error = format!("Error casting {:?} to 'JsResponse'", origin_val);
                return MmError::err(SlurpError::Internal(error));
            },
        };

        let status_code = js_response.status();
        let status_code = match StatusCode::from_u16(status_code) {
            Ok(code) => code,
            Err(e) => {
                let error = format!("Unexpected HTTP status code, found {}: {}", status_code, e);
                return MmError::err(SlurpError::ErrorDeserializing { uri, error });
            },
        };

        Ok((status_code, js_response))
    }

    /// The private non-Send method that is called in a spawned future.
    async fn fetch_str(request: Self) -> FetchResult<String> {
        let uri = request.uri.clone();
        let (status_code, js_response) = Self::fetch(request).await?;

        let resp_txt_fut = match js_response.text() {
            Ok(txt) => txt,
            Err(e) => {
                let error = format!("Expected text, found {:?}: {}", js_response, stringify_js_error(&e));
                return MmError::err(SlurpError::ErrorDeserializing { uri, error });
            },
        };
        let resp_txt = JsFuture::from(resp_txt_fut)
            .await
            .map_to_mm(|e| SlurpError::Transport {
                uri: uri.clone(),
                error: stringify_js_error(&e),
            })?;

        let resp_str = match resp_txt.as_string() {
            Some(string) => string,
            None => {
                let error = format!("Expected a UTF-8 string JSON, found {:?}", resp_txt);
                return MmError::err(SlurpError::ErrorDeserializing { uri, error });
            },
        };

        Ok((status_code, resp_str))
    }

    /// The private non-Send method that is called in a spawned future.
    async fn fetch_array(request: Self) -> FetchResult<Vec<u8>> {
        let uri = request.uri.clone();
        let (status_code, js_response) = Self::fetch(request).await?;

        let resp_array_fut = match js_response.array_buffer() {
            Ok(blob) => blob,
            Err(e) => {
                let error = format!(
                    "Expected blob, found {:?}: {}",
                    js_response,
                    common::stringify_js_error(&e)
                );
                return MmError::err(SlurpError::ErrorDeserializing { uri, error });
            },
        };
        let resp_array = JsFuture::from(resp_array_fut)
            .await
            .map_to_mm(|e| SlurpError::ErrorDeserializing {
                uri: uri.clone(),
                error: stringify_js_error(&e),
            })?;

        let array = Uint8Array::new(&resp_array);

        Ok((status_code, array.to_vec()))
    }

    /// The private non-Send method that is called in a spawned future.
    async fn fetch_and_stream_response(request: Self) -> FetchResult<Response<ResponseBody>> {
        let uri = request.uri.clone();
        let (status_code, js_response) = Self::fetch(request).await?;

        let resp_stream = match js_response.body() {
            Some(txt) => txt,
            None => {
                let error = format!("Expected readable stream, found {:?}:", js_response,);
                return MmError::err(SlurpError::ErrorDeserializing { uri, error });
            },
        };

        let builder = Response::builder().status(status_code);
        let (builder, content_type) = set_response_headers_and_content_type(builder, &js_response)?;
        let content_type =
            content_type.ok_or_else(|| MmError::new(SlurpError::InvalidRequest("MissingContentType".to_string())))?;
        let body = ResponseBody::new(resp_stream, &content_type)
            .map_to_mm(|err| SlurpError::InvalidRequest(format!("{err:?}")))?;

        Ok((
            status_code,
            builder
                .body(body)
                .map_to_mm(|err| SlurpError::InvalidRequest(err.to_string()))?,
        ))
    }
}

enum FetchMethod {
    Get,
    Post,
}

impl FetchMethod {
    fn as_str(&self) -> &'static str {
        match self {
            FetchMethod::Get => "GET",
            FetchMethod::Post => "POST",
        }
    }
}

enum RequestBody {
    Utf8(String),
    Bytes(Vec<u8>),
}

impl RequestBody {
    fn into_js_value(self) -> JsValue {
        match self {
            RequestBody::Utf8(string) => JsValue::from_str(&string),
            RequestBody::Bytes(bytes) => {
                let js_array = Uint8Array::from(bytes.as_slice());
                js_array.into()
            },
        }
    }
}

#[derive(Clone)]
pub struct TonicClient(String);

impl TonicClient {
    pub fn new(url: String) -> Self { Self(url) }
}

impl Service<Request<BoxBody>> for TonicClient {
    type Response = Response<ResponseBody>;

    type Error = MmError<PostGrpcWebErr>;

    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> { Poll::Ready(Ok(())) }

    fn call(&mut self, request: Request<BoxBody>) -> Self::Future { Box::pin(call(self.0.clone(), request)) }
}

async fn call(mut base_url: String, request: Request<BoxBody>) -> MmResult<Response<ResponseBody>, PostGrpcWebErr> {
    base_url.push_str(&request.uri().to_string());

    let body = request
        .into_body()
        .data()
        .await
        .transpose()
        .map_err(|err| PostGrpcWebErr::Status(err.to_string()))?;
    let body = body.ok_or_else(|| MmError::new(PostGrpcWebErr::InvalidRequest("Invalid request body".to_string())))?;

    Ok(FetchRequest::post(&base_url)
        .body_bytes(body.to_vec())
        .header(CONTENT_TYPE.as_str(), APPLICATION_GRPC_WEB_PROTO)
        .header(ACCEPT.as_str(), APPLICATION_GRPC_WEB_PROTO)
        .header(X_GRPC_WEB, "1")
        .request_stream_response()
        .await?
        .1)
}

mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn fetch_get_test() {
        let (status, body) = FetchRequest::get(
            "https://testnet.qtum.info/api/raw-tx/d71846e7881af5eee026f4de92765a4fc75d99fae5ebd33311c91e9719ddafa5",
        )
        .request_str()
        .await
        .expect("!FetchRequest::request_str");

        let expected = "02000000017059c44c764ce06c22b1144d05a19b72358e75708836fc9472490a6f68862b79010000004847304402204ecc54f493c5c75efdbad0771f76173b3314ee7836c469f97a4659e1eef9de4a02200dfe70294e0aa0c6795ae349ddc858212c3293b8affd8c44a6bf6699abaef9d701ffffffff0300000000000000000016c3e748040000002321037d86ede18754defcd4759cf7fda52bff47703701a7feb66e2045e8b6c6aac236ace8b9df05000000001976a9149e032d4b0090a11dc40fe6c47601499a35d55fbb88ac00000000".to_string();

        assert!(status.is_success(), "{:?} {:?}", status, body);
        assert_eq!(body, expected);
    }
}