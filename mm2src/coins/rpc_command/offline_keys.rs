use crate::CoinProtocol;
use crate::CoinsContext;
use bitcoin_hashes::hex::ToHex;
use bitcrypto::ChecksumType;
use common::HttpStatusCode;
use crypto::privkey::key_pair_from_seed;
use derive_more::Display;
use http::StatusCode;
use keys::{AddressBuilder, AddressFormat, AddressPrefix, NetworkAddressPrefixes, Private};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum KeyExportMode {
    #[serde(rename = "standard")]
    Standard,
    #[serde(rename = "hd")]
    Hd,
    #[serde(rename = "iguana")]
    Iguana,
}

#[derive(Debug, Deserialize)]
pub struct GetPrivateKeysRequest {
    pub coins: Vec<String>,
    #[serde(default = "default_mode")]
    pub mode: KeyExportMode,
    #[serde(default)]
    pub start_index: Option<u32>,
    #[serde(default)]
    pub end_index: Option<u32>,
    #[serde(default)]
    pub account_index: Option<u32>,
}

fn default_mode() -> KeyExportMode { KeyExportMode::Standard }

#[derive(Debug, Deserialize)]
pub struct OfflineKeysRequest {
    pub coins: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CoinKeyInfo {
    pub coin: String,
    pub pubkey: String,
    pub address: String,
    pub priv_key: String,
}

#[derive(Debug, Serialize)]
pub struct HdCoinKeyInfo {
    pub coin: String,
    pub addresses: Vec<HdAddressInfo>,
}

#[derive(Debug, Serialize)]
pub struct HdAddressInfo {
    pub derivation_path: String,
    pub pubkey: String,
    pub address: String,
    pub priv_key: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum GetPrivateKeysResponse {
    Standard(StandardKeysResponse),
    Hd(HdKeysResponse),
}

#[derive(Debug, Serialize)]
pub struct StandardKeysResponse {
    pub result: Vec<CoinKeyInfo>,
}

#[derive(Debug, Serialize)]
pub struct HdKeysResponse {
    pub result: Vec<HdCoinKeyInfo>,
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum OfflineKeysError {
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
    #[display(fmt = "Coin configuration not found for {}", _0)]
    CoinConfigNotFound(String),
    #[display(fmt = "Failed to derive keys for {}: {}", ticker, error)]
    KeyDerivationFailed { ticker: String, error: String },
    #[display(
        fmt = "HD index range is invalid: start_index {} must be less than or equal to end_index {}",
        start_index,
        end_index
    )]
    InvalidHdRange { start_index: u32, end_index: u32 },
    #[display(fmt = "HD index range is too large: maximum range is 100 addresses")]
    HdRangeTooLarge,
    #[display(fmt = "'display_priv_key' is not supported for Hardware Wallets")]
    HardwareWalletNotSupported,
    #[display(fmt = "'display_priv_key' is not supported for MetaMask")]
    MetamaskNotSupported,
    #[display(fmt = "Missing prefix value for {}: {}", ticker, prefix_type)]
    MissingPrefixValue { ticker: String, prefix_type: String },
    #[display(fmt = "Invalid parameters: start_index and end_index are only valid for HD mode")]
    InvalidParametersForMode,
}

#[derive(Debug, Clone)]
enum PrefixValues {
    Utxo { wif_type: u8, pub_type: u8, p2sh_type: u8 },
    NonUtxo,
}

impl HttpStatusCode for OfflineKeysError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::CoinConfigNotFound(_) => StatusCode::BAD_REQUEST,
            Self::KeyDerivationFailed { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::InvalidHdRange { .. } => StatusCode::BAD_REQUEST,
            Self::HdRangeTooLarge => StatusCode::BAD_REQUEST,
            Self::HardwareWalletNotSupported => StatusCode::BAD_REQUEST,
            Self::MetamaskNotSupported => StatusCode::BAD_REQUEST,
            Self::MissingPrefixValue { .. } => StatusCode::BAD_REQUEST,
            Self::InvalidParametersForMode => StatusCode::BAD_REQUEST,
        }
    }
}

