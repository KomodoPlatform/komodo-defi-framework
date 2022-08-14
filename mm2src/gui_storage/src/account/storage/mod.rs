use crate::account::{AccountId, AccountInfo, AccountType, AccountWithCoins, AccountWithEnabledFlag, EnabledAccountId,
                     EnabledAccountType};
use async_trait::async_trait;
use derive_more::Display;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use std::collections::BTreeMap;
use std::error::Error as StdError;

#[cfg(not(target_arch = "wasm32"))] mod sqlite_storage;
#[cfg(target_arch = "wasm32")] mod wasm_storage;

pub(crate) type AccountStorageBoxed = Box<dyn AccountStorage + Send + Sync>;
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
    pub(crate) fn to_pair(&self) -> (AccountType, Option<u32>) {
        match self {
            AccountId::Iguana => (AccountType::Iguana, None),
            AccountId::HD { account_idx } => (AccountType::HD, Some(*account_idx)),
            AccountId::HW { account_idx } => (AccountType::HW, Some(*account_idx)),
        }
    }

    /// Tries to construct `AccountId` from the pair.
    pub(crate) fn try_from_pair(
        account_type: AccountType,
        account_idx: Option<u32>,
    ) -> AccountStorageResult<AccountId> {
        match (account_type, account_idx) {
            (AccountType::Iguana, None) => Ok(AccountId::Iguana),
            (AccountType::Iguana, Some(_)) => {
                let error = "'account_id' should be None if 'account_type' is Iguana".to_owned();
                MmError::err(AccountStorageError::ErrorDeserializing(error))
            },
            (AccountType::HD, Some(account_idx)) => Ok(AccountId::HD { account_idx }),
            (AccountType::HW, Some(account_idx)) => Ok(AccountId::HW { account_idx }),
            (_, None) => {
                let error = "'account_idx' should be Some(id) if 'account_type' is HD or HW".to_owned();
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
            (EnabledAccountType::Iguana, Some(_)) => {
                let error = "'account_id' should be None if 'account_type' is Iguana".to_owned();
                MmError::err(AccountStorageError::ErrorDeserializing(error))
            },
            (EnabledAccountType::HD, Some(account_idx)) => Ok(EnabledAccountId::HD { account_idx }),
            (EnabledAccountType::HD, None) => {
                let error = "'account_idx' should be Some(id) if 'account_type' is HD".to_owned();
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
    pub(crate) fn new(ctx: &'a MmArc) -> Self { AccountStorageBuilder { ctx } }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn build(self) -> AccountStorageBoxed { Box::new(wasm_storage::WasmAccountStorage::new(self.ctx)) }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn build(self) -> AccountStorageBoxed { todo!() }
}

/// An account storage interface.
#[async_trait]
pub(crate) trait AccountStorage {
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
