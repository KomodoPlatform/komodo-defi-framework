use crate::my_tx_history_v2::{GetHistoryResult, RemoveTxResult, TxHistoryStorage, TxHistoryStorageError};
use crate::tx_history_storage::{token_id_from_tx_type, ConfirmationStatus, CreateTxHistoryStorageError,
                                FilteringAddresses, GetTxHistoryFilters, WalletId};
use crate::TransactionDetails;
use async_trait::async_trait;
use common::{async_blocking, PagingOptionsEnum};
use db_common::sql_query::SqlQuery;
use db_common::sqlite::rusqlite::types::Type;
use db_common::sqlite::rusqlite::{Connection, Error as SqlError, Row, NO_PARAMS};
use db_common::sqlite::{query_single_row, string_from_row, validate_table_name, CHECK_TABLE_EXISTS_SQL};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use rpc::v1::types::Bytes as BytesJson;
use serde_json::{self as json};
use std::convert::TryInto;
use std::sync::{Arc, Mutex};

fn tx_history_table(wallet_id: &WalletId) -> String { wallet_id.to_sql_table_name() + "_tx_history" }

fn tx_address_table(wallet_id: &WalletId) -> String { wallet_id.to_sql_table_name() + "_tx_address" }

/// Please note TX cache table name doesn't depend on [`WalletId::hd_wallet_rmd160`].
fn tx_cache_table(wallet_id: &WalletId) -> String { format!("{}_tx_cache", wallet_id.ticker) }

fn create_tx_history_table_sql(wallet_id: &WalletId) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(wallet_id);
    validate_table_name(&table_name)?;

    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (
            id INTEGER NOT NULL PRIMARY KEY,
            tx_hash VARCHAR(255) NOT NULL,
            internal_id VARCHAR(255) NOT NULL UNIQUE,
            block_height INTEGER NOT NULL,
            confirmation_status INTEGER NOT NULL,
            token_id VARCHAR(255) NOT NULL,
            details_json TEXT
        );",
        table_name
    );

    Ok(sql)
}

fn create_tx_address_table_sql(wallet_id: &WalletId) -> Result<String, MmError<SqlError>> {
    let tx_address_table = tx_address_table(wallet_id);
    validate_table_name(&tx_address_table)?;

    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (
            id INTEGER NOT NULL PRIMARY KEY,
            internal_id VARCHAR(255) NOT NULL,
            address TEXT NOT NULL
        );",
        tx_address_table
    );

    Ok(sql)
}

fn create_tx_cache_table_sql(wallet_id: &WalletId) -> Result<String, MmError<SqlError>> {
    let table_name = tx_cache_table(wallet_id);
    validate_table_name(&table_name)?;

    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (
            tx_hash VARCHAR(255) NOT NULL UNIQUE,
            tx_hex TEXT NOT NULL
        );",
        table_name
    );

    Ok(sql)
}

fn create_internal_id_index_sql<F>(wallet_id: &WalletId, table_name_creator: F) -> Result<String, MmError<SqlError>>
where
    F: FnOnce(&WalletId) -> String,
{
    let table_name = table_name_creator(wallet_id);
    validate_table_name(&table_name)?;

    let sql = format!(
        "CREATE INDEX IF NOT EXISTS internal_id_idx ON {} (internal_id);",
        table_name
    );
    Ok(sql)
}

fn insert_tx_in_history_sql(wallet_id: &WalletId) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(wallet_id);
    validate_table_name(&table_name)?;

    let sql = format!(
        "INSERT INTO {} (
            tx_hash,
            internal_id,
            block_height,
            confirmation_status,
            token_id,
            details_json
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6
        );",
        table_name
    );

    Ok(sql)
}

fn insert_tx_address_sql(wallet_id: &WalletId) -> Result<String, MmError<SqlError>> {
    let table_name = tx_address_table(wallet_id);
    validate_table_name(&table_name)?;

    let sql = format!(
        "INSERT INTO {} (
            internal_id,
            address
        ) VALUES (?1, ?2);",
        table_name
    );

    Ok(sql)
}

fn insert_tx_in_cache_sql(wallet_id: &WalletId) -> Result<String, MmError<SqlError>> {
    let table_name = tx_cache_table(wallet_id);
    validate_table_name(&table_name)?;

    // We can simply ignore the repetitive attempt to insert the same tx_hash
    let sql = format!(
        "INSERT OR IGNORE INTO {} (tx_hash, tx_hex) VALUES (?1, ?2);",
        table_name
    );

    Ok(sql)
}

