use crate::{TxFeeDetails, WithdrawFee};
use mm2_number::BigDecimal;
use rpc::v1::types::Bytes as BytesJson;
use serde::Deserialize;
use std::str::FromStr;
// use serde_json::Value as Json;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct NftListReq {
    pub(crate) chains: Vec<Chain>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct NftMetadataReq {
    token_address: String,
    token_id: BigDecimal,
    chain: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub enum Chain {
    Eth,
    Bnb,
}

#[derive(Debug, Display, PartialEq)]
pub enum ParseContractTypeError {
    UnsupportedContractType,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum ContractType {
    Erc721,
    Erc1155,
}

impl FromStr for ContractType {
    type Err = ParseContractTypeError;

    #[inline]
    fn from_str(s: &str) -> Result<ContractType, ParseContractTypeError> {
        match s {
            "ERC721" => Ok(ContractType::Erc721),
            "ERC1155" => Ok(ContractType::Erc1155),
            _ => Err(ParseContractTypeError::UnsupportedContractType),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Nft {
    pub(crate) chain: Chain,
    pub(crate) token_address: Option<String>,
    pub(crate) token_id: Option<BigDecimal>,
    pub(crate) amount: Option<BigDecimal>,
    pub(crate) owner_of: Option<String>,
    pub(crate) token_hash: Option<String>,
    pub(crate) block_number_minted: Option<u64>,
    pub(crate) block_number: Option<u64>,
    pub(crate) contract_type: Option<ContractType>,
    pub(crate) name: Option<String>,
    pub(crate) symbol: Option<String>,
    pub(crate) token_uri: Option<String>,
    pub(crate) metadata: Option<String>,
    pub(crate) last_token_uri_sync: Option<String>,
    pub(crate) last_metadata_sync: Option<String>,
    pub(crate) minter_address: Option<String>,
}

/// This structure is for deserializing NFT json from Moralis to struct.
/// Its needed to convert fields properly, bcz Moralis returns json where all fields have string type.
#[derive(Debug, Deserialize, Serialize)]
pub struct NftWrapper {
    pub(crate) token_address: Option<String>,
    pub(crate) token_id: Option<Wrap<BigDecimal>>,
    pub(crate) amount: Option<Wrap<BigDecimal>>,
    pub(crate) owner_of: Option<String>,
    pub(crate) token_hash: Option<String>,
    pub(crate) block_number_minted: Option<Wrap<u64>>,
    pub(crate) block_number: Option<Wrap<u64>>,
    pub(crate) contract_type: Option<Wrap<ContractType>>,
    pub(crate) name: Option<String>,
    pub(crate) symbol: Option<String>,
    pub(crate) token_uri: Option<String>,
    pub(crate) metadata: Option<String>,
    pub(crate) last_token_uri_sync: Option<String>,
    pub(crate) last_metadata_sync: Option<String>,
    pub(crate) minter_address: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Wrap<T>(pub(crate) T);

impl<'de, T> Deserialize<'de> for Wrap<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Debug + std::fmt::Display,
{
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value: &str = Deserialize::deserialize(deserializer)?;
        let value: T = match value.parse() {
            Ok(v) => v,
            Err(e) => return Err(<D::Error as serde::de::Error>::custom(e)),
        };
        Ok(Wrap(value))
    }
}

impl<T> std::ops::Deref for Wrap<T> {
    type Target = T;
    fn deref(&self) -> &T { &self.0 }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NftList {
    pub(crate) count: u64,
    pub(crate) nfts: Vec<Nft>,
}

#[allow(dead_code)]
#[derive(Clone, Deserialize)]
pub struct WithdrawErc721Request {
    pub coin: String,
    to: String,
    token_address: String,
    token_id: BigDecimal,
    fee: Option<WithdrawFee>,
}

#[allow(dead_code)]
#[derive(Clone, Deserialize)]
pub struct WithdrawErc1155Request {
    pub coin: String,
    to: String,
    token_address: String,
    token_id: BigDecimal,
    amount: BigDecimal,
    #[serde(default)]
    max: bool,
    fee: Option<WithdrawFee>,
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
#[derive(Clone, Deserialize)]
pub struct NftTransfersReq {
    chains: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
enum NftTxType {
    Single,
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub struct NftTransferHistory {
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
#[derive(Debug, Serialize)]
pub struct NftsTransferHistoryByChain {
    chain: Chain,
    count: u64,
    transfer_history: Vec<NftTransferHistory>,
}
