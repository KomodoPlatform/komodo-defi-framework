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

/// Defines an open state.
pub(crate) const HTLC_STATE_OPEN: i32 = 0;

/// Defines a completed state.
pub(crate) const HTLC_STATE_COMPLETED: i32 = 1;

/// Defines a refunded state.
pub(crate) const HTLC_STATE_REFUNDED: i32 = 2;

/// Indicates whether this is an IRIS or Nucleus HTLC.
#[derive(Copy, Clone)]
pub(crate) enum HtlcType {
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
    /// Returns the ABCI endpoint path for querying HTLCs.
    pub(crate) fn get_htlc_abci_query_path(&self) -> String {
        const NUCLEUS_PATH: &str = "/nucleus.htlc.Query/HTLC";
        const IRIS_PATH: &str = "/irismod.htlc.Query/HTLC";

        match self {
            Self::Nucleus => NUCLEUS_PATH.to_owned(),
            Self::Iris => IRIS_PATH.to_owned(),
        }
    }
}

/// Custom Tendermint message types specific to certain Cosmos chains and may not be available on all chains.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum CustomTendermintMsgType {
    /// Create HTLC as sender.
    SendHtlcAmount,
    /// Claim HTLC as reciever.
    ClaimHtlcAmount,
    /// Claim HTLC for reciever.
    SignClaimHtlc,
}

/// Defines the state of an HTLC.
#[derive(prost::Enumeration, Debug)]
#[repr(i32)]
pub enum HtlcState {
    /// Open state.
    Open = HTLC_STATE_OPEN,
    /// Completed state.
    Completed = HTLC_STATE_COMPLETED,
    /// Refunded state.
    Refunded = HTLC_STATE_REFUNDED,
}

#[allow(dead_code)]
pub(crate) struct TendermintHtlc {
    /// Generated HTLC's ID.
    pub(crate) id: String,

    /// Message payload to be sent.
    pub(crate) msg_payload: Any,
}

#[derive(prost::Message)]
pub(crate) struct QueryHtlcRequestProto {
    /// HTLC ID to query.
    #[prost(string, tag = "1")]
    pub(crate) id: prost::alloc::string::String,
}

/// Generic enum for abstracting multiple types of create HTLC messages.
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
    pub(crate) fn new(
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

    /// Returns the inner field `sender`.
    pub(crate) fn sender(&self) -> &AccountId {
        match self {
            Self::Iris(inner) => &inner.sender,
            Self::Nucleus(inner) => &inner.sender,
        }
    }

    /// Returns the inner field `to`.
    pub(crate) fn to(&self) -> &AccountId {
        match self {
            Self::Iris(inner) => &inner.to,
            Self::Nucleus(inner) => &inner.to,
        }
    }

    /// Returns the inner field `amount`.
    pub(crate) fn amount(&self) -> &[Coin] {
        match self {
            Self::Iris(inner) => &inner.amount,
            Self::Nucleus(inner) => &inner.amount,
        }
    }

    /// Generates `Any` from the inner CreateHTLC message.
    pub(crate) fn to_any(&self) -> Result<Any, ErrorReport> {
        match self {
            Self::Iris(inner) => inner.to_any(),
            Self::Nucleus(inner) => inner.to_any(),
        }
    }
}

/// Generic enum for abstracting multiple types of claim HTLC messages.
pub(crate) enum ClaimHtlcMsg {
    Nucleus(NucleusClaimHtlcMsg),
    Iris(IrisClaimHtlcMsg),
}

impl ClaimHtlcMsg {
    pub(crate) fn new(htlc_type: HtlcType, id: String, sender: AccountId, secret: String) -> Self {
        match htlc_type {
            HtlcType::Iris => ClaimHtlcMsg::Iris(IrisClaimHtlcMsg { sender, id, secret }),
            HtlcType::Nucleus => ClaimHtlcMsg::Nucleus(NucleusClaimHtlcMsg { sender, id, secret }),
        }
    }

