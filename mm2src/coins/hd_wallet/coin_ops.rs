use super::{inner_impl, AccountUpdatingError, AddressDerivingError, AddressDerivingResult, HDAccountMut, HDAccountOps,
            HDCoinHDAccount, HDCoinHDAddress, HDConfirmAddress, HDWalletOps, HDXPubExtractor, NewAccountCreatingError,
            NewAddressDeriveConfirmError, NewAddressDerivingError};
use crate::hd_wallet::{HDAddressOps, HDWalletStorageOps};
use async_trait::async_trait;
use bip32::{ChildNumber, DerivationPath};
use crypto::{Bip44Chain, Secp256k1ExtendedPublicKey};
use itertools::Itertools;
use mm2_err_handle::mm_error::{MmError, MmResult};
use std::collections::HashMap;

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct HDAddressId {
    pub chain: Bip44Chain,
    pub address_id: u32,
}

/// `HDWalletCoinOps` defines operations that coins should support to have HD wallet functionalities.
/// This trait outlines fundamental operations like address derivation, account creation, and more.
#[async_trait]
pub trait HDWalletCoinOps {
    /// The type representing the HD Wallet operations associated with this coin.
    type HDWallet: HDWalletOps + HDWalletStorageOps + Send + Sync;
    fn address_from_extended_pubkey(
        &self,
        extended_pubkey: &Secp256k1ExtendedPublicKey,
        derivation_path: DerivationPath,
    ) -> HDCoinHDAddress<Self>;

    /// Retrieves an HD address from the cache or derives it if it hasn't been derived yet.
    ///
    /// # Parameters
    /// - `hd_account`: The HD account from which the address will be derived.
    /// - `hd_addresses_cache`: The cache of derived addresses.
    /// - `hd_address_id`: The unique address identifier.
    ///
    /// # Returns
    /// A result containing the derived `HDCoinHDAddress<Self>` instance or an error.
    fn derive_address_with_cache(
        &self,
        hd_account: &HDCoinHDAccount<Self>,
        hd_addresses_cache: &mut HashMap<HDAddressId, HDCoinHDAddress<Self>>,
        hd_address_id: HDAddressId,
    ) -> AddressDerivingResult<HDCoinHDAddress<Self>> {
        // Check if the given HD address has been derived already.
        if let Some(hd_address) = hd_addresses_cache.get(&hd_address_id) {
            return Ok(hd_address.clone());
        }

        let change_child = hd_address_id.chain.to_child_number();
        let address_id_child = ChildNumber::from(hd_address_id.address_id);

        let derived_pubkey = hd_account
            .extended_pubkey()
            .derive_child(change_child)?
            .derive_child(address_id_child)?;

        let mut derivation_path = hd_account.account_derivation_path();
        derivation_path.push(change_child);
        derivation_path.push(address_id_child);
        drop_mutability!(derivation_path);
        let hd_address = self.address_from_extended_pubkey(&derived_pubkey, derivation_path);

        // Cache the derived `hd_address`.
        hd_addresses_cache.insert(hd_address_id, hd_address.clone());
        Ok(hd_address)
    }

    /// Derives a single HD address for a given account, chain, and address identifier.
    ///
    /// # Parameters
    /// - `hd_account`: The HD account from which the address will be derived.
    /// - `chain`: The Bip44 chain identifier.
    /// - `address_id`: The unique address identifier.
    ///
    /// # Returns
    /// A result containing the derived `HDCoinHDAddress<Self>` instance or an error.
    async fn derive_address(
        &self,
        hd_account: &HDCoinHDAccount<Self>,
        chain: Bip44Chain,
        address_id: u32,
    ) -> AddressDerivingResult<HDCoinHDAddress<Self>> {
        self.derive_addresses(hd_account, std::iter::once(HDAddressId { chain, address_id }))
            .await?
            .into_iter()
            .exactly_one()
            // Unfortunately, we can't use [`MapToMmResult::map_to_mm`] due to unsatisfied trait bounds,
            // and it's easier to use [`Result::map_err`] instead of adding more trait bounds to this method.
            .map_err(|e| MmError::new(AddressDerivingError::Internal(e.to_string())))
    }

