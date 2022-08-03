use super::{z_coin_errors::*, ZcoinConsensusParams};
use crate::utxo::rpc_clients;
use common::executor::Timer;
use common::log::{debug, error, info};
use common::{async_blocking, spawn_abortable, AbortOnDropHandle};
use db_common::sqlite::rusqlite::{params, Connection, Error as SqliteError, NO_PARAMS};
use db_common::sqlite::{query_single_row, run_optimization_pragmas};
use futures::channel::mpsc::{channel, Receiver as AsyncReceiver, Sender as AsyncSender};
use futures::channel::oneshot::{channel as oneshot_channel, Sender as OneshotSender};
use futures::lock::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};
use futures::StreamExt;
use http::Uri;
use mm2_err_handle::prelude::*;
use parking_lot::Mutex;
use prost::Message;
use protobuf::Message as ProtobufMessage;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::task::block_in_place;
use tonic::transport::{Channel, ClientTlsConfig};
use zcash_client_backend::data_api::chain::{scan_cached_blocks, validate_chain};
use zcash_client_backend::data_api::error::Error as ChainError;
use zcash_client_backend::data_api::{BlockSource, WalletRead, WalletWrite};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_sqlite::error::SqliteClientError as ZcashClientError;
use zcash_client_sqlite::wallet::init::{init_accounts_table, init_wallet_db};
use zcash_client_sqlite::WalletDb;
use zcash_primitives::consensus::BlockHeight;
use zcash_primitives::transaction::TxId;
use zcash_primitives::zip32::ExtendedFullViewingKey;

mod z_coin_grpc {
    tonic::include_proto!("cash.z.wallet.sdk.rpc");
}
use z_coin_grpc::compact_tx_streamer_client::CompactTxStreamerClient;
use z_coin_grpc::{BlockId, BlockRange, ChainSpec, TxFilter};

pub type WalletDbShared = Arc<Mutex<WalletDb<ZcoinConsensusParams>>>;

struct CompactBlockRow {
    height: BlockHeight,
    data: Vec<u8>,
}

/// A wrapper for the SQLite connection to the block cache database.
pub struct BlockDb(Connection);

impl BlockDb {
    /// Opens a connection to the wallet database stored at the specified path.
    pub fn for_path<P: AsRef<Path>>(path: P) -> Result<Self, db_common::sqlite::rusqlite::Error> {
        let conn = Connection::open(path)?;
        run_optimization_pragmas(&conn)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS compactblocks (
            height INTEGER PRIMARY KEY,
            data BLOB NOT NULL
        )",
            NO_PARAMS,
        )?;
        Ok(BlockDb(conn))
    }

    fn with_blocks<F>(
        &self,
        from_height: BlockHeight,
        limit: Option<u32>,
        mut with_row: F,
    ) -> Result<(), ZcashClientError>
    where
        F: FnMut(CompactBlock) -> Result<(), ZcashClientError>,
    {
        // Fetch the CompactBlocks we need to scan
        let mut stmt_blocks = self
            .0
            .prepare("SELECT height, data FROM compactblocks WHERE height > ? ORDER BY height ASC LIMIT ?")?;

        let rows = stmt_blocks.query_map(
            params![u32::from(from_height), limit.unwrap_or(u32::max_value()),],
            |row| {
                Ok(CompactBlockRow {
                    height: BlockHeight::from_u32(row.get(0)?),
                    data: row.get(1)?,
                })
            },
        )?;

        for row_result in rows {
            let cbr = row_result?;
            let block = CompactBlock::parse_from_bytes(&cbr.data)
                .map_err(zcash_client_backend::data_api::error::Error::from)?;

            if block.height() != cbr.height {
                return Err(ZcashClientError::CorruptedData(format!(
                    "Block height {} did not match row's height field value {}",
                    block.height(),
                    cbr.height
                )));
            }

            with_row(block)?;
        }

        Ok(())
    }

    fn get_latest_block(&self) -> Result<i64, MmError<SqliteError>> {
        Ok(query_single_row(
            &self.0,
            "SELECT height FROM compactblocks ORDER BY height DESC LIMIT 1",
            db_common::sqlite::rusqlite::NO_PARAMS,
            |row| row.get(0),
        )?
        .unwrap_or(0))
    }

    fn insert_block(
        &self,
        height: u32,
        cb_bytes: Vec<u8>,
    ) -> Result<usize, MmError<db_common::sqlite::rusqlite::Error>> {
        self.0
            .prepare("INSERT INTO compactblocks (height, data) VALUES (?, ?)")?
            .execute(params![height, cb_bytes])
            .map_err(MmError::new)
    }

    fn rewind_to_height(&self, height: u32) -> Result<usize, MmError<SqliteError>> {
        self.0
            .execute("DELETE from compactblocks WHERE height > ?1", [height])
            .map_err(MmError::new)
    }
}

