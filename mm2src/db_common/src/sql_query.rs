use crate::sqlite::{query_single_row, validate_table_name, OwnedSqlParam, OwnedSqlParams, SqlParamsBuilder};
use log::debug;
use rusqlite::{Connection, Error as SqlError, Result as SqlResult, Row};
use sql_builder::SqlBuilder;
use std::error::Error as StdError;
use std::fmt;

#[derive(Clone)]
pub struct SqlQuery<'a> {
    conn: &'a Connection,
    sql_builder: SqlBuilder,
    params: SqlParamsBuilder,
    ordering: Vec<SqlOrdering>,
}

impl<'a> SqlQuery<'a> {
    /// Create SELECT query.
    /// Please note the function validates the given `table` name.
    pub fn select_from(conn: &'a Connection, table: &str) -> SqlResult<Self> {
        validate_table_name(table)?;
        Ok(SqlQuery {
            conn,
            sql_builder: SqlBuilder::select_from(table),
            params: SqlParamsBuilder::default(),
            ordering: Vec::default(),
        })
    }

    /// Create SELECT query.
    /// The method takes the `alias` of the `table`.
    ///
    /// Please note the function validates the given `table` and `alias` names.
    pub fn select_from_alias(conn: &'a Connection, table: &str, alias: &'static str) -> SqlResult<Self> {
        validate_table_name(table)?;
        validate_table_name(alias)?;
        Ok(SqlQuery {
            conn,
            sql_builder: SqlBuilder::select_from(format!("{} {}", table, alias)),
            params: SqlParamsBuilder::default(),
            ordering: Vec::default(),
        })
    }

    /// Create SELECT query.
    /// Please note that [`SqlQuery::ordering`] is not inherited by the external query.
    pub fn select_from_subquery(subquery: SqlSubquery<'a>) -> SqlResult<Self> {
        let subquery_sql = subquery
            .0
            .sql_builder
            .subquery()
            .map_err(|e| SqlError::ToSqlConversionFailure(e.into()))?;
        Ok(SqlQuery {
            conn: subquery.0.conn,
            sql_builder: SqlBuilder::select_from(subquery_sql),
            params: subquery.0.params,
            ordering: Vec::default(),
        })
    }

    /// Add COUNT(field).
    /// For more details see [`SqlBuilder::count`].
    #[inline]
    pub fn count<S: ToString>(&mut self, field: S) -> &mut Self {
        self.sql_builder.count(field);
        self
    }

    /// Add field.
    /// For more details see [`SqlBuilder::field`].
    #[inline]
    pub fn field<S: ToString>(&mut self, field: S) -> &mut Self {
        self.sql_builder.field(field);
        self
    }

    /// Set OFFSET.
    /// For more details see [`SqlBuilder::offset`].
    #[inline]
    pub fn offset<S: ToString>(&mut self, offset: S) -> &mut Self {
        self.sql_builder.offset(offset);
        self
    }

    /// Set LIMIT.
    /// For more details see [`SqlBuilder::limit`].
    #[inline]
    pub fn limit<S: ToString>(&mut self, limit: S) -> &mut Self {
        self.sql_builder.limit(limit);
        self
    }

    /// Add GROUP BY part.
    /// For more details see [`SqlBuilder::group_by`].
    #[inline]
    pub fn group_by<S: ToString>(&mut self, field: S) -> &mut Self {
        self.sql_builder.group_by(field);
        self
    }

    /// Add ORDER BY ASC.
    /// For more details see [`SqlBuilder::order_asc`].
    #[inline]
    pub fn order_asc<S: ToString>(&mut self, field: S) -> &mut Self {
        self.ordering.push(SqlOrdering::Asc(field.to_string()));
        self
    }

    /// Add ORDER BY DESC.
    /// For more details see [`SqlBuilder::order_desc`].
    #[inline]
    pub fn order_desc<S: ToString>(&mut self, field: S) -> &mut Self {
        self.ordering.push(SqlOrdering::Desc(field.to_string()));
        self
    }

    /// Join the given `table` and gives it the `alias`.
    ///
    /// Please note the function validates the given `table` name.
    /// For more details see [`SqlBuilder::join`].
    #[inline]
    pub fn join_alias(&mut self, table: &str, alias: &'static str) -> SqlResult<&mut Self> {
        validate_table_name(table)?;
        validate_table_name(alias)?;
        self.sql_builder.join(format!("{} {}", table, alias));
        Ok(self)
    }

