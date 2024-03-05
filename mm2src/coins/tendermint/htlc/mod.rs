mod iris;
mod nucleus;

use std::{convert::TryFrom, str::FromStr};

use cosmrs::ErrorReport;
use iris::htlc::{IrisClaimHtlcMsg, IrisCreateHtlcMsg};
use nucleus::htlc::{NucleusClaimHtlcMsg, NucleusCreateHtlcMsg};

use iris::htlc_proto::{IrisClaimHtlcProto, IrisCreateHtlcProto, IrisQueryHtlcResponseProto};
use nucleus::htlc_proto::{NucleusClaimHtlcProto, NucleusCreateHtlcProto, NucleusQueryHtlcResponseProto};
use prost::{DecodeError, Message};
use std::io;

pub(crate) const HTLC_STATE_OPEN: i32 = 0;
pub(crate) const HTLC_STATE_COMPLETED: i32 = 1;
pub(crate) const HTLC_STATE_REFUNDED: i32 = 2;

pub enum HtlcType {
    Nucleus,
    Iris,
}

impl FromStr for HtlcType {
    type Err = io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "iaa" => Ok(HtlcType::Iris),
            "nucl" => Ok(HtlcType::Nucleus),
            unsupported => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("Account type '{unsupported}' is not supported for HTLCs"),
            )),
        }
    }
}

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
    Nucleus(NucleusCreateHtlcMsg),
    Iris(IrisCreateHtlcMsg),
}

impl CreateHtlcMsg {}

pub enum ClaimHtlcMsg {
    Nucleus(NucleusClaimHtlcMsg),
    Iris(IrisClaimHtlcMsg),
}

impl ClaimHtlcMsg {
    pub fn secret(&self) -> &str {
        match self {
            ClaimHtlcMsg::Iris(inner) => &inner.secret,
            ClaimHtlcMsg::Nucleus(inner) => &inner.secret,
        }
    }
}

impl TryFrom<ClaimHtlcProto> for ClaimHtlcMsg {
    type Error = ErrorReport;

    fn try_from(value: ClaimHtlcProto) -> Result<Self, Self::Error> {
        match value {
            ClaimHtlcProto::Nucleus(inner) => Ok(ClaimHtlcMsg::Nucleus(NucleusClaimHtlcMsg::try_from(inner)?)),
            ClaimHtlcProto::Iris(inner) => Ok(ClaimHtlcMsg::Iris(IrisClaimHtlcMsg::try_from(inner)?)),
        }
    }
}

pub enum CreateHtlcProto {
    Nucleus(NucleusCreateHtlcProto),
    Iris(IrisCreateHtlcProto),
}

impl CreateHtlcProto {}

pub enum ClaimHtlcProto {
    Nucleus(NucleusClaimHtlcProto),
    Iris(IrisClaimHtlcProto),
}

impl ClaimHtlcProto {
    pub fn decode(htlc_type: HtlcType, bytes: &[u8]) -> Result<Self, DecodeError> {
        match htlc_type {
            HtlcType::Nucleus => Ok(ClaimHtlcProto::Nucleus(NucleusClaimHtlcProto::decode(bytes)?)),
            HtlcType::Iris => Ok(ClaimHtlcProto::Iris(IrisClaimHtlcProto::decode(bytes)?)),
        }
    }

    pub fn secret(&self) -> &str {
        match self {
            ClaimHtlcProto::Iris(inner) => &inner.secret,
            ClaimHtlcProto::Nucleus(inner) => &inner.secret,
        }
    }
}

pub enum QueryHtlcResponse {
    Nucleus(NucleusQueryHtlcResponseProto),
    Iris(IrisQueryHtlcResponseProto),
}

impl QueryHtlcResponse {}
