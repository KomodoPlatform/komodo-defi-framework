use super::*;
use crate::lightning::ln_db::LightningDB;
use crate::lightning::ln_platform::{get_best_header, ln_best_block_update_loop, update_best_block};
use crate::lightning::ln_sql::SqliteLightningDB;
use crate::lightning::ln_storage::{LightningStorage, NodesAddressesMap};
use crate::utxo::rpc_clients::BestBlock as RpcBestBlock;
use bitcoin::hash_types::BlockHash;
use bitcoin_hashes::{sha256d, Hash};
use common::executor::SpawnFuture;
use common::log::LogState;
use lightning::chain::keysinterface::{InMemorySigner, KeysManager};
use lightning::chain::{chainmonitor, BestBlock, Watch};
use lightning::ln::channelmanager::{ChainParameters, ChannelManagerReadArgs, SimpleArcChannelManager};
use lightning::routing::gossip::RoutingFees;
use lightning::routing::router::{RouteHint, RouteHintHop};
use lightning::util::config::UserConfig;
use lightning::util::ser::ReadableArgs;
use mm2_core::mm_ctx::MmArc;
use std::collections::hash_map::Entry;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

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
            .join(hex::encode(ctx.rmd160().as_slice()))
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

pub fn init_keys_manager(platform: &Platform) -> EnableLightningResult<Arc<KeysManager>> {
    // The current time is used to derive random numbers from the seed where required, to ensure all random generation is unique across restarts.
    // TODO validate that this is right
    let seed: [u8; 32] = platform
        .coin
        .as_ref()
        .priv_key_policy
        .key_pair_or_err()?
        .private()
        .secret
        .into();
    let cur = get_local_duration_since_epoch().map_to_mm(|e| EnableLightningError::SystemTimeError(e.to_string()))?;

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
            let mut manager_file = File::open(persister.manager_path())?;

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
    platform.spawner().spawn(ln_best_block_update_loop(
        platform.clone(),
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

// Todo: Make this public in rust-lightning by opening a PR there instead of importing it here
/// Filters the `channels` for an invoice, and returns the corresponding `RouteHint`s to include
/// in the invoice.
///
/// The filtering is based on the following criteria:
/// * Only one channel per counterparty node
/// * Always select the channel with the highest inbound capacity per counterparty node
/// * Filter out channels with a lower inbound capacity than `min_inbound_capacity_msat`, if any
/// channel with a higher or equal inbound capacity than `min_inbound_capacity_msat` exists
/// * If any public channel exists, the returned `RouteHint`s will be empty, and the sender will
/// need to find the path by looking at the public channels instead
pub(crate) fn filter_channels(channels: Vec<ChannelDetails>, min_inbound_capacity_msat: Option<u64>) -> Vec<RouteHint> {
    let mut filtered_channels: HashMap<PublicKey, &ChannelDetails> = HashMap::new();
    let min_inbound_capacity = min_inbound_capacity_msat.unwrap_or(0);
    let mut min_capacity_channel_exists = false;

    for channel in channels.iter() {
        if channel.get_inbound_payment_scid().is_none() || channel.counterparty.forwarding_info.is_none() {
            continue;
        }

        // Todo: if all public channels have inbound_capacity_msat less than min_inbound_capacity we need to give the user the option to reveal his/her private channels to the swap counterparty in this case or not
        // Todo: the problem with revealing the private channels in the swap message (invoice) is that it can be used by malicious nodes to probe for private channels so maybe there should be a
        // Todo: requirement that the other party has the amount required to be sent in the swap first (do we have a way to check if the other side of the swap has the balance required for the swap on-chain or not)
        if channel.is_public {
            // If any public channel exists, return no hints and let the sender
            // look at the public channels instead.
            return vec![];
        }

        if channel.inbound_capacity_msat >= min_inbound_capacity {
            min_capacity_channel_exists = true;
        };
        match filtered_channels.entry(channel.counterparty.node_id) {
            Entry::Occupied(entry) if channel.inbound_capacity_msat < entry.get().inbound_capacity_msat => continue,
            Entry::Occupied(mut entry) => entry.insert(channel),
            Entry::Vacant(entry) => entry.insert(channel),
        };
    }

    let route_hint_from_channel = |channel: &ChannelDetails| {
        // It's safe to unwrap here since all filtered_channels have forwarding_info
        let forwarding_info = channel.counterparty.forwarding_info.as_ref().unwrap();
        RouteHint(vec![RouteHintHop {
            src_node_id: channel.counterparty.node_id,
            // It's safe to unwrap here since all filtered_channels have inbound_payment_scid
            short_channel_id: channel.get_inbound_payment_scid().unwrap(),
            fees: RoutingFees {
                base_msat: forwarding_info.fee_base_msat,
                proportional_millionths: forwarding_info.fee_proportional_millionths,
            },
            cltv_expiry_delta: forwarding_info.cltv_expiry_delta,
            htlc_minimum_msat: channel.inbound_htlc_minimum_msat,
            htlc_maximum_msat: channel.inbound_htlc_maximum_msat,
        }])
    };
    // If all channels are private, return the route hint for the highest inbound capacity channel
    // per counterparty node. If channels with an higher inbound capacity than the
    // min_inbound_capacity exists, filter out the channels with a lower capacity than that.
    filtered_channels
        .into_iter()
        .filter(|(_counterparty_id, channel)| {
            !min_capacity_channel_exists || channel.inbound_capacity_msat >= min_inbound_capacity
        })
        .map(|(_counterparty_id, channel)| route_hint_from_channel(channel))
        .collect::<Vec<RouteHint>>()
}
