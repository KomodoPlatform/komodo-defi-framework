use crate::utxo::rpc_clients::{BestBlock as RpcBestBlock, ElectrumBlockHeader, ElectrumClient, ElectrumNonce,
                               UtxoRpcClientOps};
use bitcoin::blockdata::block::BlockHeader;
use bitcoin::blockdata::constants::genesis_block;
use bitcoin::consensus::encode::deserialize;
use bitcoin::hash_types::{BlockHash, TxMerkleNode};
use bitcoin::network::constants::Network;
use bitcoin_hashes::{sha256d, Hash};
use common::executor::{spawn, Timer};
use common::log;
use common::log::LogState;
use common::mm_ctx::MmArc;
use futures::compat::Future01CompatExt;
use lightning::chain::keysinterface::{InMemorySigner, KeysInterface, KeysManager};
use lightning::chain::{chainmonitor, Access, BestBlock, Confirm};
use lightning::ln::channelmanager;
use lightning::ln::channelmanager::{ChainParameters, SimpleArcChannelManager};
use lightning::ln::msgs::NetAddress;
use lightning::ln::peer_handler::{IgnoringMessageHandler, MessageHandler, SimpleArcPeerManager};
use lightning::routing::network_graph::{NetGraphMsgHandler, NetworkGraph};
use lightning::util::config::UserConfig;
use lightning::util::events::Event;
use lightning_background_processor::BackgroundProcessor;
use lightning_net_tokio::SocketDescriptor;
use lightning_persister::FilesystemPersister;
use rand::RngCore;
use std::convert::TryInto;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::net::TcpListener;

const CHECK_FOR_NEW_BEST_BLOCK_INTERVAL: u64 = 60;
const BROADCAST_NODE_ANNOUNCEMENT_INTERVAL: u64 = 60;

type ChainMonitor = chainmonitor::ChainMonitor<
    InMemorySigner,
    Arc<ElectrumClient>,
    Arc<ElectrumClient>,
    Arc<ElectrumClient>,
    Arc<LogState>,
    Arc<FilesystemPersister>,
>;

type ChannelManager = channelmanager::ChannelManager<
    InMemorySigner,
    Arc<ChainMonitor>,
    Arc<ElectrumClient>,
    Arc<KeysManager>,
    Arc<ElectrumClient>,
    Arc<LogState>,
>;

type PeerManager = SimpleArcPeerManager<
    SocketDescriptor,
    ChainMonitor,
    ElectrumClient,
    ElectrumClient,
    dyn Access + Send + Sync,
    LogState,
>;

type SimpleChannelManager = SimpleArcChannelManager<ChainMonitor, ElectrumClient, ElectrumClient, LogState>;

#[derive(Debug)]
pub struct LightningConf {
    /// RPC client (Using only electrum for now as part of the PoC)
    pub rpc_client: ElectrumClient,
    // Mainnet/Testnet/Signet/RegTest
    pub network: Network,
    // The listening port for the p2p LN node
    pub ln_peer_listening_port: u16,
    /// The set (possibly empty) of socket addresses on which this node accepts incoming connections.
    /// If the user wishes to preserve privacy, addresses should likely contain only Tor Onion addresses.
    pub ln_announced_listen_addr: Vec<NetAddress>,
    // Printable human-readable string to describe this node to other users.
    pub ln_announced_node_name: [u8; 32],
}

impl LightningConf {
    // TODO: add network, listen address to fn params
    pub fn new(rpc_client: ElectrumClient, port: u16, node_name: String) -> Self {
        LightningConf {
            rpc_client,
            network: Network::Testnet,
            ln_peer_listening_port: port,
            ln_announced_listen_addr: Vec::new(),
            ln_announced_node_name: node_name.as_bytes().try_into().expect("Node name has incorrect length"),
        }
    }
}

fn my_ln_data_dir(ctx: &MmArc) -> PathBuf { ctx.dbdir().join("LIGHTNING") }

// TODO: Implement all the cases
async fn handle_ln_events(event: &Event) {
    match event {
        Event::FundingGenerationReady { .. } => (),
        Event::PaymentReceived { .. } => (),
        Event::PaymentSent { .. } => (),
        Event::PaymentPathFailed { .. } => (),
        Event::PendingHTLCsForwardable { .. } => (),
        Event::SpendableOutputs { .. } => (),
        Event::PaymentForwarded { .. } => (),
        Event::ChannelClosed { .. } => (),
    }
}

