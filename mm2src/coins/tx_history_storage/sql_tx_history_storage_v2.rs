use crate::my_tx_history_v2::{GetHistoryResult, HistoryCoinType, RemoveTxResult, TxHistoryStorage,
                              TxHistoryStorageError};
use crate::tx_history_storage::{token_id_from_tx_type, CoinTokenId, ConfirmationStatus, CreateTxHistoryStorageError};
use crate::TransactionDetails;
use async_trait::async_trait;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::{async_blocking, PagingOptionsEnum};
use db_common::sqlite::rusqlite::types::Type;
use db_common::sqlite::rusqlite::{Connection, Error as SqlError, Row, NO_PARAMS};
use db_common::sqlite::sql_builder::SqlBuilder;
use db_common::sqlite::{offset_by_id, query_single_row, string_from_row, validate_table_name, CHECK_TABLE_EXISTS_SQL};
use rpc::v1::types::Bytes as BytesJson;
use serde_json::{self as json};
use std::convert::TryInto;
use std::sync::{Arc, Mutex};

fn tx_history_table(ticker: &str) -> String { ticker.to_owned() + "_tx_history" }

fn tx_cache_table(ticker: &str) -> String { ticker.to_owned() + "_tx_cache" }

fn create_tx_history_table_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
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

fn create_tx_cache_table_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_cache_table(for_coin);
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

fn insert_tx_in_history_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
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

fn insert_tx_in_cache_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_cache_table(for_coin);
    validate_table_name(&table_name)?;

    // We can simply ignore the repetitive attempt to insert the same tx_hash
    let sql = format!(
        "INSERT OR IGNORE INTO {} (tx_hash, tx_hex) VALUES (?1, ?2);",
        table_name
    );

    Ok(sql)
}

fn remove_tx_by_internal_id_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!("DELETE FROM {} WHERE internal_id=?1;", table_name);

    Ok(sql)
}

fn select_tx_by_internal_id_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!("SELECT details_json FROM {} WHERE internal_id=?1;", table_name);

    Ok(sql)
}

fn update_tx_in_table_by_internal_id_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
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

fn contains_unconfirmed_transactions_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!(
        "SELECT COUNT(id) FROM {} WHERE confirmation_status = {};",
        table_name,
        ConfirmationStatus::Unconfirmed.to_sql_param()
    );

    Ok(sql)
}

fn get_unconfirmed_transactions_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!(
        "SELECT details_json FROM {} WHERE confirmation_status = {};",
        table_name,
        ConfirmationStatus::Unconfirmed.to_sql_param()
    );

    Ok(sql)
}

fn has_transactions_with_hash_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!("SELECT COUNT(id) FROM {} WHERE tx_hash = ?1;", table_name);

    Ok(sql)
}

fn unique_tx_hashes_num_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!("SELECT COUNT(DISTINCT tx_hash) FROM {};", table_name);

    Ok(sql)
}

fn get_tx_hex_from_cache_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_cache_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = format!("SELECT tx_hex FROM {} WHERE tx_hash = ?1 LIMIT 1;", table_name);

    Ok(sql)
}

fn get_history_builder_preimage(for_coin: &str) -> Result<SqlBuilder, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let mut sql_builder = SqlBuilder::select_from(table_name);
    sql_builder.and_where("token_id = ?1");
    Ok(sql_builder)
}

fn finalize_get_history_sql_builder(sql_builder: &mut SqlBuilder, offset: usize, limit: usize) {
    sql_builder.field("details_json");
    sql_builder.offset(offset);
    sql_builder.limit(limit);
    sql_builder.order_asc("confirmation_status");
    sql_builder.order_desc("block_height");
    sql_builder.order_asc("id");
}

fn tx_details_from_row(row: &Row<'_>) -> Result<TransactionDetails, SqlError> {
    let json_string: String = row.get(0)?;
    json::from_str(&json_string).map_err(|e| SqlError::FromSqlConversionFailure(0, Type::Text, Box::new(e)))
}

