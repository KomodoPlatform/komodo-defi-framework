use crate::hd_wallet::{HDAccountOps, HDWalletOps};
use crate::{CoinWithDerivationMethod, CoinWithPrivKeyPolicy, DerivationMethod, MarketCoinOps, MmCoin, PrivKeyPolicy};
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
    #[display(fmt = "No such coin: {}", _0)]
    NoSuchCoin(String),
    #[display(fmt = "Coin {} doesn't support HD wallet derivation", _0)]
    CoinDoesntSupportDerivation(String),
    #[display(fmt = "Hardware/remote wallet doesn't allow exporting private keys")]
    HwWalletNotAllowed,
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl HttpStatusCode for DerivePrivKeyError {
    fn status_code(&self) -> StatusCode {
        match self {
            DerivePrivKeyError::NoSuchCoin(_) => StatusCode::NOT_FOUND,
            DerivePrivKeyError::CoinDoesntSupportDerivation(_) => StatusCode::BAD_REQUEST,
            DerivePrivKeyError::HwWalletNotAllowed => StatusCode::FORBIDDEN,
            DerivePrivKeyError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[async_trait]
pub trait DerivePrivKeyV2: MmCoin + CoinWithPrivKeyPolicy + CoinWithDerivationMethod + Sized {
    async fn derive_priv_key(&self, req: &DerivePrivKeyReq) -> Result<DerivedPrivKey, MmError<DerivePrivKeyError>>;
}

#[async_trait]
impl<Coin> DerivePrivKeyV2 for Coin
where
    Coin: MmCoin + CoinWithPrivKeyPolicy + CoinWithDerivationMethod + MarketCoinOps + Sync,
{
    async fn derive_priv_key(&self, req: &DerivePrivKeyReq) -> Result<DerivedPrivKey, MmError<DerivePrivKeyError>> {
        match self.priv_key_policy() {
            PrivKeyPolicy::Iguana(_) => MmError::err(DerivePrivKeyError::CoinDoesntSupportDerivation(
                self.ticker().to_string(),
            )),
            PrivKeyPolicy::Trezor | PrivKeyPolicy::WalletConnect { .. } => {
                MmError::err(DerivePrivKeyError::HwWalletNotAllowed)
            },
            PrivKeyPolicy::HDWallet { .. } => {
                let hd_wallet = match self.derivation_method() {
                    DerivationMethod::HDWallet(hd_wallet) => hd_wallet,
                    _ => {
                        return MmError::err(DerivePrivKeyError::CoinDoesntSupportDerivation(
                            self.ticker().to_string(),
                        ))
                    },
                };

                let account = hd_wallet
                    .get_account(req.account_id)
                    .await
                    .ok_or_else(|| DerivePrivKeyError::Internal(format!("Account {} not found", req.account_id)))?;

                let mut path_to_address = account.account_derivation_path();
                path_to_address.push(req.chain.unwrap_or(Bip44Chain::External).to_child_number());
                path_to_address.push(ChildNumber::new(req.address_id, false).expect("non-hardened"));

                let secret_key = self
                    .priv_key_policy()
                    .hd_wallet_derived_priv_key_or_err(&path_to_address)
                    .map_err(|e| DerivePrivKeyError::Internal(format!("Error deriving secret key: {}", e)))?;

                let private = Private {
                    prefix: self.wif_prefix().unwrap_or(0),
                    secret: secret_key.into(),
                    compressed: true,
                    checksum_type: Default::default(),
                };

                let key_pair = KeyPair::from_private(private)
                    .map_err(|e| DerivePrivKeyError::Internal(format!("Error creating key pair from secret: {}", e)))?;

                let pubkey_slice = key_pair.public_slice();
                let pubkey: [u8; 33] = pubkey_slice
                    .try_into()
                    .map_err(|_| DerivePrivKeyError::Internal("Error converting pubkey slice to array".to_string()))?;

                let address = self
                    .address_from_pubkey(&pubkey.into())
                    .map_err(|e| DerivePrivKeyError::Internal(format!("Error getting address from pubkey: {}", e)))?;

                let priv_key_wif = key_pair.private().to_string();
                let priv_key_hex = format!("0x{}", hex::encode(key_pair.private_bytes()));

                let priv_key = if self.is_utxo() { priv_key_wif } else { priv_key_hex };

                let response = DerivedPrivKey {
                    coin: self.ticker().to_string(),
                    address: address.to_string(),
                    derivation_path: path_to_address.to_string(),
                    priv_key,
                    pub_key: if self.is_utxo() {
                        hex::encode(pubkey)
                    } else {
                        format!("0x{}", hex::encode(pubkey))
                    },
                };
                Ok(response)
            },
        }
    }
}