fn remove_tx_by_internal_id_sql<F>(wallet_id: &WalletId, table_name_creator: F) -> Result<String, MmError<SqlError>>
where
    F: FnOnce(&WalletId) -> String,
{
    let table_name = table_name_creator(wallet_id);
    validate_table_name(&table_name)?;
    let sql = format!("DELETE FROM {} WHERE internal_id=?1;", table_name);
    Ok(sql)
}

fn select_tx_by_internal_id_sql(wallet_id: &WalletId) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(wallet_id);
    validate_table_name(&table_name)?;

    let sql = format!("SELECT details_json FROM {} WHERE internal_id=?1;", table_name);

    Ok(sql)
}

fn update_tx_in_table_by_internal_id_sql(wallet_id: &WalletId) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(wallet_id);
    validate_table_name(&table_name)?;

    let sql = format!(
        "UPDATE {} SET
            block_height = ?1,
            confirmation_status = ?2,
            details_json = ?3
        WHERE
            internal_id=?4;",
        table_name
    );

    Ok(sql)
}

fn contains_unconfirmed_transactions_sql(wallet_id: &WalletId) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(wallet_id);
    validate_table_name(&table_name)?;

    let sql = format!(
        "SELECT COUNT(id) FROM {} WHERE confirmation_status = {};",
        table_name,
        ConfirmationStatus::Unconfirmed.to_sql_param()
    );

    Ok(sql)
}

fn get_unconfirmed_transactions_sql(wallet_id: &WalletId) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(wallet_id);
    validate_table_name(&table_name)?;

    let sql = format!(
        "SELECT details_json FROM {} WHERE confirmation_status = {};",
        table_name,
        ConfirmationStatus::Unconfirmed.to_sql_param()
    );

    Ok(sql)
}

fn has_transactions_with_hash_sql(wallet_id: &WalletId) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(wallet_id);
    validate_table_name(&table_name)?;

    let sql = format!("SELECT COUNT(id) FROM {} WHERE tx_hash = ?1;", table_name);

    Ok(sql)
}

fn unique_tx_hashes_num_sql(wallet_id: &WalletId) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(wallet_id);
    validate_table_name(&table_name)?;

    let sql = format!("SELECT COUNT(DISTINCT tx_hash) FROM {};", table_name);

    Ok(sql)
}

fn get_tx_hex_from_cache_sql(wallet_id: &WalletId) -> Result<String, MmError<SqlError>> {
    let table_name = tx_cache_table(wallet_id);
    validate_table_name(&table_name)?;

    let sql = format!("SELECT tx_hex FROM {} WHERE tx_hash = ?1 LIMIT 1;", table_name);

    Ok(sql)
}

/// Creates an `SqlQuery` instance with the required `WHERE`, `ORDER`, `GROUP_BY` constraints.
/// Please note you can refer to the [`tx_history_table(wallet_id)`] table by the `tx_history` alias.
fn get_history_builder_preimage<'a>(
    connection: &'a Connection,
    wallet_id: &WalletId,
    token_id: String,
    for_addresses: Option<FilteringAddresses>,
) -> Result<SqlQuery<'a>, MmError<SqlError>> {
    let mut sql_builder = SqlQuery::select_from_alias(connection, &tx_history_table(wallet_id), "tx_history")?;

    // Check if we need to join the [`tx_address_table(wallet_id)`] table
    // to query transactions that were sent from/to `for_addresses` addresses.
    if let Some(for_addresses) = for_addresses {
        let tx_address_table_name = tx_address_table(wallet_id);

        sql_builder
            .join_alias(&tx_address_table_name, "tx_address")?
            .on_join_eq("tx_history.internal_id", "tx_address.internal_id")?;

        sql_builder
            .and_where_in_params("tx_address.address", for_addresses)?
            .group_by("tx_history.internal_id")?;
    }

    sql_builder
        .and_where_eq_param("tx_history.token_id", token_id)?
        .order_asc("tx_history.confirmation_status")?
        .order_desc("tx_history.block_height")?
        .order_asc("tx_history.id")?;
    Ok(sql_builder)
}

