use super::*;
use crate::lightning::ln_db::LightningDB;
use crate::lightning::ln_filesystem_persister::LightningPersisterShared;
use crate::lightning::ln_platform::{get_best_header, ln_best_block_update_loop, update_best_block};
use crate::lightning::ln_sql::SqliteLightningDB;
use crate::lightning::ln_storage::{LightningStorage, NodesAddressesMap, Scorer};
use crate::utxo::rpc_clients::BestBlock as RpcBestBlock;
use bitcoin::hash_types::BlockHash;
use bitcoin_hashes::{sha256d, Hash};
use common::executor::{spawn, Timer};
use common::log;
use common::log::LogState;
use lightning::chain::keysinterface::{InMemorySigner, KeysManager};
use lightning::chain::{chainmonitor, BestBlock, Watch};
use lightning::ln::channelmanager;
use lightning::ln::channelmanager::{ChainParameters, ChannelManagerReadArgs, SimpleArcChannelManager};
use lightning::util::config::UserConfig;
use lightning::util::ser::ReadableArgs;
use mm2_core::mm_ctx::MmArc;
use std::fs::File;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

const SCORER_PERSIST_INTERVAL: u64 = 600;

pub type ChainMonitor = chainmonitor::ChainMonitor<
    InMemorySigner,
    Arc<Platform>,
    Arc<Platform>,
    Arc<Platform>,
    Arc<LogState>,
    LightningPersisterShared,
>;

pub type ChannelManager = SimpleArcChannelManager<ChainMonitor, Platform, Platform, LogState>;

#[inline]
fn ln_data_dir(ctx: &MmArc, ticker: &str) -> PathBuf { ctx.dbdir().join("LIGHTNING").join(ticker) }

#[inline]
fn ln_data_backup_dir(ctx: &MmArc, path: Option<String>, ticker: &str) -> Option<PathBuf> {
    path.map(|p| {
        PathBuf::from(&p)
            .join(&hex::encode(&**ctx.rmd160()))
            .join("LIGHTNING")
            .join(ticker)
    })
}

pub async fn init_persister(
    ctx: &MmArc,
    ticker: String,
    backup_path: Option<String>,
) -> EnableLightningResult<LightningPersisterShared> {
    let ln_data_dir = ln_data_dir(ctx, &ticker);
    let ln_data_backup_dir = ln_data_backup_dir(ctx, backup_path, &ticker);
    let persister = LightningPersisterShared(Arc::new(LightningFilesystemPersister::new(
        ln_data_dir,
        ln_data_backup_dir,
    )));

    let is_initialized = persister.is_fs_initialized().await?;
    if !is_initialized {
        persister.init_fs().await?;
    }

    Ok(persister)
}

pub async fn init_db(ctx: &MmArc, ticker: String) -> EnableLightningResult<SqliteLightningDB> {
    let db = SqliteLightningDB::new(
        ticker,
        ctx.sqlite_connection
            .ok_or(MmError::new(EnableLightningError::DbError(
                "sqlite_connection is not initialized".into(),
            )))?
            .clone(),
    );

    if !db.is_db_initialized().await? {
        db.init_db().await?;
    }

    Ok(db)
}

pub fn init_keys_manager(ctx: &MmArc) -> EnableLightningResult<Arc<KeysManager>> {
    // The current time is used to derive random numbers from the seed where required, to ensure all random generation is unique across restarts.
    let seed: [u8; 32] = ctx.secp256k1_key_pair().private().secret.into();
    let cur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_to_mm(|e| EnableLightningError::SystemTimeError(e.to_string()))?;

    Ok(Arc::new(KeysManager::new(&seed, cur.as_secs(), cur.subsec_nanos())))
}

