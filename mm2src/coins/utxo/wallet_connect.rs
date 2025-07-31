//! This module provides functionality to interact with WalletConnect for UTXO-based coins.
use std::collections::BTreeMap;
use std::convert::TryFrom;

use crate::UtxoTx;
use base64::engine::general_purpose::STANDARD as BASE64_ENGINE;
use base64::Engine;
use bitcoin::{
    consensus::Decodable, consensus::Encodable, psbt::Psbt, EcdsaSighashType, PackedLockTime, Sequence, Transaction,
};
use bitcrypto::sign_message_hash;
use chain::bytes::Bytes;
use chain::hash::H256;
use crypto::StandardHDPath;
use kdf_walletconnect::{
    chain::{WcChainId, WcRequestMethods},
    error::WalletConnectError,
    WalletConnectCtx, WcTopic,
};
use keys::Address;
use keys::{CompactSignature, Public};
use mm2_err_handle::prelude::{MmError, MmResult};
use script::{Builder, TransactionInputSigner};

/// Represents a UTXO address returned by GetAccountAddresses request in WalletConnect.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetAccountAddressesItem {
    address: String,
    public_key: Option<String>,
    path: Option<StandardHDPath>,
    #[allow(dead_code)]
    intention: Option<String>,
}

/// Get the enabled address (chosen by the user)
pub async fn get_walletconnect_address(
    wc: &WalletConnectCtx,
    session_topic: &WcTopic,
    chain_id: &WcChainId,
    derivation_path: &StandardHDPath,
) -> MmResult<(String, Option<String>), WalletConnectError> {
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
    let account = accounts.iter().find(|a| a.path.as_ref() == Some(derivation_path));

    match account {
        // If we found an account with the specific derivation path, we pick it.
        Some(account) => Ok((account.address.clone(), account.public_key.clone())),
        // If we didn't find the account with the specific derivation path, we perform some sane fallback.
        None => {
            let first_account = accounts.into_iter().next().ok_or_else(|| {
                WalletConnectError::NoAccountFound(
                    "WalletConnect returned no addresses for getAccountAddresses".to_string(),
                )
            })?;
            // If the response doesn't include derivation path information, just return the first address.
            if first_account.path.is_none() {
                common::log::warn!("WalletConnect didn't specify derivation paths for getAccountAddresses, picking the first address: {}", first_account.address);
                Ok((first_account.address, first_account.public_key))
            } else {
                // Otherwise, the response includes a derivation path, which means we didn't find the one that the user was interested in.
                MmError::err(WalletConnectError::NoAccountFound(format!(
                    "No address found for derivation path: {derivation_path}"
                )))
            }
        },
    }
}

/// The response from WalletConnect for `signMessage` request.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignMessageResponse {
    address: String,
    signature: String,
    #[allow(dead_code)]
    message_hash: Option<String>,
}

/// Get the public key associated with some address via WalletConnect signature.
pub async fn get_pubkey_via_wallatconnect_signature(
    wc: &WalletConnectCtx,
    session_topic: &WcTopic,
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

    let decoded_signature = match hex::decode(&signature_response.signature) {
        Ok(decoded) => decoded,
        Err(hex_decode_err) => BASE64_ENGINE
            .decode(&signature_response.signature)
            .map_err(|base64_decode_err| {
                WalletConnectError::InternalError(format!(
                    "Failed to decode signature={} from hex (error={:?}) and from base64 (error={:?})",
                    signature_response.signature, hex_decode_err, base64_decode_err
                ))
            })?,
    };
    let signature = CompactSignature::try_from(decoded_signature).map_err(|e| {
        WalletConnectError::InternalError(format!(
            "Failed to parse signature={} into compact signature: {:?}",
            signature_response.signature, e
        ))
    })?;
    let message_hash = sign_message_hash(sign_message_prefix, AUTH_MSG);
    let pubkey = Public::recover_compact(&H256::from(message_hash), &signature).map_err(|e| {
        WalletConnectError::InternalError(format!(
            "Failed to recover public key from walletconnect signature={signature:?}: {e:?}"
        ))
    })?;

    Ok(pubkey.to_string())
}