pub async fn start_lightning(ctx: &MmArc, conf: LightningConf) -> Result<(), String> {
    if ctx.ln_background_processor.is_some() {
        return ERR!("Lightning node is already running");
    }
    // Initialize the FeeEstimator. rpc_client implements the FeeEstimator trait, so it'll act as our fee estimator.
    let fee_estimator = Arc::new(conf.rpc_client.clone());

    // Initialize the Logger
    let logger = ctx.log.clone();

    // Initialize the BroadcasterInterface. rpc_client implements the BroadcasterInterface trait, so it'll act as our transaction
    // broadcaster.
    let broadcaster = Arc::new(conf.rpc_client.clone());

    // Initialize Persist
    let ln_data_dir = try_s!(my_ln_data_dir(ctx)
        .as_path()
        .to_str()
        .ok_or("Data dir is a non-UTF-8 string"))
    .to_string();
    let persister = Arc::new(FilesystemPersister::new(ln_data_dir.clone()));

    // Initialize the Filter. rpc_client implements the Filter trait, so it'll act as our filter.
    let filter = Some(Arc::new(conf.rpc_client.clone()));

    // Initialize the ChainMonitor
    let chain_monitor: Arc<ChainMonitor> = Arc::new(chainmonitor::ChainMonitor::new(
        filter.clone(),
        broadcaster.clone(),
        logger.0.clone(),
        fee_estimator.clone(),
        persister.clone(),
    ));

    let seed: [u8; 32] = ctx.secp256k1_key_pair().private().secret.clone().into();

    // The current time is used to derive random numbers from the seed where required, to ensure all random generation is unique across restarts.
    let cur = try_s!(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH));

    // Initialize the KeysManager
    let keys_manager = Arc::new(KeysManager::new(&seed, cur.as_secs(), cur.subsec_nanos()));

    // Read ChannelMonitor state from disk, important for lightning node is restarting and has at least 1 channel
    let channelmonitors = try_s!(persister.read_channelmonitors(keys_manager.clone()));

    // This is used for Electrum only to prepare for chain synchronization
    if let Some(ref filter) = filter {
        for (_, chan_mon) in channelmonitors.iter() {
            chan_mon.load_outputs_to_watch(filter);
        }
    }

    // Initialize the ChannelManager to starting a new node without history
    // TODO: Add the case of restarting a node
    let mut user_config = UserConfig::default();

    // TODO: need more research to find the best case for a node inside mm2
    user_config
        .peer_channel_config_limits
        .force_announced_channel_preference = false;

    let best_block = try_s!(conf.rpc_client.get_best_block().compat().await);
    let chain_params = ChainParameters {
        network: conf.network,
        best_block: BestBlock::new(
            BlockHash::from_hash(try_s!(sha256d::Hash::from_slice(&best_block.hash.0))),
            best_block.height as u32,
        ),
    };
    let new_channel_manager = Arc::new(channelmanager::ChannelManager::new(
        fee_estimator,
        chain_monitor.clone(),
        broadcaster,
        logger.0.clone(),
        keys_manager.clone(),
        user_config,
        chain_params,
    ));

    // Initialize the NetGraphMsgHandler. This is used for providing routes to send payments over
    let genesis = genesis_block(conf.network).header.block_hash();
    let router = Arc::new(NetGraphMsgHandler::new(
        NetworkGraph::new(genesis),
        None::<Arc<dyn Access + Send + Sync>>,
        logger.0.clone(),
    ));

    // Initialize the PeerManager
    // ephemeral_random_data is used to derive per-connection ephemeral keys
    let mut ephemeral_bytes = [0; 32];
    rand::thread_rng().fill_bytes(&mut ephemeral_bytes);
    let lightning_msg_handler = MessageHandler {
        chan_handler: new_channel_manager.clone(),
        route_handler: router.clone(),
    };
    // IgnoringMessageHandler is used as custom message types (experimental and application-specific messages) is not needed
    let peer_manager: Arc<PeerManager> = Arc::new(PeerManager::new(
        lightning_msg_handler,
        keys_manager.get_node_secret(),
        &ephemeral_bytes,
        logger.0.clone(),
        Arc::new(IgnoringMessageHandler {}),
    ));

    // Initialize p2p networking
    let listener = try_s!(TcpListener::bind(format!("0.0.0.0:{}", conf.ln_peer_listening_port)).await);
    spawn(ln_p2p_loop(ctx.clone(), peer_manager.clone(), listener));

    // Update best block whenever there's a new chain tip or a block has been newly disconnected
    spawn(ln_best_block_update_loop(
        ctx.clone(),
        chain_monitor.clone(),
        new_channel_manager.clone(),
        conf.rpc_client.clone(),
        best_block,
    ));

    // Handle LN Events
    // TODO: Check if it's better to do this by implementing EventHandler
    let handle = tokio::runtime::Handle::current();
    let event_handler = move |event: &Event| handle.block_on(handle_ln_events(event));

    // Persist ChannelManager
    // Note: if the ChannelManager is not persisted properly to disk, there is risk of channels force closing the next time LN starts up
    let persist_channel_manager_callback =
        move |node: &SimpleChannelManager| FilesystemPersister::persist_manager(ln_data_dir.clone(), &*node);

    // Start Background Processing. Runs tasks periodically in the background to keep LN node operational
    let background_processor = BackgroundProcessor::start(
        persist_channel_manager_callback,
        event_handler,
        chain_monitor,
        new_channel_manager.clone(),
        Some(router),
        peer_manager,
        logger.0,
    );

    if let Err(e) = ctx.ln_background_processor.pin(background_processor) {
        return ERR!("Lightning node is already running: {}", e);
    };

    // Broadcast Node Announcement
    if !conf.ln_announced_listen_addr.is_empty() {
        spawn(ln_node_announcement_loop(
            ctx.clone(),
            new_channel_manager,
            conf.ln_announced_node_name,
            conf.ln_announced_listen_addr,
        ));
    }

    Ok(())
}

