use super::*;
use crate::lightning::ln_conf::{LightningCoinConf, LightningProtocolConf};
use crate::lightning::ln_connections::{connect_to_nodes_loop, ln_p2p_loop};
use crate::utxo::rpc_clients::{electrum_script_hash, BestBlock as RpcBestBlock, ElectrumBlockHeader, ElectrumClient,
                               ElectrumNonce, UtxoRpcError};
use crate::utxo::utxo_standard::UtxoStandardCoin;
use crate::DerivationMethod;
use bitcoin::blockdata::block::BlockHeader;
use bitcoin::blockdata::constants::genesis_block;
use bitcoin::blockdata::transaction::Transaction;
use bitcoin::consensus::encode::deserialize;
use bitcoin::hash_types::{BlockHash, TxMerkleNode, Txid};
use bitcoin_hashes::{sha256d, Hash};
use common::executor::{spawn, Timer};
use common::fs::ensure_dir_is_writable;
use common::ip_addr::fetch_external_ip;
use common::jsonrpc_client::JsonRpcErrorType;
use common::log;
use common::log::LogState;
use common::mm_ctx::MmArc;
use futures::compat::Future01CompatExt;
use lightning::chain::keysinterface::{InMemorySigner, KeysInterface, KeysManager};
use lightning::chain::{chainmonitor, Access, BestBlock, Confirm, Watch};
use lightning::ln::channelmanager;
use lightning::ln::channelmanager::{ChainParameters, ChannelManagerReadArgs, SimpleArcChannelManager};
use lightning::ln::msgs::NetAddress;
use lightning::ln::peer_handler::{IgnoringMessageHandler, MessageHandler, SimpleArcPeerManager};
use lightning::routing::network_graph::{NetGraphMsgHandler, NetworkGraph};
use lightning::routing::scoring::Scorer;
use lightning::util::ser::ReadableArgs;
use lightning_background_processor::BackgroundProcessor;
use lightning_invoice::payment;
use lightning_invoice::utils::DefaultRouter;
use lightning_net_tokio::SocketDescriptor;
use lightning_persister::FilesystemPersister;
use parking_lot::Mutex as PaMutex;
use rand::RngCore;
use rpc::v1::types::H256;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::convert::TryInto;
use std::fs::File;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::net::TcpListener;

const CHECK_FOR_NEW_BEST_BLOCK_INTERVAL: u64 = 60;
const BROADCAST_NODE_ANNOUNCEMENT_INTERVAL: u64 = 600;
const NETWORK_GRAPH_PERSIST_INTERVAL: u64 = 600;
const SCORER_PERSIST_INTERVAL: u64 = 600;

pub type ChainMonitor = chainmonitor::ChainMonitor<
    InMemorySigner,
    Arc<PlatformFields>,
    Arc<UtxoStandardCoin>,
    Arc<PlatformFields>,
    Arc<LogState>,
    Arc<FilesystemPersister>,
>;

pub type ChannelManager = SimpleArcChannelManager<ChainMonitor, UtxoStandardCoin, PlatformFields, LogState>;

pub type PeerManager = SimpleArcPeerManager<
    SocketDescriptor,
    ChainMonitor,
    UtxoStandardCoin,
    PlatformFields,
    dyn Access + Send + Sync,
    LogState,
>;

pub type InvoicePayer<E> = payment::InvoicePayer<Arc<ChannelManager>, Router, Arc<Mutex<Scorer>>, Arc<LogState>, E>;

type Router = DefaultRouter<Arc<NetworkGraph>, Arc<LogState>>;

// TODO: add TOR address option
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LightningParams {
    // The listening port for the p2p LN node
    pub listening_port: u16,
    // Printable human-readable string to describe this node to other users.
    pub node_name: [u8; 32],
    // Node's RGB color. This is used for showing the node in a network graph with the desired color.
    pub node_color: [u8; 3],
    // Invoice Payer is initialized while starting the lightning node, and it requires the number of payment retries that
    // it should do before considering a payment failed or partially failed. If not provided the number of retries will be 5
    // as this is a good default value.
    pub payment_retries: Option<usize>,
}

