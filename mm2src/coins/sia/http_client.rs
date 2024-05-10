use crate::sia::address::Address;
use crate::sia::SiaHttpConf;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _; // required for .encode() method
use core::fmt::Display;
use core::time::Duration;
use mm2_number::MmNumber;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Client, Error, Url};
use serde::de::DeserializeOwned;
use std::ops::Deref;
use std::sync::Arc;

#[cfg(test)] use std::str::FromStr;

const ENDPOINT_CONSENSUS_TIP: &str = "api/consensus/tip";

/// HTTP(s) client for Sia-protocol coins
#[derive(Debug)]
pub struct SiaHttpClientImpl {
    /// Name of coin the http client is intended to work with
    pub coin_ticker: String,
    /// The uri to send requests to
    pub uri: String,
    /// Value of Authorization header password, e.g. "Basic base64(:password)"
    pub auth: String,
}

#[derive(Clone, Debug)]
pub struct SiaApiClient(pub Arc<SiaApiClientImpl>);

impl Deref for SiaApiClient {
    type Target = SiaApiClientImpl;
    fn deref(&self) -> &SiaApiClientImpl { &self.0 }
}

impl SiaApiClient {
    pub fn new(_coin_ticker: &str, http_conf: SiaHttpConf) -> Result<Self, SiaApiClientError> {
        let new_arc = SiaApiClientImpl::new(http_conf.url, &http_conf.auth)?;
        Ok(SiaApiClient(Arc::new(new_arc)))
    }
}

#[derive(Debug)]
pub struct SiaApiClientImpl {
    client: Client,
    base_url: Url,
}

// this is neccesary to show the URL in error messages returned to the user
// this can be removed in favor of using ".with_url()" once reqwest is updated to v0.11.23
#[derive(Debug)]
pub struct ReqwestErrorWithUrl {
    error: Error,
    url: Url,
}

impl Display for ReqwestErrorWithUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Error: {}, URL: {}", self.error, self.url)
    }
}

#[derive(Debug, Display)]
pub enum SiaApiClientError {
    Timeout(String),
    BuildError(String),
    ApiUnreachable(String),
    ReqwestError(ReqwestErrorWithUrl),
    UrlParse(url::ParseError),
}

impl From<SiaApiClientError> for String {
    fn from(e: SiaApiClientError) -> Self { format!("{:?}", e) }
}

async fn fetch_and_parse<T: DeserializeOwned>(client: &Client, url: Url) -> Result<T, SiaApiClientError> {
    client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| {
            SiaApiClientError::ReqwestError(ReqwestErrorWithUrl {
                error: e,
                url: url.clone(),
            })
        })?
        .json::<T>()
        .await
        .map_err(|e| SiaApiClientError::ReqwestError(ReqwestErrorWithUrl { error: e, url }))
}

// https://github.com/SiaFoundation/core/blob/4e46803f702891e7a83a415b7fcd7543b13e715e/types/types.go#L181
#[derive(Deserialize, Serialize, Debug)]
pub struct GetConsensusTipResponse {
    pub height: u64,
    pub id: String, // TODO this can match "BlockID" type
}

// https://github.com/SiaFoundation/walletd/blob/9574e69ff0bf84de1235b68e78db2a41d5e27516/api/api.go#L36
// https://github.com/SiaFoundation/walletd/blob/9574e69ff0bf84de1235b68e78db2a41d5e27516/wallet/wallet.go#L25
#[derive(Deserialize, Serialize, Debug)]
pub struct GetAddressesBalanceResponse {
    pub siacoins: MmNumber,
    #[serde(rename = "immatureSiacoins")]
    pub immature_siacoins: MmNumber,
    pub siafunds: u64,
}

impl SiaApiClientImpl {
    fn new(base_url: Url, password: &str) -> Result<Self, SiaApiClientError> {
        let mut headers = HeaderMap::new();
        let auth_value = format!("Basic {}", BASE64.encode(format!(":{}", password)));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth_value).map_err(|e| SiaApiClientError::BuildError(e.to_string()))?,
        );

        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(10)) // TODO make this configurable
            .build()
            .map_err(|e| {
                SiaApiClientError::ReqwestError(ReqwestErrorWithUrl {
                    error: e,
                    url: base_url.clone(),
                })
            })?;
        Ok(SiaApiClientImpl { client, base_url })
    }

    pub async fn get_consensus_tip(&self) -> Result<GetConsensusTipResponse, SiaApiClientError> {
        let base_url = self.base_url.clone();
        let endpoint_url = base_url
            .join(ENDPOINT_CONSENSUS_TIP)
            .map_err(SiaApiClientError::UrlParse)?;

        fetch_and_parse::<GetConsensusTipResponse>(&self.client, endpoint_url).await
    }

    pub async fn get_addresses_balance(
        &self,
        address: &Address,
    ) -> Result<GetAddressesBalanceResponse, SiaApiClientError> {
        self.get_addresses_balance_str(&address.str_without_prefix()).await
    }

    // use get_addresses_balance whenever possible to rely on Address deserialization
    pub async fn get_addresses_balance_str(
        &self,
        address: &str,
    ) -> Result<GetAddressesBalanceResponse, SiaApiClientError> {
        let base_url = self.base_url.clone();

        let endpoint_path = format!("api/addresses/{}/balance", address);
        let endpoint_url = base_url.join(&endpoint_path).map_err(SiaApiClientError::UrlParse)?;

        fetch_and_parse::<GetAddressesBalanceResponse>(&self.client, endpoint_url).await
    }

    pub async fn get_height(&self) -> Result<u64, SiaApiClientError> {
        let resp = self.get_consensus_tip().await?;
        Ok(resp.height)
    }
}

#[tokio::test]
#[ignore]
async fn test_api_client_timeout() {
    let api_client = SiaApiClientImpl::new(Url::parse("http://foo").unwrap(), "password").unwrap();
    let result = api_client.get_consensus_tip().await;
    assert!(matches!(result, Err(SiaApiClientError::Timeout(_))));
}

// TODO all of the following must be adapted to use Docker Sia node
#[tokio::test]
#[ignore]
async fn test_api_client_invalid_auth() {
    let api_client = SiaApiClientImpl::new(Url::parse("http://127.0.0.1:9980").unwrap(), "password").unwrap();
    let result = api_client.get_consensus_tip().await;
    assert!(matches!(result, Err(SiaApiClientError::BuildError(_))));
}

// TODO must be adapted to use Docker Sia node
#[tokio::test]
#[ignore]
async fn test_api_client() {
    let api_client = SiaApiClientImpl::new(Url::parse("http://127.0.0.1:9980").unwrap(), "password").unwrap();
    let _result = api_client.get_consensus_tip().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn test_api_get_addresses_balance() {
    let api_client = SiaApiClientImpl::new(Url::parse("http://127.0.0.1:9980").unwrap(), "password").unwrap();
    let address =
        Address::from_str("addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e411f").unwrap();
    let result = api_client.get_addresses_balance(&address).await.unwrap();
    println!("ret {:?}", result);
}

#[tokio::test]
#[ignore]
async fn test_api_get_addresses_balance_invalid() {
    let api_client = SiaApiClientImpl::new(Url::parse("http://127.0.0.1:9980").unwrap(), "password").unwrap();
    let result = api_client.get_addresses_balance_str("what").await.unwrap();
    println!("ret {:?}", result);
}
