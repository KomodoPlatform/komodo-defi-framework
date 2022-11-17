#[macro_use] extern crate serde_derive;

mod bip32_child;
mod crypto_ctx;
mod global_hd_ctx;
mod hw_client;
mod hw_ctx;
mod hw_error;
pub mod hw_rpc_task;
pub mod privkey;
mod standard_hd_path;
mod xpub;

pub use bip32_child::{Bip32Child, Bip32DerPathError, Bip32DerPathOps, Bip44Tail};
pub use crypto_ctx::{CryptoCtx, CryptoCtxError, CryptoInitError, CryptoInitResult, HwCtxInitError, KeyPairPolicy};
pub use global_hd_ctx::GlobalHDAccountArc;
pub use hw_client::{HwClient, HwDeviceInfo, HwProcessingError, HwPubkey, HwWalletType, TrezorConnectProcessor};
pub use hw_common::primitives::{Bip32Error, ChildNumber, DerivationPath, EcdsaCurve, ExtendedPublicKey,
                                Secp256k1ExtendedPublicKey, XPub};
pub use hw_ctx::{HardwareWalletArc, HardwareWalletCtx};
pub use hw_error::{from_hw_error, HwError, HwResult, HwRpcError, WithHwRpcError};
pub use keys::Secret as Secp256k1Secret;
pub use standard_hd_path::{Bip44Chain, StandardHDPath, StandardHDPathError, StandardHDPathToAccount,
                           StandardHDPathToCoin, UnknownChainError};
pub use trezor;
pub use xpub::{XPubConverter, XpubError};

use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::str::FromStr;

/// The derivation path generally consists of:
/// `m/purpose'/coin_type'/account'/change/address_index`.
/// For MarketMaker internal purposes, we decided to use a pubkey derived from the following path, where:
/// * `coin_type = 141` - KMD coin;
/// * `account = (2 ^ 31 - 1) = 2147483647` - latest available account index.
///   This number is chosen so that it does not cross with real accounts;
/// * `change = 0` - nothing special.
/// * `address_index` - is ether specified by the config or default `0`.
pub(crate) fn mm2_internal_der_path(address_index: Option<ChildNumber>) -> DerivationPath {
    let mut der_path = DerivationPath::from_str("m/44'/141'/2147483647/0").expect("valid derivation path");
    der_path.push(address_index.unwrap_or_default());
    der_path
}

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
