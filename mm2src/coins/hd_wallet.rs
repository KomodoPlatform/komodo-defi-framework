use crate::coin_balance::HDAddressBalance;
use crate::hd_pubkey::HDXPubExtractor;
use crate::hd_wallet_storage::HDWalletStorageError;
use crate::{lp_coinfind_or_err, BalanceError, CoinFindError, CoinWithDerivationMethod, MmCoinEnum,
            UnexpectedDerivationMethod, WithdrawError};
use async_trait::async_trait;
use common::HttpStatusCode;
use crypto::{Bip32DerPathError, Bip32Error, Bip44Chain, Bip44DerPathError, Bip44DerivationPath, ChildNumber,
             DerivationPath, HwError};
use derive_more::Display;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use rpc_task::RpcTaskError;
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::Duration;

pub use futures::lock::{MappedMutexGuard as AsyncMappedMutexGuard, Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

pub type HDAccountsMap<HDAccount> = BTreeMap<u32, HDAccount>;
pub type HDAccountsMutex<HDAccount> = AsyncMutex<HDAccountsMap<HDAccount>>;
pub type HDAccountsMut<'a, HDAccount> = AsyncMutexGuard<'a, HDAccountsMap<HDAccount>>;
pub type HDAccountMut<'a, HDAccount> = AsyncMappedMutexGuard<'a, HDAccountsMap<HDAccount>, HDAccount>;

#[derive(Display)]
pub enum AddressDerivingError {
    #[display(fmt = "BIP32 address deriving error: {}", _0)]
    Bip32Error(Bip32Error),
}

impl From<Bip32Error> for AddressDerivingError {
    fn from(e: Bip32Error) -> Self { AddressDerivingError::Bip32Error(e) }
}

impl From<AddressDerivingError> for BalanceError {
    fn from(e: AddressDerivingError) -> Self {
        match e {
            AddressDerivingError::Bip32Error(bip32) => BalanceError::Internal(bip32.to_string()),
        }
    }
}

impl From<AddressDerivingError> for WithdrawError {
    fn from(e: AddressDerivingError) -> Self { WithdrawError::UnexpectedFromAddress(e.to_string()) }
}

pub enum NewAddressDerivingError {
    AddressLimitReached { max_addresses_number: u32 },
    InvalidBip44Chain { chain: Bip44Chain },
    Bip32Error(Bip32Error),
    WalletStorageError(HDWalletStorageError),
}

impl From<Bip32Error> for NewAddressDerivingError {
    fn from(e: Bip32Error) -> Self { NewAddressDerivingError::Bip32Error(e) }
}

impl From<AddressDerivingError> for NewAddressDerivingError {
    fn from(e: AddressDerivingError) -> Self {
        match e {
            AddressDerivingError::Bip32Error(bip32) => NewAddressDerivingError::Bip32Error(bip32),
        }
    }
}

impl From<InvalidBip44ChainError> for NewAddressDerivingError {
    fn from(e: InvalidBip44ChainError) -> Self { NewAddressDerivingError::InvalidBip44Chain { chain: e.chain } }
}

impl From<AccountUpdatingError> for NewAddressDerivingError {
    fn from(e: AccountUpdatingError) -> Self {
        match e {
            AccountUpdatingError::AddressLimitReached { max_addresses_number } => {
                NewAddressDerivingError::AddressLimitReached { max_addresses_number }
            },
            AccountUpdatingError::InvalidBip44Chain(e) => NewAddressDerivingError::from(e),
            AccountUpdatingError::WalletStorageError(storage) => NewAddressDerivingError::WalletStorageError(storage),
        }
    }
}

#[derive(Display)]
pub enum NewAccountCreatingError {
    #[display(fmt = "Hardware Wallet context is not initialized")]
    HwContextNotInitialized,
    #[display(fmt = "HD wallet is unavailable")]
    HDWalletUnavailable,
    #[display(
        fmt = "Coin doesn't support Trezor hardware wallet. Please consider adding the 'trezor_coin' field to the coins config"
    )]
    CoinDoesntSupportTrezor,
    RpcTaskError(RpcTaskError),
    HardwareWalletError(HwError),
    #[display(fmt = "Accounts limit reached. Max number of accounts: {}", max_accounts_number)]
    AccountLimitReached {
        max_accounts_number: u32,
    },
    #[display(fmt = "Error saving HD account to storage: {}", _0)]
    ErrorSavingAccountToStorage(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl From<Bip32DerPathError> for NewAccountCreatingError {
    fn from(e: Bip32DerPathError) -> Self { NewAccountCreatingError::Internal(Bip44DerPathError::from(e).to_string()) }
}

impl From<HDWalletStorageError> for NewAccountCreatingError {
    fn from(e: HDWalletStorageError) -> Self {
        match e {
            HDWalletStorageError::ErrorSaving(e) | HDWalletStorageError::ErrorSerializing(e) => {
                NewAccountCreatingError::ErrorSavingAccountToStorage(e)
            },
            HDWalletStorageError::HDWalletUnavailable => NewAccountCreatingError::HDWalletUnavailable,
            HDWalletStorageError::Internal(internal) => NewAccountCreatingError::Internal(internal),
            other => NewAccountCreatingError::Internal(other.to_string()),
        }
    }
}

impl From<NewAccountCreatingError> for HDWalletRpcError {
    fn from(e: NewAccountCreatingError) -> Self {
        match e {
            NewAccountCreatingError::HwContextNotInitialized => HDWalletRpcError::HwContextNotInitialized,
            NewAccountCreatingError::HDWalletUnavailable => HDWalletRpcError::CoinIsActivatedNotWithHDWallet,
            NewAccountCreatingError::CoinDoesntSupportTrezor => HDWalletRpcError::CoinDoesntSupportTrezor,
            NewAccountCreatingError::RpcTaskError(rpc) => HDWalletRpcError::from(rpc),
            NewAccountCreatingError::HardwareWalletError(hw) => HDWalletRpcError::from(hw),
            NewAccountCreatingError::AccountLimitReached { max_accounts_number } => {
                HDWalletRpcError::AccountLimitReached { max_accounts_number }
            },
            NewAccountCreatingError::ErrorSavingAccountToStorage(e) => {
                let error = format!("Error uploading HD account info to the storage: {}", e);
                HDWalletRpcError::WalletStorageError(error)
            },
            NewAccountCreatingError::Internal(internal) => HDWalletRpcError::Internal(internal),
        }
    }
}

/// Currently, we suppose that ETH/ERC20/QRC20 don't have [`Bip44Chain::Internal`] addresses.
#[derive(Display)]
#[display(fmt = "Coin doesn't support the given BIP44 chain: {:?}", chain)]
pub struct InvalidBip44ChainError {
    pub chain: Bip44Chain,
}

#[derive(Display)]
pub enum AccountUpdatingError {
    AddressLimitReached { max_addresses_number: u32 },
    InvalidBip44Chain(InvalidBip44ChainError),
    WalletStorageError(HDWalletStorageError),
}

impl From<InvalidBip44ChainError> for AccountUpdatingError {
    fn from(e: InvalidBip44ChainError) -> Self { AccountUpdatingError::InvalidBip44Chain(e) }
}

impl From<HDWalletStorageError> for AccountUpdatingError {
    fn from(e: HDWalletStorageError) -> Self { AccountUpdatingError::WalletStorageError(e) }
}

impl From<AccountUpdatingError> for BalanceError {
    fn from(e: AccountUpdatingError) -> Self {
        let error = e.to_string();
        match e {
            AccountUpdatingError::AddressLimitReached { .. } | AccountUpdatingError::InvalidBip44Chain(_) => {
                // Account updating is expected to be called after `address_id` and `chain` validation.
                BalanceError::Internal(format!("Unexpected internal error: {}", error))
            },
            AccountUpdatingError::WalletStorageError(_) => BalanceError::WalletStorageError(error),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum HDWalletRpcError {
    /* ----------- Trezor device errors ----------- */
    #[display(fmt = "Trezor device disconnected")]
    TrezorDisconnected,
    #[display(fmt = "Trezor internal error: {}", _0)]
    HardwareWalletInternal(String),
    #[display(fmt = "No Trezor device available")]
    NoTrezorDeviceAvailable,
    #[display(fmt = "Unexpected Hardware Wallet device: {}", _0)]
    FoundUnexpectedDevice(String),
    #[display(
        fmt = "Coin doesn't support Trezor hardware wallet. Please consider adding the 'trezor_coin' field to the coins config"
    )]
    CoinDoesntSupportTrezor,
    /* ----------- HD Wallet RPC error ------------ */
    #[display(fmt = "Hardware Wallet context is not initialized")]
    HwContextNotInitialized,
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(fmt = "RPC timed out {:?}", _0)]
    Timeout(Duration),
    #[display(fmt = "Coin is expected to be activated with the HD wallet derivation method")]
    CoinIsActivatedNotWithHDWallet,
    #[display(fmt = "HD account '{}' is not activated", account_id)]
    UnknownAccount { account_id: u32 },
    #[display(fmt = "Coin doesn't support the given BIP44 chain: {:?}", chain)]
    InvalidBip44Chain { chain: Bip44Chain },
    #[display(fmt = "Error deriving an address: {}", _0)]
    ErrorDerivingAddress(String),
    #[display(fmt = "Accounts limit reached. Max number of accounts: {}", max_accounts_number)]
    AccountLimitReached { max_accounts_number: u32 },
    #[display(fmt = "Addresses limit reached. Max number of addresses: {}", max_addresses_number)]
    AddressLimitReached { max_addresses_number: u32 },
    #[display(fmt = "Electrum/Native RPC invalid response: {}", _0)]
    RpcInvalidResponse(String),
    #[display(fmt = "HD wallet storage error: {}", _0)]
    WalletStorageError(String),
    #[display(fmt = "Transport: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
}

impl From<CoinFindError> for HDWalletRpcError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => HDWalletRpcError::NoSuchCoin { coin },
        }
    }
}

