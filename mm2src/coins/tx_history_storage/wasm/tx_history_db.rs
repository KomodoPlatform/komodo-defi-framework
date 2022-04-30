use crate::tx_history_storage::wasm::tx_history_storage_v1::TxHistoryTableV1;
use async_trait::async_trait;
use common::indexed_db::{DbIdentifier, DbInstance, IndexedDb, IndexedDbBuilder, InitDbResult};

const DB_NAME: &str = "tx_history";
const DB_VERSION: u32 = 1;

pub struct TxHistoryDb {
    inner: IndexedDb,
}

#[async_trait]
impl DbInstance for TxHistoryDb {
    fn db_name() -> &'static str { DB_NAME }

    async fn init(db_id: DbIdentifier) -> InitDbResult<Self> {
        let inner = IndexedDbBuilder::new(db_id)
            .with_version(DB_VERSION)
            .with_table::<TxHistoryTableV1>()
            .build()
            .await?;
        Ok(TxHistoryDb { inner })
    }
}

impl TxHistoryDb {
    pub fn get_inner(&self) -> &IndexedDb { &self.inner }
}
