//! This module provides functionality to interact with WalletConnect for UTXO-based coins.
use std::convert::TryFrom;

use crate::utxo::utxo_common;
use chain::hash::H256;
use crypto::StandardHDPath;
use kdf_walletconnect::{chain::{WcChainId, WcRequestMethods},
                        error::WalletConnectError,
                        WalletConnectCtx};
use keys::{CompactSignature, Public};
use mm2_err_handle::prelude::{MmError, MmResult};

use base64::engine::general_purpose::STANDARD as BASE64_ENGINE;
use base64::Engine;

/// Represents a UTXO address returned by GetAccountAddresses request in WalletConnect.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetAccountAddressesItem {
    address: String,
    // FIXME: this is optional
    public_key: String,
    // FIXME: this is also optional
    path: StandardHDPath,
}

/// Get the enabled address (chosen by the user)
pub async fn get_walletconnect_address(
    wc: &WalletConnectCtx,
    session_topic: &str,
    chain_id: &WcChainId,
    derivation_path: &StandardHDPath,
) -> MmResult<(String, String), WalletConnectError> {
    wc.validate_update_active_chain_id(session_topic, chain_id).await?;
    let (account_str, _) = wc.get_account_and_properties_for_chain_id(session_topic, chain_id)?;
    let params = json!({
        "account": account_str,
    });
    let accounts: Vec<GetAccountAddressesItem> = wc
        .send_session_request_and_wait(
            session_topic,
            chain_id,
            WcRequestMethods::UtxoGetAccountAddresses,
            params,
        )
        .await?;
    // Find the address that the user is interested in (the enabled address).
    let account = accounts
        .into_iter()
        .find(|a| a.path == *derivation_path)
        .ok_or_else(|| {
            WalletConnectError::NoAccountFound(format!("No address found for derivaiton path: {}", derivation_path))
        })?;
    Ok((account.address, account.public_key))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignMessageResponse {
    address: String,
    signature: String,
    #[allow(dead_code)]
    message_hash: Option<String>,
}

pub async fn get_pubkey_via_wallatconnect_signature(
    wc: &WalletConnectCtx,
    session_topic: &str,
    chain_id: &WcChainId,
    address: &str,
    sign_message_prefix: &str,
) -> MmResult<String, WalletConnectError> {
    const AUTH_MSG: &str = "Authenticate with KDF";

    wc.validate_update_active_chain_id(session_topic, chain_id).await?;
    let (account_str, _) = wc.get_account_and_properties_for_chain_id(session_topic, chain_id)?;
    let params = json!({
        "account": account_str,
        "address": address,
        "message": AUTH_MSG,
        "protocol": "ecdsa",
    });
    let signature_response: SignMessageResponse = wc
        .send_session_request_and_wait(session_topic, chain_id, WcRequestMethods::UtxoPersonalSign, params)
        .await?;

    // The wallet is required to send back the same address in the response.
    // We validate it here even though there shouldn't be a mismatch (otherwise the wallet is broken).
    if signature_response.address != address {
        return MmError::err(WalletConnectError::InternalError(format!(
            "Address mismatch: requested signature from {}, got it from {}",
            address, signature_response.address
        )));
    }

    let decoded_signature = BASE64_ENGINE.decode(&signature_response.signature).map_err(|e| {
        WalletConnectError::InternalError(format!(
            "Failed to decode signature={}: {:?}",
            signature_response.signature, e
        ))
    })?;
    let signature = CompactSignature::try_from(decoded_signature).map_err(|e| {
        WalletConnectError::InternalError(format!(
            "Failed to parse signature={} into compact signature: {:?}",
            signature_response.signature, e
        ))
    })?;
    let message_hash = utxo_common::sign_message_hash(sign_message_prefix, AUTH_MSG);
    let pubkey = Public::recover_compact(&H256::from(message_hash), &signature).map_err(|e| {
        WalletConnectError::InternalError(format!(
            "Failed to recover public key from walletconnect signature={:?}: {:?}",
            signature, e
        ))
    })?;

    Ok(pubkey.to_string())
}