impl From<UnexpectedDerivationMethod> for HDWalletRpcError {
    fn from(e: UnexpectedDerivationMethod) -> Self {
        match e {
            UnexpectedDerivationMethod::HDWalletUnavailable => HDWalletRpcError::CoinIsActivatedNotWithHDWallet,
            unexpected_error => HDWalletRpcError::Internal(unexpected_error.to_string()),
        }
    }
}

impl From<BalanceError> for HDWalletRpcError {
    fn from(e: BalanceError) -> Self {
        match e {
            BalanceError::Transport(transport) => HDWalletRpcError::Transport(transport),
            BalanceError::InvalidResponse(rpc) => HDWalletRpcError::RpcInvalidResponse(rpc),
            BalanceError::UnexpectedDerivationMethod(der_path) => HDWalletRpcError::from(der_path),
            BalanceError::WalletStorageError(internal) | BalanceError::Internal(internal) => {
                HDWalletRpcError::Internal(internal)
            },
        }
    }
}

impl From<InvalidBip44ChainError> for HDWalletRpcError {
    fn from(e: InvalidBip44ChainError) -> Self { HDWalletRpcError::InvalidBip44Chain { chain: e.chain } }
}

impl From<AddressDerivingError> for HDWalletRpcError {
    fn from(e: AddressDerivingError) -> Self {
        match e {
            AddressDerivingError::Bip32Error(bip32) => HDWalletRpcError::ErrorDerivingAddress(bip32.to_string()),
        }
    }
}

