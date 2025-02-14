// Module implementing Tendermint (Cosmos) integration
// Useful resources
// https://docs.cosmos.network/

pub(crate) mod ethermint_account;
pub mod htlc;
mod ibc;
mod rpc;
pub mod tendermint_balance_events;
mod tendermint_coin;
mod tendermint_token;
pub mod tendermint_tx_history_v2;
pub mod wallet_connect;

pub use cosmrs::tendermint::PublicKey as TendermintPublicKey;
pub use cosmrs::AccountId;
pub use tendermint_coin::*;
pub use tendermint_token::*;
pub use wallet_connect::*;

pub(crate) const TENDERMINT_COIN_PROTOCOL_TYPE: &str = "TENDERMINT";
pub(crate) const TENDERMINT_ASSET_PROTOCOL_TYPE: &str = "TENDERMINTTOKEN";
