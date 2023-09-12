use super::{z_coin_errors::*, BlockDbImpl, WalletDbShared, ZCoinBuilder, ZcoinConsensusParams};
use crate::utxo::rpc_clients::NativeClient;
use crate::z_coin::SyncStartPoint;
use async_trait::async_trait;
use common::executor::{spawn_abortable, AbortOnDropHandle};
use futures::channel::mpsc::{Receiver as AsyncReceiver, Sender as AsyncSender};
use futures::channel::oneshot::{channel as oneshot_channel, Sender as OneshotSender};
use futures::lock::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};
use futures::StreamExt;
use mm2_err_handle::prelude::*;
use parking_lot::Mutex;
use std::sync::Arc;
use zcash_primitives::consensus::BlockHeight;
use zcash_primitives::transaction::TxId;
use zcash_primitives::zip32::ExtendedSpendingKey;

cfg_native!(
    use crate::{RpcCommonOps, ZTransaction};
    use crate::utxo::rpc_clients::{UtxoRpcClientOps, NO_TX_ERROR_CODE};
    use crate::utxo::utxo_builder::{UtxoCoinBuilderCommonOps, DAY_IN_SECONDS};
    use crate::z_coin::storage::BlockDbError;
    use crate::z_coin::CheckPointBlockInfo;

    use db_common::sqlite::rusqlite::Connection;
    use db_common::sqlite::{query_single_row, run_optimization_pragmas};
    use common::{async_blocking, now_sec};
    use common::executor::Timer;
    use common::log::{debug, error, info, LogOnError};
    use common::Future01CompatExt;
    use futures::channel::mpsc::channel;
    use group::GroupEncoding;
    use hex::{FromHex, FromHexError};
    use http::Uri;
    use prost::Message;
    use rpc::v1::types::{Bytes, H256 as H256Json};
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::str::FromStr;
    use std::time::Duration;
    use tokio::task::block_in_place;
    use tonic::transport::{Channel, ClientTlsConfig};
    use zcash_client_backend::data_api::{WalletRead, WalletWrite};
    use zcash_client_backend::data_api::chain::{scan_cached_blocks, validate_chain};
    use zcash_client_backend::data_api::error::Error as ChainError;
    use zcash_primitives::block::BlockHash;
    use zcash_primitives::zip32::ExtendedFullViewingKey;
    use zcash_client_sqlite::error::SqliteClientError as ZcashClientError;
    use zcash_client_sqlite::wallet::init::{init_accounts_table, init_blocks_table, init_wallet_db};
    use zcash_client_sqlite::WalletDb;

    mod z_coin_grpc {
        tonic::include_proto!("pirate.wallet.sdk.rpc");
    }
    use z_coin_grpc::TreeState;
    use z_coin_grpc::compact_tx_streamer_client::CompactTxStreamerClient;
    use z_coin_grpc::{BlockId, BlockRange, ChainSpec, CompactBlock as TonicCompactBlock,
                  CompactOutput as TonicCompactOutput, CompactSpend as TonicCompactSpend, CompactTx as TonicCompactTx,
                  TxFilter};
);

#[cfg(not(target_arch = "wasm32"))]
pub type OnCompactBlockFn<'a> = dyn FnMut(TonicCompactBlock) -> Result<(), MmError<UpdateBlocksCacheErr>> + Send + 'a;

#[cfg(target_arch = "wasm32")]
#[allow(unused)]
pub type OnCompactBlockFn<'a> = dyn FnMut(String) -> Result<(), MmError<UpdateBlocksCacheErr>> + Send + 'a;

/// ZRpcOps trait provides asynchronous methods for performing various operations related to
/// Zcoin blockchain and wallet synchronization.
#[async_trait]
pub trait ZRpcOps {
    /// Asynchronously retrieve the current block height from the Zcoin network.
    async fn get_block_height(&mut self) -> Result<u64, MmError<UpdateBlocksCacheErr>>;

