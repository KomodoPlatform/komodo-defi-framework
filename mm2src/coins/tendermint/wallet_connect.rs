use std::str::FromStr;

use base64::engine::general_purpose;
use base64::Engine;
use chrono::Utc;
use futures::StreamExt;
use kdf_walletconnect::error::WalletConnectError;
use kdf_walletconnect::{chain::WcRequestMethods, WalletConnectCtx};
use mm2_err_handle::prelude::*;
use relay_rpc::rpc::params::session_request::Request as SessionRequest;
use relay_rpc::rpc::params::session_request::SessionRequestRequest;
use relay_rpc::rpc::params::{RequestParams, ResponseParamsSuccess};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CosmosTxSignedData {
    pub signature: CosmosTxSignature,
    pub signed: CosmosSignData,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CosmosTxSignature {
    pub pub_key: CosmosTxPublicKey,
    pub signature: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CosmosTxPublicKey {
    #[serde(rename = "type")]
    pub key_type: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CosmosSignData {
    pub chain_id: String,
    pub account_number: String,
    #[serde(deserialize_with = "deserialize_vec_field")]
    pub auth_info_bytes: Vec<u8>,
    #[serde(deserialize_with = "deserialize_vec_field")]
    pub body_bytes: Vec<u8>,
}

pub async fn cosmos_request_wc_signed_tx(
    ctx: &WalletConnectCtx,
    sign_doc: Value,
    chain_id: &str,
    is_ledger_conn: bool,
) -> MmResult<CosmosTxSignedData, WalletConnectError> {
    let topic = ctx
        .session
        .get_session_active()
        .await
        .map(|session| session.topic.clone())
        .ok_or(WalletConnectError::NotInitialized)?;

    let method = if is_ledger_conn {
        WcRequestMethods::CosmosSignAmino
    } else {
        WcRequestMethods::CosmosSignDirect
    };
    let request = SessionRequestRequest {
        request: SessionRequest {
            method: method.as_ref().to_owned(),
            expiry: Some(Utc::now().timestamp() as u64 + 300),
            params: sign_doc,
        },
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

    MmError::err(WalletConnectError::InternalError("No response from wallet".to_string()))
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum CosmosAccountAlgo {
    #[serde(rename = "secp256k1")]
    Secp256k1,
    #[serde(rename = "tendermint/PubKeySecp256k1")]
    TendermintSecp256k1,
}

impl FromStr for CosmosAccountAlgo {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "secp256k1" => Ok(Self::Secp256k1),
            "tendermint/PubKeySecp256k1" => Ok(Self::TendermintSecp256k1),
            _ => Err(format!("Unknown pubkey type: {s}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CosmosAccount {
    pub address: String,
    #[serde(deserialize_with = "deserialize_vec_field")]
    pub pubkey: Vec<u8>,
    pub algo: CosmosAccountAlgo,
    #[serde(default)]
    pub is_ledger: Option<bool>,
}

pub async fn cosmos_get_accounts_impl(
    ctx: &WalletConnectCtx,
    chain_id: &str,
    account_index: Option<usize>,
) -> MmResult<CosmosAccount, WalletConnectError> {
    let account = ctx.get_account_for_chain_id(chain_id).await?;

    let session = ctx
        .session
        .get_session_active()
        .await
        .ok_or(WalletConnectError::NotInitialized)?;
    // Check if existing session has session_properties and return wallet account;
    if let Some(props) = &session.session_properties {
        if let Some(keys) = &props.keys {
            if let Some(key) = keys.iter().next() {
                let pubkey = decode_data(&key.pub_key).map_to_mm(|err| {
                    WalletConnectError::PayloadError(format!("error decoding pubkey payload: {err:?}"))
                })?;
                let address = decode_data(&key.address).map_to_mm(|err| {
                    WalletConnectError::PayloadError(format!("error decoding address payload: {err:?}"))
                })?;
                let address = hex::encode(address);
                let algo = CosmosAccountAlgo::from_str(&key.algo).map_to_mm(|err| {
                    WalletConnectError::PayloadError(format!("error decoding algo payload: {err:?}"))
                })?;

                return Ok(CosmosAccount {
                    address,
                    pubkey,
                    algo,
                    is_ledger: Some(key.is_nano_ledger),
                });
            }
        }
    }

    let topic = session.topic.clone();
    let request = SessionRequestRequest {
        request: SessionRequest {
            method: WcRequestMethods::CosmosGetAccounts.as_ref().to_owned(),
            expiry: Some(Utc::now().timestamp() as u64 + 300),
            params: serde_json::to_value(&account).unwrap(),
        },
        chain_id: format!("cosmos:{chain_id}"),
    };

    {
        let session_request = RequestParams::SessionRequest(request);
        ctx.publish_request(&topic, session_request).await?;
    };

    let mut session_handler = ctx.session_request_handler.lock().await;
    if let Some((message_id, data)) = session_handler.next().await {
        let accounts = serde_json::from_value::<Vec<CosmosAccount>>(data)?;
        let response = ResponseParamsSuccess::SessionEvent(true);
        ctx.publish_response_ok(&topic, response, &message_id).await?;

        if accounts.is_empty() {
            return MmError::err(WalletConnectError::EmptyAccount(chain_id.to_string()));
        };

        let account_index = account_index.unwrap_or(0);
        if accounts.len() < account_index + 1 {
            return MmError::err(WalletConnectError::NoAccountFoundForIndex(account_index));
        };

        return Ok(accounts[account_index].clone());
    };

    MmError::err(WalletConnectError::NoAccountFound(chain_id.to_owned()))
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
                    return Err(serde::de::Error::custom("Invalid format"));
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
    if encoded.chars().all(|c| c.is_ascii_hexdigit()) && encoded.len() % 2 == 0 {
        hex::decode(encoded).map_err(|_| "Invalid hex encoding")
    } else if encoded.contains('=') || encoded.contains('/') || encoded.contains('+') || encoded.len() % 4 == 0 {
        general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| "Invalid base64 encoding")
    } else {
        Err("Unknown encoding format")
    }
}

#[cfg(test)]
mod test_cosmos_walletconnect {
    use serde_json::json;

    use super::{decode_data, CosmosSignData, CosmosTxPublicKey, CosmosTxSignature, CosmosTxSignedData};

    #[test]
    fn test_decode_base64() {
        let base64_data = "SGVsbG8gd29ybGQ="; // "Hello world" in base64
        let expected = b"Hello world".to_vec();
        let result = decode_data(base64_data);
        assert_eq!(result.unwrap(), expected, "Base64 decoding failed");
    }

    #[test]
    fn test_decode_hex() {
        let hex_data = "48656c6c6f20776f726c64"; // "Hello world" in hex
        let expected = b"Hello world".to_vec();
        let result = decode_data(hex_data);
        assert_eq!(result.unwrap(), expected, "Hex decoding failed");
    }

    #[test]
    fn test_deserialize_sign_message_response() {
        let json = json!({
        "signature": {
          "signature": "eGrmDGKTmycxJO56yTQORDzTFjBEBgyBmHc8ey6FbHh9WytzgsJilYBywz5uludhyKePZdRwznamg841fXw50Q==",
          "pub_key": {
            "type": "tendermint/PubKeySecp256k1",
            "value": "AjqZ1rq/EsPAb4SA6l0qjpVMHzqXotYXz23D5kOceYYu"
          }
        },
        "signed": {
          "chainId": "cosmoshub-4",
          "authInfoBytes": "0a500a460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a21023a99d6babf12c3c06f8480ea5d2a8e954c1f3a97a2d617cf6dc3e6439c79862e12040a020801180212140a0e0a057561746f6d1205313837353010c8d007",
          "bodyBytes": "0a8e010a1c2f636f736d6f732e62616e6b2e763162657461312e4d736753656e64126e0a2d636f736d6f7331376c386432737973646e3667683636786d366664666b6575333634703836326a68396c6e6667122d636f736d6f7331376c386432737973646e3667683636786d366664666b6575333634703836326a68396c6e66671a0e0a057561746f6d12053430303030189780e00a",
          "accountNumber": "2934714"
        }
              });
        let expected_tx = CosmosTxSignedData {
            signature: CosmosTxSignature {
                pub_key: CosmosTxPublicKey {
                    key_type: "tendermint/PubKeySecp256k1".to_owned(),
                    value: "AjqZ1rq/EsPAb4SA6l0qjpVMHzqXotYXz23D5kOceYYu".to_owned(),
                },
                signature: "eGrmDGKTmycxJO56yTQORDzTFjBEBgyBmHc8ey6FbHh9WytzgsJilYBywz5uludhyKePZdRwznamg841fXw50Q=="
                    .to_owned(),
            },
            signed: CosmosSignData {
                chain_id: "cosmoshub-4".to_owned(),
                account_number: "2934714".to_owned(),
                auth_info_bytes: vec![
                    10, 80, 10, 70, 10, 31, 47, 99, 111, 115, 109, 111, 115, 46, 99, 114, 121, 112, 116, 111, 46, 115,
                    101, 99, 112, 50, 53, 54, 107, 49, 46, 80, 117, 98, 75, 101, 121, 18, 35, 10, 33, 2, 58, 153, 214,
                    186, 191, 18, 195, 192, 111, 132, 128, 234, 93, 42, 142, 149, 76, 31, 58, 151, 162, 214, 23, 207,
                    109, 195, 230, 67, 156, 121, 134, 46, 18, 4, 10, 2, 8, 1, 24, 2, 18, 20, 10, 14, 10, 5, 117, 97,
                    116, 111, 109, 18, 5, 49, 56, 55, 53, 48, 16, 200, 208, 7,
                ],
                body_bytes: vec![
                    10, 142, 1, 10, 28, 47, 99, 111, 115, 109, 111, 115, 46, 98, 97, 110, 107, 46, 118, 49, 98, 101,
                    116, 97, 49, 46, 77, 115, 103, 83, 101, 110, 100, 18, 110, 10, 45, 99, 111, 115, 109, 111, 115, 49,
                    55, 108, 56, 100, 50, 115, 121, 115, 100, 110, 54, 103, 104, 54, 54, 120, 109, 54, 102, 100, 102,
                    107, 101, 117, 51, 54, 52, 112, 56, 54, 50, 106, 104, 57, 108, 110, 102, 103, 18, 45, 99, 111, 115,
                    109, 111, 115, 49, 55, 108, 56, 100, 50, 115, 121, 115, 100, 110, 54, 103, 104, 54, 54, 120, 109,
                    54, 102, 100, 102, 107, 101, 117, 51, 54, 52, 112, 56, 54, 50, 106, 104, 57, 108, 110, 102, 103,
                    26, 14, 10, 5, 117, 97, 116, 111, 109, 18, 5, 52, 48, 48, 48, 48, 24, 151, 128, 224, 10,
                ],
            },
        };

        let actual_tx = serde_json::from_value::<CosmosTxSignedData>(json).unwrap();
        assert_eq!(expected_tx, actual_tx);
    }
}
