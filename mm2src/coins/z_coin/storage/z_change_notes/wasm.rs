use super::{ChangeNote, ChangeNoteStorage, ChangeNoteStorageError};

use mm2_core::mm_ctx::MmArc;
use mm2_db::indexed_db::{ConstructibleDb, DbIdentifier, DbInstance, DbLocked, DbUpgrader, IndexedDb, IndexedDbBuilder,
                         InitDbResult, MultiIndex, OnUpgradeResult, TableSignature};
use mm2_err_handle::prelude::*;

const DB_NAME: &str = "z_change_note_storage";
const DB_VERSION: u32 = 1;

pub type ChangeNotesDbInnerLocked<'a> = DbLocked<'a, ChangeNoteDbInner>;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChangeNoteTable {
    address: String,
    hex: String,
    hex_bytes: Vec<u8>,
    change: u64,
}

impl ChangeNoteTable {
    pub const ADDRESS_HEX_BYTES_INDEX: &str = "address_hex_bytes_index";
}

impl TableSignature for ChangeNoteTable {
    const TABLE_NAME: &'static str = "change_notes";

    fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        if let (0, 1) = (old_version, new_version) {
            let table = upgrader.create_table(Self::TABLE_NAME)?;
            table.create_multi_index(Self::ADDRESS_HEX_BYTES_INDEX, &["address", "hex"], true)?;
            table.create_index("address", false)?;
            table.create_index("hex", true)?;
        }
        Ok(())
    }
}

pub struct ChangeNoteDbInner(IndexedDb);

#[async_trait::async_trait]
impl DbInstance for ChangeNoteDbInner {
    const DB_NAME: &'static str = DB_NAME;

    async fn init(db_id: DbIdentifier) -> InitDbResult<Self> {
        let inner = IndexedDbBuilder::new(db_id)
            .with_version(DB_VERSION)
            .with_table::<ChangeNoteTable>()
            .build()
            .await?;

        Ok(Self(inner))
    }
}

impl ChangeNoteDbInner {
    pub fn get_inner(&self) -> &IndexedDb { &self.0 }
}

impl ChangeNoteStorage {
    async fn lockdb(&self) -> MmResult<ChangeNotesDbInnerLocked, ChangeNoteStorageError> {
        Ok(self.db.get_or_initialize().await?)
    }
}

impl ChangeNoteStorage {
    pub(crate) async fn new(ctx: MmArc, address: String) -> Result<Self, ChangeNoteStorageError> {
        let db = ConstructibleDb::new(&ctx).into_shared();
        Ok(Self { address, db })
    }

    pub(crate) async fn insert_note(
        &self,
        hex: String,
        hex_bytes: Vec<u8>,
        change: u64,
    ) -> MmResult<(), ChangeNoteStorageError> {
        let db = self.lockdb().await?;
        let address = self.address.clone();
        let transaction = db.get_inner().transaction().await?;
        let change_note_table = transaction.table::<ChangeNoteTable>().await?;

        let indexes = MultiIndex::new(ChangeNoteTable::ADDRESS_HEX_BYTES_INDEX)
            .with_value(&address)?
            .with_value(&hex)?;

        let change_note = ChangeNoteTable {
            address,
            hex,
            hex_bytes,
            change,
        };

        Ok(change_note_table
            .add_item_or_ignore_by_unique_multi_index(indexes, &change_note)
            .await
            .map(|_| ())?)
    }

    pub(crate) async fn remove_note(&self, hex: String) -> MmResult<(), ChangeNoteStorageError> {
        let db = self.lockdb().await?;
        let transaction = db.get_inner().transaction().await?;
        let change_note_table = transaction.table::<ChangeNoteTable>().await?;

        change_note_table
            .delete_item_by_unique_index("hex", &hex)
            .await?
            .map(|_| ())
            .ok_or(MmError::new(ChangeNoteStorageError::IndexedDbError(format!(
                "change not found for tx bytes: {hex:?}"
            ))))
    }

    pub(crate) async fn load_all_notes(&self) -> MmResult<Vec<ChangeNote>, ChangeNoteStorageError> {
        let db = self.lockdb().await?;
        let transaction = db.get_inner().transaction().await?;
        let change_note_table = transaction.table::<ChangeNoteTable>().await?;

        Ok(change_note_table
            .get_items("address", &self.address)
            .await?
            .into_iter()
            .map(
                |(
                    _,
                    ChangeNoteTable {
                        hex, hex_bytes, change, ..
                    },
                )| ChangeNote { hex, hex_bytes, change },
            )
            .collect::<Vec<_>>())
    }

    pub(crate) async fn sum_changes(&self) -> MmResult<u64, ChangeNoteStorageError> {
        Ok(self.load_all_notes().await?.into_iter().map(|c| c.change).sum())
    }
}
