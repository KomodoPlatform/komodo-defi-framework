use rpc::v1::types::{Bytes as BytesJson, H160 as H160Json};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub struct GetPublicKeyResponse {
    pub public_key: String,
}

#[derive(Deserialize, Serialize)]
pub struct GetPublicKeyHashResponse {
    pub public_key_hash: H160Json,
}

#[derive(Deserialize, Serialize)]
pub struct GetRawTransactionRequest {
    pub coin: String,
    pub tx_hash: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GetRawTransactionResponse {
    /// Raw bytes of signed transaction in hexadecimal string, this should be return hexadecimal encoded signed transaction for get_raw_transaction
    pub tx_hex: BytesJson,
}
