use crate::my_tx_history_v2::{HistoryCoinType, RemoveTxResult, TxHistoryStorage, TxHistoryStorageError};
use crate::{TransactionDetails, TransactionType};
use async_trait::async_trait;
use common::mm_error::prelude::*;
use common::rusqlite::types::Type;
use common::rusqlite::{Connection, Error as SqlError, Row, ToSql, NO_PARAMS};
use common::{async_blocking, PagingOptionsEnum};
use rpc::v1::types::Bytes as BytesJson;
use serde_json::{self as json};
use std::sync::{Arc, Mutex};

const CHECK_TABLE_EXISTS_SQL: &str = "SELECT name FROM sqlite_master WHERE type='table' AND name=?1;";

fn tx_history_table(ticker: &str) -> String { ticker.to_owned() + "_tx_history" }

fn tx_cache_table(ticker: &str) -> String { ticker.to_owned() + "_tx_cache" }

fn validate_table_name(table_name: &str) -> Result<(), MmError<SqlError>> {
    // As per https://stackoverflow.com/a/3247553, tables can't be the target of parameter substitution.
    // So we have to use a plain concatenation disallowing any characters in the table name that may lead to SQL injection.
    if table_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        Ok(())
    } else {
        MmError::err(SqlError::InvalidParameterName(table_name.to_string()))
    }
}

fn create_tx_history_table_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = "CREATE TABLE IF NOT EXISTS ".to_owned()
        + &table_name
        + " (
        id INTEGER NOT NULL PRIMARY KEY,
        tx_hash VARCHAR(255) NOT NULL,
        internal_id VARCHAR(255) NOT NULL UNIQUE,
        block_height INTEGER NOT NULL,
        confirmation_status INTEGER NOT NULL,
        token_id VARCHAR(255) NOT NULL,
        details_json TEXT
    );";

    Ok(sql)
}

fn create_tx_cache_table_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_cache_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = "CREATE TABLE IF NOT EXISTS ".to_owned()
        + &table_name
        + " (
        tx_hash VARCHAR(255) NOT NULL UNIQUE,
        tx_hex TEXT NOT NULL
    );";

    Ok(sql)
}

fn insert_tx_in_history_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = "INSERT INTO ".to_owned()
        + &table_name
        + " (tx_hash, internal_id, block_height, confirmation_status, token_id, details_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6);";

    Ok(sql)
}

fn insert_tx_in_cache_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_cache_table(for_coin);
    validate_table_name(&table_name)?;

    // We can simply ignore the repetitive attempt to insert the same tx_hash
    let sql = "INSERT OR IGNORE INTO ".to_owned() + &table_name + " (tx_hash, tx_hex) VALUES (?1, ?2);";

    Ok(sql)
}

fn remove_tx_from_table_by_internal_id_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = "DELETE FROM ".to_owned() + &table_name + " WHERE internal_id=?1;";

    Ok(sql)
}

fn select_tx_from_table_by_internal_id_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = "SELECT details_json FROM ".to_owned() + &table_name + " WHERE internal_id=?1;";

    Ok(sql)
}

fn update_tx_in_table_by_internal_id_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = "UPDATE ".to_owned()
        + &table_name
        + " SET block_height = ?1, confirmation_status = ?2, details_json = ?3 WHERE internal_id=?4;";

    Ok(sql)
}

fn contains_unconfirmed_transactions_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = "SELECT COUNT(id) FROM ".to_owned() + &table_name + " WHERE confirmation_status = 0;";

    Ok(sql)
}

fn get_unconfirmed_transactions_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = "SELECT details_json FROM ".to_owned() + &table_name + " WHERE confirmation_status = 0;";

    Ok(sql)
}

fn has_transactions_with_hash_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = "SELECT COUNT(id) FROM ".to_owned() + &table_name + " WHERE tx_hash = ?1;";

    Ok(sql)
}

