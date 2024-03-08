use cosmrs::proto::prost;

#[derive(prost::Message)]
pub(crate) struct IBCTransferV1Proto {
    #[prost(string, tag = "1")]
    pub(crate) source_port: prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub(crate) source_channel: prost::alloc::string::String,
    #[prost(message, optional, tag = "3")]
    pub(crate) token: Option<cosmrs::proto::cosmos::base::v1beta1::Coin>,
    #[prost(string, tag = "4")]
    pub(crate) sender: prost::alloc::string::String,
    #[prost(string, tag = "5")]
    pub(crate) receiver: prost::alloc::string::String,
    #[prost(message, optional, tag = "6")]
    pub(crate) timeout_height: Option<cosmrs::proto::ibc::core::client::v1::Height>,
    #[prost(uint64, tag = "7")]
    pub(crate) timeout_timestamp: u64,
    // Not supported by some of the cosmos chains like IRIS
    // #[prost(string, optional, tag = "8")]
    // pub(crate) memo: Option<String>,
}

// impl prost::Message for IBCTransferV1Proto {
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