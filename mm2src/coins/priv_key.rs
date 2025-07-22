use crate::hd_wallet::{HDAccountOps, HDWalletOps};
use crate::{CoinWithDerivationMethod, CoinWithPrivKeyPolicy, DerivationMethod, MmCoin, MmCoinEnum, PrivKeyPolicy};
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

pub async fn derive_priv_key(coin: MmCoinEnum, req: &DerivePrivKeyReq) -> Result<DerivedPrivKey, MmError<DerivePrivKeyError>> {
    match coin {
        MmCoinEnum::UtxoCoin(c) => derive_priv_key_for_utxo_coin(c, req).await,
        MmCoinEnum::Bch(c) => derive_priv_key_for_utxo_coin(c, req).await,
        MmCoinEnum::QtumCoin(c) => derive_priv_key_for_utxo_coin(c, req).await,
        MmCoinEnum::EthCoin(c) => derive_priv_key_for_eth_coin(c, req).await,
        _ => MmError::err(DerivePrivKeyError::CoinDoesntSupportDerivation {
            ticker: coin.ticker().to_string(),
        }),
    }
}

async fn derive_priv_key_for_utxo_coin(
    coin: impl MmCoin + CoinWithPrivKeyPolicy + CoinWithDerivationMethod + AsRef<crate::utxo::UtxoCoinFields>,
    req: &DerivePrivKeyReq,
) -> Result<DerivedPrivKey, MmError<DerivePrivKeyError>> {
    let coin_fields = coin.as_ref();
    
    match coin.priv_key_policy() {
        PrivKeyPolicy::Iguana(_) => MmError::err(DerivePrivKeyError::CoinDoesntSupportDerivation {
            ticker: coin.ticker().to_string(),
        }),
        PrivKeyPolicy::Trezor | PrivKeyPolicy::WalletConnect { .. } => {
            MmError::err(DerivePrivKeyError::HwWalletNotAllowed)
        },
        PrivKeyPolicy::HDWallet { .. } => {
            let hd_wallet = match coin.derivation_method() {
                DerivationMethod::HDWallet(hd_wallet) => hd_wallet,
                _ => {
                    return MmError::err(DerivePrivKeyError::CoinDoesntSupportDerivation {
                        ticker: coin.ticker().to_string(),
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

            let secret_key = coin
                .priv_key_policy()
                .hd_wallet_derived_priv_key_or_err(&path_to_address)
                .map_err(|e| DerivePrivKeyError::Internal {
                    reason: format!("Error deriving secret key: {}", e),
                })?;

            let private = Private {
                prefix: coin_fields.conf.wif_prefix,
                secret: secret_key.into(),
                compressed: true,
                checksum_type: coin_fields.conf.checksum_type,
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

            let address = coin
                .address_from_pubkey(&pubkey.into())
                .map_err(|e| DerivePrivKeyError::Internal {
                    reason: format!("Error getting address from pubkey: {}", e),
                })?;

            let priv_key_wif = key_pair.private().to_string();
            let pub_key_hex = hex::encode(pubkey);

            let response = DerivedPrivKey {
                coin: coin.ticker().to_string(),
                address: address.to_string(),
                derivation_path: path_to_address.to_string(),
                priv_key: priv_key_wif,
                pub_key: pub_key_hex,
            };
            Ok(response)
        },
        #[cfg(target_arch = "wasm32")]
        PrivKeyPolicy::Metamask(_) => MmError::err(DerivePrivKeyError::HwWalletNotAllowed),
    }
}

async fn derive_priv_key_for_eth_coin(
    coin: impl MmCoin + CoinWithPrivKeyPolicy + CoinWithDerivationMethod,
    req: &DerivePrivKeyReq,
) -> Result<DerivedPrivKey, MmError<DerivePrivKeyError>> {
    match coin.priv_key_policy() {
        PrivKeyPolicy::Iguana(_) => MmError::err(DerivePrivKeyError::CoinDoesntSupportDerivation {
            ticker: coin.ticker().to_string(),
        }),
        PrivKeyPolicy::Trezor | PrivKeyPolicy::WalletConnect { .. } => {
            MmError::err(DerivePrivKeyError::HwWalletNotAllowed)
        },
        PrivKeyPolicy::HDWallet { .. } => {
            let hd_wallet = match coin.derivation_method() {
                DerivationMethod::HDWallet(hd_wallet) => hd_wallet,
                _ => {
                    return MmError::err(DerivePrivKeyError::CoinDoesntSupportDerivation {
                        ticker: coin.ticker().to_string(),
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

            let secret_key = coin
                .priv_key_policy()
                .hd_wallet_derived_priv_key_or_err(&path_to_address)
                .map_err(|e| DerivePrivKeyError::Internal {
                    reason: format!("Error deriving secret key: {}", e),
                })?;

            let private = Private {
                prefix: 0, // ETH doesn't use WIF format
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

            let address = coin
                .address_from_pubkey(&pubkey.into())
                .map_err(|e| DerivePrivKeyError::Internal {
                    reason: format!("Error getting address from pubkey: {}", e),
                })?;

            let priv_key_hex = format!("0x{}", hex::encode(key_pair.private_bytes()));
            let pub_key_hex = format!("0x{}", hex::encode(pubkey));

            let response = DerivedPrivKey {
                coin: coin.ticker().to_string(),
                address: address.to_string(),
                derivation_path: path_to_address.to_string(),
                priv_key: priv_key_hex,
                pub_key: pub_key_hex,
            };
            Ok(response)
        },
        #[cfg(target_arch = "wasm32")]
        PrivKeyPolicy::Metamask(_) => MmError::err(DerivePrivKeyError::HwWalletNotAllowed),
    }
}
