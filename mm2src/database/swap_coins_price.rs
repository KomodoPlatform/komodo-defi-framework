use crate::mm2::lp_ordermatch::{process_price_request, KMD_PRICE_ENDPOINT};
use bigdecimal::BigDecimal;
// use common::log::{debug, error};

pub async fn swap_coins_price(base: Option<String>, rel: Option<String>) -> (Option<BigDecimal>, Option<BigDecimal>) {
    match process_price_request(KMD_PRICE_ENDPOINT).await {
        Ok(response) => match response.get_cex_rates(base.clone().unwrap(), rel.clone().unwrap()) {
            Some(fiat_price) => {
                let fiat_price = fiat_price.get_rate_price();
                return (Some(fiat_price.0), Some(fiat_price.1));
            },
            None => (None, None),
        },
        Err(_) => (None, None),
    }
}
