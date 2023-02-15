use super::{z_coin_errors::*, CheckPointBlockInfo, ZcoinConsensusParams};
use crate::utxo::rpc_clients::{NativeClient, UtxoRpcClientOps, NO_TX_ERROR_CODE};
use async_trait::async_trait;
use common::executor::{spawn_abortable, AbortOnDropHandle, Timer};
use common::log::{debug, error, info, LogOnError};
use common::{async_blocking, Future01CompatExt};
use db_common::sqlite::rusqlite::{params, Connection, Error as SqliteError, NO_PARAMS};
use db_common::sqlite::{query_single_row, run_optimization_pragmas};
use futures::channel::mpsc::{channel, Receiver as AsyncReceiver, Sender as AsyncSender};
use futures::channel::oneshot::{channel as oneshot_channel, Sender as OneshotSender};
use futures::lock::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};
use futures::StreamExt;
use group::GroupEncoding;
use http::Uri;
use mm2_err_handle::prelude::*;
use parking_lot::Mutex;
use prost::Message;
use protobuf::Message as ProtobufMessage;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::block_in_place;
use tonic::transport::{Channel, ClientTlsConfig};
use zcash_client_backend::data_api::chain::{scan_cached_blocks, validate_chain};
use zcash_client_backend::data_api::error::Error as ChainError;
use zcash_client_backend::data_api::{BlockSource, WalletRead, WalletWrite};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_sqlite::error::SqliteClientError as ZcashClientError;
use zcash_client_sqlite::wallet::init::{init_accounts_table, init_blocks_table, init_wallet_db};
use zcash_client_sqlite::WalletDb;
use zcash_primitives::block::BlockHash;
use zcash_primitives::consensus::BlockHeight;
use zcash_primitives::transaction::TxId;
use zcash_primitives::zip32::ExtendedFullViewingKey;

mod z_coin_grpc {
    tonic::include_proto!("cash.z.wallet.sdk.rpc");
}
use crate::{RpcCommonOps, ZTransaction};
use rpc::v1::types::H256 as H256Json;
use z_coin_grpc::compact_tx_streamer_client::CompactTxStreamerClient;
use z_coin_grpc::{BlockId, BlockRange, ChainSpec, CompactBlock as TonicCompactBlock,
                  CompactOutput as TonicCompactOutput, CompactSpend as TonicCompactSpend, CompactTx as TonicCompactTx,
                  TxFilter};

pub type WalletDbShared = Arc<Mutex<WalletDb<ZcoinConsensusParams>>>;

struct CompactBlockRow {
    height: BlockHeight,
    data: Vec<u8>,
}

pub type OnCompactBlockFn<'a> = dyn FnMut(TonicCompactBlock) -> Result<(), MmError<UpdateBlocksCacheErr>> + Send + 'a;

#[async_trait]
pub trait ZRpcOps {
    async fn get_block_height(&mut self) -> Result<u64, MmError<UpdateBlocksCacheErr>>;

    async fn scan_blocks(
        &mut self,
        start_block: u64,
        last_block: u64,
        on_block: &mut OnCompactBlockFn,
    ) -> Result<(), MmError<UpdateBlocksCacheErr>>;

    async fn check_tx_existence(&mut self, tx_id: TxId) -> bool;
}

struct LightRpcClient {
    rpc_clients: AsyncMutex<Vec<CompactTxStreamerClient<Channel>>>,
}

#[async_trait]
impl RpcCommonOps for LightRpcClient {
    type RpcClient = CompactTxStreamerClient<Channel>;
    type Error = MmError<UpdateBlocksCacheErr>;

    async fn get_live_client(&self) -> Result<Self::RpcClient, Self::Error> {
        let mut clients = self.rpc_clients.lock().await;
        for (i, mut client) in clients.clone().into_iter().enumerate() {
            let request = tonic::Request::new(ChainSpec {});
            // use get_latest_block method as a health check
            if client.get_latest_block(request).await.is_ok() {
                clients.rotate_left(i);
                return Ok(client);
            }
        }
        return Err(MmError::new(UpdateBlocksCacheErr::GetLiveLightClientError(
            "All the current light clients are unavailable.".to_string(),
        )));
    }
}

