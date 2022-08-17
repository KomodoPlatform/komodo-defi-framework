use common::fmt::WriteSafe;
use common::log::debug;
use common::write_safe;
use rusqlite::{Connection, Result as SqlResult, NO_PARAMS};

/// A `DELETE` SQL request builder.
pub struct SqlDelete<'a> {
    conn: &'a Connection,
    table_name: &'static str,
}

/// TODO extend the builder with `and_where_eq` etc.
impl<'a> SqlDelete<'a> {
    const DELETE_CAPACITY: usize = 200;

    /// Creates a new instance of `SqlDelete` builder.
    #[inline]
    pub fn new(conn: &'a Connection, table_name: &'static str) -> Self { SqlDelete { conn, table_name } }

    /// TODO please note `SqlDelete` deletes **ALL** records from the table.
    pub fn delete(self) -> SqlResult<()> {
        let sql = self.sql()?;

        debug!("Trying to execute SQL query {}", sql);
        self.conn.execute(&sql, NO_PARAMS)?;

        Ok(())
    }

    pub fn sql(&self) -> SqlResult<String> {
        let mut sql = String::with_capacity(Self::DELETE_CAPACITY);

        write_safe!(sql, "DELETE FROM {};", self.table_name);

        Ok(sql)
    }
}
