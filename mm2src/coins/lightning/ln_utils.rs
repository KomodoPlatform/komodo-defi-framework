use super::*;
use crate::lightning::ln_db::LightningDB;
use crate::lightning::ln_platform::{get_best_header, ln_best_block_update_loop, update_best_block};
use crate::lightning::ln_sql::SqliteLightningDB;
use crate::lightning::ln_storage::{LightningStorage, NodesAddressesMap};
use crate::utxo::rpc_clients::BestBlock as RpcBestBlock;
use bitcoin::hash_types::BlockHash;
use bitcoin_hashes::{sha256d, Hash};
use common::executor::spawn;
use common::log::LogState;
use lightning::chain::keysinterface::{InMemorySigner, KeysManager};
use lightning::chain::{chainmonitor, BestBlock, Watch};
use lightning::ln::channelmanager::{ChainParameters, ChannelManagerReadArgs, SimpleArcChannelManager};
use lightning::util::config::UserConfig;
use lightning::util::ser::ReadableArgs;
use mm2_core::mm_ctx::MmArc;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

pub type ChainMonitor = chainmonitor::ChainMonitor<
    InMemorySigner,
    Arc<Platform>,
    Arc<Platform>,
    Arc<Platform>,
    Arc<LogState>,
    Arc<LightningFilesystemPersister>,
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
) -> EnableLightningResult<Arc<LightningFilesystemPersister>> {
    let ln_data_dir = ln_data_dir(ctx, &ticker);
    let ln_data_backup_dir = ln_data_backup_dir(ctx, backup_path, &ticker);
    let persister = Arc::new(LightningFilesystemPersister::new(ln_data_dir, ln_data_backup_dir));

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
    persister: Arc<LightningFilesystemPersister>,
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
    let channels_persister = persister.clone();
    let channels_keys_manager = keys_manager.clone();
    let mut channelmonitors = async_blocking(move || {
        channels_persister
            .read_channelmonitors(channels_keys_manager)
            .map_to_mm(|e| EnableLightningError::IOError(e.to_string()))
    })
    .await?;

    // This is used for Electrum only to prepare for chain synchronization
    for (_, chan_mon) in channelmonitors.iter() {
        // Although there is a mutex lock inside the load_outputs_to_watch fn
        // it shouldn't be held by anything yet, so async_blocking is not needed.
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

    let channel_manager = if persister.manager_path().exists() {
        let chain_monitor_for_args = chain_monitor.clone();

        let (channel_manager_blockhash, channel_manager, channelmonitors) = async_blocking(move || {
            let mut manager_file = match File::open(persister.manager_path()) {
                Ok(f) => f,
                Err(e) => return Err(e.into()),
            };

            let mut channel_monitor_mut_references = Vec::new();
            for (_, channel_monitor) in channelmonitors.iter_mut() {
                channel_monitor_mut_references.push(channel_monitor);
            }

            // Read ChannelManager data from the file
            let read_args = ChannelManagerReadArgs::new(
                keys_manager.clone(),
                fee_estimator.clone(),
                chain_monitor_for_args,
                broadcaster.clone(),
                logger.clone(),
                user_config,
                channel_monitor_mut_references,
            );
            <(BlockHash, Arc<ChannelManager>)>::read(&mut manager_file, read_args)
                .map(|(h, c)| (h, c, channelmonitors))
                .map_to_mm(|e| EnableLightningError::IOError(e.to_string()))
        })
        .await?;

        // Sync ChannelMonitors and ChannelManager to chain tip if the node is restarting and has open channels
        platform
            .process_txs_confirmations(
                &rpc_client,
                &db,
                Arc::clone(&chain_monitor),
                Arc::clone(&channel_manager),
            )
            .await;
        if channel_manager_blockhash != best_block_hash {
            platform
                .process_txs_unconfirmations(Arc::clone(&chain_monitor), Arc::clone(&channel_manager))
                .await;
            update_best_block(Arc::clone(&chain_monitor), Arc::clone(&channel_manager), best_header).await;
        }

        // Give ChannelMonitors to ChainMonitor
        for (_, channel_monitor) in channelmonitors.into_iter() {
            let funding_outpoint = channel_monitor.get_funding_txo().0;
            let chain_monitor = chain_monitor.clone();
            async_blocking(move || {
                chain_monitor
                    .watch_channel(funding_outpoint, channel_monitor)
                    .map_to_mm(|e| EnableLightningError::IOError(format!("{:?}", e)))
            })
            .await?;
        }
        channel_manager
    } else {
        // Initialize the ChannelManager to starting a new node without history
        let chain_params = ChainParameters {
            network: platform.network.clone().into(),
            best_block: BestBlock::new(best_block_hash, best_block.height as u32),
        };
        Arc::new(ChannelManager::new(
            fee_estimator.clone(),
            chain_monitor.clone(),
            broadcaster.clone(),
            logger.clone(),
            keys_manager.clone(),
            user_config,
            chain_params,
        ))
    };

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

pub async fn get_open_channels_nodes_addresses(
    persister: Arc<LightningFilesystemPersister>,
    channel_manager: Arc<ChannelManager>,
) -> EnableLightningResult<NodesAddressesMap> {
    let channels = async_blocking(move || channel_manager.list_channels()).await;
    let mut nodes_addresses = persister.get_nodes_addresses().await?;
    nodes_addresses.retain(|pubkey, _node_addr| {
        channels
            .iter()
            .map(|chan| chan.counterparty.node_id)
            .any(|node_id| node_id == *pubkey)
    });
    Ok(nodes_addresses)
}
