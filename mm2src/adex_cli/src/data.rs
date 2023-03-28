use serde::Serialize;
use std::net::Ipv4Addr;

#[derive(Serialize)]
pub struct Mm2Cfg {
    pub gui: Option<String>,
    pub net_id: Option<u16>,
    pub rpc_password: Option<String>,
    pub passphrase: Option<String>,
    pub allow_weak_password: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dbdir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpcip: Option<Ipv4Addr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpcport: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpc_local_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i_am_seed: Option<bool>,
    #[serde(skip_serializing_if = "Vec::<Ipv4Addr>::is_empty")]
    pub seednodes: Vec<Ipv4Addr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hd_account_id: Option<u64>,
}
