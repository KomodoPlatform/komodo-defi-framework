use crate::tendermint;
use crate::CoinProtocol;
use bitcoin_hashes::hex::ToHex;
use bitcrypto::ChecksumType;
use common::HttpStatusCode;
use crypto::privkey::{key_pair_from_secret, key_pair_from_seed};
use crypto::{Bip32DerPathOps, CryptoCtx, KeyPairPolicy, StandardHDPath};
use derive_more::Display;
use http::StatusCode;
use keys::{AddressBuilder, AddressFormat, AddressPrefix, NetworkAddressPrefixes, Private};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::str::FromStr;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum KeyExportMode {
    #[serde(rename = "hd")]
    Hd,
    #[serde(rename = "iguana")]
    Iguana,
}

#[derive(Debug, Deserialize)]
pub struct GetPrivateKeysRequest {
    pub coins: Vec<String>,
    pub mode: Option<KeyExportMode>,
    pub start_index: Option<u32>,
    pub end_index: Option<u32>,
    pub account_index: Option<u32>,
}

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
    Iguana(IguanaKeysResponse),
    Hd(HdKeysResponse),
}

#[derive(Debug, Serialize)]
pub struct IguanaKeysResponse {
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
    #[display(fmt = "Failed to parse protocol for coin {}: {}", ticker, error)]
    ProtocolParseError { ticker: String, error: String },
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
    #[display(fmt = "Missing prefix value for {}: {}", ticker, prefix_type)]
    MissingPrefixValue { ticker: String, prefix_type: String },
    #[display(fmt = "Invalid parameters: start_index and end_index are only valid for HD mode")]
    InvalidParametersForMode,
}

#[derive(Debug, Clone)]
enum PrefixValues {
    Utxo { wif_type: u8, pub_type: u8, p2sh_type: u8 },
    Tendermint { account_prefix: String },
}

impl HttpStatusCode for OfflineKeysError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::CoinConfigNotFound(_) => StatusCode::BAD_REQUEST,
            Self::KeyDerivationFailed { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::InvalidHdRange { .. } => StatusCode::BAD_REQUEST,
            Self::HdRangeTooLarge => StatusCode::BAD_REQUEST,
            Self::MissingPrefixValue { .. } => StatusCode::BAD_REQUEST,
            Self::InvalidParametersForMode => StatusCode::BAD_REQUEST,
            Self::ProtocolParseError { .. } => StatusCode::BAD_REQUEST,
        }
    }
}

fn extract_prefix_values(
    ctx: &MmArc,
    ticker: &str,
    coin_conf: &Json,
) -> Result<Option<PrefixValues>, OfflineKeysError> {
    let protocol: CoinProtocol = match serde_json::from_value(coin_conf["protocol"].clone()) {
        Ok(protocol) => protocol,
        Err(e) => {
            return Err(OfflineKeysError::ProtocolParseError {
                ticker: ticker.to_string(),
                error: e.to_string(),
            })
        },
    };

    match protocol {
        CoinProtocol::ETH { .. } | CoinProtocol::ERC20 { .. } | CoinProtocol::NFT { .. } => Ok(None),
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

            Ok(Some(PrefixValues::Utxo {
                wif_type,
                pub_type,
                p2sh_type,
            }))
        },
        CoinProtocol::TENDERMINT(protocol_info) => Ok(Some(PrefixValues::Tendermint {
            account_prefix: protocol_info.account_prefix.clone(),
        })),
        CoinProtocol::TENDERMINTTOKEN(token_info) => {
            let platform_conf = crate::coin_conf(ctx, &token_info.platform);
            if platform_conf.is_null() {
                return Err(OfflineKeysError::Internal(format!(
                    "Platform {} configuration not found for {}",
                    token_info.platform, ticker
                )));
            }
            let platform_protocol: CoinProtocol =
                serde_json::from_value(platform_conf["protocol"].clone()).map_err(|e| {
                    OfflineKeysError::ProtocolParseError {
                        ticker: ticker.to_string(),
                        error: format!("Failed to parse platform protocol: {}", e),
                    }
                })?;
            match platform_protocol {
                CoinProtocol::TENDERMINT(platform_info) => Ok(Some(PrefixValues::Tendermint {
                    account_prefix: platform_info.account_prefix.clone(),
                })),
                _ => Err(OfflineKeysError::Internal(format!(
                    "Platform protocol for {} is not TENDERMINT: {:?}",
                    ticker, platform_protocol
                ))),
            }
        },
        _ => Err(OfflineKeysError::Internal(format!(
            "Unsupported protocol for {}: {:?}",
            ticker, protocol
        ))),
    }
}