    /// Derives a set of HD addresses for a coin using the specified HD account and address identifiers.
    ///
    /// # Parameters
    /// - `hd_account`: The HD account associated with the addresses.
    /// - `address_ids`: An iterator of `HDAddressId` items specifying which addresses to derive.
    ///
    /// # Returns
    /// A result containing a vector of derived `HDCoinHDAddress<Self>` instances or an error.
    #[cfg(not(target_arch = "wasm32"))]
    async fn derive_addresses<Ids>(
        &self,
        hd_account: &HDCoinHDAccount<Self>,
        address_ids: Ids,
    ) -> AddressDerivingResult<Vec<HDCoinHDAddress<Self>>>
    where
        Ids: Iterator<Item = HDAddressId> + Send,
    {
        let mut hd_addresses_cache_guard = hd_account.derived_addresses().lock().await;
        let hd_addresses_cache = &mut *hd_addresses_cache_guard;
        address_ids
            .map(|hd_address_id| self.derive_address_with_cache(hd_account, hd_addresses_cache, hd_address_id))
            .collect()
    }

    // Todo: combine both implementations once worker threads are supported in WASM
    /// [`HDWalletCoinOps::derive_addresses`] WASM implementation.
    ///
    /// # Important
    ///
    /// This function locks [`HDAddressesCache::cache`] mutex at each iteration.
    ///
    /// # Performance
    ///
    /// Locking the [`HDAddressesCache::cache`] mutex at each iteration may significantly degrade performance.
    /// But this is required at least for now due the facts that:
    /// 1) mm2 runs in the same thread as `KomodoPlatform/air_dex` runs;
    /// 2) [`ExtendedPublicKey::derive_child`] is a synchronous operation, and it takes a long time.
    /// So we need to periodically invoke Javascript runtime to handle UI events and other asynchronous tasks.
    #[cfg(target_arch = "wasm32")]
    async fn derive_addresses<Ids>(
        &self,
        hd_account: &HDCoinHDAccount<Self>,
        address_ids: Ids,
    ) -> AddressDerivingResult<Vec<HDCoinHDAddress<Self>>>
    where
        Ids: Iterator<Item = HDAddressId> + Send,
    {
        let mut result = Vec::new();
        for hd_address_id in address_ids {
            let mut hd_addresses_cache = hd_account.derived_addresses().lock().await;

            let hd_address = self.derive_address_with_cache(hd_account, &mut hd_addresses_cache, hd_address_id)?;
            result.push(hd_address);
        }

        Ok(result)
    }

    /// Derives known HD addresses for a specific account and chain.
    /// Essentially, this retrieves addresses that have been interacted with in the past.
    ///
    /// # Parameters
    /// - `hd_account`: The HD account from which to derive known addresses.
    /// - `chain`: The Bip44 chain identifier.
    ///
    /// # Returns
    /// A result containing a vector of previously generated `HDAddress<Self::Address, Self::Pubkey>` instances or an error.
    async fn derive_known_addresses(
        &self,
        hd_account: &HDCoinHDAccount<Self>,
        chain: Bip44Chain,
    ) -> AddressDerivingResult<Vec<HDCoinHDAddress<Self>>> {
        let known_addresses_number = hd_account.known_addresses_number(chain)?;
        let address_ids = (0..known_addresses_number)
            .into_iter()
            .map(|address_id| HDAddressId { chain, address_id });
        self.derive_addresses(hd_account, address_ids).await
    }

    /// Generates a new address for a coin and updates the corresponding number of used `hd_account` addresses.
    ///
    /// # Parameters
    /// - `hd_wallet`: The specified HD wallet from which the address will be derived.
    /// - `hd_account`: The mutable HD account.
    /// - `chain`: The Bip44 chain identifier.
    ///
    /// # Returns
    /// A result containing the generated `HDAddress<Self::Address, Self::Pubkey>` instance or an error.
    async fn generate_new_address(
        &self,
        hd_wallet: &Self::HDWallet,
        hd_account: &mut HDCoinHDAccount<Self>,
        chain: Bip44Chain,
    ) -> MmResult<HDCoinHDAddress<Self>, NewAddressDerivingError> {
        let inner_impl::NewAddress {
            hd_address: address,
            new_known_addresses_number,
        } = inner_impl::generate_new_address_immutable(self, hd_account, chain).await?;

        self.set_known_addresses_number(hd_wallet, hd_account, chain, new_known_addresses_number)
            .await?;
        Ok(address)
    }

