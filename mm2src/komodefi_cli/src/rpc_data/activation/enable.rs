use mm2_rpc::data::legacy::GasStationPricePolicy;
use serde::{Deserialize, Serialize};
use skip_serializing_none::skip_serializing_none;

use super::SetTxHistory;

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct EnableRequest {
    coin: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    urls: Vec<String>,
    swap_contract_address: Option<String>,
    fallback_swap_contract: Option<String>,
    gas_station_url: Option<String>,
    gas_station_decimals: Option<u8>,
    gas_station_policy: Option<GasStationPricePolicy>,
    mm2: Option<u8>,
    #[serde(default)]
    pub(crate) tx_history: bool,
    required_confirmations: Option<u64>,
    requires_notarization: Option<bool>,
    contract_supports_watchers: Option<bool>,
}

impl SetTxHistory for EnableRequest {
    fn set_tx_history_impl(&mut self) { self.tx_history = true; }
}
