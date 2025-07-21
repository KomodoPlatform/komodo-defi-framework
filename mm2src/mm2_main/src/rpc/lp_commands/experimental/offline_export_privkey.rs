use bitcrypto::ChecksumType;
use coins::{address_by_coin_conf_and_pubkey_str, coin_conf, utxo, CoinProtocol};
use common::HttpStatusCode;
use crypto::{privkey::{key_pair_from_secret, key_pair_from_seed},
             CryptoCtx, DerivationPath, KeyPairPolicy};
use keys::Private;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json as json;
use std::str::FromStr;

#[derive(Clone, Debug, Deserialize)]
pub struct OfflineExportPrivkeyRequest {
    pub coin: String,
    /// Starting address index (defaults to 0)
    #[serde(default)]
    pub start_index: u32,
    /// Ending address index (defaults to start_index for single address)
    /// If provided, will export keys for range [start_index, end_index] inclusive
    pub end_index: Option<u32>,
    /// Account ID for HD derivation (defaults to 0)
    #[serde(default)]
    pub account_id: u32,
    /// Whether to export change addresses (internal chain) instead of receive addresses (external chain)
    #[serde(default)]
    pub is_change: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AddressKeyPair {
    pub account_id: u32,
    pub is_change: bool,
    pub address_index: u32,
    pub derivation_path: String,
    pub address: String,
    pub public_key: String,
    pub private_key_wif: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct OfflineExportPrivkeyResponse {
    pub coin: String,
    pub keys: Vec<AddressKeyPair>,
}

#[derive(Clone, Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum OfflineExportPrivkeyError {
    #[display(fmt = "Coin {} not found", coin)]
    CoinNotFound { coin: String },
    // TODO: Uncomment when implementing HD wallet support check
    // #[display(fmt = "Coin {} does not support HD wallets", coin)]
    // HDWalletNotSupported { coin: String },
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

impl HttpStatusCode for OfflineExportPrivkeyError {
    fn status_code(&self) -> common::StatusCode {
        match self {
            OfflineExportPrivkeyError::CoinNotFound { .. } => common::StatusCode::NOT_FOUND,
            // OfflineExportPrivkeyError::HDWalletNotSupported { .. } => common::StatusCode::BAD_REQUEST,
            OfflineExportPrivkeyError::InternalError(_) => common::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Export private key and related information for a coin address without contacting electrum servers
pub async fn offline_export_privkey(
    ctx: MmArc,
    req: OfflineExportPrivkeyRequest,
) -> MmResult<OfflineExportPrivkeyResponse, OfflineExportPrivkeyError> {
    // Get coin configuration
    let coin_conf = coin_conf(&ctx, &req.coin);
    if coin_conf.is_null() {
        return MmError::err(OfflineExportPrivkeyError::CoinNotFound { coin: req.coin });
    }

    // Get the crypto context
    let crypto_ctx = CryptoCtx::from_ctx(&ctx)
        .map_err(|e| OfflineExportPrivkeyError::InternalError(format!("CryptoCtx not available: {}", e)))?;

    // Check the key policy to determine if we're in HD or legacy mode
    let is_hd_mode = matches!(crypto_ctx.key_pair_policy(), KeyPairPolicy::GlobalHDAccount(_));

    // Parse coin protocol
    let protocol: CoinProtocol = json::from_value(coin_conf["protocol"].clone())
        .map_err(|e| OfflineExportPrivkeyError::InternalError(format!("Failed to parse coin protocol: {}", e)))?;

    // Determine the range of addresses to export
    let start_index = req.start_index;
    let end_index = req.end_index.unwrap_or(start_index);

    if end_index < start_index {
        return MmError::err(OfflineExportPrivkeyError::InternalError(
            "end_index must be greater than or equal to start_index".to_string(),
        ));
    }

    // Limit the range to prevent excessive memory usage
    const MAX_ADDRESSES: u32 = 1000;
    if end_index - start_index + 1 > MAX_ADDRESSES {
        return MmError::err(OfflineExportPrivkeyError::InternalError(format!(
            "Range too large. Maximum {} addresses allowed",
            MAX_ADDRESSES
        )));
    }

    // Get common configuration values
    let (wif_prefix, checksum_type) = match &protocol {
        CoinProtocol::UTXO | CoinProtocol::BCH { .. } | CoinProtocol::QTUM | CoinProtocol::QRC20 { .. } => {
            let wif_prefix = coin_conf["wif_prefix"].as_u64().unwrap_or_else(|| {
                // Default: p2pkh prefix + 128
                coin_conf["address_prefixes"]["p2pkh"].as_u64().unwrap_or(0) + 128
            }) as u8;

            let checksum_type = match coin_conf["checksum_type"].as_str() {
                Some("DGROESTL512") => ChecksumType::DGROESTL512,
                Some("KECCAK256") => ChecksumType::KECCAK256,
                _ => ChecksumType::DSHA256,
            };

            (Some(wif_prefix), Some(checksum_type))
        },
        _ => (None, None),
    };

    let mut keys = Vec::with_capacity((end_index - start_index + 1) as usize);

    if is_hd_mode {
        // HD mode
        let hd_account = match crypto_ctx.key_pair_policy() {
            KeyPairPolicy::GlobalHDAccount(hd_acc) => hd_acc,
            _ => unreachable!("Already checked is_hd_mode"),
        };

        // Get derivation path from config
        let path_to_coin = coin_conf["derivation_path"].as_str().or_mm_err(|| {
            OfflineExportPrivkeyError::InternalError("derivation_path not found in coin config".to_string())
        })?;

        // Generate keys for the requested range
        for address_index in start_index..=end_index {
            // Build full derivation path
            let chain = if req.is_change { 1 } else { 0 };
            let path_str = format!("{}/{}'/{}/{}", path_to_coin, req.account_id, chain, address_index);
            let derivation_path = DerivationPath::from_str(&path_str)
                .map_err(|e| OfflineExportPrivkeyError::InternalError(format!("Invalid derivation path: {:?}", e)))?;

            // Derive private key
            let private_key_bytes = hd_account.derive_secp256k1_secret(&derivation_path).map_err(|e| {
                OfflineExportPrivkeyError::InternalError(format!("Failed to derive private key: {}", e))
            })?;

            // Create key pair - convert H256 to [u8; 32]
            let mut secret_array = [0u8; 32];
            secret_array.copy_from_slice(private_key_bytes.as_ref());
            let key_pair = key_pair_from_secret(&secret_array)
                .map_err(|e| OfflineExportPrivkeyError::InternalError(format!("Failed to create key pair: {}", e)))?;

            // Get public key in hex format
            let public_key = hex::encode(&**key_pair.public());

            // Generate address using the existing function
            let address = address_by_coin_conf_and_pubkey_str(
                &ctx,
                &req.coin,
                &coin_conf,
                &public_key,
                utxo::UtxoAddressFormat::Standard,
            )
            .map_err(|e| OfflineExportPrivkeyError::InternalError(format!("Failed to generate address: {}", e)))?;

            // Generate private key in appropriate format
            let private_key_wif = match &protocol {
                CoinProtocol::UTXO | CoinProtocol::BCH { .. } | CoinProtocol::QTUM | CoinProtocol::QRC20 { .. } => {
                    // Create private key in WIF format
                    let private = Private {
                        prefix: wif_prefix.unwrap(),
                        secret: private_key_bytes,
                        compressed: true,
                        checksum_type: checksum_type.unwrap(),
                    };

                    private.to_string()
                },
                CoinProtocol::ETH { .. } | CoinProtocol::ERC20 { .. } | CoinProtocol::NFT { .. } => {
                    // For ETH-based coins, use hex format
                    format!("0x{}", hex::encode(private_key_bytes.as_ref() as &[u8]))
                },
                _ => {
                    return MmError::err(OfflineExportPrivkeyError::InternalError(format!(
                        "Unsupported protocol for private key export: {:?}",
                        protocol
                    )))
                },
            };

            keys.push(AddressKeyPair {
                account_id: req.account_id,
                is_change: req.is_change,
                address_index,
                derivation_path: path_str,
                address,
                public_key,
                private_key_wif,
            });
        }
    } else {
        // Legacy (Iguana) mode - only supports single address
        if start_index != 0 || end_index != 0 {
            return MmError::err(OfflineExportPrivkeyError::InternalError(
                "Legacy mode only supports exporting the single iguana address (index 0)".to_string(),
            ));
        }

        // Get the iguana key pair
        let key_pair = match crypto_ctx.key_pair_policy() {
            KeyPairPolicy::Iguana => {
                // Get the key pair from the iguana seed
                let seed = crypto_ctx.mm2_internal_privkey_secret();
                let seed_str = hex::encode(seed.as_slice());
                key_pair_from_seed(&seed_str).map_err(|e| {
                    OfflineExportPrivkeyError::InternalError(format!("Failed to create key pair from seed: {}", e))
                })?
            },
            _ => {
                return MmError::err(OfflineExportPrivkeyError::InternalError(
                    "Not in Iguana mode but trying to access legacy address".to_string(),
                ))
            },
        };

        // Get public key in hex format
        let public_key = hex::encode(&**key_pair.public());

        // Generate address using the existing function
        let address = address_by_coin_conf_and_pubkey_str(
            &ctx,
            &req.coin,
            &coin_conf,
            &public_key,
            utxo::UtxoAddressFormat::Standard,
        )
        .map_err(|e| OfflineExportPrivkeyError::InternalError(format!("Failed to generate address: {}", e)))?;

        // Generate private key in appropriate format
        let private_key_wif = match &protocol {
            CoinProtocol::UTXO | CoinProtocol::BCH { .. } | CoinProtocol::QTUM | CoinProtocol::QRC20 { .. } => {
                // Create private key in WIF format
                let private = Private {
                    prefix: wif_prefix.unwrap(),
                    secret: key_pair.private().secret,
                    compressed: key_pair.private().compressed,
                    checksum_type: checksum_type.unwrap(),
                };

                private.to_string()
            },
            CoinProtocol::ETH { .. } | CoinProtocol::ERC20 { .. } | CoinProtocol::NFT { .. } => {
                // For ETH-based coins, use hex format
                format!("0x{}", hex::encode(key_pair.private().secret.as_ref() as &[u8]))
            },
            _ => {
                return MmError::err(OfflineExportPrivkeyError::InternalError(format!(
                    "Unsupported protocol for private key export: {:?}",
                    protocol
                )))
            },
        };

        keys.push(AddressKeyPair {
            account_id: 0,
            is_change: false,
            address_index: 0,
            derivation_path: "legacy/iguana".to_string(),
            address,
            public_key,
            private_key_wif,
        });
    }

    Ok(OfflineExportPrivkeyResponse { coin: req.coin, keys })
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::block_on;
    use crypto::CryptoCtx;
    use mm2_test_helpers::for_tests::mm_ctx_with_custom_db_with_conf;

    const TEST_SEED: &str = "also shoot benefit prefer juice shell elder veteran woman mimic image kidney";

    #[test]
    fn test_offline_export_single_btc_address() {
        // Setup configuration with BTC coin
        let conf = json!({
            "coins": [{
                "coin": "BTC",
                "protocol": {
                    "type": "UTXO"
                },
                "derivation_path": "m/44'/0'",
                "address_prefixes": {
                    "p2pkh": 0,
                    "p2sh": 5
                },
                "wif_prefix": 128,
                "checksum_type": "DSHA256"
            }]
        });
        let ctx = mm_ctx_with_custom_db_with_conf(Some(conf));

        // Initialize crypto context with test seed
        let _ = CryptoCtx::init_with_global_hd_account(ctx.clone(), TEST_SEED).expect("Failed to init crypto context");

        let request = OfflineExportPrivkeyRequest {
            coin: "BTC".to_string(),
            start_index: 0,
            end_index: None,
            account_id: 0,
            is_change: false,
        };

        let response = block_on(offline_export_privkey(ctx, request)).expect("Failed to export private key");

        assert_eq!(response.coin, "BTC");
        assert_eq!(response.keys.len(), 1);

        let key = &response.keys[0];
        assert_eq!(key.address_index, 0);
        // The actual address will depend on the test seed
        assert!(!key.address.is_empty());
        assert!(!key.public_key.is_empty());
        assert!(
            key.private_key_wif.starts_with('5')
                || key.private_key_wif.starts_with('K')
                || key.private_key_wif.starts_with('L')
        );
    }

    #[test]
    fn test_offline_export_eth_address_range() {
        // Setup configuration with ETH coin
        let conf = json!({
            "coins": [{
                "coin": "ETH",
                "protocol": {
                    "type": "ETH"
                },
                "derivation_path": "m/44'/60'"
            }]
        });
        let ctx = mm_ctx_with_custom_db_with_conf(Some(conf));

        // Initialize crypto context with test seed
        let _ = CryptoCtx::init_with_global_hd_account(ctx.clone(), TEST_SEED).expect("Failed to init crypto context");

        let request = OfflineExportPrivkeyRequest {
            coin: "ETH".to_string(),
            start_index: 0,
            end_index: Some(4),
            account_id: 0,
            is_change: false,
        };

        let response = block_on(offline_export_privkey(ctx, request)).expect("Failed to export private keys");

        assert_eq!(response.coin, "ETH");
        assert_eq!(response.keys.len(), 5); // 0 to 4 inclusive

        for (i, key) in response.keys.iter().enumerate() {
            assert_eq!(key.address_index, i as u32);
            assert!(key.address.starts_with("0x"));
            assert_eq!(key.address.len(), 42); // 0x + 40 hex chars
            assert!(!key.public_key.is_empty());
            assert!(key.private_key_wif.starts_with("0x"));
            assert_eq!(key.private_key_wif.len(), 66); // 0x + 64 hex chars
        }
    }

    #[test]
    fn test_coin_not_found() {
        let ctx = mm_ctx_with_custom_db_with_conf(Some(json!({"coins": []})));

        // Initialize crypto context with test seed
        let _ = CryptoCtx::init_with_global_hd_account(ctx.clone(), TEST_SEED).expect("Failed to init crypto context");

        let request = OfflineExportPrivkeyRequest {
            coin: "NONEXISTENT".to_string(),
            start_index: 0,
            end_index: None,
            account_id: 0,
            is_change: false,
        };

        let result = block_on(offline_export_privkey(ctx, request));
        assert!(result.is_err());

        match result.unwrap_err().into_inner() {
            OfflineExportPrivkeyError::CoinNotFound { coin } => {
                assert_eq!(coin, "NONEXISTENT");
            },
            _ => panic!("Expected CoinNotFound error"),
        }
    }

    #[test]
    fn test_invalid_range() {
        // Setup configuration with BTC coin
        let conf = json!({
            "coins": [{
                "coin": "BTC",
                "protocol": {
                    "type": "UTXO"
                },
                "derivation_path": "m/44'/0'",
                "address_prefixes": {
                    "p2pkh": 0,
                    "p2sh": 5
                }
            }]
        });
        let ctx = mm_ctx_with_custom_db_with_conf(Some(conf));

        // Initialize crypto context with test seed
        let _ = CryptoCtx::init_with_global_hd_account(ctx.clone(), TEST_SEED).expect("Failed to init crypto context");

        // Test end_index < start_index
        let request = OfflineExportPrivkeyRequest {
            coin: "BTC".to_string(),
            start_index: 5,
            end_index: Some(3),
            account_id: 0,
            is_change: false,
        };

        let result = block_on(offline_export_privkey(ctx.clone(), request));
        assert!(result.is_err());

        // Test range too large
        let request = OfflineExportPrivkeyRequest {
            coin: "BTC".to_string(),
            start_index: 0,
            end_index: Some(1001), // > MAX_ADDRESSES
            account_id: 0,
            is_change: false,
        };

        let result = block_on(offline_export_privkey(ctx, request));
        assert!(result.is_err());
    }

    #[test]
    fn test_no_hd_wallet() {
        let ctx = mm_ctx_with_custom_db_with_conf(Some(json!({"coins": []})));

        // Don't initialize crypto context with HD account
        // This simulates a non-HD wallet scenario

        let request = OfflineExportPrivkeyRequest {
            coin: "BTC".to_string(),
            start_index: 0,
            end_index: None,
            account_id: 0,
            is_change: false,
        };

        let result = block_on(offline_export_privkey(ctx, request));
        assert!(result.is_err());

        match result.unwrap_err().into_inner() {
            OfflineExportPrivkeyError::InternalError(msg) => {
                assert!(msg.contains("CryptoCtx not available"));
            },
            _ => panic!("Expected InternalError for missing HD wallet"),
        }
    }

    #[test]
    fn test_custom_account_id() {
        // Setup configuration with BTC coin
        let conf = json!({
            "coins": [{
                "coin": "BTC",
                "protocol": {
                    "type": "UTXO"
                },
                "derivation_path": "m/44'/0'",
                "address_prefixes": {
                    "p2pkh": 0,
                    "p2sh": 5
                },
                "wif_prefix": 128,
                "checksum_type": "DSHA256"
            }]
        });
        let ctx = mm_ctx_with_custom_db_with_conf(Some(conf));

        // Initialize crypto context with test seed
        let _ = CryptoCtx::init_with_global_hd_account(ctx.clone(), TEST_SEED).expect("Failed to init crypto context");

        let request = OfflineExportPrivkeyRequest {
            coin: "BTC".to_string(),
            start_index: 0,
            end_index: None,
            account_id: 5, // Custom account ID
            is_change: false,
        };

        let response = block_on(offline_export_privkey(ctx, request)).expect("Failed to export private key");

        assert_eq!(response.keys.len(), 1);
        let key = &response.keys[0];
        assert_eq!(key.account_id, 5);
        assert_eq!(key.is_change, false);
        assert!(key.derivation_path.contains("/5'/0/0"));
    }

    #[test]
    fn test_change_addresses() {
        // Setup configuration with BTC coin
        let conf = json!({
            "coins": [{
                "coin": "BTC",
                "protocol": {
                    "type": "UTXO"
                },
                "derivation_path": "m/44'/0'",
                "address_prefixes": {
                    "p2pkh": 0,
                    "p2sh": 5
                }
            }]
        });
        let ctx = mm_ctx_with_custom_db_with_conf(Some(conf));

        // Initialize crypto context with test seed
        let _ = CryptoCtx::init_with_global_hd_account(ctx.clone(), TEST_SEED).expect("Failed to init crypto context");

        let request = OfflineExportPrivkeyRequest {
            coin: "BTC".to_string(),
            start_index: 0,
            end_index: Some(2),
            account_id: 0,
            is_change: true, // Request change addresses
        };

        let response = block_on(offline_export_privkey(ctx, request)).expect("Failed to export private keys");

        assert_eq!(response.keys.len(), 3);
        for key in &response.keys {
            assert_eq!(key.is_change, true);
            assert!(key.derivation_path.contains("/0'/1/")); // Change addresses use chain 1
        }
    }

    #[test]
    fn test_legacy_iguana_mode() {
        let ctx = mm_ctx_with_custom_db();

        // Initialize crypto context in iguana mode
        let _ = CryptoCtx::init_with_iguana_passphrase(ctx.clone(), "test passphrase")
            .expect("Failed to init crypto context");

        // Add BTC coin configuration
        let btc_conf = json!({
            "coin": "BTC",
            "protocol": {
                "type": "UTXO"
            },
            "address_prefixes": {
                "p2pkh": 0,
                "p2sh": 5
            },
            "wif_prefix": 128,
            "checksum_type": "DSHA256"
        });

        ctx.conf["coins"].as_array_mut().unwrap().push(btc_conf);

        let request = OfflineExportPrivkeyRequest {
            coin: "BTC".to_string(),
            start_index: 0,
            end_index: None,
            account_id: 0,
            is_change: false,
        };

        let response = block_on(offline_export_privkey(ctx.clone(), request)).expect("Failed to export private key");

        assert_eq!(response.keys.len(), 1);
        let key = &response.keys[0];
        assert_eq!(key.address_index, 0);
        assert_eq!(key.derivation_path, "legacy/iguana");
        assert!(!key.address.is_empty());
        assert!(
            key.private_key_wif.starts_with('5')
                || key.private_key_wif.starts_with('K')
                || key.private_key_wif.starts_with('L')
        );

        // Legacy mode should fail for non-zero indices
        let request = OfflineExportPrivkeyRequest {
            coin: "BTC".to_string(),
            start_index: 1,
            end_index: None,
            account_id: 0,
            is_change: false,
        };

        let result = block_on(offline_export_privkey(ctx, request));
        assert!(result.is_err());
        match result.unwrap_err().into_inner() {
            OfflineExportPrivkeyError::InternalError(msg) => {
                assert!(msg.contains("Legacy mode only supports"));
            },
            _ => panic!("Expected InternalError for invalid legacy index"),
        }
    }
}