impl BlockSource for BlockDb {
    type Error = ZcashClientError;

    fn with_blocks<F>(&self, from_height: BlockHeight, limit: Option<u32>, with_row: F) -> Result<(), Self::Error>
    where
        F: FnMut(CompactBlock) -> Result<(), Self::Error>,
    {
        self.with_blocks(from_height, limit, with_row)
    }
}

pub(super) async fn init_light_client(
    lightwalletd_url: Uri,
    cache_db_path: PathBuf,
    wallet_db_path: PathBuf,
    consensus_params: ZcoinConsensusParams,
    evk: ExtendedFullViewingKey,
) -> Result<(AsyncMutex<SaplingSyncConnector>, WalletDbShared), MmError<ZcoinLightClientInitError>> {
    let blocks_db =
        async_blocking(|| BlockDb::for_path(cache_db_path).map_to_mm(ZcoinLightClientInitError::BlocksDbInitFailure))
            .await?;

    let wallet_db = async_blocking({
        let consensus_params = consensus_params.clone();
        move || -> Result<_, MmError<ZcoinLightClientInitError>> {
            let db = WalletDb::for_path(wallet_db_path, consensus_params)
                .map_to_mm(ZcoinLightClientInitError::WalletDbInitFailure)?;
            run_optimization_pragmas(db.sql_conn()).map_to_mm(ZcoinLightClientInitError::WalletDbInitFailure)?;
            init_wallet_db(&db).map_to_mm(ZcoinLightClientInitError::WalletDbInitFailure)?;
            if db.get_extended_full_viewing_keys()?.is_empty() {
                init_accounts_table(&db, &[evk])?;
            }
            Ok(db)
        }
    })
    .await?;

    let tonic_channel = Channel::builder(lightwalletd_url)
        .tls_config(ClientTlsConfig::new())
        .map_to_mm(ZcoinLightClientInitError::TlsConfigFailure)?
        .connect()
        .await
        .map_to_mm(ZcoinLightClientInitError::ConnectionFailure)?;
    let grpc_client = CompactTxStreamerClient::new(tonic_channel);

    let (sync_status_notifier, sync_watcher) = channel(1);
    let (on_tx_gen_notifier, on_tx_gen_watcher) = channel(1);

    let wallet_db = Arc::new(Mutex::new(wallet_db));
    let sync_handle = SaplingSyncLoopHandle {
        current_block: BlockHeight::from_u32(0),
        grpc_client,
        blocks_db,
        wallet_db: wallet_db.clone(),
        consensus_params,
        sync_status_notifier,
        on_tx_gen_watcher,
        watch_for_tx: None,
    };
    let abort_handle = spawn_abortable(light_wallet_db_sync_loop(sync_handle));

    Ok((
        SaplingSyncConnector::new_mutex_wrapped(sync_watcher, on_tx_gen_notifier, abort_handle),
        wallet_db,
    ))
}

fn is_tx_imported(conn: &Connection, tx_id: TxId) -> bool {
    const QUERY: &str = "SELECT id_tx FROM transactions WHERE txid = ?1;";
    match query_single_row(conn, QUERY, [tx_id.0.to_vec()], |row| row.get::<_, i64>(0)) {
        Ok(Some(_)) => true,
        Ok(None) | Err(_) => false,
    }
}

pub struct SaplingSyncRespawnGuard {
    pub(super) sync_handle: Option<SaplingSyncLoopHandle>,
    pub(super) abort_handle: Arc<Mutex<AbortOnDropHandle>>,
}

impl Drop for SaplingSyncRespawnGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.sync_handle.take() {
            *self.abort_handle.lock() = spawn_abortable(light_wallet_db_sync_loop(handle));
        }
    }
}

impl SaplingSyncRespawnGuard {
    pub(super) fn watch_for_tx(&mut self, tx_id: TxId) {
        if let Some(ref mut handle) = self.sync_handle {
            handle.watch_for_tx = Some(tx_id);
        }
    }

    #[inline]
    pub(super) fn current_block(&self) -> BlockHeight { self.sync_handle.as_ref().expect("always Some").current_block }
}

pub enum SyncStatus {
    UpdatingBlocksCache {
        current_scanned_block: u64,
        latest_block: u64,
    },
    BuildingWalletDb,
    Finished {
        block_number: u64,
    },
}

pub struct SaplingSyncLoopHandle {
    current_block: BlockHeight,
    grpc_client: CompactTxStreamerClient<Channel>,
    blocks_db: BlockDb,
    wallet_db: WalletDbShared,
    consensus_params: ZcoinConsensusParams,
    /// Notifies about sync status without stopping the loop, e.g. on coin activation
    sync_status_notifier: AsyncSender<SyncStatus>,
    /// If new tx is required to be generated, we stop the sync and respawn it after tx is sent
    /// This watcher waits for such notification
    on_tx_gen_watcher: AsyncReceiver<OneshotSender<Self>>,
    watch_for_tx: Option<TxId>,
}