    /// Returns the inner field `secret`.
    pub(crate) fn secret(&self) -> &str {
        match self {
            Self::Iris(inner) => &inner.secret,
            Self::Nucleus(inner) => &inner.secret,
        }
    }

    /// Generates `Any` from the inner ClaimHTLC message.
    pub(crate) fn to_any(&self) -> Result<Any, ErrorReport> {
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

/// Generic enum for abstracting multiple types of create HTLC protos.
pub(crate) enum CreateHtlcProto {
    Nucleus(NucleusCreateHtlcProto),
    Iris(IrisCreateHtlcProto),
}

impl CreateHtlcProto {
    /// Decodes an instance (depending on the given `htlc_type`) of `CreateHtlcProto` from a buffer.
    pub(crate) fn decode(htlc_type: HtlcType, buffer: &[u8]) -> Result<Self, DecodeError> {
        match htlc_type {
            HtlcType::Nucleus => Ok(Self::Nucleus(NucleusCreateHtlcProto::decode(buffer)?)),
            HtlcType::Iris => Ok(Self::Iris(IrisCreateHtlcProto::decode(buffer)?)),
        }
    }

    /// Returns the inner field `hash_lock`.
    pub(crate) fn hash_lock(&self) -> &str {
        match self {
            Self::Iris(inner) => &inner.hash_lock,
            Self::Nucleus(inner) => &inner.hash_lock,
        }
    }
}

/// Generic enum for abstracting multiple types of claim HTLC protos.
pub(crate) enum ClaimHtlcProto {
    Nucleus(NucleusClaimHtlcProto),
    Iris(IrisClaimHtlcProto),
}

impl ClaimHtlcProto {
    /// Decodes an instance (depending on the given `htlc_type`) of `ClaimHtlcProto` from a buffer.
    pub(crate) fn decode(htlc_type: HtlcType, buffer: &[u8]) -> Result<Self, DecodeError> {
        match htlc_type {
            HtlcType::Nucleus => Ok(Self::Nucleus(NucleusClaimHtlcProto::decode(buffer)?)),
            HtlcType::Iris => Ok(Self::Iris(IrisClaimHtlcProto::decode(buffer)?)),
        }
    }

    /// Returns the inner field `secret`.
    #[cfg(test)]
    pub(crate) fn secret(&self) -> &str {
        match self {
            Self::Iris(inner) => &inner.secret,
            Self::Nucleus(inner) => &inner.secret,
        }
    }
}

/// Generic enum for abstracting multiple types of HTLC responses.
pub(crate) enum QueryHtlcResponse {
    Nucleus(NucleusQueryHtlcResponseProto),
    Iris(IrisQueryHtlcResponseProto),
}

impl QueryHtlcResponse {
    /// Decodes an instance (depending on the given `htlc_type`) of `QueryHtlcResponse` from a buffer.
    pub(crate) fn decode(htlc_type: HtlcType, buffer: &[u8]) -> Result<Self, DecodeError> {
        match htlc_type {
            HtlcType::Nucleus => Ok(Self::Nucleus(NucleusQueryHtlcResponseProto::decode(buffer)?)),
            HtlcType::Iris => Ok(Self::Iris(IrisQueryHtlcResponseProto::decode(buffer)?)),
        }
    }

    /// Returns the inner field `htlc_state`.
    pub(crate) fn htlc_state(&self) -> Option<i32> {
        match self {
            Self::Iris(inner) => Some(inner.htlc.as_ref()?.state),
            Self::Nucleus(inner) => Some(inner.htlc.as_ref()?.state),
        }
    }

    /// Returns the inner field `hash_lock`.
    pub(crate) fn hash_lock(&self) -> Option<&str> {
        match self {
            Self::Iris(inner) => Some(&inner.htlc.as_ref()?.hash_lock),
            Self::Nucleus(inner) => Some(&inner.htlc.as_ref()?.hash_lock),
        }
    }
}
