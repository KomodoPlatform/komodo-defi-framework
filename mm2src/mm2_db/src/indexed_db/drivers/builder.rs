use super::{construct_event_closure, DbUpgrader, IdbDatabaseImpl, OnUpgradeError, OnUpgradeNeededCb, OPEN_DATABASES};
use common::{log::info, stringify_js_error};
use derive_more::Display;
use futures::channel::mpsc;
use futures::StreamExt;
use mm2_err_handle::prelude::*;
use std::collections::{HashMap, HashSet};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{IdbDatabase, IdbFactory, IdbOpenDbRequest, IdbTransaction, IdbVersionChangeEvent, Window,
              WorkerGlobalScope};

pub type InitDbResult<T> = Result<T, MmError<InitDbError>>;

#[derive(Debug, Display, PartialEq)]
pub enum InitDbError {
    #[display(fmt = "Cannot initialize a Database without tables")]
    EmptyTableList,
    #[display(fmt = "Database '{}' is open already", db_name)]
    DbIsOpenAlready { db_name: String },
    #[display(fmt = "It seems this browser doesn't support 'IndexedDb': {}", _0)]
    NotSupported(String),
    #[display(fmt = "Invalid Database version: {}", _0)]
    InvalidVersion(String),
    #[display(fmt = "Couldn't open Database: {}", _0)]
    OpeningError(String),
    #[display(fmt = "Type mismatch: expected '{}', found '{}'", expected, found)]
    TypeMismatch { expected: String, found: String },
    #[display(fmt = "Error occurred due to an unexpected state: {:?}", _0)]
    UnexpectedState(String),
    #[display(
        fmt = "Error occurred due to the Database upgrading from {} to {} version: {}",
        old_version,
        new_version,
        error
    )]
    UpgradingError {
        old_version: u32,
        new_version: u32,
        error: OnUpgradeError,
    },
}

pub struct IdbDatabaseBuilder {
    db_name: String,
    db_version: u32,
    tables: HashMap<String, OnUpgradeNeededCb>,
}

impl IdbDatabaseBuilder {
    pub fn new(db_name: &str) -> IdbDatabaseBuilder {
        IdbDatabaseBuilder {
            db_name: db_name.to_owned(),
            db_version: 1,
            tables: HashMap::new(),
        }
    }

    pub fn with_version(mut self, db_version: u32) -> IdbDatabaseBuilder {
        self.db_version = db_version;
        self
    }

    pub fn with_tables<Tables>(mut self, tables: Tables) -> IdbDatabaseBuilder
    where
        Tables: IntoIterator<Item = (String, OnUpgradeNeededCb)>,
    {
        self.tables.extend(tables);
        self
    }