impl From<NewAddressDerivingError> for HDWalletRpcError {
    fn from(e: NewAddressDerivingError) -> HDWalletRpcError {
        match e {
            NewAddressDerivingError::AddressLimitReached { max_addresses_number } => {
                HDWalletRpcError::AddressLimitReached { max_addresses_number }
            },
            NewAddressDerivingError::InvalidBip44Chain { chain } => HDWalletRpcError::InvalidBip44Chain { chain },
            NewAddressDerivingError::Bip32Error(bip32) => HDWalletRpcError::Internal(bip32.to_string()),
            NewAddressDerivingError::WalletStorageError(storage) => {
                HDWalletRpcError::WalletStorageError(storage.to_string())
            },
        }
    }
}

impl From<RpcTaskError> for HDWalletRpcError {
    fn from(e: RpcTaskError) -> Self {
        let error = e.to_string();
        match e {
            RpcTaskError::Canceled => HDWalletRpcError::Internal("Canceled".to_owned()),
            RpcTaskError::Timeout(timeout) => HDWalletRpcError::Timeout(timeout),
            RpcTaskError::NoSuchTask(_) | RpcTaskError::UnexpectedTaskStatus { .. } => {
                HDWalletRpcError::Internal(error)
            },
            RpcTaskError::Internal(internal) => HDWalletRpcError::Internal(internal),
        }
    }
}

impl From<HwError> for HDWalletRpcError {
    fn from(e: HwError) -> Self {
        let error = e.to_string();
        match e {
            HwError::NoTrezorDeviceAvailable => HDWalletRpcError::NoTrezorDeviceAvailable,
            HwError::FoundUnexpectedDevice { .. } => HDWalletRpcError::FoundUnexpectedDevice(error),
            _ => HDWalletRpcError::HardwareWalletInternal(error),
        }
    }
}

