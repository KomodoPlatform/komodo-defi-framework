use async_trait::async_trait;
use bitcoin::Network;
use db_common::sqlite::rusqlite::types::FromSqlError;
use derive_more::Display;
use lightning::ln::{PaymentHash, PaymentPreimage, PaymentSecret};
use lightning::routing::network_graph::NetworkGraph;
use lightning::routing::scoring::ProbabilisticScorer;
use parking_lot::Mutex as PaMutex;
use secp256k1::PublicKey;
use serde::Serialize;
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
    #[serde(skip_serializing)]
    pub funding_generated_in_block: Option<u64>,
    pub is_outbound: bool,
    pub is_public: bool,
    pub is_closed: bool,
}

impl SqlChannelDetails {
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
        }
    }
}

#[derive(Clone, Debug, Display, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HTLCStatus {
    Pending,
    Succeeded,
    Failed,
}

impl FromStr for HTLCStatus {
    type Err = FromSqlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(HTLCStatus::Pending),
            "succeeded" => Ok(HTLCStatus::Succeeded),
            "failed" => Ok(HTLCStatus::Failed),
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

#[derive(Serialize)]
pub enum PaymentType {
    #[serde(rename = "Outbound Payment")]
    OutboundPayment,
    #[serde(rename = "Inbound Payment")]
    InboundPayment,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaymentInfo {
    pub payment_hash: PaymentHash,
    pub preimage: Option<PaymentPreimage>,
    pub secret: Option<PaymentSecret>,
    pub amt_msat: Option<u64>,
    pub fee_paid_msat: Option<u64>,
    pub status: HTLCStatus,
}

#[async_trait]
pub trait SqlStorage {
    type Error;

    /// Initializes dirs/collection/tables in storage for a specified coin
    async fn init_sql(&self) -> Result<(), Self::Error>;

    async fn is_sql_initialized(&self) -> Result<bool, Self::Error>;

    async fn add_channel_to_sql(&self, details: SqlChannelDetails) -> Result<(), Self::Error>;

    async fn add_or_update_payment_in_sql(
        &self,
        info: PaymentInfo,
        payment_type: PaymentType,
    ) -> Result<(), Self::Error>;

    async fn get_channel_from_sql(&self, rpc_id: u64) -> Result<Option<SqlChannelDetails>, Self::Error>;

    async fn get_payment_from_sql(&self, hash: PaymentHash) -> Result<Option<(PaymentInfo, PaymentType)>, Self::Error>;

    async fn get_last_channel_rpc_id(&self) -> Result<u32, Self::Error>;

    async fn add_funding_tx_to_sql(
        &self,
        rpc_id: u64,
        funding_tx: String,
        funding_value: u64,
        funding_generated_in_block: u64,
    ) -> Result<(), Self::Error>;

    async fn update_channel_to_closed(&self, rpc_id: u64, closure_reason: String) -> Result<(), Self::Error>;

    async fn add_closing_tx_to_sql(&self, rpc_id: u64, closing_tx_: String) -> Result<(), Self::Error>;

    async fn get_closed_channels(&self) -> Result<Vec<SqlChannelDetails>, Self::Error>;

    async fn get_outbound_payments(&self) -> Result<Vec<(PaymentInfo, PaymentType)>, Self::Error>;

    async fn get_inbound_payments(&self) -> Result<Vec<(PaymentInfo, PaymentType)>, Self::Error>;

    async fn add_claiming_tx_to_sql(
        &self,
        closing_tx: String,
        claiming_tx: String,
        claimed_balance: f64,
    ) -> Result<(), Self::Error>;
}
