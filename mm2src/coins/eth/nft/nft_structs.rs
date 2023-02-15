use crate::{TxFeeDetails, WithdrawFee};
use mm2_number::BigDecimal;
use rpc::v1::types::Bytes as BytesJson;
use serde::Deserialize;
use std::str::FromStr;

#[derive(Debug, Deserialize)]
pub struct NftListReq {
    pub(crate) chains: Vec<Chain>,
}

#[derive(Debug, Deserialize)]
pub struct NftMetadataReq {
    pub(crate) token_address: String,
    pub(crate) token_id: BigDecimal,
    pub(crate) chain: Chain,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Chain {
    Bnb,
    Eth,
}

#[derive(Debug, Display)]
pub enum ParseContractTypeError {
    UnsupportedContractType,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ContractType {
    Erc1155,
    Erc721,
}

impl FromStr for ContractType {
    type Err = ParseContractTypeError;

    #[inline]
    fn from_str(s: &str) -> Result<ContractType, ParseContractTypeError> {
        match s {
            "ERC1155" => Ok(ContractType::Erc1155),
            "ERC721" => Ok(ContractType::Erc721),
            _ => Err(ParseContractTypeError::UnsupportedContractType),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Nft {
    pub(crate) chain: Chain,
    pub(crate) token_address: String,
    pub(crate) token_id: BigDecimal,
    pub(crate) amount: BigDecimal,
    pub(crate) owner_of: String,
    pub(crate) token_hash: String,
    pub(crate) block_number_minted: u64,
    pub(crate) block_number: u64,
    pub(crate) contract_type: Option<ContractType>,
    pub(crate) name: Option<String>,
    pub(crate) symbol: Option<String>,
    pub(crate) token_uri: Option<String>,
    pub(crate) metadata: Option<String>,
    pub(crate) last_token_uri_sync: Option<String>,
    pub(crate) last_metadata_sync: Option<String>,
    pub(crate) minter_address: Option<String>,
}

/// This structure is for deserializing NFT json to struct.
/// Its needed to convert fields properly, because all fields in json have string type.
#[derive(Debug, Deserialize)]
pub struct NftWrapper {
    pub(crate) token_address: String,
    pub(crate) token_id: SerdeStringWrap<BigDecimal>,
    pub(crate) amount: SerdeStringWrap<BigDecimal>,
    pub(crate) owner_of: String,
    pub(crate) token_hash: String,
    pub(crate) block_number_minted: SerdeStringWrap<u64>,
    pub(crate) block_number: SerdeStringWrap<u64>,
    pub(crate) contract_type: Option<SerdeStringWrap<ContractType>>,
    pub(crate) name: Option<String>,
    pub(crate) symbol: Option<String>,
    pub(crate) token_uri: Option<String>,
    pub(crate) metadata: Option<String>,
    pub(crate) last_token_uri_sync: Option<String>,
    pub(crate) last_metadata_sync: Option<String>,
    pub(crate) minter_address: Option<String>,
}

#[derive(Debug)]
pub(crate) struct SerdeStringWrap<T>(pub(crate) T);

impl<'de, T> Deserialize<'de> for SerdeStringWrap<T>
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
        Ok(SerdeStringWrap(value))
    }
}

impl<T> std::ops::Deref for SerdeStringWrap<T> {
    type Target = T;
    fn deref(&self) -> &T { &self.0 }
}

#[derive(Debug, Serialize)]
pub struct NftList {
    pub(crate) count: u64,
    pub(crate) nfts: Vec<Nft>,
}

#[allow(dead_code)]
#[derive(Clone, Deserialize)]
pub struct WithdrawErc1155Request {
    pub(crate) coin: String,
    from: String,
    to: String,
    token_address: String,
    token_id: BigDecimal,
    amount: BigDecimal,
    #[serde(default)]
    max: bool,
    fee: Option<WithdrawFee>,
}

#[allow(dead_code)]
#[derive(Clone, Deserialize)]
pub struct WithdrawErc721Request {
    pub(crate) coin: String,
    from: String,
    to: String,
    token_address: String,
    token_id: BigDecimal,
    fee: Option<WithdrawFee>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize)]
pub struct NftTransfersReq {
    pub(crate) chains: Vec<Chain>,
}

#[derive(Debug, Serialize)]
pub struct NftTransferHistory {
    pub(crate) chain: Chain,
    pub(crate) block_number: u64,
    pub(crate) block_timestamp: String,
    pub(crate) block_hash: String,
    /// Transaction hash in hexadecimal format
    pub(crate) transaction_hash: String,
    pub(crate) transaction_index: u64,
    pub(crate) log_index: u64,
    pub(crate) value: BigDecimal,
    pub(crate) contract_type: ContractType,
    pub(crate) transaction_type: String,
    pub(crate) token_address: String,
    pub(crate) token_id: BigDecimal,
    pub(crate) from_address: String,
    pub(crate) to_address: String,
    pub(crate) amount: BigDecimal,
    pub(crate) verified: u64,
    pub(crate) operator: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NftTransferHistoryWrapper {
    pub(crate) block_number: SerdeStringWrap<u64>,
    pub(crate) block_timestamp: String,
    pub(crate) block_hash: String,
    /// Transaction hash in hexadecimal format
    pub(crate) transaction_hash: String,
    pub(crate) transaction_index: u64,
    pub(crate) log_index: u64,
    pub(crate) value: SerdeStringWrap<BigDecimal>,
    pub(crate) contract_type: SerdeStringWrap<ContractType>,
    pub(crate) transaction_type: String,
    pub(crate) token_address: String,
    pub(crate) token_id: SerdeStringWrap<BigDecimal>,
    pub(crate) from_address: String,
    pub(crate) to_address: String,
    pub(crate) amount: SerdeStringWrap<BigDecimal>,
    pub(crate) verified: u64,
    pub(crate) operator: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NftsTransferHistoryList {
    pub(crate) count: u64,
    pub(crate) transfer_history: Vec<NftTransferHistory>,
}
