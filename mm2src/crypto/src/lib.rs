#[macro_use] extern crate serde_derive;

mod bip32_child;
mod bip44;
mod crypto_ctx;
mod hw_client;
mod hw_ctx;
pub mod hw_rpc_task;
mod key_pair_ctx;

pub use bip32_child::{Bip32Child, Bip32DerPathError, Bip32DerPathOps, Bip44Tail};
pub use bip44::{Bip44Chain, Bip44DerPathError, Bip44DerivationPath, Bip44PathToAccount, Bip44PathToCoin,
                UnkownBip44ChainError, BIP44_PURPOSE};
pub use crypto_ctx::{CryptoCtx, CryptoInitError, CryptoInitResult};
pub use hw_client::TrezorConnectProcessor;
pub use hw_client::{HwClient, HwError, HwProcessingError, HwResult, HwWalletType};
pub use hw_common::primitives::{Bip32Error, ChildNumber, DerivationPath, EcdsaCurve, ExtendedPublicKey,
                                Secp256k1ExtendedPublicKey, XPub};
pub use hw_ctx::{HardwareWalletArc, HardwareWalletCtx};
pub use key_pair_ctx::{KeyPairArc, KeyPairCtx};
pub use trezor;

use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq)]
pub struct RpcDerivationPath(pub DerivationPath);

impl From<DerivationPath> for RpcDerivationPath {
    fn from(der: DerivationPath) -> Self { RpcDerivationPath(der) }
}

impl From<RpcDerivationPath> for DerivationPath {
    fn from(der: RpcDerivationPath) -> Self { der.0 }
}

impl Serialize for RpcDerivationPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for RpcDerivationPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let path = String::deserialize(deserializer)?;
        let inner = DerivationPath::from_str(&path).map_err(|e| D::Error::custom(format!("{}", e)))?;
        Ok(RpcDerivationPath(inner))
    }
}