    pub async fn build(self) -> InitDbResult<IdbDatabaseImpl> {
        Self::check_if_db_is_not_open(&self.db_name)?;
        let (table_names, on_upgrade_needed_handlers) = Self::tables_into_parts(self.tables)?;
        info!("Open '{}' database with tables: {:?}", self.db_name, table_names);

        // get global context
        let global = js_sys::global();
        //
        // // check if dyn casting to window type was successful
        // let indexed_db_factory: Option<IdbFactory> = if global.dyn_ref::<Window>().is_some() {
        //     global.dyn_ref::<Window>().unwrap().indexed_db().unwrap_or(None)
        // // check if worker context
        // } else if global.dyn_ref::<WorkerGlobalScope>().is_some() {
        //     global.dyn_ref::<WorkerGlobalScope>().unwrap().indexed_db().unwrap_or(None)
        // } else {
        //     None
        // };
        // // bind value based on pattern matching against idbfactory
        // let indexed_db = match indexed_db_factory {
        //     Ok(Some(db)) => db,
        // // err handling
        //     Ok(None) => {
        //         if global.dyn_ref::<Window>().is_some() {
        //             return MmError::err(InitDbError::NotSupported("IndexedDB not supported in window context".to_owned()));
        //         } else if global.dyn_ref::<WorkerGlobalScope>().is_some() {
        //             return MmError::err(InitDbError::NotSupported("IndexedDB not supported in worker context".to_owned()));
        //         } else {
        //             return MmError::err(InitDbError::UnexpectedState("rekt".to_owned()));
        //         }
        //     },
        //     Err(e) => return MmError::err(InitDbError::NotSupported(stringify_js_error(&e)))
        // };

        // Using Result here (L111 - "cast check") for more precise error handling and messaging.
        // However, for performance and memory considerations, it might be worth evaluating the use of Option (as seen in commented-out code above).
        // Option<T> might have a slight edge in terms of memory consumption, especially if the error type E in Result<T, E> is large and/or complex.
        // Moreover, in micro-benchmarks, Option<T> seems to demonstrate marginal performance advantages.
        // But these trade-offs were considered acceptable for the benefit of clearer error states in this context.
        // Ref: https://users.rust-lang.org/t/performance-impact-of-result-vs-option/17783, etc.

        let indexed_db_result: Result<IdbFactory, InitDbError> = match global.dyn_ref::<Window>() {
            Some(window) => match window.indexed_db() {
                Ok(Some(db)) => Ok(db),
                Ok(None) => Err(InitDbError::NotSupported(
                    "IndexedDB not supported in window context".to_owned(),
                )),
                Err(e) => Err(InitDbError::NotSupported(stringify_js_error(&e))),
            },
            None => match global.dyn_ref::<WorkerGlobalScope>() {
                Some(worker) => match worker.indexed_db() {
                    Ok(Some(db)) => Ok(db),
                    Ok(None) => Err(InitDbError::NotSupported(
                        "IndexedDB not supported in worker context".to_owned(),
                    )),
                    Err(e) => Err(InitDbError::NotSupported(stringify_js_error(&e))),
                },
                None => Err(InitDbError::UnexpectedState("Unknown context".to_owned())),
            },
        };

        // imo more clear / readable "if let" pattern version - as opposed to the "idiomatic" Rust version above
        // (idiomatic Rust because favor extensive pattern matching () over if / else chains)
        // following commented-out code serves educational purposes - ca333

        // let indexed_db_result: Result<IdbFactory, InitDbError> = if let Some(window) = global.dyn_ref::<Window>() {
        //     match window.indexed_db() {
        //         Ok(Some(db)) => Ok(db),
        //         Ok(None) => Err(InitDbError::NotSupported(
        //             "IndexedDB not supported in window context".to_owned(),
        //         )),
        //         Err(e) => Err(InitDbError::NotSupported(stringify_js_error(&e))),
        //     }
        // } else if let Some(worker) = global.dyn_ref::<WorkerGlobalScope>() {
        //     match worker.indexed_db() {
        //         Ok(Some(db)) => Ok(db),
        //         Ok(None) => Err(InitDbError::NotSupported(
        //             "IndexedDB not supported in worker context".to_owned(),
        //         )),
        //         Err(e) => Err(InitDbError::NotSupported(stringify_js_error(&e))),
        //     }
        // } else {
        //     Err(InitDbError::UnexpectedState("Unknown context".to_owned()))
        // };

        let indexed_db = match indexed_db_result {
            Ok(db) => db,
            Err(e) => return MmError::err(e),
        };

        let db_request = match indexed_db.open_with_u32(&self.db_name, self.db_version) {
            Ok(r) => r,
            Err(e) => return MmError::err(InitDbError::InvalidVersion(stringify_js_error(&e))),
        };
        let (tx, mut rx) = mpsc::channel(1);

        let onerror_closure = construct_event_closure(DbOpenEvent::Failed, tx.clone());
        let onsuccess_closure = construct_event_closure(DbOpenEvent::Success, tx.clone());
        let onupgradeneeded_closure = construct_event_closure(DbOpenEvent::UpgradeNeeded, tx.clone());

        db_request.set_onerror(Some(onerror_closure.as_ref().unchecked_ref()));
        db_request.set_onsuccess(Some(onsuccess_closure.as_ref().unchecked_ref()));
        db_request.set_onupgradeneeded(Some(onupgradeneeded_closure.as_ref().unchecked_ref()));

        let mut on_upgrade_needed_handlers = Some(on_upgrade_needed_handlers);
        while let Some(event) = rx.next().await {
            match event {
                DbOpenEvent::Failed(e) => return MmError::err(InitDbError::OpeningError(stringify_js_error(&e))),
                DbOpenEvent::UpgradeNeeded(event) => {
                    Self::on_upgrade_needed(event, &db_request, &mut on_upgrade_needed_handlers)?
                },
                DbOpenEvent::Success(_) => {
                    let db = Self::get_db_from_request(&db_request)?;
                    Self::cache_open_db(self.db_name.clone());

                    return Ok(IdbDatabaseImpl {
                        db,
                        db_name: self.db_name,
                        tables: table_names,
                    });
                },
            }
        }
        unreachable!("The event channel must not be closed before either 'DbOpenEvent::Success' or 'DbOpenEvent::Failed' is received");
    }

