use db_common::{async_sql_conn::AsyncConnError,
                sqlite::{query_single_row, rusqlite::params}};
use mm2_core::mm_ctx::MmArc;
use uuid::Uuid;

// TODO: Let's let this table be called `ongoing_swaps` and remove the `is_finished` column.
//       And we might also add a new table `completed_swaps` that hold a copy of all the completed swaps for all the coins.
const INIT_GLOBAL_DB_TABLES: &str = "
    CREATE TABLE IF NOT EXISTS swaps (
        uuid VARCHAR(255) PRIMARY KEY,
        maker_address VARCHAR(255) NOT NULL
    );
";
const SELECT_ADDRESS_FOR_SWAP_UUID: &str = "SELECT maker_address FROM swaps WHERE uuid = ?1";
const INSERT_SWAP: &str = "INSERT INTO swaps (uuid, maker_address) VALUES (?1, ?2)";

/// Errors that can occur when interacting with the global database.
#[derive(Debug, Display)]
pub enum GlobalDBError {
    SqlError(AsyncConnError),
}

impl From<AsyncConnError> for GlobalDBError {
    fn from(err: AsyncConnError) -> Self { GlobalDBError::SqlError(err) }
}

/// Initializes the global database with the necessary tables.
pub async fn init_global_db(ctx: &MmArc) -> Result<(), GlobalDBError> {
    let conn = ctx.async_global_db().await;
    conn.lock()
        .await
        .call(|conn| conn.execute_batch(INIT_GLOBAL_DB_TABLES).map_err(|e| e.into()))
        .await?;
    Ok(())
}

/// Gets the maker address for a given swap UUID from the global database.
///
/// Returns `Ok(Some(addr))` if the UUID is found, `Ok(None)` if the UUID is not found, and `Err(e)` if there was an error.
pub async fn get_maker_address_for_swap_uuid(ctx: &MmArc, uuid: &Uuid) -> Result<Option<String>, GlobalDBError> {
    let conn = ctx.async_global_db().await;
    let uuid = uuid.to_string();
    let address: Option<String> = conn
        .lock()
        .await
        .call(move |conn| {
            query_single_row(conn, SELECT_ADDRESS_FOR_SWAP_UUID, params![uuid], |row| row.get(0)).map_err(|e| e.into())
        })
        .await?;
    Ok(address)
}

/// Inserts a new swap handle (uuid and maker address pair) into the global database.
pub async fn insert_swap_in_global_db(ctx: &MmArc, uuid: &Uuid, maker_address: &str) -> Result<(), GlobalDBError> {
    let conn = ctx.async_global_db().await;
    let uuid = uuid.to_string();
    let maker_address = maker_address.to_string();
    conn.lock()
        .await
        .call(move |conn| {
            conn.execute(INSERT_SWAP, params![uuid, maker_address])
                .map_err(|e| e.into())
        })
        .await?;
    Ok(())
}
