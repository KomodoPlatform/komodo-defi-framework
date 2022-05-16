use super::ZcoinConsensusParams;
use crate::utxo::rpc_clients::{NativeClient, UtxoRpcFut};
use common::executor::Timer;
use common::log::{debug, error, info};
use common::mm_error::prelude::*;
use common::mm_number::MmNumber;
use common::{async_blocking, spawn_abortable, AbortOnDropHandle};
use db_common::sqlite::rusqlite::{params, Connection, Error, NO_PARAMS};
use db_common::sqlite::{query_single_row, run_optimization_pragmas};
use derive_more::Display;
use futures::channel::mpsc::{Receiver as AsyncReceiver, Sender as AsyncSender};
use futures::channel::oneshot::Sender as OneshotSender;
use http::Uri;
use parking_lot::{Mutex, MutexGuard};
use prost::Message;
use protobuf::Message as ProtobufMessage;
use rustls::ClientConfig;
use std::path::Path;
use std::sync::Arc;
use tokio::task::block_in_place;
use tonic::transport::{Channel, ClientTlsConfig};
use zcash_client_backend::data_api::chain::{scan_cached_blocks, validate_chain};
use zcash_client_backend::data_api::error::Error as ChainError;
use zcash_client_backend::data_api::{BlockSource, WalletRead, WalletWrite};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::wallet::{AccountId, SpendableNote};
use zcash_client_sqlite::error::SqliteClientError;
use zcash_client_sqlite::wallet::get_balance;
use zcash_client_sqlite::wallet::init::{init_accounts_table, init_wallet_db};
use zcash_client_sqlite::wallet::transact::get_spendable_notes;
use zcash_client_sqlite::WalletDb;
use zcash_primitives::consensus::BlockHeight;
use zcash_primitives::transaction::TxId;
use zcash_primitives::zip32::ExtendedFullViewingKey;

mod z_coin_grpc {
    tonic::include_proto!("cash.z.wallet.sdk.rpc");
}
use z_coin_grpc::compact_tx_streamer_client::CompactTxStreamerClient;
use z_coin_grpc::{BlockId, BlockRange, ChainSpec};

pub struct ZcoinNativeClient {
    pub(super) client: NativeClient,
    /// SQLite connection that is used to cache Sapling data for shielded transactions creation
    pub(super) sqlite: Mutex<Connection>,
}

impl ZcoinNativeClient {
    #[inline(always)]
    pub(super) fn sqlite_conn(&self) -> MutexGuard<'_, Connection> { self.sqlite.lock() }
}

pub enum ZcoinRpcClient {
    Native(ZcoinNativeClient),
    Light(ZcoinLightClient),
}

impl From<ZcoinLightClient> for ZcoinRpcClient {
    fn from(client: ZcoinLightClient) -> Self { ZcoinRpcClient::Light(client) }
}

pub type WalletDbShared = Arc<Mutex<ZcoinWalletDb>>;

pub struct ZcoinWalletDb {
    blocks_db: BlockDb,
    wallet_db: WalletDb<ZcoinConsensusParams>,
}

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
    ) -> Result<(), SqliteClientError>
    where
        F: FnMut(CompactBlock) -> Result<(), SqliteClientError>,
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
                return Err(SqliteClientError::CorruptedData(format!(
                    "Block height {} did not match row's height field value {}",
                    block.height(),
                    cbr.height
                )));
            }

            with_row(block)?;
        }

        Ok(())
    }

    fn get_latest_block(&self) -> Result<i64, MmError<db_common::sqlite::rusqlite::Error>> {
        Ok(query_single_row(
            &self.0,
            "SELECT height FROM compactblocks ORDER BY height DESC LIMIT 1",
            db_common::sqlite::rusqlite::NO_PARAMS,
            |row| row.get(0),
        )
        .map_err(MmError::new)?
        .unwrap_or(0))
    }

    fn insert_block(
        &self,
        height: u32,
        cb_bytes: Vec<u8>,
    ) -> Result<usize, MmError<db_common::sqlite::rusqlite::Error>> {
        self.0
            .prepare("INSERT INTO compactblocks (height, data) VALUES (?, ?)")
            .map_err(MmError::new)?
            .execute(params![height, cb_bytes])
            .map_err(MmError::new)
    }
}