async fn ln_p2p_loop(ctx: MmArc, peer_manager: Arc<PeerManager>, listener: TcpListener) {
    loop {
        if ctx.is_stopping() {
            break;
        }
        let peer_mgr = peer_manager.clone();
        let tcp_stream = match listener.accept().await {
            Ok((stream, addr)) => {
                log::debug!("New incoming lightning connection from peer address: {}", addr);
                stream
            },
            Err(e) => {
                log::error!("Error on accepting lightning connection: {}", e);
                continue;
            },
        };
        if let Ok(stream) = tcp_stream.into_std() {
            spawn(async move {
                lightning_net_tokio::setup_inbound(peer_mgr.clone(), stream).await;
            })
        };
    }
}

async fn ln_best_block_update_loop(
    ctx: MmArc,
    chain_monitor: Arc<ChainMonitor>,
    channel_manager: Arc<ChannelManager>,
    best_header_listener: ElectrumClient,
    best_block: RpcBestBlock,
) {
    let mut current_best_block = best_block;
    loop {
        if ctx.is_stopping() {
            break;
        }
        let best_header = match best_header_listener.blockchain_headers_subscribe().compat().await {
            Ok(h) => h,
            Err(e) => {
                log::error!("Error while requesting best header for lightning node: {}", e);
                Timer::sleep(CHECK_FOR_NEW_BEST_BLOCK_INTERVAL as f64).await;
                continue;
            },
        };
        if current_best_block != best_header.clone().into() {
            current_best_block = best_header.clone().into();
            let (new_best_header, new_best_height) = match best_header {
                ElectrumBlockHeader::V12(h) => {
                    let nonce = match h.nonce {
                        ElectrumNonce::Number(n) => n as u32,
                        ElectrumNonce::Hash(_) => {
                            Timer::sleep(CHECK_FOR_NEW_BEST_BLOCK_INTERVAL as f64).await;
                            continue;
                        },
                    };
                    let prev_blockhash = match sha256d::Hash::from_slice(&h.prev_block_hash.0) {
                        Ok(h) => h,
                        Err(e) => {
                            log::error!("Error while parsing previous block hash for lightning node: {}", e);
                            Timer::sleep(CHECK_FOR_NEW_BEST_BLOCK_INTERVAL as f64).await;
                            continue;
                        },
                    };
                    let merkle_root = match sha256d::Hash::from_slice(&h.merkle_root.0) {
                        Ok(h) => h,
                        Err(e) => {
                            log::error!("Error while parsing merkle root for lightning node: {}", e);
                            Timer::sleep(CHECK_FOR_NEW_BEST_BLOCK_INTERVAL as f64).await;
                            continue;
                        },
                    };
                    (
                        BlockHeader {
                            version: h.version as i32,
                            prev_blockhash: BlockHash::from_hash(prev_blockhash),
                            merkle_root: TxMerkleNode::from_hash(merkle_root),
                            time: h.timestamp as u32,
                            bits: h.bits as u32,
                            nonce,
                        },
                        h.block_height as u32,
                    )
                },
                ElectrumBlockHeader::V14(h) => (
                    deserialize(&h.hex.into_vec()).expect("Can't deserialize block header"),
                    h.height as u32,
                ),
            };
            channel_manager.best_block_updated(&new_best_header, new_best_height);
            chain_monitor.best_block_updated(&new_best_header, new_best_height);
        }
        Timer::sleep(CHECK_FOR_NEW_BEST_BLOCK_INTERVAL as f64).await;
    }
}

async fn ln_node_announcement_loop(
    ctx: MmArc,
    channel_manager: Arc<ChannelManager>,
    node_name: [u8; 32],
    addresses: Vec<NetAddress>,
) {
    loop {
        if ctx.is_stopping() {
            break;
        }
        channel_manager.broadcast_node_announcement(
            [0; 3], // insert node's RGB color. Add to configs later as this is only useful for showing the node in a graph
            node_name,
            addresses.clone(),
        );
        Timer::sleep(BROADCAST_NODE_ANNOUNCEMENT_INTERVAL as f64).await;
    }
}
