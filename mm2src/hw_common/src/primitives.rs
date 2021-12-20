pub const HARDENED_PATH: u32 = 2147483648;

pub use bip32::{ChildNumber, DerivationPath, Error as Bip32Error};

#[derive(Clone, Copy)]
pub enum EcdsaCurve {
    Secp256k1,
}
