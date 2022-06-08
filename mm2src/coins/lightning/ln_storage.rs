// todo: break up this file to smaller files
use crate::lightning::ln_platform::Platform;
use crate::lightning::ln_utils::{ChainMonitor, ChannelManager};
use async_trait::async_trait;
use bitcoin::blockdata::constants::genesis_block;
use bitcoin::Network;
use common::log::LogState;
use common::{async_blocking, now_ms, PagingOptionsEnum};
use db_common::sqlite::rusqlite::types::FromSqlError;
use db_common::sqlite::rusqlite::{Error as SqlError, Row, ToSql, NO_PARAMS};
use db_common::sqlite::sql_builder::SqlBuilder;
use db_common::sqlite::{h256_option_slice_from_row, h256_slice_from_row, offset_by_id, query_single_row,
                        sql_text_conversion_err, string_from_row, validate_table_name, SqliteConnShared,
                        CHECK_TABLE_EXISTS_SQL};
use derive_more::Display;
use lightning::chain::channelmonitor::{ChannelMonitor, ChannelMonitorUpdate};
use lightning::chain::keysinterface::{InMemorySigner, KeysManager, Sign};
use lightning::chain::transaction::OutPoint;
use lightning::chain::{chainmonitor, ChannelMonitorUpdateErr};
use lightning::ln::{PaymentHash, PaymentPreimage, PaymentSecret};
use lightning::routing::network_graph::NetworkGraph;
use lightning::routing::scoring::{ProbabilisticScorer, ProbabilisticScoringParameters};
use lightning::util::ser::{Readable, ReadableArgs, Writeable};
use lightning_background_processor::Persister;
use lightning_persister::FilesystemPersister;
use mm2_io::fs::check_dir_operations;
use parking_lot::Mutex as PaMutex;
use secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryInto;
use std::fs;
use std::io::{BufReader, BufWriter};
use std::net::SocketAddr;
use std::ops::Deref;
use std::path::PathBuf;
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
    pub last_updated: u64,
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
            last_updated: now_ms() / 1000,
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
    async fn update_channel_to_closed(&self, rpc_id: u64, closure_reason: String) -> Result<(), Self::Error>;

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

fn channels_history_table(ticker: &str) -> String { ticker.to_owned() + "_channels_history" }

fn payments_history_table(ticker: &str) -> String { ticker.to_owned() + "_payments_history" }

fn create_channels_history_table_sql(for_coin: &str) -> Result<String, SqlError> {
    let table_name = channels_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (
            id INTEGER NOT NULL PRIMARY KEY,
            rpc_id INTEGER NOT NULL UNIQUE,
            channel_id VARCHAR(255) NOT NULL,
            counterparty_node_id VARCHAR(255) NOT NULL,
            funding_tx VARCHAR(255),
            funding_value INTEGER,
            funding_generated_in_block Integer,
            closing_tx VARCHAR(255),
            closure_reason TEXT,
            claiming_tx VARCHAR(255),
            claimed_balance REAL,
            is_outbound INTEGER NOT NULL,
            is_public INTEGER NOT NULL,
            is_closed INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            last_updated INTEGER NOT NULL
        );",
        table_name
    );

    Ok(sql)
}

fn create_payments_history_table_sql(for_coin: &str) -> Result<String, SqlError> {
    let table_name = payments_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (
            id INTEGER NOT NULL PRIMARY KEY,
            payment_hash VARCHAR(255) NOT NULL UNIQUE,
            destination VARCHAR(255),
            description VARCHAR(641) NOT NULL,
            preimage VARCHAR(255),
            secret VARCHAR(255),
            amount_msat INTEGER,
            fee_paid_msat INTEGER,
            is_outbound INTEGER NOT NULL,
            status VARCHAR(255) NOT NULL,
            created_at INTEGER NOT NULL,
            last_updated INTEGER NOT NULL
        );",
        table_name
    );

    Ok(sql)
}

fn insert_channel_sql(for_coin: &str) -> Result<String, SqlError> {
    let table_name = channels_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!(
        "INSERT INTO {} (
            rpc_id,
            channel_id,
            counterparty_node_id,
            is_outbound,
            is_public,
            is_closed,
            created_at,
            last_updated
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8
        );",
        table_name
    );

    Ok(sql)
}

fn upsert_payment_sql(for_coin: &str) -> Result<String, SqlError> {
    let table_name = payments_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!(
        "INSERT OR REPLACE INTO {} (
            payment_hash,
            destination,
            description,
            preimage,
            secret,
            amount_msat,
            fee_paid_msat,
            is_outbound,
            status,
            created_at,
            last_updated
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11
        );",
        table_name
    );

    Ok(sql)
}

fn select_channel_by_rpc_id_sql(for_coin: &str) -> Result<String, SqlError> {
    let table_name = channels_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!(
        "SELECT
            rpc_id,
            channel_id,
            counterparty_node_id,
            funding_tx,
            funding_value,
            funding_generated_in_block,
            closing_tx,
            closure_reason,
            claiming_tx,
            claimed_balance,
            is_outbound,
            is_public,
            is_closed,
            created_at,
            last_updated
        FROM
            {}
        WHERE
            rpc_id=?1",
        table_name
    );

    Ok(sql)
}

fn select_payment_by_hash_sql(for_coin: &str) -> Result<String, SqlError> {
    let table_name = payments_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!(
        "SELECT
            payment_hash,
            destination,
            description,
            preimage,
            secret,
            amount_msat,
            fee_paid_msat,
            status,
            is_outbound,
            created_at,
            last_updated
        FROM
            {}
        WHERE
            payment_hash=?1;",
        table_name
    );

    Ok(sql)
}

fn channel_details_from_row(row: &Row<'_>) -> Result<SqlChannelDetails, SqlError> {
    let channel_details = SqlChannelDetails {
        rpc_id: row.get::<_, u32>(0)? as u64,
        channel_id: row.get(1)?,
        counterparty_node_id: row.get(2)?,
        funding_tx: row.get(3)?,
        funding_value: row.get::<_, Option<u32>>(4)?.map(|v| v as u64),
        funding_generated_in_block: row.get::<_, Option<u32>>(5)?.map(|v| v as u64),
        closing_tx: row.get(6)?,
        closure_reason: row.get(7)?,
        claiming_tx: row.get(8)?,
        claimed_balance: row.get::<_, Option<f64>>(9)?,
        is_outbound: row.get(10)?,
        is_public: row.get(11)?,
        is_closed: row.get(12)?,
        created_at: row.get::<_, u32>(13)? as u64,
        last_updated: row.get::<_, u32>(14)? as u64,
    };
    Ok(channel_details)
}

fn payment_info_from_row(row: &Row<'_>) -> Result<PaymentInfo, SqlError> {
    let is_outbound = row.get::<_, bool>(8)?;
    let payment_type = if is_outbound {
        PaymentType::OutboundPayment {
            destination: PublicKey::from_str(&row.get::<_, String>(1)?).map_err(|e| sql_text_conversion_err(1, e))?,
        }
    } else {
        PaymentType::InboundPayment
    };

    let payment_info = PaymentInfo {
        payment_hash: PaymentHash(h256_slice_from_row::<String>(row, 0)?),
        payment_type,
        description: row.get(2)?,
        preimage: h256_option_slice_from_row::<String>(row, 3)?.map(PaymentPreimage),
        secret: h256_option_slice_from_row::<String>(row, 4)?.map(PaymentSecret),
        amt_msat: row.get::<_, Option<u32>>(5)?.map(|v| v as u64),
        fee_paid_msat: row.get::<_, Option<u32>>(6)?.map(|v| v as u64),
        status: HTLCStatus::from_str(&row.get::<_, String>(7)?)?,
        created_at: row.get::<_, u32>(9)? as u64,
        last_updated: row.get::<_, u32>(10)? as u64,
    };
    Ok(payment_info)
}

