use mm2_number::BigDecimal;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub(crate) mod storage;

pub const MAX_ACCOUNT_NAME_LENGTH: usize = 255;
pub const MAX_ACCOUNT_DESCRIPTION_LENGTH: usize = 600;
pub const MAX_COIN_LENGTH: usize = 255;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) enum AccountType {
    Iguana,
    HD,
    HW,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type")]
pub enum AccountId {
    Iguana,
    HD { account_idx: u32 },
    HW { account_idx: u32 },
}

#[derive(Serialize)]
pub struct AccountInfo {
    pub(crate) account_id: AccountId,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) balance_usd: BigDecimal,
}

#[derive(Serialize)]
pub struct AccountWithEnabledFlag {
    #[serde(flatten)]
    account_info: AccountInfo,
    /// Whether this account is enabled.
    /// This flag is expected to be `true` for one account **only**.
    enabled: bool,
}

#[derive(Serialize)]
pub struct AccountWithCoins {
    #[serde(flatten)]
    account_info: AccountInfo,
    coins: HashSet<String>,
}