fn extract_prefix_values(ticker: &str, coin_conf: &Json) -> Result<PrefixValues, OfflineKeysError> {
    let protocol: CoinProtocol = match serde_json::from_value(coin_conf["protocol"].clone()) {
        Ok(protocol) => protocol,
        Err(_) => {
            return Err(OfflineKeysError::Internal(format!(
                "Failed to parse protocol for {}",
                ticker
            )))
        },
    };

    match protocol {
        CoinProtocol::ETH { .. } | CoinProtocol::ERC20 { .. } | CoinProtocol::NFT { .. } => Ok(PrefixValues::NonUtxo),
        CoinProtocol::UTXO | CoinProtocol::QTUM | CoinProtocol::QRC20 { .. } | CoinProtocol::BCH { .. } => {
            let wif_type = coin_conf["wiftype"]
                .as_u64()
                .ok_or_else(|| OfflineKeysError::MissingPrefixValue {
                    ticker: ticker.to_string(),
                    prefix_type: "wiftype".to_string(),
                })? as u8;

            let pub_type = coin_conf["pubtype"]
                .as_u64()
                .ok_or_else(|| OfflineKeysError::MissingPrefixValue {
                    ticker: ticker.to_string(),
                    prefix_type: "pubtype".to_string(),
                })? as u8;

            let p2sh_type = coin_conf["p2shtype"]
                .as_u64()
                .ok_or_else(|| OfflineKeysError::MissingPrefixValue {
                    ticker: ticker.to_string(),
                    prefix_type: "p2shtype".to_string(),
                })? as u8;

            Ok(PrefixValues::Utxo {
                wif_type,
                pub_type,
                p2sh_type,
            })
        },
        _ => Err(OfflineKeysError::Internal(format!(
            "Unsupported protocol for {}: {:?}",
            ticker, protocol
        ))),
    }
}

fn coin_conf_with_protocol(ctx: &MmArc, ticker: &str, conf_override: Option<Json>) -> Result<(Json, Json), String> {
    let _coins_ctx = CoinsContext::from_ctx(ctx).unwrap();
    let conf = match conf_override {
        Some(override_conf) => override_conf,
        None => match crate::coin_conf(ctx, ticker) {
            Json::Null => {
                return Err(format!("Coin '{}' not found in configuration", ticker));
            },
            conf => conf,
        },
    };
    let protocol = conf["protocol"].clone();
    Ok((conf, protocol))
}

