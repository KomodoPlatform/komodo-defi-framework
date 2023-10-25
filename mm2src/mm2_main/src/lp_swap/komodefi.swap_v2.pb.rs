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
    #[prost(uint64, tag="1")]
    pub started_at: u64,
    #[prost(uint64, tag="2")]
    pub payment_locktime: u64,
    #[prost(bytes="vec", tag="3")]
    pub secret_hash: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", tag="4")]
    pub maker_coin_htlc_pub: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", tag="5")]
    pub taker_coin_htlc_pub: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", optional, tag="6")]
    pub maker_coin_swap_contract: ::core::option::Option<::prost::alloc::vec::Vec<u8>>,
    #[prost(bytes="vec", optional, tag="7")]
    pub taker_coin_swap_contract: ::core::option::Option<::prost::alloc::vec::Vec<u8>>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Abort {
    #[prost(string, tag="1")]
    pub reason: ::prost::alloc::string::String,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TakerNegotiationData {
    #[prost(uint64, tag="1")]
    pub started_at: u64,
    #[prost(uint64, tag="2")]
    pub payment_locktime: u64,
    /// add bytes secret_hash = 3 if required
    #[prost(bytes="vec", tag="4")]
    pub maker_coin_htlc_pub: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", tag="5")]
    pub taker_coin_htlc_pub: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", optional, tag="6")]
    pub maker_coin_swap_contract: ::core::option::Option<::prost::alloc::vec::Vec<u8>>,
    #[prost(bytes="vec", optional, tag="7")]
    pub taker_coin_swap_contract: ::core::option::Option<::prost::alloc::vec::Vec<u8>>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TakerNegotiation {
    #[prost(oneof="taker_negotiation::Action", tags="1, 2")]
    pub action: ::core::option::Option<taker_negotiation::Action>,
}
/// Nested message and enum types in `TakerNegotiation`.
pub mod taker_negotiation {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Action {
        #[prost(message, tag="1")]
        Continue(super::TakerNegotiationData),
        #[prost(message, tag="2")]
        Abort(super::Abort),
    }
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MakerNegotiated {
    #[prost(bool, tag="1")]
    pub negotiated: bool,
    /// used when negotiated is false
    #[prost(string, optional, tag="2")]
    pub reason: ::core::option::Option<::prost::alloc::string::String>,
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
    #[prost(bytes="vec", tag="1")]
    pub tx_bytes: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", optional, tag="2")]
    pub next_step_instructions: ::core::option::Option<::prost::alloc::vec::Vec<u8>>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct TakerPaymentSpendPreimage {
    #[prost(bytes="vec", tag="1")]
    pub signature: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", optional, tag="2")]
    pub tx_preimage: ::core::option::Option<::prost::alloc::vec::Vec<u8>>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SwapMessage {
    #[prost(oneof="swap_message::Inner", tags="1, 2, 3, 4, 5, 6")]
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
        #[prost(message, tag="6")]
        TakerPaymentSpendPreimage(super::TakerPaymentSpendPreimage),
    }
}
