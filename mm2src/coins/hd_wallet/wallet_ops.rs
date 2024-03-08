use super::{HDAccountMut, HDAccountOps, HDAccountsMap, HDAccountsMut, HDAccountsMutex};
use async_trait::async_trait;
use crypto::{Bip44Chain, StandardHDPathToCoin};

/// `HDWalletOps`: Operations that should be implemented for Structs that represent HD wallets.
#[async_trait]
pub trait HDWalletOps {
    /// The HD account operations associated with this wallet.
    type HDAccount: HDAccountOps + Clone + Send + Sync;

    /// Returns the coin type associated with this HD Wallet.
    ///
    /// This method can be implemented to fetch the coin type as specified in the wallet's BIP44 derivation path.
    /// For example, in the derivation path `m/44'/0'/0'/0`, the coin type would be `0` (representing Bitcoin).
    ///
    /// # Returns
    ///
    /// A `u32` value representing the coin type from the wallet's derivation path.
    fn coin_type(&self) -> u32;

    /// Returns the derivation path associated with this HD Wallet.
    ///
    /// This method can be implemented to fetch the derivation path as specified in the wallet's BIP44 derivation path.
    ///
    /// # Returns
    ///
    /// A `StandardHDPathToCoin` value representing the derivation path from the wallet's derivation path.
    fn derivation_path(&self) -> &StandardHDPathToCoin;

    /// Fetches the gap limit associated with this HD Wallet.
    ///
    /// # Returns
    ///
    /// A `u32` value that specifies the gap limit.
    fn gap_limit(&self) -> u32;

    /// Specifies the limit on the number of accounts that can be added to the wallet.
    ///
    /// # Returns
    ///
    /// A `u32` value indicating the maximum number of accounts.
    /// The default is set by `DEFAULT_ACCOUNT_LIMIT`.
    fn account_limit(&self) -> u32;

    /// Specifies the default BIP44 chain for receiver addresses.
    ///
    /// # Returns
    ///
    /// A `Bip44Chain` value.
    /// The default is set by `DEFAULT_RECEIVER_CHAIN`.
    fn default_receiver_chain(&self) -> Bip44Chain;

    /// Provides a mutex that guards the HD accounts.
    ///
    /// # Returns
    ///
    /// A reference to the accounts mutex.
    fn get_accounts_mutex(&self) -> &HDAccountsMutex<Self::HDAccount>;

    /// Fetches an account based on its ID. This method will return `None` if the account is not activated.
    ///
    /// # Parameters
    ///
    /// - `account_id`: The ID of the desired account.
    ///
    /// # Returns
    ///
    /// An `Option<Self::HDAccount>` which contains the account if found.
    async fn get_account(&self, account_id: u32) -> Option<Self::HDAccount>;

    /// Similar to `get_account`, but provides a mutable reference.
    ///
    /// # Parameters
    ///
    /// - `account_id`: The ID of the desired account.
    ///
    /// # Returns
    ///
    /// An `Option` containing a mutable reference to the account if found.
    async fn get_account_mut(&self, account_id: u32) -> Option<HDAccountMut<'_, Self::HDAccount>>;

    /// Gathers all the activated accounts.
    ///
    /// # Returns
    ///
    /// A map containing all the currently activated HD accounts.
    async fn get_accounts(&self) -> HDAccountsMap<Self::HDAccount>;

    /// Similar to `get_accounts`, but provides a mutable reference to the accounts.
    ///
    /// # Returns
    ///
    /// A mutable reference to the map of all activated HD accounts.
    async fn get_accounts_mut(&self) -> HDAccountsMut<'_, Self::HDAccount>;

    /// Attempts to remove an account only if it's the last in the set.
    ///
    /// # Parameters
    ///
    /// - `account_id`: The ID of the account to be considered for removal.
    ///
    /// # Returns
    ///
    /// An `Option` containing the removed HD account if it was indeed the last one, otherwise `None`.
    async fn remove_account_if_last(&self, account_id: u32) -> Option<Self::HDAccount>;

    /// Returns an address that's currently enabled for single-address operations, such as swaps.
    ///
    /// # Returns
    ///
    /// An `Option` containing the enabled address if available.
    async fn get_enabled_address(&self) -> Option<<Self::HDAccount as HDAccountOps>::HDAddress>;
}