    /// Join constraint to the last JOIN part.
    ///
    /// Please note the function validates the given `table` name.
    /// For more details see [`SqlBuilder::join`].
    #[inline]
    pub fn join(&mut self, table: &str) -> SqlResult<&mut Self> {
        validate_table_name(table)?;
        self.sql_builder.join(table);
        Ok(self)
    }

    /// Join constraint to the last JOIN part [`SqlQuery::join`].
    /// For more details see [`SqlBuilder::on_eq`].
    #[inline]
    pub fn on_eq<C1, C2>(&mut self, c1: C1, c2: C2) -> &mut Self
    where
        C1: ToString,
        C2: ToString,
    {
        self.sql_builder.on_eq(c1, c2);
        self
    }

    /// Add WHERE condition for equal parts.
    /// For more details see [`SqlBuilder::and_where_eq`].
    #[inline]
    pub fn and_where_eq<S, T>(&mut self, field: S, value: T) -> &mut Self
    where
        S: ToString,
        T: ToString,
    {
        self.sql_builder.and_where_eq(field, value);
        self
    }

    /// Add WHERE condition for equal parts.
    /// For more details see [`SqlBuilder::and_where_eq`].
    #[inline]
    pub fn and_where_eq_param<S, T>(&mut self, field: S, param: T) -> &mut Self
    where
        S: ToString,
        OwnedSqlParam: From<T>,
    {
        self.sql_builder.and_where_eq(field, self.params.push_param(param));
        self
    }

    /// Add WHERE field IN (list).
    /// For more details see [`SqlBuilder::and_where_in`].
    #[inline]
    pub fn and_where_in<S, T>(&mut self, field: S, list: &[T]) -> &mut Self
    where
        S: ToString,
        T: ToString,
    {
        self.sql_builder.and_where_in(field, list);
        self
    }

    /// Add WHERE field IN (string list).
    /// For more details see [`SqlBuilder::and_where_in_quoted`].
    #[inline]
    pub fn and_where_in_quoted<S, T>(&mut self, field: S, list: &[T]) -> &mut Self
    where
        S: ToString,
        T: ToString,
    {
        self.sql_builder.and_where_in_quoted(field, list);
        self
    }

    /// Add WHERE field IN (string list) with the specified `params`.
    /// For more details see [`SqlBuilder::and_where_in_quoted`].
    #[inline]
    pub fn and_where_in_params<S, I, P>(&mut self, field: S, params: I) -> &mut Self
    where
        S: ToString,
        I: IntoIterator<Item = P>,
        OwnedSqlParam: From<P>,
    {
        self.sql_builder.and_where_in(field, &self.params.push_params(params));
        self
    }

    /// Add OR condition of equal parts to the last WHERE condition.
    /// For more details see [`SqlBuilder::or_where_eq`].
    #[inline]
    pub fn or_where_eq<S, T>(&mut self, field: S, value: T) -> &mut Self
    where
        S: ToString,
        T: ToString,
    {
        self.sql_builder.or_where_eq(field, value);
        self
    }

    /// Add OR condition of equal parts to the last WHERE condition.
    /// For more details see [`SqlBuilder::or_where_eq`].
    #[inline]
    pub fn or_where_eq_param<S, T>(&mut self, field: S, param: T) -> &mut Self
    where
        S: ToString,
        OwnedSqlParam: From<T>,
    {
        self.sql_builder.or_where_eq(field, self.params.push_param(param));
        self
    }

    /// Add OR field IN (list) to the last WHERE condition.
    /// For more details see [`SqlBuilder::or_where_in`].
    #[inline]
    pub fn or_where_in<S, T>(&mut self, field: S, list: &[T]) -> &mut Self
    where
        S: ToString,
        T: ToString,
    {
        self.sql_builder.or_where_in(field, list);
        self
    }

    /// Add OR field IN (string list) to the last WHERE condition.
    /// For more details see [`SqlBuilder::and_where_in_quoted`].
    #[inline]
    pub fn or_where_in_quoted<S, T>(&mut self, field: S, list: &[T]) -> &mut Self
    where
        S: ToString,
        T: ToString,
    {
        self.sql_builder.or_where_in_quoted(field, list);
        self
    }

    /// Add OR field IN (list) to the last WHERE condition.
    /// For more details see [`SqlBuilder::or_where_in_quoted`].
    #[inline]
    pub fn or_where_in_params<S, I, P>(&mut self, field: S, params: I) -> &mut Self
    where
        S: ToString,
        I: IntoIterator<Item = P>,
        OwnedSqlParam: From<P>,
    {
        self.sql_builder.or_where_in(field, &self.params.push_params(params));
        self
    }

