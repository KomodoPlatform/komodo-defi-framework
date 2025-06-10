use super::{LockedNote, LockedNotesStorage, LockedNotesStorageError};

use mm2_core::mm_ctx::MmArc;
use mm2_db::indexed_db::{ConstructibleDb, DbIdentifier, DbInstance, DbLocked, DbUpgrader, IndexedDb, IndexedDbBuilder,
                         InitDbResult, OnUpgradeResult, TableSignature, OnUpgradeError};
use mm2_err_handle::prelude::*;

const DB_NAME: &str = "z_change_note_storage";
const DB_VERSION: u32 = 1;

pub type LockedNotesDbInnerLocked<'a> = DbLocked<'a, LockedNoteDbInner>;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LockedNoteTable {
    address: String,
    hex: String,
    rseed: String,
}

impl TableSignature for LockedNoteTable {
    const TABLE_NAME: &'static str = "change_notes";

    fn on_upgrade_needed(upgrader: &DbUpgrader, mut old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
        while old_version < new_version {
            match old_version {
                0 => {
                    let table = upgrader.create_table(Self::TABLE_NAME)?;
                    table.create_index("address", false)?;
                    table.create_index("rseed", true)?;
                    table.create_index("hex", false)?;
                }
                unsupported_version => {
                    return MmError::err(OnUpgradeError::UnsupportedVersion {
                        unsupported_version,
                        old_version,
                        new_version,
                    });
                }
            }
            old_version += 1;
        }
        Ok(())
    }
}

pub struct LockedNoteDbInner(IndexedDb);

#[async_trait::async_trait]
impl DbInstance for LockedNoteDbInner {
    const DB_NAME: &'static str = DB_NAME;

    async fn init(db_id: DbIdentifier) -> InitDbResult<Self> {
        let inner = IndexedDbBuilder::new(db_id)
            .with_version(DB_VERSION)
            .with_table::<LockedNoteTable>()
            .build()
            .await?;

        Ok(Self(inner))
    }
}

impl LockedNoteDbInner {
    pub fn get_inner(&self) -> &IndexedDb { &self.0 }
}

impl LockedNotesStorage {
    async fn lockdb(&self) -> MmResult<LockedNotesDbInnerLocked, LockedNotesStorageError> {
        Ok(self.db.get_or_initialize().await?)
    }
}

impl LockedNotesStorage {
    pub(crate) async fn new(ctx: &MmArc, address: String) -> Result<Self, LockedNotesStorageError> {
        let db = ConstructibleDb::new(ctx).into_shared();
        Ok(Self { address, db })
    }

    pub(crate) async fn insert_note(&self, hex: String, rseed: String) -> MmResult<(), LockedNotesStorageError> {
        let db = self.lockdb().await?;
        let address = self.address.clone();
        let transaction = db.get_inner().transaction().await?;
        let change_note_table = transaction.table::<LockedNoteTable>().await?;

        let change_note = LockedNoteTable {
            address,
            hex,
            rseed: rseed.clone(),
        };

        Ok(change_note_table
            .add_item_or_ignore_by_unique_index("rseed", &rseed, &change_note)
            .await
            .map(|_| ())?)
    }

    pub(crate) async fn remove_note(&self, hex: String) -> MmResult<(), LockedNotesStorageError> {
        common::log::info!("unlocking {hex} notes");
        let db = self.lockdb().await?;
        let transaction = db.get_inner().transaction().await?;
        let change_note_table = transaction.table::<LockedNoteTable>().await?;

        change_note_table
            .delete_item_by_unique_index("hex", &hex)
            .await?
            .map(|_| ())
            .ok_or(MmError::new(LockedNotesStorageError::IndexedDbError(format!(
                "change not found for tx bytes: {hex:?}"
            ))))
    }

    pub(crate) async fn load_all_notes(&self) -> MmResult<Vec<LockedNote>, LockedNotesStorageError> {
        let db = self.lockdb().await?;
        let transaction = db.get_inner().transaction().await?;
        let change_note_table = transaction.table::<LockedNoteTable>().await?;

        Ok(change_note_table
            .get_items("address", &self.address)
            .await?
            .into_iter()
            .map(|(_, n)| LockedNote { rseed: n.rseed })
            .collect::<Vec<_>>())
    }
}
