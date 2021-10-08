// RPC MESSAGES

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetMempoolInfoRequest {
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetMempoolInfoResponse {
    /// The count of transactions in the mempool
    #[prost(uint32, tag="1")]
    pub size: u32,
    /// The size in bytes of all transactions in the mempool
    #[prost(uint32, tag="2")]
    pub bytes: u32,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetMempoolRequest {
    /// When `full_transactions` is true, full transaction data is provided
    /// instead of just transaction hashes. Default is false.
    #[prost(bool, tag="1")]
    pub full_transactions: bool,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetMempoolResponse {
    /// List of unconfirmed transactions.
    #[prost(message, repeated, tag="1")]
    pub transaction_data: ::prost::alloc::vec::Vec<get_mempool_response::TransactionData>,
}
/// Nested message and enum types in `GetMempoolResponse`.
pub mod get_mempool_response {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct TransactionData {
        /// Either one of the two following is provided, depending on the request.
        #[prost(oneof="transaction_data::TxidsOrTxs", tags="1, 2")]
        pub txids_or_txs: ::core::option::Option<transaction_data::TxidsOrTxs>,
    }
    /// Nested message and enum types in `TransactionData`.
    pub mod transaction_data {
        /// Either one of the two following is provided, depending on the request.
        #[derive(Clone, PartialEq, ::prost::Oneof)]
        pub enum TxidsOrTxs {
            /// The transaction hash, little-endian.
            #[prost(bytes, tag="1")]
            TransactionHash(::prost::alloc::vec::Vec<u8>),
            /// The transaction data.
            #[prost(message, tag="2")]
            Transaction(super::super::Transaction),
        }
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetBlockchainInfoRequest {
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetBlockchainInfoResponse {
    /// Which network the node is operating on.
    #[prost(enumeration="get_blockchain_info_response::BitcoinNet", tag="1")]
    pub bitcoin_net: i32,
    /// The current number of blocks on the longest chain.
    #[prost(int32, tag="2")]
    pub best_height: i32,
    /// The hash of the best (tip) block in the most-work fully-validated chain, little-endian.
    #[prost(bytes="vec", tag="3")]
    pub best_block_hash: ::prost::alloc::vec::Vec<u8>,
    /// Threshold for adding new blocks.
    #[prost(double, tag="4")]
    pub difficulty: f64,
    /// Median time of the last 11 blocks.
    #[prost(int64, tag="5")]
    pub median_time: i64,
    /// When `tx_index` is true, the node has full transaction index enabled.
    #[prost(bool, tag="6")]
    pub tx_index: bool,
    /// When `addr_index` is true, the node has address index enabled and may
    /// be used with call related by address.
    #[prost(bool, tag="7")]
    pub addr_index: bool,
    /// When `slp_index` is true, the node has the slp index enabled and may
    /// be used with slp related rpc methods and also causes slp metadata to be added
    /// in some of the existing rpc methods.
    #[prost(bool, tag="8")]
    pub slp_index: bool,
    /// When `slp_graphsearch` is true, the node is able to handle calls to slp graph search
    #[prost(bool, tag="9")]
    pub slp_graphsearch: bool,
}
/// Nested message and enum types in `GetBlockchainInfoResponse`.
pub mod get_blockchain_info_response {
    /// Bitcoin network types
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum BitcoinNet {
        /// Live public network with monetary value.
        Mainnet = 0,
        /// An isolated environment for automated testing.
        Regtest = 1,
        /// A public environment where monetary value is agreed to be zero,
        /// and some checks for transaction conformity are disabled.
        Testnet3 = 2,
        /// Private testnets for large scale simulations (or stress testing),
        /// where a specified list of nodes is used, rather than node discovery.
        Simnet = 3,
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetBlockInfoRequest {
    #[prost(oneof="get_block_info_request::HashOrHeight", tags="1, 2")]
    pub hash_or_height: ::core::option::Option<get_block_info_request::HashOrHeight>,
}
/// Nested message and enum types in `GetBlockInfoRequest`.
pub mod get_block_info_request {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum HashOrHeight {
        /// The block hash as a byte array or base64 encoded string, little-endian.
        #[prost(bytes, tag="1")]
        Hash(::prost::alloc::vec::Vec<u8>),
        /// The block number.
        #[prost(int32, tag="2")]
        Height(i32),
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetBlockInfoResponse {
    /// Marshaled block header data, as well as metadata.
    #[prost(message, optional, tag="1")]
    pub info: ::core::option::Option<BlockInfo>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetBlockRequest {
    /// When `full_transactions` is true, full transactions are returned
    /// instead of just hashes. Default is false.
    #[prost(bool, tag="3")]
    pub full_transactions: bool,
    #[prost(oneof="get_block_request::HashOrHeight", tags="1, 2")]
    pub hash_or_height: ::core::option::Option<get_block_request::HashOrHeight>,
}
/// Nested message and enum types in `GetBlockRequest`.
pub mod get_block_request {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum HashOrHeight {
        /// The block hash as a byte array or base64 encoded string, little-endian.
        #[prost(bytes, tag="1")]
        Hash(::prost::alloc::vec::Vec<u8>),
        /// The block number.
        #[prost(int32, tag="2")]
        Height(i32),
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetBlockResponse {
    /// A marshaled block.
    #[prost(message, optional, tag="1")]
    pub block: ::core::option::Option<Block>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetRawBlockRequest {
    #[prost(oneof="get_raw_block_request::HashOrHeight", tags="1, 2")]
    pub hash_or_height: ::core::option::Option<get_raw_block_request::HashOrHeight>,
}
/// Nested message and enum types in `GetRawBlockRequest`.
pub mod get_raw_block_request {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum HashOrHeight {
        /// The block hash as a byte array or base64 encoded string, little-endian.
        #[prost(bytes, tag="1")]
        Hash(::prost::alloc::vec::Vec<u8>),
        /// The block number.
        #[prost(int32, tag="2")]
        Height(i32),
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetRawBlockResponse {
    /// Raw block data (with header) serialized according the the bitcoin block protocol.
    #[prost(bytes="vec", tag="1")]
    pub block: ::prost::alloc::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetBlockFilterRequest {
    #[prost(oneof="get_block_filter_request::HashOrHeight", tags="1, 2")]
    pub hash_or_height: ::core::option::Option<get_block_filter_request::HashOrHeight>,
}
/// Nested message and enum types in `GetBlockFilterRequest`.
pub mod get_block_filter_request {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum HashOrHeight {
        /// The block hash as a byte array or base64 encoded string, little-endian.
        #[prost(bytes, tag="1")]
        Hash(::prost::alloc::vec::Vec<u8>),
        /// The block number.
        #[prost(int32, tag="2")]
        Height(i32),
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetBlockFilterResponse {
    /// A compact filter matching input outpoints and public key scripts contained
    /// in a block (encoded according to BIP158).
    #[prost(bytes="vec", tag="1")]
    pub filter: ::prost::alloc::vec::Vec<u8>,
}
/// Request headers using a list of known block hashes.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetHeadersRequest {
    /// A list of block hashes known to the client (most recent first) which
    /// is exponentially sparser toward the genesis block (0), little-endian.
    /// Common practice is to include all of the last 10 blocks, and then
    /// 9 blocks for each order of ten thereafter.
    #[prost(bytes="vec", repeated, tag="1")]
    pub block_locator_hashes: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
    /// hash of the latest desired block header, little-endian; only blocks
    /// occurring before the stop will be returned.
    #[prost(bytes="vec", tag="2")]
    pub stop_hash: ::prost::alloc::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetHeadersResponse {
    /// List of block headers.
    #[prost(message, repeated, tag="1")]
    pub headers: ::prost::alloc::vec::Vec<BlockInfo>,
}
/// Get a transaction from a transaction hash.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetTransactionRequest {
    /// A transaction hash, little-endian.
    #[prost(bytes="vec", tag="1")]
    pub hash: ::prost::alloc::vec::Vec<u8>,
    #[prost(bool, tag="2")]
    pub include_token_metadata: bool,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetTransactionResponse {
    /// A marshaled transaction.
    #[prost(message, optional, tag="1")]
    pub transaction: ::core::option::Option<Transaction>,
    #[prost(message, optional, tag="2")]
    pub token_metadata: ::core::option::Option<SlpTokenMetadata>,
}
/// Get an encoded transaction from a transaction hash.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetRawTransactionRequest {
    /// A transaction hash, little-endian.
    #[prost(bytes="vec", tag="1")]
    pub hash: ::prost::alloc::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetRawTransactionResponse {
    /// Raw transaction in bytes.
    #[prost(bytes="vec", tag="1")]
    pub transaction: ::prost::alloc::vec::Vec<u8>,
}
/// Get marshaled transactions related to a specific address.
///
/// RECOMMENDED:
/// Parameters have been provided to query without creating
///   performance issues on the node or client.
///
/// - The number of transactions to skip and fetch allow for iterating
///       over a large set of transactions, if necessary.
///
/// - A starting block parameter (either `hash` or `height`)
///       may then be used to filter results to those occurring
///       after a certain time.
///
/// This approach will reduce network traffic and response processing
///   for the client, as well as reduce workload on the node.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetAddressTransactionsRequest {
    /// The address to query transactions, in lowercase cashaddr format.
    /// The network prefix is optional (i.e. "cashaddress:").
    #[prost(string, tag="1")]
    pub address: ::prost::alloc::string::String,
    /// The number of confirmed transactions to skip, starting with the oldest first.
    /// Does not affect results of unconfirmed transactions.
    #[prost(uint32, tag="2")]
    pub nb_skip: u32,
    /// Specify the number of transactions to fetch.
    #[prost(uint32, tag="3")]
    pub nb_fetch: u32,
    #[prost(oneof="get_address_transactions_request::StartBlock", tags="4, 5")]
    pub start_block: ::core::option::Option<get_address_transactions_request::StartBlock>,
}
/// Nested message and enum types in `GetAddressTransactionsRequest`.
pub mod get_address_transactions_request {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum StartBlock {
        /// Recommended. Only get transactions after (or within) a
        /// starting block identified by hash, little-endian.
        #[prost(bytes, tag="4")]
        Hash(::prost::alloc::vec::Vec<u8>),
        /// Recommended. Only get transactions after (or within) a
        /// starting block identified by block number.
        #[prost(int32, tag="5")]
        Height(i32),
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetAddressTransactionsResponse {
    /// Transactions that have been included in a block.
    #[prost(message, repeated, tag="1")]
    pub confirmed_transactions: ::prost::alloc::vec::Vec<Transaction>,
    /// Transactions in mempool which have not been included in a block.
    #[prost(message, repeated, tag="2")]
    pub unconfirmed_transactions: ::prost::alloc::vec::Vec<MempoolTransaction>,
}
/// Get encoded transactions related to a specific address.
///
/// RECOMMENDED:
/// Parameters have been provided to query without creating
///   performance issues on the node or client.
///
/// - The number of transactions to skip and fetch allow for iterating
///       over a large set of transactions, if necessary.
///
/// - A starting block parameter (either `hash` or `height`)
///       may then be used to filter results to those occurring
///       after a certain time.
///
/// This approach will reduce network traffic and response processing
///   for the client, as well as reduce workload on the node.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetRawAddressTransactionsRequest {
    /// The address to query transactions, in lowercase cashaddr format.
    /// The network prefix is optional (i.e. "cashaddress:").
    #[prost(string, tag="1")]
    pub address: ::prost::alloc::string::String,
    /// The number of confirmed transactions to skip, starting with the oldest first.
    /// Does not affect results of unconfirmed transactions.
    #[prost(uint32, tag="2")]
    pub nb_skip: u32,
    /// Specify the number of transactions to fetch.
    #[prost(uint32, tag="3")]
    pub nb_fetch: u32,
    #[prost(oneof="get_raw_address_transactions_request::StartBlock", tags="4, 5")]
    pub start_block: ::core::option::Option<get_raw_address_transactions_request::StartBlock>,
}
/// Nested message and enum types in `GetRawAddressTransactionsRequest`.
pub mod get_raw_address_transactions_request {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum StartBlock {
        /// Recommended. Only return transactions after some starting block
        /// identified by hash, little-endian.
        #[prost(bytes, tag="4")]
        Hash(::prost::alloc::vec::Vec<u8>),
        /// Recommended. Only return transactions after some starting block
        /// identified by block number.
        #[prost(int32, tag="5")]
        Height(i32),
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetRawAddressTransactionsResponse {
    /// Transactions that have been included in a block.
    #[prost(bytes="vec", repeated, tag="1")]
    pub confirmed_transactions: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
    /// Transactions in mempool which have not been included in a block.
    #[prost(bytes="vec", repeated, tag="2")]
    pub unconfirmed_transactions: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetAddressUnspentOutputsRequest {
    /// The address to query transactions, in lowercase cashaddr format.
    /// The network identifier is optional (i.e. "cashaddress:").
    #[prost(string, tag="1")]
    pub address: ::prost::alloc::string::String,
    /// When `include_mempool` is true, unconfirmed transactions from mempool
    /// are returned. Default is false.
    #[prost(bool, tag="2")]
    pub include_mempool: bool,
    #[prost(bool, tag="3")]
    pub include_token_metadata: bool,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetAddressUnspentOutputsResponse {
    /// List of unspent outputs.
    #[prost(message, repeated, tag="1")]
    pub outputs: ::prost::alloc::vec::Vec<UnspentOutput>,
    #[prost(message, repeated, tag="2")]
    pub token_metadata: ::prost::alloc::vec::Vec<SlpTokenMetadata>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetUnspentOutputRequest {
    /// The hash of the transaction, little-endian.
    #[prost(bytes="vec", tag="1")]
    pub hash: ::prost::alloc::vec::Vec<u8>,
    /// The number of the output, starting from zero.
    #[prost(uint32, tag="2")]
    pub index: u32,
    /// When include_mempool is true, unconfirmed transactions from mempool
    /// are returned. Default is false.
    #[prost(bool, tag="3")]
    pub include_mempool: bool,
    #[prost(bool, tag="4")]
    pub include_token_metadata: bool,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetUnspentOutputResponse {
    /// A reference to the related input.
    #[prost(message, optional, tag="1")]
    pub outpoint: ::core::option::Option<transaction::input::Outpoint>,
    /// Locking script dictating how funds can be spent in the future
    #[prost(bytes="vec", tag="2")]
    pub pubkey_script: ::prost::alloc::vec::Vec<u8>,
    /// Amount in satoshi.
    #[prost(int64, tag="3")]
    pub value: i64,
    /// When is_coinbase is true, the transaction was the first in a block,
    /// created by a miner, and used to pay the block reward
    #[prost(bool, tag="4")]
    pub is_coinbase: bool,
    /// The index number of the block containing the transaction creating the output.
    #[prost(int32, tag="5")]
    pub block_height: i32,
    #[prost(message, optional, tag="6")]
    pub slp_token: ::core::option::Option<SlpToken>,
    #[prost(message, optional, tag="7")]
    pub token_metadata: ::core::option::Option<SlpTokenMetadata>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetMerkleProofRequest {
    /// A transaction hash, little-endian.
    #[prost(bytes="vec", tag="1")]
    pub transaction_hash: ::prost::alloc::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetMerkleProofResponse {
    /// Block header information for the corresponding transaction
    #[prost(message, optional, tag="1")]
    pub block: ::core::option::Option<BlockInfo>,
    /// A list containing the transaction hash, the adjacent leaf transaction hash
    /// and the hashes of the highest nodes in the merkle tree not built with the transaction.
    /// Proof hashes are ordered following transaction order, or left to right on the merkle tree
    #[prost(bytes="vec", repeated, tag="2")]
    pub hashes: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
    /// Binary representing the location of the matching transaction in the full merkle tree,
    /// starting with the root (`1`) at position/level 0, where `1` corresponds
    /// to a left branch and `01` is a right branch.
    #[prost(bytes="vec", tag="3")]
    pub flags: ::prost::alloc::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SubmitTransactionRequest {
    /// The encoded transaction.
    #[prost(bytes="vec", tag="1")]
    pub transaction: ::prost::alloc::vec::Vec<u8>,
    #[prost(bool, tag="2")]
    pub skip_slp_validity_check: bool,
    #[prost(message, repeated, tag="3")]
    pub required_slp_burns: ::prost::alloc::vec::Vec<SlpRequiredBurn>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SubmitTransactionResponse {
    /// Transaction hash, little-endian.
    #[prost(bytes="vec", tag="1")]
    pub hash: ::prost::alloc::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CheckSlpTransactionRequest {
    #[prost(bytes="vec", tag="1")]
    pub transaction: ::prost::alloc::vec::Vec<u8>,
    #[prost(message, repeated, tag="2")]
    pub required_slp_burns: ::prost::alloc::vec::Vec<SlpRequiredBurn>,
    /// Using the slp specification as a basis for validity judgement can lead to confusion for new users and
    /// result in accidental token burns.  use_spec_validity_judgement will cause the response's is_valid property
    /// to be returned according to the slp specification.  Therefore, use_spec_validity_judgement is false by
    /// default in order to avoid accidental token burns.  When use_spec_validity_judgement is false we return
    /// invalid in any case which would result in a burned token, unless the burn is explicitly included as an
    /// item in required_slp_burns property.
    ///
    /// When use_spec_validity_judgement is true, there are three cases where the is_valid response property
    /// will be returned as valid, instead of invalid, as per the slp specification.
    ///   1) inputs > outputs
    ///   2) missing transaction outputs
    ///   3) burned inputs from other tokens
    ///
    /// required_slp_burns is not used when use_spec_validity_judgement is set to true.
    ///
    #[prost(bool, tag="3")]
    pub use_spec_validity_judgement: bool,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CheckSlpTransactionResponse {
    #[prost(bool, tag="1")]
    pub is_valid: bool,
    #[prost(string, tag="2")]
    pub invalid_reason: ::prost::alloc::string::String,
    #[prost(int32, tag="3")]
    pub best_height: i32,
}
/// Request to subscribe or unsubscribe from a stream of transactions.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SubscribeTransactionsRequest {
    /// Subscribe to a filter. add items to a filter
    #[prost(message, optional, tag="1")]
    pub subscribe: ::core::option::Option<TransactionFilter>,
    /// Unsubscribe to a filter, remove items from a filter
    #[prost(message, optional, tag="2")]
    pub unsubscribe: ::core::option::Option<TransactionFilter>,
    /// When include_mempool is true, new unconfirmed transactions from mempool are
    /// included apart from the ones confirmed in a block.
    #[prost(bool, tag="3")]
    pub include_mempool: bool,
    /// When include_in_block is true, transactions are included when they are confirmed.
    /// This notification is sent in addition to any requested mempool notifications.
    #[prost(bool, tag="4")]
    pub include_in_block: bool,
    /// When serialize_tx is true, transactions are serialized using
    /// bitcoin protocol encoding. Default is false, transaction will be Marshaled
    /// (see `Transaction`, `MempoolTransaction` and `TransactionNotification`)
    #[prost(bool, tag="5")]
    pub serialize_tx: bool,
}
/// Options to define data structure to be sent by SubscribeBlock stream:
///
///  - BlockInfo (block metadata): `BlockInfo`
///      - SubscribeBlocksRequest {}
///
///  - Marshaled Block (with transaction hashes): `Block`
///      - SubscribeBlocksRequest {
///            full_block = true
///        }
///  - Marshaled Block (with full transaction data): `Block`
///      - SubscribeBlocksRequest {
///            full_block = true
///            full_transactions = true
///        }
///  - Serialized Block acccording to bitcoin protocol encoding: `bytes`
///      - SubscribeBlocksRequest {
///            serialize_block = true
///        }
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SubscribeBlocksRequest {
    /// When full_block is true, a complete marshaled block is sent. See `Block`.
    /// Default is false, block metadata is sent. See `BlockInfo`.
    #[prost(bool, tag="1")]
    pub full_block: bool,
    /// When full_transactions is true, provide full transaction info
    /// for a marshaled block.
    /// Default is false, only the transaction hashes are included for
    /// a marshaled block. See `TransactionData`.
    #[prost(bool, tag="2")]
    pub full_transactions: bool,
    /// When serialize_block is true, blocks are serialized using bitcoin protocol encoding.
    /// Default is false, block will be Marshaled (see `BlockInfo` and `BlockNotification`)
    #[prost(bool, tag="3")]
    pub serialize_block: bool,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetSlpTokenMetadataRequest {
    #[prost(bytes="vec", repeated, tag="1")]
    pub token_ids: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetSlpTokenMetadataResponse {
    #[prost(message, repeated, tag="1")]
    pub token_metadata: ::prost::alloc::vec::Vec<SlpTokenMetadata>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetSlpParsedScriptRequest {
    #[prost(bytes="vec", tag="1")]
    pub slp_opreturn_script: ::prost::alloc::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetSlpParsedScriptResponse {
    #[prost(string, tag="1")]
    pub parsing_error: ::prost::alloc::string::String,
    #[prost(bytes="vec", tag="2")]
    pub token_id: ::prost::alloc::vec::Vec<u8>,
    #[prost(enumeration="SlpAction", tag="3")]
    pub slp_action: i32,
    #[prost(enumeration="SlpTokenType", tag="4")]
    pub token_type: i32,
    #[prost(oneof="get_slp_parsed_script_response::SlpMetadata", tags="5, 6, 7, 8, 9")]
    pub slp_metadata: ::core::option::Option<get_slp_parsed_script_response::SlpMetadata>,
}
/// Nested message and enum types in `GetSlpParsedScriptResponse`.
pub mod get_slp_parsed_script_response {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum SlpMetadata {
        /// NFT1 Group also uses this
        #[prost(message, tag="5")]
        V1Genesis(super::SlpV1GenesisMetadata),
        /// NFT1 Group also uses this
        #[prost(message, tag="6")]
        V1Mint(super::SlpV1MintMetadata),
        /// NFT1 Group also uses this
        #[prost(message, tag="7")]
        V1Send(super::SlpV1SendMetadata),
        #[prost(message, tag="8")]
        V1Nft1ChildGenesis(super::SlpV1Nft1ChildGenesisMetadata),
        #[prost(message, tag="9")]
        V1Nft1ChildSend(super::SlpV1Nft1ChildSendMetadata),
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetSlpTrustedValidationRequest {
    #[prost(message, repeated, tag="1")]
    pub queries: ::prost::alloc::vec::Vec<get_slp_trusted_validation_request::Query>,
    #[prost(bool, tag="2")]
    pub include_graphsearch_count: bool,
}
/// Nested message and enum types in `GetSlpTrustedValidationRequest`.
pub mod get_slp_trusted_validation_request {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Query {
        #[prost(bytes="vec", tag="1")]
        pub prev_out_hash: ::prost::alloc::vec::Vec<u8>,
        #[prost(uint32, tag="2")]
        pub prev_out_vout: u32,
        #[prost(bytes="vec", repeated, tag="3")]
        pub graphsearch_valid_hashes: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetSlpTrustedValidationResponse {
    #[prost(message, repeated, tag="1")]
    pub results: ::prost::alloc::vec::Vec<get_slp_trusted_validation_response::ValidityResult>,
}
/// Nested message and enum types in `GetSlpTrustedValidationResponse`.
pub mod get_slp_trusted_validation_response {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct ValidityResult {
        #[prost(bytes="vec", tag="1")]
        pub prev_out_hash: ::prost::alloc::vec::Vec<u8>,
        #[prost(uint32, tag="2")]
        pub prev_out_vout: u32,
        #[prost(bytes="vec", tag="3")]
        pub token_id: ::prost::alloc::vec::Vec<u8>,
        #[prost(enumeration="super::SlpAction", tag="4")]
        pub slp_action: i32,
        #[prost(enumeration="super::SlpTokenType", tag="5")]
        pub token_type: i32,
        #[prost(bytes="vec", tag="8")]
        pub slp_txn_opreturn: ::prost::alloc::vec::Vec<u8>,
        #[prost(uint32, tag="9")]
        pub graphsearch_txn_count: u32,
        #[prost(oneof="validity_result::ValidityResultType", tags="6, 7")]
        pub validity_result_type: ::core::option::Option<validity_result::ValidityResultType>,
    }
    /// Nested message and enum types in `ValidityResult`.
    pub mod validity_result {
        #[derive(Clone, PartialEq, ::prost::Oneof)]
        pub enum ValidityResultType {
            #[prost(uint64, tag="6")]
            V1TokenAmount(u64),
            #[prost(bool, tag="7")]
            V1MintBaton(bool),
        }
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetSlpGraphSearchRequest {
    #[prost(bytes="vec", tag="1")]
    pub hash: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", repeated, tag="2")]
    pub valid_hashes: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct GetSlpGraphSearchResponse {
    #[prost(bytes="vec", repeated, tag="1")]
    pub txdata: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
}
// NOTIFICATIONS

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct BlockNotification {
    /// Whether the block is connected to the chain.
    #[prost(enumeration="block_notification::Type", tag="1")]
    pub r#type: i32,
    #[prost(oneof="block_notification::Block", tags="2, 3, 4")]
    pub block: ::core::option::Option<block_notification::Block>,
}
/// Nested message and enum types in `BlockNotification`.
pub mod block_notification {
    /// State of the block in relation to the chain.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum Type {
        Connected = 0,
        Disconnected = 1,
    }
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Block {
        /// Marshaled block header data, as well as metadata stored by the node.
        #[prost(message, tag="2")]
        BlockInfo(super::BlockInfo),
        /// A Block.
        #[prost(message, tag="3")]
        MarshaledBlock(super::Block),
        /// Binary block, serialized using bitcoin protocol encoding.
        #[prost(bytes, tag="4")]
        SerializedBlock(::prost::alloc::vec::Vec<u8>),
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TransactionNotification {
    /// Whether or not the transaction has been included in a block.
    #[prost(enumeration="transaction_notification::Type", tag="1")]
    pub r#type: i32,
    #[prost(oneof="transaction_notification::Transaction", tags="2, 3, 4")]
    pub transaction: ::core::option::Option<transaction_notification::Transaction>,
}
/// Nested message and enum types in `TransactionNotification`.
pub mod transaction_notification {
    /// State of the transaction acceptance.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum Type {
        /// A transaction in mempool.
        Unconfirmed = 0,
        /// A transaction in a block.
        Confirmed = 1,
    }
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Transaction {
        /// A transaction included in a block.
        #[prost(message, tag="2")]
        ConfirmedTransaction(super::Transaction),
        /// A transaction in mempool.
        #[prost(message, tag="3")]
        UnconfirmedTransaction(super::MempoolTransaction),
        /// Binary transaction, serialized using bitcoin protocol encoding.
        #[prost(bytes, tag="4")]
        SerializedTransaction(::prost::alloc::vec::Vec<u8>),
    }
}
// DATA MESSAGES

/// Metadata for identifying and validating a block
///
/// Identification.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct BlockInfo {
    /// The double sha256 hash of the six header fields in the first 80 bytes
    /// of the block, when encoded according the bitcoin protocol, little-endian.
    /// sha256(sha256(encoded_header))
    #[prost(bytes="vec", tag="1")]
    pub hash: ::prost::alloc::vec::Vec<u8>,
    /// The block number, an incremental index for each block mined.
    #[prost(int32, tag="2")]
    pub height: i32,
    // Block header data.

    /// A version number to track software/protocol upgrades.
    #[prost(int32, tag="3")]
    pub version: i32,
    /// Hash of the previous block, little-endian.
    #[prost(bytes="vec", tag="4")]
    pub previous_block: ::prost::alloc::vec::Vec<u8>,
    /// The root of the Merkle Tree built from all transactions in the block, little-endian.
    #[prost(bytes="vec", tag="5")]
    pub merkle_root: ::prost::alloc::vec::Vec<u8>,
    /// When mining of the block started, expressed in seconds since 1970-01-01.
    #[prost(int64, tag="6")]
    pub timestamp: i64,
    /// Difficulty in Compressed Target Format.
    #[prost(uint32, tag="7")]
    pub bits: u32,
    /// A random value that was generated during block mining which happened to
    /// result in a computed block hash below the difficulty target at the time.
    #[prost(uint32, tag="8")]
    pub nonce: u32,
    // Metadata.

    /// Number of blocks in a chain, including the block itself upon creation.
    #[prost(int32, tag="9")]
    pub confirmations: i32,
    /// Difficulty target at time of creation.
    #[prost(double, tag="10")]
    pub difficulty: f64,
    /// Hash of the next block in this chain, little-endian.
    #[prost(bytes="vec", tag="11")]
    pub next_block_hash: ::prost::alloc::vec::Vec<u8>,
    /// Size of the block in bytes.
    #[prost(int32, tag="12")]
    pub size: i32,
    /// The median block time of the latest 11 block timestamps.
    #[prost(int64, tag="13")]
    pub median_time: i64,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Block {
    /// Block header data, as well as metadata stored by the node.
    #[prost(message, optional, tag="1")]
    pub info: ::core::option::Option<BlockInfo>,
    /// List of transactions or transaction hashes.
    #[prost(message, repeated, tag="2")]
    pub transaction_data: ::prost::alloc::vec::Vec<block::TransactionData>,
}
/// Nested message and enum types in `Block`.
pub mod block {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct TransactionData {
        #[prost(oneof="transaction_data::TxidsOrTxs", tags="1, 2")]
        pub txids_or_txs: ::core::option::Option<transaction_data::TxidsOrTxs>,
    }
    /// Nested message and enum types in `TransactionData`.
    pub mod transaction_data {
        #[derive(Clone, PartialEq, ::prost::Oneof)]
        pub enum TxidsOrTxs {
            /// Just the transaction hash, little-endian.
            #[prost(bytes, tag="1")]
            TransactionHash(::prost::alloc::vec::Vec<u8>),
            /// A marshaled transaction.
            #[prost(message, tag="2")]
            Transaction(super::super::Transaction),
        }
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Transaction {
    /// The double sha256 hash of the encoded transaction, little-endian.
    /// sha256(sha256(encoded_transaction))
    #[prost(bytes="vec", tag="1")]
    pub hash: ::prost::alloc::vec::Vec<u8>,
    /// The version of the transaction format.
    #[prost(int32, tag="2")]
    pub version: i32,
    /// List of inputs.
    #[prost(message, repeated, tag="3")]
    pub inputs: ::prost::alloc::vec::Vec<transaction::Input>,
    /// List of outputs.
    #[prost(message, repeated, tag="4")]
    pub outputs: ::prost::alloc::vec::Vec<transaction::Output>,
    /// The block height or timestamp after which this transaction is allowed.
    /// If value is greater than 500 million, it is assumed to be an epoch timestamp,
    /// otherwise it is treated as a block-height. Default is zero, or lock.
    #[prost(uint32, tag="5")]
    pub lock_time: u32,
    // Metadata

    /// The size of the transaction in bytes.
    #[prost(int32, tag="8")]
    pub size: i32,
    /// When the transaction was included in a block, in epoch time.
    #[prost(int64, tag="9")]
    pub timestamp: i64,
    /// Number of blocks including proof of the transaction, including
    /// the block it appeared.
    #[prost(int32, tag="10")]
    pub confirmations: i32,
    /// Number of the block containing the transaction.
    #[prost(int32, tag="11")]
    pub block_height: i32,
    /// Hash of the block the transaction was recorded in, little-endian.
    #[prost(bytes="vec", tag="12")]
    pub block_hash: ::prost::alloc::vec::Vec<u8>,
    #[prost(message, optional, tag="13")]
    pub slp_transaction_info: ::core::option::Option<SlpTransactionInfo>,
}
/// Nested message and enum types in `Transaction`.
pub mod transaction {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Input {
        /// The number of the input, starting from zero.
        #[prost(uint32, tag="1")]
        pub index: u32,
        /// The related outpoint.
        #[prost(message, optional, tag="2")]
        pub outpoint: ::core::option::Option<input::Outpoint>,
        /// An unlocking script asserting a transaction is permitted to spend
        /// the Outpoint (UTXO)
        #[prost(bytes="vec", tag="3")]
        pub signature_script: ::prost::alloc::vec::Vec<u8>,
        /// As of BIP-68, the sequence number is interpreted as a relative
        /// lock-time for the input.
        #[prost(uint32, tag="4")]
        pub sequence: u32,
        /// Amount in satoshi.
        #[prost(int64, tag="5")]
        pub value: i64,
        /// The pubkey_script of the previous output that is being spent.
        #[prost(bytes="vec", tag="6")]
        pub previous_script: ::prost::alloc::vec::Vec<u8>,
        /// The bitcoin addresses associated with this input.
        #[prost(string, tag="7")]
        pub address: ::prost::alloc::string::String,
        #[prost(message, optional, tag="8")]
        pub slp_token: ::core::option::Option<super::SlpToken>,
    }
    /// Nested message and enum types in `Input`.
    pub mod input {
        #[derive(Clone, PartialEq, ::prost::Message)]
        pub struct Outpoint {
            /// The hash of the transaction containing the output to be spent, little-endian
            #[prost(bytes="vec", tag="1")]
            pub hash: ::prost::alloc::vec::Vec<u8>,
            /// The index of specific output on the transaction.
            #[prost(uint32, tag="2")]
            pub index: u32,
        }
    }
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Output {
        /// The number of the output, starting from zero.
        #[prost(uint32, tag="1")]
        pub index: u32,
        /// The number of satoshis to be transferred.
        #[prost(int64, tag="2")]
        pub value: i64,
        /// The public key script used to pay coins.
        #[prost(bytes="vec", tag="3")]
        pub pubkey_script: ::prost::alloc::vec::Vec<u8>,
        /// The bitcoin addresses associated with this output.
        #[prost(string, tag="4")]
        pub address: ::prost::alloc::string::String,
        /// The type of script.
        #[prost(string, tag="5")]
        pub script_class: ::prost::alloc::string::String,
        /// The script expressed in Bitcoin Cash Script.
        #[prost(string, tag="6")]
        pub disassembled_script: ::prost::alloc::string::String,
        #[prost(message, optional, tag="7")]
        pub slp_token: ::core::option::Option<super::SlpToken>,
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MempoolTransaction {
    #[prost(message, optional, tag="1")]
    pub transaction: ::core::option::Option<Transaction>,
    /// The time when the transaction was added too the pool.
    #[prost(int64, tag="2")]
    pub added_time: i64,
    /// The block height when the transaction was added to the pool.
    #[prost(int32, tag="3")]
    pub added_height: i32,
    /// The total fee in satoshi the transaction pays.
    #[prost(int64, tag="4")]
    pub fee: i64,
    /// The fee in satoshi per kilobyte the transaction pays.
    #[prost(int64, tag="5")]
    pub fee_per_kb: i64,
    /// The priority of the transaction when it was added to the pool.
    #[prost(double, tag="6")]
    pub starting_priority: f64,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct UnspentOutput {
    /// A reference to the output given by transaction hash and index.
    #[prost(message, optional, tag="1")]
    pub outpoint: ::core::option::Option<transaction::input::Outpoint>,
    /// The public key script used to pay coins.
    #[prost(bytes="vec", tag="2")]
    pub pubkey_script: ::prost::alloc::vec::Vec<u8>,
    /// The amount in satoshis
    #[prost(int64, tag="3")]
    pub value: i64,
    /// When is_coinbase is true, the output is the first in the block,
    /// a generation transaction, the result of mining.
    #[prost(bool, tag="4")]
    pub is_coinbase: bool,
    /// The block number containing the UXTO.
    #[prost(int32, tag="5")]
    pub block_height: i32,
    #[prost(message, optional, tag="6")]
    pub slp_token: ::core::option::Option<SlpToken>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TransactionFilter {
    /// Filter by address(es)
    #[prost(string, repeated, tag="1")]
    pub addresses: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
    /// Filter by output hash and index.
    #[prost(message, repeated, tag="2")]
    pub outpoints: ::prost::alloc::vec::Vec<transaction::input::Outpoint>,
    /// Filter by data elements contained in pubkey scripts.
    #[prost(bytes="vec", repeated, tag="3")]
    pub data_elements: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
    /// Subscribed/Unsubscribe to everything. Other filters
    /// will be ignored.
    #[prost(bool, tag="4")]
    pub all_transactions: bool,
    /// Subscribed/Unsubscribe to everything slp. Other filters
    /// will be ignored, except this filter will be overriden by all_transactions=true
    #[prost(bool, tag="5")]
    pub all_slp_transactions: bool,
    /// only transactions associated with the included tokenIds
    #[prost(bytes="vec", repeated, tag="6")]
    pub slp_token_ids: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
}
/// SlpToken info used in transaction inputs / outputs
///
/// WARNING: Some languages (e.g., JavaScript) may not properly handle the 'uint64'
/// for large amounts. For this reason, an annotation has been added for JS to
/// return a string for the amount field instead of casting uint64 to the JS 'number'
/// type. Other languages may require similar treatment.
///
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SlpToken {
    #[prost(bytes="vec", tag="1")]
    pub token_id: ::prost::alloc::vec::Vec<u8>,
    #[prost(uint64, tag="2")]
    pub amount: u64,
    #[prost(bool, tag="3")]
    pub is_mint_baton: bool,
    #[prost(string, tag="4")]
    pub address: ::prost::alloc::string::String,
    #[prost(uint32, tag="5")]
    pub decimals: u32,
    #[prost(enumeration="SlpAction", tag="6")]
    pub slp_action: i32,
    #[prost(enumeration="SlpTokenType", tag="7")]
    pub token_type: i32,
}
/// SlpTransactionInfo is used inside the Transaction message type.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SlpTransactionInfo {
    #[prost(enumeration="SlpAction", tag="1")]
    pub slp_action: i32,
    #[prost(enumeration="slp_transaction_info::ValidityJudgement", tag="2")]
    pub validity_judgement: i32,
    #[prost(string, tag="3")]
    pub parse_error: ::prost::alloc::string::String,
    #[prost(bytes="vec", tag="4")]
    pub token_id: ::prost::alloc::vec::Vec<u8>,
    #[prost(enumeration="slp_transaction_info::BurnFlags", repeated, tag="5")]
    pub burn_flags: ::prost::alloc::vec::Vec<i32>,
    #[prost(oneof="slp_transaction_info::TxMetadata", tags="6, 7, 8, 9, 10")]
    pub tx_metadata: ::core::option::Option<slp_transaction_info::TxMetadata>,
}
/// Nested message and enum types in `SlpTransactionInfo`.
pub mod slp_transaction_info {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum ValidityJudgement {
        UnknownOrInvalid = 0,
        Valid = 1,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
    #[repr(i32)]
    pub enum BurnFlags {
        BurnedInputsOutputsTooHigh = 0,
        BurnedInputsBadOpreturn = 1,
        BurnedInputsOtherToken = 2,
        BurnedOutputsMissingBchVout = 3,
        BurnedInputsGreaterThanOutputs = 4,
    }
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum TxMetadata {
        /// NFT1 Group also uses this
        #[prost(message, tag="6")]
        V1Genesis(super::SlpV1GenesisMetadata),
        /// NFT1 Group also uses this
        #[prost(message, tag="7")]
        V1Mint(super::SlpV1MintMetadata),
        /// NFT1 Group also uses this
        #[prost(message, tag="8")]
        V1Send(super::SlpV1SendMetadata),
        #[prost(message, tag="9")]
        V1Nft1ChildGenesis(super::SlpV1Nft1ChildGenesisMetadata),
        #[prost(message, tag="10")]
        V1Nft1ChildSend(super::SlpV1Nft1ChildSendMetadata),
    }
}
/// SlpV1GenesisMetadata is used to marshal type 1 and NFT1 Group GENESIS OP_RETURN scriptPubKey
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SlpV1GenesisMetadata {
    #[prost(bytes="vec", tag="1")]
    pub name: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", tag="2")]
    pub ticker: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", tag="3")]
    pub document_url: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", tag="4")]
    pub document_hash: ::prost::alloc::vec::Vec<u8>,
    #[prost(uint32, tag="5")]
    pub decimals: u32,
    #[prost(uint32, tag="6")]
    pub mint_baton_vout: u32,
    #[prost(uint64, tag="7")]
    pub mint_amount: u64,
}
/// SlpV1MintMetadata is used to marshal type 1 MINT OP_RETURN scriptPubKey
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SlpV1MintMetadata {
    #[prost(uint32, tag="1")]
    pub mint_baton_vout: u32,
    #[prost(uint64, tag="2")]
    pub mint_amount: u64,
}
/// SlpV1SendMetadata is used to marshal type 1 and NFT1 Group SEND OP_RETURN scriptPubKey
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SlpV1SendMetadata {
    #[prost(uint64, repeated, packed="false", tag="1")]
    pub amounts: ::prost::alloc::vec::Vec<u64>,
}
/// SlpV1Nft1ChildGenesisMetadata is used to marshal NFT1 Child GENESIS OP_RETURN scriptPubKey
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SlpV1Nft1ChildGenesisMetadata {
    #[prost(bytes="vec", tag="1")]
    pub name: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", tag="2")]
    pub ticker: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", tag="3")]
    pub document_url: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", tag="4")]
    pub document_hash: ::prost::alloc::vec::Vec<u8>,
    #[prost(uint32, tag="5")]
    pub decimals: u32,
    #[prost(bytes="vec", tag="6")]
    pub group_token_id: ::prost::alloc::vec::Vec<u8>,
}
/// SlpV1Nft1ChildSendMetadata is used to marshal NFT1 Child SEND OP_RETURN scriptPubKey
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SlpV1Nft1ChildSendMetadata {
    #[prost(bytes="vec", tag="1")]
    pub group_token_id: ::prost::alloc::vec::Vec<u8>,
}
/// SlpTokenMetadata is used to marshal metadata about a specific TokenID
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SlpTokenMetadata {
    #[prost(bytes="vec", tag="1")]
    pub token_id: ::prost::alloc::vec::Vec<u8>,
    #[prost(enumeration="SlpTokenType", tag="2")]
    pub token_type: i32,
    #[prost(oneof="slp_token_metadata::TypeMetadata", tags="3, 4, 5")]
    pub type_metadata: ::core::option::Option<slp_token_metadata::TypeMetadata>,
}
/// Nested message and enum types in `SlpTokenMetadata`.
pub mod slp_token_metadata {
    /// V1Fungible is used to marshal metadata specific to Type 1 token IDs
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct V1Fungible {
        #[prost(string, tag="1")]
        pub token_ticker: ::prost::alloc::string::String,
        #[prost(string, tag="2")]
        pub token_name: ::prost::alloc::string::String,
        #[prost(string, tag="3")]
        pub token_document_url: ::prost::alloc::string::String,
        #[prost(bytes="vec", tag="4")]
        pub token_document_hash: ::prost::alloc::vec::Vec<u8>,
        #[prost(uint32, tag="5")]
        pub decimals: u32,
        #[prost(bytes="vec", tag="6")]
        pub mint_baton_hash: ::prost::alloc::vec::Vec<u8>,
        #[prost(uint32, tag="7")]
        pub mint_baton_vout: u32,
    }
    /// V1NFT1Group is used to marshal metadata specific to NFT1 Group token IDs
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct V1nft1Group {
        #[prost(string, tag="1")]
        pub token_ticker: ::prost::alloc::string::String,
        #[prost(string, tag="2")]
        pub token_name: ::prost::alloc::string::String,
        #[prost(string, tag="3")]
        pub token_document_url: ::prost::alloc::string::String,
        #[prost(bytes="vec", tag="4")]
        pub token_document_hash: ::prost::alloc::vec::Vec<u8>,
        #[prost(uint32, tag="5")]
        pub decimals: u32,
        #[prost(bytes="vec", tag="6")]
        pub mint_baton_hash: ::prost::alloc::vec::Vec<u8>,
        #[prost(uint32, tag="7")]
        pub mint_baton_vout: u32,
    }
    /// V1NFT1Child is used to marshal metadata specific to NFT1 Child token IDs
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct V1nft1Child {
        #[prost(string, tag="1")]
        pub token_ticker: ::prost::alloc::string::String,
        #[prost(string, tag="2")]
        pub token_name: ::prost::alloc::string::String,
        #[prost(string, tag="3")]
        pub token_document_url: ::prost::alloc::string::String,
        #[prost(bytes="vec", tag="4")]
        pub token_document_hash: ::prost::alloc::vec::Vec<u8>,
        #[prost(bytes="vec", tag="5")]
        pub group_id: ::prost::alloc::vec::Vec<u8>,
    }
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum TypeMetadata {
        #[prost(message, tag="3")]
        V1Fungible(V1Fungible),
        #[prost(message, tag="4")]
        V1Nft1Group(V1nft1Group),
        #[prost(message, tag="5")]
        V1Nft1Child(V1nft1Child),
    }
}
/// SlpRequiredBurn is used by clients to allow token burning
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SlpRequiredBurn {
    #[prost(message, optional, tag="1")]
    pub outpoint: ::core::option::Option<transaction::input::Outpoint>,
    #[prost(bytes="vec", tag="2")]
    pub token_id: ::prost::alloc::vec::Vec<u8>,
    #[prost(enumeration="SlpTokenType", tag="3")]
    pub token_type: i32,
    #[prost(oneof="slp_required_burn::BurnIntention", tags="4, 5")]
    pub burn_intention: ::core::option::Option<slp_required_burn::BurnIntention>,
}
/// Nested message and enum types in `SlpRequiredBurn`.
pub mod slp_required_burn {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum BurnIntention {
        #[prost(uint64, tag="4")]
        Amount(u64),
        #[prost(uint32, tag="5")]
        MintBatonVout(u32),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum SlpTokenType {
    VersionNotSet = 0,
    V1Fungible = 1,
    V1Nft1Child = 65,
    V1Nft1Group = 129,
}
/// SlpAction is used to allow clients to identify the type of slp transaction from this single field.
///
/// NOTE: All enum types except for "NON_SLP" may be annotated with one or more BurnFlags.
///
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum SlpAction {
    NonSlp = 0,
    NonSlpBurn = 1,
    SlpParseError = 2,
    SlpUnsupportedVersion = 3,
    SlpV1Genesis = 4,
    SlpV1Mint = 5,
    SlpV1Send = 6,
    SlpV1Nft1GroupGenesis = 7,
    SlpV1Nft1GroupMint = 8,
    SlpV1Nft1GroupSend = 9,
    SlpV1Nft1UniqueChildGenesis = 10,
    SlpV1Nft1UniqueChildSend = 11,
}
