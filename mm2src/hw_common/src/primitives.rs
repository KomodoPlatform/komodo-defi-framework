pub const HARDENED_PATH: u32 = 2147483648;

pub use bip32::{ChildNumber, DerivationPath, Error as Bip32Error, ExtendedPublicKey};

pub type Secp256k1ExtendedPublicKey = ExtendedPublicKey<secp256k1::PublicKey>;
pub type XPub = String;

// #[derive(Clone, Debug, PartialEq)]
// pub struct PublicKeyInternal(pub secp256k1::PublicKey);
//
// impl bip32::PublicKey for PublicKeyInternal {
//     fn from_bytes(bytes: bip32::PublicKeyBytes) -> bip32::Result<Self> {
//         Ok(PublicKeyInternal(secp256k1::PublicKey::from_slice(&bytes).map_err(|_| bip32::Error::Crypto)?))
//     }
//
//     fn to_bytes(&self) -> bip32::PublicKeyBytes {
//         self.0.serialize()
//     }
//
//     fn derive_child(&self, other: bip32::PrivateKeyBytes) -> bip32::Result<Self> {
//         let engine = secp256k1::Secp256k1::<secp256k1::VerifyOnly>::verification_only();
//
//         let mut child_key = self.0;
//         child_key
//             .add_exp_assign(&engine, &other)
//             .map_err(|_| bip32::Error::Crypto)?;
//
//         Ok(PublicKeyInternal(child_key))
//     }
// }

#[derive(Clone, Copy)]
pub enum EcdsaCurve {
    Secp256k1,
}
