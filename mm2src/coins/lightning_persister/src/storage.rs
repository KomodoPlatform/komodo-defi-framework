use async_trait::async_trait;
use bitcoin::Network;
use common::{now_ms, PagingOptionsEnum};
use db_common::sqlite::rusqlite::types::FromSqlError;
use derive_more::Display;
use lightning::ln::{PaymentHash, PaymentPreimage, PaymentSecret};
use lightning::routing::network_graph::NetworkGraph;
use lightning::routing::scoring::ProbabilisticScorer;
use parking_lot::Mutex as PaMutex;
use secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

pub type NodesAddressesMap = HashMap<PublicKey, SocketAddr>;
pub type NodesAddressesMapShared = Arc<PaMutex<NodesAddressesMap>>;
pub type Scorer = ProbabilisticScorer<Arc<NetworkGraph>>;
#[async_trait]
pub trait FileSystemStorage {
    type Error;

    /// Initializes dirs/collection/tables in storage for a specified coin
    async fn init_fs(&self) -> Result<(), Self::Error>;

    async fn is_fs_initialized(&self) -> Result<bool, Self::Error>;

    async fn get_nodes_addresses(&self) -> Result<HashMap<PublicKey, SocketAddr>, Self::Error>;

    async fn save_nodes_addresses(&self, nodes_addresses: NodesAddressesMapShared) -> Result<(), Self::Error>;

    async fn get_network_graph(&self, network: Network) -> Result<NetworkGraph, Self::Error>;

    async fn save_network_graph(&self, network_graph: Arc<NetworkGraph>) -> Result<(), Self::Error>;

    async fn get_scorer(&self, network_graph: Arc<NetworkGraph>) -> Result<Scorer, Self::Error>;

