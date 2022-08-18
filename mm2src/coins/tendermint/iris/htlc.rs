use cosmrs::{tx::{Msg, MsgProto},
             AccountId, Coin, ErrorReport};
use std::convert::TryFrom;

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct HtlcProtoRep {
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
    type Proto = HtlcProtoRep;
}

impl TryFrom<HtlcProtoRep> for MsgCreateHtlc {
    type Error = ErrorReport;

    fn try_from(proto: HtlcProtoRep) -> Result<MsgCreateHtlc, Self::Error> { MsgCreateHtlc::try_from(&proto) }
}

impl TryFrom<&HtlcProtoRep> for MsgCreateHtlc {
    type Error = ErrorReport;

    fn try_from(proto: &HtlcProtoRep) -> Result<MsgCreateHtlc, Self::Error> {
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

impl From<MsgCreateHtlc> for HtlcProtoRep {
    fn from(coin: MsgCreateHtlc) -> HtlcProtoRep { HtlcProtoRep::from(&coin) }
}

impl From<&MsgCreateHtlc> for HtlcProtoRep {
    fn from(msg: &MsgCreateHtlc) -> HtlcProtoRep {
        HtlcProtoRep {
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

impl MsgProto for HtlcProtoRep {
    const TYPE_URL: &'static str = "/irismod.htlc.MsgCreateHTLC";
}
