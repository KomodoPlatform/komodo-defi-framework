use crate::encoding::{Encodable, Encoder};
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::str::FromStr;

// this macro allows us to define the byte arrays as constants at compile time
macro_rules! define_byte_array_const {
    ($name:ident, $size:expr, $value:expr) => {
        pub const $name: [u8; $size] = {
            let mut arr = [0u8; $size];
            let bytes = $value.as_bytes();
            let mut i = 0;
            while i < bytes.len() && i < $size {
                arr[i] = bytes[i];
                i += 1;
            }
            arr
        };
    };
}

define_byte_array_const!(ED25519, 16, "ed25519");
define_byte_array_const!(SIACOIN_OUTPUT, 16, "siacoin output");
define_byte_array_const!(SIAFUND_OUTPUT, 16, "siafund output");
define_byte_array_const!(FILE_CONTRACT, 16, "file contract");
define_byte_array_const!(STORAGE_PROOF, 16, "storage proof");
define_byte_array_const!(FOUNDATION, 16, "foundation");
define_byte_array_const!(ENTROPY, 16, "entropy");
// Sia Go technically supports arbitrary Specifiers
// we will use "unknown" as a catch all in serde and encoding
define_byte_array_const!(UNKNOWN, 16, "unknown");

// https://github.com/SiaFoundation/core/blob/6c19657baf738c6b730625288e9b5413f77aa659/types/types.go#L40-L49
// A Specifier is a fixed-size, 0-padded identifier.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum Specifier {
    Ed25519,
    SiacoinOutput,
    SiafundOutput,
    FileContract,
    StorageProof,
    Foundation,
    Entropy,
    Unknown,
}

impl Encodable for Specifier {
    fn encode(&self, encoder: &mut Encoder) { encoder.write_slice(self.as_bytes()); }
}

impl Specifier {
    pub fn as_bytes(&self) -> &'static [u8; 16] {
        match self {
            Specifier::Ed25519 => &ED25519,
            Specifier::SiacoinOutput => &SIACOIN_OUTPUT,
            Specifier::SiafundOutput => &SIAFUND_OUTPUT,
            Specifier::FileContract => &FILE_CONTRACT,
            Specifier::StorageProof => &STORAGE_PROOF,
            Specifier::Foundation => &FOUNDATION,
            Specifier::Entropy => &ENTROPY,
            Specifier::Unknown => &UNKNOWN,
        }
    }

    pub fn from_str_expect(s: &str) -> Self { Specifier::from_str(s).expect("from_str cannot return Err") }

    pub fn to_str(&self) -> &'static str {
        match self {
            Specifier::Ed25519 => "ed25519",
            Specifier::SiacoinOutput => "siacoin output",
            Specifier::SiafundOutput => "siafund output",
            Specifier::FileContract => "file contract",
            Specifier::StorageProof => "storage proof",
            Specifier::Foundation => "foundation",
            Specifier::Entropy => "entropy",
            Specifier::Unknown => "unknown",
        }
    }
}

#[derive(Debug)]
pub struct ParseSpecifierError;

impl FromStr for Specifier {
    type Err = ParseSpecifierError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let r = match s {
            "ed25519" => Specifier::Ed25519,
            "siacoin output" => Specifier::SiacoinOutput,
            "siafund output" => Specifier::SiafundOutput,
            "file contract" => Specifier::FileContract,
            "storage proof" => Specifier::StorageProof,
            "foundation" => Specifier::Foundation,
            "entropy" => Specifier::Entropy,
            _ => Specifier::Unknown,
        };
        Ok(r)
    }
}

impl Display for Specifier {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "{}", self.to_str()) }
}
