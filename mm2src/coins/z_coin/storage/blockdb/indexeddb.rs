use async_trait::async_trait;
use mm2_db::indexed_db::{BeBigUint, DbIdentifier, DbInstance, DbLocked, DbUpgrader, IndexedDb, IndexedDbBuilder,
                         InitDbResult, OnUpgradeResult, TableSignature};

const DB_NAME: &str = "z_compactblocks_cache";
const DB_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BlockDbTable {
    height: BeBigUint,
    data: Vec<u8>,
}

impl BlockDbTable {
    pub const BLOCK_HEIGHT_INDEX: &str = "block_height_index";
}

impl TableSignature for BlockDbTable {
    fn table_name() -> &'static str { "compactblocks" }

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::table_name())?;
            table.create_index("BLOCK_HEIGHT_INDEX", true)?;
        }
        Ok(())
    }
}

pub struct BlockDbInner {
    pub inner: IndexedDb,
}

#[async_trait]
impl DbInstance for BlockDbInner {
    fn db_name() -> &'static str { DB_NAME }

    async fn init(db_id: DbIdentifier) -> InitDbResult<Self> {
        let inner = IndexedDbBuilder::new(db_id)
            .with_version(DB_VERSION)
            .with_table::<BlockDbTable>()
            .build()
            .await?;

        Ok(Self { inner })
    }
}