fn unique_tx_hashes_num_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_history_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = "SELECT COUNT(DISTINCT tx_hash) FROM ".to_owned() + &table_name + ";";

    Ok(sql)
}

fn get_tx_hex_from_cache_sql(for_coin: &str) -> Result<String, MmError<SqlError>> {
    let table_name = tx_cache_table(for_coin);
    validate_table_name(&table_name)?;

    let sql = "SELECT tx_hex FROM ".to_owned() + &table_name + " WHERE tx_hash = ?1 LIMIT 1;";

    Ok(sql)
}

#[derive(Clone)]
pub struct SqliteTxHistoryStorage(pub Arc<Mutex<Connection>>);

#[cfg(test)]
impl SqliteTxHistoryStorage {
    pub fn in_memory() -> Self { SqliteTxHistoryStorage(Arc::new(Mutex::new(Connection::open_in_memory().unwrap()))) }

    fn is_table_empty(&self, table_name: &str) -> bool {
        validate_table_name(table_name).unwrap();
        let sql = "SELECT COUNT(id) FROM ".to_owned() + table_name + ";";
        let conn = self.0.lock().unwrap();
        let rows_count: u32 = conn.query_row(&sql, NO_PARAMS, |row| row.get(0)).unwrap();
        rows_count == 0
    }
}

impl TxHistoryStorageError for SqlError {}

