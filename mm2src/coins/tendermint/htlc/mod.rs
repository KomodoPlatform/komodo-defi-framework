mod iris;
mod nucleus;

pub(crate) use commons::*;
pub(crate) use iris::*;
pub(crate) use nucleus::*;

pub(crate) mod commons {
    #[allow(dead_code)]
    pub(crate) struct TendermintHtlc {
        /// Generated HTLC's ID.
        pub(crate) id: String,

        /// Message payload to be sent
        pub(crate) msg_payload: cosmrs::Any,
    }

    #[rustfmt::skip]
    #[derive(prost::Enumeration, Debug)]
    #[repr(i32)]
    pub enum HtlcState {
        Open = 0,
        Completed = 1,
        Refunded = 2,
    }

    #[derive(prost::Message)]
    pub(crate) struct QueryHtlcRequestProto {
        #[prost(string, tag = "1")]
        pub(crate) id: prost::alloc::string::String,
    }
}
