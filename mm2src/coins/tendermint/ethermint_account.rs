use cosmrs::proto::cosmos::auth::v1beta1::BaseAccount;

#[derive(prost::Message)]
pub struct EthermintAccount {
    #[prost(message, optional, tag = "1")]
    pub base_account: core::option::Option<BaseAccount>,
    #[prost(string, tag = "2")]
    pub code_hash: prost::alloc::string::String,
}