#[derive(Deserialize)]
struct SignedPsbt {
    psbt: String,
    txid: Option<String>,
}

async fn sign_psbt(
    wc: &WalletConnectCtx,
    session_topic: &WcTopic,
    chain_id: &WcChainId,
    psbt: String,
    inputs_to_sign: BTreeMap<u32, (String, Vec<u8>)>,
    broadcast: bool,
) -> MmResult<SignedPsbt, WalletConnectError> {
    wc.validate_update_active_chain_id(session_topic, chain_id).await?;
    let (account_str, _) = wc.get_account_and_properties_for_chain_id(session_topic, chain_id)?;
    let sign_inputs = inputs_to_sign
        .into_iter()
        .map(|(idx, (addr, sig_hashes))| {
            json!({
                "address": addr,
                "index": idx,
                "sighashTypes": sig_hashes,
            })
        })
        .collect::<Vec<_>>();
    let params = json!({
        "account": account_str,
        "psbt": psbt,
        "signInputs": sign_inputs,
        "broadcast": broadcast,
    });
    let signed_psbt: SignedPsbt = wc
        .send_session_request_and_wait(session_topic, chain_id, WcRequestMethods::UtxoSignPsbt, params)
        .await?;
    Ok(signed_psbt)
}

// fixme: remove the panics
async fn sign_p2sh_with_walletconnect(
    wc: &WalletConnectCtx,
    session_topic: &WcTopic,
    chain_id: &WcChainId,
    signing_address: &Address,
    tx_input_signer: &TransactionInputSigner,
    prev_tx: UtxoTx,
    redeem_script: Bytes,
    unlocking_script: Bytes,
) -> UtxoTx {
    let signing_address = signing_address.display_address().unwrap();
    let unsigned_tx = unsigned_tx_from_input_signer(tx_input_signer);
    assert!(
        unsigned_tx.input.len() == 1,
        "Expected exactly one input for P2SH signing"
    );
    let mut psbt = Psbt::from_unsigned_tx(unsigned_tx).unwrap();
    psbt.inputs[0].non_witness_utxo = Some(prev_tx.into());
    psbt.inputs[0].redeem_script = Some(redeem_script.take().into());
    psbt.inputs[0].sighash_type = Some(EcdsaSighashType::All.into());

    let mut serialized_psbt = Vec::new();
    psbt.consensus_encode(&mut serialized_psbt).unwrap();
    let serialized_psbt = BASE64_ENGINE.encode(serialized_psbt);

    let signed_psbt = sign_psbt(
        wc,
        session_topic,
        chain_id,
        serialized_psbt,
        BTreeMap::from([(0, (signing_address, vec![EcdsaSighashType::All as u8]))]),
        false,
    )
    .await
    .unwrap();
    let signed_psbt = BASE64_ENGINE
        .decode(signed_psbt.psbt)
        .expect("Failed to decode signed PSBT");

    let signed_psbt = Psbt::consensus_decode(&mut &signed_psbt[..]).unwrap();
    // The signed PSBT is enough, but we combine them nonetheless to make sure they are compatible and that
    // walletconnect didn't trick us with random shit.
    psbt.combine(signed_psbt).unwrap();

    let walletconnect_sig = psbt.inputs[0].partial_sigs.values().next().unwrap();
    let redeem_script = psbt.inputs[0].redeem_script.as_ref().unwrap();

    // The signature and the redeem script are inserted as data.
    let p2sh_signature = Builder::default().push_data(&walletconnect_sig.to_vec()).into_bytes();
    let redeem_script = Builder::default().push_data(&redeem_script.as_bytes()).into_bytes();

    let mut final_script_sig = Bytes::new();
    final_script_sig.extend_from_slice(&p2sh_signature);
    final_script_sig.extend_from_slice(&unlocking_script);
    final_script_sig.extend_from_slice(&redeem_script);

    let mut signed_transaction: UtxoTx = tx_input_signer.clone().into();
    signed_transaction.inputs[0].script_sig = final_script_sig;
    signed_transaction.inputs[0].script_witness = vec![];

    signed_transaction
}

