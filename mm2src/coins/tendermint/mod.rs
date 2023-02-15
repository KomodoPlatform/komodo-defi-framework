// Module implementing Tendermint (Cosmos) integration
// Useful resources
// https://docs.cosmos.network/

mod ibc;
mod iris;
mod rpc;
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

pub(crate) mod type_urls {
    pub(crate) const IBC_TRANSFER_TYPE_URL: &str = "/ibc.applications.transfer.v1.MsgTransfer";

    pub(crate) const CREATE_HTLC_TYPE_URL: &str = "/irismod.htlc.MsgCreateHTLC";
    pub(crate) const CLAIM_HTLC_TYPE_URL: &str = "/irismod.htlc.MsgClaimHTLC";
}