#[async_trait]
impl ZRpcOps for LightRpcClient {
    async fn get_block_height(&mut self) -> Result<u64, MmError<UpdateBlocksCacheErr>> {
        let request = tonic::Request::new(ChainSpec {});
        let block = self
            .get_live_client()
            .await?
            .get_latest_block(request)
            .await
            .map_to_mm(UpdateBlocksCacheErr::GrpcError)?
            // return the message
            .into_inner();
        Ok(block.height)
    }

    async fn scan_blocks(
        &mut self,
        start_block: u64,
        last_block: u64,
        on_block: &mut OnCompactBlockFn,
    ) -> Result<(), MmError<UpdateBlocksCacheErr>> {
        let request = tonic::Request::new(BlockRange {
            start: Some(BlockId {
                height: start_block,
                hash: Vec::new(),
            }),
            end: Some(BlockId {
                height: last_block,
                hash: Vec::new(),
            }),
        });
        let mut response = self
            .get_live_client()
            .await?
            .get_block_range(request)
            .await
            .map_to_mm(UpdateBlocksCacheErr::GrpcError)?
            .into_inner();
        // without Pin method get_mut is not found in current scope
        while let Some(block) = Pin::new(&mut response).get_mut().message().await? {
            debug!("Got block {:?}", block);
            on_block(block)?;
        }
        Ok(())
    }

    async fn check_tx_existence(&mut self, tx_id: TxId) -> bool {
        let mut attempts = 0;
        loop {
            if let Ok(mut client) = self.get_live_client().await {
                let request = tonic::Request::new(TxFilter {
                    block: None,
                    index: 0,
                    hash: tx_id.0.into(),
                });
                match client.get_transaction(request).await {
                    Ok(_) => break,
                    Err(e) => {
                        error!("Error on getting tx {}", tx_id);
                        if e.message().contains(NO_TX_ERROR_CODE) {
                            if attempts >= 3 {
                                return false;
                            }
                            attempts += 1;
                        }
                        Timer::sleep(30.).await;
                    },
                }
            }
        }
        true
    }
}

#[async_trait]
impl ZRpcOps for NativeClient {
    async fn get_block_height(&mut self) -> Result<u64, MmError<UpdateBlocksCacheErr>> {
        Ok(self.get_block_count().compat().await?)
    }

