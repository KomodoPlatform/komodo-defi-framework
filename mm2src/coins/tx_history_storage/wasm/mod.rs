use crate::my_tx_history_v2::TxHistoryStorageError;
use mm2_db::indexed_db::{DbTransactionError, InitDbError};
use mm2_err_handle::prelude::*;

pub mod tx_history_db;
pub mod tx_history_storage_v1;
pub mod tx_history_storage_v2;

pub use tx_history_db::{TxHistoryDb, TxHistoryDbLocked};
pub use tx_history_storage_v1::{clear_tx_history, load_tx_history, save_tx_history};
pub use tx_history_storage_v2::IndexedDbTxHistoryStorage;

pub type WasmTxHistoryResult<T> = MmResult<T, WasmTxHistoryError>;
pub type WasmTxHistoryError = crate::TxHistoryError;

impl TxHistoryStorageError for WasmTxHistoryError {}

impl From<InitDbError> for WasmTxHistoryError {
    fn from(e: InitDbError) -> Self {
        match &e {
            InitDbError::NotSupported(_) => WasmTxHistoryError::NotSupported(e.to_string()),
            InitDbError::EmptyTableList
            | InitDbError::DbIsOpenAlready { .. }
            | InitDbError::InvalidVersion(_)
            | InitDbError::OpeningError(_)
            | InitDbError::TypeMismatch { .. }
            | InitDbError::UnexpectedState(_)
            | InitDbError::UpgradingError { .. } => WasmTxHistoryError::InternalError(e.to_string()),
        }
    }
}

impl From<DbTransactionError> for WasmTxHistoryError {
    fn from(e: DbTransactionError) -> Self {
        match e {
            DbTransactionError::ErrorSerializingItem(_) => WasmTxHistoryError::ErrorSerializing(e.to_string()),
            DbTransactionError::ErrorDeserializingItem(_) => WasmTxHistoryError::ErrorDeserializing(e.to_string()),
            DbTransactionError::ErrorUploadingItem(_) => WasmTxHistoryError::ErrorSaving(e.to_string()),
            DbTransactionError::ErrorGettingItems(_) | DbTransactionError::ErrorCountingItems(_) => {
                WasmTxHistoryError::ErrorLoading(e.to_string())
            },
            DbTransactionError::ErrorDeletingItems(_) => WasmTxHistoryError::ErrorClearing(e.to_string()),
            DbTransactionError::NoSuchTable { .. }
            | DbTransactionError::ErrorCreatingTransaction(_)
            | DbTransactionError::ErrorOpeningTable { .. }
            | DbTransactionError::ErrorSerializingIndex { .. }
            | DbTransactionError::UnexpectedState(_)
            | DbTransactionError::TransactionAborted
            | DbTransactionError::MultipleItemsByUniqueIndex { .. }
            | DbTransactionError::NoSuchIndex { .. }
            | DbTransactionError::InvalidIndex { .. } => WasmTxHistoryError::InternalError(e.to_string()),
        }
    }
}