impl HttpStatusCode for HDWalletRpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            HDWalletRpcError::CoinDoesntSupportTrezor
            | HDWalletRpcError::HwContextNotInitialized
            | HDWalletRpcError::NoSuchCoin { .. }
            | HDWalletRpcError::CoinIsActivatedNotWithHDWallet
            | HDWalletRpcError::UnknownAccount { .. }
            | HDWalletRpcError::InvalidBip44Chain { .. }
            | HDWalletRpcError::ErrorDerivingAddress(_)
            | HDWalletRpcError::AddressLimitReached { .. }
            | HDWalletRpcError::AccountLimitReached { .. } => StatusCode::BAD_REQUEST,
            HDWalletRpcError::TrezorDisconnected
            | HDWalletRpcError::HardwareWalletInternal(_)
            | HDWalletRpcError::NoTrezorDeviceAvailable
            | HDWalletRpcError::FoundUnexpectedDevice(_)
            | HDWalletRpcError::Timeout(_)
            | HDWalletRpcError::Transport(_)
            | HDWalletRpcError::RpcInvalidResponse(_)
            | HDWalletRpcError::WalletStorageError(_)
            | HDWalletRpcError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub struct HDAddress<Address, Pubkey> {
    pub address: Address,
    pub pubkey: Pubkey,
    pub derivation_path: DerivationPath,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct HDAddressId {
    pub account_id: u32,
    pub chain: Bip44Chain,
    pub address_id: u32,
}

impl From<Bip44DerivationPath> for HDAddressId {
    fn from(der_path: Bip44DerivationPath) -> Self {
        HDAddressId {
            account_id: der_path.account_id(),
            chain: der_path.chain(),
            address_id: der_path.address_id(),
        }
    }
}

#[async_trait]
pub trait HDWalletCoinOps {
    type Address: Send + Sync;
    type Pubkey: Send;
    type HDWallet: HDWalletOps<HDAccount = Self::HDAccount>;
    type HDAccount: HDAccountOps;

    /// Derives an address from the given info.
    fn derive_address(
        &self,
        hd_account: &Self::HDAccount,
        chain: Bip44Chain,
        address_id: u32,
    ) -> MmResult<HDAddress<Self::Address, Self::Pubkey>, AddressDerivingError>;

    /// Generates a new address and updates the corresponding number of used `hd_account` addresses.
    async fn generate_new_address(
        &self,
        hd_wallet: &Self::HDWallet,
        hd_account: &mut Self::HDAccount,
        chain: Bip44Chain,
    ) -> MmResult<HDAddress<Self::Address, Self::Pubkey>, NewAddressDerivingError> {
        let known_addresses_number = hd_account.known_addresses_number(chain)?;
        // Address IDs start from 0, so the `known_addresses_number = last_known_address_id + 1`.
        let new_address_id = known_addresses_number;
        if new_address_id >= ChildNumber::HARDENED_FLAG {
            return MmError::err(NewAddressDerivingError::AddressLimitReached {
                max_addresses_number: ChildNumber::HARDENED_FLAG,
            });
        }
        let new_address = self
            .derive_address(hd_account, chain, new_address_id)
            .mm_err(NewAddressDerivingError::from)?;
        self.set_known_addresses_number(hd_wallet, hd_account, chain, known_addresses_number + 1)
            .await?;
        Ok(new_address)
    }

    /// Creates a new HD account, registers it within the given `hd_wallet`
    /// and returns a mutable reference to the registered account.
    async fn create_new_account<'a, XPubExtractor>(
        &self,
        hd_wallet: &'a Self::HDWallet,
        xpub_extractor: &XPubExtractor,
    ) -> MmResult<HDAccountMut<'a, Self::HDAccount>, NewAccountCreatingError>
    where
        XPubExtractor: HDXPubExtractor + Sync;

    async fn set_known_addresses_number(
        &self,
        hd_wallet: &Self::HDWallet,
        hd_account: &mut Self::HDAccount,
        chain: Bip44Chain,
        new_known_addresses_number: u32,
    ) -> MmResult<(), AccountUpdatingError>;
}

#[async_trait]
pub trait HDWalletOps: Send + Sync {
    type HDAccount: HDAccountOps + Clone + Send;

    fn coin_type(&self) -> u32;

    fn gap_limit(&self) -> u32;

    fn get_accounts_mutex(&self) -> &HDAccountsMutex<Self::HDAccount>;

    /// Returns a copy of an account by the given `account_id` if it's activated.
    async fn get_account(&self, account_id: u32) -> Option<Self::HDAccount> {
        let accounts = self.get_accounts_mutex().lock().await;
        accounts.get(&account_id).cloned()
    }

