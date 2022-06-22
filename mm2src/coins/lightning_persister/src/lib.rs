//! Utilities that handle persisting Rust-Lightning data to disk via standard filesystem APIs.

#![feature(io_error_more)]

pub mod storage;
mod util;

extern crate async_trait;
extern crate bitcoin;
extern crate common;
extern crate libc;
extern crate lightning;
extern crate secp256k1;
extern crate serde_json;

use crate::storage::{ChannelType, ChannelVisibility, ClosedChannelsFilter, DbStorage, FileSystemStorage,
                     GetClosedChannelsResult, GetPaymentsResult, HTLCStatus, NodesAddressesMap,
                     NodesAddressesMapShared, PaymentInfo, PaymentType, PaymentsFilter, Scorer, SqlChannelDetails};
use crate::util::DiskWriteable;
use async_trait::async_trait;
use bitcoin::blockdata::constants::genesis_block;
use bitcoin::hash_types::{BlockHash, Txid};
use bitcoin::hashes::hex::{FromHex, ToHex};
use bitcoin::Network;
use common::{async_blocking, PagingOptionsEnum};
use db_common::sqlite::rusqlite::{Error as SqlError, Row, ToSql, NO_PARAMS};
use db_common::sqlite::sql_builder::SqlBuilder;
use db_common::sqlite::{h256_option_slice_from_row, h256_slice_from_row, offset_by_id, query_single_row,
                        sql_text_conversion_err, string_from_row, validate_table_name, SqliteConnShared,
                        CHECK_TABLE_EXISTS_SQL};
use lightning::chain;
use lightning::chain::chaininterface::{BroadcasterInterface, FeeEstimator};
use lightning::chain::chainmonitor;
use lightning::chain::channelmonitor::{ChannelMonitor, ChannelMonitorUpdate};
use lightning::chain::keysinterface::{KeysInterface, Sign};
use lightning::chain::transaction::OutPoint;
use lightning::ln::channelmanager::ChannelManager;
use lightning::ln::{PaymentHash, PaymentPreimage, PaymentSecret};
use lightning::routing::network_graph::NetworkGraph;
use lightning::routing::scoring::ProbabilisticScoringParameters;
use lightning::util::logger::Logger;
use lightning::util::ser::{Readable, ReadableArgs, Writeable};
use mm2_io::fs::check_dir_operations;
use secp256k1::PublicKey;
use std::collections::HashMap;
use std::convert::TryInto;
use std::fs;
use std::io::{BufReader, BufWriter, Cursor, Error};
use std::net::SocketAddr;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

/// LightningPersister persists channel data on disk, where each channel's
/// data is stored in a file named after its funding outpoint.
/// It is also used to persist payments and channels history to sqlite database.
///
/// Warning: this module does the best it can with calls to persist data, but it
/// can only guarantee that the data is passed to the drive. It is up to the
/// drive manufacturers to do the actual persistence properly, which they often
/// don't (especially on consumer-grade hardware). Therefore, it is up to the
/// user to validate their entire storage stack, to ensure the writes are
/// persistent.
/// Corollary: especially when dealing with larger amounts of money, it is best
/// practice to have multiple channel data backups and not rely only on one
/// LightningPersister.

pub struct LightningPersister {
    storage_ticker: String,
    main_path: PathBuf,
    backup_path: Option<PathBuf>,
    sqlite_connection: SqliteConnShared,
}

impl<Signer: Sign> DiskWriteable for ChannelMonitor<Signer> {
    fn write_to_file(&self, writer: &mut fs::File) -> Result<(), Error> { self.write(writer) }
}

impl<Signer: Sign, M: Deref, T: Deref, K: Deref, F: Deref, L: Deref> DiskWriteable
    for ChannelManager<Signer, M, T, K, F, L>
where
    M::Target: chain::Watch<Signer>,
    T::Target: BroadcasterInterface,
    K::Target: KeysInterface<Signer = Signer>,
    F::Target: FeeEstimator,
    L::Target: Logger,
{
    fn write_to_file(&self, writer: &mut fs::File) -> Result<(), std::io::Error> { self.write(writer) }
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
            closed_at INTEGER
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
            created_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7
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
            closed_at
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
        closed_at: row.get::<_, Option<u32>>(14)?.map(|t| t as u64),
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
            funding_generated_in_block = ?3
        WHERE
            rpc_id = ?4;",
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
        "UPDATE {} SET closure_reason = ?1, is_closed = ?2, closed_at = ?3 WHERE rpc_id = ?4;",
        table_name
    );

    Ok(sql)
}

