use common::log::debug;
use common::rusqlite::{Connection, Result as SqlResult, ToSql};
use sql_builder::SqlBuilder;
use std::convert::TryInto;
use std::num::NonZeroUsize;
use uuid::Uuid;

const fn ten() -> usize { 10 }

fn one() -> NonZeroUsize { NonZeroUsize::new(1).unwrap() }

#[derive(Debug, Deserialize)]
pub struct PagingOptions {
    #[serde(default = "ten")]
    pub limit: usize,
    #[serde(default = "one")]
    pub page_number: NonZeroUsize,
    pub from_uuid: Option<Uuid>,
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
