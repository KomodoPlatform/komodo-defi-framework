use crate::nft::nft_structs::{Chain, ConvertChain, Nft, NftList, NftTokenAddrId, NftTransferHistory,
                              NftTxHistoryFilters, NftsTransferHistoryList, TxMeta};
use crate::nft::storage::{get_offset_limit, CreateNftStorageError, NftListStorageOps, NftStorageError,
                          NftTxHistoryStorageOps, RemoveNftResult};
use async_trait::async_trait;
use common::async_blocking;
use db_common::sql_build::{SqlCondition, SqlQuery};
use db_common::sqlite::rusqlite::types::{FromSqlError, Type};
use db_common::sqlite::rusqlite::{Connection, Error as SqlError, Row};
use db_common::sqlite::sql_builder::SqlBuilder;
use db_common::sqlite::{query_single_row, string_from_row, validate_table_name, CHECK_TABLE_EXISTS_SQL};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::map_to_mm::MapToMmResult;
use mm2_err_handle::mm_error::{MmError, MmResult};
use mm2_number::BigDecimal;
use serde_json::{self as json};
use std::convert::TryInto;
use std::num::NonZeroUsize;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

fn nft_list_table_name(chain: &Chain) -> String { chain.to_ticker() + "_nft_list" }

fn nft_tx_history_table_name(chain: &Chain) -> String { chain.to_ticker() + "_nft_tx_history" }

fn scanned_nft_blocks_table_name() -> String { "scanned_nft_blocks".to_string() }

fn create_nft_list_table_sql(chain: &Chain) -> MmResult<String, SqlError> {
    let table_name = nft_list_table_name(chain);
    validate_table_name(&table_name)?;
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (
    token_address VARCHAR(256) NOT NULL,
    token_id VARCHAR(256) NOT NULL,
    chain TEXT NOT NULL,
    amount VARCHAR(256) NOT NULL,
    block_number INTEGER NOT NULL,
    contract_type TEXT NOT NULL,
    details_json TEXT,
    PRIMARY KEY (token_address, token_id)
        );",
        table_name
    );
    Ok(sql)
}

fn create_tx_history_table_sql(chain: &Chain) -> MmResult<String, SqlError> {
    let table_name = nft_tx_history_table_name(chain);
    validate_table_name(&table_name)?;
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (
    transaction_hash VARCHAR(256) PRIMARY KEY,
    chain TEXT NOT NULL,
    block_number INTEGER NOT NULL,
    block_timestamp INTEGER NOT NULL,
    contract_type TEXT NOT NULL,
    token_address VARCHAR(256) NOT NULL,
    token_id VARCHAR(256) NOT NULL,
    status TEXT NOT NULL,
    amount VARCHAR(256) NOT NULL,
    token_uri TEXT,
    collection_name TEXT,
    image_url TEXT,
    token_name TEXT,
    details_json TEXT
        );",
        table_name
    );
    Ok(sql)
}

fn create_scanned_nft_blocks_sql() -> MmResult<String, SqlError> {
    let table_name = scanned_nft_blocks_table_name();
    validate_table_name(&table_name)?;
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (
    chain TEXT PRIMARY KEY,
    last_scanned_block INTEGER DEFAULT 0
    );",
        table_name
    );
    Ok(sql)
}

impl NftStorageError for SqlError {}

#[derive(Clone)]
pub struct SqliteNftStorage(Arc<Mutex<Connection>>);

impl SqliteNftStorage {
    pub fn new(ctx: &MmArc) -> MmResult<Self, CreateNftStorageError> {
        let sqlite_connection = ctx
            .sqlite_connection
            .ok_or(MmError::new(CreateNftStorageError::Internal(
                "sqlite_connection is not initialized".to_owned(),
            )))?;
        Ok(SqliteNftStorage(sqlite_connection.clone()))
    }
}