// fixme: remove the panics
pub async fn sign_p2wpkh_with_walletconect(
    wc: &WalletConnectCtx,
    session_topic: &WcTopic,
    chain_id: &WcChainId,
    signing_address: &Address,
    tx_input_signer: &TransactionInputSigner,
) -> UtxoTx {
    let signing_address = signing_address.display_address().unwrap();
    let unsigned_tx = unsigned_tx_from_input_signer(tx_input_signer);

    let mut psbt = Psbt::from_unsigned_tx(unsigned_tx).unwrap();
    for (psbt_input, input) in psbt.inputs.iter_mut().zip(tx_input_signer.inputs.iter()) {
        psbt_input.sighash_type = Some(EcdsaSighashType::All.into());
        psbt_input.witness_utxo = Some(bitcoin::TxOut {
            value: input.amount,
            script_pubkey: input.prev_script.to_vec().into(),
        });
    }

    let mut serialized_psbt = Vec::new();
    psbt.consensus_encode(&mut serialized_psbt).unwrap();
    let serialized_psbt = BASE64_ENGINE.encode(serialized_psbt);

    let signed_psbt = sign_psbt(
        wc,
        session_topic,
        chain_id,
        serialized_psbt,
        BTreeMap::from(
            psbt.inputs
                .iter()
                .enumerate()
                .map(|(idx, _)| (idx as u32, (signing_address.clone(), vec![EcdsaSighashType::All as u8])))
                .collect::<BTreeMap<_, _>>(),
        ),
        false,
    )
    .await
    .unwrap();
    let signed_psbt = BASE64_ENGINE
        .decode(signed_psbt.psbt)
        .expect("Failed to decode signed PSBT");

    let signed_psbt = Psbt::consensus_decode(&mut &signed_psbt[..]).unwrap();
    // The signed PSBT is enough, but we combine them nonetheless to make sure they are compatible and that
    // walletconnect didn't trick us with random shit.
    psbt.combine(signed_psbt).unwrap();

    let mut signed_transaction: UtxoTx = tx_input_signer.clone().into();
    for (input, input_to_sign) in psbt.inputs.into_iter().zip(signed_transaction.inputs.iter_mut()) {
        // If the wallet already finalized the script, use it at face value.
        if let Some(final_script_witness) = input.final_script_witness {
            input_to_sign.script_witness = final_script_witness.to_vec().into_iter().map(Bytes::from).collect();
        } else {
            let (pubkey, walletconnect_sig) = input.partial_sigs.iter().next().unwrap();
            input_to_sign.script_sig = Bytes::from(Vec::new());
            input_to_sign.script_witness =
                vec![Bytes::from(walletconnect_sig.to_vec()), Bytes::from(pubkey.to_bytes())];
        }
    }

    signed_transaction
}

/// Converts a `TransactionInputSigner` to a `Transaction` without signatures.
fn unsigned_tx_from_input_signer(tx_input_signer: &TransactionInputSigner) -> Transaction {
    Transaction {
        version: tx_input_signer.version,
        lock_time: PackedLockTime(tx_input_signer.lock_time),
        input: tx_input_signer
            .inputs
            .iter()
            .map(|input| bitcoin::TxIn {
                previous_output: input.previous_output.into(),
                sequence: Sequence(input.sequence),
                ..Default::default()
            })
            .collect(),
        output: tx_input_signer
            .outputs
            .iter()
            .map(|output| output.clone().into())
            .collect(),
    }
}
