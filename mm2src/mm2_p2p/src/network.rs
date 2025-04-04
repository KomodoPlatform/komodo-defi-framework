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

// TODO: Uncomment these once re-enabled on the main network.
// Operated by Dragonhound, still on NetID 7777. Domains will update after netid migration.
// SeedNodeInfo::new("12D3KooWEsuiKcQaBaKEzuMtT6uFjs89P1E8MK3wGRZbeuCbCw6P", "168.119.236.241", "seed1.komodo.earth"), // tintaglia.dragon-seed.com
// SeedNodeInfo::new("12D3KooWHBeCnJdzNk51G4mLnao9cDsjuqiMTEo5wMFXrd25bd1F", "168.119.236.243", "seed2.komodo.earth"), // mercor.dragon-seed.com
// SeedNodeInfo::new("12D3KooWKxavLCJVrQ5Gk1kd9m6cohctGQBmiKPS9XQFoXEoyGmS", "168.119.236.249", "seed3.komodo.earth"), // karrigvestrit.dragon-seed.com
// SeedNodeInfo::new("12D3KooWGrUpCAbkxhPRioNs64sbUmPmpEcou6hYfrqQvxfWDEuf", "135.181.35.77", "seed4.komodo.earth"), // sintara.dragon-seed.com
// SeedNodeInfo::new("12D3KooWKu8pMTgteWacwFjN7zRWWHb3bctyTvHU3xx5x4x6qDYY", "65.21.56.210", "seed6.komodo.earth"), // heeby.dragon-seed.com
// SeedNodeInfo::new("12D3KooW9soGyPfX6kcyh3uVXNHq1y2dPmQNt2veKgdLXkBiCVKq", "168.119.236.246", "seed7.komodo.earth"), // kalo.dragon-seed.com
// SeedNodeInfo::new("12D3KooWL6yrrNACb7t7RPyTEPxKmq8jtrcbkcNd6H5G2hK7bXaL", "168.119.236.233", "seed8.komodo.earth"),  // relpda.dragon-seed.com
// Operated by Cipi, still on NetID 7777
// SeedNodeInfo::new("12D3KooWAd5gPXwX7eDvKWwkr2FZGfoJceKDCA53SHmTFFVkrN7Q", "46.4.87.18", "fr2.cipig.net"),

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