fn update_closing_tx_sql(for_coin: &str) -> Result<String, SqlError> {
    let table_name = channels_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!("UPDATE {} SET closing_tx = ?1 WHERE rpc_id = ?2;", table_name);

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
        .field("closed_at");
}

fn finalize_get_channels_sql_builder(sql_builder: &mut SqlBuilder, offset: usize, limit: usize) {
    sql_builder.offset(offset);
    sql_builder.limit(limit);
    sql_builder.order_desc("closed_at");
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
        "UPDATE {} SET claiming_tx = ?1, claimed_balance = ?2 WHERE closing_tx = ?3;",
        table_name
    );

    Ok(sql)
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
            main_path,
            backup_path,
            sqlite_connection,
        }
    }

    /// Get the directory which was provided when this persister was initialized.
    pub fn main_path(&self) -> PathBuf { self.main_path.clone() }

    /// Get the backup directory which was provided when this persister was initialized.
    pub fn backup_path(&self) -> Option<PathBuf> { self.backup_path.clone() }

    pub(crate) fn monitor_path(&self) -> PathBuf {
        let mut path = self.main_path();
        path.push("monitors");
        path
    }

    pub(crate) fn monitor_backup_path(&self) -> Option<PathBuf> {
        if let Some(mut backup_path) = self.backup_path() {
            backup_path.push("monitors");
            return Some(backup_path);
        }
        None
    }

    pub(crate) fn nodes_addresses_path(&self) -> PathBuf {
        let mut path = self.main_path();
        path.push("channel_nodes_data");
        path
    }

    pub(crate) fn nodes_addresses_backup_path(&self) -> Option<PathBuf> {
        if let Some(mut backup_path) = self.backup_path() {
            backup_path.push("channel_nodes_data");
            return Some(backup_path);
        }
        None
    }

    pub(crate) fn network_graph_path(&self) -> PathBuf {
        let mut path = self.main_path();
        path.push("network_graph");
        path
    }

    pub(crate) fn scorer_path(&self) -> PathBuf {
        let mut path = self.main_path();
        path.push("scorer");
        path
    }

    pub fn manager_path(&self) -> PathBuf {
        let mut path = self.main_path();
        path.push("manager");
        path
    }

    /// Writes the provided `ChannelManager` to the path provided at `LightningPersister`
    /// initialization, within a file called "manager".
    pub fn persist_manager<Signer: Sign, M: Deref, T: Deref, K: Deref, F: Deref, L: Deref>(
        &self,
        manager: &ChannelManager<Signer, M, T, K, F, L>,
    ) -> Result<(), std::io::Error>
    where
        M::Target: chain::Watch<Signer>,
        T::Target: BroadcasterInterface,
        K::Target: KeysInterface<Signer = Signer>,
        F::Target: FeeEstimator,
        L::Target: Logger,
    {
        let path = self.main_path();
        util::write_to_file(path, "manager".to_string(), manager)?;
        if let Some(backup_path) = self.backup_path() {
            util::write_to_file(backup_path, "manager".to_string(), manager)?;
        }
        Ok(())
    }

    /// Read `ChannelMonitor`s from disk.
    pub fn read_channelmonitors<Signer: Sign, K: Deref>(
        &self,
        keys_manager: K,
    ) -> Result<Vec<(BlockHash, ChannelMonitor<Signer>)>, std::io::Error>
    where
        K::Target: KeysInterface<Signer = Signer> + Sized,
    {
        let path = self.monitor_path();
        if !Path::new(&path).exists() {
            return Ok(Vec::new());
        }
        let mut res = Vec::new();
        for file_option in fs::read_dir(path).unwrap() {
            let file = file_option.unwrap();
            let owned_file_name = file.file_name();
            let filename = owned_file_name.to_str();
            if filename.is_none() || !filename.unwrap().is_ascii() || filename.unwrap().len() < 65 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid ChannelMonitor file name",
                ));
            }
            if filename.unwrap().ends_with(".tmp") {
                // If we were in the middle of committing an new update and crashed, it should be
                // safe to ignore the update - we should never have returned to the caller and
                // irrevocably committed to the new state in any way.
                continue;
            }

            let txid = Txid::from_hex(filename.unwrap().split_at(64).0);
            if txid.is_err() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid tx ID in filename",
                ));
            }

            let index = filename.unwrap().split_at(65).1.parse::<u16>();
            if index.is_err() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid tx index in filename",
                ));
            }

            let contents = fs::read(&file.path())?;
            let mut buffer = Cursor::new(&contents);
            match <(BlockHash, ChannelMonitor<Signer>)>::read(&mut buffer, &*keys_manager) {
                Ok((blockhash, channel_monitor)) => {
                    if channel_monitor.get_funding_txo().0.txid != txid.unwrap()
                        || channel_monitor.get_funding_txo().0.index != index.unwrap()
                    {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "ChannelMonitor was stored in the wrong file",
                        ));
                    }
                    res.push((blockhash, channel_monitor));
                },
                Err(e) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Failed to deserialize ChannelMonitor: {}", e),
                    ))
                },
            }
        }
        Ok(res)
    }
}

