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
