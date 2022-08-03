use crate::hd_pubkey::HDXPubExtractor;
use crate::hd_wallet_storage::HDWalletStorageError;
use crate::{BalanceError, WithdrawError};
use async_trait::async_trait;
use crypto::{Bip32DerPathError, Bip32Error, Bip44Chain, Bip44DerPathError, Bip44DerivationPath, ChildNumber,
             DerivationPath, HwError};
use derive_more::Display;
use mm2_err_handle::prelude::*;
use rpc_task::RpcTaskError;
use serde::Serialize;
use std::collections::BTreeMap;

pub use futures::lock::{MappedMutexGuard as AsyncMappedMutexGuard, Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

pub type HDAccountsMap<HDAccount> = BTreeMap<u32, HDAccount>;
pub type HDAccountsMutex<HDAccount> = AsyncMutex<HDAccountsMap<HDAccount>>;
pub type HDAccountsMut<'a, HDAccount> = AsyncMutexGuard<'a, HDAccountsMap<HDAccount>>;
pub type HDAccountMut<'a, HDAccount> = AsyncMappedMutexGuard<'a, HDAccountsMap<HDAccount>, HDAccount>;

const DEFAULT_ADDRESS_LIMIT: u32 = ChildNumber::HARDENED_FLAG;
const DEFAULT_ACCOUNT_LIMIT: u32 = ChildNumber::HARDENED_FLAG;

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
        let max_addresses_number = hd_wallet.address_limit();
        if new_address_id >= max_addresses_number {
            return MmError::err(NewAddressDerivingError::AddressLimitReached { max_addresses_number });
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

    /// Returns limit on the number of addresses.
    fn address_limit(&self) -> u32 { DEFAULT_ADDRESS_LIMIT }

    /// Returns limit on the number of accounts.
    fn account_limit(&self) -> u32 { DEFAULT_ACCOUNT_LIMIT }

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

    async fn remove_account_if_last(&self, account_id: u32) -> Option<Self::HDAccount> {
        let mut x = self.get_accounts_mutex().lock().await;
        // `BTreeMap::last_entry` is still unstable.
        let (last_account_id, _) = x.iter().last()?;
        if *last_account_id == account_id {
            x.remove(&account_id)
        } else {
            None
        }
    }
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