pub async fn start_lightning(
    ctx: &MmArc,
    platform_coin: UtxoStandardCoin,
    protocol_conf: LightningProtocolConf,
    conf: LightningCoinConf,
    params: LightningParams,
) -> EnableLightningResult<LightningCoin> {
    // Todo: add support for Hardware wallets for funding transactions and spending spendable outputs (channel closing transactions)
    if let DerivationMethod::HDWallet(_) = platform_coin.as_ref().derivation_method {
        return MmError::err(EnableLightningError::UnsupportedMode(
            "'start_lightning'".into(),
            "iguana".into(),
        ));
    }

    // The set (possibly empty) of socket addresses on which this node accepts incoming connections.
    // If the user wishes to preserve privacy, addresses should likely contain only Tor Onion addresses.
    let listening_addr = myipaddr(ctx.clone())
        .await
        .map_to_mm(EnableLightningError::InvalidAddress)?;
    // If the listening port is used start_lightning should return an error early
    let listener = TcpListener::bind(format!("{}:{}", listening_addr, params.listening_port))
        .await
        .map_to_mm(|e| EnableLightningError::IOError(e.to_string()))?;

    let network = protocol_conf.network.clone().into();
    let platform_fields = Arc::new(PlatformFields {
        platform_coin: platform_coin.clone(),
        network: protocol_conf.network,
        default_fees_and_confirmations: protocol_conf.confirmations,
        registered_txs: PaMutex::new(HashMap::new()),
        registered_outputs: PaMutex::new(Vec::new()),
        unsigned_funding_txs: PaMutex::new(HashMap::new()),
    });

    // Initialize the FeeEstimator. UtxoStandardCoin implements the FeeEstimator trait, so it'll act as our fee estimator.
    let fee_estimator = platform_fields.clone();

    // Initialize the Logger
    let logger = ctx.log.0.clone();

    // Initialize the BroadcasterInterface. UtxoStandardCoin implements the BroadcasterInterface trait, so it'll act as our transaction
    // broadcaster.
    let broadcaster = Arc::new(platform_coin);

    // Initialize Persist
    let ticker = conf.ticker.clone();
    let my_ln_data_dir = ln_storage::my_ln_data_dir(ctx, &ticker);
    let ln_data_dir = my_ln_data_dir
        .as_path()
        .to_str()
        .ok_or("Data dir is a non-UTF-8 string")
        .map_to_mm(|e| EnableLightningError::InvalidPath(e.into()))?
        .to_string();
    if !ensure_dir_is_writable(&my_ln_data_dir) {
        return MmError::err(EnableLightningError::IOError(format!(
            "{} db dir is not writable",
            ln_data_dir
        )));
    }
    let ln_data_backup_dir = match conf.backup_path.clone() {
        Some(backup_path) => {
            let my_ln_data_backup_dir = ln_storage::my_ln_data_backup_dir(&backup_path, &ticker);
            if !ensure_dir_is_writable(&my_ln_data_backup_dir) {
                return MmError::err(EnableLightningError::IOError(format!(
                    "{} db dir is not writable",
                    backup_path
                )));
            }
            Some(
                my_ln_data_backup_dir
                    .as_path()
                    .to_str()
                    .ok_or("Data dir is a non-UTF-8 string")
                    .map_to_mm(|e| EnableLightningError::InvalidPath(e.into()))?
                    .to_string(),
            )
        },
        None => None,
    };
    let persister = Arc::new(FilesystemPersister::new(
        ln_data_dir.clone(),
        ln_data_backup_dir.clone(),
    ));

    // Initialize the Filter. PlatformFields implements the Filter trait, we can use it to construct the filter.
    let filter = Some(platform_fields.clone());

    // Initialize the ChainMonitor
    let chain_monitor: Arc<ChainMonitor> = Arc::new(chainmonitor::ChainMonitor::new(
        filter.clone(),
        broadcaster.clone(),
        logger.clone(),
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
        let user_config = conf.clone().into();
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
                logger.clone(),
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
    if restarting_node && channel_manager_blockhash != best_block_hash {
        process_txs_unconfirmations(
            filter.clone().unwrap().clone(),
            chain_monitor.clone(),
            channel_manager.clone(),
        )
        .await;
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
    let default_network_graph = NetworkGraph::new(genesis_block(network).header.block_hash());
    let network_graph_path = ln_storage::network_graph_path(ctx, &ticker);
    let network_graph =
        Arc::new(ln_storage::read_network_graph_from_file(&network_graph_path).unwrap_or(default_network_graph));
    let network_gossip = Arc::new(NetGraphMsgHandler::new(
        network_graph.clone(),
        None::<Arc<dyn Access + Send + Sync>>,
        logger.clone(),
    ));
    let network_graph_persist = network_graph.clone();
    spawn(async move {
        loop {
            if let Err(e) = ln_storage::save_network_graph_to_file(&network_graph_path, &network_graph_persist) {
                log::warn!(
                    "Failed to persist network graph error: {}, please check disk space and permissions",
                    e
                );
            }
            Timer::sleep(NETWORK_GRAPH_PERSIST_INTERVAL as f64).await;
        }
    });

    // Initialize the PeerManager
    // ephemeral_random_data is used to derive per-connection ephemeral keys
    let mut ephemeral_bytes = [0; 32];
    rand::thread_rng().fill_bytes(&mut ephemeral_bytes);
    let lightning_msg_handler = MessageHandler {
        chan_handler: channel_manager.clone(),
        route_handler: network_gossip.clone(),
    };
    // IgnoringMessageHandler is used as custom message types (experimental and application-specific messages) is not needed
    let peer_manager: Arc<PeerManager> = Arc::new(PeerManager::new(
        lightning_msg_handler,
        keys_manager.get_node_secret(),
        &ephemeral_bytes,
        logger.clone(),
        Arc::new(IgnoringMessageHandler {}),
    ));

    // Initialize p2p networking
    spawn(ln_p2p_loop(peer_manager.clone(), listener));

    // Update best block whenever there's a new chain tip or a block has been newly disconnected
    spawn(ln_best_block_update_loop(
        // It's safe to use unwrap here for now until implementing Native Client for Lightning
        filter.clone().unwrap(),
        chain_monitor.clone(),
        channel_manager.clone(),
        rpc_client.clone(),
        best_block,
    ));

    let inbound_payments = Arc::new(PaMutex::new(HashMap::new()));
    let outbound_payments = Arc::new(PaMutex::new(HashMap::new()));

    // Initialize the event handler
    let event_handler = Arc::new(ln_events::LightningEventHandler::new(
        // It's safe to use unwrap here for now until implementing Native Client for Lightning
        filter.clone().unwrap(),
        channel_manager.clone(),
        keys_manager.clone(),
        inbound_payments.clone(),
        outbound_payments.clone(),
    ));

    // Initialize routing Scorer
    let scorer_path = ln_storage::scorer_path(ctx, &ticker);
    let scorer = Arc::new(Mutex::new(
        ln_storage::read_scorer_from_file(&scorer_path).unwrap_or_default(),
    ));
    let scorer_persist = scorer.clone();
    spawn(async move {
        loop {
            if let Err(e) = ln_storage::save_scorer_to_file(&scorer_path, &scorer_persist.lock().unwrap()) {
                log::warn!(
                    "Failed to persist scorer error: {}, please check disk space and permissions",
                    e
                );
            }
            Timer::sleep(SCORER_PERSIST_INTERVAL as f64).await;
        }
    });

    // Create InvoicePayer
    let router = DefaultRouter::new(network_graph, logger.clone());
    let invoice_payer = Arc::new(InvoicePayer::new(
        channel_manager.clone(),
        router,
        scorer,
        logger.clone(),
        event_handler,
        payment::RetryAttempts(params.payment_retries.unwrap_or(5)),
    ));

    // Persist ChannelManager
    // Note: if the ChannelManager is not persisted properly to disk, there is risk of channels force closing the next time LN starts up
    let persist_channel_manager_callback = move |node: &ChannelManager| {
        FilesystemPersister::persist_manager(ln_data_dir.clone(), ln_data_backup_dir.clone(), &*node)
    };

    // Start Background Processing. Runs tasks periodically in the background to keep LN node operational.
    // InvoicePayer will act as our event handler as it handles some of the payments related events before
    // delegating it to LightningEventHandler.
    let background_processor = BackgroundProcessor::start(
        persist_channel_manager_callback,
        invoice_payer.clone(),
        chain_monitor.clone(),
        channel_manager.clone(),
        Some(network_gossip),
        peer_manager.clone(),
        logger,
    );

    // If node is restarting read other nodes data from disk and reconnect to channel nodes/peers if possible.
    let mut nodes_addresses_map = HashMap::new();
    if restarting_node {
        let mut nodes_addresses = read_nodes_addresses_from_file(&nodes_data_path(ctx, &ticker))?;
        for (pubkey, node_addr) in nodes_addresses.drain() {
            if channel_manager
                .list_channels()
                .iter()
                .map(|chan| chan.counterparty.node_id)
                .any(|node_id| node_id == pubkey)
            {
                nodes_addresses_map.insert(pubkey, node_addr);
            }
        }
    }
    let nodes_addresses = Arc::new(PaMutex::new(nodes_addresses_map));

    if restarting_node {
        spawn(connect_to_nodes_loop(nodes_addresses.clone(), peer_manager.clone()));
    }

    // Broadcast Node Announcement
    spawn(ln_node_announcement_loop(
        channel_manager.clone(),
        params.node_name,
        params.node_color,
        listening_addr,
        params.listening_port,
    ));

    Ok(LightningCoin {
        platform_fields,
        conf,
        peer_manager,
        background_processor: Arc::new(background_processor),
        channel_manager,
        chain_monitor,
        keys_manager,
        invoice_payer,
        inbound_payments,
        outbound_payments,
        nodes_addresses,
    })
}

struct ConfirmedTransactionInfo {
    txid: Txid,
    header: BlockHeader,
    index: usize,
    transaction: Transaction,
    height: u32,
}

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

async fn process_tx_for_unconfirmation<T>(txid: Txid, filter: Arc<PlatformFields>, monitor: Arc<T>)
where
    T: Confirm,
{
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
                        monitor.transaction_unconfirmed(&txid);
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

async fn process_txs_unconfirmations(
    filter: Arc<PlatformFields>,
    chain_monitor: Arc<ChainMonitor>,
    channel_manager: Arc<ChannelManager>,
) {
    // Retrieve channel manager transaction IDs to check the chain for un-confirmations
    let channel_manager_relevant_txids = channel_manager.get_relevant_txids();
    for txid in channel_manager_relevant_txids {
        process_tx_for_unconfirmation(txid, filter.clone(), channel_manager.clone()).await;
    }

    // Retrieve chain monitor transaction IDs to check the chain for un-confirmations
    let chain_monitor_relevant_txids = chain_monitor.get_relevant_txids();
    for txid in chain_monitor_relevant_txids {
        process_tx_for_unconfirmation(txid, filter.clone(), chain_monitor.clone()).await;
    }
}

async fn get_confirmed_registered_txs(
    filter: Arc<PlatformFields>,
    client: &ElectrumClient,
    current_height: u64,
) -> Vec<ConfirmedTransactionInfo> {
    let registered_txs = filter.registered_txs.lock().clone();
    let mut confirmed_registered_txs = Vec::new();
    for (txid, scripts) in registered_txs {
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
                            if item.tx_hash == rpc_txid.clone() {
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
                                        .blockchain_transaction_get_merkle(rpc_txid.clone(), height)
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
                                    confirmed_registered_txs.push(confirmed_transaction_info);
                                    filter.registered_txs.lock().remove(&txid);
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
    confirmed_registered_txs
}

async fn append_spent_registered_output_txs(
    transactions_to_confirm: &mut Vec<ConfirmedTransactionInfo>,
    filter: Arc<PlatformFields>,
    client: &ElectrumClient,
) {
    let mut outputs_to_remove = Vec::new();
    let registered_outputs = filter.registered_outputs.lock().clone();
    for output in registered_outputs {
        let result = match ln_rpc::find_watched_output_spend_with_header(client, &output).await {
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
    filter
        .registered_outputs
        .lock()
        .retain(|output| !outputs_to_remove.contains(output));
}

async fn process_txs_confirmations(
    filter: Arc<PlatformFields>,
    client: ElectrumClient,
    chain_monitor: Arc<ChainMonitor>,
    channel_manager: Arc<ChannelManager>,
    current_height: u64,
) {
    let mut transactions_to_confirm = get_confirmed_registered_txs(filter.clone(), &client, current_height).await;
    append_spent_registered_output_txs(&mut transactions_to_confirm, filter.clone(), &client).await;

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

async fn get_best_header(best_header_listener: &ElectrumClient) -> EnableLightningResult<ElectrumBlockHeader> {
    best_header_listener
        .blockchain_headers_subscribe()
        .compat()
        .await
        .map_to_mm(|e| EnableLightningError::RpcError(e.to_string()))
}

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

async fn ln_best_block_update_loop(
    filter: Arc<PlatformFields>,
    chain_monitor: Arc<ChainMonitor>,
    channel_manager: Arc<ChannelManager>,
    best_header_listener: ElectrumClient,
    best_block: RpcBestBlock,
) {
    let mut current_best_block = best_block;
    loop {
        let best_header = match get_best_header(&best_header_listener).await {
            Ok(h) => h,
            Err(e) => {
                log::error!("Error while requesting best header for lightning node: {}", e);
                Timer::sleep(CHECK_FOR_NEW_BEST_BLOCK_INTERVAL as f64).await;
                continue;
            },
        };
        if current_best_block != best_header.clone().into() {
            process_txs_unconfirmations(filter.clone(), chain_monitor.clone(), channel_manager.clone()).await;
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

async fn ln_node_announcement_loop(
    channel_manager: Arc<ChannelManager>,
    node_name: [u8; 32],
    node_color: [u8; 3],
    addr: IpAddr,
    port: u16,
) {
    let addresses = netaddress_from_ipaddr(addr, port);
    loop {
        let addresses_to_announce = if addresses.is_empty() {
            // Right now if the node is behind NAT the external ip is fetched on every loop
            // If the node does not announce a public IP, it will not be displayed on the network graph,
            // and other nodes will not be able to open a channel with it. But it can open channels with other nodes.
            match fetch_external_ip().await {
                Ok(ip) => {
                    log::debug!("Fetch real IP successfully: {}:{}", ip, port);
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
