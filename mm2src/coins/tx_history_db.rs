use crate::TransactionDetails;
use async_trait::async_trait;
use common::mm_error::prelude::*;
use derive_more::Display;
use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
pub use native_db::TxHistoryDb;
#[cfg(target_arch = "wasm32")] pub use wasm_db::TxHistoryDb;

pub type TxHistoryResult<T> = Result<T, MmError<TxHistoryError>>;

#[derive(Debug, Display)]
pub enum TxHistoryError {
    ErrorSerializing(String),
    ErrorDeserializing(String),
    ErrorSaving(String),
    ErrorLoading(String),
    ErrorClearing(String),
    NotSupported(String),
    InternalError(String),
}

#[async_trait]
pub trait TxHistoryOps {
    async fn init_with_fs_path(db_dir: PathBuf) -> TxHistoryResult<TxHistoryDb>;

    async fn load_history(&mut self, ticker: &str, wallet_address: &str) -> TxHistoryResult<Vec<TransactionDetails>>;

    async fn save_history(
        &mut self,
        ticker: &str,
        wallet_address: &str,
        txs: Vec<TransactionDetails>,
    ) -> TxHistoryResult<()>;

    async fn clear(&mut self, ticker: &str, wallet_address: &str) -> TxHistoryResult<()>;
}

#[cfg(not(target_arch = "wasm32"))]
mod native_db {
    use super::*;
    use async_std::fs;
    use futures::AsyncWriteExt;
    use serde_json as json;
    use std::io;

    pub struct TxHistoryDb {
        tx_history_path: PathBuf,
    }

    #[async_trait]
    impl TxHistoryOps for TxHistoryDb {
        async fn init_with_fs_path(db_dir: PathBuf) -> TxHistoryResult<TxHistoryDb> {
            Ok(TxHistoryDb {
                tx_history_path: db_dir,
            })
        }

        async fn load_history(
            &mut self,
            ticker: &str,
            wallet_address: &str,
        ) -> TxHistoryResult<Vec<TransactionDetails>> {
            let path = self.ticker_history_path(ticker, wallet_address);
            let content = match fs::read(&path).await {
                Ok(content) => content,
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    return Ok(Vec::new());
                },
                Err(err) => {
                    let error = format!("Error '{}' reading from the history file {}", err, path.display());
                    return MmError::err(TxHistoryError::ErrorLoading(error));
                },
            };
            json::from_slice(&content).map_to_mm(|e| TxHistoryError::ErrorDeserializing(e.to_string()))
        }

        async fn save_history(
            &mut self,
            ticker: &str,
            wallet_address: &str,
            txs: Vec<TransactionDetails>,
        ) -> TxHistoryResult<()> {
            let content = json::to_vec(&txs).map_to_mm(|e| TxHistoryError::ErrorSerializing(e.to_string()))?;
            let path = self.ticker_history_path(ticker, wallet_address);

            let tmp_file = format!("{}.tmp", path.display());
            let fut = async {
                let mut file = fs::File::create(&tmp_file).await?;
                file.write_all(&content).await?;
                file.flush().await?;
                fs::rename(&tmp_file, &path).await?;
                Ok(())
            };
            let res: io::Result<_> = fut.await;
            if let Err(e) = res {
                let error = format!("Error '{}' creating/writing/renaming the tmp file {}", e, tmp_file);
                return MmError::err(TxHistoryError::ErrorSaving(error));
            }
            Ok(())
        }

        async fn clear(&mut self, ticker: &str, wallet_address: &str) -> TxHistoryResult<()> {
            fs::remove_file(&self.ticker_history_path(ticker, wallet_address))
                .await
                .map_to_mm(|e| TxHistoryError::ErrorClearing(e.to_string()))
        }
    }

    impl TxHistoryDb {
        fn ticker_history_path(&self, ticker: &str, wallet_address: &str) -> PathBuf {
            // BCH cash address format has colon after prefix, e.g. bitcoincash:
            // Colon can't be used in file names on Windows so it should be escaped
            let wallet_address = wallet_address.replace(":", "_");
            self.tx_history_path.join(format!("{}_{}.json", ticker, wallet_address))
        }
    }
}

