// Module implementing Tendermint (Cosmos) integration
// Useful resources
// https://docs.cosmos.network/

mod htlc;
mod ibc;
mod rpc;
mod tendermint_balance_events;
mod tendermint_coin;
mod tendermint_token;
pub mod tendermint_tx_history_v2;

pub use tendermint_coin::*;
pub use tendermint_token::*;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum CustomTendermintMsgType {
    /// Create HTLC as sender
    SendHtlcAmount,
    /// Claim HTLC as reciever
    ClaimHtlcAmount,
    /// Claim HTLC for reciever
    SignClaimHtlc,
}

pub(crate) const TENDERMINT_COIN_PROTOCOL_TYPE: &str = "TENDERMINT";
pub(crate) const TENDERMINT_ASSET_PROTOCOL_TYPE: &str = "TENDERMINTTOKEN";

pub const HTLC_STATE_OPEN: i32 = 0;
pub const HTLC_STATE_COMPLETED: i32 = 1;
pub const HTLC_STATE_REFUNDED: i32 = 2;
