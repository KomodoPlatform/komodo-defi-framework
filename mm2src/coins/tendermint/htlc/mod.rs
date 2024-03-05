mod iris;
mod nucleus;

use iris::htlc::{IrisClaimHtlcMsg, IrisCreateHtlcMsg};
use nucleus::htlc::{NucleusClaimHtlcMsg, NucleusCreateHtlcMsg};

use iris::htlc_proto::{IrisClaimHtlcProto, IrisCreateHtlcProto, IrisQueryHtlcResponseProto};
use nucleus::htlc_proto::{NucleusClaimHtlcProto, NucleusCreateHtlcProto, NucleusQueryHtlcResponseProto};

pub(crate) const HTLC_STATE_OPEN: i32 = 0;
pub(crate) const HTLC_STATE_COMPLETED: i32 = 1;
pub(crate) const HTLC_STATE_REFUNDED: i32 = 2;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum CustomTendermintMsgType {
    /// Create HTLC as sender
    SendHtlcAmount,
    /// Claim HTLC as reciever
    ClaimHtlcAmount,
    /// Claim HTLC for reciever
    SignClaimHtlc,
}

#[derive(prost::Enumeration, Debug)]
#[repr(i32)]
pub enum HtlcState {
    Open = 0,
    Completed = 1,
    Refunded = 2,
}

#[allow(dead_code)]
pub(crate) struct TendermintHtlc {
    /// Generated HTLC's ID.
    pub(crate) id: String,

    /// Message payload to be sent
    pub(crate) msg_payload: cosmrs::Any,
}

#[derive(prost::Message)]
pub(crate) struct QueryHtlcRequestProto {
    #[prost(string, tag = "1")]
    pub(crate) id: prost::alloc::string::String,
}

pub enum CreateHtlcMsg {
    Iris(IrisCreateHtlcMsg),
    Nucleus(NucleusCreateHtlcMsg),
}

impl CreateHtlcMsg {}

pub enum ClaimHtlcMsg {
    Iris(IrisClaimHtlcMsg),
    Nucleus(NucleusClaimHtlcMsg),
}

impl ClaimHtlcMsg {}

pub enum CreateHtlcProto {
    Iris(IrisCreateHtlcProto),
    Nucleus(NucleusCreateHtlcProto),
}

impl CreateHtlcProto {}

pub enum ClaimHtlcProto {
    Iris(IrisClaimHtlcProto),
    Nucleus(NucleusClaimHtlcProto),
}

impl ClaimHtlcProto {}

pub enum QueryHtlcResponse {
    Iris(IrisQueryHtlcResponseProto),
    Nucleus(NucleusQueryHtlcResponseProto),
}

impl QueryHtlcResponse {}
