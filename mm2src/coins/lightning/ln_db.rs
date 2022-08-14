use async_trait::async_trait;
use common::{now_ms, PagingOptionsEnum};
use db_common::sqlite::rusqlite::types::FromSqlError;
use derive_more::Display;
use lightning::ln::{PaymentHash, PaymentPreimage, PaymentSecret};
use secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DBChannelDetails {
    pub rpc_id: i64,
    pub channel_id: String,
    pub counterparty_node_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funding_tx: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funding_value: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closing_tx: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closure_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claiming_tx: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_balance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funding_generated_in_block: Option<i64>,
    pub is_outbound: bool,
    pub is_public: bool,
    pub is_closed: bool,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<i64>,
}

impl DBChannelDetails {
    #[inline]
    pub fn new(
        rpc_id: u64,
        channel_id: [u8; 32],
        counterparty_node_id: PublicKey,
        is_outbound: bool,
        is_public: bool,
    ) -> Self {
        DBChannelDetails {
            rpc_id: rpc_id as i64,
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
            created_at: (now_ms() / 1000) as i64,
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
    pub from_funding_value: Option<i64>,
    pub to_funding_value: Option<i64>,
    pub closing_tx: Option<String>,
    pub closure_reason: Option<String>,
    pub claiming_tx: Option<String>,
    pub from_claimed_balance: Option<f64>,
    pub to_claimed_balance: Option<f64>,
    pub channel_type: Option<ChannelType>,
    pub channel_visibility: Option<ChannelVisibility>,
}

pub struct GetClosedChannelsResult {
    pub channels: Vec<DBChannelDetails>,
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
pub struct DBPaymentInfo {
    pub payment_hash: PaymentHash,
    pub payment_type: PaymentType,
    pub description: String,
    pub preimage: Option<PaymentPreimage>,
    pub secret: Option<PaymentSecret>,
    pub amt_msat: Option<i64>,
    pub fee_paid_msat: Option<i64>,
    pub status: HTLCStatus,
    pub created_at: i64,
    pub last_updated: i64,
}

#[derive(Clone)]
pub struct DBPaymentsFilter {
    pub is_outbound: Option<bool>,
    pub destination: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub from_amount_msat: Option<i64>,
    pub to_amount_msat: Option<i64>,
    pub from_fee_paid_msat: Option<i64>,
    pub to_fee_paid_msat: Option<i64>,
    pub from_timestamp: Option<i64>,
    pub to_timestamp: Option<i64>,
}

pub struct GetPaymentsResult {
    pub payments: Vec<DBPaymentInfo>,
    pub skipped: usize,
    pub total: usize,
}

#[async_trait]
pub trait LightningDB {
    type Error;

    /// Initializes tables in DB.
    async fn init_db(&self) -> Result<(), Self::Error>;

    /// Checks if tables have been initialized or not in DB.
    async fn is_db_initialized(&self) -> Result<bool, Self::Error>;

    /// Gets the last added channel rpc_id. Can be used to deduce the rpc_id for a new channel to be added to DB.
    async fn get_last_channel_rpc_id(&self) -> Result<u32, Self::Error>;

    /// Inserts a new channel record in the DB. The record's data is completed using add_funding_tx_to_db,
    /// add_closing_tx_to_db, add_claiming_tx_to_db when this information is available.
    async fn add_channel_to_db(&self, details: DBChannelDetails) -> Result<(), Self::Error>;

    /// Updates a channel's DB record with the channel's funding transaction information.
    async fn add_funding_tx_to_db(
        &self,
        rpc_id: i64,
        funding_tx: String,
        funding_value: i64,
        funding_generated_in_block: i64,
    ) -> Result<(), Self::Error>;

    /// Updates funding_tx_block_height value for a channel in the DB. Should be used to update the block height of
    /// the funding tx when the transaction is confirmed on-chain.
    async fn update_funding_tx_block_height(&self, funding_tx: String, block_height: i64) -> Result<(), Self::Error>;

    /// Updates the is_closed value for a channel in the DB to 1.
    async fn update_channel_to_closed(
        &self,
        rpc_id: i64,
        closure_reason: String,
        close_at: i64,
    ) -> Result<(), Self::Error>;

    /// Gets the list of closed channels records in the DB that have funding tx hashes saved with no closing
    /// tx hashes saved yet.
    /// Can be used to check if the closing tx hash needs to be fetched from the chain and saved to DB
    /// when initializing the persister.
    async fn get_closed_channels_with_no_closing_tx(&self) -> Result<Vec<DBChannelDetails>, Self::Error>;

    /// Updates a channel's DB record with the channel's closing transaction hash.
    async fn add_closing_tx_to_db(&self, rpc_id: i64, closing_tx: String) -> Result<(), Self::Error>;

    /// Updates a channel's DB record with information about the transaction responsible for claiming the channel's
    /// closing balance back to the user's address.
    async fn add_claiming_tx_to_db(
        &self,
        closing_tx: String,
        claiming_tx: String,
        claimed_balance: f64,
    ) -> Result<(), Self::Error>;

    /// Gets a channel record from DB by the channel's rpc_id.
    async fn get_channel_from_db(&self, rpc_id: u64) -> Result<Option<DBChannelDetails>, Self::Error>;

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
    async fn add_or_update_payment_in_db(&self, info: DBPaymentInfo) -> Result<(), Self::Error>;

    /// Gets a payment's record from DB by the payment's hash.
    async fn get_payment_from_db(&self, hash: PaymentHash) -> Result<Option<DBPaymentInfo>, Self::Error>;

    /// Gets the list of payments that match the provided filter criteria. The number of requested records is specified
    /// by the limit parameter, the starting record to list from is specified by the paging parameter. The total number
    /// of matched records along with the number of skipped records are also returned in the result.
    async fn get_payments_by_filter(
        &self,
        filter: Option<DBPaymentsFilter>,
        paging: PagingOptionsEnum<PaymentHash>,
        limit: usize,
    ) -> Result<GetPaymentsResult, Self::Error>;
}
