use crate::account::storage::AccountStorageError;
use crate::account::{AccountId, AccountInfo, AccountWithEnabledFlag, EnabledAccountId, MAX_ACCOUNT_DESCRIPTION_LENGTH,
                     MAX_ACCOUNT_NAME_LENGTH, MAX_TICKER_LENGTH};
use crate::context::AccountContext;
use common::{HttpStatusCode, StatusCode, SuccessResponse};
use derive_more::Display;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::BigDecimal;
use ser_error_derive::SerializeErrorType;
use serde::{Deserialize, Serialize};

#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum AccountRpcError {
    #[display(fmt = "Account name is too long, expected shorter or equal to {}", max_len)]
    NameTooLong { max_len: usize },
    #[display(fmt = "Account description is too long, expected shorter or equal to {}", max_len)]
    DescriptionTooLong { max_len: usize },
    #[display(fmt = "Coin ticker is too long, expected shorter or equal to {}", max_len)]
    TickerTooLong { max_len: usize },
    #[display(fmt = "No such account {:?}", _0)]
    NoSuchAccount(AccountId),
    #[display(fmt = "No enabled account yet. Consider using 'enable_account' RPC")]
    NoEnabledAccount,
    #[display(fmt = "Account {:?} exists already", _0)]
    AccountExistsAlready(AccountId),
    #[display(fmt = "Error loading account: {}", _0)]
    ErrorLoadingAccount(String),
    #[display(fmt = "Error saving changes in accounts storage: {}", _0)]
    ErrorSavingAccount(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl From<AccountStorageError> for AccountRpcError {
    fn from(e: AccountStorageError) -> Self {
        match e {
            AccountStorageError::NoSuchAccount(account_id) => AccountRpcError::NoSuchAccount(account_id),
            AccountStorageError::NoEnabledAccount => AccountRpcError::NoEnabledAccount,
            AccountStorageError::AccountExistsAlready(account_id) => AccountRpcError::AccountExistsAlready(account_id),
            AccountStorageError::ErrorDeserializing(e) | AccountStorageError::ErrorLoading(e) => {
                AccountRpcError::ErrorLoadingAccount(e)
            },
            AccountStorageError::ErrorSaving(e) | AccountStorageError::ErrorSerializing(e) => {
                AccountRpcError::ErrorSavingAccount(e)
            },
            AccountStorageError::Internal(internal) => AccountRpcError::Internal(internal),
        }
    }
}

impl HttpStatusCode for AccountRpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            AccountRpcError::NameTooLong { .. }
            | AccountRpcError::DescriptionTooLong { .. }
            | AccountRpcError::TickerTooLong { .. }
            | AccountRpcError::NoSuchAccount(_)
            | AccountRpcError::NoEnabledAccount
            | AccountRpcError::AccountExistsAlready(_) => StatusCode::BAD_REQUEST,
            AccountRpcError::ErrorLoadingAccount(_)
            | AccountRpcError::ErrorSavingAccount(_)
            | AccountRpcError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Deserialize)]
pub struct NewAccount<Id> {
    account_id: Id,
    name: String,
    #[serde(default)]
    description: String,
}

impl<Id> From<NewAccount<Id>> for AccountInfo
where
    AccountId: From<Id>,
{
    fn from(orig: NewAccount<Id>) -> Self {
        AccountInfo {
            account_id: AccountId::from(orig.account_id),
            name: orig.name,
            description: orig.description,
            balance_usd: BigDecimal::default(),
        }
    }
}

#[derive(Deserialize)]
pub struct EnableAccountRequest {
    #[serde(flatten)]
    policy: EnableAccountPolicy,
}

#[derive(Deserialize)]
#[serde(tag = "policy")]
pub enum EnableAccountPolicy {
    Exist(EnabledAccountId),
    New(NewAccount<EnabledAccountId>),
}

#[derive(Deserialize)]
pub struct AddAccountRequest {
    #[serde(flatten)]
    account: NewAccount<AccountId>,
}