fn query_single_row<T>(
    conn: &mut Connection,
    query: &str,
    params: &[dyn ToSql],
    map_fn: FnOnce(&Row<'_>) -> Result<T, SqlError>,
) -> Result<Option<T>, MmError<SqlError>> {
    let maybe_result = conn.query_row(query, params, map_fn);
    if let Err(SqlError::QueryReturnedNoRows) = maybe_result {
        return Ok(None);
    }

    let result = maybe_result?;
    Ok(Some(result))
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
            let mut conn = selfi.0.lock().unwrap();
        })
        .await
    }

    async fn add_transactions_to_history(
        &self,
        for_coin: &str,
        transactions: impl IntoIterator<Item = TransactionDetails> + Send + 'static,
    ) -> Result<(), MmError<Self::Error>> {
        let for_coin = for_coin.to_owned();
        let selfi = self.clone();

        async_blocking(move || {
            let mut conn = selfi.0.lock().unwrap();
            let sql_transaction = conn.transaction()?;

            for tx in transactions {
                let tx_hash = format!("{:02x}", tx.tx_hash);
                let internal_id = format!("{:02x}", tx.internal_id);
                let confirmation_status = if tx.block_height > 0 { 1 } else { 0 };
                let token_id = if let TransactionType::TokenTransfer(token_id) = &tx.transaction_type {
                    format!("{:02x}", token_id)
                } else {
                    "".to_owned()
                };
                let tx_json = json::to_string(&tx).expect("serialization should not fail");

                let tx_hex = format!("{:02x}", tx.tx_hex);
                let tx_cache_params = [&tx_hash, &tx_hex];

                sql_transaction.execute(&insert_tx_in_cache_sql(&for_coin)?, &tx_cache_params)?;

                let params = [
                    tx_hash,
                    internal_id,
                    tx.block_height.to_string(),
                    confirmation_status.to_string(),
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
        let sql = remove_tx_from_table_by_internal_id_sql(for_coin)?;
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
        let sql = select_tx_from_table_by_internal_id_sql(for_coin)?;
        let selfi = self.clone();

        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let maybe_json_string = conn.query_row::<String, _, _>(&sql, &params, |row| row.get(0));
            if let Err(SqlError::QueryReturnedNoRows) = maybe_json_string {
                return Ok(None);
            }

            let json_string = maybe_json_string?;
            let tx_details = json::from_str(&json_string)
                .map_err(|e| SqlError::FromSqlConversionFailure(0, Type::Text, Box::new(e)))?;

            Ok(Some(tx_details))
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
            let result = rows
                .mapped(|row| {
                    let string: String = row.get(0)?;
                    json::from_str(&string).map_err(|e| SqlError::FromSqlConversionFailure(0, Type::Text, Box::new(e)))
                })
                .collect::<Result<_, _>>()?;
            Ok(result)
        })
        .await
    }

    async fn update_tx_in_history(&self, for_coin: &str, tx: &TransactionDetails) -> Result<(), MmError<Self::Error>> {
        let sql = update_tx_in_table_by_internal_id_sql(for_coin)?;

        let block_height = tx.block_height.to_string();
        let confirmation_status = if tx.block_height > 0 { 1 } else { 0 };
        let json_details = json::to_string(tx).unwrap();
        let internal_id = format!("{:02x}", tx.internal_id);

        let params = [block_height, confirmation_status.to_string(), json_details, internal_id];

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
        tx_hash: &BytesJson,
        tx_hex: &BytesJson,
    ) -> Result<(), MmError<Self::Error>> {
        let sql = insert_tx_in_cache_sql(for_coin)?;
        let params = [format!("{:02x}", tx_hash), format!("{:02x}", tx_hex)];
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
        tx_hash: &BytesJson,
    ) -> Result<Option<BytesJson>, MmError<Self::Error>> {
        let sql = get_tx_hex_from_cache_sql(for_coin)?;
        let params = [format!("{:02x}", tx_hash)];
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
        paging: &PagingOptionsEnum<BytesJson>,
        limit: usize,
    ) -> Result<Vec<TransactionDetails>, MmError<Self::Error>> {
        todo!()
    }
}

#[cfg(test)]
mod sql_tx_history_storage_tests {
    use super::*;
    use common::block_on;

    #[test]
    fn test_init_collection() {
        let for_coin = "init_collection";
        let storage = SqliteTxHistoryStorage::in_memory();
        let initialized = block_on(storage.is_initialized_for(for_coin)).unwrap();
        assert!(!initialized);

        block_on(storage.init(for_coin)).unwrap();
        // repetitive init must not fail
        block_on(storage.init(for_coin)).unwrap();

        let initialized = block_on(storage.is_initialized_for(for_coin)).unwrap();
        assert!(initialized);
    }

    #[test]
    fn test_add_transactions() {
        let for_coin = "add_transactions";
        let storage = SqliteTxHistoryStorage::in_memory();
        let table = tx_history_table(for_coin);

        block_on(storage.init(for_coin)).unwrap();
        let tx1_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
        let tx1: TransactionDetails = json::from_str(&tx1_json).unwrap();
        let transactions = [tx1.clone(), tx1.clone()];

        // must fail because we are adding transactions with the same internal_id
        block_on(storage.add_transactions_to_history(for_coin, transactions)).unwrap_err();
        assert!(storage.is_table_empty(&table));

        let tx2_json = r#"{"tx_hex":"0400008085202f890158d6bccb2141e18633171f631f594b7f1ae85985390b534733ea5be4da220426030000006b483045022100895dea201a1dc59480d59790569df8664cf3d1d9332efeea7dcc38b4a96399b402206c183f33a3e87eb473a7d3da1488ee9a7d9580cfc86cc8460c79a69c08818478012102d09f2cb1693be9c0ea73bb48d45ce61805edd1c43590681b02f877206078a5b3ffffffff0400e1f505000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac00c2eb0b000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588aca01f791c000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac500df208ed0000001976a91490a0d8ba62c339ade97a14e81b6f531de03fdbb288ac00000000000000000000000000000000000000","tx_hash":"8d61223938c56ca97e9a0e1a295734c5f7b9dba8e4e0c1c638125190e7e796fa","from":["RNTv4xTLLm26p3SvsQCBy9qNK7s1RgGYSB"],"to":["RNTv4xTLLm26p3SvsQCBy9qNK7s1RgGYSB","RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10188.3504","spent_by_me":"0","received_by_me":"7.777","my_balance_change":"7.777","block_height":793474,"timestamp":1612780908,"fee_details":{"type":"Utxo","amount":"0.0001"},"coin":"RICK","internal_id":"8d61223938c56ca97e9a0e1a295734c5f7b9dba8e4e0c1c638125190e7e796fa"}"#;
        let tx2 = json::from_str(tx2_json).unwrap();
        let transactions = [tx1, tx2];
        block_on(storage.add_transactions_to_history(for_coin, transactions)).unwrap();
        assert!(!storage.is_table_empty(&table));
    }

    #[test]
    fn test_remove_transaction() {
        let for_coin = "remove_transaction";
        let storage = SqliteTxHistoryStorage::in_memory();

        block_on(storage.init(for_coin)).unwrap();
        let tx_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
        block_on(storage.add_transactions_to_history(for_coin, [json::from_str(tx_json).unwrap()])).unwrap();

        let remove_res = block_on(storage.remove_tx_from_history(
            for_coin,
            &"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c".into(),
        ))
        .unwrap();
        assert!(remove_res.tx_existed());

        let remove_res = block_on(storage.remove_tx_from_history(
            for_coin,
            &"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c".into(),
        ))
        .unwrap();
        assert!(!remove_res.tx_existed());
    }

    #[test]
    fn test_get_transaction() {
        let for_coin = "get_transaction";
        let storage = SqliteTxHistoryStorage::in_memory();
        block_on(storage.init(for_coin)).unwrap();

        let tx_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
        block_on(storage.add_transactions_to_history(for_coin, [json::from_str(tx_json).unwrap()])).unwrap();

        let tx = block_on(storage.get_tx_from_history(
            for_coin,
            &"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c".into(),
        ))
        .unwrap()
        .unwrap();
        println!("{:?}", tx);

        block_on(storage.remove_tx_from_history(
            for_coin,
            &"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c".into(),
        ))
        .unwrap();

        let tx = block_on(storage.get_tx_from_history(
            for_coin,
            &"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c".into(),
        ))
        .unwrap();
        assert!(tx.is_none());
    }

    #[test]
    fn test_update_transaction() {
        let for_coin = "update_transaction";
        let storage = SqliteTxHistoryStorage::in_memory();
        block_on(storage.init(for_coin)).unwrap();

        let tx_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
        let mut tx_details: TransactionDetails = json::from_str(tx_json).unwrap();
        block_on(storage.add_transactions_to_history(for_coin, [tx_details.clone()])).unwrap();

        tx_details.block_height = 12345;

        block_on(storage.update_tx_in_history(for_coin, &tx_details)).unwrap();

        let updated = block_on(storage.get_tx_from_history(for_coin, &tx_details.internal_id))
            .unwrap()
            .unwrap();

        assert_eq!(12345, updated.block_height);
    }

    #[test]
    fn test_contains_and_get_unconfirmed_transaction() {
        let for_coin = "contains_and_get_unconfirmed_transaction";
        let storage = SqliteTxHistoryStorage::in_memory();
        block_on(storage.init(for_coin)).unwrap();

        let tx_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
        let mut tx_details: TransactionDetails = json::from_str(tx_json).unwrap();
        tx_details.block_height = 0;
        block_on(storage.add_transactions_to_history(for_coin, [tx_details.clone()])).unwrap();

        let contains_unconfirmed = block_on(storage.history_contains_unconfirmed_txes(for_coin)).unwrap();
        assert!(contains_unconfirmed);

        let unconfirmed_transactions = block_on(storage.get_unconfirmed_txes_from_history(for_coin)).unwrap();
        assert_eq!(unconfirmed_transactions.len(), 1);

        tx_details.block_height = 12345;
        block_on(storage.update_tx_in_history(for_coin, &tx_details)).unwrap();

        let contains_unconfirmed = block_on(storage.history_contains_unconfirmed_txes(for_coin)).unwrap();
        assert!(!contains_unconfirmed);

        let unconfirmed_transactions = block_on(storage.get_unconfirmed_txes_from_history(for_coin)).unwrap();
        assert!(unconfirmed_transactions.is_empty());
    }

    #[test]
    fn test_has_transactions_with_hash() {
        let for_coin = "has_transactions_with_hash";
        let storage = SqliteTxHistoryStorage::in_memory();
        block_on(storage.init(for_coin)).unwrap();

        assert!(!block_on(storage.history_has_tx_hash(
            for_coin,
            "2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"
        ))
        .unwrap());

        let tx_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
        let tx_details: TransactionDetails = json::from_str(tx_json).unwrap();

        block_on(storage.add_transactions_to_history(for_coin, [tx_details])).unwrap();

        assert!(block_on(storage.history_has_tx_hash(
            for_coin,
            "2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"
        ))
        .unwrap());
    }

    #[test]
    fn test_unique_tx_hashes_num() {
        let for_coin = "unique_tx_hashes_num";
        let storage = SqliteTxHistoryStorage::in_memory();
        block_on(storage.init(for_coin)).unwrap();

        let tx1_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
        let tx1: TransactionDetails = json::from_str(&tx1_json).unwrap();

        let mut tx2 = tx1.clone();
        tx2.internal_id = BytesJson(vec![1; 32]);

        let tx3_json = r#"{"tx_hex":"0400008085202f890158d6bccb2141e18633171f631f594b7f1ae85985390b534733ea5be4da220426030000006b483045022100895dea201a1dc59480d59790569df8664cf3d1d9332efeea7dcc38b4a96399b402206c183f33a3e87eb473a7d3da1488ee9a7d9580cfc86cc8460c79a69c08818478012102d09f2cb1693be9c0ea73bb48d45ce61805edd1c43590681b02f877206078a5b3ffffffff0400e1f505000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac00c2eb0b000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588aca01f791c000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac500df208ed0000001976a91490a0d8ba62c339ade97a14e81b6f531de03fdbb288ac00000000000000000000000000000000000000","tx_hash":"8d61223938c56ca97e9a0e1a295734c5f7b9dba8e4e0c1c638125190e7e796fa","from":["RNTv4xTLLm26p3SvsQCBy9qNK7s1RgGYSB"],"to":["RNTv4xTLLm26p3SvsQCBy9qNK7s1RgGYSB","RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10188.3504","spent_by_me":"0","received_by_me":"7.777","my_balance_change":"7.777","block_height":793474,"timestamp":1612780908,"fee_details":{"type":"Utxo","amount":"0.0001"},"coin":"RICK","internal_id":"8d61223938c56ca97e9a0e1a295734c5f7b9dba8e4e0c1c638125190e7e796fa"}"#;
        let tx3 = json::from_str(tx3_json).unwrap();

        let transactions = [tx1, tx2, tx3];
        block_on(storage.add_transactions_to_history(for_coin, transactions)).unwrap();

        let tx_hashes_num = block_on(storage.unique_tx_hashes_num_in_history(for_coin)).unwrap();
        assert_eq!(2, tx_hashes_num);
    }

    #[test]
    fn test_add_and_get_tx_from_cache() {
        let for_coin = "test_add_and_get_tx_from_cache";
        let storage = SqliteTxHistoryStorage::in_memory();
        block_on(storage.init(for_coin)).unwrap();

        let tx = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
        let tx: TransactionDetails = json::from_str(tx).unwrap();

        block_on(storage.add_tx_to_cache(for_coin, &tx.tx_hash, &tx.tx_hex)).unwrap();

        let tx_hex = block_on(storage.tx_bytes_from_cache(for_coin, &tx.tx_hash))
            .unwrap()
            .unwrap();

        assert_eq!(tx_hex, tx.tx_hex);
    }

    #[test]
    fn test_get_raw_tx_bytes_on_add_transactions() {
        let for_coin = "test_get_raw_tx_bytes_on_add_transactions";
        let storage = SqliteTxHistoryStorage::in_memory();
        block_on(storage.init(for_coin)).unwrap();

        let tx_hash = "2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c".into();

        let maybe_tx_hex = block_on(storage.tx_bytes_from_cache(for_coin, &tx_hash)).unwrap();
        assert!(maybe_tx_hex.is_none());

        let tx1_json = r#"{"tx_hex":"0400008085202f890708b189a2d740a74042541fe687a8d698b7a00c1bfdaf0c708b6bb32f8f7307aa000000006946304302201529f09fdf9177e8b5e2d494488da1e49ec7c1b85a457871e1a78df4e3ba0541021f74538866128b21ed0b77701289ad49ee9a74f8349b9670f73cf6babc4a8ce5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff6403323bb3cd025754336cad57ddc36aedb56107a7a1c6f6ddbfbc893c69d556000000006a4730440220560b8d87f3f020856d3e4704be15a307aa8a49290bf7a8e27a66fc0436e3eb9c0220585c1705a701a669b6b53dae2aad2729786590fbbfbb8f7998bb22e38b60c2d5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff1c5f114649d5194b15502f286d337e03ca7fc3eb0798bc91e6006a645c525f96000000006a473044022078439f12c288d9d694820dbff1e1ceb592be28f7b7e9ba91c73af8110b171c3f02200c8a061f3d48daefaeed40e667543693bb5f206e58fa15b93808e2ecf762ec2f012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffff322a446b2373782c727e2f83a914707d5f8af8fd4f4db34243c7223d438f5f5000000006b483045022100dd101b16dfbe02201768eab2bbbd9df40e56a565492b38e7304284385f04cccf02207ac4e8f1aa768162d24a9b1fb73df0771f34942c2120f980228961e9fcb338ea012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d000000006a47304402207c539bcb32efe7a13f1ff6a7b44a5dce4f794a3af7009eb960a65b03214f2fa102204bc3cddc50c8042c2f852a18c0c68107418ac692f0984c3e7ec2f2d1bf23adf5012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d010000006b4830450221009170c72f25f68e9200b398695e9f6edc706b868d75f7a1e194e068ac1377c95e02206265bb27fcf97fa0d13842d49772bd4b37b8661592df6d7fcec5b7e6c828ecf7012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36fafffffffffa96e7e790511238c6c1e0e4a8dbb9f7c53457291a0e9a7ea96cc5383922618d020000006a47304402206dce88dc192623e69a17cc56609872c75e35b5c608ffeaa31f6df70b09ddbd5302206cf9688439b2192ba57d72af024855741bf77a2a58acf10e5eddfcc36fe7be74012103ad6f89abc2e5beaa8a3ac28e22170659b3209fe2ddf439681b4b8f31508c36faffffffff0198e8d440000000001976a914d55f0df6cb82630ad21a4e6049522a6f2b6c9d4588ac59cbb060000000000000000000000000000000","tx_hash":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c","from":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"to":["RUjPst697T7ahtF8EpZ1whpAmJZfqfwW36"],"total_amount":"10.87696","spent_by_me":"10.87696","received_by_me":"10.87695","my_balance_change":"-0.00001","block_height":949554,"timestamp":1622199314,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"2c33baf0c40eebcb70fc22eab0158e315e2176e4a3f20acddcd849186fca492c"}"#;
        let tx1: TransactionDetails = json::from_str(tx1_json).unwrap();

        let mut tx2 = tx1.clone();
        tx2.internal_id = BytesJson(vec![1; 32]);

        let expected_tx_hex = tx1.tx_hex.clone();

        let transactions = [tx1, tx2];
        block_on(storage.add_transactions_to_history(for_coin, transactions)).unwrap();

        let tx_hex = block_on(storage.tx_bytes_from_cache(for_coin, &tx_hash))
            .unwrap()
            .unwrap();

        assert_eq!(tx_hex, expected_tx_hex);
    }
}