fn finalize_get_total_count_sql_builder(mut subquery: SqlQuery<'_>) -> Result<SqlQuery<'_>, MmError<SqlError>> {
    /// The alias is needed so that the external query can access the results of the subquery.
    /// Example:
    ///   SUBQUERY: `SELECT h.internal_id AS __INTERNAL_ID_ALIAS FROM tx_history h JOIN tx_address a ON h.internal_id = a.internal_id WHERE a.address IN ('address_2', 'address_4') GROUP BY h.internal_id`
    ///   EXTERNAL_QUERY: `SELECT COUNT(__INTERNAL_ID_ALIAS) FROM (<SUBQUERY>);`
    /// Here we can't use `h.internal_id` in the external query because it doesn't know about the `tx_history h` table.
    /// So we need to give the `h.internal_id` an alias like `__INTERNAL_ID_ALIAS`.
    const INTERNAL_ID_ALIAS: &str = "__INTERNAL_ID_ALIAS";

    // Query `id_field` and give it the `__ID_FIELD` alias.
    subquery.field_alias("tx_history.internal_id", INTERNAL_ID_ALIAS)?;

    let mut external_query = SqlQuery::select_from_subquery(subquery.subquery())?;
    external_query.count(INTERNAL_ID_ALIAS)?;
    Ok(external_query)
}

fn finalize_get_history_sql_builder(sql_builder: &mut SqlQuery, offset: usize, limit: usize) -> Result<(), SqlError> {
    sql_builder
        .field("tx_history.details_json")?
        .offset(offset)
        .limit(limit);
    Ok(())
}

fn tx_details_from_row(row: &Row<'_>) -> Result<TransactionDetails, SqlError> {
    let json_string: String = row.get(0)?;
    json::from_str(&json_string).map_err(|e| SqlError::FromSqlConversionFailure(0, Type::Text, Box::new(e)))
}

impl TxHistoryStorageError for SqlError {}

impl ConfirmationStatus {
    fn to_sql_param(self) -> String { (self as u8).to_string() }
}

impl WalletId {
    fn to_sql_table_name(&self) -> String {
        match self.hd_wallet_rmd160 {
            Some(hd_wallet_rmd160) => format!("{}_{}", self.ticker, hd_wallet_rmd160),
            None => self.ticker.clone(),
        }
    }
}

#[derive(Clone)]
pub struct SqliteTxHistoryStorage(Arc<Mutex<Connection>>);

impl SqliteTxHistoryStorage {
    pub fn new(ctx: &MmArc) -> Result<Self, MmError<CreateTxHistoryStorageError>> {
        let sqlite_connection = ctx
            .sqlite_connection
            .ok_or(MmError::new(CreateTxHistoryStorageError::Internal(
                "sqlite_connection is not initialized".to_owned(),
            )))?;
        Ok(SqliteTxHistoryStorage(sqlite_connection.clone()))
    }
}

#[async_trait]
impl TxHistoryStorage for SqliteTxHistoryStorage {
    type Error = SqlError;

    async fn init(&self, wallet_id: &WalletId) -> Result<(), MmError<Self::Error>> {
        let selfi = self.clone();

        let sql_history = create_tx_history_table_sql(wallet_id)?;
        let sql_cache = create_tx_cache_table_sql(wallet_id)?;
        let sql_addr = create_tx_address_table_sql(wallet_id)?;

        let sql_history_index = create_internal_id_index_sql(wallet_id, tx_history_table)?;
        let sql_addr_index = create_internal_id_index_sql(wallet_id, tx_address_table)?;

        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();

            conn.execute(&sql_history, NO_PARAMS).map(|_| ())?;
            conn.execute(&sql_addr, NO_PARAMS).map(|_| ())?;
            conn.execute(&sql_cache, NO_PARAMS).map(|_| ())?;

            conn.execute(&sql_history_index, NO_PARAMS).map(|_| ())?;
            conn.execute(&sql_addr_index, NO_PARAMS).map(|_| ())?;
            Ok(())
        })
        .await
    }

    async fn is_initialized_for(&self, wallet_id: &WalletId) -> Result<bool, MmError<Self::Error>> {
        let tx_history_table = tx_history_table(wallet_id);
        validate_table_name(&tx_history_table)?;

        let tx_cache_table = tx_cache_table(wallet_id);
        validate_table_name(&tx_cache_table)?;

        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let history_initialized =
                query_single_row(&conn, CHECK_TABLE_EXISTS_SQL, [tx_history_table], string_from_row)?;
            let cache_initialized = query_single_row(&conn, CHECK_TABLE_EXISTS_SQL, [tx_cache_table], string_from_row)?;
            Ok(history_initialized.is_some() && cache_initialized.is_some())
        })
        .await
    }

    async fn add_transactions_to_history<I>(
        &self,
        wallet_id: &WalletId,
        transactions: I,
    ) -> Result<(), MmError<Self::Error>>
    where
        I: IntoIterator<Item = TransactionDetails> + Send + 'static,
        I::IntoIter: Send,
    {
        let selfi = self.clone();
        let wallet_id = wallet_id.clone();
        async_blocking(move || {
            let mut conn = selfi.0.lock().unwrap();
            let sql_transaction = conn.transaction()?;

            for tx in transactions {
                let tx_hash = tx.tx_hash.clone();
                let internal_id = format!("{:02x}", tx.internal_id);
                let confirmation_status = ConfirmationStatus::from_block_height(tx.block_height);
                let token_id = token_id_from_tx_type(&tx.transaction_type);
                let tx_json = json::to_string(&tx).expect("serialization should not fail");

                let tx_hex = format!("{:02x}", tx.tx_hex);
                let tx_cache_params = [&tx_hash, &tx_hex];

                sql_transaction.execute(&insert_tx_in_cache_sql(&wallet_id)?, tx_cache_params)?;

                let params = [
                    tx_hash,
                    internal_id.clone(),
                    tx.block_height.to_string(),
                    confirmation_status.to_sql_param(),
                    token_id,
                    tx_json,
                ];
                sql_transaction.execute(&insert_tx_in_history_sql(&wallet_id)?, &params)?;

                let addresses: FilteringAddresses = tx.from.into_iter().chain(tx.to.into_iter()).collect();
                for address in addresses {
                    let params = [internal_id.clone(), address];
                    sql_transaction.execute(&insert_tx_address_sql(&wallet_id)?, &params)?;
                }
            }
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn remove_tx_from_history(
        &self,
        wallet_id: &WalletId,
        internal_id: &BytesJson,
    ) -> Result<RemoveTxResult, MmError<Self::Error>> {
        let remove_tx_history_sql = remove_tx_by_internal_id_sql(wallet_id, tx_history_table)?;
        let remove_tx_addr_sql = remove_tx_by_internal_id_sql(wallet_id, tx_address_table)?;

        let params = [format!("{:02x}", internal_id)];
        let selfi = self.clone();

        async_blocking(move || {
            let mut conn = selfi.0.lock().unwrap();
            let sql_transaction = conn.transaction()?;

            sql_transaction.execute(&remove_tx_addr_sql, &params)?;

            let rows_num = sql_transaction.execute(&remove_tx_history_sql, &params)?;
            let remove_tx_result = if rows_num > 0 {
                RemoveTxResult::TxRemoved
            } else {
                RemoveTxResult::TxDidNotExist
            };

            sql_transaction.commit()?;
            Ok(remove_tx_result)
        })
        .await
    }

    async fn get_tx_from_history(
        &self,
        wallet_id: &WalletId,
        internal_id: &BytesJson,
    ) -> Result<Option<TransactionDetails>, MmError<Self::Error>> {
        let params = [format!("{:02x}", internal_id)];
        let sql = select_tx_by_internal_id_sql(wallet_id)?;
        let selfi = self.clone();

        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            query_single_row(&conn, &sql, params, tx_details_from_row).map_to_mm(SqlError::from)
        })
        .await
    }

    async fn history_contains_unconfirmed_txes(&self, wallet_id: &WalletId) -> Result<bool, MmError<Self::Error>> {
        let sql = contains_unconfirmed_transactions_sql(wallet_id)?;
        let selfi = self.clone();

        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let count_unconfirmed = conn.query_row::<u32, _, _>(&sql, NO_PARAMS, |row| row.get(0))?;
            Ok(count_unconfirmed > 0)
        })
        .await
    }

    async fn get_unconfirmed_txes_from_history(
        &self,
        wallet_id: &WalletId,
    ) -> Result<Vec<TransactionDetails>, MmError<Self::Error>> {
        let sql = get_unconfirmed_transactions_sql(wallet_id)?;
        let selfi = self.clone();

        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query(NO_PARAMS)?;
            let result = rows.mapped(tx_details_from_row).collect::<Result<_, _>>()?;
            Ok(result)
        })
        .await
    }

    async fn update_tx_in_history(
        &self,
        wallet_id: &WalletId,
        tx: &TransactionDetails,
    ) -> Result<(), MmError<Self::Error>> {
        let sql = update_tx_in_table_by_internal_id_sql(wallet_id)?;

        let block_height = tx.block_height.to_string();
        let confirmation_status = ConfirmationStatus::from_block_height(tx.block_height);
        let json_details = json::to_string(tx).unwrap();
        let internal_id = format!("{:02x}", tx.internal_id);

        let params = [
            block_height,
            confirmation_status.to_sql_param(),
            json_details,
            internal_id,
        ];

        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            conn.execute(&sql, params).map(|_| ()).map_err(MmError::new)
        })
        .await
    }

    async fn history_has_tx_hash(&self, wallet_id: &WalletId, tx_hash: &str) -> Result<bool, MmError<Self::Error>> {
        let sql = has_transactions_with_hash_sql(wallet_id)?;
        let params = [tx_hash.to_owned()];

        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let count: u32 = conn.query_row(&sql, params, |row| row.get(0))?;
            Ok(count > 0)
        })
        .await
    }

    async fn unique_tx_hashes_num_in_history(&self, wallet_id: &WalletId) -> Result<usize, MmError<Self::Error>> {
        let sql = unique_tx_hashes_num_sql(wallet_id)?;
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let count: u32 = conn.query_row(&sql, NO_PARAMS, |row| row.get(0))?;
            Ok(count as usize)
        })
        .await
    }

    async fn add_tx_to_cache(
        &self,
        wallet_id: &WalletId,
        tx_hash: &str,
        tx_hex: &BytesJson,
    ) -> Result<(), MmError<Self::Error>> {
        let sql = insert_tx_in_cache_sql(wallet_id)?;
        let params = [tx_hash.to_owned(), format!("{:02x}", tx_hex)];
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            conn.execute(&sql, params)?;
            Ok(())
        })
        .await
    }

    async fn tx_bytes_from_cache(
        &self,
        wallet_id: &WalletId,
        tx_hash: &str,
    ) -> Result<Option<BytesJson>, MmError<Self::Error>> {
        let sql = get_tx_hex_from_cache_sql(wallet_id)?;
        let params = [tx_hash.to_owned()];
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let maybe_tx_hex: Result<String, _> = conn.query_row(&sql, params, |row| row.get(0));
            if let Err(SqlError::QueryReturnedNoRows) = maybe_tx_hex {
                return Ok(None);
            }
            let tx_hex = maybe_tx_hex?;
            let tx_bytes =
                hex::decode(&tx_hex).map_err(|e| SqlError::FromSqlConversionFailure(0, Type::Text, Box::new(e)))?;
            Ok(Some(tx_bytes.into()))
        })
        .await
    }

    async fn get_history(
        &self,
        wallet_id: &WalletId,
        filters: GetTxHistoryFilters,
        paging: PagingOptionsEnum<BytesJson>,
        limit: usize,
    ) -> Result<GetHistoryResult, MmError<Self::Error>> {
        // Check if [`GetTxHistoryFilters::for_addresses`] is specified and empty.
        // If it is, it's much more efficient to return an empty result before we do any query.
        if matches!(filters.for_addresses, Some(ref for_addresses) if for_addresses.is_empty()) {
            return Ok(GetHistoryResult {
                transactions: Vec::new(),
                skipped: 0,
                total: 0,
            });
        }

        let wallet_id = wallet_id.clone();
        let selfi = self.clone();

        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let token_id = filters.token_id_or_exclude();
            let mut sql_builder = get_history_builder_preimage(&conn, &wallet_id, token_id, filters.for_addresses)?;

            let total_count_builder = finalize_get_total_count_sql_builder(sql_builder.clone())?;
            let total: isize = total_count_builder
                .query_single_row(|row| row.get(0))?
                .or_mm_err(|| SqlError::QueryReturnedNoRows)?;
            let total = total.try_into().expect("count should be always above zero");

            let offset = match paging {
                PagingOptionsEnum::PageNumber(page) => (page.get() - 1) * limit,
                PagingOptionsEnum::FromId(from_internal_id) => {
                    let maybe_offset = sql_builder
                        .clone()
                        .query_offset_by_id("tx_history.internal_id", format!("{:02x}", from_internal_id))?;
                    match maybe_offset {
                        Some(offset) => offset,
                        None => {
                            // TODO do we need to return `SqlError::QueryReturnedNoRows` error instead?
                            return Ok(GetHistoryResult {
                                transactions: vec![],
                                skipped: 0,
                                total,
                            });
                        },
                    }
                },
            };

            finalize_get_history_sql_builder(&mut sql_builder, offset, limit)?;
            let transactions = sql_builder.query(tx_details_from_row)?;

            let result = GetHistoryResult {
                transactions,
                skipped: offset,
                total,
            };
            Ok(result)
        })
        .await
    }
}