pub async fn offline_keys_export_internal(
    ctx: MmArc,
    req: OfflineKeysRequest,
) -> Result<StandardKeysResponse, MmError<OfflineKeysError>> {
    let mut result = Vec::with_capacity(req.coins.len());

    for ticker in &req.coins {
        let (coin_conf, _) = coin_conf_with_protocol(&ctx, ticker, None)
            .map_err(|_| OfflineKeysError::CoinConfigNotFound(ticker.clone()))?;

        if let Some(wallet_type) = coin_conf["wallet_type"].as_str() {
            if wallet_type == "trezor" {
                return MmError::err(OfflineKeysError::HardwareWalletNotSupported);
            }
            if wallet_type == "metamask" {
                return MmError::err(OfflineKeysError::MetamaskNotSupported);
            }
        }

        let passphrase = ctx.conf["passphrase"].as_str().unwrap_or("");

        let key_pair = {
            match key_pair_from_seed(passphrase) {
                Ok(kp) => kp,
                Err(e) => {
                    return MmError::err(OfflineKeysError::KeyDerivationFailed {
                        ticker: ticker.clone(),
                        error: e.to_string(),
                    })
                },
            }
        };

        let prefix_values = extract_prefix_values(ticker, &coin_conf)?;

        let pubkey = key_pair.public().to_vec().to_hex().to_string();

        let (address, priv_key) = match prefix_values {
            PrefixValues::Utxo {
                wif_type,
                pub_type,
                p2sh_type,
            } => {
                let private = Private {
                    prefix: wif_type,
                    secret: key_pair.private().secret.clone(),
                    compressed: true,
                    checksum_type: ChecksumType::DSHA256,
                };

                let address_prefixes = NetworkAddressPrefixes {
                    p2pkh: AddressPrefix::from([pub_type]),
                    p2sh: AddressPrefix::from([p2sh_type]),
                };

                let address =
                    AddressBuilder::new(AddressFormat::Standard, ChecksumType::DSHA256, address_prefixes, None)
                        .as_pkh_from_pk(*key_pair.public())
                        .build()
                        .map_err(|e| OfflineKeysError::Internal(e.to_string()))?;

                (address.to_string(), private.to_string())
            },
            PrefixValues::NonUtxo => {
                let protocol: CoinProtocol = serde_json::from_value(coin_conf["protocol"].clone())
                    .map_err(|_| OfflineKeysError::Internal(format!("Failed to parse protocol for {}", ticker)))?;

                let address = match protocol {
                    CoinProtocol::ETH { .. } | CoinProtocol::ERC20 { .. } | CoinProtocol::NFT { .. } => {
                        crate::eth::addr_from_pubkey_str(&pubkey)
                            .map_err(|e| OfflineKeysError::Internal(e.to_string()))?
                    },
                    _ => {
                        return MmError::err(OfflineKeysError::Internal(format!(
                            "Unsupported non-UTXO protocol: {:?}",
                            protocol
                        )))
                    },
                };

                let priv_key = format!("0x{}", key_pair.private().secret.to_hex());

                (address, priv_key)
            },
        };

        result.push(CoinKeyInfo {
            coin: ticker.clone(),
            pubkey,
            address,
            priv_key,
        });
    }

    Ok(StandardKeysResponse { result })
}

