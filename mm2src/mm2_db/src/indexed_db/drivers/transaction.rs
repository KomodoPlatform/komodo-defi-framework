use super::{construct_event_closure, IdbObjectStoreImpl};
use common::stringify_js_error;
use derive_more::Display;
use futures::channel::{mpsc, oneshot};
use futures::StreamExt;
use mm2_ehandle::mm_error::prelude::*;
use serde_json::Value as Json;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::IdbTransaction;

pub type DbTransactionResult<T> = Result<T, MmError<DbTransactionError>>;

#[derive(Debug, Display, PartialEq)]
pub enum DbTransactionError {
    #[display(fmt = "No such table '{}'", table)]
    NoSuchTable { table: String },
    #[display(fmt = "Error creating DbTransaction: {:?}", _0)]
    ErrorCreatingTransaction(String),
    #[display(fmt = "Error opening the '{}' table: {}", table, description)]
    ErrorOpeningTable { table: String, description: String },
    #[display(fmt = "Error serializing the '{}' index: {:?}", index, description)]
    ErrorSerializingIndex { index: String, description: String },
    #[display(fmt = "Error serializing an item: {:?}", _0)]
    ErrorSerializingItem(String),
    #[display(fmt = "Error deserializing an item: {:?}", _0)]
    ErrorDeserializingItem(String),
    #[display(fmt = "Error uploading an item: {:?}", _0)]
    ErrorUploadingItem(String),
    #[display(fmt = "Error getting items: {:?}", _0)]
    ErrorGettingItems(String),
    #[display(fmt = "Error deleting items: {:?}", _0)]
    ErrorDeletingItems(String),
    #[display(fmt = "Expected only one item by the unique '{}' index, got {}", index, got_items)]
    MultipleItemsByUniqueIndex { index: String, got_items: usize },
    #[display(fmt = "No such index '{}'", index)]
    NoSuchIndex { index: String },
    #[display(fmt = "Invalid index '{}:{}': {:?}", index, index_value, description)]
    InvalidIndex {
        index: String,
        index_value: Json,
        description: String,
    },
    #[display(fmt = "Error occurred due to an unexpected state: {:?}", _0)]
    UnexpectedState(String),
    #[display(fmt = "Transaction was aborted")]
    TransactionAborted,
}

pub struct IdbTransactionImpl {
    transaction: IdbTransaction,
    tables: HashSet<String>,
    aborted: Arc<AtomicBool>,
    complete_rx: oneshot::Receiver<Result<JsValue, JsValue>>,
}

impl !Send for IdbTransactionImpl {}

impl IdbTransactionImpl {
    pub fn aborted(&self) -> bool { self.aborted.load(Ordering::Relaxed) }

    pub fn open_table(&self, table_name: &str) -> DbTransactionResult<IdbObjectStoreImpl> {
        if self.aborted.load(Ordering::Relaxed) {
            return MmError::err(DbTransactionError::TransactionAborted);
        }

        if !self.tables.contains(table_name) {
            let table = table_name.to_owned();
            return MmError::err(DbTransactionError::NoSuchTable { table });
        }

        match self.transaction.object_store(table_name) {
            Ok(object_store) => Ok(IdbObjectStoreImpl {
                object_store,
                aborted: self.aborted.clone(),
            }),
            Err(e) => MmError::err(DbTransactionError::ErrorOpeningTable {
                table: table_name.to_owned(),
                description: stringify_js_error(&e),
            }),
        }
    }

    pub async fn wait_for_complete(self) -> DbTransactionResult<()> {
        if self.aborted.load(Ordering::Relaxed) {
            return MmError::err(DbTransactionError::TransactionAborted);
        }

        let result = self.complete_rx.await.expect("The complete channel must not be closed");
        match result {
            Ok(_event) => Ok(()),
            Err(_error_event) => return MmError::err(DbTransactionError::TransactionAborted),
        }
    }

    pub fn init(transaction: IdbTransaction, tables: HashSet<String>) -> IdbTransactionImpl {
        let (complete_tx, complete_rx) = oneshot::channel();
        let (event_tx, mut event_rx) = mpsc::channel(2);

        let oncomplete_closure = construct_event_closure(Ok, event_tx.clone());
        let onabort_closure = construct_event_closure(Err, event_tx.clone());

        transaction.set_oncomplete(Some(oncomplete_closure.as_ref().unchecked_ref()));
        // Don't set the `onerror` closure, because the `onabort` is called immediately after the error.
        transaction.set_onabort(Some(onabort_closure.as_ref().unchecked_ref()));

        // move the closures into this async block to keep it alive until either `oncomplete` or `onabort` handler is called
        let fut = async move {
            let complete_or_abort = event_rx.next().await.expect("The event channel must not be closed");
            // ignore if the receiver is closed
            let _res = complete_tx.send(complete_or_abort);

            // do any action to move the closures into this async block to keep it alive until the `state_machine` finishes
            drop(oncomplete_closure);
            drop(onabort_closure);
        };

        wasm_bindgen_futures::spawn_local(fut);
        IdbTransactionImpl {
            transaction,
            tables,
            aborted: Arc::new(AtomicBool::new(false)),
            complete_rx,
        }
    }
}
