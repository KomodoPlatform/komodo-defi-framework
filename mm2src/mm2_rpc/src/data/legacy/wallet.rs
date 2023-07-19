use serde::{Deserialize, Serialize};

use mm2_number::BigDecimal;

#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename = "my_balance")]
pub struct BalanceRequest {
    pub coin: String,
}

#[derive(Serialize, Deserialize)]
pub struct BalanceResponse {
    pub coin: String,
    pub balance: BigDecimal,
    pub unspendable_balance: BigDecimal,
    pub address: String,
}
