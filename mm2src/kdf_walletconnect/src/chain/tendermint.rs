use crate::{error::WalletConnectCtxError, WalletConnectCtx};

use base64::{engine::general_purpose, Engine};
use chrono::Utc;
use common::log::info;
use futures::StreamExt;
use mm2_err_handle::prelude::{MmError, MmResult};
use relay_rpc::rpc::params::{session_request::{Request as SessionRequest, SessionRequestRequest},
                             RequestParams, ResponseParamsSuccess};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::WcRequestMethods;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum CosmosAccountAlgo {
    #[serde(rename = "secp256k1")]
    Secp256k1,
    #[serde(rename = "tendermint/PubKeySecp256k1")]
    TendermintSecp256k1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CosmosAccount {
    pub address: String,
    #[serde(deserialize_with = "deserialize_vec_field")]
    pub pubkey: Vec<u8>,
    pub algo: CosmosAccountAlgo,
}

fn deserialize_vec_field<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;

    match value {
        Value::Object(map) => {
            let mut vec = Vec::new();
            for i in 0..map.len() {
                if let Some(Value::Number(num)) = map.get(&i.to_string()) {
                    if let Some(byte) = num.as_u64() {
                        vec.push(byte as u8);
                    } else {
                        return Err(serde::de::Error::custom("Invalid byte value"));
                    }
                } else {
                    return Err(serde::de::Error::custom("Invalid pubkey format"));
                }
            }
            Ok(vec)
        },
        Value::Array(arr) => arr
            .into_iter()
            .map(|v| {
                v.as_u64()
                    .ok_or_else(|| serde::de::Error::custom("Invalid byte value"))
                    .map(|n| n as u8)
            })
            .collect(),
        Value::String(data) => {
            let data = decode_data(&data).map_err(|err| serde::de::Error::custom(err.to_string()))?;
            Ok(data)
        },
        _ => Err(serde::de::Error::custom("Pubkey must be an string, object or array")),
    }
}

fn decode_data(encoded: &str) -> Result<Vec<u8>, &'static str> {
    // Check if the string is base64 or hex
    if encoded.contains('=') || encoded.contains('/') || encoded.contains('+') {
        // Try to decode as base64
        general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| "Invalid base64 encoding")
    } else if encoded.chars().all(|c| c.is_ascii_hexdigit()) && encoded.len() % 2 == 0 {
        // Try to decode as hex
        hex::decode(encoded).map_err(|_| "Invalid hex encoding")
    } else {
        Err("Unknown encoding format")
    }
}

pub async fn cosmos_get_accounts_impl(
    ctx: &WalletConnectCtx,
    chain_id: &str,
) -> MmResult<Vec<CosmosAccount>, WalletConnectCtxError> {
    let account = ctx.get_account_for_chain_id(chain_id).await?;

    let topic = {
        let session = ctx.session.get_session_active().await;
        if session.is_none() {
            return MmError::err(WalletConnectCtxError::NotInitialized);
        };

        session.unwrap().topic
    };

    let request = SessionRequest {
        method: WcRequestMethods::CosmosGetAccounts.as_ref().to_owned(),
        expiry: Some(Utc::now().timestamp() as u64 + 300),
        params: serde_json::to_value(&account).unwrap(),
    };
    let request = SessionRequestRequest {
        request,
        chain_id: format!("cosmos:{chain_id}"),
    };

    {
        let session_request = RequestParams::SessionRequest(request);
        ctx.publish_request(&topic, session_request).await?;
    };

    let mut session_handler = ctx.session_request_handler.lock().await;
    if let Some((message_id, data)) = session_handler.next().await {
        let result = serde_json::from_value::<Vec<CosmosAccount>>(data)?;
        let response = ResponseParamsSuccess::SessionEvent(true);
        ctx.publish_response_ok(&topic, response, &message_id).await?;

        return Ok(result);
    };

    MmError::err(WalletConnectCtxError::NoAccountFound(chain_id.to_owned()))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CosmosTxSignedData {
    pub signature: CosmosTxSignature,
    pub signed: CosmosSignData,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CosmosTxSignature {
    pub pub_key: CosmosTxPublicKey,
    pub signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CosmosTxPublicKey {
    #[serde(rename = "type")]
    pub key_type: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CosmosSignData {
    pub chain_id: String,
    pub account_number: String,
    #[serde(deserialize_with = "deserialize_vec_field")]
    pub auth_info_bytes: Vec<u8>,
    #[serde(deserialize_with = "deserialize_vec_field")]
    pub body_bytes: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountNumber {
    low: u64,
    high: u64,
    unsigned: bool,
}

pub async fn cosmos_sign_direct_impl(
    ctx: &WalletConnectCtx,
    sign_doc: Value,
    chain_id: &str,
) -> MmResult<CosmosTxSignedData, WalletConnectCtxError> {
    let topic = {
        let session = ctx.session.get_session_active().await;
        // return not NotInitialized error if no session is found.
        if session.is_none() {
            return MmError::err(WalletConnectCtxError::NotInitialized);
        };

        session.unwrap().topic
    };

    let request = SessionRequest {
        method: WcRequestMethods::CosmosSignDirect.as_ref().to_owned(),
        expiry: Some(Utc::now().timestamp() as u64 + 300),
        params: sign_doc,
    };
    let request = SessionRequestRequest {
        request,
        chain_id: format!("cosmos:{chain_id}"),
    };
    {
        let session_request = RequestParams::SessionRequest(request);
        ctx.publish_request(&topic, session_request).await?;
    }

    let mut session_handler = ctx.session_request_handler.lock().await;
    if let Some((message_id, data)) = session_handler.next().await {
        let result = serde_json::from_value::<CosmosTxSignedData>(data)?;
        let response = ResponseParamsSuccess::SessionEvent(true);
        ctx.publish_response_ok(&topic, response, &message_id).await?;

        return Ok(result);
    }

    MmError::err(WalletConnectCtxError::InternalError(
        "No response from wallet".to_string(),
    ))
}
