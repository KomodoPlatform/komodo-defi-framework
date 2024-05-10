use crate::sia::blake2b_internal::standard_unlock_hash;
use blake2b_simd::Params;
use ed25519_dalek::PublicKey;
use hex::FromHexError;
use rpc::v1::types::H256;
use serde::{Deserialize, Serialize};
use std::convert::TryInto;
use std::fmt;
use std::str::FromStr;

// TODO this could probably include the checksum within the data type
// generating the checksum on the fly is how Sia Go does this however
#[derive(Debug, Clone, PartialEq)]
pub struct Address(pub H256);

impl Address {
    pub fn str_without_prefix(&self) -> String {
        let bytes = self.0 .0.as_ref();
        let checksum = blake2b_checksum(bytes);
        format!("{}{}", hex::encode(bytes), hex::encode(checksum))
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "addr:{}", self.str_without_prefix()) }
}

impl fmt::Display for ParseAddressError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "Failed to parse Address: {:?}", self) }
}

#[derive(Debug, Deserialize, Serialize)]
pub enum ParseAddressError {
    #[serde(rename = "Address must begin with addr: prefix")]
    MissingPrefix,
    InvalidHexEncoding(String),
    InvalidChecksum,
    InvalidLength,
    // Add other error kinds as needed
}

impl From<FromHexError> for ParseAddressError {
    fn from(e: FromHexError) -> Self { ParseAddressError::InvalidHexEncoding(e.to_string()) }
}

impl FromStr for Address {
    type Err = ParseAddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.starts_with("addr:") {
            return Err(ParseAddressError::MissingPrefix);
        }

        let without_prefix = &s[5..];
        if without_prefix.len() != (32 + 6) * 2 {
            return Err(ParseAddressError::InvalidLength);
        }

        let (address_hex, checksum_hex) = without_prefix.split_at(32 * 2);

        let address_bytes: [u8; 32] = hex::decode(address_hex)
            .map_err(ParseAddressError::from)?
            .try_into()
            .expect("length is 32 bytes");

        let checksum = hex::decode(checksum_hex).map_err(ParseAddressError::from)?;
        let checksum_bytes: [u8; 6] = checksum.try_into().expect("length is 6 bytes");

        if checksum_bytes != blake2b_checksum(&address_bytes) {
            return Err(ParseAddressError::InvalidChecksum);
        }

        Ok(Address(H256::from(address_bytes)))
    }
}

// Sia uses the first 6 bytes of blake2b(preimage) appended
// to address as checksum
fn blake2b_checksum(preimage: &[u8]) -> [u8; 6] {
    let hash = Params::new().hash_length(32).to_state().update(preimage).finalize();
    hash.as_array()[0..6].try_into().expect("array is 64 bytes long")
}

pub fn v1_standard_address_from_pubkey(pubkey: &PublicKey) -> Address {
    let hash = standard_unlock_hash(pubkey);
    Address(hash)
}

#[test]
fn test_v1_standard_address_from_pubkey() {
    let pubkey = PublicKey::from_bytes(
        &hex::decode("8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c").unwrap(),
    )
    .unwrap();
    let address = v1_standard_address_from_pubkey(&pubkey);
    assert_eq!(
        format!("{}", address),
        "addr:c959f9b423b662c36ee58057b8157acedb4095cfeb7926e4ba44cd9ee1f49a5b7803c7501a7b"
    )
}

#[test]
fn test_blake2b_checksum() {
    let checksum =
        blake2b_checksum(&hex::decode("591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a884").unwrap());
    let expected: [u8; 6] = hex::decode("0be0653e411f").unwrap().try_into().unwrap();
    assert_eq!(checksum, expected);
}

#[test]
fn test_address_display() {
    let address = Address("591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a884".into());
    let address_str = "addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e411f";
    assert_eq!(format!("{}", address), address_str);
}

#[test]
fn test_address_fromstr() {
    let address1 = Address("591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a884".into());

    let address2 =
        Address::from_str("addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e411f").unwrap();
    assert_eq!(address1, address2);
}

#[test]
fn test_address_fromstr_bad_length() {
    let address = Address::from_str("addr:dead");
    assert!(matches!(address, Err(ParseAddressError::InvalidLength)));
}

#[test]
fn test_address_fromstr_odd_length() {
    let address = Address::from_str("addr:f00");
    assert!(matches!(address, Err(ParseAddressError::InvalidLength)));
}

#[test]
fn test_address_fromstr_invalid_hex() {
    let address =
        Address::from_str("addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e41gg");
    assert!(matches!(address, Err(ParseAddressError::InvalidHexEncoding(_))));
}

#[test]
fn test_address_fromstr_missing_prefix() {
    let address = Address::from_str("591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e41gg");
    assert!(matches!(address, Err(ParseAddressError::MissingPrefix)));
}

#[test]
fn test_address_fromstr_invalid_checksum() {
    let address =
        Address::from_str("addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a884ffffffffffff");
    assert!(matches!(address, Err(ParseAddressError::InvalidChecksum)));
}

#[test]
fn test_address_str_without_prefix() {
    let address =
        Address::from_str("addr:591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e411f").unwrap();

    assert_eq!(
        address.str_without_prefix(),
        "591fcf237f8854b5653d1ac84ae4c107b37f148c3c7b413f292d48db0c25a8840be0653e411f"
    );
}
