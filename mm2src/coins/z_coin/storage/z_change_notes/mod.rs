use enum_derives::EnumFromStringify;

cfg_native!(
    pub(crate) mod sqlite;

    use db_common::async_sql_conn::{AsyncConnError, AsyncConnection};
    use futures::lock::Mutex;
    use std::sync::Arc;
);

cfg_wasm32!(
    pub(crate) mod wasm;

    use self::wasm::ChangeNoteDbInner;
    use mm2_db::indexed_db::{DbTransactionError, InitDbError, SharedDb};
);

#[derive(Debug, Clone)]
pub(crate) struct ChangeNote {
    pub(crate) hex_bytes: Vec<u8>,
    pub(crate) hex: String,
    pub(crate) change: u64,
}

/// A wrapper for the db connection to the change note cache database in native and browser.
#[derive(Clone)]
pub(crate) struct ChangeNoteStorage {
    #[cfg(not(target_arch = "wasm32"))]
    pub db: Arc<Mutex<AsyncConnection>>,
    #[cfg(target_arch = "wasm32")]
    pub db: SharedDb<ChangeNoteDbInner>,
    address: String,
}

#[derive(Clone, Debug, Display, Eq, PartialEq, EnumFromStringify)]
pub(crate) enum ChangeNoteStorageError {
    #[cfg(not(target_arch = "wasm32"))]
    #[display(fmt = "Sqlite Error: {_0}")]
    #[from_stringify("AsyncConnError", "db_common::sqlite::rusqlite::Error")]
    SqliteError(String),
    #[cfg(target_arch = "wasm32")]
    #[display(fmt = "IndexedDb Error: {_0}")]
    #[from_stringify("InitDbError", "DbTransactionError")]
    IndexedDbError(String),
}

#[cfg(test)]
pub(super) mod change_note_test {
    use crate::z_coin::storage::z_change_notes::ChangeNoteStorage;

    use mm2_test_helpers::for_tests::mm_ctx_with_custom_db;

    const MY_ADDRESS: &str = "my_address";

    pub(crate) async fn test_insert_and_remove_note_impl() {
        let ctx = mm_ctx_with_custom_db();
        let db = ChangeNoteStorage::new(ctx, MY_ADDRESS.to_string()).await.unwrap();

        // insert note
        let result = db.insert_note("some_hex".to_string(), vec![0, 4, 1, 0], 500).await;
        assert!(result.is_ok());

        // remove note
        let result = db.remove_note("some_hex".to_owned()).await;
        assert!(result.is_ok());

        // get notes(should be empty)
        let notes = db.load_all_notes().await.unwrap();
        assert!(notes.is_empty());
    }

    pub(crate) async fn test_load_all_notes_impl() {
        let ctx = mm_ctx_with_custom_db();
        let db = ChangeNoteStorage::new(ctx, MY_ADDRESS.to_string()).await.unwrap();

        let result = db.insert_note("some_hex".to_string(), vec![0, 4, 1, 0], 500).await;
        assert!(result.is_ok());

        let notes = db.load_all_notes().await.unwrap();
        assert!(notes.len() == 1);
    }

    pub(crate) async fn test_sum_changes_impl() {
        let ctx = mm_ctx_with_custom_db();
        let db = ChangeNoteStorage::new(ctx, MY_ADDRESS.to_string()).await.unwrap();

        let result = db.insert_note("some_hex".to_string(), vec![0, 4, 1, 0], 500).await;
        assert!(result.is_ok());

        let result = db.insert_note("another_hex".to_string(), vec![1, 4, 1, 0], 500).await;
        assert!(result.is_ok());

        let changes = db.sum_changes().await.unwrap();
        assert_eq!(changes, 1000);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod change_note_native_tests {
    use common::block_on;

    use super::change_note_test::{test_insert_and_remove_note_impl, test_load_all_notes_impl, test_sum_changes_impl};

    #[test]
    fn test_insert_and_remove_note() { block_on(test_insert_and_remove_note_impl()) }

    #[test]
    fn test_load_all_notes() { block_on(test_load_all_notes_impl()) }

    #[test]
    fn test_sum_changes() { block_on(test_sum_changes_impl()) }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod change_note_native_tests {
    use super::change_note_test::{test_insert_and_remove_note_impl, test_load_all_notes_impl, test_sum_changes_impl};
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    async fn test_insert_and_remove_note() { test_insert_and_remove_note_impl().await }

    #[wasm_bindgen_test]
    async fn test_load_all_notes() { test_load_all_notes_impl().await }

    #[wasm_bindgen_test]
    async fn test_sum_changes() { test_sum_changes_impl().await }
}
