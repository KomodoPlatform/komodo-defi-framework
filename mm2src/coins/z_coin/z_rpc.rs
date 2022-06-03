use crate::utxo::rpc_clients::{NativeClient, UtxoRpcClientEnum, UtxoRpcError, UtxoRpcFut};
use bigdecimal::BigDecimal;
use common::jsonrpc_client::{JsonRpcClient, JsonRpcRequest};
use common::mm_number::MmNumber;
use mm2_err_handle::prelude::*;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json, H264 as H264Json};
use serde_json::{self as json, Value as Json};

#[derive(Debug, Serialize)]
pub struct ZSendManyItem {
    pub amount: BigDecimal,
    #[serde(rename = "opreturn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_return: Option<BytesJson>,
    pub address: String,
}

#[derive(Debug, Deserialize)]
pub struct ZOperationTxid {
    pub txid: H256Json,
}

#[derive(Debug, Deserialize)]
pub struct ZOperationHex {
    pub hex: BytesJson,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status")]
#[serde(rename_all = "lowercase")]
pub enum ZOperationStatus<T> {
    Queued {
        id: String,
        creation_time: u64,
        method: String,
        params: Json,
    },
    Executing {
        id: String,
        creation_time: u64,
        method: String,
        params: Json,
    },
    Success {
        id: String,
        creation_time: u64,
        result: T,
        execution_secs: f64,
        method: String,
        params: Json,
    },
    Failed {
        id: String,
        creation_time: u64,
        method: String,
        params: Json,
        error: Json,
    },
}

#[derive(Debug, Serialize)]
pub struct ZSendManyHtlcParams {
    pub pubkey: H264Json,
    pub refund_pubkey: H264Json,
    pub secret_hash: BytesJson,
    pub input_txid: H256Json,
    pub input_index: usize,
    pub input_amount: BigDecimal,
    pub locktime: u32,
}

#[derive(Debug, Deserialize)]
pub struct ZUnspent {
    pub txid: H256Json,
    #[serde(rename = "outindex")]
    pub out_index: u32,
    pub confirmations: u32,
    #[serde(rename = "raw_confirmations")]
    pub raw_confirmations: Option<u32>,
    pub spendable: bool,
    pub address: String,
    pub amount: MmNumber,
    pub memo: BytesJson,
    pub change: bool,
}

pub trait ZRpcOps {
    fn z_get_balance(&self, address: &str, min_conf: u32) -> UtxoRpcFut<MmNumber>;

    fn z_get_send_many_status(&self, op_ids: &[&str]) -> UtxoRpcFut<Vec<ZOperationStatus<ZOperationTxid>>>;

    fn z_list_unspent(
        &self,
        min_conf: u32,
        max_conf: u32,
        watch_only: bool,
        addresses: &[&str],
    ) -> UtxoRpcFut<Vec<ZUnspent>>;

    fn z_send_many(&self, from_address: &str, send_to: Vec<ZSendManyItem>) -> UtxoRpcFut<String>;

    fn z_import_key(&self, key: &str) -> UtxoRpcFut<()>;
}

impl ZRpcOps for NativeClient {
    fn z_get_balance(&self, address: &str, min_conf: u32) -> UtxoRpcFut<MmNumber> {
        let fut = rpc_func!(self, "z_getbalance", address, min_conf);
        Box::new(fut.map_to_mm_fut(UtxoRpcError::from))
    }

    fn z_get_send_many_status(&self, op_ids: &[&str]) -> UtxoRpcFut<Vec<ZOperationStatus<ZOperationTxid>>> {
        let fut = rpc_func!(self, "z_getoperationstatus", op_ids);
        Box::new(fut.map_to_mm_fut(UtxoRpcError::from))
    }

    fn z_list_unspent(
        &self,
        min_conf: u32,
        max_conf: u32,
        watch_only: bool,
        addresses: &[&str],
    ) -> UtxoRpcFut<Vec<ZUnspent>> {
        let fut = rpc_func!(self, "z_listunspent", min_conf, max_conf, watch_only, addresses);
        Box::new(fut.map_to_mm_fut(UtxoRpcError::from))
    }

    fn z_send_many(&self, from_address: &str, send_to: Vec<ZSendManyItem>) -> UtxoRpcFut<String> {
        let fut = rpc_func!(self, "z_sendmany", from_address, send_to);
        Box::new(fut.map_to_mm_fut(UtxoRpcError::from))
    }

    fn z_import_key(&self, key: &str) -> UtxoRpcFut<()> {
        let fut = rpc_func!(self, "z_importkey", key, "no");
        Box::new(fut.map_to_mm_fut(UtxoRpcError::from))
    }
}

impl AsRef<dyn ZRpcOps + Send + Sync> for UtxoRpcClientEnum {
    fn as_ref(&self) -> &(dyn ZRpcOps + Send + Sync + 'static) {
        match self {
            UtxoRpcClientEnum::Native(native) => native,
            UtxoRpcClientEnum::Electrum(_) => panic!("Electrum client does not support ZRpcOps"),
        }
    }
}

mod z_coin_grpc {
    tonic::include_proto!("cash.z.wallet.sdk.rpc");
}
