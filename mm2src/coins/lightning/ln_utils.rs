use super::*;
use crate::utxo::utxo_standard::UtxoStandardCoin;
use bitcoin::network::constants::Network;
use common::mm_ctx::MmArc;
use derive_more::Display;
use secp256k1::PublicKey;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::str::FromStr;

cfg_native! {
    use crate::utxo::rpc_clients::{electrum_script_hash, BestBlock as RpcBestBlock, ElectrumBlockHeader, ElectrumClient,
                                   ElectrumNonce, UtxoRpcError};
    use bitcoin::blockdata::block::BlockHeader;
    use bitcoin::blockdata::constants::genesis_block;
    use bitcoin::blockdata::script::Script;
    use bitcoin::blockdata::transaction::{Transaction, TxOut};
    use bitcoin::consensus::encode::deserialize;
    use bitcoin::hash_types::{BlockHash, TxMerkleNode, Txid};
    use bitcoin_hashes::{sha256d, Hash};
    use common::executor::{spawn, Timer};
    use common::ip_addr::fetch_external_ip;
    use common::jsonrpc_client::JsonRpcErrorType;
    use common::{block_on, log};
    use common::log::LogState;
    use futures::compat::Future01CompatExt;
    use futures::lock::Mutex as AsyncMutex;
    use lightning::chain::keysinterface::{InMemorySigner, KeysInterface, KeysManager};
    use lightning::chain::transaction::OutPoint;
    use lightning::chain::{chainmonitor, Access, BestBlock, Confirm, Filter, Watch, WatchedOutput};
    use lightning::ln::channelmanager;
    use lightning::ln::channelmanager::{ChainParameters, ChannelManagerReadArgs, SimpleArcChannelManager};
    use lightning::ln::msgs::NetAddress;
    use lightning::ln::peer_handler::{IgnoringMessageHandler, MessageHandler, SimpleArcPeerManager};
    use lightning::routing::network_graph::{NetGraphMsgHandler, NetworkGraph};
    use lightning::util::config::UserConfig;
    use lightning::util::events::{Event, EventHandler};
    use lightning::util::ser::ReadableArgs;
    use lightning_background_processor::BackgroundProcessor;
    use lightning_net_tokio::SocketDescriptor;
    use lightning_persister::FilesystemPersister;
    use rand::RngCore;
    use rpc::v1::types::H256;
    use script::{Builder, SignatureVersion};
    use std::cmp::Ordering;
    use std::convert::{TryFrom, TryInto};
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;
    use std::time::SystemTime;
    use tokio::net::TcpListener;
    use utxo_signer::with_key_pair::sign_tx;
}

cfg_native! {
    const CHECK_FOR_NEW_BEST_BLOCK_INTERVAL: u64 = 60;
    const BROADCAST_NODE_ANNOUNCEMENT_INTERVAL: u64 = 60;
    const TRY_RECONNECTING_TO_NODE_INTERVAL: u64 = 60;
}

#[cfg(not(target_arch = "wasm32"))]
type ChainMonitor = chainmonitor::ChainMonitor<
    InMemorySigner,
    Arc<PlatformFields>,
    Arc<UtxoStandardCoin>,
    Arc<UtxoStandardCoin>,
    Arc<LogState>,
    Arc<FilesystemPersister>,
>;

#[cfg(not(target_arch = "wasm32"))]
pub type ChannelManager = SimpleArcChannelManager<ChainMonitor, UtxoStandardCoin, UtxoStandardCoin, LogState>;

#[cfg(not(target_arch = "wasm32"))]
pub type PeerManager = SimpleArcPeerManager<
    SocketDescriptor,
    ChainMonitor,
    UtxoStandardCoin,
    UtxoStandardCoin,
    dyn Access + Send + Sync,
    LogState,
>;

// TODO: add TOR address option
#[cfg(not(target_arch = "wasm32"))]
fn netaddress_from_ipaddr(addr: IpAddr, port: u16) -> Vec<NetAddress> {
    if addr == Ipv4Addr::new(0, 0, 0, 0) || addr == Ipv4Addr::new(127, 0, 0, 1) {
        return Vec::new();
    }
    let mut addresses = Vec::new();
    let address = match addr {
        IpAddr::V4(addr) => NetAddress::IPv4 {
            addr: u32::from(addr).to_be_bytes(),
            port,
        },
        IpAddr::V6(addr) => NetAddress::IPv6 {
            addr: u128::from(addr).to_be_bytes(),
            port,
        },
    };
    addresses.push(address);
    addresses
}

fn my_ln_data_dir(ctx: &MmArc, ticker: &str) -> PathBuf { ctx.dbdir().join("LIGHTNING").join(ticker) }

