use common::mm_number::MmNumber;
use common::StatusCode;
use common::{log::{debug, error},
             mm_number::BigDecimal};
use mm2_err_handle::prelude::{MmError, OrMmError};
use mm2_net::transport::SlurpError;
use std::collections::HashMap;
use std::str::Utf8Error;

const PRICE_ENDPOINTS: [&str; 2] = [
    "https://prices.komodo.live:1313/api/v2/tickers",
    "https://prices.cipig.net:1717/api/v2/tickers",
];

#[derive(Debug)]
pub enum PriceServiceRequestError {
    HttpProcessError(String),
    ParsingAnswerError(String),
    Internal(String),
}

#[derive(Default)]
pub struct TickerInfosRegistry(HashMap<String, TickerInfos>);

#[derive(Debug, Serialize, Deserialize)]
pub struct TickerInfos {
    pub ticker: String,
    pub last_price: MmNumber,
    pub last_updated: String,
    pub last_updated_timestamp: u64,
    #[serde(rename = "volume24h")]
    pub volume24_h: MmNumber,
    pub price_provider: Provider,
    pub volume_provider: Provider,
    #[serde(rename = "sparkline_7d")]
    pub sparkline_7_d: Option<Vec<f64>>,
    pub sparkline_provider: Provider,
    #[serde(rename = "change_24h")]
    pub change_24_h: MmNumber,
    #[serde(rename = "change_24h_provider")]
    pub change_24_h_provider: Provider,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum Provider {
    #[serde(rename = "binance")]
    Binance,
    #[serde(rename = "coingecko")]
    Coingecko,
    #[serde(rename = "coinpaprika")]
    Coinpaprika,
    #[serde(rename = "forex")]
    Forex,
    #[serde(rename = "nomics")]
    Nomics,
    #[serde(rename = "unknown", other)]
    Unknown,
}

impl Default for Provider {
    fn default() -> Self { Provider::Unknown }
}

impl From<serde_json::Error> for PriceServiceRequestError {
    fn from(error: serde_json::Error) -> Self { PriceServiceRequestError::ParsingAnswerError(error.to_string()) }
}

impl From<std::string::String> for PriceServiceRequestError {
    fn from(error: String) -> Self { PriceServiceRequestError::HttpProcessError(error) }
}

impl From<std::str::Utf8Error> for PriceServiceRequestError {
    fn from(error: Utf8Error) -> Self { PriceServiceRequestError::HttpProcessError(error.to_string()) }
}

impl From<SlurpError> for PriceServiceRequestError {
    fn from(e: SlurpError) -> Self {
        let error = e.to_string();
        match e {
            SlurpError::ErrorDeserializing { .. } => PriceServiceRequestError::ParsingAnswerError(error),
            SlurpError::Transport { .. } | SlurpError::Timeout { .. } => {
                PriceServiceRequestError::HttpProcessError(error)
            },
            SlurpError::Internal(_) | SlurpError::InvalidRequest(_) => PriceServiceRequestError::Internal(error),
        }
    }
}

#[derive(Default, Clone, Debug)]
pub struct RateInfos {
    #[allow(dead_code)]
    pub base: String,
    #[allow(dead_code)]
    pub rel: String,
    pub base_price: MmNumber,
    pub rel_price: MmNumber,
    pub price: MmNumber,
    pub last_updated_timestamp: Option<u64>,
    pub base_provider: Provider,
    pub rel_provider: Provider,
}

impl RateInfos {
    pub fn retrieve_elapsed_times(&self) -> f64 {
        let time_diff: f64 = common::now_float() - self.last_updated_timestamp.unwrap_or_default() as f64;
        time_diff
    }

    pub fn new(base: String, rel: String) -> RateInfos {
        RateInfos {
            base,
            rel,
            base_provider: Provider::Unknown,
            rel_provider: Provider::Unknown,
            last_updated_timestamp: None,
            ..Default::default()
        }
    }

