use async_trait::async_trait;
use chain::BlockHeader;
use common::async_blocking;
use db_common::{sqlite::rusqlite::Error as SqlError,
                sqlite::rusqlite::{Connection, Row, ToSql, NO_PARAMS},
                sqlite::string_from_row,
                sqlite::validate_table_name,
                sqlite::CHECK_TABLE_EXISTS_SQL};
use primitives::hash::H256;
use spv_validation::storage::{BlockHeaderStorageError, BlockHeaderStorageOps};
use spv_validation::work::MAX_BITS_BTC;
use std::collections::HashMap;
use std::convert::TryInto;
use std::sync::{Arc, Mutex};

fn block_headers_cache_table(ticker: &str) -> String { ticker.to_owned() + "_block_headers_cache" }

fn get_table_name_and_validate(for_coin: &str) -> Result<String, BlockHeaderStorageError> {
    let table_name = block_headers_cache_table(for_coin);
    validate_table_name(&table_name).map_err(|e| BlockHeaderStorageError::CantRetrieveTableError {
        coin: for_coin.to_string(),
        reason: e.to_string(),
    })?;
    Ok(table_name)
}

fn create_block_header_cache_table_sql(for_coin: &str) -> Result<String, BlockHeaderStorageError> {
    let table_name = get_table_name_and_validate(for_coin)?;
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (
            block_height INTEGER NOT NULL UNIQUE,
            hex TEXT NOT NULL,
            block_bits INTEGER NOT NULL,
            block_hash VARCHAR(255) NOT NULL UNIQUE
        );",
        table_name
    );

    Ok(sql)
}

fn insert_block_header_in_cache_sql(for_coin: &str) -> Result<String, BlockHeaderStorageError> {
    let table_name = get_table_name_and_validate(for_coin)?;
    // Always update the block headers with new values just in case a chain reorganization occurs.
    let sql = format!(
        "INSERT OR REPLACE INTO {} (block_height, hex, block_bits, block_hash) VALUES (?1, ?2, ?3, ?4);",
        table_name
    );
    Ok(sql)
}

fn get_block_header_by_height(for_coin: &str) -> Result<String, BlockHeaderStorageError> {
    let table_name = get_table_name_and_validate(for_coin)?;
    let sql = format!("SELECT hex FROM {} WHERE block_height=?1;", table_name);

    Ok(sql)
}

fn get_last_block_header_with_non_max_bits_sql(for_coin: &str) -> Result<String, BlockHeaderStorageError> {
    let table_name = get_table_name_and_validate(for_coin)?;
    let sql = format!(
        "SELECT hex FROM {} WHERE block_bits<>{} ORDER BY block_height DESC LIMIT 1;",
        table_name, MAX_BITS_BTC
    );

    Ok(sql)
}

fn get_block_height_by_hash(for_coin: &str) -> Result<String, BlockHeaderStorageError> {
    let table_name = get_table_name_and_validate(for_coin)?;
    let sql = format!("SELECT block_height FROM {} WHERE block_hash=?1;", table_name);

    Ok(sql)
}

#[derive(Clone, Debug)]
pub struct SqliteBlockHeadersStorage(pub Arc<Mutex<Connection>>);

