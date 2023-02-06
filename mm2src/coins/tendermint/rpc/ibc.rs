use common::HttpStatusCode;
use mm2_err_handle::prelude::MmError;
use mm2_number::BigDecimal;

pub type IBCChainRegistriesResult = Result<IBCChainRegistriesResponse, MmError<IBCChainsRequestError>>;
pub type IBCTransferChannelsResult = Result<IBCTransferChannelsResponse, MmError<IBCTransferChannelsRequestError>>;

// Global constants for interacting with https://github.com/cosmos/chain-registry repository
// using `mm2_git` crate.
pub(crate) const CHAIN_REGISTRY_REPO_OWNER: &str = "cosmos";
pub(crate) const CHAIN_REGISTRY_REPO_NAME: &str = "chain-registry";
pub(crate) const CHAIN_REGISTRY_BRANCH: &str = "master";
pub(crate) const CHAIN_REGISTRY_IBC_DIR_NAME: &str = "_IBC";

#[derive(Clone, Deserialize)]
pub struct IBCWithdrawRequest {
    pub(crate) ibc_source_channel: String,
    pub(crate) coin: String,
    pub(crate) to: String,
    #[serde(default)]
    pub(crate) amount: BigDecimal,
    #[serde(default)]
    pub(crate) max: bool,
    pub(crate) memo: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct IBCTransferChannelsRequest {
    pub(crate) coin: String,
    pub(crate) destination_chain_registry_name: String,
}

#[derive(Clone, Serialize)]
pub struct IBCTransferChannelsResponse {
    pub(crate) ibc_transfer_channels: Vec<IBCTransferChannel>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct IBCTransferChannel {
    pub(crate) channel_id: String,
    pub(crate) ordering: String,
    pub(crate) version: String,
    pub(crate) tags: Option<IBCTransferChannelTag>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct IBCTransferChannelTag {
    pub(crate) status: String,
    pub(crate) preferred: bool,
    pub(crate) dex: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct IBCChainRegistriesResponse {
    pub(crate) chain_registry_list: Vec<String>,
}

#[derive(Clone, Debug, Display, Serialize, SerializeErrorType, PartialEq)]
#[serde(tag = "error_type", content = "error_data")]
pub enum IBCTransferChannelsRequestError {
    #[display(fmt = "No such coin {}", _0)]
    NoSuchCoin(String),
    #[display(
        fmt = "Only tendermint based coins are allowed for `ibc_transfer_channels` operation. Current coin: {}",
        _0
    )]
    UnsupportedCoin(String),
    #[display(fmt = "Could not find '{}' registry source.", _0)]
    RegistrySourceCouldNotFound(String),
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

#[derive(Clone, Debug, Display, Serialize, SerializeErrorType, PartialEq)]
#[serde(tag = "error_type", content = "error_data")]
pub enum IBCChainsRequestError {
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

impl HttpStatusCode for IBCChainsRequestError {
    fn status_code(&self) -> common::StatusCode {
        match self {
            IBCChainsRequestError::Transport(_) => common::StatusCode::SERVICE_UNAVAILABLE,
            IBCChainsRequestError::InternalError(_) => common::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl HttpStatusCode for IBCTransferChannelsRequestError {
    fn status_code(&self) -> common::StatusCode {
        match self {
            IBCTransferChannelsRequestError::UnsupportedCoin(_) | IBCTransferChannelsRequestError::NoSuchCoin(_) => {
                common::StatusCode::BAD_REQUEST
            },
            IBCTransferChannelsRequestError::RegistrySourceCouldNotFound(_) => common::StatusCode::NOT_FOUND,
            IBCTransferChannelsRequestError::Transport(_) => common::StatusCode::SERVICE_UNAVAILABLE,
            IBCTransferChannelsRequestError::InternalError(_) => common::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