fn get_nft_list_builder_preimage(chains: Vec<Chain>) -> MmResult<SqlBuilder, SqlError> {
    let union_sql_strings = chains
        .iter()
        .map(|chain| {
            let table_name = nft_list_table_name(chain);
            validate_table_name(&table_name)?;
            let sql_builder = SqlBuilder::select_from(table_name.as_str());
            let sql_string = sql_builder
                .sql()
                .map_err(|e| SqlError::ToSqlConversionFailure(e.into()))?
                .trim_end_matches(';')
                .to_string();
            Ok(sql_string)
        })
        .collect::<MmResult<Vec<_>, SqlError>>()?;
    let union_alias_sql = format!("({}) AS nft_list", union_sql_strings.join(" UNION ALL "));
    let mut final_sql_builder = SqlBuilder::select_from(union_alias_sql);
    final_sql_builder.order_desc("nft_list.block_number");
    drop_mutability!(final_sql_builder);
    Ok(final_sql_builder)
}

fn get_nft_tx_builder_preimage(
    chains: Vec<Chain>,
    filters: Option<NftTxHistoryFilters>,
) -> MmResult<SqlBuilder, SqlError> {
    let union_sql_strings = chains
        .into_iter()
        .map(|chain| {
            let table_name = nft_tx_history_table_name(&chain);
            validate_table_name(&table_name)?;
            let sql_builder = nft_history_table_builder_preimage(table_name.as_str(), filters)?;
            let sql_string = sql_builder
                .sql()
                .map_err(|e| SqlError::ToSqlConversionFailure(e.into()))?
                .trim_end_matches(';')
                .to_string();
            Ok(sql_string)
        })
        .collect::<MmResult<Vec<_>, SqlError>>()?;
    let union_alias_sql = format!("({}) AS nft_history", union_sql_strings.join(" UNION ALL "));
    let mut final_sql_builder = SqlBuilder::select_from(union_alias_sql);
    final_sql_builder.order_desc("nft_history.block_timestamp");
    drop_mutability!(final_sql_builder);
    Ok(final_sql_builder)
}

fn nft_history_table_builder_preimage(
    table_name: &str,
    filters: Option<NftTxHistoryFilters>,
) -> Result<SqlBuilder, SqlError> {
    let mut sql_builder = SqlBuilder::select_from(table_name);
    if let Some(filters) = filters {
        if filters.send && !filters.receive {
            sql_builder.and_where_eq("status", "'Send'");
        } else if filters.receive && !filters.send {
            sql_builder.and_where_eq("status", "'Receive'");
        }
        if let Some(date) = filters.from_date {
            sql_builder.and_where(format!("block_timestamp >= {}", date));
        }
        if let Some(date) = filters.to_date {
            sql_builder.and_where(format!("block_timestamp <= {}", date));
        }
    }
    drop_mutability!(sql_builder);
    Ok(sql_builder)
}

fn finalize_nft_list_sql_builder(
    mut sql_builder: SqlBuilder,
    offset: usize,
    limit: usize,
) -> MmResult<String, SqlError> {
    let sql = sql_builder
        .field("nft_list.details_json")
        .offset(offset)
        .limit(limit)
        .sql()
        .map_err(|e| SqlError::ToSqlConversionFailure(e.into()))?;
    Ok(sql)
}

fn finalize_nft_history_sql_builder(
    mut sql_builder: SqlBuilder,
    offset: usize,
    limit: usize,
) -> MmResult<String, SqlError> {
    let sql = sql_builder
        .field("nft_history.details_json")
        .offset(offset)
        .limit(limit)
        .sql()
        .map_err(|e| SqlError::ToSqlConversionFailure(e.into()))?;
    Ok(sql)
}

fn nft_from_row(row: &Row<'_>) -> Result<Nft, SqlError> {
    let json_string: String = row.get(0)?;
    json::from_str(&json_string).map_err(|e| SqlError::FromSqlConversionFailure(0, Type::Text, Box::new(e)))
}