/// Since `IndexedDb`, `DbTransaction`, `DbTable` are not `Send`,
/// so we have to spawn locally the database and communicate with it through the `mpsc` channel.
#[cfg(target_arch = "wasm32")]
mod wasm_db {
    use super::*;
    use common::executor::spawn_local;
    use common::panic_w;
    use common::wasm_indexed_db::{DbTransactionError, DbUpgrader, IndexedDb, IndexedDbBuilder, InitDbError,
                                  InitDbResult, OnUpgradeResult, TableSignature};
    use common::WasmUnwrapExt;
    use futures::channel::{mpsc, oneshot};
    use futures::StreamExt;
    use std::path::PathBuf;

    const DB_NAME: &str = "tx_history";
    const DB_VERSION: u32 = 1;

    impl From<InitDbError> for TxHistoryError {
        fn from(e: InitDbError) -> Self {
            match &e {
                InitDbError::NotSupported(_) => TxHistoryError::NotSupported(e.to_string()),
                InitDbError::EmptyTableList
                | InitDbError::DbIsOpenAlready { .. }
                | InitDbError::InvalidVersion(_)
                | InitDbError::OpeningError(_)
                | InitDbError::TypeMismatch { .. }
                | InitDbError::UnexpectedState(_)
                | InitDbError::UpgradingError { .. } => TxHistoryError::InternalError(e.to_string()),
            }
        }
    }

    impl From<DbTransactionError> for TxHistoryError {
        fn from(e: DbTransactionError) -> Self {
            match e {
                DbTransactionError::ErrorSerializingItem(_) => TxHistoryError::ErrorSerializing(e.to_string()),
                DbTransactionError::ErrorDeserializingItem(_) => TxHistoryError::ErrorDeserializing(e.to_string()),
                DbTransactionError::ErrorUploadingItem(_) => TxHistoryError::ErrorSaving(e.to_string()),
                DbTransactionError::ErrorGettingItems(_) => TxHistoryError::ErrorLoading(e.to_string()),
                DbTransactionError::ErrorDeletingItems(_) => TxHistoryError::ErrorClearing(e.to_string()),
                DbTransactionError::NoSuchTable { .. }
                | DbTransactionError::ErrorCreatingTransaction(_)
                | DbTransactionError::ErrorOpeningTable { .. }
                | DbTransactionError::UnexpectedState(_)
                | DbTransactionError::TransactionAborted
                | DbTransactionError::NoSuchIndex { .. }
                | DbTransactionError::InvalidIndex { .. } => TxHistoryError::InternalError(e.to_string()),
            }
        }
    }

    type LoadHistoryResult = TxHistoryResult<Vec<TransactionDetails>>;
    type SaveHistoryResult = TxHistoryResult<()>;
    type ClearHistoryResult = TxHistoryResult<()>;

    #[derive(Debug)]
    enum TxHistoryEvent {
        LoadHistory {
            history_id: HistoryId,
            result_tx: oneshot::Sender<LoadHistoryResult>,
        },
        SaveHistory {
            history_id: HistoryId,
            txs: Vec<TransactionDetails>,
            result_tx: oneshot::Sender<SaveHistoryResult>,
        },
        Clear {
            history_id: HistoryId,
            result_tx: oneshot::Sender<ClearHistoryResult>,
        },
    }

    pub struct TxHistoryDb {
        event_tx: mpsc::Sender<TxHistoryEvent>,
    }

    #[async_trait]
    impl TxHistoryOps for TxHistoryDb {
        async fn init_with_fs_path(_path: PathBuf) -> TxHistoryResult<TxHistoryDb> {
            let (init_tx, init_rx) = oneshot::channel();
            let (event_tx, event_rx) = mpsc::channel(1024);

            Self::init_and_spawn(init_tx, event_rx);
            init_rx.await.expect_w("The init channel must not be closed")?;
            Ok(TxHistoryDb { event_tx })
        }

        async fn load_history(&mut self, ticker: &str, wallet_address: &str) -> LoadHistoryResult {
            let (result_tx, result_rx) = oneshot::channel();
            let load_event = TxHistoryEvent::LoadHistory {
                history_id: HistoryId::new(ticker, wallet_address),
                result_tx,
            };
            if let Err(e) = self.event_tx.try_send(load_event) {
                let error = format!("Couldn't send the 'TxHistoryEvent::LoadHistory' event: {}", e);
                return MmError::err(TxHistoryError::InternalError(error));
            }
            result_rx.await.expect_w("The result channel must not be closed")
        }

