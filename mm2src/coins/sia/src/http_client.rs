use crate::http_endpoints::{AddressBalanceRequest, AddressBalanceResponse, ConsensusTipRequest, SiaApiRequest};
use crate::types::Address;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _; // required for .encode() method
use core::fmt::Display;
use core::time::Duration;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Client, Error as ReqwestError, Request, Url};
// use reqwest::Proxy; TODO remove debugging code
use derive_more::Display;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SiaHttpConf {
    pub url: Url,
    pub password: String,
}

#[derive(Debug, Clone)]
pub struct SiaApiClient {
    client: Client,
    conf: SiaHttpConf,
}

// this is neccesary to show the URL in error messages returned to the user
// this can be removed in favor of using ".with_url()" once reqwest is updated to v0.11.23
#[derive(Debug)]
pub struct ReqwestErrorWithUrl {
    error: ReqwestError,
    url: Url,
}

impl Display for ReqwestErrorWithUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Error: {}, URL: {}", self.error, self.url)
    }
}

// TODO clean up reqwest errors
// update reqwest to latest for `.with_url()` method
#[derive(Debug, Display)]
pub enum SiaApiClientError {
    Timeout(String),
    BuildError(String),
    ServerUnreachable(String),
    ReqwestFetchError(ReqwestErrorWithUrl), // TODO make an enum
    ReqwestError(reqwest::Error),
    ReqwestParseInvalidEncodingError(String),
    ReqwestParseInvalidJsonError(String),
    ReqwestParseUnexpectedTypeError(String),
    ReqwestTlsError(ReqwestErrorWithUrl),
    UrlParse(url::ParseError),
    UnexpectedHttpStatus(u16),
    ApiInternalError(String),
    SerializationError(serde_json::Error),
}

impl From<SiaApiClientError> for String {
    fn from(e: SiaApiClientError) -> Self { format!("{:?}", e) }
}

/// Generic function to fetch data from a URL and deserialize it into a specified type.
async fn fetch_and_parse<T: DeserializeOwned>(client: &Client, request: Request) -> Result<T, SiaApiClientError> {
    let url = request.url().clone(); // TODO remove this once reqwest crate is updated
    let fetched = client.execute(request).await.map_err(|e| {
        SiaApiClientError::ReqwestFetchError(ReqwestErrorWithUrl {
            error: e,
            url: url.clone(),
        })
    })?;

    let status = fetched.status().as_u16();
    match status {
        200 => {},
        500 => {
            // FIXME handle unwrap gracefully
            return Err(SiaApiClientError::ApiInternalError(fetched.text().await.unwrap()));
        },
        _ => {
            return Err(SiaApiClientError::UnexpectedHttpStatus(status));
        },
    }
    let response_text = fetched.text().await.map_err(|e| {
        SiaApiClientError::ReqwestParseInvalidEncodingError(
            ReqwestErrorWithUrl {
                error: e,
                url: url.clone(),
            }
            .to_string(),
        )
    })?;

    let json: serde_json::Value = serde_json::from_str(&response_text).map_err(|e| {
        SiaApiClientError::ReqwestParseInvalidJsonError(format!(
            "Response text: {} is not JSON as expected. {}",
            response_text,
            e.to_string()
        ))
    })?;

    let parsed: T = serde_json::from_value(json.clone()).map_err(|e| {
        SiaApiClientError::ReqwestParseUnexpectedTypeError(format!(
            "Response text: {} is not the expected type {:?} . {}",
            json.to_string(),
            std::any::type_name::<T>(),
            e.to_string()
        ))
    })?;

    Ok(parsed)
}

/// Implements the methods for sending specific requests and handling their responses.
impl SiaApiClient {
    /// Constructs a new instance of the API client using the provided base URL and password for authentication.
    pub async fn new(conf: SiaHttpConf) -> Result<Self, SiaApiClientError> {
        let mut headers = HeaderMap::new();
        let auth_value = format!("Basic {}", BASE64.encode(format!(":{}", conf.password)));
        headers.insert(
            AUTHORIZATION,
            // This error does not require a test case as it is impossible to trigger in practice
            // the from_str method can only return Err if the str is invalid ASCII
            // the encode() method can only return valid ASCII
            HeaderValue::from_str(&auth_value).map_err(|e| SiaApiClientError::BuildError(e.to_string()))?,
        );
        //let proxy = Proxy::http("http://127.0.0.1:8080").unwrap(); TODO remove debugging code
        let client = Client::builder()
            //.proxy(proxy)
            .default_headers(headers)
            .timeout(Duration::from_secs(10)) // TODO make this configurable
            .build()
            // covering this with a unit test seems to require altering the system's ssl certificates
            .map_err(|e| {
                SiaApiClientError::ReqwestTlsError(ReqwestErrorWithUrl {
                    error: e,
                    url: conf.url.clone(),
                })
            })?;
        let ret = SiaApiClient { client, conf };
        ret.dispatcher(ConsensusTipRequest).await?;
        Ok(ret)
    }

    /// General method for dispatching requests, handling routing and response parsing.
    pub async fn dispatcher<R: SiaApiRequest + Send>(&self, request: R) -> Result<R::Response, SiaApiClientError> {
        let req = request.to_http_request(&self.client, &self.conf.url)?;
        fetch_and_parse::<R::Response>(&self.client, req).await
    }

    pub async fn current_height(&self) -> Result<u64, SiaApiClientError> {
        let response = self.dispatcher(ConsensusTipRequest).await?;
        Ok(response.height)
    }

    pub async fn address_balance(&self, address: Address) -> Result<AddressBalanceResponse, SiaApiClientError> {
        self.dispatcher(AddressBalanceRequest { address }).await
    }
}
