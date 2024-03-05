// Nucleus HTLC implementation in Rust on top of Cosmos SDK(cosmrs) for komodo-defi-framework.
//
// This module includes HTLC creating & claiming representation structstures
// and their trait implementations.

use super::htlc_proto::{NucleusClaimHtlcProto, NucleusCreateHtlcProto};

use cosmrs::{tx::{Msg, MsgProto},
             AccountId, Coin, ErrorReport};
use std::convert::TryFrom;

pub(crate) const NUCLEUS_CREATE_HTLC_TYPE_URL: &str = "/nucleus.htlc.MsgCreateHTLC";
pub(crate) const NUCLEUS_CLAIM_HTLC_TYPE_URL: &str = "/nucleus.htlc.MsgClaimHTLC";

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
    type Proto = NucleusCreateHtlcProto;
}

impl TryFrom<NucleusCreateHtlcProto> for MsgCreateHtlc {
    type Error = ErrorReport;

    fn try_from(proto: NucleusCreateHtlcProto) -> Result<MsgCreateHtlc, Self::Error> { MsgCreateHtlc::try_from(&proto) }
}

impl TryFrom<&NucleusCreateHtlcProto> for MsgCreateHtlc {
    type Error = ErrorReport;

    fn try_from(proto: &NucleusCreateHtlcProto) -> Result<MsgCreateHtlc, Self::Error> {
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

impl From<MsgCreateHtlc> for NucleusCreateHtlcProto {
    fn from(coin: MsgCreateHtlc) -> NucleusCreateHtlcProto { NucleusCreateHtlcProto::from(&coin) }
}

impl From<&MsgCreateHtlc> for NucleusCreateHtlcProto {
    fn from(msg: &MsgCreateHtlc) -> NucleusCreateHtlcProto {
        NucleusCreateHtlcProto {
            sender: msg.sender.to_string(),
            to: msg.to.to_string(),
            amount: msg.amount.iter().map(Into::into).collect(),
            hash_lock: msg.hash_lock.clone(),
            timestamp: msg.timestamp,
            time_lock: msg.time_lock,
        }
    }
}

impl MsgProto for NucleusCreateHtlcProto {
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
    type Proto = NucleusClaimHtlcProto;
}

impl TryFrom<NucleusClaimHtlcProto> for MsgClaimHtlc {
    type Error = ErrorReport;

    fn try_from(proto: NucleusClaimHtlcProto) -> Result<MsgClaimHtlc, Self::Error> { MsgClaimHtlc::try_from(&proto) }
}

impl TryFrom<&NucleusClaimHtlcProto> for MsgClaimHtlc {
    type Error = ErrorReport;

    fn try_from(proto: &NucleusClaimHtlcProto) -> Result<MsgClaimHtlc, Self::Error> {
        Ok(MsgClaimHtlc {
            sender: proto.sender.parse()?,
            id: proto.id.clone(),
            secret: proto.secret.clone(),
        })
    }
}

impl From<MsgClaimHtlc> for NucleusClaimHtlcProto {
    fn from(coin: MsgClaimHtlc) -> NucleusClaimHtlcProto { NucleusClaimHtlcProto::from(&coin) }
}

impl From<&MsgClaimHtlc> for NucleusClaimHtlcProto {
    fn from(msg: &MsgClaimHtlc) -> NucleusClaimHtlcProto {
        NucleusClaimHtlcProto {
            sender: msg.sender.to_string(),
            id: msg.id.clone(),
            secret: msg.secret.clone(),
        }
    }
}

impl MsgProto for NucleusClaimHtlcProto {
    const TYPE_URL: &'static str = NUCLEUS_CLAIM_HTLC_TYPE_URL;
}
