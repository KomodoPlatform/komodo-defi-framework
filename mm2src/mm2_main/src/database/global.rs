use db_common::sqlite::rusqlite::{params_from_iter, Error as SqlError};
use mm2_core::mm_ctx::MmArc;
use uuid::Uuid;
use db_common::sqlite::{query_single_row, StringError};

// FIXME: Should we increase the max address size here?
// FIXME: Consider omitting the taker address as it could be easily queried from the address DB once we have the maker address.
const INIT_GLOBAL_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS swaps (
        uuid VARCHAR(255) PRIMARY KEY,
        maker_address VARCHAR(255),
        taker_address VARCHAR(255),
        is_finished BOOLEAN
    );
    CREATE TABLE IF NOT EXISTS orders (
        uuid TEXT PRIMARY KEY,
        address VARCHAR(255),
    );
";



// FIXME: Better error types please.
//        Also querying should really be async, but this issue really applies to all the sqlite code.
pub async fn get_address_for_order_uuid(ctx: &MmArc, uuid: &Uuid) -> Result<Option<String>, String> {
    const SELECT_ADDRESS_FOR_ORDER_UUID: &str = "SELECT address FROM orders WHERE uuid = ?1";
    let conn = ctx.global_db().await?;
    query_single_row(&conn, SELECT_ADDRESS_FOR_ORDER_UUID, params_from_iter([uuid.to_string()]), |row| row.get(0)).map_err(|e| e.to_string())
}

pub async fn get_address_for_swap_uuid(ctx: &MmArc, uuid: &Uuid) -> Result<Option<String>, String> {
    const SELECT_ADDRESS_FOR_SWAP_UUID: &str = "SELECT maker_address FROM swaps WHERE uuid = ?1";
    let conn = ctx.global_db().await?;
    query_single_row(&conn, SELECT_ADDRESS_FOR_SWAP_UUID, params_from_iter([uuid.to_string()]), |row| row.get(0)).map_err(|e| e.to_string())
}

pub async fn get_unfinished_swaps(ctx: &MmArc) -> Result<Vec<(Uuid, String, String)>, String> {
    const SELECT_UNFINISHED_SWAPS_UUIDS_BY_TYPE: &str =
        "SELECT uuid, maker_address, taker_address FROM swaps WHERE is_finished = 0;";
    let conn = ctx.global_db().await?;
    let unfinished_swaps_result: Result<Vec<_>, _> = conn.prepare(SELECT_UNFINISHED_SWAPS_UUIDS_BY_TYPE).map_err(|e| e.to_string())?
        .query_map([], |row| {
            let uuid: String = row.get(0)?;
            let maker_address: String = row.get(1)?;
            let taker_address: String = row.get(2)?;
            let uuid = Uuid::parse_str(&uuid).map_err(|e| SqlError::ToSqlConversionFailure(StringError::from(e.to_string()).into_boxed()))?;
            Ok((uuid, maker_address, taker_address))
        })
        .map_err(|e| e.to_string())?
        .collect();
    match unfinished_swaps_result {
        Ok(unfinished_swaps) => Ok(unfinished_swaps),
        Err(e) => Err(e.to_string())
    }
}

pub async fn get_all_order_addresses(ctx: &MmArc) -> Result<Vec<String>, String> {
    const SELECT_ALL_ORDER_ADDRESSES: &str = "SELECT address FROM orders";
    let conn = ctx.global_db().await?;
    let addresses_result: Result<Vec<_>, _> = conn.prepare(SELECT_ALL_ORDER_ADDRESSES).map_err(|e| e.to_string())?
        .query_map([], |row| row.get(0)).map_err(|e| e.to_string())?
        .collect();
    match addresses_result {
        Ok(addresses) => Ok(addresses),
        Err(e) => Err(e.to_string())
    }
}

// FIXME: Remove this, actually let just the call .global_db() return an already initialized & migrated DB.
pub async fn init_global_db(ctx: &MmArc) -> Result<(), String> {
    let conn = ctx.global_db().await?;
    conn.execute_batch(INIT_GLOBAL_TABLE).map_err(|e| e.to_string())
}