#[path = "activation/utxo.rs"] pub mod utxo;

use common::serde_derive::{Deserialize, Serialize};
use mm2_number::BigDecimal;

#[derive(Serialize, Deserialize)]
pub struct EnabledCoin {
    pub ticker: String,
    pub address: String,
}

pub type GetEnabledResponse = Vec<EnabledCoin>;

#[derive(Debug, Serialize, Deserialize)]
pub struct CoinInitResponse {
    pub result: String,
    pub address: String,
    pub balance: BigDecimal,
    pub unspendable_balance: BigDecimal,
    pub coin: String,
    pub required_confirmations: u64,
    pub requires_notarization: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mature_confirmations: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
/// The priority of an electrum connection when selective policy is in effect.
///
/// Primary connections are considered first and only if all of them are faulty
/// will the secondary connections be considered.
pub enum Priority {
    Primary,
    Secondary,
}

impl Default for Priority {
    fn default() -> Self { Priority::Secondary }
}