    /// Asynchronously retrieve the tree state for a specific block height from the Zcoin network.
    #[cfg(not(target_arch = "wasm32"))]
    async fn get_tree_state(&mut self, height: u64) -> Result<TreeState, MmError<UpdateBlocksCacheErr>>;

    /// Asynchronously scan and process blocks within a specified block height range.
    ///
    /// This method allows for scanning and processing blocks starting from `start_block` up to
    /// and including `last_block`. It invokes the provided `on_block` function for each compact
    /// block within the specified range.
    async fn scan_blocks(
        &mut self,
        start_block: u64,
        last_block: u64,
        on_block: &mut OnCompactBlockFn,
    ) -> Result<(), MmError<UpdateBlocksCacheErr>>;

    async fn check_tx_existence(&mut self, tx_id: TxId) -> bool;

    /// Retrieves checkpoint block information from the database at a specific height.
    ///
    /// checkpoint_block_from_height retrieves tree state information from rpc corresponding to the given
    /// height and constructs a `CheckPointBlockInfo` struct containing some needed details such as
    /// block height, hash, time, and sapling tree.
    #[cfg(not(target_arch = "wasm32"))]
    async fn checkpoint_block_from_height(
        &mut self,
        height: u64,
    ) -> MmResult<Option<CheckPointBlockInfo>, UpdateBlocksCacheErr>;
}

#[cfg(not(target_arch = "wasm32"))]
struct LightRpcClient {
    rpc_clients: AsyncMutex<Vec<CompactTxStreamerClient<Channel>>>,
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
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