        async fn save_history(
            &mut self,
            ticker: &str,
            wallet_address: &str,
            txs: Vec<TransactionDetails>,
        ) -> SaveHistoryResult {
            let (result_tx, result_rx) = oneshot::channel();
            let save_event = TxHistoryEvent::SaveHistory {
                history_id: HistoryId::new(ticker, wallet_address),
                txs,
                result_tx,
            };
            if let Err(e) = self.event_tx.try_send(save_event) {
                let error = format!("Couldn't send the 'TxHistoryEvent::SaveHistory' event: {}", e);
                return MmError::err(TxHistoryError::InternalError(error));
            }
            result_rx.await.expect_w("The result channel must not be closed")
        }

        async fn clear(&mut self, ticker: &str, wallet_address: &str) -> TxHistoryResult<()> {
            let (result_tx, result_rx) = oneshot::channel();
            let clear_event = TxHistoryEvent::Clear {
                history_id: HistoryId::new(ticker, wallet_address),
                result_tx,
            };
            if let Err(e) = self.event_tx.try_send(clear_event) {
                let error = format!("Couldn't send the 'TxHistoryEvent::Clear' event: {}", e);
                return MmError::err(TxHistoryError::InternalError(error));
            }
            result_rx.await.expect_w("The result channel must not be closed")
        }
    }

    impl TxHistoryDb {
        fn init_and_spawn(init_tx: oneshot::Sender<InitDbResult<()>>, event_rx: mpsc::Receiver<TxHistoryEvent>) {
            let fut = async move {
                let db = match IndexedDbBuilder::new(DB_NAME)
                    .with_version(DB_VERSION)
                    .with_table::<TxHistoryTable>()
                    .init()
                    .await
                {
                    Ok(db) => db,
                    Err(e) => {
                        // ignore if the receiver is closed
                        let _res = init_tx.send(Err(e));
                        return;
                    },
                };

                // ignore if the receiver is closed
                let _res = init_tx.send(Ok(()));
                // run the event loop
                Self::event_loop(event_rx, db).await;
            };
            spawn_local(fut);
        }

        async fn event_loop(mut rx: mpsc::Receiver<TxHistoryEvent>, db: IndexedDb) {
            while let Some(event) = rx.next().await {
                match event {
                    TxHistoryEvent::LoadHistory { history_id, result_tx } => {
                        let result = Self::load_history(&db, history_id).await;
                        // ignore if the receiver is closed
                        let _res = result_tx.send(result);
                    },
                    TxHistoryEvent::SaveHistory {
                        history_id,
                        txs,
                        result_tx,
                    } => {
                        let result = Self::save_history(&db, history_id, txs).await;
                        // ignore if the receiver is closed
                        let _res = result_tx.send(result);
                    },
                    TxHistoryEvent::Clear { history_id, result_tx } => {
                        let result = Self::clear_history(&db, history_id).await;
                        // ignore if the receiver is closed
                        let _res = result_tx.send(result);
                    },
                }
            }
        }

        async fn load_history(db: &IndexedDb, history_id: HistoryId) -> LoadHistoryResult {
            let transaction = db.transaction()?;
            let table = transaction.open_table::<TxHistoryTable>()?;
            let items = table.get_items("history_id", &history_id.0).await?;
            if items.len() > 1 {
                let error = format!(
                    "Expected only one item by the 'history_id' index, found {}",
                    items.len()
                );
                return MmError::err(TxHistoryError::InternalError(error));
            }

            let mut item_iter = items.into_iter();
            match item_iter.next() {
                Some((_item_id, TxHistoryTable { txs, .. })) => Ok(txs),
                None => Ok(Vec::new()),
            }
        }

