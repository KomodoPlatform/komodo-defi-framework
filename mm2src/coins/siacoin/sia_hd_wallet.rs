use crate::hd_wallet::{HDAccount, HDAddress, HDWallet};
use bip32::{ExtendedPublicKey, PrivateKeyBytes, PublicKey as bip32PublicKey, PublicKeyBytes, Result as bip32Result};
use sia_rust::types::Address;
use sia_rust::PublicKey;

pub struct SiaPublicKey(pub PublicKey);

pub type SiaHDAddress = HDAddress<Address, SiaPublicKey>;
pub type SiaHDAccount = HDAccount<SiaHDAddress, Ed25519ExtendedPublicKey>;
pub type SiaHDWallet = HDWallet<SiaHDAccount>;
pub type Ed25519ExtendedPublicKey = ExtendedPublicKey<SiaPublicKey>;

impl bip32PublicKey for SiaPublicKey {
    fn from_bytes(_bytes: PublicKeyBytes) -> bip32Result<Self> {
        todo!()
        //Ok(secp256k1_ffi::PublicKey::from_slice(&bytes)?)
    }

    fn to_bytes(&self) -> PublicKeyBytes {
        todo!()
        // self.serialize()
    }

    fn derive_child(&self, _other: PrivateKeyBytes) -> bip32Result<Self> {
        todo!()
        // use secp256k1_ffi::{Secp256k1, VerifyOnly};
        // let engine = Secp256k1::<VerifyOnly>::verification_only();

        // let mut child_key = *self;
        // child_key
        //     .add_exp_assign(&engine, &other)
        //     .map_err(|_| Error::Crypto)?;

        // Ok(child_key)
    }
}

// coin type 1991
// path component 0x800007c7

#[test]
fn test_something() {
    println!("This is a test");
}
