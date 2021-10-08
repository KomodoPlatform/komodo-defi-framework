use crate::executor::spawn_local;
use crate::log::warn;
use crate::stringify_js_error;
use futures::channel::oneshot;
use http::StatusCode;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, RequestMode, Response as JsResponse};

/// The result containing either a pair of (HTTP status code, body) or a stringified error.
pub type FetchResult<T> = Result<(StatusCode, T), String>;

macro_rules! js_err {
    ($($arg:tt)*) => {
        Err(JsValue::from_str(&ERRL!($($arg)*)))
    };
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

    pub async fn request_str(self) -> FetchResult<String> {
        let (tx, rx) = oneshot::channel();
        Self::spawn_fetch_str(self, tx);
        match rx.await {
            Ok(res) => res,
            Err(_e) => ERR!("Spawned future has been canceled"),
        }
    }

    fn spawn_fetch_str(request: Self, tx: oneshot::Sender<FetchResult<String>>) {
        let fut = async move {
            let result = Self::fetch_str(request)
                .await
                .map_err(|e| ERRL!("{}", stringify_js_error(&e)));
            if let Err(_res) = tx.send(result) {
                warn!("spawn_fetch_str] the channel already closed");
            }
        };
        spawn_local(fut);
    }

    /// The private non-Send method that is called in a spawned future.
    async fn fetch_str(request: Self) -> Result<(StatusCode, String), JsValue> {
        let window = web_sys::window().expect("!window");

        let mut req_init = RequestInit::new();
        req_init.method(request.method.as_str());
        req_init.body(request.body.map(RequestBody::into_js_value).as_ref());

        if let Some(mode) = request.mode {
            req_init.mode(mode);
        }

        let js_request = Request::new_with_str_and_init(&request.uri, &req_init)?;
        for (hkey, hval) in request.headers {
            js_request.headers().set(&hkey, &hval)?;
        }

        let request_promise = window.fetch_with_request(&js_request);

        let future = JsFuture::from(request_promise);
        let resp_value = future.await?;
        let js_response: JsResponse = match resp_value.dyn_into() {
            Ok(res) => res,
            Err(origin_val) => return js_err!("Error casting {:?} to 'JsResponse'", origin_val),
        };

        let resp_txt_fut = match js_response.text() {
            Ok(txt) => txt,
            Err(e) => {
                return js_err!(
                    "Expected text, found {:?}: {}",
                    js_response,
                    crate::stringify_js_error(&e)
                )
            },
        };
        let resp_txt = JsFuture::from(resp_txt_fut).await?;

        let resp_str = match resp_txt.as_string() {
            Some(string) => string,
            None => return js_err!("Expected a UTF-8 string JSON, found {:?}", resp_txt),
        };

        let status_code = js_response.status();
        let status_code = match StatusCode::from_u16(status_code) {
            Ok(code) => code,
            Err(e) => return js_err!("Unexpected HTTP status code, found {}: {}", status_code, e),
        };
        Ok((status_code, resp_str))
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
}

impl RequestBody {
    fn into_js_value(self) -> JsValue {
        match self {
            RequestBody::Utf8(string) => JsValue::from_str(&string),
        }
    }
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
