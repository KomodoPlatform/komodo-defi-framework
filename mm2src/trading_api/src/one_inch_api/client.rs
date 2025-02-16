use super::errors::ApiClientError;
use crate::one_inch_api::errors::NativeError;
use common::{log, StatusCode};
#[cfg(feature = "test-ext-api")] use lazy_static::lazy_static;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::{map_mm_error::MapMmError,
                     map_to_mm::MapToMmResult,
                     mm_error::{MmError, MmResult}};
use mm2_net::transport::slurp_url_with_headers;
use serde::de::DeserializeOwned;
use url::Url;

#[cfg(feature = "test-ext-api")] use common::executor::Timer;

#[cfg(feature = "test-ext-api")]
use futures::lock::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

#[cfg(any(test, feature = "mocktopus"))]
use mocktopus::macros::*;

const CLASSIC_SWAP_ENDPOINT_V6_0: &str = "swap/v6.0/";
const PORTFOLIO_PRICES_ENDPOINT_V1_0: &str = "portfolio/integrations/prices/v1/";

const SWAP_METHOD: &str = "swap";
const QUOTE_METHOD: &str = "quote";
const LIQUIDITY_SOURCES_METHOD: &str = "liquidity-sources";
const TOKENS_METHOD: &str = "tokens";
const CROSS_PRICES_METHOD: &str = "time_range/cross_prices";

const ONE_INCH_AGGREGATION_ROUTER_CONTRACT_V6_0: &str = "0x111111125421ca6dc452d289314280a0f8842a65";
const ONE_INCH_ETH_SPECIAL_CONTRACT: &str = "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

#[cfg(test)]
const ONE_INCH_API_TEST_URL: &str = "https://api.1inch.dev";

#[cfg(feature = "test-ext-api")]
lazy_static! {
    /// API key for testing
    static ref ONE_INCH_API_TEST_AUTH: String = std::env::var("ONE_INCH_API_TEST_AUTH").unwrap_or_default();
}

pub(crate) type QueryParams<'life> = Vec<(&'life str, String)>;

/// 1inch v6.0 supported eth-based chains
const ONE_INCH_V6_0_SUPPORTED_CHAINS: &[(&str, u64)] = &[
    ("Ethereum", 1),
    ("Optimism", 10),
    ("BSC", 56),
    ("Gnosis", 100),
    ("Polygon", 137),
    ("Fantom", 250),
    ("ZkSync", 324),
    ("Klaytn", 8217),
    ("Base", 8453),
    ("Arbitrum", 42161),
    ("Avalanche", 43114),
    ("Aurora", 1313161554),
];

pub(crate) struct UrlBuilder<'a> {
    base_url: Url,
    endpoint: &'a str,
    chain_id: Option<u64>,
    method_name: String,
    query_params: QueryParams<'a>,
}

impl<'a> UrlBuilder<'a> {
    /// Build url to call 1inch api's.
    /// Note: in the classic swap api chain_id is added into url path, in portfolio chain_id is a query param so it would be optional here.
    pub(crate) fn new(api_client: &ApiClient, chain_id: Option<u64>, endpoint: &'a str, method_name: String) -> Self {
        Self {
            base_url: api_client.base_url.clone(),
            endpoint,
            chain_id,
            method_name,
            query_params: vec![],
        }
    }

    pub(crate) fn with_query_params(&mut self, mut more_params: QueryParams<'a>) -> &mut Self {
        self.query_params.append(&mut more_params);
        self
    }

    #[allow(clippy::result_large_err)]
    pub(crate) fn build(&self) -> MmResult<Url, ApiClientError> {
        let url = self.base_url.join(self.endpoint)?;
        let url = if let Some(chain_id) = self.chain_id {
            url.join(&format!("{}/", chain_id))?
        } else {
            url
        };
        let url = url.join(self.method_name.as_str())?;
        Ok(Url::parse_with_params(
            url.as_str(),
            self.query_params
                .iter()
                .map(|v| (v.0, v.1.as_str()))
                .collect::<Vec<_>>(),
        )?)
    }
}

/// 1-inch API caller
pub struct ApiClient {
    base_url: Url,
}