impl BlockSource for BlockDb {
    type Error = SqliteClientError;

    fn with_blocks<F>(&self, from_height: BlockHeight, limit: Option<u32>, with_row: F) -> Result<(), Self::Error>
    where
        F: FnMut(CompactBlock) -> Result<(), Self::Error>,
    {
        self.with_blocks(from_height, limit, with_row)
    }
}

#[allow(dead_code)]
pub struct ZcoinLightClient {
    db: WalletDbShared,
}

#[derive(Debug, Display)]
#[non_exhaustive]
pub enum ZcoinLightClientInitError {
    TlsConfigFailure(tonic::transport::Error),
    ConnectionFailure(tonic::transport::Error),
    BlocksDbInitFailure(db_common::sqlite::rusqlite::Error),
    WalletDbInitFailure(db_common::sqlite::rusqlite::Error),
    ZcashSqliteError(SqliteClientError),
}

impl From<SqliteClientError> for ZcoinLightClientInitError {
    fn from(err: SqliteClientError) -> Self { ZcoinLightClientInitError::ZcashSqliteError(err) }
}
#[derive(Debug, Display)]
#[non_exhaustive]
pub enum UpdateBlocksCacheErr {
    GrpcError(tonic::Status),
    DbError(db_common::sqlite::rusqlite::Error),
}

impl From<tonic::Status> for UpdateBlocksCacheErr {
    fn from(err: tonic::Status) -> Self { UpdateBlocksCacheErr::GrpcError(err) }
}

impl From<db_common::sqlite::rusqlite::Error> for UpdateBlocksCacheErr {
    fn from(err: Error) -> Self { UpdateBlocksCacheErr::DbError(err) }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum MyBalanceError {}

impl ZcoinLightClient {
    pub async fn init(
        lightwalletd_url: Uri,
        cache_db_path: impl AsRef<Path> + Send + 'static,
        wallet_db_path: impl AsRef<Path> + Send + 'static,
        consensus_params: ZcoinConsensusParams,
        evk: ExtendedFullViewingKey,
        simple_sync_notifier: AsyncSender<BlockHeight>,
        on_tx_generation_watcher: OnTxGenWatcher,
    ) -> Result<(Self, AbortOnDropHandle), MmError<ZcoinLightClientInitError>> {
        let blocks_db = async_blocking(|| {
            BlockDb::for_path(cache_db_path).map_to_mm(ZcoinLightClientInitError::BlocksDbInitFailure)
        })
        .await?;

        let wallet_db = async_blocking({
            let consensus_params = consensus_params.clone();
            move || {
                WalletDb::for_path(wallet_db_path, consensus_params)
                    .map_to_mm(ZcoinLightClientInitError::WalletDbInitFailure)
            }
        })
        .await?;
        block_in_place(|| {
            run_optimization_pragmas(wallet_db.sql_conn()).map_to_mm(ZcoinLightClientInitError::WalletDbInitFailure)
        })?;

        block_in_place(|| init_wallet_db(&wallet_db).map_to_mm(ZcoinLightClientInitError::WalletDbInitFailure))?;

        let existing_keys = block_in_place(|| wallet_db.get_extended_full_viewing_keys())?;
        if existing_keys.is_empty() {
            block_in_place(|| init_accounts_table(&wallet_db, &[evk]))?;
        }

        let mut config = ClientConfig::new();
        config
            .root_store
            .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
        config.set_protocols(&["h2".to_string().into()]);
        let tls = ClientTlsConfig::new().rustls_client_config(config);

        let channel = Channel::builder(lightwalletd_url)
            .tls_config(tls)
            .map_to_mm(ZcoinLightClientInitError::TlsConfigFailure)?
            .connect()
            .await
            .map_to_mm(ZcoinLightClientInitError::ConnectionFailure)?;
        let grpc_client = CompactTxStreamerClient::new(channel);

        let db = Arc::new(Mutex::new(ZcoinWalletDb { blocks_db, wallet_db }));
        let sync_handle = SaplingSyncRespawnHandle {
            current_block: BlockHeight::from_u32(0),
            grpc_client,
            db: db.clone(),
            consensus_params,
            simple_sync_notifier,
            on_tx_gen_watcher: on_tx_generation_watcher,
            watch_for_tx: None,
        };
        let db_sync_abort_handler = spawn_abortable(light_wallet_db_sync_loop(sync_handle));

        Ok((ZcoinLightClient { db }, db_sync_abort_handler))
    }
}

pub async fn update_blocks_cache(
    grpc_client: &mut CompactTxStreamerClient<Channel>,
    db: &WalletDbShared,
) -> Result<BlockHeight, MmError<UpdateBlocksCacheErr>> {
    let request = tonic::Request::new(ChainSpec {});
    let current_blockchain_block = grpc_client.get_latest_block(request).await?;
    let current_block_in_db = db.lock().blocks_db.get_latest_block()?;

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

        let mut response = grpc_client.get_block_range(request).await?;

        while let Ok(Some(block)) = response.get_mut().message().await {
            common::log::info!("Got block {:?}", block);
            db.lock()
                .blocks_db
                .insert_block(block.height as u32, block.encode_to_vec())?;
        }
    }
    Ok(BlockHeight::from_u32(current_block as u32))
}