    async fn scan_blocks(
        &mut self,
        start_block: u64,
        last_block: u64,
        on_block: &mut OnCompactBlockFn,
    ) -> Result<(), MmError<UpdateBlocksCacheErr>> {
        for height in start_block..=last_block {
            let block = self.get_block_by_height(height).await?;
            debug!("Got block {:?}", block);
            let mut compact_txs = Vec::new();
            // By default, CompactBlocks only contain CompactTxs for transactions that contain Sapling spends or outputs.
            // Create and push compact_tx during iteration.
            for (tx_id, hash_tx) in block.tx.iter().enumerate() {
                let tx_bytes = self.get_transaction_bytes(hash_tx).compat().await?;
                let tx = ZTransaction::read(tx_bytes.as_slice()).unwrap();
                let mut spends = Vec::new();
                let mut outputs = Vec::new();
                if !tx.shielded_spends.is_empty() || !tx.shielded_outputs.is_empty() {
                    // Create and push spends with outs for compact_tx during iterations.
                    for spend in &tx.shielded_spends {
                        let compact_spend = TonicCompactSpend {
                            nf: spend.nullifier.to_vec(),
                        };
                        spends.push(compact_spend);
                    }
                    for out in &tx.shielded_outputs {
                        let compact_out = TonicCompactOutput {
                            cmu: out.cmu.to_bytes().to_vec(),
                            epk: out.ephemeral_key.to_bytes().to_vec(),
                            // https://zips.z.cash/zip-0307#output-compression
                            // The first 52 bytes of the ciphertext contain the contents and opening of the note commitment,
                            // which is all of the data needed to spend the note and to verify that the note is spendable.
                            ciphertext: out.enc_ciphertext[0..52].to_vec(),
                        };
                        outputs.push(compact_out);
                    }
                    // Shadowing mut variables as immutable. No longer need to update them.
                    drop_mutability!(spends);
                    drop_mutability!(outputs);
                    let mut hash_tx_vec = hash_tx.0.to_vec();
                    hash_tx_vec.reverse();

                    let compact_tx = TonicCompactTx {
                        index: tx_id as u64,
                        hash: hash_tx_vec,
                        fee: 0,
                        spends,
                        outputs,
                    };
                    compact_txs.push(compact_tx);
                }
            }
            let mut hash = block.hash.0.to_vec();
            hash.reverse();
            // Set 0 in vector in the case of genesis block.
            let mut prev_hash = block.previousblockhash.unwrap_or_default().0.to_vec();
            prev_hash.reverse();
            // Shadowing mut variables as immutable.
            drop_mutability!(hash);
            drop_mutability!(prev_hash);
            drop_mutability!(compact_txs);

            let compact_block = TonicCompactBlock {
                proto_version: 0,
                height,
                hash,
                prev_hash,
                time: block.time,
                // (hash, prevHash, and time) OR (full header)
                header: Vec::new(),
                vtx: compact_txs,
            };
            on_block(compact_block)?;
        }
        Ok(())
    }

    async fn check_tx_existence(&mut self, tx_id: TxId) -> bool {
        let mut attempts = 0;
        loop {
            match self.get_raw_transaction_bytes(&H256Json::from(tx_id.0)).compat().await {
                Ok(_) => break,
                Err(e) => {
                    error!("Error on getting tx {}", tx_id);
                    if e.to_string().contains(NO_TX_ERROR_CODE) {
                        if attempts >= 3 {
                            return false;
                        }
                        attempts += 1;
                    }
                    Timer::sleep(30.).await;
                },
            }
        }
        true
    }
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

