use super::{ExtendedPublicKeyOps, HDAddressOps, HDAddressesCache, InvalidBip44ChainError};
use crypto::{Bip44Chain, DerivationPath, HDPathToAccount};
use mm2_err_handle::prelude::*;

/// `HDAccountOps` Trait
///
/// Defines operations associated with an HD (Hierarchical Deterministic) account.
/// In the context of BIP-44 derivation paths, an HD account corresponds to the third level (`account'`)
/// in the structure `m / purpose' / coin_type' / account' / chain (or change) / address_index`.
/// This allows for segregating funds into different accounts under the same seed,
/// with each account having multiple chains (often representing internal and external addresses).
///
/// Implementors of this trait should provide details about such HD account like its specific derivation path, known addresses, and its index.
pub trait HDAccountOps {
    type HDAddress: HDAddressOps + Clone + Send;
    /// Any type that represents an extended public key, whether it's secp256k1, ed25519, Schnorr, etc.
    /// This type should implement the `ExtendedPublicKeyOps` trait.
    type ExtendedPublicKey: ExtendedPublicKeyOps;

    /// A constructor for any type that implements `HDAccountOps`.
    fn new(
        account_id: u32,
        account_extended_pubkey: Self::ExtendedPublicKey,
        account_derivation_path: HDPathToAccount,
    ) -> Self;

    /// Returns the limit on the number of addresses that can be added to an account.
    fn address_limit(&self) -> u32;

    /// Returns the number of known addresses for this account for a specific chain/change
    /// (internal/external) path.
    fn known_addresses_number(&self, chain: Bip44Chain) -> MmResult<u32, InvalidBip44ChainError>;

    /// Sets the number of known addresses for this account for a specific chain/change
    /// (internal/external) path.
    fn set_known_addresses_number(&mut self, chain: Bip44Chain, new_known_addresses_number: u32);

    /// Returns the derivation path associated with this account.
    fn account_derivation_path(&self) -> DerivationPath;

    /// Returns the index of this account.
    /// The account index is used as part of the derivation path,
    /// following the pattern `m/purpose'/coin'/account'`.
    fn account_id(&self) -> u32;

    /// Checks if a specific address is activated (known) for this account at the present time.
    fn is_address_activated(&self, chain: Bip44Chain, address_id: u32) -> MmResult<bool, InvalidBip44ChainError>;

    /// Fetches the derived/cached addresses.
    fn derived_addresses(&self) -> &HDAddressesCache<Self::HDAddress>;

    /// Fetches the extended public key associated with this account.
    fn extended_pubkey(&self) -> &Self::ExtendedPublicKey;
}