async fn offline_hd_keys_export_internal(
    ctx: MmArc,
    coins: Vec<String>,
    start_index: u32,
    end_index: u32,
    account_index: u32,
) -> Result<HdKeysResponse, MmError<OfflineKeysError>> {
    if start_index > end_index {
        return MmError::err(OfflineKeysError::InvalidHdRange { start_index, end_index });
    }

    if end_index - start_index > 100 {
        return MmError::err(OfflineKeysError::HdRangeTooLarge);
    }

    let mut result = Vec::with_capacity(coins.len());

    for ticker in &coins {
        let (coin_conf, _) = coin_conf_with_protocol(&ctx, ticker, None)
            .map_err(|_| OfflineKeysError::CoinConfigNotFound(ticker.clone()))?;

        if let Some(wallet_type) = coin_conf["wallet_type"].as_str() {
            if wallet_type == "trezor" {
                return MmError::err(OfflineKeysError::HardwareWalletNotSupported);
            }
            if wallet_type == "metamask" {
                return MmError::err(OfflineKeysError::MetamaskNotSupported);
            }
        }

        let prefix_values = extract_prefix_values(ticker, &coin_conf)?;

        if coin_conf["derivation_path"].is_null() {
            return MmError::err(OfflineKeysError::KeyDerivationFailed {
                ticker: ticker.clone(),
                error: "Derivation path not defined for this coin. HD mode requires a valid derivation_path in the coin configuration.".to_string(),
            });
        }

        let base_derivation_path =
            coin_conf["derivation_path"]
                .as_str()
                .ok_or_else(|| OfflineKeysError::KeyDerivationFailed {
                    ticker: ticker.clone(),
                    error: "Invalid derivation_path format in coin configuration".to_string(),
                })?;

        let mut addresses = Vec::with_capacity((end_index - start_index + 1) as usize);

        let passphrase = ctx.conf["passphrase"].as_str().unwrap_or("");

        for index in start_index..=end_index {
            let seed = format!(
                "{}{}/{}_{}_{}_{}",
                passphrase, ticker, account_index, index, ticker, index
            );

            let key_pair = {
                match key_pair_from_seed(&seed) {
                    Ok(kp) => kp,
                    Err(e) => {
                        return MmError::err(OfflineKeysError::KeyDerivationFailed {
                            ticker: ticker.clone(),
                            error: format!("Failed to derive HD key at index {}: {}", index, e),
                        })
                    },
                }
            };

            let pubkey = key_pair.public().to_vec().to_hex().to_string();

            let (address, priv_key) = match &prefix_values {
                PrefixValues::Utxo {
                    wif_type,
                    pub_type,
                    p2sh_type,
                } => {
                    let private = Private {
                        prefix: *wif_type,
                        secret: key_pair.private().secret.clone(),
                        compressed: true,
                        checksum_type: ChecksumType::DSHA256,
                    };

                    let address_prefixes = NetworkAddressPrefixes {
                        p2pkh: AddressPrefix::from([*pub_type]),
                        p2sh: AddressPrefix::from([*p2sh_type]),
                    };

                    let address =
                        AddressBuilder::new(AddressFormat::Standard, ChecksumType::DSHA256, address_prefixes, None)
                            .as_pkh_from_pk(*key_pair.public())
                            .build()
                            .map_err(|e| OfflineKeysError::Internal(e.to_string()))?;

                    (address.to_string(), private.to_string())
                },
                PrefixValues::NonUtxo => {
                    let protocol: CoinProtocol = serde_json::from_value(coin_conf["protocol"].clone())
                        .map_err(|_| OfflineKeysError::Internal(format!("Failed to parse protocol for {}", ticker)))?;

                    let address = match protocol {
                        CoinProtocol::ETH { .. } | CoinProtocol::ERC20 { .. } | CoinProtocol::NFT { .. } => {
                            crate::eth::addr_from_pubkey_str(&pubkey)
                                .map_err(|e| OfflineKeysError::Internal(e.to_string()))?
                        },
                        _ => {
                            return MmError::err(OfflineKeysError::Internal(format!(
                                "Unsupported non-UTXO protocol: {:?}",
                                protocol
                            )))
                        },
                    };

                    let priv_key = format!("0x{}", key_pair.private().secret.to_hex());

                    (address, priv_key)
                },
            };

            let derivation_path = format!("{}/{}/0/{}", base_derivation_path, account_index, index);

            addresses.push(HdAddressInfo {
                derivation_path,
                pubkey,
                address,
                priv_key,
            });
        }

        result.push(HdCoinKeyInfo {
            coin: ticker.clone(),
            addresses,
        });
    }

    Ok(HdKeysResponse { result })
}