fn get_last_channel_rpc_id_sql(for_coin: &str) -> Result<String, SqlError> {
    let table_name = channels_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!("SELECT IFNULL(MAX(rpc_id), 0) FROM {};", table_name);

    Ok(sql)
}

fn update_funding_tx_sql(for_coin: &str) -> Result<String, SqlError> {
    let table_name = channels_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!(
        "UPDATE {} SET
            funding_tx = ?1,
            funding_value = ?2,
            funding_generated_in_block = ?3,
            last_updated = ?4
        WHERE
            rpc_id = ?5;",
        table_name
    );

    Ok(sql)
}

fn update_funding_tx_block_height_sql(for_coin: &str) -> Result<String, SqlError> {
    let table_name = channels_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!(
        "UPDATE {} SET funding_generated_in_block = ?1 WHERE funding_tx = ?2;",
        table_name
    );

    Ok(sql)
}

fn update_channel_to_closed_sql(for_coin: &str) -> Result<String, SqlError> {
    let table_name = channels_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!(
        "UPDATE {} SET closure_reason = ?1, is_closed = ?2, last_updated = ?3 WHERE rpc_id = ?4;",
        table_name
    );

    Ok(sql)
}

fn update_closing_tx_sql(for_coin: &str) -> Result<String, SqlError> {
    let table_name = channels_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!(
        "UPDATE {} SET closing_tx = ?1, last_updated = ?2 WHERE rpc_id = ?3;",
        table_name
    );

    Ok(sql)
}

fn get_channels_builder_preimage(for_coin: &str) -> Result<SqlBuilder, SqlError> {
    let table_name = channels_history_table(for_coin);
    validate_table_name(&table_name)?;

    let mut sql_builder = SqlBuilder::select_from(table_name);
    sql_builder.and_where("is_closed = 1");
    Ok(sql_builder)
}

fn add_fields_to_get_channels_sql_builder(sql_builder: &mut SqlBuilder) {
    sql_builder
        .field("rpc_id")
        .field("channel_id")
        .field("counterparty_node_id")
        .field("funding_tx")
        .field("funding_value")
        .field("funding_generated_in_block")
        .field("closing_tx")
        .field("closure_reason")
        .field("claiming_tx")
        .field("claimed_balance")
        .field("is_outbound")
        .field("is_public")
        .field("is_closed")
        .field("created_at")
        .field("last_updated");
}

fn finalize_get_channels_sql_builder(sql_builder: &mut SqlBuilder, offset: usize, limit: usize) {
    sql_builder.offset(offset);
    sql_builder.limit(limit);
    sql_builder.order_desc("last_updated");
}

fn apply_get_channels_filter(builder: &mut SqlBuilder, params: &mut Vec<(&str, String)>, filter: ClosedChannelsFilter) {
    if let Some(channel_id) = filter.channel_id {
        builder.and_where("channel_id = :channel_id");
        params.push((":channel_id", channel_id));
    }

    if let Some(counterparty_node_id) = filter.counterparty_node_id {
        builder.and_where("counterparty_node_id = :counterparty_node_id");
        params.push((":counterparty_node_id", counterparty_node_id));
    }

    if let Some(funding_tx) = filter.funding_tx {
        builder.and_where("funding_tx = :funding_tx");
        params.push((":funding_tx", funding_tx));
    }

    if let Some(from_funding_value) = filter.from_funding_value {
        builder.and_where("funding_value >= :from_funding_value");
        params.push((":from_funding_value", from_funding_value.to_string()));
    }

    if let Some(to_funding_value) = filter.to_funding_value {
        builder.and_where("funding_value <= :to_funding_value");
        params.push((":to_funding_value", to_funding_value.to_string()));
    }

    if let Some(closing_tx) = filter.closing_tx {
        builder.and_where("closing_tx = :closing_tx");
        params.push((":closing_tx", closing_tx));
    }

    if let Some(closure_reason) = filter.closure_reason {
        builder.and_where(format!("closure_reason LIKE '%{}%'", closure_reason));
    }

    if let Some(claiming_tx) = filter.claiming_tx {
        builder.and_where("claiming_tx = :claiming_tx");
        params.push((":claiming_tx", claiming_tx));
    }

    if let Some(from_claimed_balance) = filter.from_claimed_balance {
        builder.and_where("claimed_balance >= :from_claimed_balance");
        params.push((":from_claimed_balance", from_claimed_balance.to_string()));
    }

    if let Some(to_claimed_balance) = filter.to_claimed_balance {
        builder.and_where("claimed_balance <= :to_claimed_balance");
        params.push((":to_claimed_balance", to_claimed_balance.to_string()));
    }

    if let Some(channel_type) = filter.channel_type {
        let is_outbound = match channel_type {
            ChannelType::Outbound => true as i32,
            ChannelType::Inbound => false as i32,
        };

        builder.and_where("is_outbound = :is_outbound");
        params.push((":is_outbound", is_outbound.to_string()));
    }

    if let Some(channel_visibility) = filter.channel_visibility {
        let is_public = match channel_visibility {
            ChannelVisibility::Public => true as i32,
            ChannelVisibility::Private => false as i32,
        };

        builder.and_where("is_public = :is_public");
        params.push((":is_public", is_public.to_string()));
    }
}

fn get_payments_builder_preimage(for_coin: &str) -> Result<SqlBuilder, SqlError> {
    let table_name = payments_history_table(for_coin);
    validate_table_name(&table_name)?;

    Ok(SqlBuilder::select_from(table_name))
}

fn finalize_get_payments_sql_builder(sql_builder: &mut SqlBuilder, offset: usize, limit: usize) {
    sql_builder
        .field("payment_hash")
        .field("destination")
        .field("description")
        .field("preimage")
        .field("secret")
        .field("amount_msat")
        .field("fee_paid_msat")
        .field("status")
        .field("is_outbound")
        .field("created_at")
        .field("last_updated");
    sql_builder.offset(offset);
    sql_builder.limit(limit);
    sql_builder.order_desc("last_updated");
}

fn apply_get_payments_filter(builder: &mut SqlBuilder, params: &mut Vec<(&str, String)>, filter: PaymentsFilter) {
    if let Some(payment_type) = filter.payment_type {
        let (is_outbound, destination) = match payment_type {
            PaymentType::OutboundPayment { destination } => (true as i32, Some(destination.to_string())),
            PaymentType::InboundPayment => (false as i32, None),
        };
        if let Some(dest) = destination {
            builder.and_where("destination = :dest");
            params.push((":dest", dest));
        }

        builder.and_where("is_outbound = :is_outbound");
        params.push((":is_outbound", is_outbound.to_string()));
    }

    if let Some(description) = filter.description {
        builder.and_where(format!("description LIKE '%{}%'", description));
    }

    if let Some(status) = filter.status {
        builder.and_where("status = :status");
        params.push((":status", status.to_string()));
    }

    if let Some(from_amount) = filter.from_amount_msat {
        builder.and_where("amount_msat >= :from_amount");
        params.push((":from_amount", from_amount.to_string()));
    }

    if let Some(to_amount) = filter.to_amount_msat {
        builder.and_where("amount_msat <= :to_amount");
        params.push((":to_amount", to_amount.to_string()));
    }

    if let Some(from_fee) = filter.from_fee_paid_msat {
        builder.and_where("fee_paid_msat >= :from_fee");
        params.push((":from_fee", from_fee.to_string()));
    }

    if let Some(to_fee) = filter.to_fee_paid_msat {
        builder.and_where("fee_paid_msat <= :to_fee");
        params.push((":to_fee", to_fee.to_string()));
    }

    if let Some(from_time) = filter.from_timestamp {
        builder.and_where("created_at >= :from_time");
        params.push((":from_time", from_time.to_string()));
    }

    if let Some(to_time) = filter.to_timestamp {
        builder.and_where("created_at <= :to_time");
        params.push((":to_time", to_time.to_string()));
    }
}