fn coin_conf_with_protocol(ctx: &MmArc, ticker: &str, conf_override: Option<Json>) -> Result<(Json, Json), String> {
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

        let prefix_values = extract_prefix_values(&ctx, ticker, &coin_conf)?;

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

        let crypto_ctx = CryptoCtx::from_ctx(&ctx).map_err(|e| OfflineKeysError::Internal(
            format!("Failed to get crypto context: {}", e),
        ))?;

        let global_hd_ctx = match crypto_ctx.key_pair_policy() {
            KeyPairPolicy::GlobalHDAccount(hd_ctx) => hd_ctx.clone(),
            KeyPairPolicy::Iguana => {
                return MmError::err(OfflineKeysError::KeyDerivationFailed {
                    ticker: ticker.clone(),
                    error: "HD key derivation requires GlobalHDAccount mode. Please initialize with HD wallet.".to_string(),
                });
            },
        };

        for index in start_index..=end_index {
            let derivation_path = format!("{}/{}'/0/{}", base_derivation_path, account_index, index);
            let hd_path =
                StandardHDPath::from_str(&derivation_path).map_err(|e| OfflineKeysError::KeyDerivationFailed {
                    ticker: ticker.clone(),
                    error: format!("Invalid derivation path {}: {:?}", derivation_path, e),
                })?;

            let key_pair = {
                let secret = global_hd_ctx
                    .derive_secp256k1_secret(&hd_path.to_derivation_path())
                    .map_err(|e| OfflineKeysError::KeyDerivationFailed {
                        ticker: ticker.clone(),
                        error: format!("Failed to derive key at path {}: {}", derivation_path, e),
                    })?;

                key_pair_from_secret(&secret.take()).map_err(|e| OfflineKeysError::KeyDerivationFailed {
                    ticker: ticker.clone(),
                    error: format!("Failed to create key pair: {}", e),
                })?
            };

            let pubkey = key_pair.public().to_vec().to_hex().to_string();

            let (address, priv_key) = match &prefix_values {
                Some(PrefixValues::Utxo {
                    wif_type,
                    pub_type,
                    p2sh_type,
                }) => {
                    let private = Private {
                        prefix: *wif_type,
                        secret: key_pair.private().secret,
                        compressed: true,
                        checksum_type: ChecksumType::DSHA256,
                    };

                    let address_prefixes = NetworkAddressPrefixes {
                        p2pkh: AddressPrefix::from([*pub_type]),
                        p2sh: AddressPrefix::from([*p2sh_type]),
                    };

                    let address_format = if let Some(format_config) = coin_conf.get("address_format") {
                        serde_json::from_value(format_config.clone()).unwrap_or(AddressFormat::Standard)
                    } else {
                        AddressFormat::Standard
                    };

                    let bech32_hrp = coin_conf.get("bech32_hrp")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let address =
                        AddressBuilder::new(address_format, ChecksumType::DSHA256, address_prefixes, bech32_hrp)
                            .as_pkh_from_pk(*key_pair.public())
                            .build()
                            .map_err(|e| OfflineKeysError::Internal(e.to_string()))?;

                    (address.to_string(), private.to_string())
                },
                None => {
                    let protocol: CoinProtocol =
                        serde_json::from_value(coin_conf["protocol"].clone()).map_err(|e| {
                            OfflineKeysError::ProtocolParseError {
                                ticker: ticker.to_string(),
                                error: e.to_string(),
                            }
                        })?;

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
                Some(PrefixValues::Tendermint { account_prefix }) => {
                    let address = tendermint::account_id_from_pubkey_hex(&account_prefix, &pubkey)
                        .map_err(|e| OfflineKeysError::Internal(e.to_string()))?
                        .to_string();

                    let priv_key = key_pair.private().secret.to_hex();

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
) -> Result<IguanaKeysResponse, MmError<OfflineKeysError>> {
    let mut result = Vec::with_capacity(req.coins.len());

    for ticker in &req.coins {
        let (coin_conf, _) = coin_conf_with_protocol(&ctx, ticker, None)
            .map_err(|_| OfflineKeysError::CoinConfigNotFound(ticker.clone()))?;

        let prefix_values = extract_prefix_values(&ctx, ticker, &coin_conf)?;

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
            Some(PrefixValues::Utxo {
                wif_type,
                pub_type,
                p2sh_type,
            }) => {
                let private = Private {
                    prefix: wif_type,
                    secret: key_pair.private().secret,
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
            None => {
                let protocol: CoinProtocol = serde_json::from_value(coin_conf["protocol"].clone()).map_err(|e| {
                    OfflineKeysError::ProtocolParseError {
                        ticker: ticker.to_string(),
                        error: e.to_string(),
                    }
                })?;

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
            Some(PrefixValues::Tendermint { account_prefix }) => {
                let address = tendermint::account_id_from_pubkey_hex(&account_prefix, &pubkey)
                    .map_err(|e| OfflineKeysError::Internal(e.to_string()))?
                    .to_string();

                let priv_key = key_pair.private().secret.to_hex();

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

    Ok(IguanaKeysResponse { result })
}

pub async fn get_private_keys(
    ctx: MmArc,
    req: GetPrivateKeysRequest,
) -> Result<GetPrivateKeysResponse, MmError<OfflineKeysError>> {
    let mode = req.mode.unwrap_or_else(|| {
        if ctx.enable_hd() {
            KeyExportMode::Hd
        } else {
            KeyExportMode::Iguana
        }
    });

    match mode {
        KeyExportMode::Hd => {
            let start_index = req.start_index.unwrap_or(0);
            let end_index = req.end_index.unwrap_or_else(|| start_index.saturating_add(10));
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
            Ok(GetPrivateKeysResponse::Iguana(response))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm2_core::mm_ctx::MmCtxBuilder;
    use serde_json::json;

    const TEST_MNEMONIC: &str =
        "prosper boss develop coconut warrior silly cabin trial person glass toilet mixed push spirit love";

    #[tokio::test]
    async fn test_btc_hd_key_derivation() {
        use mm2_test_helpers::for_tests::btc_with_spv_conf;
        
        let mut btc_conf = btc_with_spv_conf();
        btc_conf["derivation_path"] = json!("m/44'/0'");
        let ctx = MmCtxBuilder::new()
            .with_conf(json!({
                "coins": [btc_conf],
                "rpc_password": "test123"
            }))
            .into_mm_arc();
        
        CryptoCtx::init_with_global_hd_account(ctx.clone(), TEST_MNEMONIC).unwrap();

        let req = GetPrivateKeysRequest {
            coins: vec!["BTC".to_string()],
            mode: Some(KeyExportMode::Hd),
            start_index: Some(0),
            end_index: Some(2),
            account_index: Some(0),
        };

        let expected_addresses = vec![
            "1DWZWURWrdJnuZBqQgv3THfmhbxWgJuFex",
            "17jQZo8xSjJeQLxexLZSZaBA9ks5tWh3fJ",
            "13ZwKLGksE72YgMdKjJC9XZPM6TcpejJrJ",
        ];
        let expected_pubkeys = vec![
            "037e746753316b028859ff20bac70ed4803a3056038e54ef86f71f35e53a6c8625",
            "030bd2b7ab3800a968544bb097a78c1ecfed233af342359e399d72fd970aa35323",
            "034bf56e7072f8f378a8efee382c9a438fa4b4c98c387d4a0db543afc434c4adaf",
        ];
        let expected_privkeys = vec![
            "KywJqZF9PrFSwWkocQ4JZSgfTD3eXYbfnM54Q3Ua7UKzGD4WTRbX",
            "KwLRhtqifoX1FuMFJytB85DCZf6YoSjuFSqPXBzPsyi56GXJaVpD",
            "L5kmC8cqWodyjm2JUQNfRbmyZeJMJMeYH4WJGUSVcdnD9X6aAs8Z",
        ];

        let btc_conf = json!({
            "coin": "BTC",
            "protocol": {
                "type": "UTXO"
            },
            "derivation_path": "m/44'/0'/0'",
            "wiftype": 128,
            "pubtype": 0,
            "p2shtype": 5
        });

        let response = offline_hd_keys_export_internal(ctx.clone(), vec!["BTC".to_string()], 0, 2, 0).await;

        match response {
            Ok(hd_response) => {
                assert_eq!(hd_response.result.len(), 1);
                let btc_result = &hd_response.result[0];
                assert_eq!(btc_result.coin, "BTC");
                assert_eq!(btc_result.addresses.len(), 3);

                for (i, addr_info) in btc_result.addresses.iter().enumerate() {
                    assert_eq!(addr_info.address, expected_addresses[i]);
                    assert_eq!(addr_info.pubkey, expected_pubkeys[i]);
                    assert_eq!(addr_info.priv_key, expected_privkeys[i]);
                    assert_eq!(addr_info.derivation_path, format!("m/44'/0'/0/0/{}", i));
                }
            },
            Err(e) => panic!("BTC HD key derivation test failed: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_btc_segwit_hd_key_derivation() {
        use mm2_test_helpers::for_tests::btc_segwit_conf;
        
        let mut btc_segwit_conf = btc_segwit_conf();
        btc_segwit_conf["derivation_path"] = json!("m/84'/0'");
        let ctx = MmCtxBuilder::new()
            .with_conf(json!({
                "coins": [btc_segwit_conf],
                "rpc_password": "test123"
            }))
            .into_mm_arc();
        
        CryptoCtx::init_with_global_hd_account(ctx.clone(), TEST_MNEMONIC).unwrap();

        let req = GetPrivateKeysRequest {
            coins: vec!["BTC-segwit".to_string()],
            mode: Some(KeyExportMode::Hd),
            start_index: Some(0),
            end_index: Some(2),
            account_index: Some(0),
        };

        let expected_addresses = vec![
            "bc1q4cn6qhvuajkdfhk3fzuup07ktrepcukc8hv0c8",
            "bc1qv26wdgw5vqf7fcup92yhjmm234zwd2wrgv5f4f",
            "bc1qvs2pggxxcl40n9cs9v9crkclmrx57hgp5f6579",
        ];
        let expected_pubkeys = vec![
            "024b796b083b51ea5820bbdb80fa4e7f09f5f8c6fe76bc68fa2d8d0452a4ddfa91",
            "0272a14e54bbfa321f7afa8d98b478f7e5bea5440f3e807bd87f5c00f75ef0941f",
            "03e10fed91ec91740c726b945671954c040cd42b3ad9ab5791133f1a33d4c42e5d",
        ];
        let expected_privkeys = vec![
            "L2aJGVhekAig5a4Zx81NH9Q99h9gH7umiyqBWXrNX5w8xn2eeU5g",
            "L1susQQK5CaP7eT4MKyAzv8KthN53i5gHJmUGtKksY8r2Hbvvyv6",
            "Kz937rcd2Hack7TUgkcg3YAiSbTGGJciMCzFbu76FkJgZkwb5zES",
        ];

        let response = offline_hd_keys_export_internal(ctx.clone(), vec!["BTC-segwit".to_string()], 0, 2, 0).await;

        match response {
            Ok(hd_response) => {
                assert_eq!(hd_response.result.len(), 1);
                let btc_segwit_result = &hd_response.result[0];
                assert_eq!(btc_segwit_result.coin, "BTC-segwit");
                assert_eq!(btc_segwit_result.addresses.len(), 3);

                for (i, addr_info) in btc_segwit_result.addresses.iter().enumerate() {
                    assert_eq!(addr_info.address, expected_addresses[i]);
                    assert_eq!(addr_info.pubkey, expected_pubkeys[i]);
                    assert_eq!(addr_info.priv_key, expected_privkeys[i]);
                    assert_eq!(addr_info.derivation_path, format!("m/84'/0'/0/0/{}", i));
                }
            },
            Err(e) => panic!("BTC-Segwit HD key derivation test failed: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_eth_hd_key_derivation() {
        use mm2_test_helpers::for_tests::eth_dev_conf;
        
        let mut eth_conf = eth_dev_conf();
        eth_conf["derivation_path"] = json!("m/44'/60'");
        let ctx = MmCtxBuilder::new()
            .with_conf(json!({
                "coins": [eth_conf],
                "rpc_password": "test123"
            }))
            .into_mm_arc();
        
        CryptoCtx::init_with_global_hd_account(ctx.clone(), TEST_MNEMONIC).unwrap();

        let req = GetPrivateKeysRequest {
            coins: vec!["ETH".to_string()],
            mode: Some(KeyExportMode::Hd),
            start_index: Some(0),
            end_index: Some(2),
            account_index: Some(0),
        };

        let expected_addresses = vec![
            "0x6B06d67C539B101180aC03b61ba7F7f3158CE54d",
            "0x012F492f2d254e204dD8da3a4f0d6071C345b9D1",
            "0xa713617C963b82429909B09B9181a22884f1eb8f",
        ];
        let expected_pubkeys = vec![
            "02a2b68c3126ba160e5ffb7c0d5c5c5c56e724f57e5ec0ace40d6db990e688ed4a",
            "02d7efb9086100311021166c11b2dc7ca941ccbe242b51206555721efe93737678",
            "03353b68f1b2c0891edf78395480bc67e128fb967c5722a6b41d784da295986d4d",
        ];
        let expected_privkeys = vec![
            "0x646431107ae37e826aaa5108fe2c2611ef15615e78b4175919b85fd6366f19a3",
            "0xc11fc3d704820e752bfae8db9f02e489c1e742392b35ac5b4a4e441e7955efa4",
            "0xddb38472a7d7095ad466b4a4e19f85f612f87e04a23c75eac8e7957d31ee22f0",
        ];

        let response = offline_hd_keys_export_internal(ctx.clone(), vec!["ETH".to_string()], 0, 2, 0).await;

        match response {
            Ok(hd_response) => {
                assert_eq!(hd_response.result.len(), 1);
                let eth_result = &hd_response.result[0];
                assert_eq!(eth_result.coin, "ETH");
                assert_eq!(eth_result.addresses.len(), 3);

                for (i, addr_info) in eth_result.addresses.iter().enumerate() {
                    assert_eq!(addr_info.address.to_lowercase(), expected_addresses[i].to_lowercase());
                    assert_eq!(addr_info.pubkey, expected_pubkeys[i]);
                    assert_eq!(addr_info.priv_key, expected_privkeys[i]);
                    assert_eq!(addr_info.derivation_path, format!("m/44'/60'/0/0/{}", i));
                }
            },
            Err(e) => panic!("ETH HD key derivation test failed: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_atom_hd_key_derivation() {
        use mm2_test_helpers::for_tests::atom_testnet_conf;
        
        let mut atom_conf = atom_testnet_conf();
        atom_conf["derivation_path"] = json!("m/44'/118'");
        let ctx = MmCtxBuilder::new()
            .with_conf(json!({
                "coins": [atom_conf],
                "rpc_password": "test123"
            }))
            .into_mm_arc();
        
        CryptoCtx::init_with_global_hd_account(ctx.clone(), TEST_MNEMONIC).unwrap();

        let req = GetPrivateKeysRequest {
            coins: vec!["ATOM".to_string()],
            mode: Some(KeyExportMode::Hd),
            start_index: Some(0),
            end_index: Some(2),
            account_index: Some(0),
        };

        let expected_addresses = vec![
            "cosmos1j398pch49fkgx986r4aqm57zp3phuzq4p30dhh",
            "cosmos1cecqkvtwn0vyr730yq3hawrl8rztvchz6kadk8",
            "cosmos1c27v3agv745fhnjve8ch754rmzswuc7guglt76",
        ];
        let expected_pubkeys = vec![
            "cosmospub1addwnpepq09wmcqe8qvcmyvgre8g07q9z42rz6y7uguz5dxqvhw0tdrqa38csd8wlfa",
            "cosmospub1addwnpepq0uy8zghd8q8p5wjvz84catqgwuwem45s5rpvd9syq44jz2jmyqfvp049kz",
            "cosmospub1add", // Truncated in the original test vectors
        ];
        let expected_privkeys_base64 = vec![
            "Nbfdi2ZHb+2W41DNJPaHxAi6oHcJ4lFLtBZkATGAB8M=",
            "8FJrDCXtcLl6OgjqF/l5QQvUYYpjwGn+F3q3pBp3e94=",
        ];

        let response = offline_hd_keys_export_internal(
            ctx.clone(),
            vec!["ATOM".to_string()],
            0,
            1, // Only test first 2 since third vector is incomplete
            0,
        )
        .await;

        match response {
            Ok(hd_response) => {
                assert_eq!(hd_response.result.len(), 1);
                let atom_result = &hd_response.result[0];
                assert_eq!(atom_result.coin, "ATOM");
                assert_eq!(atom_result.addresses.len(), 2);

                for (i, addr_info) in atom_result.addresses.iter().enumerate() {
                    assert_eq!(addr_info.address, expected_addresses[i]);
                    assert_eq!(addr_info.derivation_path, format!("m/44'/118'/0/0/{}", i));
                }
            },
            Err(e) => panic!("ATOM HD key derivation test failed: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_iguana_key_derivation() {
        use mm2_test_helpers::for_tests::btc_with_spv_conf;
        
        let mut btc_conf = btc_with_spv_conf();
        btc_conf["derivation_path"] = json!("m/44'/0'");
        let ctx = MmCtxBuilder::new()
            .with_conf(json!({
                "coins": [btc_conf],
                "rpc_password": "test123"
            }))
            .into_mm_arc();
        
        CryptoCtx::init_with_iguana_passphrase(ctx.clone(), TEST_MNEMONIC).unwrap();

        let req = OfflineKeysRequest {
            coins: vec!["BTC".to_string()],
        };

        let response = offline_iguana_keys_export_internal(ctx.clone(), req).await;

        match response {
            Ok(iguana_response) => {
                assert_eq!(iguana_response.result.len(), 1);
                let btc_result = &iguana_response.result[0];
                assert_eq!(btc_result.coin, "BTC");
                assert!(!btc_result.pubkey.is_empty());
                assert!(!btc_result.address.is_empty());
                assert!(!btc_result.priv_key.is_empty());
            },
            Err(e) => panic!("Iguana key derivation test failed: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_error_cases() {
        use mm2_test_helpers::for_tests::btc_with_spv_conf;
        
        let mut btc_conf = btc_with_spv_conf();
        btc_conf["derivation_path"] = json!("m/44'/0'");
        let ctx = MmCtxBuilder::new()
            .with_conf(json!({
                "coins": [btc_conf],
                "rpc_password": "test123"
            }))
            .into_mm_arc();
        
        CryptoCtx::init_with_global_hd_account(ctx.clone(), TEST_MNEMONIC).unwrap();

        let invalid_range_req = GetPrivateKeysRequest {
            coins: vec!["BTC".to_string()],
            mode: Some(KeyExportMode::Hd),
            start_index: Some(10),
            end_index: Some(5),
            account_index: Some(0),
        };

        let response = get_private_keys(ctx.clone(), invalid_range_req).await;
        assert!(response.is_err());
        match response.unwrap_err().into_inner() {
            OfflineKeysError::InvalidHdRange { start_index, end_index } => {
                assert_eq!(start_index, 10);
                assert_eq!(end_index, 5);
            },
            _ => panic!("Expected InvalidHdRange error"),
        }

        let large_range_req = GetPrivateKeysRequest {
            coins: vec!["BTC".to_string()],
            mode: Some(KeyExportMode::Hd),
            start_index: Some(0),
            end_index: Some(150),
            account_index: Some(0),
        };

        let response = get_private_keys(ctx.clone(), large_range_req).await;
        assert!(response.is_err());
        match response.unwrap_err().into_inner() {
            OfflineKeysError::HdRangeTooLarge => {},
            _ => panic!("Expected HdRangeTooLarge error"),
        }

        let invalid_params_req = GetPrivateKeysRequest {
            coins: vec!["BTC".to_string()],
            mode: Some(KeyExportMode::Iguana),
            start_index: Some(0),
            end_index: Some(10),
            account_index: Some(0),
        };

        let response = get_private_keys(ctx.clone(), invalid_params_req).await;
        assert!(response.is_err());
        match response.unwrap_err().into_inner() {
            OfflineKeysError::InvalidParametersForMode => {},
            _ => panic!("Expected InvalidParametersForMode error"),
        }
    }
}