        async fn save_history(
            db: &IndexedDb,
            history_id: HistoryId,
            txs: Vec<TransactionDetails>,
        ) -> SaveHistoryResult {
            let history_id_value = history_id.0.clone();
            let tx_history_item = TxHistoryTable { history_id, txs };

            let transaction = db.transaction()?;
            let table = transaction.open_table::<TxHistoryTable>()?;

            // First, check if the coin's tx history exists already.
            let ids = table.get_item_ids("history_id", &history_id_value).await?;
            match ids.len() {
                // The history doesn't exist, add the new `tx_history_item`.
                0 => {
                    table.add_item(&tx_history_item).await?;
                },
                // The history exists already, replace it with the actual `tx_history_item`.
                1 => {
                    let item_id = ids[0];
                    table.replace_item(item_id, tx_history_item).await?;
                },
                unexpected_len => {
                    let error = format!(
                        "Expected only one item by the 'history_id' index, found {}",
                        unexpected_len
                    );
                    return MmError::err(TxHistoryError::InternalError(error));
                },
            }

            transaction.wait_for_complete().await?;
            Ok(())
        }

        async fn clear_history(db: &IndexedDb, history_id: HistoryId) -> ClearHistoryResult {
            let transaction = db.transaction()?;
            let table = transaction.open_table::<TxHistoryTable>()?;

            // First, check if the coin's tx history exists.
            let ids = table.get_item_ids("history_id", &history_id.0).await?;
            match ids.len() {
                // The history doesn't exist, we don't need to do anything.
                0 => (),
                1 => {
                    let item_id = ids[0];
                    table.delete_item(item_id).await?;
                },
                unexpected_len => {
                    let error = format!(
                        "Expected only one item by the 'history_id' index, found {}",
                        unexpected_len
                    );
                    return MmError::err(TxHistoryError::InternalError(error));
                },
            }

            transaction.wait_for_complete().await?;
            Ok(())
        }
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct HistoryId(String);

    impl HistoryId {
        fn new(ticker: &str, wallet_address: &str) -> HistoryId { HistoryId(format!("{}_{}", ticker, wallet_address)) }
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct TxHistoryTable {
        history_id: HistoryId,
        txs: Vec<TransactionDetails>,
    }

    impl TableSignature for TxHistoryTable {
        fn table_name() -> &'static str { "tx_history" }