fn update_claiming_tx_sql(for_coin: &str) -> Result<String, SqlError> {
    let table_name = channels_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!(
        "UPDATE {} SET claiming_tx = ?1, claimed_balance = ?2, last_updated = ?3 WHERE closing_tx = ?4;",
        table_name
    );

    Ok(sql)
}

pub struct LightningPersister {
    // todo: check again if the available trait can be used for storage_ticker
    storage_ticker: String,
    main_path: PathBuf,
    backup_path: Option<PathBuf>,
    channels_persister: FilesystemPersister,
    sqlite_connection: SqliteConnShared,
}

impl LightningPersister {
    /// Initialize a new LightningPersister and set the path to the individual channels'
    /// files.
    pub fn new(
        storage_ticker: String,
        main_path: PathBuf,
        backup_path: Option<PathBuf>,
        sqlite_connection: SqliteConnShared,
    ) -> Self {
        Self {
            storage_ticker,
            main_path: main_path.clone(),
            backup_path,
            channels_persister: FilesystemPersister::new(main_path.display().to_string()),
            sqlite_connection,
        }
    }

    /// Get the directory which was provided when this persister was initialized.
    pub fn main_path(&self) -> PathBuf { self.main_path.clone() }

    /// Get the backup directory which was provided when this persister was initialized.
    pub fn backup_path(&self) -> Option<PathBuf> { self.backup_path.clone() }

    /// Get the channels_persister which was initialized when this persister was initialized.
    pub fn channels_persister(&self) -> &FilesystemPersister { &self.channels_persister }

    pub fn monitor_backup_path(&self) -> Option<PathBuf> {
        if let Some(mut backup_path) = self.backup_path() {
            backup_path.push("monitors");
            return Some(backup_path);
        }
        None
    }

    pub fn nodes_addresses_path(&self) -> PathBuf {
        let mut path = self.main_path();
        path.push("channel_nodes_data");
        path
    }

    pub fn nodes_addresses_backup_path(&self) -> Option<PathBuf> {
        if let Some(mut backup_path) = self.backup_path() {
            backup_path.push("channel_nodes_data");
            return Some(backup_path);
        }
        None
    }

    pub fn network_graph_path(&self) -> PathBuf {
        let mut path = self.main_path();
        path.push("network_graph");
        path
    }

    pub fn scorer_path(&self) -> PathBuf {
        let mut path = self.main_path();
        path.push("scorer");
        path
    }

    pub fn manager_path(&self) -> PathBuf {
        let mut path = self.main_path();
        path.push("manager");
        path
    }
}

// todo: try to find a better way, probably breaking up LightningPersister struct to smaller structs can be a solution
pub struct LightningPersisterShared(pub Arc<LightningPersister>);

impl Clone for LightningPersisterShared {
    fn clone(&self) -> Self { LightningPersisterShared(self.0.clone()) }
}

impl Deref for LightningPersisterShared {
    type Target = LightningPersister;
    fn deref(&self) -> &LightningPersister { self.0.deref() }
}

// todo: revise this, and try to make LightningPersisterShared -> LightningPersister
impl Persister<InMemorySigner, Arc<ChainMonitor>, Arc<Platform>, Arc<KeysManager>, Arc<Platform>, Arc<LogState>>
    for LightningPersisterShared
{
    fn persist_manager(&self, channel_manager: &ChannelManager) -> Result<(), std::io::Error> {
        FilesystemPersister::persist_manager(
            // todo: revise this (might be better to make main_path a String)
            self.0.main_path().display().to_string(),
            channel_manager,
        )?;
        if let Some(backup_path) = self.0.backup_path() {
            // todo: test all backups
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(backup_path)?;
            channel_manager.write(&mut BufWriter::new(file))?;
        }
        Ok(())
    }

    fn persist_graph(&self, network_graph: &NetworkGraph) -> Result<(), std::io::Error> {
        if FilesystemPersister::persist_network_graph(self.0.main_path().display().to_string(), network_graph).is_err()
        {
            // Persistence errors here are non-fatal as we can just fetch the routing graph
            // again later, but they may indicate a disk error which could be fatal elsewhere.
            eprintln!("Warning: Failed to persist network graph, check your disk and permissions");
        }

        Ok(())
    }
}

//todo: revise this
impl<ChannelSigner: Sign> chainmonitor::Persist<ChannelSigner> for LightningPersister {
    fn persist_new_channel(
        &self,
        funding_txo: OutPoint,
        monitor: &ChannelMonitor<ChannelSigner>,
        update_id: chainmonitor::MonitorUpdateId,
    ) -> Result<(), ChannelMonitorUpdateErr> {
        self.channels_persister
            .persist_new_channel(funding_txo, monitor, update_id)?;
        if let Some(backup_path) = self.monitor_backup_path() {
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(backup_path)
                .map_err(|_| ChannelMonitorUpdateErr::TemporaryFailure)?;
            // todo: implement from for ChannelMonitorUpdateErr
            monitor
                .write(&mut BufWriter::new(file))
                .map_err(|_| ChannelMonitorUpdateErr::TemporaryFailure)?;
        }
        Ok(())
    }

    fn update_persisted_channel(
        &self,
        funding_txo: OutPoint,
        update: &Option<ChannelMonitorUpdate>,
        monitor: &ChannelMonitor<ChannelSigner>,
        update_id: chainmonitor::MonitorUpdateId,
    ) -> Result<(), ChannelMonitorUpdateErr> {
        self.channels_persister
            .update_persisted_channel(funding_txo, update, monitor, update_id)?;
        if let Some(backup_path) = self.monitor_backup_path() {
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(backup_path)
                .map_err(|_| ChannelMonitorUpdateErr::TemporaryFailure)?;
            monitor
                .write(&mut BufWriter::new(file))
                .map_err(|_| ChannelMonitorUpdateErr::TemporaryFailure)?;
        }
        Ok(())
    }
}

#[async_trait]
impl FileSystemStorage for LightningPersister {
    // todo: maybe change this to MmError
    type Error = std::io::Error;

    async fn init_fs(&self) -> Result<(), Self::Error> {
        let path = self.main_path();
        let backup_path = self.backup_path();
        async_blocking(move || {
            fs::create_dir_all(path.clone())?;
            if let Some(path) = backup_path {
                fs::create_dir_all(path.clone())?;
                check_dir_operations(&path)?;
            }
            check_dir_operations(&path)
        })
        .await
    }

