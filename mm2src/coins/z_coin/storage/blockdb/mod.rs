#[cfg(target_arch = "wasm32")] pub(crate) mod block_idb;

use mm2_core::mm_ctx::MmArc;
use std::path::Path;
use zcash_client_backend::data_api::BlockSource;
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_primitives::consensus::BlockHeight;

cfg_native!(
    use db_common::sqlite::rusqlite::{params, Connection};
    use db_common::sqlite::{query_single_row, run_optimization_pragmas};
    use protobuf::Message;
    use mm2_err_handle::prelude::*;
    use std::sync::{Arc, Mutex};
    use zcash_client_sqlite::error::{SqliteClientError as ZcashClientError, SqliteClientError};
    use zcash_client_sqlite::NoteId;
    use zcash_client_backend::data_api::error::Error as ChainError;

    struct CompactBlockRow {
        height: BlockHeight,
        data: Vec<u8>,
    }
);

#[derive(Debug, Display)]
pub enum BlockDbError {
    #[cfg(not(target_arch = "wasm32"))]
    SqliteError(SqliteClientError),
    #[cfg(target_arch = "wasm32")]
    IndexedDBError(String),
    CorruptedData(String),
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqliteClientError> for BlockDbError {
    fn from(value: SqliteClientError) -> Self { Self::SqliteError(value) }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<ChainError<NoteId>> for BlockDbError {
    fn from(value: ChainError<NoteId>) -> Self { Self::SqliteError(SqliteClientError::from(value)) }
}

/// A wrapper for the db connection to the block cache database.
pub struct BlockDbImpl {
    #[cfg(not(target_arch = "wasm32"))]
    pub db: Arc<Mutex<Connection>>,
    #[cfg(target_arch = "wasm32")]
    pub db: SharedDb<BlockDbInner>,
    #[allow(unused)]
    ticker: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl BlockDbImpl {
    pub async fn new(_ctx: MmArc, ticker: String, path: impl AsRef<Path>) -> MmResult<Self, BlockDbError> {
        let conn = Connection::open(path).map_err(|err| BlockDbError::SqliteError(SqliteClientError::from(err)))?;
        run_optimization_pragmas(&conn).map_err(|err| BlockDbError::SqliteError(SqliteClientError::from(err)))?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS compactblocks (
            height INTEGER PRIMARY KEY,
            data BLOB NOT NULL
        )",
            [],
        )
        .map_to_mm(|err| BlockDbError::SqliteError(SqliteClientError::from(err)))?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            ticker,
        })
    }

    pub(crate) fn get_latest_block(&self) -> Result<u32, ZcashClientError> {
        Ok(query_single_row(
            &self.db.lock().unwrap(),
            "SELECT height FROM compactblocks ORDER BY height DESC LIMIT 1",
            [],
            |row| row.get(0),
        )?
        .unwrap_or(0))
    }

    pub(crate) fn insert_block(&self, height: u32, cb_bytes: Vec<u8>) -> Result<usize, BlockDbError> {
        self.db
            .lock()
            .unwrap()
            .prepare("INSERT INTO compactblocks (height, data) VALUES (?, ?)")
            .map_err(|err| BlockDbError::SqliteError(SqliteClientError::from(err)))?
            .execute(params![height, cb_bytes])
            .map_err(|err| BlockDbError::SqliteError(SqliteClientError::from(err)))
    }

    pub(crate) fn rewind_to_height(&self, height: u32) -> Result<usize, BlockDbError> {
        self.db
            .lock()
            .unwrap()
            .execute("DELETE from compactblocks WHERE height > ?1", [height])
            .map_err(|err| BlockDbError::SqliteError(SqliteClientError::from(err)))
    }

    fn with_blocks<F>(
        &self,
        from_height: BlockHeight,
        limit: Option<u32>,
        mut with_row: F,
    ) -> Result<(), SqliteClientError>
    where
        F: FnMut(CompactBlock) -> Result<(), SqliteClientError>,
    {
        // Fetch the CompactBlocks we need to scan
        let stmt_blocks = self.db.lock().unwrap();
        let mut stmt_blocks = stmt_blocks.prepare(
            "SELECT height, data FROM compactblocks WHERE height > ? ORDER BY height ASC \
        LIMIT ?",
        )?;

        let rows = stmt_blocks.query_map(
            params![u32::from(from_height), limit.unwrap_or(u32::max_value()),],
            |row| {
                Ok(CompactBlockRow {
                    height: BlockHeight::from_u32(row.get(0)?),
                    data: row.get(1)?,
                })
            },
        )?;

        for row_result in rows {
            let cbr = row_result?;
            let block = CompactBlock::parse_from_bytes(&cbr.data).map_err(ChainError::from)?;

            if block.height() != cbr.height {
                return Err(SqliteClientError::CorruptedData(format!(
                    "Block height {} did not match row's height field value {}",
                    block.height(),
                    cbr.height
                )));
            }

            with_row(block)?;
        }

        Ok(())
    }

    pub(crate) async fn get_earliest_block(&self) -> Result<u32, ZcashClientError> {
        Ok(query_single_row(
            &self.db.lock().unwrap(),
            "SELECT MIN(height) from compactblocks",
            [],
            |row| row.get::<_, Option<u32>>(0),
        )?
        .flatten()
        .unwrap_or(0))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl BlockSource for BlockDbImpl {
    type Error = SqliteClientError;

    fn with_blocks<F>(&self, from_height: BlockHeight, limit: Option<u32>, with_row: F) -> Result<(), Self::Error>
    where
        F: FnMut(CompactBlock) -> Result<(), Self::Error>,
    {
        self.with_blocks(from_height, limit, with_row)
    }
}

cfg_wasm32!(
    use crate::z_coin::storage::blockdb::block_idb::BlockDbInner;
    use mm2_db::indexed_db::{ConstructibleDb, DbLocked, SharedDb};
    use mm2_err_handle::prelude::*;

    pub type BlockDbRes<T> = MmResult<T, BlockDbError>;
    pub type BlockDbInnerLocked<'a> = DbLocked<'a, BlockDbInner>;

    impl BlockDbImpl {
        pub async fn new(ctx: MmArc, ticker: String, _path: impl AsRef<Path>) -> Result<Self, BlockDbError> {
            Ok(Self {
                db: ConstructibleDb::new(&ctx).into_shared(),
                ticker,
            })
        }

        #[allow(unused)]
        async fn lock_db(&self) -> BlockDbRes<BlockDbInnerLocked<'_>> {
            self.db
            .get_or_initialize()
            .await
            .mm_err(|err| BlockDbError::IndexedDBError(err.to_string()))
        }

        pub fn get_latest_block(&self) -> Result<u32, BlockDbError> { todo!() }

        pub fn insert_block(&self, _height: u32, _cb_bytes: Vec<u8>) -> Result<usize, BlockDbError> { todo!() }

        pub fn rewind_to_height(&self, _height: u32) -> Result<usize, BlockDbError> { todo!() }

        pub fn with_blocks<F>(&self, _from_height: BlockHeight, _limit: Option<u32>, mut _with_row: F) -> Result<(),
        BlockDbError>
        where F: FnMut(CompactBlock) -> Result<(), BlockDbError>
        { todo!() }
    }

    impl BlockSource for BlockDbImpl {
        type Error = BlockDbError;
        fn with_blocks<F>(&self, _from_height: BlockHeight, _limit: Option<u32>, _with_row: F) -> Result<(),
        Self::Error>
        where F: FnMut(CompactBlock) -> Result<(), Self::Error>,
        { todo!() }
    }
);