    /// Returns a mutable reference to an account by the given `account_id` if it's activated.
    async fn get_account_mut(&self, account_id: u32) -> Option<HDAccountMut<'_, Self::HDAccount>> {
        let accounts = self.get_accounts_mutex().lock().await;
        if !accounts.contains_key(&account_id) {
            return None;
        }

        Some(AsyncMutexGuard::map(accounts, |accounts| {
            accounts
                .get_mut(&account_id)
                .expect("getting an element should never fail due to the checks above")
        }))
    }

    /// Returns copies of all activated accounts.
    async fn get_accounts(&self) -> HDAccountsMap<Self::HDAccount> { self.get_accounts_mutex().lock().await.clone() }

    /// Returns a mutable reference to all activated accounts.
    async fn get_accounts_mut(&self) -> HDAccountsMut<'_, Self::HDAccount> { self.get_accounts_mutex().lock().await }
}

pub trait HDAccountOps: Send + Sync {
    /// Returns a number of used addresses of this account
    /// or an `InvalidBip44ChainError` error if the coin doesn't support the given `chain`.
    fn known_addresses_number(&self, chain: Bip44Chain) -> MmResult<u32, InvalidBip44ChainError>;

    /// Returns a derivation path of this account.
    fn account_derivation_path(&self) -> DerivationPath;

    /// Returns an index of this account.
    fn account_id(&self) -> u32;

    /// Returns true if the given address is known at this time.
    fn is_address_activated(&self, chain: Bip44Chain, address_id: u32) -> MmResult<bool, InvalidBip44ChainError> {
        let is_activated = address_id < self.known_addresses_number(chain)?;
        Ok(is_activated)
    }
}

#[derive(Deserialize)]
pub struct GetNewHDAddressRequest {
    coin: String,
    #[serde(flatten)]
    params: GetNewHDAddressParams,
}

#[derive(Deserialize)]
pub struct GetNewHDAddressParams {
    account_id: u32,
    chain: Bip44Chain,
}

#[derive(Serialize)]
pub struct GetNewHDAddressResponse {
    new_address: HDAddressBalance,
}

#[async_trait]
pub trait HDWalletRpcOps {
    async fn get_new_address_rpc(
        &self,
        params: GetNewHDAddressParams,
    ) -> MmResult<GetNewHDAddressResponse, HDWalletRpcError>;
}

pub async fn get_new_address(
    ctx: MmArc,
    req: GetNewHDAddressRequest,
) -> MmResult<GetNewHDAddressResponse, HDWalletRpcError> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    match coin {
        MmCoinEnum::UtxoCoin(utxo) => utxo.get_new_address_rpc(req.params).await,
        MmCoinEnum::QtumCoin(qtum) => qtum.get_new_address_rpc(req.params).await,
        _ => MmError::err(HDWalletRpcError::CoinIsActivatedNotWithHDWallet),
    }
}

pub mod common_impl {
    use super::*;
    use crate::coin_balance::HDWalletBalanceOps;
    use crate::MarketCoinOps;
    use crypto::RpcDerivationPath;
    use std::fmt;
    use std::ops::DerefMut;

    pub async fn get_new_address_rpc<Coin>(
        coin: &Coin,
        params: GetNewHDAddressParams,
    ) -> MmResult<GetNewHDAddressResponse, HDWalletRpcError>
    where
        Coin: HDWalletBalanceOps
            + CoinWithDerivationMethod<HDWallet = <Coin as HDWalletCoinOps>::HDWallet>
            + MarketCoinOps
            + Sync
            + Send,
        <Coin as HDWalletCoinOps>::Address: fmt::Display,
    {
        let account_id = params.account_id;
        let chain = params.chain;

        let hd_wallet = coin.derivation_method().hd_wallet_or_err()?;
        let mut hd_account = hd_wallet
            .get_account_mut(params.account_id)
            .await
            .or_mm_err(|| HDWalletRpcError::UnknownAccount { account_id })?;

        let HDAddress {
            address,
            derivation_path,
            ..
        } = coin
            .generate_new_address(hd_wallet, hd_account.deref_mut(), chain)
            .await?;
        let balance = coin.known_address_balance(&address).await?;

        Ok(GetNewHDAddressResponse {
            new_address: HDAddressBalance {
                address: address.to_string(),
                derivation_path: RpcDerivationPath(derivation_path),
                chain,
                balance,
            },
        })
    }
}