    fn get_latest_block(&self) -> Result<u32, MmError<SqliteError>> {
        Ok(query_single_row(
            &self.0,
            "SELECT height FROM compactblocks ORDER BY height DESC LIMIT 1",
            NO_PARAMS,
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

pub async fn create_wallet_db(
    wallet_db_path: PathBuf,
    consensus_params: ZcoinConsensusParams,
    check_point_block: Option<CheckPointBlockInfo>,
    evk: ExtendedFullViewingKey,
) -> Result<WalletDb<ZcoinConsensusParams>, MmError<ZcoinClientInitError>> {
    async_blocking({
        move || -> Result<WalletDb<ZcoinConsensusParams>, MmError<ZcoinClientInitError>> {
            let db = WalletDb::for_path(wallet_db_path, consensus_params)
                .map_to_mm(ZcoinClientInitError::WalletDbInitFailure)?;
            run_optimization_pragmas(db.sql_conn()).map_to_mm(ZcoinClientInitError::WalletDbInitFailure)?;
            init_wallet_db(&db).map_to_mm(ZcoinClientInitError::WalletDbInitFailure)?;
            if db.get_extended_full_viewing_keys()?.is_empty() {
                init_accounts_table(&db, &[evk])?;
                if let Some(check_point) = check_point_block {
                    init_blocks_table(
                        &db,
                        BlockHeight::from_u32(check_point.height),
                        BlockHash(check_point.hash.0),
                        check_point.time,
                        &check_point.sapling_tree.0,
                    )?;
                }
            }
            Ok(db)
        }
    })
    .await
}

pub(super) async fn init_light_client(
    coin: String,
    lightwalletd_urls: Vec<String>,
    blocks_db: BlockDb,
    wallet_db: WalletDbShared,
    consensus_params: ZcoinConsensusParams,
    scan_blocks_per_iteration: u32,
    scan_interval_ms: u64,
) -> Result<(AsyncMutex<SaplingSyncConnector>, WalletDbShared), MmError<ZcoinClientInitError>> {
    let (sync_status_notifier, sync_watcher) = channel(1);
    let (on_tx_gen_notifier, on_tx_gen_watcher) = channel(1);
    let mut rpc_clients = Vec::new();
    let mut errors = Vec::new();
    if lightwalletd_urls.is_empty() {
        return MmError::err(ZcoinClientInitError::EmptyLightwalletdUris);
    }
    for url in lightwalletd_urls {
        let uri = match Uri::from_str(&url) {
            Ok(uri) => uri,
            Err(err) => {
                errors.push(UrlIterError::InvalidUri(err));
                continue;
            },
        };
        let endpoint = match Channel::builder(uri).tls_config(ClientTlsConfig::new()) {
            Ok(endpoint) => endpoint,
            Err(err) => {
                errors.push(UrlIterError::TlsConfigFailure(err));
                continue;
            },
        };
        let tonic_channel = match endpoint.connect().await {
            Ok(tonic_channel) => tonic_channel,
            Err(err) => {
                errors.push(UrlIterError::ConnectionFailure(err));
                continue;
            },
        };
        rpc_clients.push(CompactTxStreamerClient::new(tonic_channel));
    }
    drop_mutability!(errors);
    drop_mutability!(rpc_clients);
    // check if rpc_clients is empty, then for loop wasn't successful
    if rpc_clients.is_empty() {
        return MmError::err(ZcoinClientInitError::UrlIterFailure(errors));
    }

    let sync_handle = SaplingSyncLoopHandle {
        coin,
        current_block: BlockHeight::from_u32(0),
        blocks_db,
        wallet_db: wallet_db.clone(),
        consensus_params,
        sync_status_notifier,
        on_tx_gen_watcher,
        watch_for_tx: None,
        scan_blocks_per_iteration,
        scan_interval_ms,
    };

    let light_rpc_clients = LightRpcClient {
        rpc_clients: AsyncMutex::new(rpc_clients),
    };
    let abort_handle = spawn_abortable(light_wallet_db_sync_loop(sync_handle, Box::new(light_rpc_clients)));

    Ok((
        SaplingSyncConnector::new_mutex_wrapped(sync_watcher, on_tx_gen_notifier, abort_handle),
        wallet_db,
    ))
}

pub(super) async fn init_native_client(
    coin: String,
    native_client: NativeClient,
    blocks_db: BlockDb,
    wallet_db: WalletDbShared,
    consensus_params: ZcoinConsensusParams,
    scan_blocks_per_iteration: u32,
    scan_interval_ms: u64,
) -> Result<(AsyncMutex<SaplingSyncConnector>, WalletDbShared), MmError<ZcoinClientInitError>> {
    let (sync_status_notifier, sync_watcher) = channel(1);
    let (on_tx_gen_notifier, on_tx_gen_watcher) = channel(1);

    let sync_handle = SaplingSyncLoopHandle {
        coin,
        current_block: BlockHeight::from_u32(0),
        blocks_db,
        wallet_db: wallet_db.clone(),
        consensus_params,
        sync_status_notifier,
        on_tx_gen_watcher,
        watch_for_tx: None,
        scan_blocks_per_iteration,
        scan_interval_ms,
    };
    let abort_handle = spawn_abortable(light_wallet_db_sync_loop(sync_handle, Box::new(native_client)));

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
    pub(super) sync_handle: Option<(SaplingSyncLoopHandle, Box<dyn ZRpcOps + Send>)>,
    pub(super) abort_handle: Arc<Mutex<AbortOnDropHandle>>,
}

impl Drop for SaplingSyncRespawnGuard {
    fn drop(&mut self) {
        if let Some((handle, rpc)) = self.sync_handle.take() {
            *self.abort_handle.lock() = spawn_abortable(light_wallet_db_sync_loop(handle, rpc));
        }
    }
}

impl SaplingSyncRespawnGuard {
    pub(super) fn watch_for_tx(&mut self, tx_id: TxId) {
        if let Some(ref mut handle) = self.sync_handle {
            handle.0.watch_for_tx = Some(tx_id);
        }
    }

    #[inline]
    pub(super) fn current_block(&self) -> BlockHeight {
        self.sync_handle.as_ref().expect("always Some").0.current_block
    }
}

pub enum SyncStatus {
    UpdatingBlocksCache {
        current_scanned_block: u64,
        latest_block: u64,
    },
    BuildingWalletDb {
        current_scanned_block: u64,
        latest_block: u64,
    },
    TemporaryError(String),
    Finished {
        block_number: u64,
    },
}

pub struct SaplingSyncLoopHandle {
    coin: String,
    current_block: BlockHeight,
    blocks_db: BlockDb,
    wallet_db: WalletDbShared,
    consensus_params: ZcoinConsensusParams,
    /// Notifies about sync status without stopping the loop, e.g. on coin activation
    sync_status_notifier: AsyncSender<SyncStatus>,
    /// If new tx is required to be generated, we stop the sync and respawn it after tx is sent
    /// This watcher waits for such notification
    on_tx_gen_watcher: AsyncReceiver<OneshotSender<(Self, Box<dyn ZRpcOps + Send>)>>,
    watch_for_tx: Option<TxId>,
    scan_blocks_per_iteration: u32,
    scan_interval_ms: u64,
}

impl SaplingSyncLoopHandle {
    fn notify_blocks_cache_status(&mut self, current_scanned_block: u64, latest_block: u64) {
        self.sync_status_notifier
            .try_send(SyncStatus::UpdatingBlocksCache {
                current_scanned_block,
                latest_block,
            })
            .debug_log_with_msg("No one seems interested in SyncStatus");
    }

    fn notify_building_wallet_db(&mut self, current_scanned_block: u64, latest_block: u64) {
        self.sync_status_notifier
            .try_send(SyncStatus::BuildingWalletDb {
                current_scanned_block,
                latest_block,
            })
            .debug_log_with_msg("No one seems interested in SyncStatus");
    }

    fn notify_on_error(&mut self, error: String) {
        self.sync_status_notifier
            .try_send(SyncStatus::TemporaryError(error))
            .debug_log_with_msg("No one seems interested in SyncStatus");
    }

    fn notify_sync_finished(&mut self) {
        self.sync_status_notifier
            .try_send(SyncStatus::Finished {
                block_number: self.current_block.into(),
            })
            .debug_log_with_msg("No one seems interested in SyncStatus");
    }

    async fn update_blocks_cache(
        &mut self,
        rpc: &mut (dyn ZRpcOps + Send),
    ) -> Result<(), MmError<UpdateBlocksCacheErr>> {
        let current_block = rpc.get_block_height().await?;
        let current_block_in_db = block_in_place(|| self.blocks_db.get_latest_block())?;
        let extrema = block_in_place(|| self.wallet_db.lock().block_height_extrema())?;
        let mut from_block = self
            .consensus_params
            .sapling_activation_height
            .max(current_block_in_db + 1) as u64;

        if let Some((_, max_in_wallet)) = extrema {
            from_block = from_block.max(max_in_wallet.into());
        }
        if current_block >= from_block {
            rpc.scan_blocks(from_block, current_block, &mut |block: TonicCompactBlock| {
                block_in_place(|| self.blocks_db.insert_block(block.height as u32, block.encode_to_vec()))?;
                self.notify_blocks_cache_status(block.height, current_block);
                Ok(())
            })
            .await?;
        }
        self.current_block = BlockHeight::from_u32(current_block as u32);
        Ok(())
    }

    /// Scans cached blocks, validates the chain and updates WalletDb.
    /// For more notes on the process, check https://github.com/zcash/librustzcash/blob/master/zcash_client_backend/src/data_api/chain.rs#L2
    fn scan_blocks(&mut self) -> Result<(), MmError<ZcashClientError>> {
        // required to avoid immutable borrow of self
        let wallet_db_arc = self.wallet_db.clone();
        let wallet_guard = wallet_db_arc.lock();
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

        let current_block = BlockHeight::from_u32(self.blocks_db.get_latest_block()?);
        loop {
            match wallet_ops.block_height_extrema()? {
                Some((_, max_in_wallet)) => {
                    if max_in_wallet >= current_block {
                        break;
                    } else {
                        self.notify_building_wallet_db(max_in_wallet.into(), current_block.into());
                    }
                },
                None => self.notify_building_wallet_db(0, current_block.into()),
            }

            scan_cached_blocks(
                &self.consensus_params,
                &self.blocks_db,
                &mut wallet_ops,
                Some(self.scan_blocks_per_iteration),
            )?;
            if self.scan_interval_ms > 0 {
                std::thread::sleep(Duration::from_millis(self.scan_interval_ms));
            }
        }
        Ok(())
    }

    async fn check_watch_for_tx_existence(&mut self, rpc: &mut (dyn ZRpcOps + Send)) {
        if let Some(tx_id) = self.watch_for_tx {
            if !rpc.check_tx_existence(tx_id).await {
                self.watch_for_tx = None;
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
async fn light_wallet_db_sync_loop(mut sync_handle: SaplingSyncLoopHandle, mut client: Box<dyn ZRpcOps + Send>) {
    info!(
        "(Re)starting light_wallet_db_sync_loop for {}, blocks per iteration {}, interval in ms {}",
        sync_handle.coin, sync_handle.scan_blocks_per_iteration, sync_handle.scan_interval_ms
    );
    // this loop is spawned as standalone task so it's safe to use block_in_place here
    loop {
        if let Err(e) = sync_handle.update_blocks_cache(client.as_mut()).await {
            error!("Error {} on blocks cache update", e);
            sync_handle.notify_on_error(e.to_string());
            Timer::sleep(10.).await;
            continue;
        }

        if let Err(e) = block_in_place(|| sync_handle.scan_blocks()) {
            error!("Error {} on scan_blocks", e);
            sync_handle.notify_on_error(e.to_string());
            Timer::sleep(10.).await;
            continue;
        }

        sync_handle.notify_sync_finished();

        sync_handle.check_watch_for_tx_existence(client.as_mut()).await;

        if let Some(tx_id) = sync_handle.watch_for_tx {
            if !block_in_place(|| is_tx_imported(sync_handle.wallet_db.lock().sql_conn(), tx_id)) {
                info!("Tx {} is not imported yet", tx_id);
                Timer::sleep(10.).await;
                continue;
            }
            sync_handle.watch_for_tx = None;
        }

        if let Ok(Some(sender)) = sync_handle.on_tx_gen_watcher.try_next() {
            match sender.send((sync_handle, client)) {
                Ok(_) => break,
                Err((handle_from_channel, rpc_from_channel)) => {
                    sync_handle = handle_from_channel;
                    client = rpc_from_channel;
                },
            }
        }

        Timer::sleep(10.).await;
    }
}

type SyncWatcher = AsyncReceiver<SyncStatus>;
type NewTxNotifier = AsyncSender<OneshotSender<(SaplingSyncLoopHandle, Box<dyn ZRpcOps + Send>)>>;

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
            .map(|(handle, rpc)| SaplingSyncRespawnGuard {
                sync_handle: Some((handle, rpc)),
                abort_handle: self.abort_handle.clone(),
            })
            .map_to_mm(|_| BlockchainScanStopped {})
    }
}

pub(super) struct SaplingSyncGuard<'a> {
    pub(super) _connector_guard: AsyncMutexGuard<'a, SaplingSyncConnector>,
    pub(super) respawn_guard: SaplingSyncRespawnGuard,
}
