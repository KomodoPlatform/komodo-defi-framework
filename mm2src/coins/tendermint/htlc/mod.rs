mod iris;
mod nucleus;

use std::{convert::TryFrom, str::FromStr};

use cosmrs::{tx::Msg, AccountId, Any, Coin, ErrorReport};
use iris::htlc::{IrisClaimHtlcMsg, IrisCreateHtlcMsg};
use nucleus::htlc::{NucleusClaimHtlcMsg, NucleusCreateHtlcMsg};

use iris::htlc_proto::{IrisClaimHtlcProto, IrisCreateHtlcProto, IrisQueryHtlcResponseProto};
use nucleus::htlc_proto::{NucleusClaimHtlcProto, NucleusCreateHtlcProto, NucleusQueryHtlcResponseProto};
use prost::{DecodeError, Message};
use std::io;

pub(crate) const HTLC_STATE_OPEN: i32 = 0;
pub(crate) const HTLC_STATE_COMPLETED: i32 = 1;
pub(crate) const HTLC_STATE_REFUNDED: i32 = 2;

#[derive(Copy, Clone)]
pub enum HtlcType {
    Nucleus,
    Iris,
}

impl FromStr for HtlcType {
    type Err = io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "iaa" => Ok(HtlcType::Iris),
            "nuc" => Ok(HtlcType::Nucleus),
            unsupported => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("Account type '{unsupported}' is not supported for HTLCs"),
            )),
        }
    }
}

