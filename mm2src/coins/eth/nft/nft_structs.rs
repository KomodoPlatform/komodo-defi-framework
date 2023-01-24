use crate::TxFeeDetails;
use mm2_number::BigDecimal;
use rpc::v1::types::Bytes as BytesJson;

#[allow(dead_code)]
#[derive(Debug)]
enum Chain {
    Eth,
    Bnb,
}

#[allow(dead_code)]
#[derive(Debug)]
enum ContractType {
    Erc721,
    Erc1155,
}

#[allow(dead_code)]
#[derive(Debug)]
struct Nft {
    token_address: String,
    token_id: BigDecimal,
    amount: BigDecimal,
    owner_of: String,
    token_hash: String,
    block_number_minted: u64,
    block_number: u64,
    contract_type: ContractType,
    name: Option<String>,
    symbol: Option<String>,
    token_uri: Option<String>,
    metadata: Option<String>,
    last_token_uri_sync: Option<String>,
    last_metadata_sync: Option<String>,
    minter_address: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug)]
struct Nfts {
    chain: Chain,
    count: i64,
    nfts: Vec<Nft>,
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct WithdrawErc721Request {
    coin: String,
    to: String,
    token_address: String,
    token_id: BigDecimal,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct WithdrawErc1155Request {
    coin: String,
    to: String,
    token_address: String,
    token_id: BigDecimal,
    amount: BigDecimal,
    #[serde(default)]
    max: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TransactionNftDetails {
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
    timestamp: u64,
    /// Internal MM2 id used for internal transaction identification, for some coins it might be equal to transaction hash
    internal_id: i64,
}

#[allow(dead_code)]
#[derive(Debug)]
enum NftTxType {
    Single,
}

#[allow(dead_code)]
#[derive(Debug)]
struct NftTransferHistory {
    block_number: u64,
    block_timestamp: u64,
    block_hash: String,
    /// Transaction hash in hexadecimal format
    tx_hash: String,
    tx_index: u64,
    log_index: u64,
    value: u64,
    contract_type: ContractType,
    tx_type: NftTxType,
    token_address: String,
    token_id: u64,
    from: String,
    to: String,
    amount: BigDecimal,
    verified: u64,
    operator: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug)]
struct NftTransferHistoryByChain {
    chain: Chain,
    count: i64,
    transfer_history: Vec<NftTransferHistory>,
}