impl SaplingSyncLoopHandle {
    fn notify_blocks_cache_status(&mut self, current_scanned_block: u64, latest_block: u64) {
        if self
            .sync_status_notifier
            .try_send(SyncStatus::UpdatingBlocksCache {
                current_scanned_block,
                latest_block,
            })
            .is_err()
        {
            debug!("No one seems interested in SyncStatus");
        }
    }

    fn notify_building_wallet_db(&mut self) {
        if self
            .sync_status_notifier
            .try_send(SyncStatus::BuildingWalletDb)
            .is_err()
        {
            debug!("No one seems interested in SyncStatus");
        }
    }

    fn notify_sync_finished(&mut self) {
        if self
            .sync_status_notifier
            .try_send(SyncStatus::Finished {
                block_number: self.current_block.into(),
            })
            .is_err()
        {
            debug!("No one seems interested in SyncStatus");
        }
    }

    async fn update_blocks_cache(&mut self) -> Result<(), MmError<UpdateBlocksCacheErr>> {
        let request = tonic::Request::new(ChainSpec {});
        let current_blockchain_block = self.grpc_client.get_latest_block(request).await?;
        let current_block_in_db = block_in_place(|| self.blocks_db.get_latest_block())?;

        let from_block = current_block_in_db as u64 + 1;
        let current_block: u64 = current_blockchain_block.into_inner().height;

        if current_block >= from_block {
            let request = tonic::Request::new(BlockRange {
                start: Some(BlockId {
                    height: from_block,
                    hash: Vec::new(),
                }),
                end: Some(BlockId {
                    height: current_block,
                    hash: Vec::new(),
                }),
            });

            let mut response = self.grpc_client.get_block_range(request).await?;

            while let Some(block) = response.get_mut().message().await? {
                debug!("Got block {:?}", block);
                block_in_place(|| self.blocks_db.insert_block(block.height as u32, block.encode_to_vec()))?;
                self.notify_blocks_cache_status(block.height, current_block);
            }
        }

        self.current_block = BlockHeight::from_u32(current_block as u32);
        Ok(())
    }

    /// Scans cached blocks, validates the chain and updates WalletDb.
    /// For more notes on the process, check https://github.com/zcash/librustzcash/blob/master/zcash_client_backend/src/data_api/chain.rs#L2
    fn scan_blocks(&mut self) -> Result<(), MmError<ZcashClientError>> {
        let wallet_guard = self.wallet_db.lock();
        let mut wallet_ops = wallet_guard.get_update_ops().expect("get_update_ops always returns Ok");

        if let Err(e) = validate_chain(
            &self.consensus_params,
            &self.blocks_db,
            wallet_ops.get_max_height_hash()?,
        ) {
            match e {
                ZcashClientError::BackendError(ChainError::InvalidChain(lower_bound, _)) => {
                    let rewind_height = if lower_bound > BlockHeight::from_u32(10) {
                        lower_bound - 10
                    } else {
                        BlockHeight::from_u32(0)
                    };
                    wallet_ops.rewind_to_height(rewind_height)?;
                    self.blocks_db.rewind_to_height(rewind_height.into())?;
                },
                e => return MmError::err(e),
            }
        }

        scan_cached_blocks(&self.consensus_params, &self.blocks_db, &mut wallet_ops, None)?;
        Ok(())
    }

    async fn check_watch_for_tx_existence(&mut self) {
        if let Some(tx_id) = self.watch_for_tx {
            let mut attempts = 0;
            loop {
                let filter = TxFilter {
                    block: None,
                    index: 0,
                    hash: tx_id.0.into(),
                };
                let request = tonic::Request::new(filter);
                match self.grpc_client.get_transaction(request).await {
                    Ok(_) => break,
                    Err(e) => {
                        error!("Error on getting tx {}", tx_id);
                        if e.message().contains(rpc_clients::NO_TX_ERROR_CODE) {
                            if attempts >= 3 {
                                self.watch_for_tx = None;
                                return;
                            }
                            attempts += 1;
                        }
                        Timer::sleep(30.).await;
                    },
                }
            }
        }
    }
}