pub fn nodes_data_path(ctx: &MmArc, ticker: &str) -> PathBuf { my_ln_data_dir(ctx, ticker).join("channel_nodes_data") }

pub fn last_request_id_path(ctx: &MmArc, ticker: &str) -> PathBuf {
    my_ln_data_dir(ctx, ticker).join("LAST_REQUEST_ID")
}

#[cfg(not(target_arch = "wasm32"))]
struct LightningEventHandler {
    filter: Arc<PlatformFields>,
    channel_manager: Arc<ChannelManager>,
}

#[cfg(not(target_arch = "wasm32"))]
impl LightningEventHandler {
    fn new(filter: Arc<PlatformFields>, channel_manager: Arc<ChannelManager>) -> Self {
        LightningEventHandler {
            filter,
            channel_manager,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl EventHandler for LightningEventHandler {
    // TODO: Implement all the cases
    fn handle_event(&self, event: &Event) {
        match event {
            Event::FundingGenerationReady {
                temporary_channel_id,
                channel_value_satoshis,
                output_script,
                user_channel_id,
            } => {
                let funding_tx = match block_on(sign_funding_transaction(
                    user_channel_id,
                    output_script.clone(),
                    self.filter.clone(),
                )) {
                    Ok(tx) => tx,
                    Err(e) => {
                        log::error!(
                            "Error generating funding transaction for temporary channel id {:?}: {}",
                            temporary_channel_id,
                            e.to_string()
                        );
                        // TODO: use issue_channel_close_events here when implementing channel closure this will push a Event::DiscardFunding
                        // event for the other peer
                        return;
                    },
                };
                // Give the funding transaction back to LDK for opening the channel.
                match self
                    .channel_manager
                    .funding_transaction_generated(temporary_channel_id, funding_tx.clone())
                {
                    Ok(_) => {
                        let txid = funding_tx.txid();
                        self.filter.register_tx(&txid, output_script);
                        let output_to_be_registered = TxOut {
                            value: *channel_value_satoshis,
                            script_pubkey: output_script.clone(),
                        };
                        let output_index = match funding_tx
                            .output
                            .iter()
                            .position(|tx_out| tx_out == &output_to_be_registered)
                        {
                            Some(i) => i,
                            None => {
                                log::error!(
                                    "Output to register is not found in the output of the transaction: {}",
                                    txid
                                );
                                return;
                            },
                        };
                        self.filter.register_output(WatchedOutput {
                            block_hash: None,
                            outpoint: OutPoint {
                                txid,
                                index: output_index as u16,
                            },
                            script_pubkey: output_script.clone(),
                        });
                    },
                    // When transaction is unconfirmed by process_txs_confirmations LDK will try to rebroadcast the tx
                    Err(e) => log::error!("{:?}", e),
                }
            },
            Event::PaymentReceived { .. } => (),
            Event::PaymentSent { .. } => (),
            Event::PaymentPathFailed { .. } => (),
            Event::PendingHTLCsForwardable { .. } => (),
            Event::SpendableOutputs { .. } => (),
            Event::PaymentForwarded { .. } => (),
            Event::ChannelClosed { .. } => (),
            Event::DiscardFunding { .. } => (),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LightningParams {
    // The listening port for the p2p LN node
    pub listening_port: u16,
    // Printable human-readable string to describe this node to other users.
    pub node_name: [u8; 32],
    // Node's RGB color. This is used for showing the node in a network graph with the desired color.
    pub node_color: [u8; 3],
}

#[cfg(target_arch = "wasm32")]
pub async fn start_lightning(
    _ctx: &MmArc,
    _platform_coin: UtxoStandardCoin,
    _ticker: String,
    _params: LightningParams,
    _network: Network,
) -> EnableLightningResult<LightningCoin> {
    MmError::err(EnableLightningError::UnsupportedMode(
        "'connect_to_lightning_node'".into(),
        "native".into(),
    ))
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn start_lightning(
    ctx: &MmArc,
    platform_coin: UtxoStandardCoin,
    ticker: String,
    params: LightningParams,
    network: Network,
) -> EnableLightningResult<LightningCoin> {
    // The set (possibly empty) of socket addresses on which this node accepts incoming connections.
    // If the user wishes to preserve privacy, addresses should likely contain only Tor Onion addresses.
    let listening_addr = myipaddr(ctx.clone())
        .await
        .map_to_mm(EnableLightningError::InvalidAddress)?;
    // If the listening port is used start_lightning should return an error early
    let listener = TcpListener::bind(format!("{}:{}", listening_addr, params.listening_port))
        .await
        .map_to_mm(|e| EnableLightningError::IOError(e.to_string()))?;

    // Initialize the FeeEstimator. UtxoStandardCoin implements the FeeEstimator trait, so it'll act as our fee estimator.
    let fee_estimator = Arc::new(platform_coin.clone());

    // Initialize the Logger
    let logger = ctx.log.clone();

    // Initialize the BroadcasterInterface. UtxoStandardCoin implements the BroadcasterInterface trait, so it'll act as our transaction
    // broadcaster.
    let broadcaster = Arc::new(platform_coin.clone());

    // Initialize Persist
    let ln_data_dir = my_ln_data_dir(ctx, &ticker)
        .as_path()
        .to_str()
        .ok_or("Data dir is a non-UTF-8 string")
        .map_to_mm(|e| EnableLightningError::InvalidPath(e.into()))?
        .to_string();
    let persister = Arc::new(FilesystemPersister::new(ln_data_dir.clone()));

    let platform_fields = Arc::new(PlatformFields {
        platform_coin,
        registered_txs: AsyncMutex::new(HashMap::new()),
        registered_outputs: AsyncMutex::new(Vec::new()),
        unsigned_funding_txs: AsyncMutex::new(HashMap::new()),
    });
    // Initialize the Filter. PlatformFields implements the Filter trait, we can use it to construct the filter.
    let filter = Some(platform_fields.clone());

    // Initialize the ChainMonitor
    let chain_monitor: Arc<ChainMonitor> = Arc::new(chainmonitor::ChainMonitor::new(
        filter.clone(),
        broadcaster.clone(),
        logger.0.clone(),
        fee_estimator.clone(),
        persister.clone(),
    ));

    let seed: [u8; 32] = ctx.secp256k1_key_pair().private().secret.into();

    // The current time is used to derive random numbers from the seed where required, to ensure all random generation is unique across restarts.
    let cur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_to_mm(|e| EnableLightningError::SystemTimeError(e.to_string()))?;

    // Initialize the KeysManager
    let keys_manager = Arc::new(KeysManager::new(&seed, cur.as_secs(), cur.subsec_nanos()));

    // Read ChannelMonitor state from disk, important for lightning node is restarting and has at least 1 channel
    let mut channelmonitors = persister
        .read_channelmonitors(keys_manager.clone())
        .map_to_mm(|e| EnableLightningError::IOError(e.to_string()))?;

    // This is used for Electrum only to prepare for chain synchronization
    if let Some(ref filter) = filter {
        for (_, chan_mon) in channelmonitors.iter() {
            chan_mon.load_outputs_to_watch(filter);
        }
    }

    let mut user_config = UserConfig::default();
    // When set to false an incoming channel doesn't have to match our announced channel preference which allows public channels
    // TODO: Add user config to LightningCoinConf maybe get it from coin config / also add to lightning context
    user_config
        .peer_channel_config_limits
        .force_announced_channel_preference = false;

    let mut restarting_node = true;
    // TODO: Right now it's safe to unwrap here, when implementing Native client for lightning whenever filter is used
    // the code it's used in will be a part of the electrum client implementation only
    let rpc_client = match &filter.clone().unwrap().platform_coin.as_ref().rpc_client {
        UtxoRpcClientEnum::Electrum(c) => c.clone(),
        UtxoRpcClientEnum::Native(_) => {
            return MmError::err(EnableLightningError::UnsupportedMode(
                "Lightning network".into(),
                "electrum".into(),
            ))
        },
    };
    let best_header = get_best_header(&rpc_client).await?;
    let best_block = RpcBestBlock::from(best_header.clone());
    let best_block_hash = BlockHash::from_hash(
        sha256d::Hash::from_slice(&best_block.hash.0).map_to_mm(|e| EnableLightningError::HashError(e.to_string()))?,
    );
    let (channel_manager_blockhash, channel_manager) = {
        if let Ok(mut f) = File::open(format!("{}/manager", ln_data_dir.clone())) {
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
                logger.0.clone(),
                user_config,
                channel_monitor_mut_references,
            );
            <(BlockHash, ChannelManager)>::read(&mut f, read_args)
                .map_to_mm(|e| EnableLightningError::IOError(e.to_string()))?
        } else {
            // Initialize the ChannelManager to starting a new node without history
            restarting_node = false;
            let chain_params = ChainParameters {
                network,
                best_block: BestBlock::new(best_block_hash, best_block.height as u32),
            };
            let new_channel_manager = channelmanager::ChannelManager::new(
                fee_estimator.clone(),
                chain_monitor.clone(),
                broadcaster.clone(),
                logger.0.clone(),
                keys_manager.clone(),
                user_config,
                chain_params,
            );
            (best_block_hash, new_channel_manager)
        }
    };

    let channel_manager: Arc<ChannelManager> = Arc::new(channel_manager);

    // Sync ChannelMonitors and ChannelManager to chain tip if the node is restarting and has open channels
    if restarting_node && channel_manager_blockhash != best_block_hash {
        process_txs_confirmations(
            // It's safe to use unwrap here for now until implementing Native Client for Lightning
            filter.clone().unwrap().clone(),
            rpc_client.clone(),
            chain_monitor.clone(),
            channel_manager.clone(),
            best_header.block_height(),
        )
        .await;
        update_best_block(chain_monitor.clone(), channel_manager.clone(), best_header).await;
    }

    // Give ChannelMonitors to ChainMonitor
    for (_, channel_monitor) in channelmonitors.drain(..) {
        let funding_outpoint = channel_monitor.get_funding_txo().0;
        chain_monitor
            .watch_channel(funding_outpoint, channel_monitor)
            .map_to_mm(|e| EnableLightningError::IOError(format!("{:?}", e)))?;
    }

    // Initialize the NetGraphMsgHandler. This is used for providing routes to send payments over
    let genesis = genesis_block(network).header.block_hash();
    let router = Arc::new(NetGraphMsgHandler::new(
        Arc::new(NetworkGraph::new(genesis)),
        None::<Arc<dyn Access + Send + Sync>>,
        logger.0.clone(),
    ));

    // Initialize the PeerManager
    // ephemeral_random_data is used to derive per-connection ephemeral keys
    let mut ephemeral_bytes = [0; 32];
    rand::thread_rng().fill_bytes(&mut ephemeral_bytes);
    let lightning_msg_handler = MessageHandler {
        chan_handler: channel_manager.clone(),
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
    spawn(ln_p2p_loop(ctx.clone(), peer_manager.clone(), listener));

    // Update best block whenever there's a new chain tip or a block has been newly disconnected
    spawn(ln_best_block_update_loop(
        ctx.clone(),
        // It's safe to use unwrap here for now until implementing Native Client for Lightning
        filter.clone().unwrap(),
        chain_monitor.clone(),
        channel_manager.clone(),
        rpc_client.clone(),
        best_block,
    ));

    // Persist ChannelManager
    // Note: if the ChannelManager is not persisted properly to disk, there is risk of channels force closing the next time LN starts up
    // TODO: for some reason the persister doesn't persist the current best block when best_block_updated is called although it does
    // persist the channel_manager which should have the current best block in it, when other operations that requires persisting occurs
    // The current best block get persisted
    let persist_channel_manager_callback =
        move |node: &ChannelManager| FilesystemPersister::persist_manager(ln_data_dir.clone(), &*node);

    // Start Background Processing. Runs tasks periodically in the background to keep LN node operational
    let background_processor = BackgroundProcessor::start(
        persist_channel_manager_callback,
        // It's safe to use unwrap here for now until implementing Native Client for Lightning
        LightningEventHandler::new(filter.clone().unwrap(), channel_manager.clone()),
        chain_monitor,
        channel_manager.clone(),
        Some(router),
        peer_manager.clone(),
        logger.0,
    );

    // If node is restarting read other nodes data from disk and reconnect to channel nodes/peers if possible.
    if restarting_node {
        let mut nodes_data = read_nodes_data_from_file(&nodes_data_path(ctx, &ticker))?;
        for (pubkey, node_addr) in nodes_data.drain() {
            for chan_info in channel_manager.list_channels() {
                if pubkey == chan_info.counterparty.node_id {
                    spawn(connect_to_node_loop(
                        ctx.clone(),
                        pubkey,
                        node_addr,
                        peer_manager.clone(),
                    ));
                }
            }
        }
    }

    // Broadcast Node Announcement
    spawn(ln_node_announcement_loop(
        ctx.clone(),
        channel_manager.clone(),
        params.node_name,
        params.node_color,
        listening_addr,
        params.listening_port,
    ));

    Ok(LightningCoin {
        platform_fields,
        conf: Arc::new(LightningCoinConf { ticker }),
        peer_manager,
        background_processor: Arc::new(background_processor),
        channel_manager,
    })
}

#[cfg(not(target_arch = "wasm32"))]
async fn ln_p2p_loop(ctx: MmArc, peer_manager: Arc<PeerManager>, listener: TcpListener) {
    loop {
        if ctx.is_stopping() {
            break;
        }
        let peer_mgr = peer_manager.clone();
        let tcp_stream = match listener.accept().await {
            Ok((stream, addr)) => {
                log::debug!("New incoming lightning connection from node address: {}", addr);
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

#[cfg(not(target_arch = "wasm32"))]
struct ConfirmedTransactionInfo {
    txid: Txid,
    header: BlockHeader,
    index: usize,
    transaction: Transaction,
    height: u32,
}

#[cfg(not(target_arch = "wasm32"))]
impl ConfirmedTransactionInfo {
    fn new(txid: Txid, header: BlockHeader, index: usize, transaction: Transaction, height: u32) -> Self {
        ConfirmedTransactionInfo {
            txid,
            header,
            index,
            transaction,
            height,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn process_tx_for_unconfirmation(txid: Txid, filter: Arc<PlatformFields>, channel_manager: Arc<ChannelManager>) {
    if let Err(err) = filter
        .platform_coin
        .as_ref()
        .rpc_client
        .get_transaction_bytes(&H256::from(txid.as_hash().into_inner()).reversed())
        .compat()
        .await
        .map_err(|e| e.into_inner())
    {
        if let UtxoRpcError::ResponseParseError(ref json_err) = err {
            if let JsonRpcErrorType::Response(_, json) = &json_err.error {
                if let Some(message) = json["message"].as_str() {
                    if message.contains("'code': -5") {
                        log::info!(
                            "Transaction {} is not found on chain :{}. The transaction will be re-broadcasted.",
                            txid,
                            err
                        );
                        channel_manager.transaction_unconfirmed(&txid);
                    }
                }
            }
        }
        log::error!(
            "Error while trying to check if the transaction {} is discarded or not :{}",
            txid,
            err
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn process_txs_confirmations(
    filter: Arc<PlatformFields>,
    client: ElectrumClient,
    chain_monitor: Arc<ChainMonitor>,
    channel_manager: Arc<ChannelManager>,
    current_height: u64,
) {
    // Retrieve transaction IDs to check the chain for un-confirmations
    let channel_manager_relevant_txids = channel_manager.get_relevant_txids();
    let chain_monitor_relevant_txids = chain_monitor.get_relevant_txids();

    for txid in channel_manager_relevant_txids {
        process_tx_for_unconfirmation(txid, filter.clone(), channel_manager.clone()).await;
    }

    for txid in chain_monitor_relevant_txids {
        process_tx_for_unconfirmation(txid, filter.clone(), channel_manager.clone()).await;
    }

    let mut registered_txs = filter.registered_txs.lock().await;
    let mut transactions_to_confirm = Vec::new();
    for (txid, scripts) in registered_txs.clone() {
        let rpc_txid = H256::from(txid.as_hash().into_inner()).reversed();
        match filter
            .platform_coin
            .as_ref()
            .rpc_client
            .get_transaction_bytes(&rpc_txid)
            .compat()
            .await
        {
            Ok(bytes) => {
                let transaction: Transaction = match deserialize(&bytes.into_vec()) {
                    Ok(tx) => tx,
                    Err(e) => {
                        log::error!("Transaction deserialization error: {}", e.to_string());
                        continue;
                    },
                };
                for (_, vout) in transaction.output.iter().enumerate() {
                    if scripts.contains(&vout.script_pubkey) {
                        let script_hash = hex::encode(electrum_script_hash(vout.script_pubkey.as_ref()));
                        let history = client
                            .scripthash_get_history(&script_hash)
                            .compat()
                            .await
                            .unwrap_or_default();
                        for item in history {
                            if item.tx_hash == rpc_txid {
                                // If a new block mined the transaction while running process_txs_confirmations it will be confirmed later in ln_best_block_update_loop
                                if item.height > 0 && item.height <= current_height as i64 {
                                    let height: u64 = match item.height.try_into() {
                                        Ok(h) => h,
                                        Err(e) => {
                                            log::error!("Block height convertion to u64 error: {}", e.to_string());
                                            continue;
                                        },
                                    };
                                    let header = match client.blockchain_block_header(height).compat().await {
                                        Ok(block_header) => match deserialize(&block_header) {
                                            Ok(h) => h,
                                            Err(e) => {
                                                log::error!("Block header deserialization error: {}", e.to_string());
                                                continue;
                                            },
                                        },
                                        Err(_) => continue,
                                    };
                                    let index = match client
                                        .blockchain_transaction_get_merkle(rpc_txid, height)
                                        .compat()
                                        .await
                                    {
                                        Ok(merkle_branch) => merkle_branch.pos,
                                        Err(e) => {
                                            log::error!(
                                                "Error getting transaction position in the block: {}",
                                                e.to_string()
                                            );
                                            continue;
                                        },
                                    };
                                    let confirmed_transaction_info = ConfirmedTransactionInfo::new(
                                        txid,
                                        header,
                                        index,
                                        transaction.clone(),
                                        height as u32,
                                    );
                                    transactions_to_confirm.push(confirmed_transaction_info);
                                    registered_txs.remove(&txid);
                                }
                            }
                        }
                    }
                }
            },
            Err(e) => {
                log::error!("Error getting transaction {} from chain: {}", txid, e);
                continue;
            },
        };
    }
    drop(registered_txs);

    let mut outputs_to_remove = Vec::new();
    let mut registered_outputs = filter.registered_outputs.lock().await;
    for output in registered_outputs.clone() {
        let result = match ln_rpc::find_watched_output_spend_with_header(&client, &output).await {
            Ok(res) => res,
            Err(e) => {
                log::error!(
                    "Error while trying to find if the registered output {:?} is spent: {}",
                    output.outpoint,
                    e
                );
                continue;
            },
        };
        if let Some((header, _, tx, height)) = result {
            if !transactions_to_confirm.iter().any(|info| info.txid == tx.txid()) {
                let rpc_txid = H256::from(tx.txid().as_hash().into_inner()).reversed();
                let index = match client
                    .blockchain_transaction_get_merkle(rpc_txid, height)
                    .compat()
                    .await
                {
                    Ok(merkle_branch) => merkle_branch.pos,
                    Err(e) => {
                        log::error!("Error getting transaction position in the block: {}", e.to_string());
                        continue;
                    },
                };
                let confirmed_transaction_info =
                    ConfirmedTransactionInfo::new(tx.txid(), header, index, tx, height as u32);
                transactions_to_confirm.push(confirmed_transaction_info);
            }
            outputs_to_remove.push(output);
        }
    }
    registered_outputs.retain(|output| !outputs_to_remove.contains(output));
    drop(registered_outputs);

    transactions_to_confirm.sort_by(|a, b| {
        let block_order = a.height.cmp(&b.height);
        match block_order {
            Ordering::Equal => a.index.cmp(&b.index),
            _ => block_order,
        }
    });

    for confirmed_transaction_info in transactions_to_confirm {
        channel_manager.transactions_confirmed(
            &confirmed_transaction_info.header,
            &[(
                confirmed_transaction_info.index,
                &confirmed_transaction_info.transaction,
            )],
            confirmed_transaction_info.height,
        );
        chain_monitor.transactions_confirmed(
            &confirmed_transaction_info.header,
            &[(
                confirmed_transaction_info.index,
                &confirmed_transaction_info.transaction,
            )],
            confirmed_transaction_info.height,
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn get_best_header(best_header_listener: &ElectrumClient) -> EnableLightningResult<ElectrumBlockHeader> {
    best_header_listener
        .blockchain_headers_subscribe()
        .compat()
        .await
        .map_to_mm(|e| EnableLightningError::RpcError(e.to_string()))
}

#[cfg(not(target_arch = "wasm32"))]
async fn update_best_block(
    chain_monitor: Arc<ChainMonitor>,
    channel_manager: Arc<ChannelManager>,
    best_header: ElectrumBlockHeader,
) {
    {
        let (new_best_header, new_best_height) = match best_header {
            ElectrumBlockHeader::V12(h) => {
                let nonce = match h.nonce {
                    ElectrumNonce::Number(n) => n as u32,
                    ElectrumNonce::Hash(_) => {
                        return;
                    },
                };
                let prev_blockhash = match sha256d::Hash::from_slice(&h.prev_block_hash.0) {
                    Ok(h) => h,
                    Err(e) => {
                        log::error!("Error while parsing previous block hash for lightning node: {}", e);
                        return;
                    },
                };
                let merkle_root = match sha256d::Hash::from_slice(&h.merkle_root.0) {
                    Ok(h) => h,
                    Err(e) => {
                        log::error!("Error while parsing merkle root for lightning node: {}", e);
                        return;
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
            ElectrumBlockHeader::V14(h) => {
                let block_header = match deserialize(&h.hex.into_vec()) {
                    Ok(header) => header,
                    Err(e) => {
                        log::error!("Block header deserialization error: {}", e.to_string());
                        return;
                    },
                };
                (block_header, h.height as u32)
            },
        };
        channel_manager.best_block_updated(&new_best_header, new_best_height);
        chain_monitor.best_block_updated(&new_best_header, new_best_height);
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn ln_best_block_update_loop(
    ctx: MmArc,
    filter: Arc<PlatformFields>,
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
        let best_header = match get_best_header(&best_header_listener).await {
            Ok(h) => h,
            Err(e) => {
                log::error!("Error while requesting best header for lightning node: {}", e);
                Timer::sleep(CHECK_FOR_NEW_BEST_BLOCK_INTERVAL as f64).await;
                continue;
            },
        };
        if current_best_block != best_header.clone().into() {
            process_txs_confirmations(
                filter.clone(),
                best_header_listener.clone(),
                chain_monitor.clone(),
                channel_manager.clone(),
                best_header.block_height(),
            )
            .await;
            current_best_block = best_header.clone().into();
            update_best_block(chain_monitor.clone(), channel_manager.clone(), best_header).await;
        }
        Timer::sleep(CHECK_FOR_NEW_BEST_BLOCK_INTERVAL as f64).await;
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn ln_node_announcement_loop(
    ctx: MmArc,
    channel_manager: Arc<ChannelManager>,
    node_name: [u8; 32],
    node_color: [u8; 3],
    addr: IpAddr,
    port: u16,
) {
    let addresses = netaddress_from_ipaddr(addr, port);
    loop {
        if ctx.is_stopping() {
            break;
        }

        let addresses_to_announce = if addresses.is_empty() {
            // Right now if the node is behind NAT the external ip is fetched on every loop
            // If the node does not announce a public IP, it will not be displayed on the network graph,
            // and other nodes will not be able to open a channel with it. But it can open channels with other nodes.
            // TODO: Fetch external ip on reconnection only
            match fetch_external_ip().await {
                Ok(ip) => {
                    log::info!("Fetch real IP successfully: {}:{}", ip, port);
                    netaddress_from_ipaddr(ip, port)
                },
                Err(e) => {
                    log::error!("Error while fetching external ip for node announcement: {}", e);
                    Timer::sleep(BROADCAST_NODE_ANNOUNCEMENT_INTERVAL as f64).await;
                    continue;
                },
            }
        } else {
            addresses.clone()
        };

        channel_manager.broadcast_node_announcement(node_color, node_name, addresses_to_announce);

        Timer::sleep(BROADCAST_NODE_ANNOUNCEMENT_INTERVAL as f64).await;
    }
}

fn pubkey_and_addr_from_str(pubkey_str: &str, addr_str: &str) -> ConnectToNodeResult<(PublicKey, SocketAddr)> {
    // TODO: support connection to onion addresses
    let addr = addr_str
        .to_socket_addrs()
        .map(|mut r| r.next())
        .map_to_mm(|e| ConnectToNodeError::ParseError(e.to_string()))?
        .ok_or_else(|| ConnectToNodeError::ParseError(format!("Couldn't parse {} into a socket address", addr_str)))?;

    let pubkey = PublicKey::from_str(pubkey_str).map_to_mm(|e| ConnectToNodeError::ParseError(e.to_string()))?;

    Ok((pubkey, addr))
}

pub fn parse_node_info(node_pubkey_and_ip_addr: String) -> ConnectToNodeResult<(PublicKey, SocketAddr)> {
    let mut pubkey_and_addr = node_pubkey_and_ip_addr.split('@');

    let pubkey = pubkey_and_addr.next().ok_or_else(|| {
        ConnectToNodeError::ParseError(format!(
            "Incorrect node id format for {}. The format should be `pubkey@host:port`",
            node_pubkey_and_ip_addr
        ))
    })?;

    let node_addr_str = pubkey_and_addr.next().ok_or_else(|| {
        ConnectToNodeError::ParseError(format!(
            "Incorrect node id format for {}. The format should be `pubkey@host:port`",
            node_pubkey_and_ip_addr
        ))
    })?;

    let (pubkey, node_addr) = pubkey_and_addr_from_str(pubkey, node_addr_str)?;
    Ok((pubkey, node_addr))
}

pub fn read_nodes_data_from_file(path: &Path) -> ConnectToNodeResult<HashMap<PublicKey, SocketAddr>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let mut nodes_data = HashMap::new();
    let file = File::open(path).map_to_mm(|e| ConnectToNodeError::IOError(e.to_string()))?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line.map_to_mm(|e| ConnectToNodeError::IOError(e.to_string()))?;
        let (pubkey, socket_addr) = parse_node_info(line)?;
        nodes_data.insert(pubkey, socket_addr);
    }
    Ok(nodes_data)
}

pub fn save_node_data_to_file(path: &Path, node_info: &str) -> ConnectToNodeResult<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_to_mm(|e| ConnectToNodeError::IOError(e.to_string()))?;
    file.write_all(format!("{}\n", node_info).as_bytes())
        .map_to_mm(|e| ConnectToNodeError::IOError(e.to_string()))
}

#[derive(Display)]
pub enum ConnectToNodeRes {
    #[display(fmt = "Already connected to node: {}@{}", _0, _1)]
    AlreadyConnected(String, String),
    #[display(fmt = "Connected successfully to node : {}@{}", _0, _1)]
    ConnectedSuccessfully(String, String),
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn connect_to_node(
    pubkey: PublicKey,
    node_addr: SocketAddr,
    peer_manager: Arc<PeerManager>,
) -> ConnectToNodeResult<ConnectToNodeRes> {
    for node_pubkey in peer_manager.get_peer_node_ids() {
        if node_pubkey == pubkey {
            return Ok(ConnectToNodeRes::AlreadyConnected(
                node_pubkey.to_string(),
                node_addr.to_string(),
            ));
        }
    }

    match lightning_net_tokio::connect_outbound(Arc::clone(&peer_manager), pubkey, node_addr).await {
        Some(connection_closed_future) => {
            let mut connection_closed_future = Box::pin(connection_closed_future);
            loop {
                // Make sure the connection is still established.
                match futures::poll!(&mut connection_closed_future) {
                    std::task::Poll::Ready(_) => {
                        return MmError::err(ConnectToNodeError::ConnectionError(format!(
                            "Node {} disconnected before finishing the handshake",
                            pubkey
                        )));
                    },
                    std::task::Poll::Pending => {},
                }
                // Wait for the handshake to complete.
                match peer_manager.get_peer_node_ids().iter().find(|id| **id == pubkey) {
                    Some(_) => break,
                    None => Timer::sleep_ms(10).await,
                }
            }
        },
        None => {
            return MmError::err(ConnectToNodeError::ConnectionError(format!(
                "Failed to connect to node: {}",
                pubkey
            )))
        },
    }

    Ok(ConnectToNodeRes::ConnectedSuccessfully(
        pubkey.to_string(),
        node_addr.to_string(),
    ))
}

#[cfg(not(target_arch = "wasm32"))]
async fn connect_to_node_loop(ctx: MmArc, pubkey: PublicKey, node_addr: SocketAddr, peer_manager: Arc<PeerManager>) {
    for node_pubkey in peer_manager.get_peer_node_ids() {
        if node_pubkey == pubkey {
            log::info!("Already connected to node: {}", node_pubkey);
            return;
        }
    }

    loop {
        if ctx.is_stopping() {
            break;
        }

        match connect_to_node(pubkey, node_addr, peer_manager.clone()).await {
            Ok(res) => {
                log::info!("{}", res.to_string());
                break;
            },
            Err(e) => log::error!("{}", e.to_string()),
        }

        Timer::sleep(TRY_RECONNECTING_TO_NODE_INTERVAL as f64).await;
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn open_ln_channel(
    node_pubkey: PublicKey,
    amount_in_sat: u64,
    events_id: u64,
    announce_channel: bool,
    channel_manager: Arc<ChannelManager>,
) -> OpenChannelResult<[u8; 32]> {
    // TODO: get user_config from context when it's added to it
    let mut user_config = UserConfig::default();
    user_config
        .peer_channel_config_limits
        .force_announced_channel_preference = false;
    user_config.channel_options.announced_channel = announce_channel;

    // TODO: push_msat parameter
    channel_manager
        .create_channel(node_pubkey, amount_in_sat, 0, events_id, Some(user_config))
        .map_to_mm(|e| OpenChannelError::FailureToOpenChannel(node_pubkey.to_string(), format!("{:?}", e)))
}

// Generates the raw funding transaction with one output equal to the channel value.
#[cfg(not(target_arch = "wasm32"))]
async fn sign_funding_transaction(
    request_id: &u64,
    output_script: Script,
    filter: Arc<PlatformFields>,
) -> OpenChannelResult<Transaction> {
    let coin = &filter.platform_coin;
    let mut unsigned = {
        let unsigned_funding_txs = filter.unsigned_funding_txs.lock().await;
        unsigned_funding_txs
            .get(request_id)
            .ok_or_else(|| {
                OpenChannelError::InternalError(format!("Unsigned funding tx not found for request id: {}", request_id))
            })?
            .clone()
    };
    unsigned.outputs[0].script_pubkey = output_script.to_bytes().into();

    let my_address = coin.as_ref().derivation_method.iguana_or_err()?;
    let key_pair = coin.as_ref().priv_key_policy.key_pair_or_err()?;

    let prev_script = Builder::build_p2pkh(&my_address.hash);
    let signed = sign_tx(
        unsigned,
        key_pair,
        prev_script,
        SignatureVersion::WitnessV0,
        coin.as_ref().conf.fork_id,
    )?;

    Transaction::try_from(signed).map_to_mm(|e| OpenChannelError::ConvertTxErr(e.to_string()))
}

pub fn save_last_request_id_to_file(path: &Path, last_request_id: u64) -> OpenChannelResult<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(path)
        .map_to_mm(|e| OpenChannelError::IOError(e.to_string()))?;
    file.write_all(format!("{}", last_request_id).as_bytes())
        .map_to_mm(|e| OpenChannelError::IOError(e.to_string()))
}

pub fn read_last_request_id_from_file(path: &Path) -> OpenChannelResult<u64> {
    if !path.exists() {
        return MmError::err(OpenChannelError::InvalidPath(format!(
            "Path {} does not exist",
            path.display()
        )));
    }
    let mut file = File::open(path).map_to_mm(|e| OpenChannelError::IOError(e.to_string()))?;
    let mut contents = String::new();
    let _ = file.read_to_string(&mut contents);
    contents
        .parse::<u64>()
        .map_to_mm(|e| OpenChannelError::IOError(e.to_string()))
}