    #[inline]
    pub fn sql(mut self) -> SqlResult<String> {
        self.apply_ordering();
        self.sql_builder
            .sql()
            .map_err(|e| SqlError::ToSqlConversionFailure(e.into()))
    }

    /// Returns an SQL subquery that can be used in [`SqlQuery::select_from_subquery`].
    #[inline]
    pub fn subquery(self) -> SqlSubquery<'a> { SqlSubquery(self) }

    /// Returns the reference to the specified SQL parameters.
    #[inline]
    pub fn params(&self) -> &OwnedSqlParams { self.params.params() }

    /// # Usage
    ///
    /// 1) Create a `SqlQuery` instance;
    /// 2) Join tables, specify `WHERE`, `ORDER`, `GROUP BY` constraints;
    /// 3) Create a copy of the instance;
    /// 4) Call [`SqlQuery::offset_by_id`] method on the copy instance
    ///    to get an offset from which you need to query rows from the storage;
    /// 5) Specify the returned `offset` value by [`SqlQuery::offset`] on the original instance;
    /// 6) Query rows by [`SqlQuery::query`].
    ///
    /// # Note
    ///
    /// 1) It's recommended not to specify fields by [`SqlQuery::field`] before [`SqlQuery::offset_by_id`] is used;
    /// 2) Don't specify any `WHERE` constraint, ordering on the original `SqlQuery` instance
    ///    after [`SqlQuery::offset_by_id`] is called.
    #[inline]
    pub fn query_offset_by_id<T>(mut self, id_field: &str, where_id_eq_param: T) -> SqlResult<Option<usize>>
    where
        OwnedSqlParam: From<T>,
    {
        /// The alias is needed so that the external query can access the results of the subquery.
        /// Example:
        ///   SUBQUERY: `SELECT ROW_NUMBER() OVER (ORDER BY h.height ASC, h.total_amount DESC) AS row, h.tx_hash as __ID_FIELD FROM tx_history h JOIN tx_address a ON h.tx_hash = a.tx_hash WHERE a.address IN ('address_2', 'address_4') GROUP BY h.tx_hash`
        ///   EXTERNAL_QUERY: `SELECT row FROM (<SUBQUERY>) WHERE __ID_FIELD = :1;`
        /// Here we can't use `id_field = "h.tx_hash"` in the external query because it doesn't know about the `tx_history h` table.
        /// So we need to give the `id_field` an alias like `__ID_FIELD`.
        const ID_FIELD_ALIAS: &str = "__ID_FIELD";

        if self.ordering.is_empty() {
            let error = "SQL ORDERs must be specified before `SqlQuery::query_offset_by_id` is called";
            return Err(SqlError::ToSqlConversionFailure(StringError::from(error).into_boxed()));
        }

        let order_by = self
            .ordering
            .iter()
            .map(SqlOrdering::to_sql)
            .collect::<Vec<_>>()
            .join(", ");
        self
            // Query the number of the row with the specified `order_by` ordering.
            .field(format!("ROW_NUMBER() OVER (ORDER BY {}) AS row", order_by))
            // Query `id_field` and give it the `__ID_FIELD` alias.
            .field(format!("{} as {}", id_field, ID_FIELD_ALIAS));

        let mut external_query = SqlQuery::select_from_subquery(self.subquery())?;
        external_query
            .field("row")
            .and_where_eq_param(ID_FIELD_ALIAS, where_id_eq_param);
        Ok(external_query
            .query_single_row(|row| row.get::<_, isize>(0))?
            .map(|offset| offset.try_into().expect("row index should be always above zero")))
    }

