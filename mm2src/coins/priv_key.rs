use crate::hd_wallet::{HDAccountOps, HDWalletOps};
use crate::{BchCoin, QtumCoin, Qrc20Coin, UtxoStandardCoin, ZCoin, SlpToken, EthCoin, TendermintCoin, CoinWithDerivationMethod, CoinWithPrivKeyPolicy, DerivationMethod, MmCoin, PrivKeyPolicy};
use async_trait::async_trait;
use bip32::ChildNumber;
use common::HttpStatusCode;
use crypto::Bip44Chain;
use derive_more::Display;
use http::StatusCode;
use keys::{KeyPair, Private};
use mm2_err_handle::prelude::*;
use serde::{Deserialize, Serialize};
use std::convert::TryInto;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DerivedPrivKey {
    pub coin: String,
    pub address: String,
    pub derivation_path: String,
    pub priv_key: String,
    pub pub_key: String,
}

#[derive(Debug, Deserialize)]
pub struct DerivePrivKeyReq {
    pub account_id: u32,
    pub chain: Option<Bip44Chain>,
    pub address_id: u32,
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum DerivePrivKeyError {
    #[display(fmt = "No such coin: {}", ticker)]
    NoSuchCoin {
        ticker: String,
    },
    #[display(fmt = "Coin {} doesn't support HD wallet derivation", ticker)]
    CoinDoesntSupportDerivation {
        ticker: String,
    },
    #[display(fmt = "Hardware/remote wallet doesn't allow exporting private keys")]
    HwWalletNotAllowed,
    #[display(fmt = "Internal error: {}", reason)]
    Internal {
        reason: String,
    },
}

impl HttpStatusCode for DerivePrivKeyError {
    fn status_code(&self) -> StatusCode {
        match self {
            DerivePrivKeyError::NoSuchCoin { .. } => StatusCode::NOT_FOUND,
            DerivePrivKeyError::CoinDoesntSupportDerivation { .. } => StatusCode::BAD_REQUEST,
            DerivePrivKeyError::HwWalletNotAllowed => StatusCode::FORBIDDEN,
            DerivePrivKeyError::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

trait CoinKeyInfo {
    fn wif_prefix(&self) -> u8;
    fn uses_utxo_format(&self) -> bool;
}

macro_rules! impl_utxo_key_info {
    ($coin_type:ty) => {
        impl CoinKeyInfo for $coin_type {
            fn wif_prefix(&self) -> u8 { self.as_ref().conf.wif_prefix }
            fn uses_utxo_format(&self) -> bool { true }
        }
    };
}

macro_rules! impl_non_utxo_key_info {
    ($coin_type:ty) => {
        impl CoinKeyInfo for $coin_type {
            fn wif_prefix(&self) -> u8 { 0 }
            fn uses_utxo_format(&self) -> bool { false }
        }
    };
}

// UTXO coins
impl_utxo_key_info!(UtxoStandardCoin);
impl_utxo_key_info!(QtumCoin);
impl_utxo_key_info!(BchCoin);
impl_utxo_key_info!(Qrc20Coin);
impl_utxo_key_info!(SlpToken);
#[cfg(not(target_arch = "wasm32"))]
impl_utxo_key_info!(ZCoin);

// Non-UTXO coins
impl_non_utxo_key_info!(EthCoin);
impl_non_utxo_key_info!(TendermintCoin);

#[async_trait]
pub trait DerivePrivKeyV2: MmCoin + CoinWithPrivKeyPolicy + CoinWithDerivationMethod + Sized {
    async fn derive_priv_key(&self, req: &DerivePrivKeyReq) -> Result<DerivedPrivKey, MmError<DerivePrivKeyError>>;
}

#[async_trait]
impl<Coin> DerivePrivKeyV2 for Coin
where
    Coin: MmCoin + CoinWithPrivKeyPolicy + CoinWithDerivationMethod + CoinKeyInfo + Sync,
{
    async fn derive_priv_key(&self, req: &DerivePrivKeyReq) -> Result<DerivedPrivKey, MmError<DerivePrivKeyError>> {
        match self.priv_key_policy() {
            PrivKeyPolicy::Iguana(_) => MmError::err(DerivePrivKeyError::CoinDoesntSupportDerivation {
                ticker: self.ticker().to_string(),
            }),
            PrivKeyPolicy::Trezor | PrivKeyPolicy::WalletConnect { .. } => {
                MmError::err(DerivePrivKeyError::HwWalletNotAllowed)
            },
            PrivKeyPolicy::HDWallet { .. } => {
                let hd_wallet = match self.derivation_method() {
                    DerivationMethod::HDWallet(hd_wallet) => hd_wallet,
                    _ => {
                        return MmError::err(DerivePrivKeyError::CoinDoesntSupportDerivation {
                            ticker: self.ticker().to_string(),
                        })
                    },
                };

                let account = hd_wallet
                    .get_account(req.account_id)
                    .await
                    .ok_or_else(|| DerivePrivKeyError::Internal {
                        reason: format!("Account {} not found", req.account_id),
                    })?;

                let mut path_to_address = account.account_derivation_path();
                path_to_address.push(req.chain.unwrap_or(Bip44Chain::External).to_child_number());
                path_to_address.push(ChildNumber::new(req.address_id, false).expect("non-hardened"));

                let secret_key = self
                    .priv_key_policy()
                    .hd_wallet_derived_priv_key_or_err(&path_to_address)
                    .map_err(|e| DerivePrivKeyError::Internal {
                        reason: format!("Error deriving secret key: {}", e),
                    })?;

                let private = Private {
                    prefix: self.wif_prefix(),
                    secret: secret_key.into(),
                    compressed: true,
                    checksum_type: Default::default(),
                };

                let key_pair = KeyPair::from_private(private)
                    .map_err(|e| DerivePrivKeyError::Internal {
                        reason: format!("Error creating key pair from secret: {}", e),
                    })?;

                let pubkey_slice = key_pair.public_slice();
                let pubkey: [u8; 33] = pubkey_slice
                    .try_into()
                    .map_err(|_| DerivePrivKeyError::Internal {
                        reason: "Error converting pubkey slice to array".to_string(),
                    })?;

                let address = self
                    .address_from_pubkey(&pubkey.into())
                    .map_err(|e| DerivePrivKeyError::Internal {
                        reason: format!("Error getting address from pubkey: {}", e),
                    })?;

                let priv_key_wif = key_pair.private().to_string();
                let priv_key_hex = format!("0x{}", hex::encode(key_pair.private_bytes()));

                let priv_key = if self.uses_utxo_format() { priv_key_wif } else { priv_key_hex };

                let pub_key = if self.uses_utxo_format() {
                    hex::encode(pubkey)
                } else {
                    format!("0x{}", hex::encode(pubkey))
                };

                let response = DerivedPrivKey {
                    coin: self.ticker().to_string(),
                    address: address.to_string(),
                    derivation_path: path_to_address.to_string(),
                    priv_key,
                    pub_key,
                };
                Ok(response)
            },
        }
    }
}