fn is_tx_imported(conn: &Connection, tx_id: TxId) -> bool {
    const QUERY: &str = "SELECT id_tx FROM transactions WHERE txid = ?1;";
    match query_single_row(conn, QUERY, [tx_id.0.to_vec()], |row| row.get::<_, i64>(0)) {
        Ok(Some(_)) => true,
        Ok(None) | Err(_) => false,
    }
}

type OnTxGenWatcher = AsyncReceiver<OneshotSender<SaplingSyncRespawnHandle>>;

pub struct SaplingSyncRespawnGuard {
    pub(super) sync_handle: Option<SaplingSyncRespawnHandle>,
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

pub struct SaplingSyncRespawnHandle {
    pub(super) current_block: BlockHeight,
    grpc_client: CompactTxStreamerClient<Channel>,
    db: WalletDbShared,
    consensus_params: ZcoinConsensusParams,
    /// Notifies that sync is done without stopping the loop, e.g. on coin activation
    simple_sync_notifier: AsyncSender<BlockHeight>,
    /// If new tx is required to be generated, we stop the sync and respawn it after tx is sent
    /// This watcher waits for such notification
    on_tx_gen_watcher: AsyncReceiver<OneshotSender<Self>>,
    pub(super) watch_for_tx: Option<TxId>,
}

async fn light_wallet_db_sync_loop(mut sync_handle: SaplingSyncRespawnHandle) {
    loop {
        sync_handle.current_block = match update_blocks_cache(&mut sync_handle.grpc_client, &sync_handle.db).await {
            Ok(b) => b,
            Err(e) => {
                error!("Error {} on blocks cache update", e);
                Timer::sleep(10.).await;
                continue;
            },
        };

        {
            let db_guard = sync_handle.db.lock();
            let mut db_data = db_guard.wallet_db.get_update_ops().unwrap();

            // 1) Download new CompactBlocks into db_cache.

            // 2) Run the chain validator on the received blocks.
            //
            // Given that we assume the server always gives us correct-at-the-time blocks, any
            // errors are in the blocks we have previously cached or scanned.
            if let Err(e) = validate_chain(
                &sync_handle.consensus_params,
                &db_guard.blocks_db,
                db_data.get_max_height_hash().unwrap(),
            ) {
                match e {
                    SqliteClientError::BackendError(ChainError::InvalidChain(lower_bound, _)) => {
                        // a) Pick a height to rewind to.
                        //
                        // This might be informed by some external chain reorg information, or
                        // heuristics such as the platform, available bandwidth, size of recent
                        // CompactBlocks, etc.
                        let rewind_height = lower_bound - 10;

                        // b) Rewind scanned block information.
                        db_data.rewind_to_height(rewind_height).unwrap();
                        // c) Delete cached blocks from rewind_height onwards.
                        //
                        // This does imply that assumed-valid blocks will be re-downloaded, but it
                        // is also possible that in the intervening time, a chain reorg has
                        // occurred that orphaned some of those blocks.

                        // d) If there is some separate thread or service downloading
                        // CompactBlocks, tell it to go back and download from rewind_height
                        // onwards.
                    },
                    e => {
                        // Handle or return other errors.
                        panic!("{:?}", e);
                    },
                }
            }

            // 3) Scan (any remaining) cached blocks.
            //
            // At this point, the cache and scanned data are locally consistent (though not
            // necessarily consistent with the latest chain tip - this would be discovered the
            // next time this codepath is executed after new blocks are received).
            scan_cached_blocks(&sync_handle.consensus_params, &db_guard.blocks_db, &mut db_data, None).unwrap();
        }

        if let Some(tx_id) = sync_handle.watch_for_tx {
            if !is_tx_imported(sync_handle.db.lock().wallet_db.sql_conn(), tx_id) {
                info!("Tx {} is not imported yet", hex::encode(tx_id.0));
                Timer::sleep(10.).await;
                continue;
            } else {
                sync_handle.watch_for_tx = None;
            }
        }

        if sync_handle
            .simple_sync_notifier
            .try_send(sync_handle.current_block)
            .is_err()
        {
            debug!("No one seems to be interested about sync state");
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

#[test]
// This is a temporary test used to experiment with librustzcash and lightwalletd
fn try_grpc() {
    use common::block_on;
    use futures::channel::mpsc::channel;
    use futures::executor::block_on_stream;
    use std::str::FromStr;
    use z_coin_grpc::RawTransaction;
    use zcash_client_backend::encoding::decode_extended_spending_key;
    use zcash_client_backend::encoding::decode_payment_address;
    use zcash_primitives::consensus::Parameters;
    use zcash_primitives::transaction::components::Amount;
    use zcash_proofs::prover::LocalTxProver;

    let zombie_consensus_params = ZcoinConsensusParams::for_zombie();

    let z_key = decode_extended_spending_key(zombie_consensus_params.hrp_sapling_extended_spending_key(), "secret-extended-key-main1q0k2ga2cqqqqpq8m8j6yl0say83cagrqp53zqz54w38ezs8ly9ly5ptamqwfpq85u87w0df4k8t2lwyde3n9v0gcr69nu4ryv60t0kfcsvkr8h83skwqex2nf0vr32794fmzk89cpmjptzc22lgu5wfhhp8lgf3f5vn2l3sge0udvxnm95k6dtxj2jwlfyccnum7nz297ecyhmd5ph526pxndww0rqq0qly84l635mec0x4yedf95hzn6kcgq8yxts26k98j9g32kjc8y83fe").unwrap().unwrap();
    let evk = ExtendedFullViewingKey::from(&z_key);

    let (simple_sync_notifier, simple_sync_watcher) = channel(1);
    let (on_tx_gen_notifier, on_tx_gen_watcher) = channel(1);

    let uri = Uri::from_str("http://zombie.sirseven.me:443").unwrap();
    let (client, _handle) = block_on(ZcoinLightClient::init(
        uri,
        "test_cache_zombie.db",
        "test_wallet_zombie.db",
        zombie_consensus_params.clone(),
        evk,
        simple_sync_notifier,
        on_tx_gen_watcher,
    ))
    .unwrap();

    let current_block = block_on_stream(simple_sync_watcher).next().unwrap();
    let db = client.db.lock();

    let mut notes = db.wallet_db.get_spendable_notes(AccountId(0), current_block).unwrap();

    notes.sort_by(|a, b| b.note_value.cmp(&a.note_value));
    use zcash_primitives::consensus;
    use zcash_primitives::transaction::builder::Builder as ZTxBuilder;

    let mut tx_builder = ZTxBuilder::new(zombie_consensus_params.clone(), current_block);

    let from_key = ExtendedFullViewingKey::from(&z_key);
    let (_, from_addr) = from_key.default_address().unwrap();
    let to_addr = decode_payment_address(
        zombie_consensus_params.hrp_sapling_payment_address(),
        "zs1aj847k37mwme6rx40vfv3rutqv3q3sczxh9alqyhvsdrkjgu6cedczyxm25fk4elhglxg3a3mfv",
    )
    .unwrap()
    .unwrap();

    let amount_to_send = Amount::from_i64(777777777).unwrap();
    let change = notes[0].note_value - amount_to_send - Amount::from_i64(1000).unwrap();
    for spendable_note in notes.into_iter().take(1) {
        let note = from_addr
            .create_note(spendable_note.note_value.into(), spendable_note.rseed)
            .unwrap();
        tx_builder
            .add_sapling_spend(
                z_key.clone(),
                spendable_note.diversifier,
                note,
                spendable_note.witness.path().unwrap(),
            )
            .unwrap();
    }

    tx_builder
        .add_sapling_output(None, to_addr, amount_to_send, None)
        .unwrap();
    tx_builder.add_sapling_output(None, from_addr, change, None).unwrap();

    let prover = LocalTxProver::with_default_location().unwrap();
    let (transaction, _) = tx_builder.build(consensus::BranchId::Sapling, &prover).unwrap();

    println!("{:?}", transaction);

    let mut tx_bytes = Vec::with_capacity(1024);
    transaction.write(&mut tx_bytes).expect("Write should not fail");

    let request = tonic::Request::new(RawTransaction {
        data: tx_bytes,
        height: 0,
    });

    let mut config = ClientConfig::new();
    config
        .root_store
        .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
    config.set_protocols(&["h2".to_string().into()]);
    let tls = ClientTlsConfig::new().rustls_client_config(config);

    let channel = block_on(
        Channel::from_static("http://zombie.sirseven.me:443")
            .tls_config(tls)
            .unwrap()
            .connect(),
    )
    .unwrap();
    let mut grpc_client = CompactTxStreamerClient::new(channel);
    let response = block_on(grpc_client.send_transaction(request)).unwrap();

    println!("{:?}", response);
}

impl ZcoinRpcClient {
    pub async fn my_balance_sat(&self) -> Result<u64, ()> {
        match self {
            ZcoinRpcClient::Native(_) => unimplemented!(),
            ZcoinRpcClient::Light(light) => {
                let db = light.db.clone();
                async_blocking(move || {
                    let balance = get_balance(&db.lock().wallet_db, AccountId::default()).unwrap().into();
                    Ok(balance)
                })
                .await
            },
        }
    }

    pub fn z_get_balance(&self, _address: &str, _min_conf: u32) -> UtxoRpcFut<MmNumber> { todo!() }

    pub async fn get_spendable_notes(&self) -> Result<Vec<SpendableNote>, MmError<SqliteClientError>> {
        match self {
            ZcoinRpcClient::Native(_) => unimplemented!(),
            ZcoinRpcClient::Light(light) => {
                let db = light.db.clone();
                async_blocking(move || {
                    let guard = db.lock();
                    let (_, latest_db_block) = guard.wallet_db.block_height_extrema()?.unwrap();
                    Ok(get_spendable_notes(
                        &guard.wallet_db,
                        AccountId::default(),
                        latest_db_block,
                    )?)
                })
                .await
            },
        }
    }

    pub fn z_import_key(&self, _key: &str) -> UtxoRpcFut<()> { todo!() }
}
