use async_trait::async_trait;
use mm2_db::indexed_db::{DbIdentifier, DbInstance, DbUpgrader, IndexedDb, IndexedDbBuilder, OnUpgradeResult,
                         TableSignature};
use std::ops::Deref;
use uuid::Uuid;

pub use mm2_db::indexed_db::{cursor_prelude, DbTransactionError, DbTransactionResult, InitDbError, InitDbResult,
                             ItemId};
pub use tables::{MySwapsFiltersTable, SavedSwapTable, SwapLockTable};

const DB_NAME: &str = "swap";
const DB_VERSION: u32 = 1;

pub struct SwapDb {
    inner: IndexedDb,
}

#[async_trait]
impl DbInstance for SwapDb {
    fn db_name() -> &'static str { DB_NAME }

    async fn init(db_id: DbIdentifier) -> InitDbResult<Self> {
        let inner = IndexedDbBuilder::new(db_id)
            .with_version(DB_VERSION)
            .with_table::<SwapLockTable>()
            .with_table::<SavedSwapTable>()
            .with_table::<MySwapsFiltersTable>()
            .build()
            .await?;
        Ok(SwapDb { inner })
    }
}

impl Deref for SwapDb {
    type Target = IndexedDb;

    fn deref(&self) -> &Self::Target { &self.inner }
}

pub mod tables {
    use super::*;
    use serde_json::Value as Json;

    #[derive(Debug, Deserialize, Clone, Serialize, PartialEq)]
    pub struct SwapLockTable {
        pub uuid: Uuid,
        pub timestamp: u64,
    }

    impl TableSignature for SwapLockTable {
        fn table_name() -> &'static str { "swap_lock" }

        fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
            on_upgrade_swap_table_by_uuid_v1(upgrader, old_version, new_version, Self::table_name())
        }
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    pub struct SavedSwapTable {
        pub uuid: Uuid,
        pub saved_swap: Json,
    }

    impl TableSignature for SavedSwapTable {
        fn table_name() -> &'static str { "saved_swap" }

        fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
            on_upgrade_swap_table_by_uuid_v1(upgrader, old_version, new_version, Self::table_name())
        }
    }

    /// This table is used to select uuids applying given filters.
    /// When we iterate over an index like `["my_coin", "other_coin"]`, a cursor returns items with all fields.
    /// So, if we combine `SavedSwapTable` and `MySwapsFiltersTable` into one, we will get `saved_swap` on every cursor callback that is overhead.
    #[derive(Debug, Serialize, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
    pub struct MySwapsFiltersTable {
        pub uuid: Uuid,
        pub my_coin: String,
        pub other_coin: String,
        pub started_at: u32,
    }

    impl TableSignature for MySwapsFiltersTable {
        fn table_name() -> &'static str { "my_swaps" }

        fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
            if let (0, 1) = (old_version, new_version) {
                let table = upgrader.create_table(Self::table_name())?;
                table.create_index("uuid", true)?;
                table.create_index("started_at", false)?;
                table.create_multi_index("with_my_coin", &["my_coin", "started_at"], false)?;
                table.create_multi_index("with_other_coin", &["other_coin", "started_at"], false)?;
                table.create_multi_index("with_my_other_coins", &["my_coin", "other_coin", "started_at"], false)?;
            }
            Ok(())
        }
    }

    /// [`TableSignature::on_upgrade_needed`] implementation common for the most tables with the only `uuid` unique index.
    fn on_upgrade_swap_table_by_uuid_v1(
        upgrader: &DbUpgrader,
        old_version: u32,
        new_version: u32,
        table_name: &'static str,
    ) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(table_name)?;
            table.create_index("uuid", true)?;
        }
        Ok(())
    }
}