impl<ChannelSigner: Sign> chainmonitor::Persist<ChannelSigner> for LightningPersister {
    // TODO: We really need a way for the persister to inform the user that its time to crash/shut
    // down once these start returning failure.
    // A PermanentFailure implies we need to shut down since we're force-closing channels without
    // even broadcasting!

    fn persist_new_channel(
        &self,
        funding_txo: OutPoint,
        monitor: &ChannelMonitor<ChannelSigner>,
        _update_id: chainmonitor::MonitorUpdateId,
    ) -> Result<(), chain::ChannelMonitorUpdateErr> {
        let filename = format!("{}_{}", funding_txo.txid.to_hex(), funding_txo.index);
        util::write_to_file(self.monitor_path(), filename.clone(), monitor)
            .map_err(|_| chain::ChannelMonitorUpdateErr::PermanentFailure)?;
        if let Some(backup_path) = self.monitor_backup_path() {
            util::write_to_file(backup_path, filename, monitor)
                .map_err(|_| chain::ChannelMonitorUpdateErr::PermanentFailure)?;
        }
        Ok(())
    }

    fn update_persisted_channel(
        &self,
        funding_txo: OutPoint,
        _update: &Option<ChannelMonitorUpdate>,
        monitor: &ChannelMonitor<ChannelSigner>,
        _update_id: chainmonitor::MonitorUpdateId,
    ) -> Result<(), chain::ChannelMonitorUpdateErr> {
        let filename = format!("{}_{}", funding_txo.txid.to_hex(), funding_txo.index);
        util::write_to_file(self.monitor_path(), filename.clone(), monitor)
            .map_err(|_| chain::ChannelMonitorUpdateErr::PermanentFailure)?;
        if let Some(backup_path) = self.monitor_backup_path() {
            util::write_to_file(backup_path, filename, monitor)
                .map_err(|_| chain::ChannelMonitorUpdateErr::PermanentFailure)?;
        }
        Ok(())
    }
}