fn tx_history_from_row(row: &Row<'_>) -> Result<NftTransferHistory, SqlError> {
    let json_string: String = row.get(0)?;
    json::from_str(&json_string).map_err(|e| SqlError::FromSqlConversionFailure(0, Type::Text, Box::new(e)))
}

fn token_address_id_from_row(row: &Row<'_>) -> Result<NftTokenAddrId, SqlError> {
    let token_address: String = row.get("token_address")?;
    let token_id_str: String = row.get("token_id")?;
    let token_id = BigDecimal::from_str(&token_id_str).map_err(|_| SqlError::from(FromSqlError::InvalidType))?;
    Ok(NftTokenAddrId {
        token_address,
        token_id,
    })
}

fn insert_nft_in_list_sql(chain: &Chain) -> MmResult<String, SqlError> {
    let table_name = nft_list_table_name(chain);
    validate_table_name(&table_name)?;

    let sql = format!(
        "INSERT INTO {} (
            token_address, token_id, chain, amount, block_number, contract_type, details_json
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7
        );",
        table_name
    );
    Ok(sql)
}

fn insert_tx_in_history_sql(chain: &Chain) -> MmResult<String, SqlError> {
    let table_name = nft_tx_history_table_name(chain);
    validate_table_name(&table_name)?;

    let sql = format!(
        "INSERT INTO {} (
            transaction_hash, chain, block_number, block_timestamp, contract_type,
            token_address, token_id, status, amount, collection_name, image_url, token_name, details_json
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13
        );",
        table_name
    );
    Ok(sql)
}

fn upsert_last_scanned_block_sql() -> MmResult<String, SqlError> {
    let table_name = scanned_nft_blocks_table_name();
    validate_table_name(&table_name)?;
    let sql = format!(
        "INSERT OR REPLACE INTO {} (chain, last_scanned_block) VALUES (?1, ?2);",
        table_name
    );
    Ok(sql)
}

fn update_details_json_by_token_add_id_sql<F>(chain: &Chain, table_name_creator: F) -> MmResult<String, SqlError>
where
    F: FnOnce(&Chain) -> String,
{
    let table_name = table_name_creator(chain);

    validate_table_name(&table_name)?;
    let sql = format!(
        "UPDATE {} SET details_json = ?1 WHERE token_address = ?2 AND token_id = ?3;",
        table_name
    );
    Ok(sql)
}

fn update_meta_by_tx_hash_sql(chain: &Chain) -> MmResult<String, SqlError> {
    let table_name = nft_tx_history_table_name(chain);

    validate_table_name(&table_name)?;
    let sql = format!(
        "UPDATE {} SET token_uri = ?1, collection_name = ?2, image_url = ?3, token_name = ?4, details_json = ?5 WHERE transaction_hash = ?6;",
        table_name
    );
    Ok(sql)
}

fn update_nft_amount_sql<F>(chain: &Chain, table_name_creator: F) -> MmResult<String, SqlError>
where
    F: FnOnce(&Chain) -> String,
{
    let table_name = table_name_creator(chain);

    validate_table_name(&table_name)?;
    let sql = format!(
        "UPDATE {} SET amount = ?1, details_json = ?2 WHERE token_address = ?3 AND token_id = ?4;",
        table_name
    );
    Ok(sql)
}

fn update_nft_amount_and_block_number_sql<F>(chain: &Chain, table_name_creator: F) -> MmResult<String, SqlError>
where
    F: FnOnce(&Chain) -> String,
{
    let table_name = table_name_creator(chain);

    validate_table_name(&table_name)?;
    let sql = format!(
        "UPDATE {} SET amount = ?1, block_number = ?2, details_json = ?3 WHERE token_address = ?4 AND token_id = ?5;",
        table_name
    );
    Ok(sql)
}

fn get_nft_metadata_sql(chain: &Chain) -> MmResult<String, SqlError> {
    let table_name = nft_list_table_name(chain);
    validate_table_name(&table_name)?;
    let sql = format!(
        "SELECT details_json FROM {} WHERE token_address=?1 AND token_id=?2",
        table_name
    );
    Ok(sql)
}

