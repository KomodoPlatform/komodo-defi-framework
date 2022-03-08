use crate::hd_wallet::HDWalletCoinOps;
use async_trait::async_trait;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use crypto::{CryptoCtx, CryptoInitError, XPub};
use derive_more::Display;
use primitives::hash::H160;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fmt::Formatter;
use std::ops::Deref;

#[cfg(not(target_arch = "wasm32"))] mod hd_wallet_sqlite_storage;
#[cfg(not(target_arch = "wasm32"))]
use hd_wallet_sqlite_storage::HDWalletSqliteStorage as HDWalletStorageInstance;
// #[cfg(not(target_arch = "wasm32"))] pub use hw_wallet_sqlite_storage::

#[cfg(target_arch = "wasm32")] mod hd_wallet_wasm_storage;
#[cfg(target_arch = "wasm32")]
use hd_wallet_wasm_storage::HDWalletIndexedDbStorage as HDWalletStorageInstance;
#[cfg(target_arch = "wasm32")]
pub use hd_wallet_wasm_storage::{HDWalletDb, HDWalletDbLocked};

pub type HDWalletStorageResult<T> = MmResult<T, HDWalletStorageError>;

#[derive(Debug, Display)]
pub enum HDWalletStorageError {
    #[display(fmt = "HD wallet not allowed")]
    HDWalletUnavailable,
    #[display(fmt = "HD account '{:?}':{} not found", wallet_id, account_id)]
    HDAccountNotFound { wallet_id: HDWalletId, account_id: u32 },
    #[display(fmt = "Error saving the a swap: {}", _0)]
    ErrorSaving(String),
    #[display(fmt = "Error loading a swap: {}", _0)]
    ErrorLoading(String),
    #[display(fmt = "Error deserializing a swap: {}", _0)]
    ErrorDeserializing(String),
    #[display(fmt = "Error serializing a swap: {}", _0)]
    ErrorSerializing(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl From<CryptoInitError> for HDWalletStorageError {
    fn from(e: CryptoInitError) -> Self { HDWalletStorageError::Internal(e.to_string()) }
}

impl HDWalletStorageError {
    pub fn is_deserializing_err(&self) -> bool { matches!(self, HDWalletStorageError::ErrorDeserializing(_)) }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HDWalletId(String);

impl HDWalletId {
    /// `mm2_rmd160` is RIPEMD160(SHA256(x)) where x is a pubkey with which mm2 is launched.
    /// It's expected to be equal to [`MmCtx::rmd160`].
    /// This property allows us to store DB items that are unique to each user (passphrase).
    ///
    /// `hd_wallet_rmd160` is RIPEMD160(SHA256(x)) where x is a pubkey extracted from a Hardware Wallet device or passphrase.
    /// This property allows us to store DB items that are unique to each Hardware Wallet device.
    /// Please note it can be equal to [`HDWalletId::mm2_rmd160`] if mm2 is launched with a HD private key derived from a passphrase.
    pub fn new(ticker: &str, mm2_rmd160: &H160, hd_wallet_rmd160: &H160) -> HDWalletId {
        HDWalletId(format!(
            "{}_{}_{}",
            ticker,
            display_rmd160(mm2_rmd160),
            display_rmd160(hd_wallet_rmd160)
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HDAccountStorageItem {
    pub account_id: u32,
    pub account_xpub: XPub,
    /// The number of addresses that we know have been used by the user.
    pub external_addresses_number: u32,
    pub internal_addresses_number: u32,
}

#[async_trait]
pub trait HDWalletStorageInternalOps {
    fn new(ctx: &MmArc) -> HDWalletStorageResult<Self>
    where
        Self: Sized;

    async fn load_accounts(&self, wallet_id: HDWalletId) -> HDWalletStorageResult<Vec<HDAccountStorageItem>>;

    async fn load_account(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
    ) -> HDWalletStorageResult<Option<HDAccountStorageItem>>;

    async fn update_external_addresses_number(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
        new_external_addresses_number: u32,
    ) -> HDWalletStorageResult<()>;

    async fn update_internal_addresses_number(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
        new_internal_addresses_number: u32,
    ) -> HDWalletStorageResult<()>;

    async fn update_addresses_numbers(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
        new_external_addresses_number: u32,
        new_internal_addresses_number: u32,
    ) -> HDWalletStorageResult<()>;

    async fn upload_new_account(
        &self,
        wallet_id: HDWalletId,
        account: HDAccountStorageItem,
    ) -> HDWalletStorageResult<()>;

    async fn clear_accounts(&self, wallet_id: HDWalletId) -> HDWalletStorageResult<()>;
}

#[async_trait]
pub trait HDWalletCoinWithStorageOps: HDWalletCoinOps {
    fn hd_wallet_storage<'a>(&self, hd_wallet: &'a Self::HDWallet) -> &'a HDWalletCoinStorage;

    async fn load_all_accounts(&self, hd_wallet: &Self::HDWallet) -> HDWalletStorageResult<Vec<HDAccountStorageItem>> {
        let storage = self.hd_wallet_storage(hd_wallet);
        storage.load_all_accounts().await
    }

    async fn load_account(
        &self,
        hd_wallet: &Self::HDWallet,
        account_id: u32,
    ) -> HDWalletStorageResult<Option<HDAccountStorageItem>> {
        let storage = self.hd_wallet_storage(hd_wallet);
        storage.load_account(account_id).await
    }

    async fn update_external_addresses_number(
        &self,
        hd_wallet: &Self::HDWallet,
        account_id: u32,
        new_external_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        let storage = self.hd_wallet_storage(hd_wallet);
        storage
            .update_external_addresses_number(account_id, new_external_addresses_number)
            .await
    }

    async fn update_internal_addresses_number(
        &self,
        hd_wallet: &Self::HDWallet,
        account_id: u32,
        new_internal_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        let storage = self.hd_wallet_storage(hd_wallet);
        storage
            .update_internal_addresses_number(account_id, new_internal_addresses_number)
            .await
    }

    async fn update_addresses_numbers(
        &self,
        hd_wallet: &Self::HDWallet,
        account_id: u32,
        new_external_addresses_number: u32,
        new_internal_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        let storage = self.hd_wallet_storage(hd_wallet);
        storage
            .update_addresses_numbers(account_id, new_external_addresses_number, new_internal_addresses_number)
            .await
    }

    async fn upload_new_account(
        &self,
        hd_wallet: &Self::HDWallet,
        account_info: HDAccountStorageItem,
    ) -> HDWalletStorageResult<()> {
        let storage = self.hd_wallet_storage(hd_wallet);
        storage.upload_new_account(account_info).await
    }

    async fn clear_accounts(&self, hd_wallet: &Self::HDWallet) -> HDWalletStorageResult<()> {
        let storage = self.hd_wallet_storage(hd_wallet);
        storage.clear_accounts().await
    }
}

/// The wrapper over the [`HDWalletStorage::inner`] database implementation.
/// It's associated with a specific mm2 user, HD wallet and coin.
pub struct HDWalletCoinStorage {
    coin: String,
    /// RIPEMD160(SHA256(x)) where x is a pubkey with which mm2 is launched.
    /// It's expected to be equal to [`MmCtx::rmd160`].
    /// This property allows us to store DB items that are unique to each user (passphrase).
    mm2_rmd160: H160,
    /// RIPEMD160(SHA256(x)) where x is a pubkey extracted from a Hardware Wallet device or passphrase.
    /// This property allows us to store DB items that are unique to each Hardware Wallet device.
    /// Please note it can be equal to [`HDWalletId::mm2_rmd160`] if mm2 is launched with a HD private key derived from a passphrase.
    hd_wallet_rmd160: H160,
    inner: HDWalletStorageInstance,
}

impl fmt::Debug for HDWalletCoinStorage {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("HDWalletCoinStorage")
            .field("coin", &self.coin)
            .field("mm2_rmd160", &self.mm2_rmd160)
            .field("hd_wallet_rmd160", &self.hd_wallet_rmd160)
            .finish()
    }
}

impl HDWalletCoinStorage {
    pub fn new(ctx: &MmArc, coin: String) -> HDWalletStorageResult<HDWalletCoinStorage> {
        let inner = HDWalletStorageInstance::new(ctx)?;
        let crypto_ctx = CryptoCtx::from_ctx(ctx)?;
        let hd_wallet_rmd160 = crypto_ctx
            .hd_wallet_rmd160()
            .or_mm_err(|| HDWalletStorageError::HDWalletUnavailable)?;
        Ok(HDWalletCoinStorage {
            coin,
            mm2_rmd160: *ctx.rmd160(),
            hd_wallet_rmd160,
            inner,
        })
    }

    #[cfg(any(test, target_arch = "wasm32"))]
    fn with_rmd160(
        ctx: &MmArc,
        coin: String,
        mm2_rmd160: H160,
        hd_wallet_rmd160: H160,
    ) -> HDWalletStorageResult<HDWalletCoinStorage> {
        let inner = HDWalletStorageInstance::new(ctx)?;
        Ok(HDWalletCoinStorage {
            coin,
            mm2_rmd160,
            hd_wallet_rmd160,
            inner,
        })
    }

    pub fn wallet_id(&self) -> HDWalletId { HDWalletId::new(&self.coin, &self.mm2_rmd160, &self.hd_wallet_rmd160) }

    pub async fn load_all_accounts(&self) -> HDWalletStorageResult<Vec<HDAccountStorageItem>> {
        let wallet_id = self.wallet_id();
        self.inner.load_accounts(wallet_id).await
    }

    async fn load_account(&self, account_id: u32) -> HDWalletStorageResult<Option<HDAccountStorageItem>> {
        let wallet_id = self.wallet_id();
        self.inner.load_account(wallet_id, account_id).await
    }

    async fn update_external_addresses_number(
        &self,
        account_id: u32,
        new_external_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        let wallet_id = self.wallet_id();
        self.inner
            .update_external_addresses_number(wallet_id, account_id, new_external_addresses_number)
            .await
    }

    async fn update_internal_addresses_number(
        &self,
        account_id: u32,
        new_internal_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        let wallet_id = self.wallet_id();
        self.inner
            .update_internal_addresses_number(wallet_id, account_id, new_internal_addresses_number)
            .await
    }

    async fn update_addresses_numbers(
        &self,
        account_id: u32,
        new_external_addresses_number: u32,
        new_internal_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        let wallet_id = self.wallet_id();
        self.inner
            .update_addresses_numbers(
                wallet_id,
                account_id,
                new_external_addresses_number,
                new_internal_addresses_number,
            )
            .await
    }

    async fn upload_new_account(&self, account_info: HDAccountStorageItem) -> HDWalletStorageResult<()> {
        let wallet_id = self.wallet_id();
        self.inner.upload_new_account(wallet_id, account_info).await
    }

    pub async fn clear_accounts(&self) -> HDWalletStorageResult<()> {
        let wallet_id = self.wallet_id();
        self.inner.clear_accounts(wallet_id).await
    }
}

fn display_rmd160(rmd160: &H160) -> String { hex::encode(rmd160.deref()) }
