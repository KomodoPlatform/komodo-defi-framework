pub use rusqlite;
pub use sql_builder;

use log::debug;
use rusqlite::types::{FromSql, Type as SqlType, Value};
use rusqlite::{Connection, Error as SqlError, Result as SqlResult, Row, ToSql};
use sql_builder::SqlBuilder;
use std::sync::{Arc, Mutex, Weak};
use uuid::Uuid;

pub type SqliteConnShared = Arc<Mutex<Connection>>;
pub type SqliteConnWeak = Weak<Mutex<Connection>>;

pub const CHECK_TABLE_EXISTS_SQL: &str = "SELECT name FROM sqlite_master WHERE type='table' AND name=?1;";

/// The macro returns `OwnedSqlNamedParams`.
#[macro_export]
macro_rules! owned_named_params {
    () => {
        Vec::new()
    };
    ($($param_name:literal: $param_val:expr),+ $(,)?) => {
        vec![$(($param_name, Value::from($param_val))),+]
    };
}

type SqlNamedParam<'a> = (&'a str, &'a dyn ToSql);
type SqlNamedParams<'a> = Vec<SqlNamedParam<'a>>;
pub type OwnedSqlNamedParam = (&'static str, Value);
pub type OwnedSqlNamedParams = Vec<OwnedSqlNamedParam>;

pub trait AsSqlNamedParams {
    fn as_sql_named_params(&self) -> SqlNamedParams<'_>;
}

impl AsSqlNamedParams for OwnedSqlNamedParams {
    fn as_sql_named_params(&self) -> SqlNamedParams<'_> {
        self.iter().map(|(name, param)| (*name, param as &dyn ToSql)).collect()
    }
}

pub fn string_from_row(row: &Row<'_>) -> Result<String, SqlError> { row.get(0) }

pub fn query_single_row<T, P, F>(conn: &Connection, query: &str, params: P, map_fn: F) -> Result<Option<T>, SqlError>
where
    P: IntoIterator,
    P::Item: ToSql,
    F: FnOnce(&Row<'_>) -> Result<T, SqlError>,
{
    let maybe_result = conn.query_row(query, params, map_fn);
    if let Err(SqlError::QueryReturnedNoRows) = maybe_result {
        return Ok(None);
    }

    let result = maybe_result?;
    Ok(Some(result))
}

pub fn validate_table_name(table_name: &str) -> SqlResult<()> {
    // As per https://stackoverflow.com/a/3247553, tables can't be the target of parameter substitution.
    // So we have to use a plain concatenation disallowing any characters in the table name that may lead to SQL injection.
    if table_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        Ok(())
    } else {
        Err(SqlError::InvalidParameterName(table_name.to_string()))
    }
}

/// Calculates the offset to skip records by uuid.
/// Expects `query_builder` to have where clauses applied *before* calling this fn.
pub fn offset_by_uuid(
    conn: &Connection,
    query_builder: &SqlBuilder,
    params: &[(&str, String)],
    uuid: &Uuid,
) -> SqlResult<usize> {
    // building following query to determine offset by from_uuid
    // select row from (
    //     select uuid, ROW_NUMBER() OVER (ORDER BY started_at DESC) AS row
    //     from my_swaps
    //     where ... filtering options here ...
    // ) where uuid = "from_uuid";
    let subquery = query_builder
        .clone()
        .field("ROW_NUMBER() OVER (ORDER BY started_at DESC) AS row")
        .field("uuid")
        .subquery()
        .expect("SQL query builder should never fail here");

    let external_query = SqlBuilder::select_from(subquery)
        .field("row")
        .and_where("uuid = :uuid")
        .sql()
        .expect("SQL query builder should never fail here");

    let mut params_for_offset = params.to_owned();
    params_for_offset.push((":uuid", uuid.to_string()));
    let params_as_trait: Vec<_> = params_for_offset
        .iter()
        .map(|(key, value)| (*key, value as &dyn ToSql))
        .collect();

    debug!(
        "Trying to execute SQL query {} with params {:?}",
        external_query, params_for_offset
    );

    let mut stmt = conn.prepare(&external_query)?;
    let offset: isize = stmt.query_row_named(params_as_trait.as_slice(), |row| row.get(0))?;
    Ok(offset.try_into().expect("row index should be always above zero"))
}

/// A more universal offset_by_id query that will replace offset_by_uuid at some point
pub fn offset_by_id<P>(
    conn: &Connection,
    query_builder: &SqlBuilder,
    params: P,
    id_field: &str,
    order_by: &str,
    where_id: &str,
) -> SqlResult<Option<usize>>
where
    P: IntoIterator + std::fmt::Debug,
    P::Item: ToSql,
{
    let row_number = format!("ROW_NUMBER() OVER (ORDER BY {}) AS row", order_by);
    let subquery = query_builder
        .clone()
        .field(&row_number)
        .field(id_field)
        .subquery()
        .expect("SQL query builder should never fail here");

    let external_query = SqlBuilder::select_from(subquery)
        .field("row")
        .and_where(where_id)
        .sql()
        .expect("SQL query builder should never fail here");

    debug!(
        "Trying to execute SQL query {} with params {:?}",
        external_query, params,
    );

    let mut stmt = conn.prepare(&external_query)?;
    let maybe_offset = stmt.query_row(params, |row| row.get::<_, isize>(0));
    if let Err(SqlError::QueryReturnedNoRows) = maybe_offset {
        return Ok(None);
    }
    let offset = maybe_offset?;
    Ok(Some(offset.try_into().expect("row index should be always above zero")))
}

pub fn sql_text_conversion_err<E>(field_id: usize, e: E) -> SqlError
where
    E: std::error::Error + Send + Sync + 'static,
{
    SqlError::FromSqlConversionFailure(field_id, SqlType::Text, Box::new(e))
}

pub fn h256_slice_from_row<T>(row: &Row<'_>, column_id: usize) -> Result<[u8; 32], SqlError>
where
    T: AsRef<[u8]> + FromSql,
{
    let mut h256_slice = [0u8; 32];
    hex::decode_to_slice(row.get::<_, T>(column_id)?, &mut h256_slice as &mut [u8])
        .map_err(|e| sql_text_conversion_err(column_id, e))?;
    Ok(h256_slice)
}

pub fn h256_option_slice_from_row<T>(row: &Row<'_>, column_id: usize) -> Result<Option<[u8; 32]>, SqlError>
where
    T: AsRef<[u8]> + FromSql,
{
    let maybe_h256_slice = row.get::<_, Option<T>>(column_id)?;
    let res = match maybe_h256_slice {
        Some(s) => {
            let mut h256_slice = [0u8; 32];
            hex::decode_to_slice(s, &mut h256_slice as &mut [u8]).map_err(|e| sql_text_conversion_err(column_id, e))?;
            Some(h256_slice)
        },
        None => None,
    };
    Ok(res)
}

pub fn execute_batch(statement: &'static [&str]) -> Vec<(&'static str, Vec<String>)> {
    statement.iter().map(|sql| (*sql, vec![])).collect()
}