impl TxHistoryStorageError for SqlError {}

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

    async fn init(&self, for_coin: &str) -> Result<(), MmError<Self::Error>> {
        let selfi = self.clone();
        let sql_history = create_tx_history_table_sql(for_coin)?;
        let sql_cache = create_tx_cache_table_sql(for_coin)?;
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            conn.execute(&sql_history, NO_PARAMS).map(|_| ())?;
            conn.execute(&sql_cache, NO_PARAMS).map(|_| ())?;
            Ok(())
        })
        .await
    }

    async fn is_initialized_for(&self, for_coin: &str) -> Result<bool, MmError<Self::Error>> {
        let tx_history_table = tx_history_table(for_coin);
        validate_table_name(&tx_history_table)?;

        let tx_cache_table = tx_cache_table(for_coin);
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

    async fn add_transactions_to_history<I>(&self, for_coin: &str, transactions: I) -> Result<(), MmError<Self::Error>>
    where
        I: IntoIterator<Item = TransactionDetails> + Send + 'static,
        I::IntoIter: Send,
    {
        let for_coin = for_coin.to_owned();
        let selfi = self.clone();

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

                sql_transaction.execute(&insert_tx_in_cache_sql(&for_coin)?, tx_cache_params)?;

                let params = [
                    tx_hash,
                    internal_id,
                    tx.block_height.to_string(),
                    confirmation_status.to_sql_param(),
                    token_id,
                    tx_json,
                ];

                sql_transaction.execute(&insert_tx_in_history_sql(&for_coin)?, &params)?;
            }
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn remove_tx_from_history(
        &self,
        for_coin: &str,
        internal_id: &BytesJson,
    ) -> Result<RemoveTxResult, MmError<Self::Error>> {
        let sql = remove_tx_by_internal_id_sql(for_coin)?;
        let params = [format!("{:02x}", internal_id)];
        let selfi = self.clone();

        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            conn.execute(&sql, &params)
                .map(|rows_num| {
                    if rows_num > 0 {
                        RemoveTxResult::TxRemoved
                    } else {
                        RemoveTxResult::TxDidNotExist
                    }
                })
                .map_err(MmError::new)
        })
        .await
    }

    async fn get_tx_from_history(
        &self,
        for_coin: &str,
        internal_id: &BytesJson,
    ) -> Result<Option<TransactionDetails>, MmError<Self::Error>> {
        let params = [format!("{:02x}", internal_id)];
        let sql = select_tx_by_internal_id_sql(for_coin)?;
        let selfi = self.clone();

        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            query_single_row(&conn, &sql, params, tx_details_from_row).map_to_mm(SqlError::from)
        })
        .await
    }

    async fn history_contains_unconfirmed_txes(&self, for_coin: &str) -> Result<bool, MmError<Self::Error>> {
        let sql = contains_unconfirmed_transactions_sql(for_coin)?;
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
        for_coin: &str,
    ) -> Result<Vec<TransactionDetails>, MmError<Self::Error>> {
        let sql = get_unconfirmed_transactions_sql(for_coin)?;
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

    async fn update_tx_in_history(&self, for_coin: &str, tx: &TransactionDetails) -> Result<(), MmError<Self::Error>> {
        let sql = update_tx_in_table_by_internal_id_sql(for_coin)?;

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

    async fn history_has_tx_hash(&self, for_coin: &str, tx_hash: &str) -> Result<bool, MmError<Self::Error>> {
        let sql = has_transactions_with_hash_sql(for_coin)?;
        let params = [tx_hash.to_owned()];

        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let count: u32 = conn.query_row(&sql, params, |row| row.get(0))?;
            Ok(count > 0)
        })
        .await
    }

    async fn unique_tx_hashes_num_in_history(&self, for_coin: &str) -> Result<usize, MmError<Self::Error>> {
        let sql = unique_tx_hashes_num_sql(for_coin)?;
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
        for_coin: &str,
        tx_hash: &str,
        tx_hex: &BytesJson,
    ) -> Result<(), MmError<Self::Error>> {
        let sql = insert_tx_in_cache_sql(for_coin)?;
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
        for_coin: &str,
        tx_hash: &str,
    ) -> Result<Option<BytesJson>, MmError<Self::Error>> {
        let sql = get_tx_hex_from_cache_sql(for_coin)?;
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
        coin_type: HistoryCoinType,
        paging: PagingOptionsEnum<BytesJson>,
        limit: usize,
    ) -> Result<GetHistoryResult, MmError<Self::Error>> {
        let selfi = self.clone();

        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let CoinTokenId { coin, token_id } = CoinTokenId::from_history_coin_type(coin_type);
            let mut sql_builder = get_history_builder_preimage(&coin)?;

            let mut total_builder = sql_builder.clone();
            total_builder.count("id");
            let total_sql = total_builder.sql().expect("valid sql");
            let total: isize = conn.query_row(&total_sql, [&token_id], |row| row.get(0))?;
            let total = total.try_into().expect("count should be always above zero");

            let offset = match paging {
                PagingOptionsEnum::PageNumber(page) => (page.get() - 1) * limit,
                PagingOptionsEnum::FromId(id) => {
                    let id_str = format!("{:02x}", id);
                    let params = [&token_id, &id_str];
                    let maybe_offset = offset_by_id(
                        &conn,
                        &sql_builder,
                        params,
                        "internal_id",
                        "confirmation_status ASC, block_height DESC, id ASC",
                        "internal_id = ?2",
                    )?;
                    match maybe_offset {
                        Some(offset) => offset,
                        None => {
                            return Ok(GetHistoryResult {
                                transactions: vec![],
                                skipped: 0,
                                total,
                            })
                        },
                    }
                },
            };

            finalize_get_history_sql_builder(&mut sql_builder, offset, limit);
            let params = [token_id];

            let sql = sql_builder.sql().expect("valid sql");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query(params)?;
            let transactions = rows.mapped(tx_details_from_row).collect::<Result<_, _>>()?;
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