    #[cfg(not(target_arch = "wasm32"))]
    async fn get_tree_state(&mut self, height: u64) -> Result<TreeState, MmError<UpdateBlocksCacheErr>> {
        let request = tonic::Request::new(BlockId { height, hash: vec![] });

        Ok(self
            .get_live_client()
            .await?
            .get_tree_state(request)
            .await
            .map_to_mm(UpdateBlocksCacheErr::GrpcError)?
            .into_inner())
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

    #[cfg(not(target_arch = "wasm32"))]
    async fn checkpoint_block_from_height(
        &mut self,
        height: u64,
    ) -> MmResult<Option<CheckPointBlockInfo>, UpdateBlocksCacheErr> {
        let tree_state = self.get_tree_state(height).await?;
        let hash = H256Json::from_str(&tree_state.hash)
            .map_err(|err| UpdateBlocksCacheErr::DecodeError(err.to_string()))?
            .reversed();
        let sapling_tree = Bytes::new(
            FromHex::from_hex(&tree_state.tree)
                .map_err(|err: FromHexError| UpdateBlocksCacheErr::DecodeError(err.to_string()))?,
        );

        Ok(Some(CheckPointBlockInfo {
            height: tree_state.height as u32,
            hash,
            time: tree_state.time,
            sapling_tree,
        }))
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
impl ZRpcOps for NativeClient {
    async fn get_block_height(&mut self) -> Result<u64, MmError<UpdateBlocksCacheErr>> {
        Ok(self.get_block_count().compat().await?)
    }

    #[cfg(not(target_arch = "wasm32"))]
    async fn get_tree_state(&mut self, _height: u64) -> Result<TreeState, MmError<UpdateBlocksCacheErr>> { todo!() }

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

    #[cfg(not(target_arch = "wasm32"))]
    async fn checkpoint_block_from_height(
        &mut self,
        _height: u64,
    ) -> MmResult<Option<CheckPointBlockInfo>, UpdateBlocksCacheErr> {
        todo!()
    }
}

/// `create_wallet_db` is responsible for creating a new Zcoin wallet database, initializing it
/// with the provided parameters, and executing various initialization steps. These steps include checking and
/// potentially rewinding the database to a specified synchronization height, performing optimizations, and
/// setting up the initial state of the wallet database.
#[cfg(not(target_arch = "wasm32"))]
pub async fn create_wallet_db(
    wallet_db_path: PathBuf,
    consensus_params: ZcoinConsensusParams,
    checkpoint_block: Option<CheckPointBlockInfo>,
    evk: ExtendedFullViewingKey,
) -> Result<WalletDb<ZcoinConsensusParams>, MmError<ZcoinClientInitError>> {
    async_blocking({
        move || -> Result<WalletDb<ZcoinConsensusParams>, MmError<ZcoinClientInitError>> {
            let db = WalletDb::for_path(wallet_db_path, consensus_params)
                .map_to_mm(|err| ZcoinClientInitError::ZcashDBError(err.to_string()))?;
            let extrema = db.block_height_extrema()?;
            let min_sync_height = extrema.map(|(min, _)| u32::from(min));
            let init_block_height = checkpoint_block.clone().map(|block| block.height);

            run_optimization_pragmas(db.sql_conn())
                .map_to_mm(|err| ZcoinClientInitError::ZcashDBError(err.to_string()))?;
            init_wallet_db(&db).map_to_mm(|err| ZcoinClientInitError::ZcashDBError(err.to_string()))?;

            // Check if the initial block height is less than the previous synchronization height and
            // Rewind walletdb to the minimum possible height.
            if db.get_extended_full_viewing_keys()?.is_empty() || init_block_height != min_sync_height {
                info!("Older/Newer sync height detected!, rewinding walletdb to new height: {init_block_height:?}");
                let mut wallet_ops = db.get_update_ops().expect("get_update_ops always returns Ok");
                wallet_ops
                    .rewind_to_height(u32::MIN.into())
                    .map_to_mm(|err| ZcoinClientInitError::ZcashDBError(err.to_string()))?;
                if let Some(block) = checkpoint_block.clone() {
                    init_blocks_table(
                        &db,
                        BlockHeight::from_u32(block.height),
                        BlockHash(block.hash.0),
                        block.time,
                        &block.sapling_tree.0,
                    )?;
                }
            }

            if db.get_extended_full_viewing_keys()?.is_empty() {
                init_accounts_table(&db, &[evk])?;
            }
            Ok(db)
        }
    })
    .await
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) async fn init_light_client<'a>(
    builder: &ZCoinBuilder<'a>,
    lightwalletd_urls: Vec<String>,
    blocks_db: BlockDbImpl,
    sync_params: &Option<SyncStartPoint>,
    z_spending_key: &ExtendedSpendingKey,
) -> Result<(AsyncMutex<SaplingSyncConnector>, WalletDbShared), MmError<ZcoinClientInitError>> {
    let coin = builder.ticker.to_string();
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

    let mut light_rpc_clients = LightRpcClient {
        rpc_clients: AsyncMutex::new(rpc_clients),
    };

    let current_block_height = light_rpc_clients
        .get_block_height()
        .await
        .mm_err(ZcoinClientInitError::UpdateBlocksCacheErr)?;
    let sapling_activation_height = builder.protocol_info.consensus_params.sapling_activation_height as u64;
    let sync_height = match sync_params {
        Some(SyncStartPoint::Date(date)) => builder
            .calculate_starting_height_from_date(*date, current_block_height)
            .mm_err(ZcoinClientInitError::UtxoCoinBuildError)?
            .unwrap_or(sapling_activation_height),
        Some(SyncStartPoint::Height(height)) => *height,
        Some(SyncStartPoint::Earliest) => sapling_activation_height,
        None => builder
            .calculate_starting_height_from_date(now_sec() - DAY_IN_SECONDS, current_block_height)
            .mm_err(ZcoinClientInitError::UtxoCoinBuildError)?
            .unwrap_or(sapling_activation_height),
    };
    let maybe_checkpoint_block = light_rpc_clients
        .checkpoint_block_from_height(sync_height.max(sapling_activation_height))
        .await?;

    let wallet_db = WalletDbShared::new(builder, maybe_checkpoint_block, z_spending_key)
        .await
        .mm_err(|err| ZcoinClientInitError::ZcashDBError(err.to_string()))?;

    // Get min_height in blocks_db and rewind blocks_db to 0 if sync_height != min_height
    let min_height = blocks_db.get_earliest_block().await?;
    if sync_height != min_height as u64 {
        blocks_db
            .rewind_to_height(u32::MIN)
            .map_err(|err| ZcoinClientInitError::ZcashDBError(err.to_string()))?;
    };

    let sync_handle = SaplingSyncLoopHandle {
        coin,
        current_block: BlockHeight::from_u32(0),
        blocks_db,
        wallet_db: wallet_db.clone(),
        consensus_params: builder.protocol_info.consensus_params.clone(),
        sync_status_notifier,
        on_tx_gen_watcher,
        watch_for_tx: None,
        scan_blocks_per_iteration: builder.z_coin_params.scan_blocks_per_iteration,
        scan_interval_ms: builder.z_coin_params.scan_interval_ms,
        first_sync_block: FirstSyncBlock {
            requested: sync_height,
            is_pre_sapling: sync_height < sapling_activation_height,
            actual: sync_height.max(sapling_activation_height),
        },
    };

    let abort_handle = spawn_abortable(light_wallet_db_sync_loop(sync_handle, Box::new(light_rpc_clients)));

    Ok((
        SaplingSyncConnector::new_mutex_wrapped(sync_watcher, on_tx_gen_notifier, abort_handle),
        wallet_db,
    ))
}

#[cfg(target_arch = "wasm32")]
#[allow(unused)]
pub(super) async fn init_light_client<'a>(
    _builder: &ZCoinBuilder<'a>,
    _lightwalletd_urls: Vec<String>,
    _blocks_db: BlockDbImpl,
    _sync_params: &Option<SyncStartPoint>,
    z_spending_key: &ExtendedSpendingKey,
) -> Result<(AsyncMutex<SaplingSyncConnector>, WalletDbShared), MmError<ZcoinClientInitError>> {
    todo!()
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) async fn init_native_client<'a>(
    builder: &ZCoinBuilder<'a>,
    native_client: NativeClient,
    blocks_db: BlockDbImpl,
    z_spending_key: &ExtendedSpendingKey,
) -> Result<(AsyncMutex<SaplingSyncConnector>, WalletDbShared), MmError<ZcoinClientInitError>> {
    let coin = builder.ticker.to_string();
    let (sync_status_notifier, sync_watcher) = channel(1);
    let (on_tx_gen_notifier, on_tx_gen_watcher) = channel(1);
    let checkpoint_block = builder.protocol_info.check_point_block.clone();
    let sapling_height = builder.protocol_info.consensus_params.sapling_activation_height;
    let checkpoint_height = checkpoint_block.clone().map(|b| b.height).unwrap_or(sapling_height) as u64;
    let first_sync_block = FirstSyncBlock {
        requested: checkpoint_height,
        is_pre_sapling: false,
        actual: checkpoint_height,
    };
    let wallet_db = WalletDbShared::new(builder, checkpoint_block, z_spending_key)
        .await
        .mm_err(|err| ZcoinClientInitError::ZcashDBError(err.to_string()))?;

    let sync_handle = SaplingSyncLoopHandle {
        coin,
        current_block: BlockHeight::from_u32(0),
        blocks_db,
        wallet_db: wallet_db.clone(),
        consensus_params: builder.protocol_info.consensus_params.clone(),
        sync_status_notifier,
        on_tx_gen_watcher,
        watch_for_tx: None,
        scan_blocks_per_iteration: builder.z_coin_params.scan_blocks_per_iteration,
        scan_interval_ms: builder.z_coin_params.scan_interval_ms,
        first_sync_block,
    };
    let abort_handle = spawn_abortable(light_wallet_db_sync_loop(sync_handle, Box::new(native_client)));

    Ok((
        SaplingSyncConnector::new_mutex_wrapped(sync_watcher, on_tx_gen_notifier, abort_handle),
        wallet_db,
    ))
}

