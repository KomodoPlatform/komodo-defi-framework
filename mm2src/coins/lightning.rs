use crate::utxo::{rpc_clients::ElectrumClient, BlockchainNetwork};
use common::log::LogArc;
use common::mm_ctx::MmArc;
use lightning::chain::chainmonitor;
use lightning::chain::keysinterface::InMemorySigner;
use lightning::ln::msgs::NetAddress;
use lightning_persister::FilesystemPersister;
use std::path::PathBuf;
use std::sync::Arc;

type ChainMonitor = chainmonitor::ChainMonitor<
    InMemorySigner,
    Arc<ElectrumClient>,
    Arc<ElectrumClient>,
    Arc<ElectrumClient>,
    LogArc,
    Arc<FilesystemPersister>,
>;

#[derive(Debug)]
pub struct LightningConf {
    /// RPC client (Using only electrum for now as part of the PoC)
    pub rpc_client: ElectrumClient,
    // Mainnet/Testnet/RegTest
    pub network: BlockchainNetwork,
    // The listening port for the p2p LN node
    pub ln_peer_listening_port: u16,
    /// The set (possibly empty) of socket addresses on which this node accepts incoming connections.
    /// If the user wishes to preserve privacy, addresses should likely contain only Tor Onion addresses.
    pub ln_announced_listen_addr: Vec<NetAddress>,
    // Printable human-readable string to describe this node to other users.
    pub ln_announced_node_name: [u8; 32],
}

fn my_ln_data_dir(ctx: &MmArc) -> PathBuf { ctx.dbdir().join("LIGHTNING") }

pub async fn start_lightning(ctx: &MmArc, conf: LightningConf) {
    // Initialize the FeeEstimator. rpc_client implements the FeeEstimator trait, so it'll act as our fee estimator.
    let fee_estimator = Arc::new(conf.rpc_client.clone());

    // Initialize the Logger
    let logger = ctx.log.clone();

    // Initialize the BroadcasterInterface. rpc_client implements the BroadcasterInterface trait, so it'll act as our transaction
    // broadcaster.
    let broadcaster = Arc::new(conf.rpc_client.clone());

    // Initialize Persist
    // TODO: Error type for handling this unwarp and others
    let ln_data_dir = my_ln_data_dir(ctx).as_path().to_str().unwrap().to_string();
    let persister = Arc::new(FilesystemPersister::new(ln_data_dir));

    // Initialize the Filter. rpc_client implements the Filter trait, so it'll act as our filter.
    let filter = Some(Arc::new(conf.rpc_client));

    // Initialize the ChainMonitor
    let _chain_monitor: ChainMonitor =
        chainmonitor::ChainMonitor::new(filter, broadcaster, logger, fee_estimator, persister);
}
