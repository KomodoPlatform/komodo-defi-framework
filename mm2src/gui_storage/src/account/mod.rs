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

/// An enable account type.
/// We should not allow the user to enable [`AccountType::HW`].
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) enum EnabledAccountType {
    Iguana,
    HD,
}

impl From<EnabledAccountType> for AccountType {
    fn from(t: EnabledAccountType) -> Self {
        match t {
            EnabledAccountType::Iguana => AccountType::Iguana,
            EnabledAccountType::HD => AccountType::HD,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type")]
pub enum AccountId {
    Iguana,
    HD { account_idx: u32 },
    HW { account_idx: u32 },
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum EnabledAccountId {
    Iguana,
    HD { account_idx: u32 },
}

impl From<EnabledAccountId> for AccountId {
    fn from(id: EnabledAccountId) -> Self {
        match id {
            EnabledAccountId::Iguana => AccountId::Iguana,
            EnabledAccountId::HD { account_idx } => AccountId::HD { account_idx },
        }
    }
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
