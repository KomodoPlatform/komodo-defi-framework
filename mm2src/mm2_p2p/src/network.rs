use crate::relay_address::RelayAddress;
use libp2p::PeerId;

pub const DEFAULT_NETID: u16 = 8762;

pub struct SeedNodeInfo {
    pub id: &'static str,
    pub domain: &'static str,
}

impl SeedNodeInfo {
    pub const fn new(id: &'static str, domain: &'static str) -> Self { Self { id, domain } }
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const ALL_DEFAULT_NETID_SEEDNODES: &[SeedNodeInfo] = &[
    SeedNodeInfo::new(
        "12D3KooWEaZpH61H4yuQkaNG5AsyGdpBhKRppaLdAY52a774ab5u",
        "seed01.kmdefi.net",
    ),
    SeedNodeInfo::new(
        "12D3KooWAd5gPXwX7eDvKWwkr2FZGfoJceKDCA53SHmTFFVkrN7Q",
        "seed02.kmdefi.net",
    ),
];

#[cfg(target_arch = "wasm32")]
pub fn get_all_network_seednodes(_netid: u16) -> Vec<(PeerId, RelayAddress, String)> { Vec::new() }

#[cfg(not(target_arch = "wasm32"))]
pub fn get_all_network_seednodes(netid: u16) -> Vec<(PeerId, RelayAddress, String)> {
    use std::str::FromStr;

    if netid != DEFAULT_NETID {
        return Vec::new();
    }
    ALL_DEFAULT_NETID_SEEDNODES
        .iter()
        .map(|SeedNodeInfo { id, domain }| {
            let peer_id = PeerId::from_str(id).unwrap_or_else(|e| panic!("Valid peer id {id}: {e}"));
            let ip =
                mm2_net::ip_addr::addr_to_ipv4_string(domain).unwrap_or_else(|e| panic!("Valid domain {domain}: {e}"));
            let address = RelayAddress::IPv4(ip);
            let domain = domain.to_string();
            (peer_id, address, domain)
        })
        .collect()
}
