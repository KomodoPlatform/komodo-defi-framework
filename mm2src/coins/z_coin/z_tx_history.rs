use crate::my_tx_history_v2::MyTxHistoryErrorV2;
use crate::z_coin::storage::wasm::tables::{WalletDbAccountsTable, WalletDbBlocksTable, WalletDbReceivedNotesTable,
                                           WalletDbTransactionsTable};
use crate::z_coin::ZCoin;
use crate::MarketCoinOps;
use common::PagingOptionsEnum;
use mm2_db::indexed_db::cursor_prelude::CursorError;
use mm2_db::indexed_db::DbTransactionError;
use mm2_err_handle::prelude::MmError;
use mm2_number::BigInt;
use num_traits::ToPrimitive;

cfg_native!(
    use db_common::sqlite::sql_builder::{name, SqlBuilder, SqlName};
    use db_common::sqlite::rusqlite::Error as SqliteError;
    use db_common::sqlite::rusqlite::Row;
    use db_common::sqlite::offset_by_id;
    use common::async_blocking;
    use crate::z_coin::{BLOCKS_TABLE};
);

#[cfg(not(target_arch = "wasm32"))]
const TRANSACTIONS_TABLE: &str = "transactions";

pub(crate) enum ZTxHistoryError {
    #[cfg(not(target_arch = "wasm32"))]
    Sql(SqliteError),
    #[cfg(target_arch = "wasm32")]
    IndexedDbError(String),
    #[cfg(not(target_arch = "wasm32"))]
    FromIdDoesNotExist(i64),
}

