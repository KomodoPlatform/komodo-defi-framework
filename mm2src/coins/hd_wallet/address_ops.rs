use bip32::DerivationPath;
use std::hash::Hash;

/// A trait for converting an address into a string suitable for display in logs, errors, or messages.
pub trait DisplayAddress {
    fn display_address(&self) -> String;
}

/// Should convert coin `Self::Address` type into a properly formatted string representation.
///
/// Don't use `to_string` directly on `Self::Address` types in generic TPU code!
/// It may produce abbreviated or non-standard formats (e.g. `ethereum_types::Address` will be like this `0x7cc9…3874`),
/// which are not guaranteed to be parsable back into the original `Address` type.
/// This function should ensure the resulting string is consistently formatted and fully reversible.
pub trait AddrToString {
    fn addr_to_string(&self) -> String;
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
