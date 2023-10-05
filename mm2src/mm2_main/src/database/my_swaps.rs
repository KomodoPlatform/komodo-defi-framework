#![allow(deprecated)] // TODO: remove this once rusqlite is >= 0.29

/// This module contains code to work with my_swaps table in MM2 SQLite DB
use crate::mm2::lp_swap::{MyRecentSwapsUuids, MySwapsFilter, SavedSwap, SavedSwapIo};
use common::log::debug;
use common::PagingOptions;
use db_common::sqlite::offset_by_uuid;
use db_common::sqlite::rusqlite::{Connection, Error as SqlError, Result as SqlResult, Row, ToSql};
use db_common::sqlite::sql_builder::SqlBuilder;
use mm2_core::mm_ctx::MmArc;
use std::convert::TryInto;
use uuid::Error as UuidError;

const MY_SWAPS_TABLE: &str = "my_swaps";

// Using a macro because static variable can't be passed to concat!
// https://stackoverflow.com/a/39024422
#[macro_export]
macro_rules! CREATE_MY_SWAPS_TABLE {
    () => {
        "CREATE TABLE IF NOT EXISTS my_swaps (
            id INTEGER NOT NULL PRIMARY KEY,
            my_coin VARCHAR(255) NOT NULL,
            other_coin VARCHAR(255) NOT NULL,
            uuid VARCHAR(255) NOT NULL UNIQUE,
            started_at INTEGER NOT NULL
        );"
    };
}

/// Adds new fields required for trading protocol upgrade implementation (swap v2)
pub const TRADING_PROTO_UPGRADE_MIGRATION: &[&str] = &[
    "ALTER TABLE my_swaps ADD COLUMN is_finished BOOLEAN NOT NULL DEFAULT 0;",
    "ALTER TABLE my_swaps ADD COLUMN events_json TEXT NOT NULL DEFAULT '[]';",
    "ALTER TABLE my_swaps ADD COLUMN swap_type INTEGER NOT NULL DEFAULT 0;",
    // Storing rational numbers as text to maintain precision
    "ALTER TABLE my_swaps ADD COLUMN maker_volume TEXT;",
    // Storing rational numbers as text to maintain precision
    "ALTER TABLE my_swaps ADD COLUMN taker_volume TEXT;",
    // Storing rational numbers as text to maintain precision
    "ALTER TABLE my_swaps ADD COLUMN premium TEXT;",
    // Storing rational numbers as text to maintain precision
    "ALTER TABLE my_swaps ADD COLUMN dex_fee TEXT;",
    "ALTER TABLE my_swaps ADD COLUMN secret BLOB;",
    "ALTER TABLE my_swaps ADD COLUMN secret_hash BLOB;",
    "ALTER TABLE my_swaps ADD COLUMN secret_hash_algo INTEGER;",
    "ALTER TABLE my_swaps ADD COLUMN p2p_privkey BLOB;",
    "ALTER TABLE my_swaps ADD COLUMN lock_duration INTEGER;",
    "ALTER TABLE my_swaps ADD COLUMN maker_coin_confs INTEGER;",
    "ALTER TABLE my_swaps ADD COLUMN maker_coin_nota BOOLEAN;",
    "ALTER TABLE my_swaps ADD COLUMN taker_coin_confs INTEGER;",
    "ALTER TABLE my_swaps ADD COLUMN taker_coin_nota BOOLEAN;",
];

const INSERT_MY_SWAP: &str = "INSERT INTO my_swaps (my_coin, other_coin, uuid, started_at) VALUES (?1, ?2, ?3, ?4)";

pub fn insert_new_swap(ctx: &MmArc, my_coin: &str, other_coin: &str, uuid: &str, started_at: &str) -> SqlResult<()> {
    debug!("Inserting new swap {} to the SQLite database", uuid);
    let conn = ctx.sqlite_connection();
    let params = [my_coin, other_coin, uuid, started_at];
    conn.execute(INSERT_MY_SWAP, params).map(|_| ())
}

const INSERT_MY_SWAP_V2: &str = r#"INSERT INTO my_swaps (
    my_coin,
    other_coin,
    uuid,
    started_at,
    swap_type,
    maker_volume,
    taker_volume,
    premium,
    dex_fee,
    secret,
    secret_hash,
    secret_hash_algo,
    p2p_privkey,
    lock_duration,
    maker_coin_confs,
    maker_coin_nota,
    taker_coin_confs,
    taker_coin_nota
) VALUES (
    :my_coin,
    :other_coin,
    :uuid,
    :started_at,
    :swap_type,
    :maker_volume,
    :taker_volume,
    :premium,
    :dex_fee,
    :secret,
    :secret_hash,
    :secret_hash_algo,
    :p2p_privkey,
    :lock_duration,
    :maker_coin_confs,
    :maker_coin_nota,
    :taker_coin_confs,
    :taker_coin_nota
);"#;