fn select_last_block_number_sql<F>(chain: &Chain, table_name_creator: F) -> MmResult<String, SqlError>
where
    F: FnOnce(&Chain) -> String,
{
    let table_name = table_name_creator(chain);
    validate_table_name(&table_name)?;
    let sql = format!(
        "SELECT block_number FROM {} ORDER BY block_number DESC LIMIT 1",
        table_name
    );
    Ok(sql)
}

fn select_last_scanned_block_sql() -> MmResult<String, SqlError> {
    let table_name = scanned_nft_blocks_table_name();
    validate_table_name(&table_name)?;
    let sql = format!("SELECT last_scanned_block FROM {} WHERE chain=?1", table_name,);
    Ok(sql)
}

fn get_nft_amount_sql<F>(chain: &Chain, table_name_creator: F) -> MmResult<String, SqlError>
where
    F: FnOnce(&Chain) -> String,
{
    let table_name = table_name_creator(chain);
    validate_table_name(&table_name)?;
    let sql = format!(
        "SELECT amount FROM {} WHERE token_address=?1 AND token_id=?2",
        table_name
    );
    Ok(sql)
}

fn delete_nft_sql<F>(chain: &Chain, table_name_creator: F) -> Result<String, MmError<SqlError>>
where
    F: FnOnce(&Chain) -> String,
{
    let table_name = table_name_creator(chain);
    validate_table_name(&table_name)?;
    let sql = format!("DELETE FROM {} WHERE token_address=?1 AND token_id=?2", table_name);
    Ok(sql)
}

fn block_number_from_row(row: &Row<'_>) -> Result<i64, SqlError> { row.get::<_, i64>(0) }

fn nft_amount_from_row(row: &Row<'_>) -> Result<String, SqlError> { row.get(0) }

fn get_txs_from_block_builder<'a>(
    conn: &'a Connection,
    chain: &'a Chain,
    from_block: u64,
) -> MmResult<SqlQuery<'a>, SqlError> {
    let table_name = nft_tx_history_table_name(chain);
    validate_table_name(table_name.as_str())?;
    let mut sql_builder = SqlQuery::select_from(conn, table_name.as_str())?;
    sql_builder
        .sql_builder()
        .and_where(format!("block_number >= '{}'", from_block))
        .order_asc("block_number")
        .field("details_json");
    drop_mutability!(sql_builder);
    Ok(sql_builder)
}

fn get_txs_by_token_addr_id_builder<'a>(
    conn: &'a Connection,
    chain: &'a Chain,
    token_address: String,
    token_id: String,
) -> MmResult<SqlQuery<'a>, SqlError> {
    let table_name = nft_tx_history_table_name(chain);
    validate_table_name(table_name.as_str())?;
    let mut sql_builder = SqlQuery::select_from(conn, table_name.as_str())?;
    sql_builder
        .sql_builder()
        .and_where_eq("token_address", format!("'{}'", token_address))
        .and_where_eq("token_id", format!("'{}'", token_id))
        .field("details_json");
    drop_mutability!(sql_builder);
    Ok(sql_builder)
}

fn get_txs_with_empty_meta_builder<'a>(conn: &'a Connection, chain: &'a Chain) -> MmResult<SqlQuery<'a>, SqlError> {
    let table_name = nft_tx_history_table_name(chain);
    validate_table_name(table_name.as_str())?;
    let mut sql_builder = SqlQuery::select_from(conn, table_name.as_str())?;
    sql_builder
        .sql_builder()
        .distinct()
        .field("token_address")
        .field("token_id")
        .and_where_is_null("token_uri")
        .and_where_is_null("collection_name")
        .and_where_is_null("image_url")
        .and_where_is_null("token_name");
    drop_mutability!(sql_builder);
    Ok(sql_builder)
}

