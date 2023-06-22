use crate::nft::storage::wasm::wasm_storage::{NftListTable, NftTxHistoryTable};
use async_trait::async_trait;
use mm2_db::indexed_db::InitDbResult;
use mm2_db::indexed_db::{DbIdentifier, DbInstance, DbLocked, IndexedDb, IndexedDbBuilder};

const DB_NAME: &str = "nft_cache";
const DB_VERSION: u32 = 1;
pub type NftCacheIDBLocked<'a> = DbLocked<'a, NftCacheIDB>;

pub struct NftCacheIDB {
    inner: IndexedDb,
}

#[async_trait]
impl DbInstance for NftCacheIDB {
    fn db_name() -> &'static str { DB_NAME }

    async fn init(db_id: DbIdentifier) -> InitDbResult<Self> {
        let inner = IndexedDbBuilder::new(db_id)
            .with_version(DB_VERSION)
            .with_table::<NftListTable>()
            .with_table::<NftTxHistoryTable>()
            .build()
            .await?;
        Ok(NftCacheIDB { inner })
    }
}

#[allow(dead_code)]
impl NftCacheIDB {
    fn get_inner(&self) -> &IndexedDb { &self.inner }
}