    fn on_upgrade_needed(
        event: JsValue,
        db_request: &IdbOpenDbRequest,
        handlers: &mut Option<Vec<OnUpgradeNeededCb>>,
    ) -> InitDbResult<()> {
        let handlers = match handlers.take() {
            Some(handlers) => handlers,
            None => {
                return MmError::err(InitDbError::UnexpectedState(
                    "'IndexedDbBuilder::on_upgraded_needed' was called twice".to_owned(),
                ))
            },
        };

        let db = Self::get_db_from_request(db_request)?;
        let transaction = Self::get_transaction_from_request(db_request)?;

        let version_event = match event.dyn_into::<IdbVersionChangeEvent>() {
            Ok(version) => version,
            Err(e) => {
                return MmError::err(InitDbError::TypeMismatch {
                    expected: "IdbVersionChangeEvent".to_owned(),
                    found: format!("{:?}", e),
                })
            },
        };
        let old_version = version_event.old_version() as u32;
        let new_version = version_event
            .new_version()
            .ok_or_else(|| MmError::new(InitDbError::InvalidVersion("Expected a new_version".to_owned())))?
            as u32;

        let upgrader = DbUpgrader::new(db, transaction);
        for on_upgrade_needed_cb in handlers {
            on_upgrade_needed_cb(&upgrader, old_version, new_version).mm_err(|error| InitDbError::UpgradingError {
                old_version,
                new_version,
                error,
            })?;
        }
        Ok(())
    }

    fn cache_open_db(db_name: String) {
        let mut open_databases = OPEN_DATABASES.lock().expect("!OPEN_DATABASES.lock()");
        open_databases.insert(db_name);
    }

    fn check_if_db_is_not_open(db_name: &str) -> InitDbResult<()> {
        let open_databases = OPEN_DATABASES.lock().expect("!OPEN_DATABASES.lock()");
        if open_databases.contains(db_name) {
            MmError::err(InitDbError::DbIsOpenAlready {
                db_name: db_name.to_owned(),
            })
        } else {
            Ok(())
        }
    }

    fn get_db_from_request(db_request: &IdbOpenDbRequest) -> InitDbResult<IdbDatabase> {
        let db_result = match db_request.result() {
            Ok(res) => res,
            Err(e) => return MmError::err(InitDbError::UnexpectedState(stringify_js_error(&e))),
        };
        db_result.dyn_into::<IdbDatabase>().map_err(|db_result| {
            MmError::new(InitDbError::TypeMismatch {
                expected: "IdbDatabase".to_owned(),
                found: format!("{:?}", db_result),
            })
        })
    }

    fn get_transaction_from_request(db_request: &IdbOpenDbRequest) -> InitDbResult<IdbTransaction> {
        let transaction = match db_request.transaction() {
            Some(res) => res,
            None => {
                return MmError::err(InitDbError::UnexpectedState(
                    "Expected 'IdbOpenDbRequest::transaction'".to_owned(),
                ))
            },
        };
        transaction.dyn_into::<IdbTransaction>().map_err(|transaction| {
            MmError::new(InitDbError::TypeMismatch {
                expected: "IdbTransaction".to_owned(),
                found: format!("{:?}", transaction),
            })
        })
    }

    fn tables_into_parts(
        tables: HashMap<String, OnUpgradeNeededCb>,
    ) -> InitDbResult<(HashSet<String>, Vec<OnUpgradeNeededCb>)> {
        if tables.is_empty() {
            return MmError::err(InitDbError::EmptyTableList);
        }

        let mut table_names = HashSet::with_capacity(tables.len());
        let mut on_upgrade_needed_handlers = Vec::with_capacity(tables.len());
        for (table_name, handler) in tables {
            table_names.insert(table_name);
            on_upgrade_needed_handlers.push(handler);
        }
        Ok((table_names, on_upgrade_needed_handlers))
    }
}

#[derive(Debug)]
enum DbOpenEvent {
    Failed(JsValue),
    UpgradeNeeded(JsValue),
    Success(JsValue),
}
