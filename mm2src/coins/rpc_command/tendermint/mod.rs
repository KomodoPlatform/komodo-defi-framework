mod ibc_chains;
mod ibc_transfer_channels;
mod ibc_withdraw;

pub use ibc_chains::*;
pub use ibc_transfer_channels::*;
pub use ibc_withdraw::*;

// Global constants for interacting with https://github.com/KomodoPlatform/chain-registry repository
// using `mm2_git` crate.
pub(crate) const CHAIN_REGISTRY_REPO_OWNER: &str = "KomodoPlatform";
pub(crate) const CHAIN_REGISTRY_REPO_NAME: &str = "chain-registry";
pub(crate) const CHAIN_REGISTRY_BRANCH: &str = "master";
pub(crate) const CHAIN_REGISTRY_IBC_DIR_NAME: &str = "_IBC";
