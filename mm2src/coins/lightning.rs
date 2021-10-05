use crate::utxo::rpc_clients::{ElectrumBlockHeader, ElectrumClient, ElectrumNonce, UtxoRpcClientOps};
use bitcoin::blockdata::block::BlockHeader;
use bitcoin::blockdata::constants::genesis_block;
use bitcoin::consensus::encode::deserialize;
use bitcoin::hash_types::{BlockHash, TxMerkleNode};
use bitcoin::network::constants::Network;
use bitcoin_hashes::{sha256d, Hash};
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
use std::time::{Duration, SystemTime};

type ChainMonitor = chainmonitor::ChainMonitor<
    InMemorySigner,
    Arc<ElectrumClient>,
    Arc<ElectrumClient>,
    Arc<ElectrumClient>,
    Arc<LogState>,
    Arc<FilesystemPersister>,
>;

type PeerManager = SimpleArcPeerManager<
    SocketDescriptor,
    ChainMonitor,
    ElectrumClient,
    ElectrumClient,
    dyn Access + Send + Sync,
    LogState,
>;

type ChannelManager = SimpleArcChannelManager<ChainMonitor, ElectrumClient, ElectrumClient, LogState>;

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

pub async fn start_lightning(ctx: MmArc, conf: LightningConf) {
    // Initialize the FeeEstimator. rpc_client implements the FeeEstimator trait, so it'll act as our fee estimator.
    let fee_estimator = Arc::new(conf.rpc_client.clone());

    // Initialize the Logger
    let logger = ctx.log.clone();

    // Initialize the BroadcasterInterface. rpc_client implements the BroadcasterInterface trait, so it'll act as our transaction
    // broadcaster.
    let broadcaster = Arc::new(conf.rpc_client.clone());

    // Initialize Persist
    // TODO: Error type for handling this unwarp and others
    let ln_data_dir = my_ln_data_dir(&ctx).as_path().to_str().unwrap().to_string();
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
    let cur = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();

    // Initialize the KeysManager
    let keys_manager = Arc::new(KeysManager::new(&seed, cur.as_secs(), cur.subsec_nanos()));

    // Read ChannelMonitor state from disk, important for lightning node is restarting and has at least 1 channel
    // TODO: Error handling instead of unwrap()
    let channelmonitors = persister.read_channelmonitors(keys_manager.clone()).unwrap();

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

    // TODO: Error handling instead of unwrap()
    let best_block = conf.rpc_client.get_best_block().compat().await.unwrap();
    let chain_params = ChainParameters {
        network: conf.network,
        best_block: BestBlock::new(
            BlockHash::from_hash(sha256d::Hash::from_slice(&best_block.hash.0).unwrap()),
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

    // Initialize networking
    let peer_manager_connection_handler = peer_manager.clone();
    let listening_port = conf.ln_peer_listening_port;
    tokio::spawn(async move {
        // TODO: Error handling
        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", listening_port))
            .await
            .unwrap();
        loop {
            let peer_mgr = peer_manager_connection_handler.clone();
            let tcp_stream = listener.accept().await.unwrap().0;
            tokio::spawn(async move {
                lightning_net_tokio::setup_inbound(peer_mgr.clone(), tcp_stream.into_std().unwrap()).await;
            });
        }
    });

    // Update best block whenever there's a new chain tip or a block has been newly disconnected
    // TODO: Error handling
    let channel_manager_listener = new_channel_manager.clone();
    let chain_monitor_listener = chain_monitor.clone();
    let best_header_listener = conf.rpc_client.clone();
    tokio::spawn(async move {
        loop {
            let best_header = best_header_listener
                .blockchain_headers_subscribe()
                .compat()
                .await
                .unwrap();
            if best_block != best_header.clone().into() {
                let (new_best_header, new_best_height) = match best_header {
                    ElectrumBlockHeader::V12(h) => {
                        let nonce = match h.nonce {
                            ElectrumNonce::Number(n) => n as u32,
                            ElectrumNonce::Hash(_) => {
                                tokio::time::sleep(Duration::from_secs(60)).await;
                                continue;
                            },
                        };
                        (
                            BlockHeader {
                                version: h.version as i32,
                                prev_blockhash: BlockHash::from_hash(
                                    sha256d::Hash::from_slice(&h.prev_block_hash.0).unwrap(),
                                ),
                                merkle_root: TxMerkleNode::from_hash(
                                    sha256d::Hash::from_slice(&h.merkle_root.0).unwrap(),
                                ),
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
                channel_manager_listener.best_block_updated(&new_best_header, new_best_height);
                chain_monitor_listener.best_block_updated(&new_best_header, new_best_height);
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });

    // Handle LN Events
    // TODO: Check if it's better to do this by implementing EventHandler
    let handle = tokio::runtime::Handle::current();
    let event_handler = move |event: &Event| handle.block_on(handle_ln_events(event));

    // Persist ChannelManager
    // Note: if the ChannelManager is not persisted properly to disk, there is risk of channels force closing the next time LN starts up
    let persist_channel_manager_callback =
        move |node: &ChannelManager| FilesystemPersister::persist_manager(ln_data_dir.clone(), &*node);

    // Start Background Processing. Runs tasks periodically in the background to keep LN node operational
    let _background_processor = BackgroundProcessor::start(
        persist_channel_manager_callback,
        event_handler,
        chain_monitor,
        new_channel_manager.clone(),
        Some(router),
        peer_manager,
        logger.0,
    );

    // Broadcast Node Announcement
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
        new_channel_manager.broadcast_node_announcement(
            [0; 3], // insert node's RGB color. Add to configs later as this is only useful for showing the node in a graph
            conf.ln_announced_node_name,
            conf.ln_announced_listen_addr.clone(),
        );
    }
}
