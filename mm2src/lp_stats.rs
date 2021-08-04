/// The module is responsible for mm2 network stats collection
///
use common::executor::{spawn, Timer};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::{log, now_ms, HttpStatusCode};
use derive_more::Display;
use http::StatusCode;
use mm2_libp2p::atomicdex_behaviour::parse_relay_address;
use mm2_libp2p::{encode_message, PeerId};
use serde_json::{self as json, Value as Json};
use std::collections::{HashMap, HashSet};

use crate::mm2::lp_network::{add_reserved_peer_addresses, lp_ports, request_peers, NetIdError, P2PRequest,
                             ParseAddressError, PeerDecodedResponse};

pub type NodeVersionResult<T> = Result<T, MmError<NodeVersionError>>;

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum NodeVersionError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Database error: {}", _0)]
    DatabaseError(String),
    #[display(fmt = "Invalid address: {}", _0)]
    InvalidAddress(String),
    #[display(fmt = "Error on parse peer id {}", _0)]
    PeerIdParseError(String),
    #[display(fmt = "{} is only supported in native mode", _0)]
    UnsupportedMode(String),
}

impl HttpStatusCode for NodeVersionError {
    fn status_code(&self) -> StatusCode {
        match self {
            NodeVersionError::InvalidRequest(_)
            | NodeVersionError::InvalidAddress(_)
            | NodeVersionError::PeerIdParseError(_) => StatusCode::BAD_REQUEST,
            NodeVersionError::UnsupportedMode(_) => StatusCode::METHOD_NOT_ALLOWED,
            NodeVersionError::DatabaseError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<serde_json::Error> for NodeVersionError {
    fn from(e: serde_json::Error) -> Self { NodeVersionError::InvalidRequest(e.to_string()) }
}

impl From<NetIdError> for NodeVersionError {
    fn from(e: NetIdError) -> Self { NodeVersionError::InvalidAddress(e.to_string()) }
}

impl From<ParseAddressError> for NodeVersionError {
    fn from(e: ParseAddressError) -> Self { NodeVersionError::InvalidAddress(e.to_string()) }
}

#[derive(Serialize, Deserialize)]
pub struct NodeInfo {
    pub name: String,
    pub address: String,
    pub peer_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct NodeVersionStat {
    pub name: String,
    pub version: Option<String>,
    pub timestamp: u64,
    pub error: Option<String>,
}

#[cfg(target_arch = "wasm32")]
fn insert_node_info_to_db(_ctx: &MmArc, _node_info: &NodeInfo) -> Result<(), String> { Ok(()) }

#[cfg(not(target_arch = "wasm32"))]
fn insert_node_info_to_db(ctx: &MmArc, node_info: &NodeInfo) -> Result<(), String> {
    crate::mm2::database::stats_nodes::insert_node_info(ctx, node_info).map_err(|e| e.to_string())
}

#[cfg(target_arch = "wasm32")]
fn insert_node_version_stat_to_db(_ctx: &MmArc, _node_version_stat: NodeVersionStat) -> Result<(), String> { Ok(()) }

#[cfg(not(target_arch = "wasm32"))]
fn insert_node_version_stat_to_db(ctx: &MmArc, node_version_stat: NodeVersionStat) -> Result<(), String> {
    crate::mm2::database::stats_nodes::insert_node_version_stat(ctx, node_version_stat).map_err(|e| e.to_string())
}

#[cfg(target_arch = "wasm32")]
fn delete_node_info_from_db(_ctx: &MmArc, _name: String) -> Result<(), String> { Ok(()) }

#[cfg(not(target_arch = "wasm32"))]
fn delete_node_info_from_db(ctx: &MmArc, name: String) -> Result<(), String> {
    crate::mm2::database::stats_nodes::delete_node_info(ctx, name).map_err(|e| e.to_string())
}

#[cfg(target_arch = "wasm32")]
fn select_peers_addresses_from_db(_ctx: &MmArc) -> Result<Vec<(String, String)>, String> { Ok(Vec::new()) }

#[cfg(not(target_arch = "wasm32"))]
fn select_peers_addresses_from_db(ctx: &MmArc) -> Result<Vec<(String, String)>, String> {
    crate::mm2::database::stats_nodes::select_peers_addresses(ctx).map_err(|e| e.to_string())
}

#[cfg(target_arch = "wasm32")]
pub async fn add_node_to_version_stat(_ctx: MmArc, _req: Json) -> NodeVersionResult<String> {
    MmError::err(NodeVersionError::UnsupportedMode("'add_node_to_version_stat'".into()))
}

/// Adds node info. to db to be used later for stats collection
#[cfg(not(target_arch = "wasm32"))]
pub async fn add_node_to_version_stat(ctx: MmArc, req: Json) -> NodeVersionResult<String> {
    use crate::mm2::lp_network::addr_to_ipv4_string;

    let node_info: NodeInfo = json::from_value(req)?;

    let ipv4_addr = addr_to_ipv4_string(&node_info.address)?;
    let node_info_with_ipv4_addr = NodeInfo {
        name: node_info.name,
        address: ipv4_addr,
        peer_id: node_info.peer_id,
    };

    insert_node_info_to_db(&ctx, &node_info_with_ipv4_addr).map_to_mm(NodeVersionError::DatabaseError)?;

    Ok("success".into())
}

#[cfg(target_arch = "wasm32")]
pub async fn remove_node_from_version_stat(_ctx: MmArc, _req: Json) -> NodeVersionResult<String> {
    MmError::err(NodeVersionError::UnsupportedMode(
        "'remove_node_from_version_stat'".into(),
    ))
}

/// Removes node info. from db to skip collecting stats for this node
#[cfg(not(target_arch = "wasm32"))]
pub async fn remove_node_from_version_stat(ctx: MmArc, req: Json) -> NodeVersionResult<String> {
    let node_name: String = json::from_value(req["name"].clone())?;

    delete_node_info_from_db(&ctx, node_name).map_to_mm(NodeVersionError::DatabaseError)?;

    Ok("success".into())
}

#[derive(Debug, Deserialize, Serialize)]
struct Mm2VersionRes {
    nodes: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum NetworkInfoRequest {
    /// Get MM2 version of nodes added to stats collection
    GetMm2Version,
}

async fn process_get_version_request(ctx: MmArc) -> Result<Option<Vec<u8>>, String> {
    let response = ctx.mm_version().to_string();
    let encoded = try_s!(encode_message(&response));
    Ok(Some(encoded))
}

pub async fn process_info_request(ctx: MmArc, request: NetworkInfoRequest) -> Result<Option<Vec<u8>>, String> {
    log::debug!("Got stats request {:?}", request);
    match request {
        NetworkInfoRequest::GetMm2Version => process_get_version_request(ctx).await,
    }
}

#[cfg(target_arch = "wasm32")]
pub async fn start_version_stat_collection(_ctx: MmArc, _req: Json) -> NodeVersionResult<String> {
    MmError::err(NodeVersionError::UnsupportedMode(
        "'start_version_stat_collection'".into(),
    ))
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn start_version_stat_collection(ctx: MmArc, req: Json) -> NodeVersionResult<String> {
    let interval: f64 = json::from_value(req["interval"].clone())?;

    let peers_addresses = select_peers_addresses_from_db(&ctx).map_to_mm(NodeVersionError::DatabaseError)?;

    let netid = ctx.conf["netid"].as_u64().unwrap_or(0) as u16;
    let (_, pubport, _) = lp_ports(netid)?;

    for (peer_id, address) in peers_addresses {
        let peer_id = peer_id
            .parse::<PeerId>()
            .map_to_mm(|e| NodeVersionError::PeerIdParseError(e.to_string()))?;
        let mut addresses = HashSet::new();
        let multi_address = parse_relay_address(address, pubport);
        addresses.insert(multi_address);
        add_reserved_peer_addresses(&ctx, peer_id, addresses);
    }

    spawn(stat_collection_loop(ctx, interval));

    Ok("success".into())
}

#[cfg(not(target_arch = "wasm32"))]
async fn stat_collection_loop(ctx: MmArc, interval: f64) {
    use crate::mm2::database::stats_nodes::select_peers_names;

    loop {
        if ctx.is_stopping() {
            break;
        };
        {
            let peers_names = match select_peers_names(&ctx) {
                Ok(n) => n,
                Err(e) => {
                    log::error!("Error selecting peers names from db: {}", e);
                    Timer::sleep(10.).await;
                    continue;
                },
            };

            let peers: Vec<String> = peers_names.keys().cloned().collect();

            let timestamp = now_ms() / 1000;
            let get_versions_res = match request_peers::<String>(
                ctx.clone(),
                P2PRequest::NetworkInfo(NetworkInfoRequest::GetMm2Version),
                peers,
            )
            .await
            {
                Ok(res) => res,
                Err(e) => {
                    log::error!("Error getting nodes versions from peers: {}", e);
                    Timer::sleep(10.).await;
                    continue;
                },
            };

            for (peer_id, response) in get_versions_res {
                let name = match peers_names.get(&peer_id.to_string()) {
                    Some(n) => n.clone(),
                    None => continue,
                };

                match response {
                    PeerDecodedResponse::Ok(v) => {
                        let node_version_stat = NodeVersionStat {
                            name: name.clone(),
                            version: Some(v.clone()),
                            timestamp,
                            error: None,
                        };
                        if let Err(e) = insert_node_version_stat_to_db(&ctx, node_version_stat) {
                            log::error!("Error inserting node {} version {} into db: {}", name, v, e);
                        };
                    },
                    PeerDecodedResponse::Err(e) => {
                        log::error!(
                            "Node {} responded to version request with error: {}",
                            name.clone(),
                            e.clone()
                        );
                        let node_version_stat = NodeVersionStat {
                            name: name.clone(),
                            version: None,
                            timestamp,
                            error: Some(e.clone()),
                        };
                        if let Err(e) = insert_node_version_stat_to_db(&ctx, node_version_stat) {
                            log::error!("Error inserting node {} error into db: {}", name, e);
                        };
                    },
                    PeerDecodedResponse::None => {
                        log::debug!("Node {} did not respond to version request", name.clone());
                        let node_version_stat = NodeVersionStat {
                            name: name.clone(),
                            version: None,
                            timestamp,
                            error: None,
                        };
                        if let Err(e) = insert_node_version_stat_to_db(&ctx, node_version_stat) {
                            log::error!("Error inserting no response for node {} into db: {}", name, e);
                        };
                    },
                }
            }
        }
        Timer::sleep(interval).await;
    }
}
