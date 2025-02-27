use bip32::DerivationPath;
use std::hash::Hash;

/// A trait for converting an address into a string suitable for display in logs, errors, or messages.
pub trait DisplayAddress {
    fn display_address(&self) -> String;
}

/// `HDAddressOps` Trait
///
/// Defines operations associated with an HD (Hierarchical Deterministic) address.
/// In the context of BIP-44 derivation paths, an HD address corresponds to the fifth level (`address_index`)
/// in the structure `m / purpose' / coin_type' / account' / chain (or change) / address_index`.
/// This allows for managing individual addresses within a specific account and chain.
pub trait HDAddressOps {
    type Address: Clone + DisplayAddress + Eq + Hash + Send + Sync;
    type Pubkey: Clone;

    fn address(&self) -> Self::Address;
    fn pubkey(&self) -> Self::Pubkey;
    fn derivation_path(&self) -> &DerivationPath;
}
