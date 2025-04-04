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
        "12D3KooWHKkHiNhZtKceQehHhPqwU5W1jXpoVBgS1qst899GjvTm",
        "viserion.dragon-seed.com",
    ),
    SeedNodeInfo::new(
        "12D3KooWAToxtunEBWCoAHjefSv74Nsmxranw8juy3eKEdrQyGRF",
        "rhaegal.dragon-seed.com",
    ),
    SeedNodeInfo::new(
        "12D3KooWSmEi8ypaVzFA1AGde2RjxNW5Pvxw3qa2fVe48PjNs63R",
        "drogon.dragon-seed.com",
    ),
    SeedNodeInfo::new(
        "12D3KooWMrjLmrv8hNgAoVf1RfumfjyPStzd4nv5XL47zN4ZKisb",
        "falkor.dragon-seed.com",
    ),
    SeedNodeInfo::new(
        "12D3KooWEWzbYcosK2JK9XpFXzumfgsWJW1F7BZS15yLTrhfjX2Z",
        "smaug.dragon-seed.com",
    ),
    SeedNodeInfo::new(
        "12D3KooWJWBnkVsVNjiqUEPjLyHpiSmQVAJ5t6qt1Txv5ctJi9Xd",
        "balerion.dragon-seed.com",
    ),
    SeedNodeInfo::new(
        "12D3KooWPR2RoPi19vQtLugjCdvVmCcGLP2iXAzbDfP3tp81ZL4d",
        "kalessin.dragon-seed.com",
    ),
    SeedNodeInfo::new(
        "12D3KooWJDoV9vJdy6PnzwVETZ3fWGMhV41VhSbocR1h2geFqq9Y",
        "icefyre.dragon-seed.com",
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