#[async_trait]
impl FileSystemStorage for LightningPersister {
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
                    std::io::ErrorKind::NotADirectory,
                    format!("{} is not a directory", dir_path.display()),
                ))
            } else if backup_dir_path.as_ref().map(|path| !path.is_dir()).unwrap_or(false) {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
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

    async fn save_network_graph(&self, network_graph: Arc<NetworkGraph>) -> Result<(), Self::Error> {
        let path = self.network_graph_path();
        async_blocking(move || {
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)?;
            network_graph.write(&mut BufWriter::new(file))
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

        let params = [
            rpc_id,
            channel_id,
            counterparty_node_id,
            is_outbound,
            is_public,
            is_closed,
            created_at,
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
        let rpc_id = rpc_id.to_string();

        let params = [funding_tx, funding_value, funding_generated_in_block, rpc_id];

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

    async fn update_channel_to_closed(
        &self,
        rpc_id: u64,
        closure_reason: String,
        closed_at: u64,
    ) -> Result<(), Self::Error> {
        let for_coin = self.storage_ticker.clone();
        let is_closed = "1".to_string();
        let rpc_id = rpc_id.to_string();

        let params = [closure_reason, is_closed, closed_at.to_string(), rpc_id];

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
        let rpc_id = rpc_id.to_string();

        let params = [closing_tx, rpc_id];

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

        let params = [claiming_tx, claimed_balance, closing_tx];

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
                    let maybe_offset =
                        offset_by_id(&conn, &sql_builder, params, "rpc_id", "closed_at DESC", "rpc_id = ?1")?;
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
    extern crate bitcoin;
    extern crate lightning;
    use bitcoin::blockdata::block::{Block, BlockHeader};
    use bitcoin::hashes::hex::FromHex;
    use bitcoin::Txid;
    use common::{block_on, now_ms};
    use db_common::sqlite::rusqlite::Connection;
    use lightning::chain::chainmonitor::Persist;
    use lightning::chain::transaction::OutPoint;
    use lightning::chain::ChannelMonitorUpdateErr;
    use lightning::ln::features::InitFeatures;
    use lightning::ln::functional_test_utils::*;
    use lightning::util::events::{ClosureReason, MessageSendEventsProvider};
    use lightning::util::test_utils;
    use lightning::{check_added_monitors, check_closed_broadcast, check_closed_event};
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
                closed_at: Some(rng.gen::<u32>() as u64),
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

    // Integration-test the LightningPersister. Test relaying a few payments
    // and check that the persisted data is updated the appropriate number of
    // times.
    #[test]
    fn test_filesystem_persister() {
        // Create the nodes, giving them LightningPersisters for data persisters.
        let persister_0 = LightningPersister::new(
            "test_filesystem_persister_0".into(),
            PathBuf::from("test_filesystem_persister_0"),
            None,
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        );
        let persister_1 = LightningPersister::new(
            "test_filesystem_persister_1".into(),
            PathBuf::from("test_filesystem_persister_1"),
            None,
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        );
        let chanmon_cfgs = create_chanmon_cfgs(2);
        let mut node_cfgs = create_node_cfgs(2, &chanmon_cfgs);
        let chain_mon_0 = test_utils::TestChainMonitor::new(
            Some(&chanmon_cfgs[0].chain_source),
            &chanmon_cfgs[0].tx_broadcaster,
            &chanmon_cfgs[0].logger,
            &chanmon_cfgs[0].fee_estimator,
            &persister_0,
            &node_cfgs[0].keys_manager,
        );
        let chain_mon_1 = test_utils::TestChainMonitor::new(
            Some(&chanmon_cfgs[1].chain_source),
            &chanmon_cfgs[1].tx_broadcaster,
            &chanmon_cfgs[1].logger,
            &chanmon_cfgs[1].fee_estimator,
            &persister_1,
            &node_cfgs[1].keys_manager,
        );
        node_cfgs[0].chain_monitor = chain_mon_0;
        node_cfgs[1].chain_monitor = chain_mon_1;
        let node_chanmgrs = create_node_chanmgrs(2, &node_cfgs, &[None, None]);
        let nodes = create_network(2, &node_cfgs, &node_chanmgrs);

        // Check that the persisted channel data is empty before any channels are
        // open.
        let mut persisted_chan_data_0 = persister_0.read_channelmonitors(nodes[0].keys_manager).unwrap();
        assert_eq!(persisted_chan_data_0.len(), 0);
        let mut persisted_chan_data_1 = persister_1.read_channelmonitors(nodes[1].keys_manager).unwrap();
        assert_eq!(persisted_chan_data_1.len(), 0);

        // Helper to make sure the channel is on the expected update ID.
        macro_rules! check_persisted_data {
            ($expected_update_id: expr) => {
                persisted_chan_data_0 = persister_0.read_channelmonitors(nodes[0].keys_manager).unwrap();
                assert_eq!(persisted_chan_data_0.len(), 1);
                for (_, mon) in persisted_chan_data_0.iter() {
                    assert_eq!(mon.get_latest_update_id(), $expected_update_id);
                }
                persisted_chan_data_1 = persister_1.read_channelmonitors(nodes[1].keys_manager).unwrap();
                assert_eq!(persisted_chan_data_1.len(), 1);
                for (_, mon) in persisted_chan_data_1.iter() {
                    assert_eq!(mon.get_latest_update_id(), $expected_update_id);
                }
            };
        }

        // Create some initial channel and check that a channel was persisted.
        let _ = create_announced_chan_between_nodes(&nodes, 0, 1, InitFeatures::known(), InitFeatures::known());
        check_persisted_data!(0);

        // Send a few payments and make sure the monitors are updated to the latest.
        send_payment(&nodes[0], &vec![&nodes[1]][..], 8000000);
        check_persisted_data!(5);
        send_payment(&nodes[1], &vec![&nodes[0]][..], 4000000);
        check_persisted_data!(10);

        // Force close because cooperative close doesn't result in any persisted
        // updates.
        nodes[0]
            .node
            .force_close_channel(&nodes[0].node.list_channels()[0].channel_id)
            .unwrap();
        check_closed_event!(nodes[0], 1, ClosureReason::HolderForceClosed);
        check_closed_broadcast!(nodes[0], true);
        check_added_monitors!(nodes[0], 1);

        let node_txn = nodes[0].tx_broadcaster.txn_broadcasted.lock().unwrap();
        assert_eq!(node_txn.len(), 1);

        let header = BlockHeader {
            version: 0x20000000,
            prev_blockhash: nodes[0].best_block_hash(),
            merkle_root: Default::default(),
            time: 42,
            bits: 42,
            nonce: 42,
        };
        connect_block(&nodes[1], &Block {
            header,
            txdata: vec![node_txn[0].clone(), node_txn[0].clone()],
        });
        check_closed_broadcast!(nodes[1], true);
        check_closed_event!(nodes[1], 1, ClosureReason::CommitmentTxConfirmed);
        check_added_monitors!(nodes[1], 1);

        // Make sure everything is persisted as expected after close.
        check_persisted_data!(11);
    }

    // Test that if the persister's path to channel data is read-only, writing a
    // monitor to it results in the persister returning a PermanentFailure.
    // Windows ignores the read-only flag for folders, so this test is Unix-only.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_readonly_dir_perm_failure() {
        let persister = LightningPersister::new(
            "test_readonly_dir_perm_failure".into(),
            PathBuf::from("test_readonly_dir_perm_failure"),
            None,
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        );
        fs::create_dir_all(&persister.main_path).unwrap();

        // Set up a dummy channel and force close. This will produce a monitor
        // that we can then use to test persistence.
        let chanmon_cfgs = create_chanmon_cfgs(2);
        let node_cfgs = create_node_cfgs(2, &chanmon_cfgs);
        let node_chanmgrs = create_node_chanmgrs(2, &node_cfgs, &[None, None]);
        let nodes = create_network(2, &node_cfgs, &node_chanmgrs);
        let chan = create_announced_chan_between_nodes(&nodes, 0, 1, InitFeatures::known(), InitFeatures::known());
        nodes[1].node.force_close_channel(&chan.2).unwrap();
        check_closed_event!(nodes[1], 1, ClosureReason::HolderForceClosed);
        let mut added_monitors = nodes[1].chain_monitor.added_monitors.lock().unwrap();
        let update_map = nodes[1].chain_monitor.latest_monitor_update_id.lock().unwrap();
        let update_id = update_map.get(&added_monitors[0].0.to_channel_id()).unwrap();

        // Set the persister's directory to read-only, which should result in
        // returning a permanent failure when we then attempt to persist a
        // channel update.
        let path = &persister.main_path;
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(path, perms).unwrap();

        let test_txo = OutPoint {
            txid: Txid::from_hex("8984484a580b825b9972d7adb15050b3ab624ccd731946b3eeddb92f4e7ef6be").unwrap(),
            index: 0,
        };
        match persister.persist_new_channel(test_txo, &added_monitors[0].1, update_id.2) {
            Err(ChannelMonitorUpdateErr::PermanentFailure) => {},
            _ => panic!("unexpected result from persisting new channel"),
        }

        nodes[1].node.get_and_clear_pending_msg_events();
        added_monitors.clear();
    }

    // Test that if a persister's directory name is invalid, monitor persistence
    // will fail.
    #[cfg(target_os = "windows")]
    #[test]
    fn test_fail_on_open() {
        // Set up a dummy channel and force close. This will produce a monitor
        // that we can then use to test persistence.
        let chanmon_cfgs = create_chanmon_cfgs(2);
        let node_cfgs = create_node_cfgs(2, &chanmon_cfgs);
        let node_chanmgrs = create_node_chanmgrs(2, &node_cfgs, &[None, None]);
        let nodes = create_network(2, &node_cfgs, &node_chanmgrs);
        let chan = create_announced_chan_between_nodes(&nodes, 0, 1, InitFeatures::known(), InitFeatures::known());
        nodes[1].node.force_close_channel(&chan.2).unwrap();
        check_closed_event!(nodes[1], 1, ClosureReason::HolderForceClosed);
        let mut added_monitors = nodes[1].chain_monitor.added_monitors.lock().unwrap();
        let update_map = nodes[1].chain_monitor.latest_monitor_update_id.lock().unwrap();
        let update_id = update_map.get(&added_monitors[0].0.to_channel_id()).unwrap();

        // Create the persister with an invalid directory name and test that the
        // channel fails to open because the directories fail to be created. There
        // don't seem to be invalid filename characters on Unix that Rust doesn't
        // handle, hence why the test is Windows-only.
        let persister = LightningPersister::new(
            "test_fail_on_open".into(),
            PathBuf::from(":<>/"),
            None,
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        );

        let test_txo = OutPoint {
            txid: Txid::from_hex("8984484a580b825b9972d7adb15050b3ab624ccd731946b3eeddb92f4e7ef6be").unwrap(),
            index: 0,
        };
        match persister.persist_new_channel(test_txo, &added_monitors[0].1, update_id.2) {
            Err(ChannelMonitorUpdateErr::PermanentFailure) => {},
            _ => panic!("unexpected result from persisting new channel"),
        }

        nodes[1].node.get_and_clear_pending_msg_events();
        added_monitors.clear();
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

        let current_time = now_ms() / 1000;
        block_on(persister.update_channel_to_closed(2, "the channel was cooperatively closed".into(), current_time))
            .unwrap();
        expected_channel_details.closure_reason = Some("the channel was cooperatively closed".into());
        expected_channel_details.is_closed = true;
        expected_channel_details.closed_at = Some(current_time);

        let actual_channel_details = block_on(persister.get_channel_from_db(2)).unwrap().unwrap();
        assert_eq!(expected_channel_details, actual_channel_details);

        let actual_channels = block_on(persister.get_closed_channels_with_no_closing_tx()).unwrap();
        assert_eq!(actual_channels.len(), 1);

        let closed_channels =
            block_on(persister.get_closed_channels_by_filter(None, PagingOptionsEnum::default(), 10)).unwrap();
        assert_eq!(closed_channels.channels.len(), 1);
        assert_eq!(expected_channel_details, closed_channels.channels[0]);

        block_on(persister.update_channel_to_closed(1, "the channel was cooperatively closed".into(), now_ms() / 1000))
            .unwrap();
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

        let channels = generate_random_channels(100);

        for channel in channels {
            block_on(persister.add_channel_to_db(channel.clone())).unwrap();
            block_on(persister.add_funding_tx_to_db(
                channel.rpc_id,
                channel.funding_tx.unwrap(),
                channel.funding_value.unwrap(),
                channel.funding_generated_in_block.unwrap(),
            ))
            .unwrap();
            block_on(persister.update_channel_to_closed(channel.rpc_id, channel.closure_reason.unwrap(), 1655806080))
                .unwrap();
            block_on(persister.add_closing_tx_to_db(channel.rpc_id, channel.closing_tx.clone().unwrap())).unwrap();
            block_on(persister.add_claiming_tx_to_db(
                channel.closing_tx.unwrap(),
                channel.claiming_tx.unwrap(),
                channel.claimed_balance.unwrap(),
            ))
            .unwrap();
        }

        // get all channels from SQL since updated_at changed from channels generated by generate_random_channels
        let channels = block_on(persister.get_closed_channels_by_filter(None, PagingOptionsEnum::default(), 100))
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

        let expected_channels = channels[20..23].to_vec();
        let actual_channels = result.channels;

        assert_eq!(expected_channels, actual_channels);

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