pub fn insert_new_swap_v2(ctx: &MmArc, params: &[(&str, &dyn ToSql)]) -> SqlResult<()> {
    let conn = ctx.sqlite_connection();
    conn.execute(INSERT_MY_SWAP_V2, params).map(|_| ())
}

/// Returns SQL statements to initially fill my_swaps table using existing DB with JSON files
pub async fn fill_my_swaps_from_json_statements(ctx: &MmArc) -> Vec<(&'static str, Vec<String>)> {
    let swaps = SavedSwap::load_all_my_swaps_from_db(ctx).await.unwrap_or_default();
    swaps.into_iter().filter_map(insert_saved_swap_sql).collect()
}

fn insert_saved_swap_sql(swap: SavedSwap) -> Option<(&'static str, Vec<String>)> {
    let swap_info = match swap.get_my_info() {
        Some(s) => s,
        // get_my_info returning None means that swap did not even start - so we can keep it away from indexing.
        None => return None,
    };
    let params = vec![
        swap_info.my_coin,
        swap_info.other_coin,
        swap.uuid().to_string(),
        swap_info.started_at.to_string(),
    ];
    Some((INSERT_MY_SWAP, params))
}

#[derive(Debug)]
pub enum SelectRecentSwapsUuidsErr {
    Sql(SqlError),
    Parse(UuidError),
}

impl std::fmt::Display for SelectRecentSwapsUuidsErr {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "{:?}", self) }
}

impl From<SqlError> for SelectRecentSwapsUuidsErr {
    fn from(err: SqlError) -> Self { SelectRecentSwapsUuidsErr::Sql(err) }
}

impl From<UuidError> for SelectRecentSwapsUuidsErr {
    fn from(err: UuidError) -> Self { SelectRecentSwapsUuidsErr::Parse(err) }
}

/// Adds where clauses determined by MySwapsFilter
fn apply_my_swaps_filter(builder: &mut SqlBuilder, params: &mut Vec<(&str, String)>, filter: &MySwapsFilter) {
    if let Some(my_coin) = &filter.my_coin {
        builder.and_where("my_coin = :my_coin");
        params.push((":my_coin", my_coin.clone()));
    }

    if let Some(other_coin) = &filter.other_coin {
        builder.and_where("other_coin = :other_coin");
        params.push((":other_coin", other_coin.clone()));
    }

    if let Some(from_timestamp) = &filter.from_timestamp {
        builder.and_where("started_at >= :from_timestamp");
        params.push((":from_timestamp", from_timestamp.to_string()));
    }

    if let Some(to_timestamp) = &filter.to_timestamp {
        builder.and_where("started_at < :to_timestamp");
        params.push((":to_timestamp", to_timestamp.to_string()));
    }
}

pub fn select_uuids_by_my_swaps_filter(
    conn: &Connection,
    filter: &MySwapsFilter,
    paging_options: Option<&PagingOptions>,
) -> SqlResult<MyRecentSwapsUuids, SelectRecentSwapsUuidsErr> {
    let mut query_builder = SqlBuilder::select_from(MY_SWAPS_TABLE);
    let mut params = vec![];
    apply_my_swaps_filter(&mut query_builder, &mut params, filter);

    // count total records matching the filter
    let mut count_builder = query_builder.clone();
    count_builder.count("id");

    let count_query = count_builder.sql().expect("SQL query builder should never fail here");
    debug!("Trying to execute SQL query {} with params {:?}", count_query, params);

    let params_as_trait: Vec<_> = params.iter().map(|(key, value)| (*key, value as &dyn ToSql)).collect();
    let total_count: isize = conn.query_row_named(&count_query, params_as_trait.as_slice(), |row| row.get(0))?;
    let total_count = total_count.try_into().expect("COUNT should always be >= 0");
    if total_count == 0 {
        return Ok(MyRecentSwapsUuids::default());
    }

    // query the uuids finally
    query_builder.field("uuid");
    query_builder.order_desc("started_at");

    let skipped = match paging_options {
        Some(paging) => {
            // calculate offset, page_number is ignored if from_uuid is set
            let offset = match paging.from_uuid {
                Some(uuid) => offset_by_uuid(conn, &query_builder, &params, &uuid)?,
                None => (paging.page_number.get() - 1) * paging.limit,
            };
            query_builder.limit(paging.limit);
            query_builder.offset(offset);
            offset
        },
        None => 0,
    };

    let uuids_query = query_builder.sql().expect("SQL query builder should never fail here");
    debug!("Trying to execute SQL query {} with params {:?}", uuids_query, params);
    let mut stmt = conn.prepare(&uuids_query)?;
    let uuids = stmt
        .query_map_named(params_as_trait.as_slice(), |row| row.get(0))?
        .collect::<SqlResult<Vec<String>>>()?;
    let uuids: SqlResult<Vec<_>, _> = uuids.into_iter().map(|uuid| uuid.parse()).collect();
    let uuids = uuids?;

    Ok(MyRecentSwapsUuids {
        uuids,
        total_count,
        skipped,
    })
}

