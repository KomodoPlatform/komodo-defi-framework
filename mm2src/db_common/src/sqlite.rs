pub use rusqlite;
pub use sql_builder;

use log::debug;
use rusqlite::{Connection, Error as SqlError, Result as SqlResult, ToSql};
use sql_builder::SqlBuilder;
use std::sync::{Arc, Mutex, Weak};
use uuid::Uuid;

pub type SqliteConnShared = Arc<Mutex<Connection>>;
pub type SqliteConnWeak = Weak<Mutex<Connection>>;

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