    async fn is_fs_initialized(&self) -> Result<bool, Self::Error> {
        let dir_path = self.main_path();
        let backup_dir_path = self.backup_path();
        async_blocking(move || {
            if !dir_path.exists() || backup_dir_path.as_ref().map(|path| !path.exists()).unwrap_or(false) {
                Ok(false)
            } else if !dir_path.is_dir() {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("{} is not a directory", dir_path.display()),
                ))
            } else if backup_dir_path.as_ref().map(|path| !path.is_dir()).unwrap_or(false) {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Backup path is not a directory",
                ))
            } else {
                let check_backup_ops = if let Some(backup_path) = backup_dir_path {
                    check_dir_operations(&backup_path).is_ok()
                } else {
                    true
                };
                check_dir_operations(&dir_path).map(|_| check_backup_ops)
            }
        })
        .await
    }

    async fn get_nodes_addresses(&self) -> Result<NodesAddressesMap, Self::Error> {
        let path = self.nodes_addresses_path();
        if !path.exists() {
            return Ok(HashMap::new());
        }
        async_blocking(move || {
            let file = fs::File::open(path)?;
            let reader = BufReader::new(file);
            let nodes_addresses: HashMap<String, SocketAddr> =
                serde_json::from_reader(reader).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            nodes_addresses
                .iter()
                .map(|(pubkey_str, addr)| {
                    let pubkey = PublicKey::from_str(pubkey_str)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                    Ok((pubkey, *addr))
                })
                .collect()
        })
        .await
    }

    async fn save_nodes_addresses(&self, nodes_addresses: NodesAddressesMapShared) -> Result<(), Self::Error> {
        let path = self.nodes_addresses_path();
        let backup_path = self.nodes_addresses_backup_path();
        async_blocking(move || {
            let nodes_addresses: HashMap<String, SocketAddr> = nodes_addresses
                .lock()
                .iter()
                .map(|(pubkey, addr)| (pubkey.to_string(), *addr))
                .collect();

            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)?;
            serde_json::to_writer(file, &nodes_addresses)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

            if let Some(path) = backup_path {
                let file = fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(path)?;
                serde_json::to_writer(file, &nodes_addresses)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            }

            Ok(())
        })
        .await
    }

    async fn get_network_graph(&self, network: Network) -> Result<NetworkGraph, Self::Error> {
        let path = self.network_graph_path();
        if !path.exists() {
            return Ok(NetworkGraph::new(genesis_block(network).header.block_hash()));
        }
        async_blocking(move || {
            let file = fs::File::open(path)?;
            common::log::info!("Reading the saved lightning network graph from file, this can take some time!");
            NetworkGraph::read(&mut BufReader::new(file))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
        })
        .await
    }

    async fn get_scorer(&self, network_graph: Arc<NetworkGraph>) -> Result<Scorer, Self::Error> {
        let path = self.scorer_path();
        if !path.exists() {
            return Ok(Scorer::new(ProbabilisticScoringParameters::default(), network_graph));
        }
        async_blocking(move || {
            let file = fs::File::open(path)?;
            Scorer::read(
                &mut BufReader::new(file),
                (ProbabilisticScoringParameters::default(), network_graph),
            )
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
        })
        .await
    }

    async fn save_scorer(&self, scorer: Arc<Mutex<Scorer>>) -> Result<(), Self::Error> {
        let path = self.scorer_path();
        async_blocking(move || {
            let scorer = scorer.lock().unwrap();
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)?;
            scorer.write(&mut BufWriter::new(file))
        })
        .await
    }
}

#[async_trait]
impl DbStorage for LightningPersister {
    type Error = SqlError;

    async fn init_db(&self) -> Result<(), Self::Error> {
        let sqlite_connection = self.sqlite_connection.clone();
        let sql_channels_history = create_channels_history_table_sql(self.storage_ticker.as_str())?;
        let sql_payments_history = create_payments_history_table_sql(self.storage_ticker.as_str())?;
        async_blocking(move || {
            let conn = sqlite_connection.lock().unwrap();
            conn.execute(&sql_channels_history, NO_PARAMS).map(|_| ())?;
            conn.execute(&sql_payments_history, NO_PARAMS).map(|_| ())?;
            Ok(())
        })
        .await
    }

    async fn is_db_initialized(&self) -> Result<bool, Self::Error> {
        let channels_history_table = channels_history_table(self.storage_ticker.as_str());
        validate_table_name(&channels_history_table)?;
        let payments_history_table = payments_history_table(self.storage_ticker.as_str());
        validate_table_name(&payments_history_table)?;

        let sqlite_connection = self.sqlite_connection.clone();
        async_blocking(move || {
            let conn = sqlite_connection.lock().unwrap();
            let channels_history_initialized =
                query_single_row(&conn, CHECK_TABLE_EXISTS_SQL, [channels_history_table], string_from_row)?;
            let payments_history_initialized =
                query_single_row(&conn, CHECK_TABLE_EXISTS_SQL, [payments_history_table], string_from_row)?;
            Ok(channels_history_initialized.is_some() && payments_history_initialized.is_some())
        })
        .await
    }

    async fn get_last_channel_rpc_id(&self) -> Result<u32, Self::Error> {
        let sql = get_last_channel_rpc_id_sql(self.storage_ticker.as_str())?;
        let sqlite_connection = self.sqlite_connection.clone();

        async_blocking(move || {
            let conn = sqlite_connection.lock().unwrap();
            let count: u32 = conn.query_row(&sql, NO_PARAMS, |r| r.get(0))?;
            Ok(count)
        })
        .await
    }