impl From<ZTxHistoryError> for MyTxHistoryErrorV2 {
    fn from(err: ZTxHistoryError) -> Self {
        match err {
            #[cfg(not(target_arch = "wasm32"))]
            ZTxHistoryError::Sql(sql) => MyTxHistoryErrorV2::StorageError(sql.to_string()),
            ZTxHistoryError::FromIdDoesNotExist(id) => {
                MyTxHistoryErrorV2::StorageError(format!("from_id {} does not exist", id))
            },
            #[cfg(target_arch = "wasm32")]
            ZTxHistoryError::IndexedDbError(err) => MyTxHistoryErrorV2::StorageError(err),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<SqliteError> for ZTxHistoryError {
    fn from(err: SqliteError) -> Self { ZTxHistoryError::Sql(err) }
}

#[cfg(target_arch = "wasm32")]
impl From<DbTransactionError> for ZTxHistoryError {
    fn from(err: DbTransactionError) -> Self { ZTxHistoryError::IndexedDbError(err.to_string()) }
}

#[cfg(target_arch = "wasm32")]
impl From<CursorError> for ZTxHistoryError {
    fn from(err: CursorError) -> Self { ZTxHistoryError::IndexedDbError(err.to_string()) }
}

pub(crate) struct ZCoinTxHistoryItem {
    pub(crate) tx_hash: Vec<u8>,
    pub(crate) internal_id: i64,
    pub(crate) height: i64,
    pub(crate) timestamp: i64,
    pub(crate) received_amount: i64,
    pub(crate) spent_amount: i64,
}

#[cfg(not(target_arch = "wasm32"))]
impl ZCoinTxHistoryItem {
    fn try_from_sql_row(row: &Row<'_>) -> Result<Self, SqliteError> {
        let mut tx_hash: Vec<u8> = row.get(0)?;
        tx_hash.reverse();
        Ok(ZCoinTxHistoryItem {
            tx_hash,
            internal_id: row.get(1)?,
            height: row.get(2)?,
            timestamp: row.get(3)?,
            received_amount: row.get(4)?,
            spent_amount: row.get(5)?,
        })
    }
}

pub(crate) struct ZTxHistoryRes {
    pub(crate) transactions: Vec<ZCoinTxHistoryItem>,
    pub(crate) total_tx_count: u32,
    pub(crate) skipped: usize,
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn fetch_tx_history_from_db(
    z: &ZCoin,
    limit: usize,
    paging_options: PagingOptionsEnum<i64>,
) -> Result<ZTxHistoryRes, MmError<ZTxHistoryError>> {
    let wallet_db = z.z_fields.light_wallet_db.clone();
    let wallet_db = wallet_db.db.lock_db().await.unwrap();
    let db_transaction = wallet_db.get_inner().transaction().await?;
    let tx_table = db_transaction.table::<WalletDbTransactionsTable>().await?;
    let total_tx_count = tx_table.count_all().await? as u32;
    let offset = match paging_options {
        PagingOptionsEnum::FromId(from_address_id) => from_address_id + 1,
        PagingOptionsEnum::PageNumber(page_number) => ((page_number.get() - 1) * limit) as i64,
    };

    let txs = tx_table
        .cursor_builder()
        .only("ticker", &z.ticker())?
        .bound("height", 0u32, u32::MAX)
        .open_cursor(WalletDbAccountsTable::TICKER_ACCOUNT_INDEX)
        .await?
        .collect()
        .await?;

    let rn_table = db_transaction.table::<WalletDbReceivedNotesTable>().await?;
    let received_notes = rn_table
        .cursor_builder()
        .only("ticker", &z.ticker())?
        .bound("height", 0u32, u32::MAX)
        .open_cursor(WalletDbReceivedNotesTable::TICKER_ACCOUNT_INDEX)
        .await?
        .collect()
        .await?;

    // detect blocks
    let blocks_table = db_transaction.table::<WalletDbBlocksTable>().await?;
    let blocks = blocks_table
        .cursor_builder()
        .only("ticker", &z.ticker())?
        .open_cursor("ticker")
        .await?
        .collect()
        .await?;

    let mut tx_details = vec![];
    for (tx_id, tx) in txs {
        let height = blocks
            .iter()
            .find(|(_, block)| tx.block.map(|b| b == block.height).unwrap_or_default());
        if let Some((_, WalletDbBlocksTable { height, time, .. })) = height {
            let internal_id = tx_id;
            let mut tx_hash = tx.txid;
            let mut received_amount = 0;
            let mut spent_amount = 0;

            for (_, note) in &received_notes {
                if internal_id == note.tx {
                    received_amount += (*&note.value.to_u64().ok_or_else(|| {
                        ZTxHistoryError::IndexedDbError("Number is too large to fit in a u64".to_string())
                    })?) as i64;
                }

                // detecting spent amount by "spent" field in received_notes table
                if let Some(spent) = &note.spent {
                    if &BigInt::from(internal_id) == spent {
                        spent_amount += (*&note.value.to_u64().ok_or_else(|| {
                            ZTxHistoryError::IndexedDbError("Number is too large to fit in a u64".to_string())
                        })?) as i64;
                    }
                }
            }

            tx_hash.reverse();
            tx_details.push(ZCoinTxHistoryItem {
                tx_hash,
                internal_id: internal_id as i64,
                height: *height as i64,
                timestamp: *time as i64,
                received_amount,
                spent_amount,
            });
        }
    }

    Ok(ZTxHistoryRes {
        transactions: tx_details,
        total_tx_count,
        skipped: offset as usize,
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn fetch_tx_history_from_db(
    z: &ZCoin,
    limit: usize,
    paging_options: PagingOptionsEnum<i64>,
) -> Result<ZTxHistoryRes, MmError<ZTxHistoryError>> {
    let wallet_db = z.z_fields.light_wallet_db.clone();
    async_blocking(move || {
        let db_guard = wallet_db.db.inner();
        let db_guard = db_guard.lock().unwrap();
        let conn = db_guard.sql_conn();

        let total_sql = SqlBuilder::select_from(TRANSACTIONS_TABLE)
            .field("COUNT(id_tx)")
            .sql()
            .expect("valid SQL");
        let total_tx_count = conn.query_row(&total_sql, [], |row| row.get(0))?;

        let mut sql_builder = SqlBuilder::select_from(name!(TRANSACTIONS_TABLE; "txes"));
        sql_builder
            .field("txes.txid")
            .field("txes.id_tx as internal_id")
            .field("txes.block as block");

        let offset = match paging_options {
            PagingOptionsEnum::PageNumber(page) => (page.get() - 1) * limit,
            PagingOptionsEnum::FromId(id) => {
                offset_by_id(conn, &sql_builder, [id], "id_tx", "block DESC, id_tx ASC", "id_tx = ?1")?
                    .ok_or(ZTxHistoryError::FromIdDoesNotExist(id))?
            },
        };

        let sql = sql_builder
            .field("blocks.time")
            .field("COALESCE(rn.received_amount, 0)")
            .field("COALESCE(sn.sent_amount, 0)")
            .left()
            .join("(SELECT tx, SUM(value) as received_amount FROM received_notes GROUP BY tx) as rn")
            .on("txes.id_tx = rn.tx")
            // detecting spent amount by "spent" field in received_notes table
            .join("(SELECT spent, SUM(value) as sent_amount FROM received_notes GROUP BY spent) as sn")
            .on("txes.id_tx = sn.spent")
            .join(BLOCKS_TABLE)
            .on("txes.block = blocks.height")
            .group_by("internal_id")
            .order_by("block", true)
            .order_by("internal_id", false)
            .offset(offset)
            .limit(limit)
            .sql()
            .expect("valid query");

        let sql_items = conn
            .prepare(&sql)?
            .query_map([], ZCoinTxHistoryItem::try_from_sql_row)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ZTxHistoryRes {
            transactions: sql_items,
            total_tx_count,
            skipped: offset,
        })
    })
    .await
}
