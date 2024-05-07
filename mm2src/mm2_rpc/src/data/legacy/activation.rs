#[path = "activation/eth.rs"] pub mod eth;
#[path = "activation/utxo.rs"] pub mod utxo;

use skip_serializing_none::skip_serializing_none;

use common::serde_derive::{Deserialize, Serialize};
use mm2_number::BigDecimal;

#[derive(Serialize, Deserialize)]
pub struct EnabledCoin {
    pub ticker: String,
    pub address: String,
}

pub type GetEnabledResponse = Vec<EnabledCoin>;

#[skip_serializing_none]
#[derive(Debug, Serialize, Deserialize)]
pub struct CoinInitResponse {
    pub result: String,
    pub address: String,
    pub balance: BigDecimal,
    pub unspendable_balance: BigDecimal,
    pub coin: String,
    pub required_confirmations: u64,
    pub requires_notarization: bool,
    pub mature_confirmations: Option<u32>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename = "set_required_confirmations")]
pub struct SetRequiredConfRequest {
    pub coin: String,
    pub confirmations: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename = "set_requires_notarization")]
pub struct SetRequiredNotaRequest {
    pub coin: String,
    pub requires_notarization: bool,
}