    /// Generates a new address with an added confirmation step.
    /// This method prompts the user to verify if the derived address matches
    /// the hardware wallet display, ensuring security and accuracy when
    /// dealing with hardware wallets.
    ///
    /// # Parameters
    /// - `hd_wallet`: The specified HD wallet.
    /// - `hd_account`: The mutable HD account.
    /// - `chain`: The Bip44 chain identifier.
    /// - `confirm_address`: Address confirmation method.
    ///
    /// # Returns
    /// A result containing the confirmed `HDAddress<Self::Address, Self::Pubkey>` instance or an error.
    async fn generate_and_confirm_new_address<ConfirmAddress>(
        &self,
        hd_wallet: &Self::HDWallet,
        hd_account: &mut HDCoinHDAccount<Self>,
        chain: Bip44Chain,
        confirm_address: &ConfirmAddress,
    ) -> MmResult<HDCoinHDAddress<Self>, NewAddressDeriveConfirmError>
    where
        ConfirmAddress: HDConfirmAddress,
    {
        use super::inner_impl;

        let inner_impl::NewAddress {
            hd_address,
            new_known_addresses_number,
        } = inner_impl::generate_new_address_immutable(self, hd_account, chain).await?;

        let trezor_coin = self.trezor_coin()?;
        let derivation_path = hd_address.derivation_path().clone();
        let expected_address = hd_address.address().to_string();
        // Ask the user to confirm if the given `expected_address` is the same as on the HW display.
        confirm_address
            .confirm_address(trezor_coin, derivation_path, expected_address)
            .await?;

        let actual_known_addresses_number = hd_account.known_addresses_number(chain)?;
        // Check if the actual `known_addresses_number` hasn't been changed while we waited for the user confirmation.
        // If the actual value is greater than the new one, we don't need to update.
        if actual_known_addresses_number < new_known_addresses_number {
            self.set_known_addresses_number(hd_wallet, hd_account, chain, new_known_addresses_number)
                .await?;
        }

        Ok(hd_address)
    }

    /// Creates and registers a new HD account for a specific wallet.
    ///
    /// # Parameters
    /// - `hd_wallet`: The specified HD wallet.
    /// - `xpub_extractor`: Optional method for extracting the extended public key.
    ///   This is especially useful when dealing with hardware wallets. It can
    ///   allow for the extraction of the extended public key directly from the
    ///   wallet when needed.
    /// - `account_id`: Optional account identifier.
    ///
    /// # Returns
    /// A result containing a mutable reference to the created `Self::HDAccount` or an error.
    async fn create_new_account<'a, XPubExtractor>(
        &self,
        hd_wallet: &'a Self::HDWallet,
        xpub_extractor: Option<XPubExtractor>,
        account_id: Option<u32>,
    ) -> MmResult<HDAccountMut<'a, HDCoinHDAccount<Self>>, NewAccountCreatingError>
    where
        XPubExtractor: HDXPubExtractor + Send;

    /// Updates the count of known addresses for a specified HD account and chain.
    /// This is useful for tracking the number of created addresses.
    ///
    /// # Parameters
    /// - `hd_wallet`: The specified HD wallet.
    /// - `hd_account`: The mutable HD account to be updated.
    /// - `chain`: The Bip44 chain identifier.
    /// - `new_known_addresses_number`: The new count of known addresses.
    ///
    /// # Returns
    /// A result indicating success or an error.
    async fn set_known_addresses_number(
        &self,
        hd_wallet: &Self::HDWallet,
        hd_account: &mut HDCoinHDAccount<Self>,
        chain: Bip44Chain,
        new_known_addresses_number: u32,
    ) -> MmResult<(), AccountUpdatingError> {
        let max_addresses_number = hd_account.address_limit();
        if new_known_addresses_number >= max_addresses_number {
            return MmError::err(AccountUpdatingError::AddressLimitReached { max_addresses_number });
        }
        match chain {
            Bip44Chain::External => {
                hd_wallet
                    .update_external_addresses_number(hd_account.account_id(), new_known_addresses_number)
                    .await?
            },
            Bip44Chain::Internal => {
                hd_wallet
                    .update_internal_addresses_number(hd_account.account_id(), new_known_addresses_number)
                    .await?
            },
        }
        hd_account.set_known_addresses_number(chain, new_known_addresses_number);

        Ok(())
    }

    // Todo: Maybe make a trait specifically for HW wallets that is a superset of HDWalletCoinOps
    // Todo: Use an error type that is specific to this function
    /// Returns the Trezor coin name for this coin.
    fn trezor_coin(&self) -> MmResult<String, NewAddressDeriveConfirmError>;
}