    /// Convenience method to execute a query that is expected to return mapped rows.
    /// For more details see [`SqlBuilder::query_row`].
    pub fn query<F, B>(mut self, f: F) -> SqlResult<Vec<B>>
    where
        F: FnMut(&Row<'_>) -> SqlResult<B>,
    {
        self.apply_ordering();
        let sql = self
            .sql_builder
            .sql()
            .map_err(|e| SqlError::ToSqlConversionFailure(e.into()))?;

        debug!("Trying to execute SQL query {} with params {:?}", sql, self.params());
        let mut stmt = self.conn.prepare(&sql)?;
        let items = stmt.query_map(self.params(), f)?.collect::<SqlResult<Vec<_>>>()?;
        // Otherwise, we'll get the compile error:
        // `stmt` does not live long enough
        Ok(items)
    }

    /// Convenience method to execute a query that is expected to return a single row.
    /// For more details see [`SqlBuilder::query_row`].
    pub fn query_single_row<F, T>(mut self, f: F) -> SqlResult<Option<T>>
    where
        F: FnOnce(&Row<'_>) -> SqlResult<T>,
    {
        self.apply_ordering();
        let sql = self
            .sql_builder
            .sql()
            .map_err(|e| SqlError::ToSqlConversionFailure(e.into()))?;
        debug!("Trying to execute SQL query {} with params {:?}", sql, self.params());
        query_single_row(self.conn, &sql, self.params(), f)
    }

    /// Applies [`SqlQuery::ordering`] to [`SqlQuery::sql_builder`].
    /// Please note the functions clears [`SqlQuery::ordering`].
    fn apply_ordering(&mut self) {
        for order in self.ordering.drain(..) {
            match order {
                SqlOrdering::Asc(column) => self.sql_builder.order_asc(column),
                SqlOrdering::Desc(column) => self.sql_builder.order_desc(column),
            };
        }
    }
}

/// An instance of this structure is returned by [`SqlQuery::subquery`].
pub struct SqlSubquery<'a>(SqlQuery<'a>);

#[derive(Clone)]
enum SqlOrdering {
    Asc(String),
    Desc(String),
}

impl SqlOrdering {
    fn to_sql(&self) -> String {
        match self {
            SqlOrdering::Asc(column) => format!("{} ASC", column),
            SqlOrdering::Desc(column) => format!("{} DESC", column),
        }
    }
}

#[derive(Debug)]
struct StringError(String);

impl fmt::Display for StringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.0) }
}

impl StdError for StringError {}

impl From<&'static str> for StringError {
    fn from(s: &str) -> Self { StringError(s.to_owned()) }
}