pub async fn init_channel_manager(
    platform: Arc<Platform>,
    logger: Arc<LogState>,
    persister: LightningPersisterShared,
    db: SqliteLightningDB,
    keys_manager: Arc<KeysManager>,
    user_config: UserConfig,
) -> EnableLightningResult<(Arc<ChainMonitor>, Arc<ChannelManager>)> {
    // Initialize the FeeEstimator. UtxoStandardCoin implements the FeeEstimator trait, so it'll act as our fee estimator.
    let fee_estimator = platform.clone();

    // Initialize the BroadcasterInterface. UtxoStandardCoin implements the BroadcasterInterface trait, so it'll act as our transaction
    // broadcaster.
    let broadcaster = platform.clone();

    // Initialize the ChainMonitor
    let chain_monitor: Arc<ChainMonitor> = Arc::new(chainmonitor::ChainMonitor::new(
        Some(platform.clone()),
        broadcaster.clone(),
        logger.clone(),
        fee_estimator.clone(),
        persister.clone(),
    ));

    // Read ChannelMonitor state from disk, important for lightning node is restarting and has at least 1 channel
    let mut channelmonitors = persister
        .channels_persister()
        .read_channelmonitors(keys_manager.clone())
        .map_to_mm(|e| EnableLightningError::IOError(e.to_string()))?;

    // This is used for Electrum only to prepare for chain synchronization
    for (_, chan_mon) in channelmonitors.iter() {
        chan_mon.load_outputs_to_watch(&platform);
    }

    let rpc_client = match &platform.coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Electrum(c) => c.clone(),
        UtxoRpcClientEnum::Native(_) => {
            return MmError::err(EnableLightningError::UnsupportedMode(
                "Lightning network".into(),
                "electrum".into(),
            ))
        },
    };
    let best_header = get_best_header(&rpc_client).await?;
    platform.update_best_block_height(best_header.block_height());
    let best_block = RpcBestBlock::from(best_header.clone());
    let best_block_hash = BlockHash::from_hash(sha256d::Hash::from_inner(best_block.hash.0));
    let (channel_manager_blockhash, channel_manager) = {
        if let Ok(mut f) = File::open(persister.manager_path()) {
            let mut channel_monitor_mut_references = Vec::new();
            for (_, channel_monitor) in channelmonitors.iter_mut() {
                channel_monitor_mut_references.push(channel_monitor);
            }
            // Read ChannelManager data from the file
            let read_args = ChannelManagerReadArgs::new(
                keys_manager.clone(),
                fee_estimator.clone(),
                chain_monitor.clone(),
                broadcaster.clone(),
                logger.clone(),
                user_config,
                channel_monitor_mut_references,
            );
            <(BlockHash, ChannelManager)>::read(&mut f, read_args)
                .map_to_mm(|e| EnableLightningError::IOError(e.to_string()))?
        } else {
            // Initialize the ChannelManager to starting a new node without history
            let chain_params = ChainParameters {
                network: platform.network.clone().into(),
                best_block: BestBlock::new(best_block_hash, best_block.height as u32),
            };
            let new_channel_manager = channelmanager::ChannelManager::new(
                fee_estimator.clone(),
                chain_monitor.clone(),
                broadcaster.clone(),
                logger.clone(),
                keys_manager.clone(),
                user_config,
                chain_params,
            );
            (best_block_hash, new_channel_manager)
        }
    };

    let channel_manager: Arc<ChannelManager> = Arc::new(channel_manager);

    // Sync ChannelMonitors and ChannelManager to chain tip if the node is restarting and has open channels
    platform
        .process_txs_confirmations(&rpc_client, &db, &chain_monitor, &channel_manager)
        .await;
    if channel_manager_blockhash != best_block_hash {
        platform
            .process_txs_unconfirmations(&chain_monitor, &channel_manager)
            .await;
        update_best_block(&chain_monitor, &channel_manager, best_header).await;
    }

    // Give ChannelMonitors to ChainMonitor
    for (_, channel_monitor) in channelmonitors.drain(..) {
        let funding_outpoint = channel_monitor.get_funding_txo().0;
        chain_monitor
            .watch_channel(funding_outpoint, channel_monitor)
            .map_to_mm(|e| EnableLightningError::IOError(format!("{:?}", e)))?;
    }

    // Update best block whenever there's a new chain tip or a block has been newly disconnected
    spawn(ln_best_block_update_loop(
        platform,
        db,
        chain_monitor.clone(),
        channel_manager.clone(),
        rpc_client.clone(),
        best_block,
    ));

    Ok((chain_monitor, channel_manager))
}

pub async fn persist_scorer_loop(persister: LightningPersisterShared, scorer: Arc<Mutex<Scorer>>) {
    loop {
        if let Err(e) = persister.save_scorer(scorer.clone()).await {
            log::warn!(
                "Failed to persist scorer error: {}, please check disk space and permissions",
                e
            );
        }
        Timer::sleep(SCORER_PERSIST_INTERVAL as f64).await;
    }
}

pub async fn get_open_channels_nodes_addresses(
    persister: LightningPersisterShared,
    channel_manager: Arc<ChannelManager>,
) -> EnableLightningResult<NodesAddressesMap> {
    let channels = channel_manager.list_channels();
    let mut nodes_addresses = persister.get_nodes_addresses().await?;
    nodes_addresses.retain(|pubkey, _node_addr| {
        channels
            .iter()
            .map(|chan| chan.counterparty.node_id)
            .any(|node_id| node_id == *pubkey)
    });
    Ok(nodes_addresses)
}