/// Queries swap type by uuid
pub fn get_swap_type(conn: &Connection, uuid: &str) -> SqlResult<u8> {
    const SELECT_SWAP_TYPE_BY_UUID: &str = "SELECT swap_type FROM my_swaps WHERE uuid = :uuid;";
    let mut stmt = conn.prepare(SELECT_SWAP_TYPE_BY_UUID)?;
    let swap_type = stmt.query_row(&[(":uuid", uuid)], |row| row.get(0))?;
    Ok(swap_type)
}

/// Queries swap events by uuid
pub fn get_swap_events(conn: &Connection, uuid: &str) -> SqlResult<String> {
    const SELECT_SWAP_EVENTS_BY_UUID: &str = "SELECT events_json FROM my_swaps WHERE uuid = :uuid;";
    let mut stmt = conn.prepare(SELECT_SWAP_EVENTS_BY_UUID)?;
    let swap_type = stmt.query_row(&[(":uuid", uuid)], |row| row.get(0))?;
    Ok(swap_type)
}

/// Updates swap events by uuid
pub fn update_swap_events(conn: &Connection, uuid: &str, events_json: &str) -> SqlResult<()> {
    const UPDATE_SWAP_EVENTS_BY_UUID: &str = "UPDATE my_swaps SET events_json = :events_json WHERE uuid = :uuid;";
    let mut stmt = conn.prepare(UPDATE_SWAP_EVENTS_BY_UUID)?;
    stmt.execute(&[(":uuid", uuid), (":events_json", events_json)])
        .map(|_| ())
}

pub fn set_swap_is_finished(conn: &Connection, uuid: &str) -> SqlResult<()> {
    const UPDATE_SWAP_IS_FINISHED_BY_UUID: &str = "UPDATE my_swaps SET is_finished = 1 WHERE uuid = :uuid;";
    let mut stmt = conn.prepare(UPDATE_SWAP_IS_FINISHED_BY_UUID)?;
    stmt.execute(&[(":uuid", uuid)]).map(|_| ())
}

const SELECT_MY_SWAP_V2_FOR_RPC_BY_UUID: &str = r#"SELECT
    my_coin,
    other_coin,
    uuid,
    started_at,
    is_finished,
    events_json,
    maker_volume,
    taker_volume,
    premium,
    dex_fee,
    secret_hash,
    secret_hash_algo,
    lock_duration,
    maker_coin_confs,
    maker_coin_nota,
    taker_coin_confs,
    taker_coin_nota
FROM my_swaps
WHERE uuid = :uuid;
"#;

/// Represents data of the swap used for RPC, omits fields that should be kept in secret
#[derive(Debug, Serialize)]
pub struct MySwapForRpc {
    my_coin: String,
    other_coin: String,
    uuid: String,
    started_at: i64,
    is_finished: bool,
    events_json: String,
    maker_volume: String,
    taker_volume: String,
    premium: String,
    dex_fee: String,
    secret_hash: Vec<u8>,
    secret_hash_algo: i64,
    lock_duration: i64,
    maker_coin_confs: i64,
    maker_coin_nota: bool,
    taker_coin_confs: i64,
    taker_coin_nota: bool,
}

impl MySwapForRpc {
    fn from_row(row: &Row) -> SqlResult<Self> {
        Ok(Self {
            my_coin: row.get(0)?,
            other_coin: row.get(1)?,
            uuid: row.get(2)?,
            started_at: row.get(3)?,
            is_finished: row.get(4)?,
            events_json: row.get(5)?,
            maker_volume: row.get(6)?,
            taker_volume: row.get(7)?,
            premium: row.get(8)?,
            dex_fee: row.get(9)?,
            secret_hash: row.get(10)?,
            secret_hash_algo: row.get(11)?,
            lock_duration: row.get(12)?,
            maker_coin_confs: row.get(13)?,
            maker_coin_nota: row.get(14)?,
            taker_coin_confs: row.get(15)?,
            taker_coin_nota: row.get(16)?,
        })
    }
}

/// Queries `MySwapForRpc` by uuid
pub fn get_swap_data_for_rpc(conn: &Connection, uuid: &str) -> SqlResult<MySwapForRpc> {
    let mut stmt = conn.prepare(SELECT_MY_SWAP_V2_FOR_RPC_BY_UUID)?;
    let swap_data = stmt.query_row(&[(":uuid", uuid)], MySwapForRpc::from_row)?;
    Ok(swap_data)
}