    async fn add_channel_to_db(&self, details: SqlChannelDetails) -> Result<(), Self::Error> {
        let for_coin = self.storage_ticker.clone();
        let rpc_id = details.rpc_id.to_string();
        let channel_id = details.channel_id;
        let counterparty_node_id = details.counterparty_node_id;
        let is_outbound = (details.is_outbound as i32).to_string();
        let is_public = (details.is_public as i32).to_string();
        let is_closed = (details.is_closed as i32).to_string();
        let created_at = (details.created_at as u32).to_string();
        let last_updated = (details.last_updated as u32).to_string();

        let params = [
            rpc_id,
            channel_id,
            counterparty_node_id,
            is_outbound,
            is_public,
            is_closed,
            created_at,
            last_updated,
        ];

        let sqlite_connection = self.sqlite_connection.clone();
        async_blocking(move || {
            let mut conn = sqlite_connection.lock().unwrap();
            let sql_transaction = conn.transaction()?;
            sql_transaction.execute(&insert_channel_sql(&for_coin)?, &params)?;
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn add_funding_tx_to_db(
        &self,
        rpc_id: u64,
        funding_tx: String,
        funding_value: u64,
        funding_generated_in_block: u64,
    ) -> Result<(), Self::Error> {
        let for_coin = self.storage_ticker.clone();
        let funding_value = funding_value.to_string();
        let funding_generated_in_block = funding_generated_in_block.to_string();
        let last_updated = (now_ms() / 1000).to_string();
        let rpc_id = rpc_id.to_string();

        let params = [
            funding_tx,
            funding_value,
            funding_generated_in_block,
            last_updated,
            rpc_id,
        ];

        let sqlite_connection = self.sqlite_connection.clone();
        async_blocking(move || {
            let mut conn = sqlite_connection.lock().unwrap();
            let sql_transaction = conn.transaction()?;
            sql_transaction.execute(&update_funding_tx_sql(&for_coin)?, &params)?;
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn update_funding_tx_block_height(&self, funding_tx: String, block_height: u64) -> Result<(), Self::Error> {
        let for_coin = self.storage_ticker.clone();
        let generated_in_block = block_height as u32;

        let sqlite_connection = self.sqlite_connection.clone();
        async_blocking(move || {
            let mut conn = sqlite_connection.lock().unwrap();
            let sql_transaction = conn.transaction()?;
            let params = [&generated_in_block as &dyn ToSql, &funding_tx as &dyn ToSql];
            sql_transaction.execute(&update_funding_tx_block_height_sql(&for_coin)?, &params)?;
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn update_channel_to_closed(&self, rpc_id: u64, closure_reason: String) -> Result<(), Self::Error> {
        let for_coin = self.storage_ticker.clone();
        let is_closed = "1".to_string();
        let last_updated = (now_ms() / 1000).to_string();
        let rpc_id = rpc_id.to_string();

        let params = [closure_reason, is_closed, last_updated, rpc_id];

        let sqlite_connection = self.sqlite_connection.clone();
        async_blocking(move || {
            let mut conn = sqlite_connection.lock().unwrap();
            let sql_transaction = conn.transaction()?;
            sql_transaction.execute(&update_channel_to_closed_sql(&for_coin)?, &params)?;
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn get_closed_channels_with_no_closing_tx(&self) -> Result<Vec<SqlChannelDetails>, Self::Error> {
        let mut builder = get_channels_builder_preimage(self.storage_ticker.as_str())?;
        builder.and_where("closing_tx IS NULL");
        add_fields_to_get_channels_sql_builder(&mut builder);
        let sql = builder.sql().expect("valid sql");
        let sqlite_connection = self.sqlite_connection.clone();

        async_blocking(move || {
            let conn = sqlite_connection.lock().unwrap();

            let mut stmt = conn.prepare(&sql)?;
            let result = stmt
                .query_map_named(&[], channel_details_from_row)?
                .collect::<Result<_, _>>()?;
            Ok(result)
        })
        .await
    }

    async fn add_closing_tx_to_db(&self, rpc_id: u64, closing_tx: String) -> Result<(), Self::Error> {
        let for_coin = self.storage_ticker.clone();
        let last_updated = (now_ms() / 1000).to_string();
        let rpc_id = rpc_id.to_string();

        let params = [closing_tx, last_updated, rpc_id];

        let sqlite_connection = self.sqlite_connection.clone();
        async_blocking(move || {
            let mut conn = sqlite_connection.lock().unwrap();
            let sql_transaction = conn.transaction()?;
            sql_transaction.execute(&update_closing_tx_sql(&for_coin)?, &params)?;
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn add_claiming_tx_to_db(
        &self,
        closing_tx: String,
        claiming_tx: String,
        claimed_balance: f64,
    ) -> Result<(), Self::Error> {
        let for_coin = self.storage_ticker.clone();
        let claimed_balance = claimed_balance.to_string();
        let last_updated = (now_ms() / 1000).to_string();

        let params = [claiming_tx, claimed_balance, last_updated, closing_tx];

        let sqlite_connection = self.sqlite_connection.clone();
        async_blocking(move || {
            let mut conn = sqlite_connection.lock().unwrap();
            let sql_transaction = conn.transaction()?;
            sql_transaction.execute(&update_claiming_tx_sql(&for_coin)?, &params)?;
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn get_channel_from_db(&self, rpc_id: u64) -> Result<Option<SqlChannelDetails>, Self::Error> {
        let params = [rpc_id.to_string()];
        let sql = select_channel_by_rpc_id_sql(self.storage_ticker.as_str())?;
        let sqlite_connection = self.sqlite_connection.clone();

        async_blocking(move || {
            let conn = sqlite_connection.lock().unwrap();
            query_single_row(&conn, &sql, params, channel_details_from_row)
        })
        .await
    }

    async fn get_closed_channels_by_filter(
        &self,
        filter: Option<ClosedChannelsFilter>,
        paging: PagingOptionsEnum<u64>,
        limit: usize,
    ) -> Result<GetClosedChannelsResult, Self::Error> {
        let mut sql_builder = get_channels_builder_preimage(self.storage_ticker.as_str())?;
        let sqlite_connection = self.sqlite_connection.clone();

        async_blocking(move || {
            let conn = sqlite_connection.lock().unwrap();

            let mut total_builder = sql_builder.clone();
            total_builder.count("id");
            let total_sql = total_builder.sql().expect("valid sql");
            let total: isize = conn.query_row(&total_sql, NO_PARAMS, |row| row.get(0))?;
            let total = total.try_into().expect("count should be always above zero");

            let offset = match paging {
                PagingOptionsEnum::PageNumber(page) => (page.get() - 1) * limit,
                PagingOptionsEnum::FromId(rpc_id) => {
                    let params = [rpc_id as u32];
                    let maybe_offset = offset_by_id(
                        &conn,
                        &sql_builder,
                        params,
                        "rpc_id",
                        "last_updated DESC",
                        "rpc_id = ?1",
                    )?;
                    match maybe_offset {
                        Some(offset) => offset,
                        None => {
                            return Ok(GetClosedChannelsResult {
                                channels: vec![],
                                skipped: 0,
                                total,
                            })
                        },
                    }
                },
            };

            let mut params = vec![];
            if let Some(f) = filter {
                apply_get_channels_filter(&mut sql_builder, &mut params, f);
            }
            let params_as_trait: Vec<_> = params.iter().map(|(key, value)| (*key, value as &dyn ToSql)).collect();
            add_fields_to_get_channels_sql_builder(&mut sql_builder);
            finalize_get_channels_sql_builder(&mut sql_builder, offset, limit);

            let sql = sql_builder.sql().expect("valid sql");
            let mut stmt = conn.prepare(&sql)?;
            let channels = stmt
                .query_map_named(params_as_trait.as_slice(), channel_details_from_row)?
                .collect::<Result<_, _>>()?;
            let result = GetClosedChannelsResult {
                channels,
                skipped: offset,
                total,
            };
            Ok(result)
        })
        .await
    }

    async fn add_or_update_payment_in_db(&self, info: PaymentInfo) -> Result<(), Self::Error> {
        let for_coin = self.storage_ticker.clone();
        let payment_hash = hex::encode(info.payment_hash.0);
        let (is_outbound, destination) = match info.payment_type {
            PaymentType::OutboundPayment { destination } => (true as i32, Some(destination.to_string())),
            PaymentType::InboundPayment => (false as i32, None),
        };
        let description = info.description;
        let preimage = info.preimage.map(|p| hex::encode(p.0));
        let secret = info.secret.map(|s| hex::encode(s.0));
        let amount_msat = info.amt_msat.map(|a| a as u32);
        let fee_paid_msat = info.fee_paid_msat.map(|f| f as u32);
        let status = info.status.to_string();
        let created_at = info.created_at as u32;
        let last_updated = info.last_updated as u32;

        let sqlite_connection = self.sqlite_connection.clone();
        async_blocking(move || {
            let params = [
                &payment_hash as &dyn ToSql,
                &destination as &dyn ToSql,
                &description as &dyn ToSql,
                &preimage as &dyn ToSql,
                &secret as &dyn ToSql,
                &amount_msat as &dyn ToSql,
                &fee_paid_msat as &dyn ToSql,
                &is_outbound as &dyn ToSql,
                &status as &dyn ToSql,
                &created_at as &dyn ToSql,
                &last_updated as &dyn ToSql,
            ];
            let mut conn = sqlite_connection.lock().unwrap();
            let sql_transaction = conn.transaction()?;
            sql_transaction.execute(&upsert_payment_sql(&for_coin)?, &params)?;
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn get_payment_from_db(&self, hash: PaymentHash) -> Result<Option<PaymentInfo>, Self::Error> {
        let params = [hex::encode(hash.0)];
        let sql = select_payment_by_hash_sql(self.storage_ticker.as_str())?;
        let sqlite_connection = self.sqlite_connection.clone();

        async_blocking(move || {
            let conn = sqlite_connection.lock().unwrap();
            query_single_row(&conn, &sql, params, payment_info_from_row)
        })
        .await
    }

    async fn get_payments_by_filter(
        &self,
        filter: Option<PaymentsFilter>,
        paging: PagingOptionsEnum<PaymentHash>,
        limit: usize,
    ) -> Result<GetPaymentsResult, Self::Error> {
        let mut sql_builder = get_payments_builder_preimage(self.storage_ticker.as_str())?;
        let sqlite_connection = self.sqlite_connection.clone();

        async_blocking(move || {
            let conn = sqlite_connection.lock().unwrap();

            let mut total_builder = sql_builder.clone();
            total_builder.count("id");
            let total_sql = total_builder.sql().expect("valid sql");
            let total: isize = conn.query_row(&total_sql, NO_PARAMS, |row| row.get(0))?;
            let total = total.try_into().expect("count should be always above zero");

            let offset = match paging {
                PagingOptionsEnum::PageNumber(page) => (page.get() - 1) * limit,
                PagingOptionsEnum::FromId(hash) => {
                    let hash_str = hex::encode(hash.0);
                    let params = [&hash_str];
                    let maybe_offset = offset_by_id(
                        &conn,
                        &sql_builder,
                        params,
                        "payment_hash",
                        "last_updated DESC",
                        "payment_hash = ?1",
                    )?;
                    match maybe_offset {
                        Some(offset) => offset,
                        None => {
                            return Ok(GetPaymentsResult {
                                payments: vec![],
                                skipped: 0,
                                total,
                            })
                        },
                    }
                },
            };

            let mut params = vec![];
            if let Some(f) = filter {
                apply_get_payments_filter(&mut sql_builder, &mut params, f);
            }
            let params_as_trait: Vec<_> = params.iter().map(|(key, value)| (*key, value as &dyn ToSql)).collect();
            finalize_get_payments_sql_builder(&mut sql_builder, offset, limit);

            let sql = sql_builder.sql().expect("valid sql");
            let mut stmt = conn.prepare(&sql)?;
            let payments = stmt
                .query_map_named(params_as_trait.as_slice(), payment_info_from_row)?
                .collect::<Result<_, _>>()?;
            let result = GetPaymentsResult {
                payments,
                skipped: offset,
                total,
            };
            Ok(result)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{block_on, now_ms};
    use db_common::sqlite::rusqlite::Connection;
    use rand::distributions::Alphanumeric;
    use rand::{Rng, RngCore};
    use secp256k1::{Secp256k1, SecretKey};
    use std::fs;
    use std::num::NonZeroUsize;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    impl Drop for LightningPersister {
        fn drop(&mut self) {
            // We test for invalid directory names, so it's OK if directory removal
            // fails.
            match fs::remove_dir_all(&self.main_path) {
                Err(e) => println!("Failed to remove test persister directory: {}", e),
                _ => {},
            }
        }
    }

    fn generate_random_channels(num: u64) -> Vec<SqlChannelDetails> {
        let mut rng = rand::thread_rng();
        let mut channels = vec![];
        let s = Secp256k1::new();
        let mut bytes = [0; 32];
        for i in 0..num {
            let details = SqlChannelDetails {
                rpc_id: i + 1,
                channel_id: {
                    rng.fill_bytes(&mut bytes);
                    hex::encode(bytes)
                },
                counterparty_node_id: {
                    rng.fill_bytes(&mut bytes);
                    let secret = SecretKey::from_slice(&bytes).unwrap();
                    let pubkey = PublicKey::from_secret_key(&s, &secret);
                    pubkey.to_string()
                },
                funding_tx: {
                    rng.fill_bytes(&mut bytes);
                    Some(hex::encode(bytes))
                },
                funding_value: Some(rng.gen::<u32>() as u64),
                closing_tx: {
                    rng.fill_bytes(&mut bytes);
                    Some(hex::encode(bytes))
                },
                closure_reason: {
                    Some(
                        rng.sample_iter(&Alphanumeric)
                            .take(30)
                            .map(char::from)
                            .collect::<String>(),
                    )
                },
                claiming_tx: {
                    rng.fill_bytes(&mut bytes);
                    Some(hex::encode(bytes))
                },
                claimed_balance: Some(rng.gen::<f64>()),
                funding_generated_in_block: Some(rng.gen::<u32>() as u64),
                is_outbound: rand::random(),
                is_public: rand::random(),
                is_closed: rand::random(),
                created_at: rng.gen::<u32>() as u64,
                last_updated: rng.gen::<u32>() as u64,
            };
            channels.push(details);
        }
        channels
    }

    fn generate_random_payments(num: u64) -> Vec<PaymentInfo> {
        let mut rng = rand::thread_rng();
        let mut payments = vec![];
        let s = Secp256k1::new();
        let mut bytes = [0; 32];
        for _ in 0..num {
            let payment_type = if let 0 = rng.gen::<u8>() % 2 {
                PaymentType::InboundPayment
            } else {
                rng.fill_bytes(&mut bytes);
                let secret = SecretKey::from_slice(&bytes).unwrap();
                PaymentType::OutboundPayment {
                    destination: PublicKey::from_secret_key(&s, &secret),
                }
            };
            let status_rng: u8 = rng.gen();
            let status = if status_rng % 3 == 0 {
                HTLCStatus::Succeeded
            } else if status_rng % 3 == 1 {
                HTLCStatus::Pending
            } else {
                HTLCStatus::Failed
            };
            let description: String = rng.sample_iter(&Alphanumeric).take(30).map(char::from).collect();
            let info = PaymentInfo {
                payment_hash: {
                    rng.fill_bytes(&mut bytes);
                    PaymentHash(bytes)
                },
                payment_type,
                description,
                preimage: {
                    rng.fill_bytes(&mut bytes);
                    Some(PaymentPreimage(bytes))
                },
                secret: {
                    rng.fill_bytes(&mut bytes);
                    Some(PaymentSecret(bytes))
                },
                amt_msat: Some(rng.gen::<u32>() as u64),
                fee_paid_msat: Some(rng.gen::<u32>() as u64),
                status,
                created_at: rng.gen::<u32>() as u64,
                last_updated: rng.gen::<u32>() as u64,
            };
            payments.push(info);
        }
        payments
    }

    #[test]
    fn test_init_sql_collection() {
        let persister = LightningPersister::new(
            "init_sql_collection".into(),
            PathBuf::from("test_filesystem_persister"),
            None,
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        );
        let initialized = block_on(persister.is_db_initialized()).unwrap();
        assert!(!initialized);

        block_on(persister.init_db()).unwrap();
        // repetitive init must not fail
        block_on(persister.init_db()).unwrap();

        let initialized = block_on(persister.is_db_initialized()).unwrap();
        assert!(initialized);
    }

    #[test]
    fn test_add_get_channel_sql() {
        let persister = LightningPersister::new(
            "add_get_channel".into(),
            PathBuf::from("test_filesystem_persister"),
            None,
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        );

        block_on(persister.init_db()).unwrap();

        let last_channel_rpc_id = block_on(persister.get_last_channel_rpc_id()).unwrap();
        assert_eq!(last_channel_rpc_id, 0);

        let channel = block_on(persister.get_channel_from_db(1)).unwrap();
        assert!(channel.is_none());

        let mut expected_channel_details = SqlChannelDetails::new(
            1,
            [0; 32],
            PublicKey::from_str("038863cf8ab91046230f561cd5b386cbff8309fa02e3f0c3ed161a3aeb64a643b9").unwrap(),
            true,
            true,
        );
        block_on(persister.add_channel_to_db(expected_channel_details.clone())).unwrap();
        let last_channel_rpc_id = block_on(persister.get_last_channel_rpc_id()).unwrap();
        assert_eq!(last_channel_rpc_id, 1);

        let actual_channel_details = block_on(persister.get_channel_from_db(1)).unwrap().unwrap();
        assert_eq!(expected_channel_details, actual_channel_details);

        // must fail because we are adding channel with the same rpc_id
        block_on(persister.add_channel_to_db(expected_channel_details.clone())).unwrap_err();
        assert_eq!(last_channel_rpc_id, 1);

        expected_channel_details.rpc_id = 2;
        block_on(persister.add_channel_to_db(expected_channel_details.clone())).unwrap();
        let last_channel_rpc_id = block_on(persister.get_last_channel_rpc_id()).unwrap();
        assert_eq!(last_channel_rpc_id, 2);

        block_on(persister.add_funding_tx_to_db(
            2,
            "9cdafd6d42dcbdc06b0b5bce1866deb82630581285bbfb56870577300c0a8c6e".into(),
            3000,
            50000,
        ))
        .unwrap();
        expected_channel_details.funding_tx =
            Some("9cdafd6d42dcbdc06b0b5bce1866deb82630581285bbfb56870577300c0a8c6e".into());
        expected_channel_details.funding_value = Some(3000);
        expected_channel_details.funding_generated_in_block = Some(50000);

        let actual_channel_details = block_on(persister.get_channel_from_db(2)).unwrap().unwrap();
        assert_eq!(expected_channel_details, actual_channel_details);

        block_on(persister.update_funding_tx_block_height(
            "9cdafd6d42dcbdc06b0b5bce1866deb82630581285bbfb56870577300c0a8c6e".into(),
            50001,
        ))
        .unwrap();
        expected_channel_details.funding_generated_in_block = Some(50001);

        let actual_channel_details = block_on(persister.get_channel_from_db(2)).unwrap().unwrap();
        assert_eq!(expected_channel_details, actual_channel_details);

        block_on(persister.update_channel_to_closed(2, "the channel was cooperatively closed".into())).unwrap();
        expected_channel_details.closure_reason = Some("the channel was cooperatively closed".into());
        expected_channel_details.is_closed = true;

        let actual_channel_details = block_on(persister.get_channel_from_db(2)).unwrap().unwrap();
        assert_eq!(expected_channel_details, actual_channel_details);

        let actual_channels = block_on(persister.get_closed_channels_with_no_closing_tx()).unwrap();
        assert_eq!(actual_channels.len(), 1);

        let closed_channels =
            block_on(persister.get_closed_channels_by_filter(None, PagingOptionsEnum::default(), 10)).unwrap();
        assert_eq!(closed_channels.channels.len(), 1);
        assert_eq!(expected_channel_details, closed_channels.channels[0]);

        block_on(persister.update_channel_to_closed(1, "the channel was cooperatively closed".into())).unwrap();
        let closed_channels =
            block_on(persister.get_closed_channels_by_filter(None, PagingOptionsEnum::default(), 10)).unwrap();
        assert_eq!(closed_channels.channels.len(), 2);

        let actual_channels = block_on(persister.get_closed_channels_with_no_closing_tx()).unwrap();
        assert_eq!(actual_channels.len(), 2);

        block_on(persister.add_closing_tx_to_db(
            2,
            "5557df9ad2c9b3c57a4df8b4a7da0b7a6f4e923b4a01daa98bf9e5a3b33e9c8f".into(),
        ))
        .unwrap();
        expected_channel_details.closing_tx =
            Some("5557df9ad2c9b3c57a4df8b4a7da0b7a6f4e923b4a01daa98bf9e5a3b33e9c8f".into());

        let actual_channels = block_on(persister.get_closed_channels_with_no_closing_tx()).unwrap();
        assert_eq!(actual_channels.len(), 1);

        let actual_channel_details = block_on(persister.get_channel_from_db(2)).unwrap().unwrap();
        assert_eq!(expected_channel_details, actual_channel_details);

        block_on(persister.add_claiming_tx_to_db(
            "5557df9ad2c9b3c57a4df8b4a7da0b7a6f4e923b4a01daa98bf9e5a3b33e9c8f".into(),
            "97f061634a4a7b0b0c2b95648f86b1c39b95e0cf5073f07725b7143c095b612a".into(),
            2000.333333,
        ))
        .unwrap();
        expected_channel_details.claiming_tx =
            Some("97f061634a4a7b0b0c2b95648f86b1c39b95e0cf5073f07725b7143c095b612a".into());
        expected_channel_details.claimed_balance = Some(2000.333333);

        let actual_channel_details = block_on(persister.get_channel_from_db(2)).unwrap().unwrap();
        assert_eq!(expected_channel_details, actual_channel_details);
    }

    #[test]
    fn test_add_get_payment_sql() {
        let persister = LightningPersister::new(
            "add_get_payment".into(),
            PathBuf::from("test_filesystem_persister"),
            None,
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        );

        block_on(persister.init_db()).unwrap();

        let payment = block_on(persister.get_payment_from_db(PaymentHash([0; 32]))).unwrap();
        assert!(payment.is_none());

        let mut expected_payment_info = PaymentInfo {
            payment_hash: PaymentHash([0; 32]),
            payment_type: PaymentType::InboundPayment,
            description: "test payment".into(),
            preimage: Some(PaymentPreimage([2; 32])),
            secret: Some(PaymentSecret([3; 32])),
            amt_msat: Some(2000),
            fee_paid_msat: Some(100),
            status: HTLCStatus::Failed,
            created_at: now_ms() / 1000,
            last_updated: now_ms() / 1000,
        };
        block_on(persister.add_or_update_payment_in_db(expected_payment_info.clone())).unwrap();

        let actual_payment_info = block_on(persister.get_payment_from_db(PaymentHash([0; 32])))
            .unwrap()
            .unwrap();
        assert_eq!(expected_payment_info, actual_payment_info);

        expected_payment_info.payment_hash = PaymentHash([1; 32]);
        expected_payment_info.payment_type = PaymentType::OutboundPayment {
            destination: PublicKey::from_str("038863cf8ab91046230f561cd5b386cbff8309fa02e3f0c3ed161a3aeb64a643b9")
                .unwrap(),
        };
        expected_payment_info.secret = None;
        expected_payment_info.amt_msat = None;
        expected_payment_info.status = HTLCStatus::Succeeded;
        expected_payment_info.last_updated = now_ms() / 1000;
        block_on(persister.add_or_update_payment_in_db(expected_payment_info.clone())).unwrap();

        let actual_payment_info = block_on(persister.get_payment_from_db(PaymentHash([1; 32])))
            .unwrap()
            .unwrap();
        assert_eq!(expected_payment_info, actual_payment_info);
    }

    #[test]
    fn test_get_payments_by_filter() {
        let persister = LightningPersister::new(
            "test_get_payments_by_filter".into(),
            PathBuf::from("test_filesystem_persister"),
            None,
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        );

        block_on(persister.init_db()).unwrap();

        let mut payments = generate_random_payments(100);

        for payment in payments.clone() {
            block_on(persister.add_or_update_payment_in_db(payment)).unwrap();
        }

        let paging = PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap());
        let limit = 4;

        let result = block_on(persister.get_payments_by_filter(None, paging, limit)).unwrap();

        payments.sort_by(|a, b| b.last_updated.cmp(&a.last_updated));
        let expected_payments = &payments[..4].to_vec();
        let actual_payments = &result.payments;

        assert_eq!(0, result.skipped);
        assert_eq!(100, result.total);
        assert_eq!(expected_payments, actual_payments);

        let paging = PagingOptionsEnum::PageNumber(NonZeroUsize::new(2).unwrap());
        let limit = 5;

        let result = block_on(persister.get_payments_by_filter(None, paging, limit)).unwrap();

        let expected_payments = &payments[5..10].to_vec();
        let actual_payments = &result.payments;

        assert_eq!(5, result.skipped);
        assert_eq!(100, result.total);
        assert_eq!(expected_payments, actual_payments);

        let from_payment_hash = payments[20].payment_hash;
        let paging = PagingOptionsEnum::FromId(from_payment_hash);
        let limit = 3;

        let result = block_on(persister.get_payments_by_filter(None, paging, limit)).unwrap();

        let expected_payments = &payments[21..24].to_vec();
        let actual_payments = &result.payments;

        assert_eq!(expected_payments, actual_payments);

        let mut filter = PaymentsFilter {
            payment_type: Some(PaymentType::InboundPayment),
            description: None,
            status: None,
            from_amount_msat: None,
            to_amount_msat: None,
            from_fee_paid_msat: None,
            to_fee_paid_msat: None,
            from_timestamp: None,
            to_timestamp: None,
        };
        let paging = PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap());
        let limit = 10;

        let result = block_on(persister.get_payments_by_filter(Some(filter.clone()), paging.clone(), limit)).unwrap();
        let expected_payments_vec: Vec<PaymentInfo> = payments
            .iter()
            .map(|p| p.clone())
            .filter(|p| p.payment_type == PaymentType::InboundPayment)
            .collect();
        let expected_payments = if expected_payments_vec.len() > 10 {
            expected_payments_vec[..10].to_vec()
        } else {
            expected_payments_vec.clone()
        };
        let actual_payments = result.payments;

        assert_eq!(expected_payments, actual_payments);

        filter.status = Some(HTLCStatus::Succeeded);
        let result = block_on(persister.get_payments_by_filter(Some(filter.clone()), paging.clone(), limit)).unwrap();
        let expected_payments_vec: Vec<PaymentInfo> = expected_payments_vec
            .iter()
            .map(|p| p.clone())
            .filter(|p| p.status == HTLCStatus::Succeeded)
            .collect();
        let expected_payments = if expected_payments_vec.len() > 10 {
            expected_payments_vec[..10].to_vec()
        } else {
            expected_payments_vec
        };
        let actual_payments = result.payments;

        assert_eq!(expected_payments, actual_payments);

        let description = &payments[42].description;
        let substr = &description[5..10];
        filter.payment_type = None;
        filter.status = None;
        filter.description = Some(substr.to_string());
        let result = block_on(persister.get_payments_by_filter(Some(filter), paging, limit)).unwrap();
        let expected_payments_vec: Vec<PaymentInfo> = payments
            .iter()
            .map(|p| p.clone())
            .filter(|p| p.description.contains(&substr))
            .collect();
        let expected_payments = if expected_payments_vec.len() > 10 {
            expected_payments_vec[..10].to_vec()
        } else {
            expected_payments_vec.clone()
        };
        let actual_payments = result.payments;

        assert_eq!(expected_payments, actual_payments);
    }

    #[test]
    fn test_get_channels_by_filter() {
        let persister = LightningPersister::new(
            "test_get_channels_by_filter".into(),
            PathBuf::from("test_filesystem_persister"),
            None,
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        );

        block_on(persister.init_db()).unwrap();

        let mut channels = generate_random_channels(100);

        for channel in channels.clone() {
            block_on(persister.add_channel_to_db(channel.clone())).unwrap();
            block_on(persister.add_funding_tx_to_db(
                channel.rpc_id,
                channel.funding_tx.unwrap(),
                channel.funding_value.unwrap(),
                channel.funding_generated_in_block.unwrap(),
            ))
            .unwrap();
            block_on(persister.update_channel_to_closed(channel.rpc_id, channel.closure_reason.unwrap())).unwrap();
            block_on(persister.add_closing_tx_to_db(channel.rpc_id, channel.closing_tx.clone().unwrap())).unwrap();
            block_on(persister.add_claiming_tx_to_db(
                channel.closing_tx.unwrap(),
                channel.claiming_tx.unwrap(),
                channel.claimed_balance.unwrap(),
            ))
            .unwrap();
        }

        // get all channels from SQL since updated_at changed from channels generated by generate_random_channels
        channels = block_on(persister.get_closed_channels_by_filter(None, PagingOptionsEnum::default(), 100))
            .unwrap()
            .channels;
        assert_eq!(100, channels.len());

        let paging = PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap());
        let limit = 4;

        let result = block_on(persister.get_closed_channels_by_filter(None, paging, limit)).unwrap();

        let expected_channels = &channels[..4].to_vec();
        let actual_channels = &result.channels;

        assert_eq!(0, result.skipped);
        assert_eq!(100, result.total);
        assert_eq!(expected_channels, actual_channels);

        let paging = PagingOptionsEnum::PageNumber(NonZeroUsize::new(2).unwrap());
        let limit = 5;

        let result = block_on(persister.get_closed_channels_by_filter(None, paging, limit)).unwrap();

        let expected_channels = &channels[5..10].to_vec();
        let actual_channels = &result.channels;

        assert_eq!(5, result.skipped);
        assert_eq!(100, result.total);
        assert_eq!(expected_channels, actual_channels);

        let from_rpc_id = 20;
        let paging = PagingOptionsEnum::FromId(from_rpc_id);
        let limit = 3;

        let result = block_on(persister.get_closed_channels_by_filter(None, paging, limit)).unwrap();

        channels.sort_by(|a, b| a.rpc_id.cmp(&b.rpc_id));

        let expected_channels = &channels[20..23].to_vec();
        let actual_channels = &result.channels;

        assert_eq!(expected_channels, actual_channels);

        channels.sort_by(|a, b| b.last_updated.cmp(&a.last_updated));
        let mut filter = ClosedChannelsFilter {
            channel_id: None,
            counterparty_node_id: None,
            funding_tx: None,
            from_funding_value: None,
            to_funding_value: None,
            closing_tx: None,
            closure_reason: None,
            claiming_tx: None,
            from_claimed_balance: None,
            to_claimed_balance: None,
            channel_type: Some(ChannelType::Outbound),
            channel_visibility: None,
        };
        let paging = PagingOptionsEnum::PageNumber(NonZeroUsize::new(1).unwrap());
        let limit = 10;

        let result =
            block_on(persister.get_closed_channels_by_filter(Some(filter.clone()), paging.clone(), limit)).unwrap();
        let expected_channels_vec: Vec<SqlChannelDetails> = channels
            .iter()
            .map(|chan| chan.clone())
            .filter(|chan| chan.is_outbound)
            .collect();
        let expected_channels = if expected_channels_vec.len() > 10 {
            expected_channels_vec[..10].to_vec()
        } else {
            expected_channels_vec.clone()
        };
        let actual_channels = result.channels;

        assert_eq!(expected_channels, actual_channels);

        filter.channel_visibility = Some(ChannelVisibility::Public);
        let result =
            block_on(persister.get_closed_channels_by_filter(Some(filter.clone()), paging.clone(), limit)).unwrap();
        let expected_channels_vec: Vec<SqlChannelDetails> = expected_channels_vec
            .iter()
            .map(|chan| chan.clone())
            .filter(|chan| chan.is_public)
            .collect();
        let expected_channels = if expected_channels_vec.len() > 10 {
            expected_channels_vec[..10].to_vec()
        } else {
            expected_channels_vec
        };
        let actual_channels = result.channels;

        assert_eq!(expected_channels, actual_channels);

        let channel_id = channels[42].channel_id.clone();
        filter.channel_type = None;
        filter.channel_visibility = None;
        filter.channel_id = Some(channel_id.clone());
        let result = block_on(persister.get_closed_channels_by_filter(Some(filter), paging, limit)).unwrap();
        let expected_channels_vec: Vec<SqlChannelDetails> = channels
            .iter()
            .map(|chan| chan.clone())
            .filter(|chan| chan.channel_id == channel_id)
            .collect();
        let expected_channels = if expected_channels_vec.len() > 10 {
            expected_channels_vec[..10].to_vec()
        } else {
            expected_channels_vec.clone()
        };
        let actual_channels = result.channels;

        assert_eq!(expected_channels, actual_channels);
    }
}
