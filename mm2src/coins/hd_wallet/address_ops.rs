use bip32::DerivationPath;
use std::fmt::Display;
use std::hash::Hash;

/// `HDAddressOps` Trait
///
/// Defines operations associated with an HD (Hierarchical Deterministic) address.
/// In the context of BIP-44 derivation paths, an HD address corresponds to the fifth level (`address_index`)
/// in the structure `m / purpose' / coin_type' / account' / chain (or change) / address_index`.
/// This allows for managing individual addresses within a specific account and chain.
pub trait HDAddressOps {
    type Address: Clone + Display + Eq + Hash + Send + Sync;
    type Pubkey: Clone;

    fn address(&self) -> Self::Address;
    fn pubkey(&self) -> Self::Pubkey;
    fn derivation_path(&self) -> &DerivationPath;
}
