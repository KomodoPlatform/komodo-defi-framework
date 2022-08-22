use cosmrs::{tx::{Msg, MsgProto},
             AccountId, Coin, ErrorReport};
use std::convert::TryFrom;

// Creating

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CreateHtlcProtoRep {
    #[prost(string, tag = "1")]
    pub sender: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub to: ::prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub receiver_on_other_chain: ::prost::alloc::string::String,
    #[prost(string, tag = "4")]
    pub sender_on_other_chain: ::prost::alloc::string::String,
    #[prost(message, repeated, tag = "5")]
    pub amount: ::prost::alloc::vec::Vec<cosmrs::proto::cosmos::base::v1beta1::Coin>,
    #[prost(string, tag = "6")]
    pub hash_lock: ::prost::alloc::string::String,
    #[prost(uint64, tag = "7")]
    pub timestamp: u64,
    #[prost(uint64, tag = "8")]
    pub time_lock: u64,
    #[prost(bool, tag = "9")]
    pub transfer: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct MsgCreateHtlc {
    /// Sender's address.
    pub to: AccountId,

    /// Recipient's address.
    pub sender: AccountId,

    /// The claim receiving address on the other chain.
    pub receiver_on_other_chain: String,

    /// The counterparty creator address on the other chain.
    pub sender_on_other_chain: String,

    /// Amount to send.
    pub amount: Vec<Coin>,

    /// The sha256 hash generated from secret and timestamp.
    pub hash_lock: String,

    /// The number of blocks to wait before the asset may be returned to.
    pub time_lock: u64,

    /// The timestamp in seconds for generating hash lock if provided.
    pub timestamp: u64,

    /// Whether it is an HTLT transaction.
    pub transfer: bool,
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
    const TYPE_URL: &'static str = "/irismod.htlc.MsgCreateHTLC";
}

// Claiming

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ClaimHtlcProtoRep {
    #[prost(string, tag = "1")]
    pub sender: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub id: ::prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub secret: ::prost::alloc::string::String,
}

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct MsgClaimHtlc {
    /// Sender's address.
    pub sender: AccountId,

    /// Generated HTLC ID
    pub id: String,

    /// Secret that has been used for generating hash_lock
    pub secret: String,
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
    const TYPE_URL: &'static str = "/irismod.htlc.MsgClaimHTLC";
}
