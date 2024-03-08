use cosmrs::proto::prost;

#[derive(prost::Message)]
pub(crate) struct CreateHtlcProtoRep {
    #[prost(string, tag = "1")]
    pub(crate) sender: prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub(crate) to: prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub(crate) receiver_on_other_chain: prost::alloc::string::String,
    #[prost(string, tag = "4")]
    pub(crate) sender_on_other_chain: prost::alloc::string::String,
    #[prost(message, repeated, tag = "5")]
    pub(crate) amount: prost::alloc::vec::Vec<cosmrs::proto::cosmos::base::v1beta1::Coin>,
    #[prost(string, tag = "6")]
    pub(crate) hash_lock: prost::alloc::string::String,
    #[prost(uint64, tag = "7")]
    pub(crate) timestamp: u64,
    #[prost(uint64, tag = "8")]
    pub(crate) time_lock: u64,
    #[prost(bool, tag = "9")]
    pub(crate) transfer: bool,
}

// impl prost::Message for CreateHtlcProtoRep {
//     fn encode_raw<B>(&self, buf: &mut B)
//         where
//             B: prost::bytes::BufMut,
//             Self: Sized {
//         ::prost::Message::encode_raw(self, buf)
//     }

//     fn merge_field<B>(
//             &mut self,
//             tag: u32,
//             wire_type: prost::encoding::WireType,
//             buf: &mut B,
//             ctx: prost::encoding::DecodeContext,
//         ) -> Result<(), prost::DecodeError>
//         where
//             B: hyper::body::Buf,
//             Self: Sized {
//         ::prost::Message::merge_field(self, tag, wire_type, buf, ctx)
//     }

//     fn encoded_len(&self) -> usize {
//         ::prost::Message::encoded_len(self)
//     }

//     fn clear(&mut self) {
//         ::prost::Message::clear(self)
//     }
// }

#[derive(prost::Message)]
pub(crate) struct ClaimHtlcProtoRep {
    #[prost(string, tag = "1")]
    pub(crate) sender: prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub(crate) id: prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub(crate) secret: prost::alloc::string::String,
}

// impl prost::Message for ClaimHtlcProtoRep {
//     fn encode_raw<B>(&self, buf: &mut B)
//         where
//             B: prost::bytes::BufMut,
//             Self: Sized {
//         ::prost::Message::encode_raw(self, buf)
//     }

//     fn merge_field<B>(
//             &mut self,
//             tag: u32,
//             wire_type: prost::encoding::WireType,
//             buf: &mut B,
//             ctx: prost::encoding::DecodeContext,
//         ) -> Result<(), prost::DecodeError>
//         where
//             B: hyper::body::Buf,
//             Self: Sized {
//         ::prost::Message::merge_field(self, tag, wire_type, buf, ctx)
//     }

//     fn encoded_len(&self) -> usize {
//         ::prost::Message::encoded_len(self)
//     }

//     fn clear(&mut self) {
//         ::prost::Message::clear(self)
//     }
// }

#[derive(prost::Message)]
pub(crate) struct QueryHtlcRequestProto {
    #[prost(string, tag = "1")]
    pub(crate) id: prost::alloc::string::String,
}

#[derive(prost::Enumeration, Debug)]
#[repr(i32)]
pub enum HtlcState {
    Open = 0,
    Completed = 1,
    Refunded = 2,
}

#[derive(prost::Message)]
pub struct HtlcProto {
    #[prost(string, tag = "1")]
    pub(crate) id: prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub(crate) sender: prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub(crate) to: prost::alloc::string::String,
    #[prost(string, tag = "4")]
    pub(crate) receiver_on_other_chain: prost::alloc::string::String,
    #[prost(string, tag = "5")]
    pub(crate) sender_on_other_chain: prost::alloc::string::String,
    #[prost(message, repeated, tag = "6")]
    pub(crate) amount: prost::alloc::vec::Vec<cosmrs::proto::cosmos::base::v1beta1::Coin>,
    #[prost(string, tag = "7")]
    pub(crate) hash_lock: prost::alloc::string::String,
    #[prost(string, tag = "8")]
    pub(crate) secret: prost::alloc::string::String,
    #[prost(uint64, tag = "9")]
    pub(crate) timestamp: u64,
    #[prost(uint64, tag = "10")]
    pub(crate) expiration_height: u64,
    #[prost(enumeration = "HtlcState", tag = "11")]
    pub(crate) state: i32,
    #[prost(uint64, tag = "12")]
    pub(crate) closed_block: u64,
    #[prost(bool, tag = "13")]
    pub(crate) transfer: bool,
}

#[derive(prost::Message)]
pub(crate) struct QueryHtlcResponseProto {
    #[prost(message, tag = "1")]
    pub(crate) htlc: Option<HtlcProto>,
}
