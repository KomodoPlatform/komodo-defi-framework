#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SignedMessage {
    #[prost(bytes="vec", tag="1")]
    pub from: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", tag="2")]
    pub signature: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", tag="3")]
    pub payload: ::prost::alloc::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MakerNegotiation {
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TakerNegotiation {
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MakerNegotiated {
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TakerPaymentInfo {
    #[prost(bytes="vec", tag="1")]
    pub tx_bytes: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", optional, tag="2")]
    pub next_step_instructions: ::core::option::Option<::prost::alloc::vec::Vec<u8>>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MakerPaymentInfo {
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SwapMessage {
    #[prost(oneof="swap_message::Inner", tags="1, 2, 3, 4, 5")]
    pub inner: ::core::option::Option<swap_message::Inner>,
}
/// Nested message and enum types in `SwapMessage`.
pub mod swap_message {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Inner {
        #[prost(message, tag="1")]
        MakerNegotiation(super::MakerNegotiation),
        #[prost(message, tag="2")]
        TakerNegotiation(super::TakerNegotiation),
        #[prost(message, tag="3")]
        MakerNegotiated(super::MakerNegotiated),
        #[prost(message, tag="4")]
        TakerPaymentInfo(super::TakerPaymentInfo),
        #[prost(message, tag="5")]
        MakerPaymentInfo(super::MakerPaymentInfo),
    }
}