fn query_single_row<T, P, F>(
    conn: &Connection,
    query: &str,
    params: P,
    map_fn: F,
) -> Result<Option<T>, BlockHeaderStorageError>
where
    P: IntoIterator,
    P::Item: ToSql,
    F: FnOnce(&Row<'_>) -> Result<T, SqlError>,
{
    db_common::sqlite::query_single_row(conn, query, params, map_fn).map_err(|e| BlockHeaderStorageError::QueryError {
        query: query.to_string(),
        reason: e.to_string(),
    })
}

#[async_trait]
impl BlockHeaderStorageOps for SqliteBlockHeadersStorage {
    async fn init(&self, for_coin: &str) -> Result<(), BlockHeaderStorageError> {
        let selfi = self.clone();
        let sql_cache = create_block_header_cache_table_sql(for_coin)?;
        let coin = for_coin.to_owned();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            conn.execute(&sql_cache, NO_PARAMS).map(|_| ()).map_err(|e| {
                BlockHeaderStorageError::InitializationError {
                    coin,
                    reason: e.to_string(),
                }
            })?;
            Ok(())
        })
        .await
    }

    async fn is_initialized_for(&self, for_coin: &str) -> Result<bool, BlockHeaderStorageError> {
        let block_headers_cache_table = get_table_name_and_validate(for_coin)?;
        let selfi = self.clone();
        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            let cache_initialized = query_single_row(
                &conn,
                CHECK_TABLE_EXISTS_SQL,
                [block_headers_cache_table],
                string_from_row,
            )?;
            Ok(cache_initialized.is_some())
        })
        .await
    }

    async fn add_block_headers_to_storage(
        &self,
        for_coin: &str,
        headers: HashMap<u64, BlockHeader>,
    ) -> Result<(), BlockHeaderStorageError> {
        let for_coin = for_coin.to_owned();
        let selfi = self.clone();
        async_blocking(move || {
            let mut conn = selfi.0.lock().unwrap();
            let sql_transaction = conn
                .transaction()
                .map_err(|e| BlockHeaderStorageError::AddToStorageError {
                    coin: for_coin.to_string(),
                    reason: e.to_string(),
                })?;

            for (height, header) in headers {
                let height = height as i64;
                let hash = header.hash().reversed().to_string();
                let raw_header = hex::encode(header.raw());
                let bits: u32 = header.bits.into();
                let block_cache_params = [
                    &height as &dyn ToSql,
                    &raw_header as &dyn ToSql,
                    &bits as &dyn ToSql,
                    &hash as &dyn ToSql,
                ];
                sql_transaction
                    .execute(&insert_block_header_in_cache_sql(&for_coin)?, block_cache_params)
                    .map_err(|e| BlockHeaderStorageError::AddToStorageError {
                        coin: for_coin.to_string(),
                        reason: e.to_string(),
                    })?;
            }
            sql_transaction
                .commit()
                .map_err(|e| BlockHeaderStorageError::AddToStorageError {
                    coin: for_coin.to_string(),
                    reason: e.to_string(),
                })?;
            Ok(())
        })
        .await
    }

    async fn get_block_header(
        &self,
        for_coin: &str,
        height: u64,
    ) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        if let Some(header_raw) = self.get_block_header_raw(for_coin, height).await? {
            let header: BlockHeader =
                header_raw
                    .try_into()
                    .map_err(|e: serialization::Error| BlockHeaderStorageError::DecodeError {
                        coin: for_coin.to_string(),
                        reason: e.to_string(),
                    })?;
            return Ok(Some(header));
        }
        Ok(None)
    }

    async fn get_block_header_raw(
        &self,
        for_coin: &str,
        height: u64,
    ) -> Result<Option<String>, BlockHeaderStorageError> {
        let params = [height as i64];
        let sql = get_block_header_by_height(for_coin)?;
        let selfi = self.clone();

        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            query_single_row(&conn, &sql, params, string_from_row)
        })
        .await
        .map_err(|e| BlockHeaderStorageError::GetFromStorageError {
            coin: for_coin.to_string(),
            reason: e.to_string(),
        })
    }

    async fn get_last_block_header_with_non_max_bits(
        &self,
        for_coin: &str,
    ) -> Result<Option<BlockHeader>, BlockHeaderStorageError> {
        let sql = get_last_block_header_with_non_max_bits_sql(for_coin)?;
        let selfi = self.clone();

        let maybe_header_raw = async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            query_single_row(&conn, &sql, NO_PARAMS, string_from_row)
        })
        .await
        .map_err(|e| BlockHeaderStorageError::GetFromStorageError {
            coin: for_coin.to_string(),
            reason: e.to_string(),
        })?;

        if let Some(header_raw) = maybe_header_raw {
            let header: BlockHeader =
                header_raw
                    .try_into()
                    .map_err(|e: serialization::Error| BlockHeaderStorageError::DecodeError {
                        coin: for_coin.to_string(),
                        reason: e.to_string(),
                    })?;
            return Ok(Some(header));
        }
        Ok(None)
    }

    async fn get_block_height_by_hash(
        &self,
        for_coin: &str,
        hash: H256,
    ) -> Result<Option<i64>, BlockHeaderStorageError> {
        let params = [hash.to_string()];
        let sql = get_block_height_by_hash(for_coin)?;
        let selfi = self.clone();

        async_blocking(move || {
            let conn = selfi.0.lock().unwrap();
            query_single_row(&conn, &sql, params, |row| row.get(0))
        })
        .await
        .map_err(|e| BlockHeaderStorageError::GetFromStorageError {
            coin: for_coin.to_string(),
            reason: e.to_string(),
        })
    }
}

#[cfg(test)]
impl SqliteBlockHeadersStorage {
    pub fn in_memory() -> Self {
        SqliteBlockHeadersStorage(Arc::new(Mutex::new(Connection::open_in_memory().unwrap())))
    }

    fn is_table_empty(&self, table_name: &str) -> bool {
        validate_table_name(table_name).unwrap();
        let sql = "SELECT COUNT(block_height) FROM ".to_owned() + table_name + ";";
        let conn = self.0.lock().unwrap();
        let rows_count: u32 = conn.query_row(&sql, NO_PARAMS, |row| row.get(0)).unwrap();
        rows_count == 0
    }
}

#[cfg(test)]
mod sql_block_headers_storage_tests {
    use super::*;
    use chain::BlockHeaderBits;
    use common::block_on;
    use primitives::hash::H256;

    #[test]
    fn test_init_collection() {
        let for_coin = "init_collection";
        let storage = SqliteBlockHeadersStorage::in_memory();
        let initialized = block_on(storage.is_initialized_for(for_coin)).unwrap();
        assert!(!initialized);

        block_on(storage.init(for_coin)).unwrap();
        // repetitive init must not fail
        block_on(storage.init(for_coin)).unwrap();

        let initialized = block_on(storage.is_initialized_for(for_coin)).unwrap();
        assert!(initialized);
    }

