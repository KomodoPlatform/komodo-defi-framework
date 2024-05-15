use common::HttpStatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;

use crate::{coin_conf, tendermint::get_ibc_transfer_channels};

pub type IBCTransferChannelsResult = Result<IBCTransferChannelsResponse, MmError<IBCTransferChannelsRequestError>>;

#[derive(Clone, Deserialize)]
pub struct IBCTransferChannelsRequest {
    pub(crate) source_coin: String,
    pub(crate) destination_coin: String,
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
    #[display(
        fmt = "'chain_registry_name' was not found in coins configuration for '{}' prefix. Either update the coins configuration or use 'ibc_source_channel' in the request.",
        _0
    )]
    RegistryNameIsMissing(String),
    #[display(fmt = "Could not find '{}' registry source.", _0)]
    RegistrySourceCouldNotFound(String),
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Could not found channel for '{}'.", _0)]
    CouldNotFindChannel(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

impl HttpStatusCode for IBCTransferChannelsRequestError {
    fn status_code(&self) -> common::StatusCode {
        match self {
            IBCTransferChannelsRequestError::UnsupportedCoin(_) | IBCTransferChannelsRequestError::NoSuchCoin(_) => {
                common::StatusCode::BAD_REQUEST
            },
            IBCTransferChannelsRequestError::CouldNotFindChannel(_)
            | IBCTransferChannelsRequestError::RegistryNameIsMissing(_)
            | IBCTransferChannelsRequestError::RegistrySourceCouldNotFound(_) => common::StatusCode::NOT_FOUND,
            IBCTransferChannelsRequestError::Transport(_) => common::StatusCode::SERVICE_UNAVAILABLE,
            IBCTransferChannelsRequestError::InternalError(_) => common::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub async fn ibc_transfer_channels(ctx: MmArc, req: IBCTransferChannelsRequest) -> IBCTransferChannelsResult {
    let source_coin_conf = coin_conf(&ctx, &req.source_coin);
    let source_registry_name = source_coin_conf
        .get("protocol")
        .unwrap_or(&serde_json::Value::Null)
        .get("protocol_data")
        .unwrap_or(&serde_json::Value::Null)
        .get("chain_registry_name")
        .map(|t| t.as_str().unwrap_or_default().to_owned());

    let Some(source_registry_name) = source_registry_name else {
        return MmError::err(IBCTransferChannelsRequestError::RegistryNameIsMissing(req.source_coin));
    };

    let destination_coin_conf = coin_conf(&ctx, &req.destination_coin);
    let destination_registry_name = destination_coin_conf
        .get("protocol")
        .unwrap_or(&serde_json::Value::Null)
        .get("protocol_data")
        .unwrap_or(&serde_json::Value::Null)
        .get("chain_registry_name")
        .map(|t| t.as_str().unwrap_or_default().to_owned());

    let Some(destination_registry_name) = destination_registry_name else {
        return MmError::err(IBCTransferChannelsRequestError::RegistryNameIsMissing(
            req.destination_coin,
        ));
    };

    get_ibc_transfer_channels(source_registry_name, destination_registry_name).await
}
