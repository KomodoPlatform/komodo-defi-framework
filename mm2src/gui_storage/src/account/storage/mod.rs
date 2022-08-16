use crate::account::{AccountId, AccountInfo, AccountType, AccountWithCoins, AccountWithEnabledFlag, EnabledAccountId,
                     EnabledAccountType, HwPubkey};
use async_trait::async_trait;
use derive_more::Display;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use std::collections::BTreeMap;
use std::error::Error as StdError;

#[cfg(any(test, target_arch = "wasm32"))]
mod account_storage_tests;
#[cfg(not(target_arch = "wasm32"))] mod sqlite_storage;
#[cfg(target_arch = "wasm32")] mod wasm_storage;

pub(crate) type AccountStorageBoxed = Box<dyn AccountStorage>;
pub type AccountStorageResult<T> = MmResult<T, AccountStorageError>;

#[derive(Debug, Display)]
pub enum AccountStorageError {
    #[display(fmt = "No such account {:?}", _0)]
    NoSuchAccount(AccountId),
    #[display(fmt = "No enabled account yet")]
    NoEnabledAccount,
    #[display(fmt = "Account {:?} exists already", _0)]
    AccountExistsAlready(AccountId),
    #[display(fmt = "Error saving changes in accounts storage: {}", _0)]
    ErrorSaving(String),
    #[display(fmt = "Error loading account: {}", _0)]
    ErrorLoading(String),
    #[display(fmt = "Error deserializing an account: {}", _0)]
    ErrorDeserializing(String),
    #[display(fmt = "Error serializing an account: {}", _0)]
    ErrorSerializing(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl StdError for AccountStorageError {}

impl AccountStorageError {
    pub(crate) fn unknown_account_in_enabled_table(account_id: AccountId) -> AccountStorageError {
        let error = format!(
            "'EnabledAccountTable' contains an account {:?} that is not in 'AccountTable'",
            account_id
        );
        AccountStorageError::Internal(error)
    }
}

impl AccountId {
    /// Splits `AccountId` to the pair.
    pub(crate) fn to_tuple(&self) -> (AccountType, Option<u32>, Option<HwPubkey>) {
        match self {
            AccountId::Iguana => (AccountType::Iguana, None, None),
            AccountId::HD { account_idx } => (AccountType::HD, Some(*account_idx), None),
            AccountId::HW { device_pubkey } => (AccountType::HW, None, Some(device_pubkey.clone())),
        }
    }

    /// Tries to construct `AccountId` from the pair.
    pub(crate) fn try_from_tuple(
        account_type: AccountType,
        account_idx: Option<u32>,
        device_pubkey: Option<HwPubkey>,
    ) -> AccountStorageResult<AccountId> {
        match (account_type, account_idx, device_pubkey) {
            (AccountType::Iguana, None, None) => Ok(AccountId::Iguana),
            (AccountType::HD, Some(account_idx), None) => Ok(AccountId::HD { account_idx }),
            (AccountType::HW, None, Some(device_pubkey)) => Ok(AccountId::HW { device_pubkey }),
            (account_type, account_idx, device_pubkey) => {
                let error = format!(
                    "An invalid AccountId tuple: {:?}/{:?}/{:?}",
                    account_type, account_idx, device_pubkey
                );
                MmError::err(AccountStorageError::ErrorDeserializing(error))
            },
        }
    }
}

impl EnabledAccountId {
    pub(crate) fn to_pair(&self) -> (EnabledAccountType, Option<u32>) {
        match self {
            EnabledAccountId::Iguana => (EnabledAccountType::Iguana, None),
            EnabledAccountId::HD { account_idx } => (EnabledAccountType::Iguana, Some(*account_idx)),
        }
    }

    pub(crate) fn try_from_pair(
        account_type: EnabledAccountType,
        account_idx: Option<u32>,
    ) -> AccountStorageResult<EnabledAccountId> {
        match (account_type, account_idx) {
            (EnabledAccountType::Iguana, None) => Ok(EnabledAccountId::Iguana),
            (EnabledAccountType::HD, Some(account_idx)) => Ok(EnabledAccountId::HD { account_idx }),
            (account_type, account_idx) => {
                let error = format!("An invalid AccountId tuple: {:?}/{:?}", account_type, account_idx);
                MmError::err(AccountStorageError::ErrorDeserializing(error))
            },
        }
    }
}

/// `AccountStorageBoxed` builder.
/// The implementation depends on the target architecture.
pub(crate) struct AccountStorageBuilder<'a> {
    ctx: &'a MmArc,
}

impl<'a> AccountStorageBuilder<'a> {
    pub fn new(ctx: &'a MmArc) -> Self { AccountStorageBuilder { ctx } }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn build(self) -> AccountStorageResult<AccountStorageBoxed> {
        sqlite_storage::SqliteAccountStorage::new(self.ctx).map(|storage| -> AccountStorageBoxed { Box::new(storage) })
    }

    #[cfg(target_arch = "wasm32")]
    pub fn build(self) -> AccountStorageResult<AccountStorageBoxed> {
        Ok(Box::new(wasm_storage::WasmAccountStorage::new(self.ctx)))
    }
}

/// An account storage interface.
#[async_trait]
pub(crate) trait AccountStorage: Send + Sync {
    /// Initialize the storage.
    async fn init(&self) -> AccountStorageResult<()>;

    /// Loads accounts from the storage and marks **only** one account as enabled.
    async fn load_accounts_with_enabled_flag(
        &self,
    ) -> AccountStorageResult<BTreeMap<AccountId, AccountWithEnabledFlag>>;

    /// Loads an enabled account ID, or returns an error if there is no enabled account yet.
    async fn load_enabled_account_id(&self) -> AccountStorageResult<EnabledAccountId>;

    /// Loads an enabled account with activated coins, or returns an error if there is no enabled account yet.
    async fn load_enabled_account_with_coins(&self) -> AccountStorageResult<AccountWithCoins>;

    /// Checks whether the given account exists in the storage and sets it as an enabled account.
    async fn enable_account(&self, account_id: EnabledAccountId) -> AccountStorageResult<()>;

    /// Checks whether the given account doesn't exist in the storage and uploads it.
    async fn upload_account(&self, account: AccountInfo) -> AccountStorageResult<()>;

    /// Sets the account name.
    async fn set_name(&self, account_id: AccountId, name: String) -> AccountStorageResult<()>;

    /// Sets the account description.
    async fn set_description(&self, account_id: AccountId, description: String) -> AccountStorageResult<()>;

    /// Sets the account balance.
    async fn set_balance(&self, account_id: AccountId, balance_usd: BigDecimal) -> AccountStorageResult<()>;

    /// Puts the given `ticker` coin to the account's activated coins in the storage.
    async fn activate_coin(&self, account_id: AccountId, ticker: String) -> AccountStorageResult<()>;

    /// Erases the given `ticker` coin from the account's activated coins in the storage.
    async fn deactivate_coin(&self, account_id: AccountId, ticker: &str) -> AccountStorageResult<()>;
}
