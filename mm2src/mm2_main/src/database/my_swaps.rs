#![allow(deprecated)] // TODO: remove this once rusqlite is >= 0.29

use std::convert::TryInto;

use common::log::debug;
use common::PagingOptions;
use db_common::sqlite::offset_by_uuid;
use db_common::sqlite::rusqlite::{Connection, Error as SqlError, Result as SqlResult, ToSql};
use db_common::sqlite::sql_builder::SqlBuilder;
use mm2_core::mm_ctx::MmArc;
use uuid::Error as UuidError;

/// This module contains code to work with my_swaps table in MM2 SQLite DB
use crate::mm2::lp_swap::{MyRecentSwapsUuids, MySwapsFilter, SavedSwap, SavedSwapIo};

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
const INSERT_MY_SWAP: &str = "INSERT INTO my_swaps (my_coin, other_coin, uuid, started_at) VALUES (?1, ?2, ?3, ?4)";

pub fn insert_new_swap(ctx: &MmArc, my_coin: &str, other_coin: &str, uuid: &str, started_at: &str) -> SqlResult<()> {
    debug!("Inserting new swap {} to the SQLite database", uuid);
    let conn = ctx.sqlite_connection();
    let params = [my_coin, other_coin, uuid, started_at];
    conn.execute(INSERT_MY_SWAP, params).map(|_| ())
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
