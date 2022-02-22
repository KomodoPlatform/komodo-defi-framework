use super::*;
use common::mm_ctx::MmArc;
use lightning::routing::network_graph::NetworkGraph;
use lightning::routing::scoring::Scorer;
use lightning::util::ser::{Readable, Writeable};
use secp256k1::PublicKey;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub fn my_ln_data_dir(ctx: &MmArc, ticker: &str) -> PathBuf { ctx.dbdir().join("LIGHTNING").join(ticker) }

pub fn my_ln_data_backup_dir(path: &str, ticker: &str) -> PathBuf { PathBuf::from(path).join("LIGHTNING").join(ticker) }

pub fn nodes_data_path(ctx: &MmArc, ticker: &str) -> PathBuf { my_ln_data_dir(ctx, ticker).join("channel_nodes_data") }

pub fn nodes_data_backup_path(path: &str, ticker: &str) -> PathBuf {
    my_ln_data_backup_dir(path, ticker).join("channel_nodes_data")
}

pub fn network_graph_path(ctx: &MmArc, ticker: &str) -> PathBuf { my_ln_data_dir(ctx, ticker).join("network_graph") }

pub fn scorer_path(ctx: &MmArc, ticker: &str) -> PathBuf { my_ln_data_dir(ctx, ticker).join("scorer") }

pub fn read_nodes_addresses_from_file(path: &Path) -> ConnectToNodeResult<HashMap<PublicKey, SocketAddr>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let file = File::open(path).map_to_mm(|e| ConnectToNodeError::IOError(e.to_string()))?;
    let reader = BufReader::new(file);
    let nodes_addresses: HashMap<String, SocketAddr> =
        serde_json::from_reader(reader).map_to_mm(|e| ConnectToNodeError::IOError(e.to_string()))?;
    nodes_addresses
        .iter()
        .map(|(pubkey_str, addr)| {
            let pubkey =
                PublicKey::from_str(pubkey_str).map_to_mm(|e| ConnectToNodeError::ParseError(e.to_string()))?;
            Ok((pubkey, *addr))
        })
        .collect()
}

pub fn write_nodes_addresses_to_file(
    path: &Path,
    nodes_addresses: HashMap<PublicKey, SocketAddr>,
) -> ConnectToNodeResult<()> {
    let nodes_addresses: HashMap<String, SocketAddr> = nodes_addresses
        .iter()
        .map(|(pubkey, addr)| (pubkey.to_string(), *addr))
        .collect();
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_to_mm(|e| ConnectToNodeError::IOError(e.to_string()))?;
    serde_json::to_writer(file, &nodes_addresses).map_to_mm(|e| ConnectToNodeError::IOError(e.to_string()))
}

pub fn save_network_graph_to_file(path: &Path, network_graph: &NetworkGraph) -> EnableLightningResult<()> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(path)
        .map_to_mm(|e| EnableLightningError::IOError(e.to_string()))?;
    network_graph
        .write(&mut BufWriter::new(file))
        .map_to_mm(|e| EnableLightningError::IOError(e.to_string()))
}

pub fn read_network_graph_from_file(path: &Path) -> EnableLightningResult<NetworkGraph> {
    let file = File::open(path).map_to_mm(|e| EnableLightningError::IOError(e.to_string()))?;
    log::info!("Reading the saved lightning network graph from file, this can take some time!");
    NetworkGraph::read(&mut BufReader::new(file)).map_to_mm(|e| EnableLightningError::IOError(e.to_string()))
}

pub fn save_scorer_to_file(path: &Path, scorer: &Scorer) -> EnableLightningResult<()> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(path)
        .map_to_mm(|e| EnableLightningError::IOError(e.to_string()))?;
    scorer
        .write(&mut BufWriter::new(file))
        .map_to_mm(|e| EnableLightningError::IOError(e.to_string()))
}

pub fn read_scorer_from_file(path: &Path) -> EnableLightningResult<Scorer> {
    let file = File::open(path).map_to_mm(|e| EnableLightningError::IOError(e.to_string()))?;
    Scorer::read(&mut BufReader::new(file)).map_to_mm(|e| EnableLightningError::IOError(e.to_string()))
}
