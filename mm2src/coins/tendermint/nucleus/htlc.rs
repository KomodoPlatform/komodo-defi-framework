// Nucleus HTLC implementation in Rust on top of Cosmos SDK(cosmrs) for komodo-defi-framework.
//
// This module includes HTLC creating & claiming representation structstures
// and their trait implementations.

use super::htlc_proto::{ClaimHtlcProtoRep, CreateHtlcProtoRep};

use cosmrs::{tx::{Msg, MsgProto},
             AccountId, Coin, ErrorReport};
use std::convert::TryFrom;

pub(crate) const NUCLEUS_CREATE_HTLC_TYPE_URL: &str = "/nucleus.htlc.MsgCreateHTLC";
pub(crate) const NUCLEUS_CLAIM_HTLC_TYPE_URL: &str = "/nucleus.htlc.MsgClaimHTLC";

#[allow(dead_code)]
pub(crate) struct NucleusHtlc {
    /// Generated HTLC's ID.
    pub(crate) id: String,

    /// Message payload to be sent
    pub(crate) msg_payload: cosmrs::Any,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MsgCreateHtlc {
    /// Sender's address.
    pub(crate) to: AccountId,

    /// Recipient's address.
    pub(crate) sender: AccountId,

    /// Amount to send.
    pub(crate) amount: Vec<Coin>,

    /// The sha256 hash generated from secret and timestamp.
    pub(crate) hash_lock: String,

    /// The number of blocks to wait before the asset may be returned to.
    pub(crate) time_lock: u64,

    /// The timestamp in seconds for generating hash lock if provided.
    pub(crate) timestamp: u64,
}

impl Msg for MsgCreateHtlc {
    type Proto = CreateHtlcProtoRep;
}

impl TryFrom<CreateHtlcProtoRep> for MsgCreateHtlc {
    type Error = ErrorReport;

    fn try_from(proto: CreateHtlcProtoRep) -> Result<MsgCreateHtlc, Self::Error> { MsgCreateHtlc::try_from(&proto) }
}

impl TryFrom<&CreateHtlcProtoRep> for MsgCreateHtlc {
    type Error = ErrorReport;

    fn try_from(proto: &CreateHtlcProtoRep) -> Result<MsgCreateHtlc, Self::Error> {
        Ok(MsgCreateHtlc {
            sender: proto.sender.parse()?,
            to: proto.to.parse()?,
            amount: proto.amount.iter().map(TryFrom::try_from).collect::<Result<_, _>>()?,
            hash_lock: proto.hash_lock.clone(),
            timestamp: proto.timestamp,
            time_lock: proto.time_lock,
        })
    }
}

impl From<MsgCreateHtlc> for CreateHtlcProtoRep {
    fn from(coin: MsgCreateHtlc) -> CreateHtlcProtoRep { CreateHtlcProtoRep::from(&coin) }
}

impl From<&MsgCreateHtlc> for CreateHtlcProtoRep {
    fn from(msg: &MsgCreateHtlc) -> CreateHtlcProtoRep {
        CreateHtlcProtoRep {
            sender: msg.sender.to_string(),
            to: msg.to.to_string(),
            amount: msg.amount.iter().map(Into::into).collect(),
            hash_lock: msg.hash_lock.clone(),
            timestamp: msg.timestamp,
            time_lock: msg.time_lock,
        }
    }
}

impl MsgProto for CreateHtlcProtoRep {
    const TYPE_URL: &'static str = NUCLEUS_CREATE_HTLC_TYPE_URL;
}

#[derive(Clone)]
pub(crate) struct MsgClaimHtlc {
    /// Sender's address.
    pub(crate) sender: AccountId,

    /// Generated HTLC ID
    pub(crate) id: String,

    /// Secret that has been used for generating hash_lock
    pub(crate) secret: String,
}

impl Msg for MsgClaimHtlc {
    type Proto = ClaimHtlcProtoRep;
}

impl TryFrom<ClaimHtlcProtoRep> for MsgClaimHtlc {
    type Error = ErrorReport;

    fn try_from(proto: ClaimHtlcProtoRep) -> Result<MsgClaimHtlc, Self::Error> { MsgClaimHtlc::try_from(&proto) }
}

impl TryFrom<&ClaimHtlcProtoRep> for MsgClaimHtlc {
    type Error = ErrorReport;

    fn try_from(proto: &ClaimHtlcProtoRep) -> Result<MsgClaimHtlc, Self::Error> {
        Ok(MsgClaimHtlc {
            sender: proto.sender.parse()?,
            id: proto.id.clone(),
            secret: proto.secret.clone(),
        })
    }
}

impl From<MsgClaimHtlc> for ClaimHtlcProtoRep {
    fn from(coin: MsgClaimHtlc) -> ClaimHtlcProtoRep { ClaimHtlcProtoRep::from(&coin) }
}

impl From<&MsgClaimHtlc> for ClaimHtlcProtoRep {
    fn from(msg: &MsgClaimHtlc) -> ClaimHtlcProtoRep {
        ClaimHtlcProtoRep {
            sender: msg.sender.to_string(),
            id: msg.id.clone(),
            secret: msg.secret.clone(),
        }
    }
}

impl MsgProto for ClaimHtlcProtoRep {
    const TYPE_URL: &'static str = NUCLEUS_CLAIM_HTLC_TYPE_URL;
}
