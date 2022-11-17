use crate::privkey::{bip39_seed_from_passphrase, key_pair_from_secret, PrivKeyError};
use crate::{mm2_internal_der_path, Bip32DerPathOps, Bip32Error, CryptoInitError, CryptoInitResult,
            StandardHDPathToCoin};
use bip32::{ChildNumber, ExtendedPrivateKey};
use keys::{KeyPair, Secret as Secp256k1Secret};
use mm2_err_handle::prelude::*;
use std::convert::TryInto;
use std::num::TryFromIntError;
use std::ops::Deref;
use std::sync::Arc;

const HARDENED: bool = true;
const NON_HARDENED: bool = false;

pub(super) type Mm2InternalKeyPair = KeyPair;

#[derive(Clone)]
pub struct GlobalHDAccountArc(Arc<GlobalHDAccountCtx>);

impl Deref for GlobalHDAccountArc {
    type Target = GlobalHDAccountCtx;

    fn deref(&self) -> &Self::Target { &self.0 }
}

pub struct GlobalHDAccountCtx {
    bip39_seed: bip39::Seed,
    bip39_secp_priv_key: ExtendedPrivateKey<secp256k1::SecretKey>,
    /// This account is set globally for every activated coin.
    hd_account: ChildNumber,
}

impl GlobalHDAccountCtx {
    pub fn new(passphrase: &str, hd_account_id: u64) -> CryptoInitResult<(Mm2InternalKeyPair, GlobalHDAccountCtx)> {
        let bip39_seed = bip39_seed_from_passphrase(passphrase)?;
        let bip39_secp_priv_key: ExtendedPrivateKey<secp256k1::SecretKey> =
            ExtendedPrivateKey::new(bip39_seed.as_bytes())
                .map_to_mm(|e| PrivKeyError::InvalidPrivKey(e.to_string()))?;

        let hd_account_id =
            hd_account_id
                .try_into()
                .map_to_mm(|e: TryFromIntError| CryptoInitError::InvalidHdAccount {
                    hd_account_id,
                    error: e.to_string(),
                })?;
        let hd_account =
            ChildNumber::new(hd_account_id, NON_HARDENED).map_to_mm(|e| CryptoInitError::InvalidHdAccount {
                hd_account_id: hd_account_id as u64,
                error: e.to_string(),
            })?;

        let derivation_path = mm2_internal_der_path(Some(hd_account));

        let mut internal_priv_key = bip39_secp_priv_key.clone();
        for child in derivation_path {
            internal_priv_key = internal_priv_key
                .derive_child(child)
                .map_to_mm(|e| CryptoInitError::InvalidPassphrase(PrivKeyError::InvalidPrivKey(e.to_string())))?;
        }

        let mm2_internal_key_pair = key_pair_from_secret(internal_priv_key.private_key().as_ref())?;

        let global_hd_ctx = GlobalHDAccountCtx {
            bip39_seed,
            bip39_secp_priv_key,
            hd_account,
        };
        Ok((mm2_internal_key_pair, global_hd_ctx))
    }

    #[inline]
    pub fn into_arc(self) -> GlobalHDAccountArc { GlobalHDAccountArc(Arc::new(self)) }

    /// Returns an identifier of the selected HD account.
    pub fn account_id(&self) -> u32 { self.hd_account.index() }

    /// Returns the root BIP39 seed.
    pub fn root_seed(&self) -> &bip39::Seed { &self.bip39_seed }

    /// Returns the root BIP39 seed as bytes.
    pub fn root_seed_bytes(&self) -> &[u8] { self.bip39_seed.as_bytes() }

    /// Derives a `secp256k1::SecretKey` from [`HDAccountCtx::bip39_secp_priv_key`]
    /// at the given `m/purpose'/coin_type'/account_id'/chain/address_id` derivation path,
    /// where:
    /// * `m/purpose'/coin_type'` is specified by `derivation_path`.
    /// * `account_id = 0`, `chain = 0`.
    /// * `address_id = HDAccountCtx::hd_account`.
    ///
    /// Returns the `secp256k1::Private` Secret 256-bit key
    pub fn derive_secp256k1_secret(
        &self,
        derivation_path: &StandardHDPathToCoin,
    ) -> MmResult<Secp256k1Secret, Bip32Error> {
        const ACCOUNT_ID: u32 = 0;
        const CHAIN_ID: u32 = 0;

        let mut account_der_path = derivation_path.to_derivation_path();
        account_der_path.push(ChildNumber::new(ACCOUNT_ID, HARDENED).unwrap());
        account_der_path.push(ChildNumber::new(CHAIN_ID, NON_HARDENED).unwrap());
        account_der_path.push(self.hd_account);

        let mut priv_key = self.bip39_secp_priv_key.clone();
        for child in account_der_path {
            priv_key = priv_key.derive_child(child)?;
        }

        let secret = *priv_key.private_key().as_ref();
        Ok(Secp256k1Secret::from(secret))
    }
}
