use common::HttpStatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;

use crate::{lp_coinfind_or_err, MmCoinEnum};

pub type IBCTransferChannelsResult = Result<IBCTransferChannelsResponse, MmError<IBCTransferChannelsRequestError>>;

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

pub async fn ibc_transfer_channels(ctx: MmArc, req: IBCTransferChannelsRequest) -> IBCTransferChannelsResult {
    let coin = lp_coinfind_or_err(&ctx, &req.coin)
        .await
        .map_err(|_| IBCTransferChannelsRequestError::NoSuchCoin(req.coin.clone()))?;

    match coin {
        MmCoinEnum::Tendermint(coin) => coin.get_ibc_transfer_channels(req).await,
        MmCoinEnum::TendermintToken(token) => token.platform_coin.get_ibc_transfer_channels(req).await,
        _ => MmError::err(IBCTransferChannelsRequestError::UnsupportedCoin(req.coin)),
    }
}
