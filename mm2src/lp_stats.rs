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
use std::net::ToSocketAddrs;

use crate::mm2::lp_network::{add_peer_addresses, request_peers, P2PRequest, PeerDecodedResponse};

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
    crate::mm2::database::stats_nodes::insert_node_info(ctx, node_info).map_err(|e| ERRL!("{}", e))
}

#[cfg(target_arch = "wasm32")]
fn insert_node_version_stat_to_db(_ctx: &MmArc, _node_version_stat: NodeVersionStat) -> Result<(), String> { Ok(()) }

#[cfg(not(target_arch = "wasm32"))]
fn insert_node_version_stat_to_db(ctx: &MmArc, node_version_stat: NodeVersionStat) -> Result<(), String> {
    crate::mm2::database::stats_nodes::insert_node_version_stat(ctx, node_version_stat).map_err(|e| ERRL!("{}", e))
}

#[cfg(target_arch = "wasm32")]
fn delete_node_info_from_db(_ctx: &MmArc, _name: String) -> Result<(), String> { Ok(()) }

#[cfg(not(target_arch = "wasm32"))]
fn delete_node_info_from_db(ctx: &MmArc, name: String) -> Result<(), String> {
    crate::mm2::database::stats_nodes::delete_node_info(ctx, name).map_err(|e| ERRL!("{}", e))
}

#[cfg(target_arch = "wasm32")]
pub async fn add_node_to_version_stat(_ctx: MmArc, _req: Json) -> NodeVersionResult<String> {
    MmError::err(NodeVersionError::UnsupportedMode("'add_node_to_version_stat'".into()))
}

/// Adds node info. to db to be used later for stats collection
#[cfg(not(target_arch = "wasm32"))]
pub async fn add_node_to_version_stat(ctx: MmArc, req: Json) -> NodeVersionResult<String> {
    let node_info: NodeInfo = json::from_value(req).map_to_mm(NodeVersionError::from)?;
    let netid = ctx.conf["netid"].as_u64().unwrap_or(0) as u16;
    let (_, pubport, _) = lp_ports(netid)?;
    let addr = addr_to_ipv4_string(&node_info.address)?;
    let relay_address = parse_relay_address(addr, pubport);

    let mut addresses = HashSet::new();
    addresses.insert(relay_address.clone());

    let peer_id: PeerId = match node_info.peer_id.parse() {
        Ok(p) => p,
        Err(e) => return MmError::err(NodeVersionError::PeerIdParseError(e.to_string())),
    };

    add_peer_addresses(&ctx, peer_id, addresses);

    let node_info_with_formated_addr = NodeInfo {
        name: node_info.name,
        address: relay_address.to_string(),
        peer_id: node_info.peer_id,
    };

    if let Err(e) = insert_node_info_to_db(&ctx, &node_info_with_formated_addr) {
        return MmError::err(NodeVersionError::DatabaseError(e));
    }

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
    let node_name: String = json::from_value(req["name"].clone()).map_to_mm(NodeVersionError::from)?;
    if let Err(e) = delete_node_info_from_db(&ctx, node_name) {
        return MmError::err(NodeVersionError::DatabaseError(e));
    }

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
    let interval: f64 = json::from_value(req["interval"].clone()).map_to_mm(NodeVersionError::from)?;

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

#[derive(Debug, Display)]
pub enum ParseAddressError {
    #[display(fmt = "Address {} resolved to IPv6 which is not supported", _0)]
    UnsupportedIPv6Address(String),
    #[display(fmt = "Address {} to_socket_addrs empty iter", _0)]
    EmptyIterator(String),
    // error return for second string
    #[display(fmt = "Couldn't resolve '{}' Address: {}", _0, _1)]
    UnresolvedAddress(String, String),
}

fn addr_to_ipv4_string(address: &str) -> Result<String, MmError<ParseAddressError>> {
    let address_with_port = if address.contains(':') {
        address.to_string()
    } else {
        format!("{}{}", address, ":0")
    };
    match address_with_port.as_str().to_socket_addrs() {
        Ok(mut iter) => match iter.next() {
            Some(addr) => {
                if addr.is_ipv4() {
                    Ok(addr.ip().to_string())
                } else {
                    MmError::err(ParseAddressError::UnsupportedIPv6Address(address.into()))
                }
            },
            None => MmError::err(ParseAddressError::EmptyIterator(address.into())),
        },
        Err(e) => MmError::err(ParseAddressError::UnresolvedAddress(address.into(), e.to_string())),
    }
}

#[derive(Debug, Display)]
pub enum NetIdError {
    #[display(fmt = "Netid {} is larger than max {}", netid, max_netid)]
    LargerThanMax { netid: u16, max_netid: u16 },
}

pub fn lp_ports(netid: u16) -> Result<(u16, u16, u16), MmError<NetIdError>> {
    const LP_RPCPORT: u16 = 7783;
    let max_netid = (65535 - 40 - LP_RPCPORT) / 4;
    if netid > max_netid {
        return MmError::err(NetIdError::LargerThanMax { netid, max_netid });
    }

    let other_ports = if netid != 0 {
        let net_mod = netid % 10;
        let net_div = netid / 10;
        (net_div * 40) + LP_RPCPORT + net_mod
    } else {
        LP_RPCPORT
    };
    Ok((other_ports + 10, other_ports + 20, other_ports + 30))
}
