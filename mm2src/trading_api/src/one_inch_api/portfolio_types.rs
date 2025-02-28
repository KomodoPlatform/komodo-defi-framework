//! Structs to call 1inch portfolio api

#![allow(clippy::result_large_err)]

use super::client::QueryParams;
use super::errors::ApiClientError;
use common::{def_with_opt_param, push_if_some};
use mm2_err_handle::mm_error::MmResult;
use mm2_number::BigDecimal;
use serde::Deserialize;

#[derive(Default)]
pub enum DataGranularity {
    Month,
    Week,
    Day,
    FourHour,
    Hour,
    FifteenMin,
    #[default]
    FiveMin,
}

impl ToString for DataGranularity {
    fn to_string(&self) -> String {
        match self {
            DataGranularity::Month => "month".to_owned(),
            DataGranularity::Week => "week".to_owned(),
            DataGranularity::Day => "day".to_owned(),
            DataGranularity::FourHour => "4hour".to_owned(),
            DataGranularity::Hour => "hour".to_owned(),
            DataGranularity::FifteenMin => "15min".to_owned(),
            DataGranularity::FiveMin => "5min".to_owned(),
        }
    }
}

/// API params builder to get OHLC price history for token pair
/// See 1inch docs: https://portal.1inch.dev/documentation/apis/portfolio/swagger?method=get&path=%2Fintegrations%2Fprices%2Fv1%2Ftime_range%2Fcross_prices
#[derive(Default)]
pub struct CrossPriceParams {
    chain_id: u64,
    /// Base token address
    token0_address: String,
    /// Quote token address
    token1_address: String,
    /// Returned time series intervals
    granularity: Option<DataGranularity>,
    /// max number of time series
    limit: Option<u32>,
}

impl CrossPriceParams {
    pub fn new(chain_id: u64, token0_address: String, token1_address: String) -> Self {
        Self {
            chain_id,
            token0_address,
            token1_address,
            ..Default::default()
        }
    }

    def_with_opt_param!(granularity, DataGranularity);
    def_with_opt_param!(limit, u32);

    pub fn build_query_params(&self) -> MmResult<QueryParams<'static>, ApiClientError> {
        let mut params = vec![
            ("chain_id", self.chain_id.to_string()),
            ("token0_address", self.token0_address.clone()),
            ("token1_address", self.token1_address.clone()),
        ];

        push_if_some!(params, "granularity", &self.granularity);
        push_if_some!(params, "limit", &self.limit);

        Ok(params)
    }
}

/// Element of token_0/token_1 prices series returned from a cross prices call
#[derive(Clone, Deserialize, Debug)]
pub struct CrossPricesData {
    pub timestamp: u64,
    pub open: BigDecimal,
    pub low: BigDecimal,
    pub avg: BigDecimal,
    pub high: BigDecimal,
    pub close: BigDecimal,
}