impl HtlcType {
    pub(crate) fn get_htlc_abci_query_path(&self) -> String {
        const NUCLEUS_PATH: &str = "/nucleus.htlc.Query/HTLC";
        const IRIS_PATH: &str = "/iris.htlc.Query/HTLC";

        match self {
            Self::Nucleus => NUCLEUS_PATH.to_owned(),
            Self::Iris => IRIS_PATH.to_owned(),
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

#[derive(Debug, PartialEq)]
pub(crate) enum CreateHtlcMsg {
    Nucleus(NucleusCreateHtlcMsg),
    Iris(IrisCreateHtlcMsg),
}

impl TryFrom<CreateHtlcProto> for CreateHtlcMsg {
    type Error = ErrorReport;

    fn try_from(value: CreateHtlcProto) -> Result<Self, Self::Error> {
        match value {
            CreateHtlcProto::Nucleus(inner) => Ok(CreateHtlcMsg::Nucleus(NucleusCreateHtlcMsg::try_from(inner)?)),
            CreateHtlcProto::Iris(inner) => Ok(CreateHtlcMsg::Iris(IrisCreateHtlcMsg::try_from(inner)?)),
        }
    }
}

impl CreateHtlcMsg {
    pub fn new(
        htlc_type: HtlcType,
        sender: AccountId,
        to: AccountId,
        amount: Vec<Coin>,
        hash_lock: String,
        timestamp: u64,
        time_lock: u64,
    ) -> Self {
        match htlc_type {
            HtlcType::Iris => CreateHtlcMsg::Iris(IrisCreateHtlcMsg {
                to,
                sender,
                receiver_on_other_chain: String::default(),
                sender_on_other_chain: String::default(),
                amount,
                hash_lock,
                time_lock,
                timestamp,
                transfer: false,
            }),
            HtlcType::Nucleus => CreateHtlcMsg::Nucleus(NucleusCreateHtlcMsg {
                to,
                sender,
                amount,
                hash_lock,
                time_lock,
                timestamp,
            }),
        }
    }

    pub fn sender(&self) -> &AccountId {
        match self {
            Self::Iris(inner) => &inner.sender,
            Self::Nucleus(inner) => &inner.sender,
        }
    }

    pub fn to(&self) -> &AccountId {
        match self {
            Self::Iris(inner) => &inner.to,
            Self::Nucleus(inner) => &inner.to,
        }
    }

    pub fn amount(&self) -> &[Coin] {
        match self {
            Self::Iris(inner) => &inner.amount,
            Self::Nucleus(inner) => &inner.amount,
        }
    }

    pub fn to_any(&self) -> Result<Any, ErrorReport> {
        match self {
            Self::Iris(inner) => inner.to_any(),
            Self::Nucleus(inner) => inner.to_any(),
        }
    }
}

pub(crate) enum ClaimHtlcMsg {
    Nucleus(NucleusClaimHtlcMsg),
    Iris(IrisClaimHtlcMsg),
}

impl ClaimHtlcMsg {
    pub fn new(htlc_type: HtlcType, id: String, sender: AccountId, secret: String) -> Self {
        match htlc_type {
            HtlcType::Iris => ClaimHtlcMsg::Iris(IrisClaimHtlcMsg { sender, id, secret }),
            HtlcType::Nucleus => ClaimHtlcMsg::Nucleus(NucleusClaimHtlcMsg { sender, id, secret }),
        }
    }

    pub fn secret(&self) -> &str {
        match self {
            Self::Iris(inner) => &inner.secret,
            Self::Nucleus(inner) => &inner.secret,
        }
    }

    pub fn to_any(&self) -> Result<Any, ErrorReport> {
        match self {
            Self::Iris(inner) => inner.to_any(),
            Self::Nucleus(inner) => inner.to_any(),
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

pub(crate) enum CreateHtlcProto {
    Nucleus(NucleusCreateHtlcProto),
    Iris(IrisCreateHtlcProto),
}

impl CreateHtlcProto {
    pub fn decode(htlc_type: HtlcType, bytes: &[u8]) -> Result<Self, DecodeError> {
        match htlc_type {
            HtlcType::Nucleus => Ok(Self::Nucleus(NucleusCreateHtlcProto::decode(bytes)?)),
            HtlcType::Iris => Ok(Self::Iris(IrisCreateHtlcProto::decode(bytes)?)),
        }
    }

    pub fn hash_lock(&self) -> &str {
        match self {
            Self::Iris(inner) => &inner.hash_lock,
            Self::Nucleus(inner) => &inner.hash_lock,
        }
    }
}

pub(crate) enum ClaimHtlcProto {
    Nucleus(NucleusClaimHtlcProto),
    Iris(IrisClaimHtlcProto),
}

impl ClaimHtlcProto {
    pub fn decode(htlc_type: HtlcType, bytes: &[u8]) -> Result<Self, DecodeError> {
        match htlc_type {
            HtlcType::Nucleus => Ok(Self::Nucleus(NucleusClaimHtlcProto::decode(bytes)?)),
            HtlcType::Iris => Ok(Self::Iris(IrisClaimHtlcProto::decode(bytes)?)),
        }
    }

    #[allow(dead_code)]
    pub fn secret(&self) -> &str {
        match self {
            Self::Iris(inner) => &inner.secret,
            Self::Nucleus(inner) => &inner.secret,
        }
    }
}

pub(crate) enum QueryHtlcResponse {
    Nucleus(NucleusQueryHtlcResponseProto),
    Iris(IrisQueryHtlcResponseProto),
}

impl QueryHtlcResponse {
    pub fn decode(htlc_type: HtlcType, bytes: &[u8]) -> Result<Self, DecodeError> {
        match htlc_type {
            HtlcType::Nucleus => Ok(Self::Nucleus(NucleusQueryHtlcResponseProto::decode(bytes)?)),
            HtlcType::Iris => Ok(Self::Iris(IrisQueryHtlcResponseProto::decode(bytes)?)),
        }
    }

    pub fn htlc_state(&self) -> Option<i32> {
        match self {
            Self::Iris(inner) => Some(inner.htlc.as_ref()?.state),
            Self::Nucleus(inner) => Some(inner.htlc.as_ref()?.state),
        }
    }

    pub fn hash_lock(&self) -> Option<&str> {
        match self {
            Self::Iris(inner) => Some(&inner.htlc.as_ref()?.hash_lock),
            Self::Nucleus(inner) => Some(&inner.htlc.as_ref()?.hash_lock),
        }
    }
}