#[cfg(target_arch = "wasm32")]
pub(super) async fn _init_native_client<'a>(
    _builder: &ZCoinBuilder<'a>,
    mut _native_client: NativeClient,
    _blocks_db: BlockDbImpl,
    _z_spending_key: &ExtendedSpendingKey,
) -> Result<(AsyncMutex<SaplingSyncConnector>, WalletDbShared), MmError<ZcoinClientInitError>> {
    todo!()
}

#[cfg(not(target_arch = "wasm32"))]
fn is_tx_imported(conn: &Connection, tx_id: TxId) -> bool {
    const QUERY: &str = "SELECT id_tx FROM transactions WHERE txid = ?1;";
    match query_single_row(conn, QUERY, [tx_id.0.to_vec()], |row| row.get::<_, i64>(0)) {
        Ok(Some(_)) => true,
        Ok(None) | Err(_) => false,
    }
}

#[cfg(target_arch = "wasm32")]
#[allow(unused)]
fn is_tx_imported(_conn: String, _tx_id: TxId) -> bool { todo!() }

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

#[allow(unused)]
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

/// `SyncStatus` enumerates different states that may occur during the execution of
/// Zcoin-related operations during block sync.
///
/// - `UpdatingBlocksCache`: Represents the state of updating the blocks cache, with associated data
///   about the first synchronization block, the current scanned block, and the latest block.
/// - `BuildingWalletDb`: Denotes the state of building the wallet db, with associated data about
///   the first synchronization block, the current scanned block, and the latest block.
/// - `TemporaryError(String)`: Represents a temporary error state, with an associated error message
///   providing details about the error.
/// - `RequestingWalletBalance`: Indicates the process of requesting the wallet balance.
/// - `Finishing`: Represents the finishing state of an operation.
pub enum SyncStatus {
    UpdatingBlocksCache {
        first_sync_block: FirstSyncBlock,
        current_scanned_block: u64,
        latest_block: u64,
    },
    BuildingWalletDb {
        first_sync_block: FirstSyncBlock,
        current_scanned_block: u64,
        latest_block: u64,
    },
    TemporaryError(String),
    Finished {
        first_sync_block: FirstSyncBlock,
        block_number: u64,
    },
}