async fn offline_iguana_keys_export_internal(
    ctx: MmArc,
    req: OfflineKeysRequest,
) -> Result<StandardKeysResponse, MmError<OfflineKeysError>> {
    let mut result = Vec::with_capacity(req.coins.len());

    for ticker in &req.coins {
        let (coin_conf, _) = coin_conf_with_protocol(&ctx, ticker, None)
            .map_err(|_| OfflineKeysError::CoinConfigNotFound(ticker.clone()))?;

        if let Some(wallet_type) = coin_conf["wallet_type"].as_str() {
            if wallet_type == "trezor" {
                return MmError::err(OfflineKeysError::HardwareWalletNotSupported);
            }
            if wallet_type == "metamask" {
                return MmError::err(OfflineKeysError::MetamaskNotSupported);
            }
        }

        let prefix_values = extract_prefix_values(ticker, &coin_conf)?;

        let passphrase = ctx.conf["passphrase"].as_str().unwrap_or("");

        let key_pair = {
            match key_pair_from_seed(passphrase) {
                Ok(kp) => kp,
                Err(e) => {
                    return MmError::err(OfflineKeysError::KeyDerivationFailed {
                        ticker: ticker.clone(),
                        error: e.to_string(),
                    })
                },
            }
        };

        let pubkey = key_pair.public().to_vec().to_hex().to_string();

        let (address, priv_key) = match prefix_values {
            PrefixValues::Utxo {
                wif_type,
                pub_type,
                p2sh_type,
            } => {
                let private = Private {
                    prefix: wif_type,
                    secret: key_pair.private().secret.clone(),
                    compressed: true,
                    checksum_type: ChecksumType::DSHA256,
                };

                let address_prefixes = NetworkAddressPrefixes {
                    p2pkh: AddressPrefix::from([pub_type]),
                    p2sh: AddressPrefix::from([p2sh_type]),
                };

                let address =
                    AddressBuilder::new(AddressFormat::Standard, ChecksumType::DSHA256, address_prefixes, None)
                        .as_pkh_from_pk(*key_pair.public())
                        .build()
                        .map_err(|e| OfflineKeysError::Internal(e.to_string()))?;

                (address.to_string(), private.to_string())
            },
            PrefixValues::NonUtxo => {
                let protocol: CoinProtocol = serde_json::from_value(coin_conf["protocol"].clone())
                    .map_err(|_| OfflineKeysError::Internal(format!("Failed to parse protocol for {}", ticker)))?;

                let address = match protocol {
                    CoinProtocol::ETH { .. } | CoinProtocol::ERC20 { .. } | CoinProtocol::NFT { .. } => {
                        crate::eth::addr_from_pubkey_str(&pubkey)
                            .map_err(|e| OfflineKeysError::Internal(e.to_string()))?
                    },
                    _ => {
                        return MmError::err(OfflineKeysError::Internal(format!(
                            "Unsupported non-UTXO protocol: {:?}",
                            protocol
                        )))
                    },
                };

                let priv_key = format!("0x{}", key_pair.private().secret.to_hex());

                (address, priv_key)
            },
        };

        result.push(CoinKeyInfo {
            coin: ticker.clone(),
            pubkey,
            address,
            priv_key,
        });
    }

    Ok(StandardKeysResponse { result })
}

pub async fn get_private_keys(
    ctx: MmArc,
    req: GetPrivateKeysRequest,
) -> Result<GetPrivateKeysResponse, MmError<OfflineKeysError>> {
    match req.mode {
        KeyExportMode::Standard => {
            if req.start_index.is_some() || req.end_index.is_some() || req.account_index.is_some() {
                return MmError::err(OfflineKeysError::InvalidParametersForMode);
            }
            let offline_req = OfflineKeysRequest { coins: req.coins };
            let response = offline_keys_export_internal(ctx, offline_req).await?;
            Ok(GetPrivateKeysResponse::Standard(response))
        },
        KeyExportMode::Hd => {
            let start_index = req.start_index.unwrap_or(0);
            let end_index = req.end_index.unwrap_or_else(|| {
                start_index.saturating_add(10)
            });
            let account_index = req.account_index.unwrap_or(0);

            if start_index > end_index {
                return MmError::err(OfflineKeysError::InvalidHdRange { start_index, end_index });
            }

            if end_index.saturating_sub(start_index) > 100 {
                return MmError::err(OfflineKeysError::HdRangeTooLarge);
            }

            let response =
                offline_hd_keys_export_internal(ctx, req.coins, start_index, end_index, account_index).await?;
            Ok(GetPrivateKeysResponse::Hd(response))
        },
        KeyExportMode::Iguana => {
            if req.start_index.is_some() || req.end_index.is_some() || req.account_index.is_some() {
                return MmError::err(OfflineKeysError::InvalidParametersForMode);
            }
            let offline_req = OfflineKeysRequest { coins: req.coins };
            let response = offline_iguana_keys_export_internal(ctx, offline_req).await?;
            Ok(GetPrivateKeysResponse::Standard(response))
        },
    }
}
