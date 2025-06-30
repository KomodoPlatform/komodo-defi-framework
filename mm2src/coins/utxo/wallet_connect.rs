//! This module provides functionality to interact with WalletConnect for UTXO-based coins.

use crypto::StandardHDPath;
use kdf_walletconnect::{chain::{WcChainId, WcRequestMethods},
                        error::WalletConnectError,
                        WalletConnectCtx};
use mm2_err_handle::prelude::MmResult;

/// Represents a UTXO address returned by GetAccountAddresses request in WalletConnect.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetAccountAddressesItem {
    address: String,
    public_key: String,
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