/// The `FirstSyncBlock` struct contains details about the block block that is used to start the synchronization
/// process.
/// It includes information about the requested block height, whether it predates the Sapling activation, and the
/// actual starting block height used during synchronization.
///
/// - `requested`: The requested block height during synchronization.
/// - `is_pre_sapling`: Indicates whether the block predates the Sapling activation.
/// - `actual`: The actual block height used for synchronization(may be altered).
#[derive(Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FirstSyncBlock {
    pub requested: u64,
    pub is_pre_sapling: bool,
    pub actual: u64,
}

/// The `SaplingSyncLoopHandle` struct is used to manage and control Zcoin synchronization loop.
/// It includes information about the coin being synchronized, the current block height, database access, etc.
#[allow(unused)]
pub struct SaplingSyncLoopHandle {
    coin: String,
    current_block: BlockHeight,
    blocks_db: BlockDbImpl,
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
    first_sync_block: FirstSyncBlock,
}

#[cfg(not(target_arch = "wasm32"))]
impl SaplingSyncLoopHandle {
    fn first_sync_block(&self) -> FirstSyncBlock { self.first_sync_block.clone() }

    fn notify_blocks_cache_status(&mut self, current_scanned_block: u64, latest_block: u64) {
        self.sync_status_notifier
            .try_send(SyncStatus::UpdatingBlocksCache {
                current_scanned_block,
                latest_block,
                first_sync_block: self.first_sync_block(),
            })
            .debug_log_with_msg("No one seems interested in SyncStatus");
    }

