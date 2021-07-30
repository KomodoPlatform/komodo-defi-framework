/// The module is responsible for mm2 network stats collection
///
use common::executor::{spawn, Timer};
use common::mm_ctx::MmArc;
use common::{log, now_ms};
use http::Response;
use mm2_libp2p::atomicdex_behaviour::parse_relay_address;
use mm2_libp2p::encode_message;
use serde_json::{self as json, Value as Json};
use std::collections::HashMap;
use std::net::ToSocketAddrs;

use crate::mm2::lp_network::{request_addresses, P2PRequest, PeerDecodedResponse};

#[derive(Serialize, Deserialize)]
pub struct NodeInfo {
    pub name: String,
    pub address: String,
    pub peer_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct NodeVersionStat {
    pub name: String,
    pub version: String,
    pub timestamp: u64,
}

#[cfg(target_arch = "wasm32")]
fn insert_node_info_to_db(_ctx: &MmArc, _node_info: &NodeInfo) -> Result<(), String> { Ok(()) }

#[cfg(not(target_arch = "wasm32"))]
fn insert_node_info_to_db(ctx: &MmArc, node_info: &NodeInfo) -> Result<(), String> {
    crate::mm2::database::stats_nodes::insert_node_info(ctx, node_info).map_err(|e| ERRL!("{}", e))
}

#[cfg(target_arch = "wasm32")]
fn insert_node_version_stat_to_db(_ctx: &MmArc, _node_version_stat: &NodeVersionStat) -> Result<(), String> { Ok(()) }

#[cfg(not(target_arch = "wasm32"))]
fn insert_node_version_stat_to_db(ctx: &MmArc, node_version_stat: &NodeVersionStat) -> Result<(), String> {
    crate::mm2::database::stats_nodes::insert_node_version_stat(ctx, node_version_stat).map_err(|e| ERRL!("{}", e))
}

#[cfg(not(target_arch = "wasm32"))]
fn delete_node_info_from_db(ctx: &MmArc, name: String) -> Result<(), String> {
    crate::mm2::database::stats_nodes::delete_node_info(ctx, name).map_err(|e| ERRL!("{}", e))
}

#[cfg(target_arch = "wasm32")]
fn delete_node_info_from_db(_ctx: &MmArc, _name: String) -> Result<(), String> { Ok(()) }

#[cfg(target_arch = "wasm32")]
pub async fn add_node_to_version_stat(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {}

/// Adds node info. to db to be used later for stats collection
#[cfg(not(target_arch = "wasm32"))]
pub async fn add_node_to_version_stat(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let node_info: NodeInfo = try_s!(json::from_value(req));
    let netid = ctx.conf["netid"].as_u64().unwrap_or(0) as u16;
    let (_, pubport, _) = try_s!(lp_ports(netid));
    let addr = try_s!(addr_to_ipv4_string(&node_info.address));
    let relay_address = parse_relay_address(addr, pubport);

    let node_info_with_formated_addr = NodeInfo {
        name: node_info.name,
        address: relay_address.to_string(),
        peer_id: node_info.peer_id,
    };

    if let Err(e) = insert_node_info_to_db(&ctx, &node_info_with_formated_addr) {
        return ERR!("Error {} on node insertion", e);
    }
    let res = json!({
        "result": "success"
    });

    return Response::builder()
        .body(json::to_vec(&res).expect("Serialization failed"))
        .map_err(|e| ERRL!("{}", e));
}

#[cfg(target_arch = "wasm32")]
pub async fn add_node_to_version_stat(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {}

/// Removes node info. from db to skip collecting stats for this node
#[cfg(not(target_arch = "wasm32"))]
pub async fn remove_node_from_version_stat(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let node_name: String = try_s!(json::from_value(req["name"].clone()));
    if let Err(e) = delete_node_info_from_db(&ctx, node_name) {
        return ERR!("Error {} on node deletion", e);
    }
    let res = json!({
        "result": "success"
    });

    return Response::builder()
        .body(json::to_vec(&res).expect("Serialization failed"))
        .map_err(|e| ERRL!("{}", e));
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
pub async fn start_version_stat_collection(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {}

#[cfg(not(target_arch = "wasm32"))]
pub async fn start_version_stat_collection(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let interval: f64 = try_s!(json::from_value(req["interval"].clone()));

    spawn(stat_collection_loop(ctx, interval));

    let res = json!({
        "result": "success"
    });
    Response::builder()
        .body(json::to_vec(&res).expect("Serialization failed"))
        .map_err(|e| ERRL!("{}", e))
}

async fn stat_collection_loop(ctx: MmArc, interval: f64) {
    use crate::mm2::database::stats_nodes::{select_peers_addresses, select_peers_names};

    loop {
        if ctx.is_stopping() {
            break;
        };
        {
            let peers_addresses = match select_peers_addresses(&ctx) {
                Ok(p) => p,
                Err(e) => {
                    log::error!("Error selecting peers addresses from db: {}", e);
                    Timer::sleep(10.).await;
                    continue;
                },
            };

            let peers_names = match select_peers_names(&ctx) {
                Ok(n) => n,
                Err(e) => {
                    log::error!("Error selecting peers names from db: {}", e);
                    Timer::sleep(10.).await;
                    continue;
                },
            };

            let timestamp = now_ms() / 1000;
            let get_versions_res = match request_addresses::<String>(
                ctx.clone(),
                P2PRequest::NetworkInfo(NetworkInfoRequest::GetMm2Version),
                peers_addresses,
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
                    PeerDecodedResponse::Ok(version) => {
                        let node_version_stat = NodeVersionStat {
                            name,
                            version,
                            timestamp,
                        };
                        if let Err(e) = insert_node_version_stat_to_db(&ctx, &node_version_stat) {
                            log::error!("Error inserting nodes versions into db: {}", e);
                            continue;
                        };
                    },
                    // If a node returns an error or no response it will not be added to the stats table
                    // A simple count for every node in db will return a count for the number of responses recieved
                    PeerDecodedResponse::Err(e) => {
                        log::error!("Node {} responded to version request with error: {}", name, e);
                        continue;
                    },
                    PeerDecodedResponse::None => {
                        log::debug!("Node {} did not respond to version request", name);
                        continue;
                    },
                }
            }
        }
        Timer::sleep(interval).await;
    }
}

fn addr_to_ipv4_string(address: &str) -> Result<String, String> {
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
                    ERR!("Address {} resolved to IPv6 {} which is not supported", address, addr)
                }
            },
            None => {
                ERR!("Address {} to_socket_addrs empty iter", address)
            },
        },
        Err(e) => {
            ERR!("Couldn't resolve '{}' Address: {}", address, e)
        },
    }
}

pub fn lp_ports(netid: u16) -> Result<(u16, u16, u16), String> {
    const LP_RPCPORT: u16 = 7783;
    let max_netid = (65535 - 40 - LP_RPCPORT) / 4;
    if netid > max_netid {
        return ERR!("Netid {} is larger than max {}", netid, max_netid);
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
