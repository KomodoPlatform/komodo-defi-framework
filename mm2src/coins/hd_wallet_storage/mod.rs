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
pub use hd_wallet_wasm_storage::HDWalletDb;
#[cfg(target_arch = "wasm32")]
use hd_wallet_wasm_storage::HDWalletIndexedDbStorage as HDWalletStorageInstance;

pub type HDWalletStorageResult<T> = MmResult<T, HDWalletStorageError>;

#[derive(Display)]
pub enum HDWalletStorageError {
    // TODO consider renaming
    #[display(fmt = "HD wallet not allowed")]
    HDWalletNotAllowed,
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

pub struct HDAccountInfo {
    pub account_id: u32,
    pub account_derivation_path: String,
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

    async fn load_accounts(&self, wallet_id: HDWalletId) -> HDWalletStorageResult<Vec<HDAccountInfo>>;

    async fn load_account(
        &self,
        wallet_id: HDWalletId,
        account_id: u32,
    ) -> HDWalletStorageResult<Option<HDAccountInfo>>;

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

    async fn upload_new_account(&self, wallet_id: HDWalletId, account: HDAccountInfo) -> HDWalletStorageResult<()>;
}

#[async_trait]
pub trait HDWalletCoinWithStorageOps: HDWalletCoinOps {
    fn hd_wallet_storage(&self, hd_wallet: &Self::HDWallet) -> HDWalletCoinStorage;

    async fn load_all_accounts(&self, hd_wallet: &Self::HDWallet) -> HDWalletStorageResult<Vec<HDAccountInfo>> {
        let storage = self.hd_wallet_storage(hd_wallet);
        let wallet_id = storage.wallet_id();
        storage.inner.load_accounts(wallet_id).await
    }

    async fn load_account(
        &self,
        hd_wallet: &Self::HDWallet,
        account_id: u32,
    ) -> HDWalletStorageResult<Option<HDAccountInfo>> {
        let storage = self.hd_wallet_storage(hd_wallet);
        let wallet_id = storage.wallet_id();
        storage.inner.load_account(wallet_id, account_id).await
    }

    async fn update_external_addresses_number(
        &self,
        hd_wallet: &Self::HDWallet,
        account_id: u32,
        new_external_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        let storage = self.hd_wallet_storage(hd_wallet);
        let wallet_id = storage.wallet_id();
        storage
            .inner
            .update_external_addresses_number(wallet_id, account_id, new_external_addresses_number)
            .await
    }

    async fn update_internal_addresses_number(
        &self,
        hd_wallet: &Self::HDWallet,
        account_id: u32,
        new_internal_addresses_number: u32,
    ) -> HDWalletStorageResult<()> {
        let storage = self.hd_wallet_storage(hd_wallet);
        let wallet_id = storage.wallet_id();
        storage
            .inner
            .update_internal_addresses_number(wallet_id, account_id, new_internal_addresses_number)
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
        let wallet_id = storage.wallet_id();
        storage
            .inner
            .update_addresses_numbers(
                wallet_id,
                account_id,
                new_external_addresses_number,
                new_internal_addresses_number,
            )
            .await
    }

    async fn upload_new_account(
        &self,
        hd_wallet: &Self::HDWallet,
        account_info: HDAccountInfo,
    ) -> HDWalletStorageResult<()> {
        let storage = self.hd_wallet_storage(hd_wallet);
        let wallet_id = storage.wallet_id();
        storage.inner.upload_new_account(wallet_id, account_info).await
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
            .or_mm_err(|| HDWalletStorageError::HDWalletNotAllowed)?;
        Ok(HDWalletCoinStorage {
            coin,
            mm2_rmd160: *ctx.rmd160(),
            hd_wallet_rmd160,
            inner,
        })
    }

    pub fn wallet_id(&self) -> HDWalletId { HDWalletId::new(&self.coin, &self.mm2_rmd160, &self.hd_wallet_rmd160) }
}

fn display_rmd160(rmd160: &H160) -> String { hex::encode(rmd160.deref()) }