impl StringError {
    fn into_boxed(self) -> Box<StringError> { Box::new(self) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::NO_PARAMS;

    const CREATE_TX_HISTORY_TABLE: &str = "CREATE TABLE tx_history (
        tx_hash VARCHAR(255) NOT NULL UNIQUE,
        height INTEGER NOT NULL,
        total_amount INTEGER NOT NULL
    );";
    const CREATE_TX_ADDRESS_TABLE: &str = "CREATE TABLE tx_address (
        tx_hash VARCHAR(255) NOT NULL,
        address VARCHAR(255) NOT NULL
    );";
    const INSERT_TX_TO_HISTORY_TABLE: &str =
        "INSERT INTO tx_history (tx_hash, height, total_amount) VALUES (?1, ?2, ?3)";
    const INSERT_TX_TO_ADDRESS_TABLE: &str = "INSERT INTO tx_address (tx_hash, address) VALUES (?1, ?2)";

    fn init_table_for_test(conn: &Connection) {
        conn.execute(CREATE_TX_HISTORY_TABLE, NO_PARAMS).unwrap();
        conn.execute(CREATE_TX_ADDRESS_TABLE, NO_PARAMS).unwrap();

        let history_items = vec![
            ("tx_hash_1", 699545, 23),
            ("tx_hash_2", 699547, 10),
            ("tx_hash_3", 699547, 11),
            ("tx_hash_4", 699530, 100),
            ("tx_hash_5", 699532, 19),
        ];
        for (tx_hash, height, total_amount) in history_items {
            conn.execute(INSERT_TX_TO_HISTORY_TABLE, &[
                tx_hash.to_owned(),
                height.to_string(),
                total_amount.to_string(),
            ])
            .unwrap();
        }

        let address_table_items = vec![
            ("tx_hash_1", "address_1"),
            ("tx_hash_1", "address_2"),
            ("tx_hash_2", "address_1"),
            ("tx_hash_2", "address_3"),
            ("tx_hash_2", "address_4"),
            ("tx_hash_3", "address_3"),
            ("tx_hash_4", "address_2"),
            ("tx_hash_4", "address_4"),
            ("tx_hash_5", "address_1"),
        ];
        for (tx_hash, address) in address_table_items {
            conn.execute(INSERT_TX_TO_ADDRESS_TABLE, &[tx_hash.to_owned(), address.to_string()])
                .unwrap();
        }
    }

    #[test]
    fn test_query_join() {
        const SEARCHING_ADDRESSES: [&str; 2] = ["address_2", "address_4"];

        let conn = Connection::open_in_memory().unwrap();
        init_table_for_test(&conn);

        let mut query = SqlQuery::select_from(&conn, "tx_history").unwrap();
        query
            .field("tx_history.tx_hash")
            .join("tx_address")
            .unwrap()
            .on_eq("tx_history.tx_hash", "tx_address.tx_hash")
            .and_where_in_quoted("tx_address.address", &SEARCHING_ADDRESSES)
            .group_by("tx_history.tx_hash");
        assert_eq!(
            query.clone().sql().unwrap(),
            "SELECT tx_history.tx_hash FROM tx_history JOIN tx_address ON tx_history.tx_hash = tx_address.tx_hash WHERE tx_address.address IN ('address_2', 'address_4') GROUP BY tx_history.tx_hash;"
        );
        let actual: Vec<String> = query.query(|row| row.get(0)).unwrap();
        let expected = vec!["tx_hash_1".to_owned(), "tx_hash_2".to_owned(), "tx_hash_4".to_owned()];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_query_where_in_params() {
        const SEARCHING_HEIGHTS: [u32; 3] = [699547, 699530, 699533];

        let conn = Connection::open_in_memory().unwrap();
        init_table_for_test(&conn);

        let mut query = SqlQuery::select_from(&conn, "tx_history").unwrap();
        query
            .field("tx_hash")
            .and_where_in_params("height", SEARCHING_HEIGHTS.clone());
        assert_eq!(
            query.clone().sql().unwrap(),
            "SELECT tx_hash FROM tx_history WHERE height IN (:1, :2, :3);"
        );
        assert_eq!(query.params(), &vec![699547.into(), 699530.into(), 699533.into()]);

        let actual: Vec<String> = query.query(|row| row.get(0)).unwrap();
        let expected = vec!["tx_hash_2".to_owned(), "tx_hash_3".to_owned(), "tx_hash_4".to_owned()];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_query_offset_by_id() {
        let conn = Connection::open_in_memory().unwrap();
        init_table_for_test(&conn);

        let mut query = SqlQuery::select_from(&conn, "tx_history").unwrap();
        query.order_asc("height");
        query.order_desc("total_amount");

        // Resulting rows:
        // 1) tx_hash="tx_hash_4", height=699530, total_amount=100
        // 2) tx_hash="tx_hash_5", height=699532, total_amount=19
        // 3) tx_hash="tx_hash_1", height=699545, total_amount=23
        // 4) tx_hash="tx_hash_3", height=699547, total_amount=11
        // 5) tx_hash="tx_hash_2", height=699547, total_amount=10

        let actual = query
            .clone()
            .query_offset_by_id("tx_hash", "tx_hash_4".to_owned())
            .unwrap();
        assert_eq!(actual, Some(1));

        let actual = query
            .clone()
            .query_offset_by_id("tx_hash", "tx_hash_3".to_owned())
            .unwrap();
        assert_eq!(actual, Some(4));

        let actual = query
            .clone()
            .query_offset_by_id("tx_hash", "tx_hash_2".to_owned())
            .unwrap();
        assert_eq!(actual, Some(5));

        let actual = query.query_offset_by_id("tx_hash", "tx_hash_6".to_owned()).unwrap();
        assert_eq!(actual, None);
    }

    #[test]
    fn test_query_offset_by_id_with_join() {
        const SEARCHING_ADDRESSES: [&str; 2] = ["address_2", "address_4"];

        let conn = Connection::open_in_memory().unwrap();
        init_table_for_test(&conn);

        let mut query = SqlQuery::select_from_alias(&conn, "tx_history", "h").unwrap();
        query
            .order_asc("h.height")
            .order_desc("h.total_amount")
            .join_alias("tx_address", "a")
            .unwrap()
            .on_eq("h.tx_hash", "a.tx_hash")
            .and_where_in_quoted("a.address", &SEARCHING_ADDRESSES)
            .group_by("h.tx_hash");

        // Resulting rows:
        // 1) tx_hash="tx_hash_4", height=699530, total_amount=100
        // 2) tx_hash="tx_hash_1", height=699545, total_amount=23
        // 3) tx_hash="tx_hash_2", height=699547, total_amount=10

        let actual = query
            .clone()
            .query_offset_by_id("h.tx_hash", "tx_hash_4".to_owned())
            .unwrap();
        assert_eq!(actual, Some(1));

        let actual = query
            .clone()
            .query_offset_by_id("h.tx_hash", "tx_hash_1".to_owned())
            .unwrap();
        assert_eq!(actual, Some(2));

        let actual = query
            .clone()
            .query_offset_by_id("h.tx_hash", "tx_hash_2".to_owned())
            .unwrap();
        assert_eq!(actual, Some(3));

        let actual = query.query_offset_by_id("h.tx_hash", "tx_hash_3".to_owned()).unwrap();
        assert_eq!(actual, None);
    }
}