/// For more info on shielded light client protocol, please check the https://zips.z.cash/zip-0307
///
/// It's important to note that unlike standard UTXOs, shielded outputs are not spendable until the transaction is confirmed.
///
/// For AtomicDEX, we have additional requirements for the sync process:
/// 1. Coin should not be usable until initial sync is finished.
/// 2. During concurrent transaction generation (several simultaneous swaps using the same coin), we should prevent the same input usage.
/// 3. Once the transaction is sent, we have to wait until it's confirmed for the change to become spendable.
///
/// So the following was implemented:
/// 1. On the coin initialization, `init_light_client` creates `SaplingSyncLoopHandle`, spawns sync loop
///     and returns mutex-wrapped `SaplingSyncConnector` to interact with it.
/// 2. During sync process, the `SaplingSyncLoopHandle` notifies external code about status using `sync_status_notifier`.
/// 3. Once the sync completes, the coin becomes usable.
/// 4. When transaction is about to be generated, the external code locks the `SaplingSyncConnector` mutex,
///     and calls `SaplingSyncConnector::wait_for_gen_tx_blockchain_sync`.
///     This actually stops the loop and returns `SaplingSyncGuard`, which contains MutexGuard<SaplingSyncConnector> and `SaplingSyncRespawnGuard`.
/// 5. `SaplingSyncRespawnGuard` in its turn contains `SaplingSyncLoopHandle` that is used to respawn the sync when the guard is dropped.
/// 6. Once the transaction is generated and sent, `SaplingSyncRespawnGuard::watch_for_tx` is called to update `SaplingSyncLoopHandle` state.
/// 7. Once the loop is respawned, it will check that broadcast tx is imported (or not available anymore) before stopping in favor of
///     next wait_for_gen_tx_blockchain_sync call.
async fn light_wallet_db_sync_loop(mut sync_handle: SaplingSyncLoopHandle) {
    // this loop is spawned as standalone task so it's safe to use block_in_place here
    loop {
        if let Err(e) = sync_handle.update_blocks_cache().await {
            error!("Error {} on blocks cache update", e);
            Timer::sleep(10.).await;
            continue;
        }

        sync_handle.notify_building_wallet_db();

        if let Err(e) = block_in_place(|| sync_handle.scan_blocks()) {
            error!("Error {} on scan_blocks", e);
            Timer::sleep(10.).await;
            continue;
        }

        sync_handle.notify_sync_finished();

        sync_handle.check_watch_for_tx_existence().await;

        if let Some(tx_id) = sync_handle.watch_for_tx {
            if !block_in_place(|| is_tx_imported(sync_handle.wallet_db.lock().sql_conn(), tx_id)) {
                info!("Tx {} is not imported yet", tx_id);
                Timer::sleep(10.).await;
                continue;
            }
            sync_handle.watch_for_tx = None;
        }

        if let Ok(Some(sender)) = sync_handle.on_tx_gen_watcher.try_next() {
            match sender.send(sync_handle) {
                Ok(_) => break,
                Err(handle_from_channel) => {
                    sync_handle = handle_from_channel;
                },
            }
        }

        Timer::sleep(10.).await;
    }
}

type SyncWatcher = AsyncReceiver<SyncStatus>;
type NewTxNotifier = AsyncSender<OneshotSender<SaplingSyncLoopHandle>>;

pub(super) struct SaplingSyncConnector {
    sync_watcher: SyncWatcher,
    on_tx_gen_notifier: NewTxNotifier,
    abort_handle: Arc<Mutex<AbortOnDropHandle>>,
}

impl SaplingSyncConnector {
    #[inline]
    pub(super) fn new_mutex_wrapped(
        simple_sync_watcher: SyncWatcher,
        on_tx_gen_notifier: NewTxNotifier,
        abort_handle: AbortOnDropHandle,
    ) -> AsyncMutex<Self> {
        AsyncMutex::new(SaplingSyncConnector {
            sync_watcher: simple_sync_watcher,
            on_tx_gen_notifier,
            abort_handle: Arc::new(Mutex::new(abort_handle)),
        })
    }

    #[inline]
    pub(super) async fn current_sync_status(&mut self) -> Result<SyncStatus, MmError<BlockchainScanStopped>> {
        self.sync_watcher.next().await.or_mm_err(|| BlockchainScanStopped {})
    }

    pub(super) async fn wait_for_gen_tx_blockchain_sync(
        &mut self,
    ) -> Result<SaplingSyncRespawnGuard, MmError<BlockchainScanStopped>> {
        let (sender, receiver) = oneshot_channel();
        self.on_tx_gen_notifier
            .try_send(sender)
            .map_to_mm(|_| BlockchainScanStopped {})?;
        receiver
            .await
            .map(|handle| SaplingSyncRespawnGuard {
                sync_handle: Some(handle),
                abort_handle: self.abort_handle.clone(),
            })
            .map_to_mm(|_| BlockchainScanStopped {})
    }
}

pub(super) struct SaplingSyncGuard<'a> {
    pub(super) _connector_guard: AsyncMutexGuard<'a, SaplingSyncConnector>,
    pub(super) respawn_guard: SaplingSyncRespawnGuard,
}