        fn on_upgrade_needed(upgrader: &DbUpgrader, old_version: u32, new_version: u32) -> OnUpgradeResult<()> {
            match (old_version, new_version) {
                (0, 1) => {
                    let table = upgrader.create_table(Self::table_name())?;
                    table.create_index("history_id", true)?;
                },
                (1, 1) => (),
                v => panic_w(&format!("Unexpected (old, new) versions: {:?}", v)),
            }
            Ok(())
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod tests {
    use super::wasm_db::*;
    use super::*;
    use common::WasmUnwrapExt;
    use serde_json as json;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn test_tx_history() {
        let mut db = TxHistoryDb::init_with_fs_path(PathBuf::default())
            .await
            .expect_w("!TxHistoryDb::init_with_fs_path");

        let history = db
            .load_history("RICK", "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD")
            .await
            .expect_w("!TxHistoryDb::load_history");
        assert!(history.is_empty());

        let history_str = r#"[{"tx_hex":"0400008085202f89018ec8f6f02e008ebd57bbf94c0d8297c1825f3af204490c43f5652b002a2c8b17010000006b483045022100e625a8b77beac5ec891e7d44b18fc4d780ef8456847eb2c6fa2f765e3379a9c102200f323612189fa44ee16f5deb39809b71c4811545b36ee4d6cc622d01aab10ef3012102043663e9c5af8275771809b3889d437f559e49e8df79b6ba19ade4cc5d8eb3e0ffffffff03809698000000000017a9142ffaf6694c6b441790546eefd277e430e08e47a6870000000000000000166a14094ab490fffa9939544545a656b345bf21920a90f6b35714000000001976a9145ed376ce9faa63cb2fef5862e1a5cc811c17316588acf9d29260000000000000000000000000000000","tx_hash":"f05d786bd4b647a5720094bf0a2c6f23b5e131c451d750a96102898f7b5458e8","from":["RHvavL8j683JwrN2ygk9Bg495DvPu5QVN3"],"to":["RHvavL8j683JwrN2ygk9Bg495DvPu5QVN3","bH6y6RtvbLToqSUNtLA5rQRjSwyNNzUSNc"],"total_amount":"3.51293022","spent_by_me":"3.51293022","received_by_me":"3.41292022","my_balance_change":"-0.10001","block_height":916940,"timestamp":1620235027,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"f05d786bd4b647a5720094bf0a2c6f23b5e131c451d750a96102898f7b5458e8"}]"#;
        let history: Vec<TransactionDetails> = json::from_str(history_str).unwrap_w();
        db.save_history("RICK", "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD", history)
            .await
            .expect_w("!TxHistoryDb::save_history");

        let updated_history_str = r#"[{"tx_hex":"0400008085202f89018ec8f6f02e008ebd57bbf94c0d8297c1825f3af204490c43f5652b002a2c8b17010000006b483045022100e625a8b77beac5ec891e7d44b18fc4d780ef8456847eb2c6fa2f765e3379a9c102200f323612189fa44ee16f5deb39809b71c4811545b36ee4d6cc622d01aab10ef3012102043663e9c5af8275771809b3889d437f559e49e8df79b6ba19ade4cc5d8eb3e0ffffffff03809698000000000017a9142ffaf6694c6b441790546eefd277e430e08e47a6870000000000000000166a14094ab490fffa9939544545a656b345bf21920a90f6b35714000000001976a9145ed376ce9faa63cb2fef5862e1a5cc811c17316588acf9d29260000000000000000000000000000000","tx_hash":"f05d786bd4b647a5720094bf0a2c6f23b5e131c451d750a96102898f7b5458e8","from":["RHvavL8j683JwrN2ygk9Bg495DvPu5QVN3"],"to":["RHvavL8j683JwrN2ygk9Bg495DvPu5QVN3","bH6y6RtvbLToqSUNtLA5rQRjSwyNNzUSNc"],"total_amount":"3.51293022","spent_by_me":"3.51293022","received_by_me":"3.41292022","my_balance_change":"-0.10001","block_height":916940,"timestamp":1620235027,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"f05d786bd4b647a5720094bf0a2c6f23b5e131c451d750a96102898f7b5458e8"},{"tx_hex":"0400008085202f8901a5620f30001e5e31bcbffacee7687fd84490fa1f8625ddd1e098f0bc530d673e020000006b483045022100a9864707855307681b81d94ae17328f6feccb2a9439d27378dfeae2df0220cc102207476f8304af14794a9cd2fe6287e07a1c802c05ec134b789ddb22ab51f7d2238012102043663e9c5af8275771809b3889d437f559e49e8df79b6ba19ade4cc5d8eb3e0ffffffff0246320000000000001976a914ca1e04745e8ca0c60d8c5881531d51bec470743f88ac5e4ef014000000001976a9145ed376ce9faa63cb2fef5862e1a5cc811c17316588acacd29260000000000000000000000000000000","tx_hash":"178b2c2a002b65f5430c4904f23a5f82c197820d4cf9bb57bd8e002ef0f6c88e","from":["RHvavL8j683JwrN2ygk9Bg495DvPu5QVN3"],"to":["RHvavL8j683JwrN2ygk9Bg495DvPu5QVN3","RThtXup6Zo7LZAi8kRWgjAyi1s4u6U9Cpf"],"total_amount":"3.51306892","spent_by_me":"3.51306892","received_by_me":"3.51293022","my_balance_change":"-0.0001387","block_height":916939,"timestamp":1620234997,"fee_details":{"type":"Utxo","amount":"0.00001"},"coin":"RICK","internal_id":"178b2c2a002b65f5430c4904f23a5f82c197820d4cf9bb57bd8e002ef0f6c88e"}]"#;
        let updated_history: Vec<TransactionDetails> = json::from_str(updated_history_str).unwrap_w();
        db.save_history("RICK", "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD", updated_history.clone())
            .await
            .expect_w("!TxHistoryDb::save_history");

        let actual_history = db
            .load_history("RICK", "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD")
            .await
            .expect_w("!TxHistoryDb::load_history");
        assert_eq!(actual_history, updated_history);

        db.clear("RICK", "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD")
            .await
            .expect_w("!TxHistoryDb::clear");

        let history = db
            .load_history("RICK", "RRnMcSeKiLrNdbp91qNVQwwXx5azD4S4CD")
            .await
            .expect_w("!TxHistoryDb::load_history");
        assert!(history.is_empty());
    }
}
