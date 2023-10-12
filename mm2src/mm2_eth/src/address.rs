pub use ethkey::Address;
use ethkey::{public_to_address, Public};
use web3::types::H520;

pub fn address_from_pubkey_uncompressed(bytes: H520) -> Address {
    // Skip the first byte of the uncompressed public key.
    let public = Public::from_slice(&bytes[1..]);
    public_to_address(&public)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_address_from_pubkey_uncompressed() {
        let pubkey = H520::from_str("04d5f11f3cf90d165af39b17caec89162c607ddfc2d64d4eba2058c2eb8c2347cc422eaf112cb01a662f5f29924e2a6322153ae05d4e73526cb83cc1759c09fc01").unwrap();

        let actual = address_from_pubkey_uncompressed(pubkey);
        let expected = Address::from_str("0x9fd51e0930CA900B80Fa08f5F9166ce23ef44ea5").unwrap();
        assert_eq!(actual, expected);
    }
}
