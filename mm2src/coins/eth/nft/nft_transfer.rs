use crate::TxFeeDetails;
use mm2_number::BigDecimal;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};

pub struct WithdrawErc721Request {
    coin: String,
    to: String,
    token_address: String,
    token_id: BigDecimal,
}

pub struct WithdrawErc1155Request {
    coin: String,
    to: String,
    token_address: String,
    token_id: BigDecimal,
    amount: BigDecimal,
    #[serde(default)]
    max: bool,
}

pub struct TransactionNFTDetails {
    /// Raw bytes of signed transaction, this should be sent as is to `send_raw_transaction_bytes` RPC to broadcast the transaction
    tx_hex: BytesJson,
    /// Transaction hash in hexadecimal format
    tx_hash: String,
    /// NFTs are sent from these addresses
    from: Vec<String>,
    /// NFTs are sent to these addresses
    to: Vec<String>,
    contract_type: String,
    token_address: String,
    token_id: BigDecimal,
    amount: BigDecimal,
    fee_details: Option<TxFeeDetails>,
    /// Block height
    block_height: u64,
    /// Transaction timestamp
    timestamp: i64,
    /// Internal MM2 id used for internal transaction identification, for some coins it might be equal to transaction hash
    internal_id: i64,
}
