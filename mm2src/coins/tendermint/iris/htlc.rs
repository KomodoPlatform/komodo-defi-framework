// IRIS HTLC implementation in Rust on top of Cosmos SDK(cosmrs) for AtomicDEX.
//
// This module includes HTLC creating & claiming representation structstures
// and their trait implementations.
//
// ** Acquiring testnet assets **
//
// Since there is no sdk exists for Rust on Iris Network, we should
// either implement some of the Iris Network funcionality on Rust or
// simply use their unit tests.
//
// Because we had limited time for the HTLC implementation, for now
// we can use their unit tests in order to acquire IBC assets.
// For that, clone https://github.com/ozkanonur/irishub-sdk-js repository and check
// dummy.test.ts file(change the asset, amount, target address if needed)
// and then run the following commands:
// - yarn
// - npm run test
//
// If the sender address doesn't have enough nyan tokens to complete unit tests,
// check this page https://www.irisnet.org/docs/get-started/testnet.html#faucet

use super::htlc_proto::{ClaimHtlcProtoRep, CreateHtlcProtoRep};
use cosmrs::{tx::{Msg, MsgProto},
             AccountId, Coin, ErrorReport};
use std::convert::TryFrom;

// https://github.com/irisnet/irismod/blob/043e058cd6e17f4f96d32f17bfd20b67debfab0b/proto/htlc/htlc.proto#L36
pub const HTLC_STATE_OPEN: i32 = 0;
pub const HTLC_STATE_COMPLETED: i32 = 1;
pub const HTLC_STATE_REFUNDED: i32 = 2;

const CREATE_HTLC_TYPE_URL: &str = "/irismod.htlc.MsgCreateHTLC";
const CLAIM_HTLC_TYPE_URL: &str = "/irismod.htlc.MsgClaimHTLC";

#[allow(dead_code)]
pub(crate) struct IrisHtlc {
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

    /// The claim receiving address on the other chain.
    pub(crate) receiver_on_other_chain: String,

    /// The counterparty creator address on the other chain.
    pub(crate) sender_on_other_chain: String,

    /// Amount to send.
    pub(crate) amount: Vec<Coin>,

    /// The sha256 hash generated from secret and timestamp.
    pub(crate) hash_lock: String,

    /// The number of blocks to wait before the asset may be returned to.
    pub(crate) time_lock: u64,

    /// The timestamp in seconds for generating hash lock if provided.
    pub(crate) timestamp: u64,

    /// Whether it is an HTLT transaction.
    pub(crate) transfer: bool,
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
            receiver_on_other_chain: proto.receiver_on_other_chain.clone(),
            sender_on_other_chain: proto.sender_on_other_chain.clone(),
            hash_lock: proto.hash_lock.clone(),
            timestamp: proto.timestamp,
            time_lock: proto.time_lock,
            transfer: proto.transfer,
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
            receiver_on_other_chain: msg.receiver_on_other_chain.clone(),
            sender_on_other_chain: msg.sender_on_other_chain.clone(),
            hash_lock: msg.hash_lock.clone(),
            timestamp: msg.timestamp,
            time_lock: msg.time_lock,
            transfer: msg.transfer,
        }
    }
}

impl MsgProto for CreateHtlcProtoRep {
    const TYPE_URL: &'static str = CREATE_HTLC_TYPE_URL;
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
    const TYPE_URL: &'static str = CLAIM_HTLC_TYPE_URL;
}
