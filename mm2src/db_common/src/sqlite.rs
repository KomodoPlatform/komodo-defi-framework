pub use rusqlite;
pub use sql_builder;
use std::fmt;

use log::debug;
use rusqlite::types::{FromSql, Type as SqlType, Value};
use rusqlite::{Connection, Error as SqlError, Result as SqlResult, Row, ToSql, NO_PARAMS};
use sql_builder::SqlBuilder;
use std::sync::{Arc, Mutex, Weak};
use uuid::Uuid;

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

pub type SqliteConnShared = Arc<Mutex<Connection>>;
pub type SqliteConnWeak = Weak<Mutex<Connection>>;

pub(crate) type ParamId = String;

pub(crate) type OwnedSqlParam = Value;
pub(crate) type OwnedSqlParams = Vec<OwnedSqlParam>;

type SqlNamedParam<'a> = (&'a str, &'a dyn ToSql);
pub type SqlNamedParams<'a> = Vec<SqlNamedParam<'a>>;
type OwnedSqlNamedParam = (&'static str, Value);
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

pub fn validate_ident(ident: &str) -> SqlResult<()> {
    validate_ident_impl(ident, |c| c.is_alphanumeric() || c == '_' || c == '.')
}

pub fn validate_table_name(table_name: &str) -> SqlResult<()> {
    // As per https://stackoverflow.com/a/3247553, tables can't be the target of parameter substitution.
    // So we have to use a plain concatenation disallowing any characters in the table name that may lead to SQL injection.
    validate_ident_impl(table_name, |c| c.is_alphanumeric() || c == '_')
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

/// As per https://twitter.com/marcan42/status/1494213862970707969, I've noticed significant SQLite performance
/// difference on M1 Mac and Linux.
/// But according to https://phiresky.github.io/blog/2020/sqlite-performance-tuning/, these pragmas should
/// be safe to use, while giving great speed boost.
/// With these, Mac and Linux have comparable SQLite performance.
pub fn run_optimization_pragmas(conn: &Connection) -> Result<(), SqlError> {
    conn.query_row("pragma journal_mode = WAL;", NO_PARAMS, |row| row.get::<_, String>(0))?;
    conn.execute("pragma synchronous = normal;", NO_PARAMS)?;
    conn.execute("pragma temp_store = memory;", NO_PARAMS)?;
    Ok(())
}

pub fn execute_batch(statement: &'static [&str]) -> Vec<(&'static str, Vec<String>)> {
    statement.iter().map(|sql| (*sql, vec![])).collect()
}

pub trait ToValidSqlTable {
    /// Converts `self` to a valid SQL table name or returns an error.
    fn to_valid_sql_table(&self) -> SqlResult<String>;
}

impl<S: ToString> ToValidSqlTable for S {
    fn to_valid_sql_table(&self) -> SqlResult<String> {
        let table = self.to_string();
        validate_table_name(&table)?;
        Ok(table)
    }
}

pub trait ToValidSqlIdent {
    /// Converts `self` to a valid SQL value or returns an error.
    fn to_valid_sql_ident(&self) -> SqlResult<String>;
}

impl<S: ToString> ToValidSqlIdent for S {
    fn to_valid_sql_ident(&self) -> SqlResult<String> {
        let ident = self.to_string();
        validate_ident(&ident)?;
        Ok(ident)
    }
}

/// A valid SQL value that can be passed as an argument to the `SqlBuilder` or `SqlQuery` safely.
pub enum SqlValue {
    String(&'static str),
    Decimal(i64),
}

impl SqlValue {
    /// Converts the given `value` to string if it implements `Into<SqlValue>`.
    /// The resulting string is considered a safe SQL value.
    pub(crate) fn value_to_string<S>(value: S) -> String
    where
        SqlValue: From<S>,
    {
        SqlValue::from(value).to_string()
    }

    /// Converts the given `values` to `Vec<String>` if they implement `Into<SqlValue>`.
    /// /// The resulting strings are considered safe SQL values.
    pub(crate) fn values_to_strings<I, S>(values: I) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        SqlValue: From<S>,
    {
        values.into_iter().map(SqlValue::value_to_string).collect()
    }
}

impl fmt::Display for SqlValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SqlValue::String(string) => write!(f, "{}", string),
            SqlValue::Decimal(decimal) => write!(f, "{}", decimal),
        }
    }
}

impl From<&'static str> for SqlValue {
    fn from(string: &'static str) -> Self { SqlValue::String(string) }
}

impl From<i64> for SqlValue {
    fn from(decimal: i64) -> Self { SqlValue::Decimal(decimal) }
}

/// This structure manages the SQL parameters.
#[derive(Clone, Default)]
pub(crate) struct SqlParamsBuilder {
    next_param_id: usize,
    params: OwnedSqlParams,
}

impl SqlParamsBuilder {
    /// Pushes the given `param` and returns its `:<IDX>` identifier.
    pub(crate) fn push_param<P>(&mut self, param: P) -> ParamId
    where
        OwnedSqlParam: From<P>,
    {
        self.params.push(OwnedSqlParam::from(param));
        self.next_param_id += 1;
        format!(":{}", self.next_param_id)
    }

    /// Pushes the given `params` and returns their `:<IDX>` identifiers.
    pub(crate) fn push_params<I, P>(&mut self, params: I) -> Vec<ParamId>
    where
        I: IntoIterator<Item = P>,
        OwnedSqlParam: From<P>,
    {
        params.into_iter().map(|param| self.push_param(param)).collect()
    }

    pub(crate) fn params(&self) -> &OwnedSqlParams { &self.params }
}

fn validate_ident_impl<F>(ident: &str, is_valid: F) -> SqlResult<()>
where
    F: Fn(char) -> bool,
{
    if ident.chars().all(is_valid) {
        Ok(())
    } else {
        Err(SqlError::InvalidParameterName(ident.to_string()))
    }
}