fn get_tx_by_tx_hash_sql(chain: &Chain) -> MmResult<String, SqlError> {
    let table_name = nft_tx_history_table_name(chain);
    validate_table_name(&table_name)?;
    let sql = format!("SELECT details_json FROM {} WHERE transaction_hash=?1", table_name);
    Ok(sql)
}

#[async_trait]
impl NftListStorageOps for SqliteNftStorage {
    type Error = SqlError;

    async fn init(&self, chain: &Chain) -> MmResult<(), Self::Error> {
        let selfi = self.clone();
        let sql_nft_list = create_nft_list_table_sql(chain)?;
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            conn.execute(&sql_nft_list, []).map(|_| ())?;
            conn.execute(&create_scanned_nft_blocks_sql()?, []).map(|_| ())?;
            Ok(())
        })
        .await
    }

    async fn is_initialized(&self, chain: &Chain) -> MmResult<bool, Self::Error> {
        let table_name = nft_list_table_name(chain);
        validate_table_name(&table_name)?;
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let nft_list_initialized = query_single_row(&conn, CHECK_TABLE_EXISTS_SQL, [table_name], string_from_row)?;
            let scanned_nft_blocks_initialized = query_single_row(
                &conn,
                CHECK_TABLE_EXISTS_SQL,
                [scanned_nft_blocks_table_name()],
                string_from_row,
            )?;
            Ok(nft_list_initialized.is_some() && scanned_nft_blocks_initialized.is_some())
        })
        .await
    }

    async fn get_nft_list(
        &self,
        chains: Vec<Chain>,
        max: bool,
        limit: usize,
        page_number: Option<NonZeroUsize>,
    ) -> MmResult<NftList, Self::Error> {
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let sql_builder = get_nft_list_builder_preimage(chains)?;
            let total_count_builder_sql = sql_builder
                .clone()
                .count("*")
                .sql()
                .map_err(|e| SqlError::ToSqlConversionFailure(e.into()))?;
            let total: isize = conn
                .prepare(&total_count_builder_sql)?
                .query_row([], |row| row.get(0))?;
            let count_total = total.try_into().expect("count should not be failed");

            let (offset, limit) = get_offset_limit(max, limit, page_number, count_total);
            let sql = finalize_nft_list_sql_builder(sql_builder, offset, limit)?;
            let nfts = conn
                .prepare(&sql)?
                .query_map([], nft_from_row)?
                .collect::<Result<Vec<_>, _>>()?;
            let result = NftList {
                nfts,
                skipped: offset,
                total: count_total,
            };
            Ok(result)
        })
        .await
    }

    async fn add_nfts_to_list<I>(&self, chain: &Chain, nfts: I, last_scanned_block: u64) -> MmResult<(), Self::Error>
    where
        I: IntoIterator<Item = Nft> + Send + 'static,
        I::IntoIter: Send,
    {
        let selfi = self.clone();
        let chain = *chain;
        async_blocking(move || {
            let mut conn = selfi.0.lock().unwrap();
            let sql_transaction = conn.transaction()?;

            for nft in nfts {
                let nft_json = json::to_string(&nft).expect("serialization should not fail");
                let params = [
                    Some(nft.common.token_address),
                    Some(nft.common.token_id.to_string()),
                    Some(nft.chain.to_string()),
                    Some(nft.common.amount.to_string()),
                    Some(nft.block_number.to_string()),
                    Some(nft.contract_type.to_string()),
                    Some(nft_json),
                ];
                sql_transaction.execute(&insert_nft_in_list_sql(&chain)?, params)?;
            }
            let scanned_block_params = [chain.to_ticker(), last_scanned_block.to_string()];
            sql_transaction.execute(&upsert_last_scanned_block_sql()?, scanned_block_params)?;
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn get_nft(
        &self,
        chain: &Chain,
        token_address: String,
        token_id: BigDecimal,
    ) -> MmResult<Option<Nft>, Self::Error> {
        let sql = get_nft_metadata_sql(chain)?;
        let params = [token_address, token_id.to_string()];
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            query_single_row(&conn, &sql, params, nft_from_row).map_to_mm(SqlError::from)
        })
        .await
    }

    async fn remove_nft_from_list(
        &self,
        chain: &Chain,
        token_address: String,
        token_id: BigDecimal,
        scanned_block: u64,
    ) -> MmResult<RemoveNftResult, Self::Error> {
        let sql = delete_nft_sql(chain, nft_list_table_name)?;
        let params = [token_address, token_id.to_string()];
        let scanned_block_params = [chain.to_ticker(), scanned_block.to_string()];
        let selfi = self.clone();
        async_blocking(move || {
            let mut conn = selfi.0.lock().unwrap();
            let sql_transaction = conn.transaction()?;
            let rows_num = sql_transaction.execute(&sql, params)?;

            let remove_nft_result = if rows_num > 0 {
                RemoveNftResult::NftRemoved
            } else {
                RemoveNftResult::NftDidNotExist
            };
            sql_transaction.execute(&upsert_last_scanned_block_sql()?, scanned_block_params)?;
            sql_transaction.commit()?;
            Ok(remove_nft_result)
        })
        .await
    }

    async fn get_nft_amount(
        &self,
        chain: &Chain,
        token_address: String,
        token_id: BigDecimal,
    ) -> MmResult<Option<String>, Self::Error> {
        let sql = get_nft_amount_sql(chain, nft_list_table_name)?;
        let params = [token_address, token_id.to_string()];
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            query_single_row(&conn, &sql, params, nft_amount_from_row).map_to_mm(SqlError::from)
        })
        .await
    }

    async fn refresh_nft_metadata(&self, chain: &Chain, nft: Nft) -> MmResult<(), Self::Error> {
        let sql = update_details_json_by_token_add_id_sql(chain, nft_list_table_name)?;
        let nft_json = json::to_string(&nft).expect("serialization should not fail");
        let selfi = self.clone();
        async_blocking(move || {
            let mut conn = selfi.0.lock().unwrap();
            let sql_transaction = conn.transaction()?;
            let params = [nft_json, nft.common.token_address, nft.common.token_id.to_string()];
            sql_transaction.execute(&sql, params)?;
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn get_last_block_number(&self, chain: &Chain) -> MmResult<Option<u64>, Self::Error> {
        let sql = select_last_block_number_sql(chain, nft_list_table_name)?;
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            query_single_row(&conn, &sql, [], block_number_from_row).map_to_mm(SqlError::from)
        })
        .await?
        .map(|b| b.try_into())
        .transpose()
        .map_to_mm(|e| SqlError::FromSqlConversionFailure(2, Type::Integer, Box::new(e)))
    }

    async fn get_last_scanned_block(&self, chain: &Chain) -> MmResult<Option<u64>, Self::Error> {
        let sql = select_last_scanned_block_sql()?;
        let params = [chain.to_ticker()];
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            query_single_row(&conn, &sql, params, block_number_from_row).map_to_mm(SqlError::from)
        })
        .await?
        .map(|b| b.try_into())
        .transpose()
        .map_to_mm(|e| SqlError::FromSqlConversionFailure(2, Type::Integer, Box::new(e)))
    }

    async fn update_nft_amount(&self, chain: &Chain, nft: Nft, scanned_block: u64) -> MmResult<(), Self::Error> {
        let sql = update_nft_amount_sql(chain, nft_list_table_name)?;
        let nft_json = json::to_string(&nft).expect("serialization should not fail");
        let scanned_block_params = [chain.to_ticker(), scanned_block.to_string()];
        let selfi = self.clone();
        async_blocking(move || {
            let mut conn = selfi.0.lock().unwrap();
            let sql_transaction = conn.transaction()?;
            let params = [
                Some(nft.common.amount.to_string()),
                Some(nft_json),
                Some(nft.common.token_address),
                Some(nft.common.token_id.to_string()),
            ];
            sql_transaction.execute(&sql, params)?;
            sql_transaction.execute(&upsert_last_scanned_block_sql()?, scanned_block_params)?;
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn update_nft_amount_and_block_number(&self, chain: &Chain, nft: Nft) -> MmResult<(), Self::Error> {
        let sql = update_nft_amount_and_block_number_sql(chain, nft_list_table_name)?;
        let nft_json = json::to_string(&nft).expect("serialization should not fail");
        let scanned_block_params = [chain.to_ticker(), nft.block_number.to_string()];
        let selfi = self.clone();
        async_blocking(move || {
            let mut conn = selfi.0.lock().unwrap();
            let sql_transaction = conn.transaction()?;
            let params = [
                Some(nft.common.amount.to_string()),
                Some(nft.block_number.to_string()),
                Some(nft_json),
                Some(nft.common.token_address),
                Some(nft.common.token_id.to_string()),
            ];
            sql_transaction.execute(&sql, params)?;
            sql_transaction.execute(&upsert_last_scanned_block_sql()?, scanned_block_params)?;
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }
}

#[async_trait]
impl NftTxHistoryStorageOps for SqliteNftStorage {
    type Error = SqlError;

    async fn init(&self, chain: &Chain) -> MmResult<(), Self::Error> {
        let selfi = self.clone();
        let sql_tx_history = create_tx_history_table_sql(chain)?;
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            conn.execute(&sql_tx_history, []).map(|_| ())?;
            Ok(())
        })
        .await
    }

    async fn is_initialized(&self, chain: &Chain) -> MmResult<bool, Self::Error> {
        let table_name = nft_tx_history_table_name(chain);
        validate_table_name(&table_name)?;
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let nft_list_initialized = query_single_row(&conn, CHECK_TABLE_EXISTS_SQL, [table_name], string_from_row)?;
            Ok(nft_list_initialized.is_some())
        })
        .await
    }

    async fn get_tx_history(
        &self,
        chains: Vec<Chain>,
        max: bool,
        limit: usize,
        page_number: Option<NonZeroUsize>,
        filters: Option<NftTxHistoryFilters>,
    ) -> MmResult<NftsTransferHistoryList, Self::Error> {
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let sql_builder = get_nft_tx_builder_preimage(chains, filters)?;
            let total_count_builder_sql = sql_builder
                .clone()
                .count("*")
                .sql()
                .map_err(|e| SqlError::ToSqlConversionFailure(e.into()))?;
            let total: isize = conn
                .prepare(&total_count_builder_sql)?
                .query_row([], |row| row.get(0))?;
            let count_total = total.try_into().expect("count should not be failed");

            let (offset, limit) = get_offset_limit(max, limit, page_number, count_total);
            let sql = finalize_nft_history_sql_builder(sql_builder, offset, limit)?;
            let txs = conn
                .prepare(&sql)?
                .query_map([], tx_history_from_row)?
                .collect::<Result<Vec<_>, _>>()?;
            let result = NftsTransferHistoryList {
                transfer_history: txs,
                skipped: offset,
                total: count_total,
            };
            Ok(result)
        })
        .await
    }

    async fn add_txs_to_history<I>(&self, chain: &Chain, txs: I) -> MmResult<(), Self::Error>
    where
        I: IntoIterator<Item = NftTransferHistory> + Send + 'static,
        I::IntoIter: Send,
    {
        let selfi = self.clone();
        let chain = *chain;
        async_blocking(move || {
            let mut conn = selfi.0.lock().unwrap();
            let sql_transaction = conn.transaction()?;

            for tx in txs {
                let tx_json = json::to_string(&tx).expect("serialization should not fail");
                let params = [
                    Some(tx.common.transaction_hash),
                    Some(tx.chain.to_string()),
                    Some(tx.block_number.to_string()),
                    Some(tx.block_timestamp.to_string()),
                    Some(tx.contract_type.to_string()),
                    Some(tx.common.token_address),
                    Some(tx.common.token_id.to_string()),
                    Some(tx.status.to_string()),
                    Some(tx.common.amount.to_string()),
                    tx.collection_name,
                    tx.image_url,
                    tx.token_name,
                    Some(tx_json),
                ];
                sql_transaction.execute(&insert_tx_in_history_sql(&chain)?, params)?;
            }
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn get_last_block_number(&self, chain: &Chain) -> MmResult<Option<u64>, Self::Error> {
        let sql = select_last_block_number_sql(chain, nft_tx_history_table_name)?;
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            query_single_row(&conn, &sql, [], block_number_from_row).map_to_mm(SqlError::from)
        })
        .await?
        .map(|b| b.try_into())
        .transpose()
        .map_to_mm(|e| SqlError::FromSqlConversionFailure(2, Type::Integer, Box::new(e)))
    }

    async fn get_txs_from_block(
        &self,
        chain: &Chain,
        from_block: u64,
    ) -> MmResult<Vec<NftTransferHistory>, Self::Error> {
        let selfi = self.clone();
        let chain = *chain;
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let sql_builder = get_txs_from_block_builder(&conn, &chain, from_block)?;
            let txs = sql_builder.query(tx_history_from_row)?;
            Ok(txs)
        })
        .await
    }

    async fn get_txs_by_token_addr_id(
        &self,
        chain: &Chain,
        token_address: String,
        token_id: BigDecimal,
    ) -> MmResult<Vec<NftTransferHistory>, Self::Error> {
        let selfi = self.clone();
        let chain = *chain;
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let sql_builder = get_txs_by_token_addr_id_builder(&conn, &chain, token_address, token_id.to_string())?;
            let txs = sql_builder.query(tx_history_from_row)?;
            Ok(txs)
        })
        .await
    }

    async fn get_tx_by_tx_hash(
        &self,
        chain: &Chain,
        transaction_hash: String,
    ) -> MmResult<Option<NftTransferHistory>, Self::Error> {
        let sql = get_tx_by_tx_hash_sql(chain)?;
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            query_single_row(&conn, &sql, [transaction_hash], tx_history_from_row).map_to_mm(SqlError::from)
        })
        .await
    }

    async fn update_tx_meta_by_hash(&self, chain: &Chain, tx: NftTransferHistory) -> MmResult<(), Self::Error> {
        let sql = update_meta_by_tx_hash_sql(chain)?;
        let tx_json = json::to_string(&tx).expect("serialization should not fail");
        let params = [
            tx.token_uri,
            tx.collection_name,
            tx.image_url,
            tx.token_name,
            Some(tx_json),
            Some(tx.common.transaction_hash),
        ];
        let selfi = self.clone();
        async_blocking(move || {
            let mut conn = selfi.0.lock().unwrap();
            let sql_transaction = conn.transaction()?;
            sql_transaction.execute(&sql, params)?;
            sql_transaction.commit()?;
            Ok(())
        })
        .await
    }

    async fn update_txs_meta_by_token_addr_id(&self, chain: &Chain, tx_meta: TxMeta) -> MmResult<(), Self::Error> {
        let selfi = self.clone();
        let txs = selfi
            .get_txs_by_token_addr_id(chain, tx_meta.token_address, tx_meta.token_id)
            .await?;
        for mut tx in txs.into_iter() {
            tx.token_uri = tx_meta.token_uri.clone();
            tx.collection_name = tx_meta.collection_name.clone();
            tx.image_url = tx_meta.image_url.clone();
            tx.token_name = tx_meta.token_name.clone();
            drop_mutability!(tx);
            selfi.update_tx_meta_by_hash(chain, tx).await?;
        }
        Ok(())
    }

    async fn get_txs_with_empty_meta(&self, chain: &Chain) -> MmResult<Vec<NftTokenAddrId>, Self::Error> {
        let selfi = self.clone();
        let chain = *chain;
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let sql_builder = get_txs_with_empty_meta_builder(&conn, &chain)?;
            let token_addr_id_pair = sql_builder.query(token_address_id_from_row)?;
            Ok(token_addr_id_pair)
        })
        .await
    }
}