#[allow(clippy::swap_ptr_to_ref)] // need for moctopus
#[cfg_attr(any(test, feature = "mocktopus"), mockable)]
impl ApiClient {
    #[allow(unused_variables)]
    #[allow(clippy::result_large_err)]
    pub fn new(ctx: &MmArc) -> MmResult<Self, ApiClientError> {
        #[cfg(not(test))]
        let url_cfg = ctx.conf["1inch_api"]
            .as_str()
            .ok_or(ApiClientError::InvalidParam("No API config param".to_owned()))?;

        #[cfg(test)]
        let url_cfg = ONE_INCH_API_TEST_URL;

        Ok(Self {
            base_url: Url::parse(url_cfg)?,
        })
    }

    pub const fn eth_special_contract() -> &'static str { ONE_INCH_ETH_SPECIAL_CONTRACT }

    pub const fn classic_swap_contract() -> &'static str { ONE_INCH_AGGREGATION_ROUTER_CONTRACT_V6_0 }

    pub fn is_chain_supported(chain_id: u64) -> bool {
        ONE_INCH_V6_0_SUPPORTED_CHAINS.iter().any(|(_name, id)| *id == chain_id)
    }

    fn get_headers() -> Vec<(&'static str, &'static str)> {
        vec![
            #[cfg(feature = "test-ext-api")]
            ("Authorization", ONE_INCH_API_TEST_AUTH.as_str()),
            ("accept", "application/json"),
            ("content-type", "application/json"),
        ]
    }

    // TODO: maybe create a struct for each 1inch api (classic swap, portfolio etc) to return endpoint and available methods
    // with a common method 'endpoint()' and one or few 'xyz_method()' relevant only for each api
    pub fn classic_swap_endpoint() -> &'static str { CLASSIC_SWAP_ENDPOINT_V6_0 }

    pub fn portfolio_prices_endpoint() -> &'static str { PORTFOLIO_PRICES_ENDPOINT_V1_0 }

    pub const fn swap_method() -> &'static str { SWAP_METHOD }

    pub const fn quote_method() -> &'static str { QUOTE_METHOD }

    pub const fn liquidity_sources_method() -> &'static str { LIQUIDITY_SOURCES_METHOD }

    pub const fn tokens_method() -> &'static str { TOKENS_METHOD }

    pub const fn cross_prices_method() -> &'static str { CROSS_PRICES_METHOD }

    pub(crate) async fn call_api<T: DeserializeOwned>(api_url: &Url) -> MmResult<T, ApiClientError> {
        #[cfg(feature = "test-ext-api")]
        let _guard = ApiClient::one_req_per_sec().await;

        log::debug!("1inch call url={}", api_url.to_string());
        let (status_code, _, body) = slurp_url_with_headers(api_url.as_str(), ApiClient::get_headers())
            .await
            .mm_err(ApiClientError::TransportError)?;
        log::debug!("1inch response body={}", String::from_utf8_lossy(&body));
        // TODO: handle text body errors like 'The limit of requests per second has been exceeded'
        let body = serde_json::from_slice(&body).map_to_mm(|err| ApiClientError::ParseBodyError {
            error_msg: err.to_string(),
        })?;
        if status_code != StatusCode::OK {
            let error = NativeError::new(status_code, body);
            return Err(MmError::new(ApiClientError::from_native_error(error)));
        }
        serde_json::from_value(body).map_err(|err| {
            ApiClientError::ParseBodyError {
                error_msg: err.to_string(),
            }
            .into()
        })
    }

    pub async fn call_one_inch_api<'l, T: DeserializeOwned>(
        self,
        chain_id: Option<u64>,
        endpoint: &'l str,
        method: String,
        params: Option<QueryParams<'l>>,
    ) -> MmResult<T, ApiClientError> {
        let mut builder = UrlBuilder::new(&self, chain_id, endpoint, method);
        if let Some(params) = params {
            builder.with_query_params(params);
        }
        let api_url = builder.build()?;

        ApiClient::call_api(&api_url).await
    }

    /// Prevent concurrent calls
    #[cfg(feature = "test-ext-api")]
    async fn one_req_per_sec<'a>() -> AsyncMutexGuard<'a, ()> {
        lazy_static! {
            /// API key for testing
            static ref ONE_INCH_REQ_SYNC: AsyncMutex<()> = AsyncMutex::new(());
        }
        let guard = ONE_INCH_REQ_SYNC.lock().await;
        Timer::sleep(1.).await; // ensure 1 req per sec to prevent 1inch rate limiter error for dev account
        guard
    }
}
