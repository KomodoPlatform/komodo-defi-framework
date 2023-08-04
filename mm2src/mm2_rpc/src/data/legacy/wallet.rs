use serde::{Deserialize, Serialize};

use mm2_number::BigDecimal;

#[derive(Serialize, Deserialize)]
pub struct BalanceResponse {
    pub coin: String,
    pub balance: BigDecimal,
    pub unspendable_balance: BigDecimal,
    pub address: String,
}