    async fn save_scorer(&self, scorer: Arc<Mutex<Scorer>>) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SqlChannelDetails {
    pub rpc_id: u64,
    pub channel_id: String,
    pub counterparty_node_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funding_tx: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funding_value: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closing_tx: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closure_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claiming_tx: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_balance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funding_generated_in_block: Option<u64>,
    pub is_outbound: bool,
    pub is_public: bool,
    pub is_closed: bool,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<u64>,
}

impl SqlChannelDetails {
    #[inline]
    pub fn new(
        rpc_id: u64,
        channel_id: [u8; 32],
        counterparty_node_id: PublicKey,
        is_outbound: bool,
        is_public: bool,
    ) -> Self {
        SqlChannelDetails {
            rpc_id,
            channel_id: hex::encode(channel_id),
            counterparty_node_id: counterparty_node_id.to_string(),
            funding_tx: None,
            funding_value: None,
            funding_generated_in_block: None,
            closing_tx: None,
            closure_reason: None,
            claiming_tx: None,
            claimed_balance: None,
            is_outbound,
            is_public,
            is_closed: false,
            created_at: now_ms() / 1000,
            closed_at: None,
        }
    }
}

#[derive(Clone, Deserialize)]
pub enum ChannelType {
    Outbound,
    Inbound,
}

#[derive(Clone, Deserialize)]
pub enum ChannelVisibility {
    Public,
    Private,
}

#[derive(Clone, Deserialize)]
pub struct ClosedChannelsFilter {
    pub channel_id: Option<String>,
    pub counterparty_node_id: Option<String>,
    pub funding_tx: Option<String>,
    pub from_funding_value: Option<u64>,
    pub to_funding_value: Option<u64>,
    pub closing_tx: Option<String>,
    pub closure_reason: Option<String>,
    pub claiming_tx: Option<String>,
    pub from_claimed_balance: Option<f64>,
    pub to_claimed_balance: Option<f64>,
    pub channel_type: Option<ChannelType>,
    pub channel_visibility: Option<ChannelVisibility>,
}

pub struct GetClosedChannelsResult {
    pub channels: Vec<SqlChannelDetails>,
    pub skipped: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Deserialize, Display, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HTLCStatus {
    Pending,
    Succeeded,
    Failed,
}

impl FromStr for HTLCStatus {
    type Err = FromSqlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(HTLCStatus::Pending),
            "Succeeded" => Ok(HTLCStatus::Succeeded),
            "Failed" => Ok(HTLCStatus::Failed),
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaymentType {
    OutboundPayment { destination: PublicKey },
    InboundPayment,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaymentInfo {
    pub payment_hash: PaymentHash,
    pub payment_type: PaymentType,
    pub description: String,
    pub preimage: Option<PaymentPreimage>,
    pub secret: Option<PaymentSecret>,
    pub amt_msat: Option<u64>,
    pub fee_paid_msat: Option<u64>,
    pub status: HTLCStatus,
    pub created_at: u64,
    pub last_updated: u64,
}

#[derive(Clone)]
pub struct PaymentsFilter {
    pub payment_type: Option<PaymentType>,
    pub description: Option<String>,
    pub status: Option<HTLCStatus>,
    pub from_amount_msat: Option<u64>,
    pub to_amount_msat: Option<u64>,
    pub from_fee_paid_msat: Option<u64>,
    pub to_fee_paid_msat: Option<u64>,
    pub from_timestamp: Option<u64>,
    pub to_timestamp: Option<u64>,
}

pub struct GetPaymentsResult {
    pub payments: Vec<PaymentInfo>,
    pub skipped: usize,
    pub total: usize,
}

#[async_trait]
pub trait DbStorage {
    type Error;

    /// Initializes tables in DB.
    async fn init_db(&self) -> Result<(), Self::Error>;

    /// Checks if tables have been initialized or not in DB.
    async fn is_db_initialized(&self) -> Result<bool, Self::Error>;

    /// Gets the last added channel rpc_id. Can be used to deduce the rpc_id for a new channel to be added to DB.
    async fn get_last_channel_rpc_id(&self) -> Result<u32, Self::Error>;

    /// Inserts a new channel record in the DB. The record's data is completed using add_funding_tx_to_db,
    /// add_closing_tx_to_db, add_claiming_tx_to_db when this information is available.
    async fn add_channel_to_db(&self, details: SqlChannelDetails) -> Result<(), Self::Error>;

    /// Updates a channel's DB record with the channel's funding transaction information.
    async fn add_funding_tx_to_db(
        &self,
        rpc_id: u64,
        funding_tx: String,
        funding_value: u64,
        funding_generated_in_block: u64,
    ) -> Result<(), Self::Error>;

    /// Updates funding_tx_block_height value for a channel in the DB. Should be used to update the block height of
    /// the funding tx when the transaction is confirmed on-chain.
    async fn update_funding_tx_block_height(&self, funding_tx: String, block_height: u64) -> Result<(), Self::Error>;

    /// Updates the is_closed value for a channel in the DB to 1.
    async fn update_channel_to_closed(
        &self,
        rpc_id: u64,
        closure_reason: String,
        close_at: u64,
    ) -> Result<(), Self::Error>;

    /// Gets the list of closed channels records in the DB with no closing tx hashs saved yet. Can be used to check if
    /// the closing tx hash needs to be fetched from the chain and saved to DB when initializing the persister.
    async fn get_closed_channels_with_no_closing_tx(&self) -> Result<Vec<SqlChannelDetails>, Self::Error>;

    /// Updates a channel's DB record with the channel's closing transaction hash.
    async fn add_closing_tx_to_db(&self, rpc_id: u64, closing_tx: String) -> Result<(), Self::Error>;

    /// Updates a channel's DB record with information about the transaction responsible for claiming the channel's
    /// closing balance back to the user's address.
    async fn add_claiming_tx_to_db(
        &self,
        closing_tx: String,
        claiming_tx: String,
        claimed_balance: f64,
    ) -> Result<(), Self::Error>;

    /// Gets a channel record from DB by the channel's rpc_id.
    async fn get_channel_from_db(&self, rpc_id: u64) -> Result<Option<SqlChannelDetails>, Self::Error>;

    /// Gets the list of closed channels that match the provided filter criteria. The number of requested records is
    /// specified by the limit parameter, the starting record to list from is specified by the paging parameter. The
    /// total number of matched records along with the number of skipped records are also returned in the result.
    async fn get_closed_channels_by_filter(
        &self,
        filter: Option<ClosedChannelsFilter>,
        paging: PagingOptionsEnum<u64>,
        limit: usize,
    ) -> Result<GetClosedChannelsResult, Self::Error>;

    /// Inserts or updates a new payment record in the DB.
    async fn add_or_update_payment_in_db(&self, info: PaymentInfo) -> Result<(), Self::Error>;

    /// Gets a payment's record from DB by the payment's hash.
    async fn get_payment_from_db(&self, hash: PaymentHash) -> Result<Option<PaymentInfo>, Self::Error>;

    /// Gets the list of payments that match the provided filter criteria. The number of requested records is specified
    /// by the limit parameter, the starting record to list from is specified by the paging parameter. The total number
    /// of matched records along with the number of skipped records are also returned in the result.
    async fn get_payments_by_filter(
        &self,
        filter: Option<PaymentsFilter>,
        paging: PagingOptionsEnum<PaymentHash>,
        limit: usize,
    ) -> Result<GetPaymentsResult, Self::Error>;
}