    pub fn get_rate_price(&self) -> (BigDecimal, BigDecimal) {
        (self.base_price.clone().into(), self.rel_price.clone().into())
    }
}

impl TickerInfosRegistry {
    fn get_infos(&self, ticker: &str) -> Option<&TickerInfos> {
        let mut ticker_infos = self.0.get(ticker);
        let limit = ticker.len() - 1;
        let pos = ticker.find('-').unwrap_or(limit);
        if ticker_infos.is_none() && pos < limit {
            ticker_infos = self.0.get(&ticker[0..pos])
        }
        ticker_infos
    }

    fn get_infos_pair(&self, base: &str, rel: &str) -> Option<(&TickerInfos, &TickerInfos)> {
        self.get_infos(base).zip(self.get_infos(rel))
    }

    pub fn get_cex_rates(&self, base: String, rel: String) -> Option<RateInfos> {
        match self.get_infos_pair(&base, &rel) {
            Some((base_price_infos, rel_price_infos)) => {
                let mut rate_infos = RateInfos::new(base, rel);
                if base_price_infos.price_provider == Provider::Unknown
                    || rel_price_infos.price_provider == Provider::Unknown
                    || base_price_infos.last_updated_timestamp == 0
                    || rel_price_infos.last_updated_timestamp == 0
                {
                    debug!(
                        "Unable to fetch tickers price. Tickers ({}/{})",
                        base_price_infos.last_price, rel_price_infos.last_price
                    );
                    return None;
                }

                rate_infos.base_provider = base_price_infos.price_provider.clone();
                rate_infos.rel_provider = rel_price_infos.price_provider.clone();
                rate_infos.last_updated_timestamp =
                    if base_price_infos.last_updated_timestamp <= rel_price_infos.last_updated_timestamp {
                        Some(base_price_infos.last_updated_timestamp)
                    } else {
                        Some(rel_price_infos.last_updated_timestamp)
                    };
                rate_infos.base_price = base_price_infos.last_price.clone();
                rate_infos.rel_price = rel_price_infos.last_price.clone();
                rate_infos.price = &base_price_infos.last_price / &rel_price_infos.last_price;
                Some(rate_infos)
            },
            None => None,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn process_price_request(price_url: &str) -> Result<TickerInfosRegistry, MmError<PriceServiceRequestError>> {
    debug!("Fetching price from: {}", price_url);
    let (status, headers, body) = mm2_net::native_http::slurp_url(price_url).await?;
    let (status_code, body, _) = (status, std::str::from_utf8(&body)?.trim().into(), headers);
    if status_code != StatusCode::OK {
        return MmError::err(PriceServiceRequestError::HttpProcessError(body));
    }
    let model: HashMap<String, TickerInfos> = serde_json::from_str(&body)?;
    Ok(TickerInfosRegistry(model))
}

#[cfg(target_arch = "wasm32")]
pub async fn process_price_request(price_url: &str) -> Result<TickerInfosRegistry, MmError<PriceServiceRequestError>> {
    debug!("Fetching price from: {}", price_url);
    let (status, headers, body) = mm2_net::wasm_http::slurp_url(price_url).await?;
    let (status_code, body, _) = (status, std::str::from_utf8(&body)?.trim().into(), headers);
    if status_code != StatusCode::OK {
        return MmError::err(PriceServiceRequestError::HttpProcessError(body));
    }
    let model: HashMap<String, TickerInfos> = serde_json::from_str(&body)?;
    Ok(TickerInfosRegistry(model))
}

pub async fn fetch_price_tickers(price_url: &str) -> Result<TickerInfosRegistry, MmError<PriceServiceRequestError>> {
    let model = process_price_request(price_url).await?;
    debug!("price registry size: {}", model.0.len());
    Ok(model)
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct CEXRates {
    pub base: BigDecimal,
    pub rel: BigDecimal,
}

/// fetcher funtion to try fetch latest price from single endoint.
async fn try_price_fetcher_endpoint(
    endpoint: &str,
    base: &str,
    rel: &str,
) -> Result<CEXRates, MmError<PriceServiceRequestError>> {
    let response = process_price_request(endpoint).await?;
    let fiat_price = response
        .get_cex_rates(base.to_string(), rel.to_string())
        .or_mm_err(|| PriceServiceRequestError::Internal("Couldn't fetch price".to_string()))?;
    let (base_usd_price, rel_usd_price) = fiat_price.get_rate_price();
    Ok(CEXRates {
        base: base_usd_price,
        rel: rel_usd_price,
    })
}

/// consume try_price_fetcher_endpoint result here using different endpoints.
/// return price data on Success or None on failure.
pub async fn fetch_swap_coins_price(base: Option<String>, rel: Option<String>) -> Option<CEXRates> {
    debug!("Trying to fetch coins latest price...");
    if let (Some(base), Some(rel)) = (base, rel) {
        for endpoint in PRICE_ENDPOINTS {
            match try_price_fetcher_endpoint(endpoint, &base, &rel).await {
                Ok(response) => return Some(response),
                // Continue fetching endpoints.
                Err(err) => error!("{:?}", err),
            }
        }
    }
    // Couldn't fetch prices.
    None
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    #[test]
    fn test_process_price_request() {
        use common::block_on;

        use super::*;
        let _resp = for endpoint in PRICE_ENDPOINTS {
            block_on(process_price_request(endpoint)).unwrap();
        };
    }

    #[test]
    fn test_fetch_swap_coins_price() {
        use common::block_on;

        use super::*;
        let expected = None;
        let actual = block_on(fetch_swap_coins_price(Some("ETH".to_string()), Some("JST".to_string())));
        assert_eq!(expected, actual);
    }

    #[test]
    fn test_get_cex_rates() {
        use common::mm_number::MmNumber;
        use wasm_timer::SystemTime;

        use crate::mm2::lp_price::{Provider, TickerInfos, TickerInfosRegistry};

        let mut registry = TickerInfosRegistry::default();
        let rates = registry
            .get_cex_rates("KMD".to_string(), "LTC".to_string())
            .unwrap_or_default();
        assert_eq!(rates.base_provider, Provider::Unknown);
        assert_eq!(rates.rel_provider, Provider::Unknown);

        registry.0.insert("KMD".to_string(), TickerInfos {
            ticker: "KMD".to_string(),
            last_price: MmNumber::from("10"),
            last_updated: "".to_string(),
            last_updated_timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            volume24_h: MmNumber::from("25000"),
            price_provider: Provider::Binance,
            volume_provider: Provider::Coinpaprika,
            sparkline_7_d: None,
            sparkline_provider: Default::default(),
            change_24_h: MmNumber::default(),
            change_24_h_provider: Default::default(),
        });

        registry.0.insert("LTC".to_string(), TickerInfos {
            ticker: "LTC".to_string(),
            last_price: MmNumber::from("500.0"),
            last_updated: "".to_string(),
            last_updated_timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            volume24_h: MmNumber::from("25000"),
            price_provider: Provider::Coingecko,
            volume_provider: Provider::Binance,
            sparkline_7_d: None,
            sparkline_provider: Default::default(),
            change_24_h: MmNumber::default(),
            change_24_h_provider: Default::default(),
        });

        registry.0.insert("USDT".to_string(), TickerInfos {
            ticker: "USDT".to_string(),
            last_price: MmNumber::from("1"),
            last_updated: "".to_string(),
            last_updated_timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            volume24_h: MmNumber::from("25000"),
            price_provider: Provider::Coingecko,
            volume_provider: Provider::Binance,
            sparkline_7_d: None,
            sparkline_provider: Default::default(),
            change_24_h: MmNumber::default(),
            change_24_h_provider: Default::default(),
        });

        let rates = registry
            .get_cex_rates("KMD".to_string(), "LTC".to_string())
            .unwrap_or_default();
        assert_eq!(rates.base_provider, Provider::Binance);
        assert_eq!(rates.rel_provider, Provider::Coingecko);
        assert_eq!(rates.price, MmNumber::from("0.02"));

        let usdt_infos = registry.get_infos("USDT-PLG20");
        assert_eq!(usdt_infos.is_some(), true);
        assert_eq!(usdt_infos.unwrap().last_price, MmNumber::from(1));

        let usdt_infos = registry.get_infos("USDT");
        assert_eq!(usdt_infos.is_some(), true);
        assert_eq!(usdt_infos.unwrap().last_price, MmNumber::from(1));
    }
}