#[derive(Deserialize)]
pub struct SetAccountNameRequest {
    account_id: AccountId,
    name: String,
}

#[derive(Deserialize)]
pub struct SetAccountDescriptionRequest {
    account_id: AccountId,
    description: String,
}

#[derive(Deserialize)]
pub struct CoinRequest {
    account_id: AccountId,
    ticker: String,
}

#[derive(Deserialize)]
pub struct GetAccountsRequest;

// #[derive(Deserialize)]
// pub struct UpdateBalanceResponse {
//     ticker: String,
//     balance_usd: BigDecimal,
// }

/// Sets the given account as an enabled (current active account).
/// The behaviour depends on [`EnableAccountRequest::policy`]:
/// * [`EnableAccountPolicy::Known`] =>
///     1) Checks whether the given account exists in the storage.
///        Returns [`AccountRpcError::NoSuchAccount`] if there is no account with the given `AccountId`;
///     2) Sets the account as an enabled.
/// * [`EnableAccountPolicy::New`] =>
///     1) Tries to upload the given account info to the storage.
///        Returns [`AccountRpcError::AccountExistsAlready`] if there is an account with the same `AccountId` already;
///     2) Sets the account as an enabled.
///
/// # Important
///
/// This RPC affects the storage **only**. It doesn't affect MarketMaker.
pub async fn enable_account(ctx: MmArc, req: EnableAccountRequest) -> MmResult<SuccessResponse, AccountRpcError> {
    let account_ctx = AccountContext::from_ctx(&ctx).map_to_mm(AccountRpcError::Internal)?;
    let account_id = match req.policy {
        EnableAccountPolicy::Exist(account_id) => account_id,
        EnableAccountPolicy::New(new_account) => {
            let account_id = new_account.account_id.clone();
            account_ctx
                .storage
                .upload_account(AccountInfo::from(new_account))
                .await?;
            account_id
        },
    };
    account_ctx.storage.enable_account(account_id).await?;
    Ok(SuccessResponse::new())
}

/// Adds the given [`AddAccountRequest::account`] to the storage.
/// Returns [`AccountRpcError::AccountExistsAlready`] if there is an account with the same `AccountId` already.
///
/// # Important
///
/// This RPC affects the storage **only**. It doesn't affect MarketMaker.
pub async fn add_account(ctx: MmArc, req: AddAccountRequest) -> MmResult<SuccessResponse, AccountRpcError> {
    let account_ctx = AccountContext::from_ctx(&ctx).map_to_mm(AccountRpcError::Internal)?;
    validate_new_account(&req.account)?;
    account_ctx
        .storage
        .upload_account(AccountInfo::from(req.account))
        .await?;
    Ok(SuccessResponse::new())
}

/// Loads accounts from the storage and marks one account as enabled **only**.
/// If an account has not been enabled yet, this RPC returns [`AccountRpcError::NoEnabledAccount`] error.
///
/// # Note
///
/// The returned accounts are sorted by `AccountId`.
pub async fn get_accounts(
    ctx: MmArc,
    _req: GetAccountsRequest,
) -> MmResult<Vec<AccountWithEnabledFlag>, AccountRpcError> {
    let account_ctx = AccountContext::from_ctx(&ctx).map_to_mm(AccountRpcError::Internal)?;
    let accounts = account_ctx
        .storage
        .load_accounts_with_enabled_flag()
        .await?
        // The given `BTreeMap<AccountId, AccountWithEnabledFlag>` accounts are sorted by `AccountId`.
        .into_iter()
        .map(|(_account_id, account)| account)
        .collect();
    Ok(accounts)
}

/// Sets account name by its [`SetAccountNameRequest::account_id`].
pub async fn set_account_name(ctx: MmArc, req: SetAccountNameRequest) -> MmResult<SuccessResponse, AccountRpcError> {
    validate_account_name(&req.name)?;
    let account_ctx = AccountContext::from_ctx(&ctx).map_to_mm(AccountRpcError::Internal)?;
    account_ctx.storage.set_name(req.account_id, req.name).await?;
    Ok(SuccessResponse::new())
}

