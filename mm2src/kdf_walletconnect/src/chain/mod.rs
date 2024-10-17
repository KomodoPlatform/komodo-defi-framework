pub mod tendermint;
use relay_rpc::rpc::params::session::{ProposeNamespace, ProposeNamespaces};
use std::collections::BTreeMap;

pub(crate) const SUPPORTED_EVENTS: &[&str] = &[];
pub(crate) const SUPPORTED_METHODS: &[&str] = &["cosmos_getAccounts", "cosmos_signDirect", "cosmos_signAmino"];
pub(crate) const SUPPORTED_CHAINS: &[&str] = &["cosmos:cosmoshub-4"];

#[derive(Debug, Clone)]
pub enum WcRequestMethods {
    CosmosSignDirect,
    CosmosSignAmino,
    CosmosGetAccounts,
}

impl AsRef<str> for WcRequestMethods {
    fn as_ref(&self) -> &str {
        match self {
            Self::CosmosSignDirect => "cosmos_signDirect",
            Self::CosmosSignAmino => "cosmos_signAmino",
            Self::CosmosGetAccounts => "cosmos_getAccounts",
        }
    }
}

pub(crate) fn build_required_namespaces() -> ProposeNamespaces {
    let mut required = BTreeMap::new();
    required.insert("cosmos".to_string(), ProposeNamespace {
        chains: SUPPORTED_CHAINS.iter().map(|c| c.to_string()).collect(),
        methods: SUPPORTED_METHODS.iter().map(|m| m.to_string()).collect(),
        events: SUPPORTED_EVENTS.iter().map(|m| m.to_string()).collect(),
    });

    ProposeNamespaces(required)
}
