use bitcrypto::{dhash160, sha256};
use derive_more::Display;
use std::convert::TryFrom;

/// Algorithm used to hash swap secret.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub enum SecretHashAlgo {
    /// ripemd160(sha256(secret))
    #[default]
    DHASH160 = 1,
    /// sha256(secret)
    SHA256 = 2,
}

#[derive(Debug, Display)]
pub struct UnsupportedSecretHashAlgo(u8);

impl std::error::Error for UnsupportedSecretHashAlgo {}

impl TryFrom<u8> for SecretHashAlgo {
    type Error = UnsupportedSecretHashAlgo;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(SecretHashAlgo::DHASH160),
            2 => Ok(SecretHashAlgo::SHA256),
            unsupported => Err(UnsupportedSecretHashAlgo(unsupported)),
        }
    }
}

impl SecretHashAlgo {
    pub fn hash_secret(&self, secret: &[u8]) -> Vec<u8> {
        match self {
            SecretHashAlgo::DHASH160 => dhash160(secret).take().into(),
            SecretHashAlgo::SHA256 => sha256(secret).take().into(),
        }
    }
}
