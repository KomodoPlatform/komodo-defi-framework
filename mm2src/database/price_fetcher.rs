use crate::mm2::lp_ordermatch::process_price_request;
use bigdecimal::BigDecimal;
use common::log::debug;

const MY_PRICE_ENDPOINT1: &str = "https://prices.komodo.live:1313/api/v2/tickers";
const MY_PRICE_ENDPOINT2: &str = "https://prices.cipig.net:1717/api/v2/tickers";

// Fetcher funtion to test fetching latest prices from different endoints.
async fn try_price_fetcher_endpoint(
    endpoint: &str,
    base: String,
    rel: String,
) -> Result<(Option<BigDecimal>, Option<BigDecimal>), String> {
    debug!("Trying {} to fetch coins latest price...", endpoint);
    match process_price_request(endpoint).await {
        Ok(response) => match response.get_cex_rates(base.to_owned(), rel.to_owned()) {
            Some(fiat_price) => {
                let fiat_price = fiat_price.get_rate_price();
                Ok((Some(fiat_price.0), Some(fiat_price.1)))
            },
            None => Err(format!(
                "Fetching from {} endpoint failed. Let's try to fetch again from another endpoint",
                endpoint
            )),
        },
        Err(_) => Err(format!(
            "Oops! an error encountered while fetching from {} too.",
            endpoint
        )),
    }
}

// Consume try_price_fetcher_endpoint result here and return None if fetching fails or successful.
pub async fn fetch_swap_coins_price(base: String, rel: String) -> (Option<BigDecimal>, Option<BigDecimal>) {
    match try_price_fetcher_endpoint(MY_PRICE_ENDPOINT1, base.to_owned(), rel.to_owned()).await {
        Ok(response) => return response,
        Err(e) => {
            debug!("{}", e);
            match try_price_fetcher_endpoint(MY_PRICE_ENDPOINT2, base.to_owned(), rel.to_owned()).await {
                Ok(response) => return response,
                Err(_) => (None, None),
            }
        },
    }
}