    #[test]
    fn test_add_block_headers() {
        let for_coin = "insert";
        let storage = SqliteBlockHeadersStorage::in_memory();
        let table = block_headers_cache_table(for_coin);
        block_on(storage.init(for_coin)).unwrap();

        let initialized = block_on(storage.is_initialized_for(for_coin)).unwrap();
        assert!(initialized);

        let mut headers = HashMap::with_capacity(1);
        let block_header: BlockHeader = "0000002076d41d3e4b0bfd4c0d3b30aa69fdff3ed35d85829efd04000000000000000000b386498b583390959d9bac72346986e3015e83ac0b54bc7747a11a494ac35c94bb3ce65a53fb45177f7e311c".into();
        headers.insert(520481, block_header);
        block_on(storage.add_block_headers_to_storage(for_coin, headers)).unwrap();
        assert!(!storage.is_table_empty(&table));
    }

    #[test]
    fn test_get_block_header() {
        let for_coin = "get";
        let storage = SqliteBlockHeadersStorage::in_memory();
        let table = block_headers_cache_table(for_coin);
        block_on(storage.init(for_coin)).unwrap();

        let initialized = block_on(storage.is_initialized_for(for_coin)).unwrap();
        assert!(initialized);

        let mut headers = HashMap::with_capacity(1);
        let block_header: BlockHeader = "0000002076d41d3e4b0bfd4c0d3b30aa69fdff3ed35d85829efd04000000000000000000b386498b583390959d9bac72346986e3015e83ac0b54bc7747a11a494ac35c94bb3ce65a53fb45177f7e311c".into();
        headers.insert(520481, block_header);

        block_on(storage.add_block_headers_to_storage(for_coin, headers)).unwrap();
        assert!(!storage.is_table_empty(&table));

        let hex = block_on(storage.get_block_header_raw(for_coin, 520481))
            .unwrap()
            .unwrap();
        assert_eq!(hex, "0000002076d41d3e4b0bfd4c0d3b30aa69fdff3ed35d85829efd04000000000000000000b386498b583390959d9bac72346986e3015e83ac0b54bc7747a11a494ac35c94bb3ce65a53fb45177f7e311c".to_string());

        let block_header = block_on(storage.get_block_header(for_coin, 520481)).unwrap().unwrap();
        let block_hash: H256 = "0000000000000000002e31d0714a5ab23100945ff87ba2d856cd566a3c9344ec".into();
        assert_eq!(block_header.hash(), block_hash.reversed());

        let height = block_on(storage.get_block_height_by_hash(for_coin, block_hash))
            .unwrap()
            .unwrap();
        assert_eq!(height, 520481);
    }

    #[test]
    fn test_get_last_block_header_with_non_max_bits() {
        let for_coin = "get";
        let storage = SqliteBlockHeadersStorage::in_memory();
        let table = block_headers_cache_table(for_coin);
        block_on(storage.init(for_coin)).unwrap();

        let initialized = block_on(storage.is_initialized_for(for_coin)).unwrap();
        assert!(initialized);

        let mut headers = HashMap::with_capacity(2);

        // This block has max difficulty
        // https://live.blockcypher.com/btc-testnet/block/00000000961a9d117feb57e516e17217207a849bf6cdfce529f31d9a96053530/
        let block_header: BlockHeader = "02000000ea01a61a2d7420a1b23875e40eb5eb4ca18b378902c8e6384514ad0000000000c0c5a1ae80582b3fe319d8543307fa67befc2a734b8eddb84b1780dfdf11fa2b20e71353ffff001d00805fe0".into();
        headers.insert(201595, block_header);

        // https://live.blockcypher.com/btc-testnet/block/0000000000ad144538e6c80289378ba14cebb50ee47538b2a120742d1aa601ea/
        let expected_block_header: BlockHeader = "02000000cbed7fd98f1f06e85c47e13ff956533642056be45e7e6b532d4d768f00000000f2680982f333fcc9afa7f9a5e2a84dc54b7fe10605cd187362980b3aa882e9683be21353ab80011c813e1fc0".into();
        headers.insert(201594, expected_block_header.clone());

        // This block has max difficulty
        // https://live.blockcypher.com/btc-testnet/block/0000000000ad144538e6c80289378ba14cebb50ee47538b2a120742d1aa601ea/
        let block_header: BlockHeader = "020000001f38c8e30b30af912fbd4c3e781506713cfb43e73dff6250348e060000000000afa8f3eede276ccb4c4ee649ad9823fc181632f262848ca330733e7e7e541beb9be51353ffff001d00a63037".into();
        headers.insert(201593, block_header);

        block_on(storage.add_block_headers_to_storage(for_coin, headers)).unwrap();
        assert!(!storage.is_table_empty(&table));

        let actual_block_header = block_on(storage.get_last_block_header_with_non_max_bits(for_coin))
            .unwrap()
            .unwrap();
        assert_ne!(actual_block_header.bits, BlockHeaderBits::Compact(MAX_BITS_BTC.into()));
        assert_eq!(actual_block_header, expected_block_header);
    }
}
