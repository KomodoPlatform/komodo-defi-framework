use relay_rpc::rpc::params::session::{ProposeNamespace, ProposeNamespaces};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const SUPPORTED_CHAINS: &[&str] = &["cosmos:cosmoshub-4", "eip155:1"];
pub(crate) const SUPPORTED_PROTOCOL: &str = "irn";

pub(crate) const COSMOS_SUPPORTED_METHODS: &[&str] = &["cosmos_getAccounts", "cosmos_signDirect", "cosmos_signAmino"];
pub(crate) const COSMOS_SUPPORTED_CHAINS: &[&str] = &["cosmos:cosmoshub-4"];

pub(crate) const ETH_SUPPORTED_METHODS: &[&str] = &["eth_signTransaction", "personal_sign"];
pub(crate) const ETH_SUPPORTED_CHAINS: &[&str] = &["eip155:1"];
pub(crate) const ETH_SUPPORTED_EVENTS: &[&str] = &["accountsChanged", "chainChanged"];

pub(crate) const DEFAULT_CHAIN_ID: &str = "1";

pub enum WcChain {
    Eip155,
    Cosmos,
}

impl AsRef<str> for WcChain {
    fn as_ref(&self) -> &str {
        match self {
            Self::Eip155 => "eip155",
            Self::Cosmos => "cosmos",
        }
    }
}

impl WcChain {
    pub fn to_chain_id(&self, chain_id: &str) -> String { format!("{}:{chain_id}", self.as_ref()) }
}

#[derive(Debug, Clone)]
pub enum WcRequestMethods {
    CosmosSignDirect,
    CosmosSignAmino,
    CosmosGetAccounts,
    EthSignTransaction,
    PersonalSign,
}

impl AsRef<str> for WcRequestMethods {
    fn as_ref(&self) -> &str {
        match self {
            Self::CosmosSignDirect => "cosmos_signDirect",
            Self::CosmosSignAmino => "cosmos_signAmino",
            Self::CosmosGetAccounts => "cosmos_getAccounts",
            Self::EthSignTransaction => "eth_signTransaction",
            Self::PersonalSign => "personal_sign",
        }
    }
}

pub(crate) fn build_default_required_namespaces() -> ProposeNamespaces {
    let required = BTreeMap::from([
        (WcChain::Eip155.as_ref().to_string(), ProposeNamespace {
            events: ETH_SUPPORTED_EVENTS.iter().map(|m| m.to_string()).collect(),
            chains: ETH_SUPPORTED_CHAINS.iter().map(|c| c.to_string()).collect(),
            methods: ETH_SUPPORTED_METHODS.iter().map(|m| m.to_string()).collect(),
        }),
        (WcChain::Cosmos.as_ref().to_string(), ProposeNamespace {
            methods: COSMOS_SUPPORTED_METHODS.iter().map(|m| m.to_string()).collect(),
            chains: COSMOS_SUPPORTED_CHAINS.iter().map(|c| c.to_string()).collect(),
            events: BTreeSet::default(),
        }),
    ]);

    ProposeNamespaces(required)
}