/// Sets account description by its [`SetAccountNameRequest::account_id`].
pub async fn set_account_description(
    ctx: MmArc,
    req: SetAccountDescriptionRequest,
) -> MmResult<SuccessResponse, AccountRpcError> {
    validate_account_desc(&req.description)?;
    let account_ctx = AccountContext::from_ctx(&ctx).map_to_mm(AccountRpcError::Internal)?;
    account_ctx
        .storage
        .set_description(req.account_id, req.description)
        .await?;
    Ok(SuccessResponse::new())
}

// /// Requests the balance of each activated coin, sums them up to get the total account USD balance,
// /// uploads to the storage, and returns as a result.
// ///
// /// # Important
// ///
// /// This RPC affects the storage **only**. It doesn't affect MarketMaker.
// pub async fn update_balance(ctx: MmArc, req: CoinRequest) -> MmResult<UpdateBalanceResponse, AccountRpcError> {
//     let account_ctx = AccountContext::from_ctx(&ctx).map_to_mm(AccountRpcError::Internal)?;
//     let balance_usd: BigDecimal = todo!();
//     account_ctx
//         .storage
//         .set_balance(req.account_id, balance_usd.clone())
//         .await?;
//     Ok(UpdateBalanceResponse {
//         ticker: req.ticker,
//         balance_usd,
//     })
// }

/// Activates the given [`CoinRequest::ticker`] for the specified [`CoinRequest::account_id`] account.
///
/// # Important
///
/// This RPC affects the storage **only**. It doesn't affect MarketMaker.
pub async fn activate_coin(ctx: MmArc, req: CoinRequest) -> MmResult<SuccessResponse, AccountRpcError> {
    let account_ctx = AccountContext::from_ctx(&ctx).map_to_mm(AccountRpcError::Internal)?;
    validate_ticker(&req.ticker)?;
    account_ctx.storage.activate_coin(req.account_id, req.ticker).await?;
    Ok(SuccessResponse::new())
}

/// Deactivates the given [`CoinRequest::ticker`] for the specified [`CoinRequest::account_id`] account.
///
/// # Important
///
/// This RPC affects the storage **only**. It doesn't affect MarketMaker.
pub async fn deactivate_coin(ctx: MmArc, req: CoinRequest) -> MmResult<SuccessResponse, AccountRpcError> {
    let account_ctx = AccountContext::from_ctx(&ctx).map_to_mm(AccountRpcError::Internal)?;
    account_ctx.storage.deactivate_coin(req.account_id, &req.ticker).await?;
    Ok(SuccessResponse::new())
}

fn validate_new_account<Id>(account: &NewAccount<Id>) -> MmResult<(), AccountRpcError> {
    validate_account_name(&account.name)?;
    validate_account_desc(&account.description)
}

fn validate_account_name(name: &str) -> MmResult<(), AccountRpcError> {
    if name.len() > MAX_ACCOUNT_NAME_LENGTH {
        return MmError::err(AccountRpcError::NameTooLong {
            max_len: MAX_ACCOUNT_NAME_LENGTH,
        });
    }
    Ok(())
}

fn validate_account_desc(description: &str) -> MmResult<(), AccountRpcError> {
    if description.len() > MAX_ACCOUNT_DESCRIPTION_LENGTH {
        return MmError::err(AccountRpcError::DescriptionTooLong {
            max_len: MAX_ACCOUNT_NAME_LENGTH,
        });
    }
    Ok(())
}

fn validate_ticker(coin: &str) -> MmResult<(), AccountRpcError> {
    if coin.len() > MAX_TICKER_LENGTH {
        return MmError::err(AccountRpcError::TickerTooLong {
            max_len: MAX_TICKER_LENGTH,
        });
    }
    Ok(())
}