    fn notify_building_wallet_db(&mut self, current_scanned_block: u64, latest_block: u64) {
        self.sync_status_notifier
            .try_send(SyncStatus::BuildingWalletDb {
                current_scanned_block,
                latest_block,
                first_sync_block: self.first_sync_block(),
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
                first_sync_block: self.first_sync_block(),
            })
            .debug_log_with_msg("No one seems interested in SyncStatus");
    }

    async fn update_blocks_cache(
        &mut self,
        rpc: &mut (dyn ZRpcOps + Send),
    ) -> Result<(), MmError<UpdateBlocksCacheErr>> {
        let current_block = rpc.get_block_height().await?;
        let current_block_in_db = block_in_place(|| self.blocks_db.get_latest_block())?;
        let wallet_db = self.wallet_db.clone();
        let extrema = block_in_place(|| {
            let conn = wallet_db.db.lock();
            conn.block_height_extrema()
        })?;
        let mut from_block = self
            .consensus_params
            .sapling_activation_height
            .max(current_block_in_db + 1) as u64;

        if let Some((_, max_in_wallet)) = extrema {
            from_block = from_block.max(max_in_wallet.into());
        }

        if current_block >= from_block {
            rpc.scan_blocks(from_block, current_block, &mut |block: TonicCompactBlock| {
                block_in_place(|| self.blocks_db.insert_block(block.height as u32, block.encode_to_vec()))
                    .map_err(|err| UpdateBlocksCacheErr::ZcashDBError(err.to_string()))?;
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
    fn scan_blocks(&mut self) -> Result<(), MmError<BlockDbError>> {
        // required to avoid immutable borrow of self
        let wallet_db_arc = self.wallet_db.clone();
        let wallet_guard = wallet_db_arc.db.lock();
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
                e => return MmError::err(BlockDbError::SqliteError(e)),
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

#[cfg(target_arch = "wasm32")]
#[allow(unused)]
impl SaplingSyncLoopHandle {
    fn notify_blocks_cache_status(&mut self, _current_scanned_block: u64, _latest_block: u64) { todo!() }

    fn notify_building_wallet_db(&mut self, _current_scanned_block: u64, _latest_block: u64) { todo!() }

    fn notify_on_error(&mut self, _error: String) { todo!() }

    fn notify_sync_finished(&mut self) { todo!() }

    async fn update_blocks_cache(
        &mut self,
        _rpc: &mut (dyn ZRpcOps + Send),
    ) -> Result<(), MmError<UpdateBlocksCacheErr>> {
        todo!()
    }

    /// Scans cached blocks, validates the chain and updates WalletDb.
    /// For more notes on the process, check https://github.com/zcash/librustzcash/blob/master/zcash_client_backend/src/data_api/chain.rs#L2
    fn scan_blocks(&mut self) -> Result<(), MmError<String>> { todo!() }

    async fn check_watch_for_tx_existence(&mut self, _rpc: &mut (dyn ZRpcOps + Send)) { todo!() }
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
#[cfg(not(target_arch = "wasm32"))]
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
            if !block_in_place(|| is_tx_imported(sync_handle.wallet_db.db.lock().sql_conn(), tx_id)) {
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

#[cfg(target_arch = "wasm32")]
async fn light_wallet_db_sync_loop(mut _sync_handle: SaplingSyncLoopHandle, mut _client: Box<dyn ZRpcOps + Send>) {
    todo!()
}

type SyncWatcher = AsyncReceiver<SyncStatus>;
type NewTxNotifier = AsyncSender<OneshotSender<(SaplingSyncLoopHandle, Box<dyn ZRpcOps + Send>)>>;

pub(super) struct SaplingSyncConnector {
    sync_watcher: SyncWatcher,
    on_tx_gen_notifier: NewTxNotifier,
    abort_handle: Arc<Mutex<AbortOnDropHandle>>,
}

impl SaplingSyncConnector {
    #[allow(unused)]
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
