use common::HttpStatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;

use crate::tendermint;

pub type IBCChainRegistriesResult = Result<IBCChainRegistriesResponse, MmError<IBCChainsRequestError>>;

#[derive(Clone, Serialize)]
pub struct IBCChainRegistriesResponse {
    pub(crate) chain_registry_list: Vec<String>,
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

#[inline(always)]
pub async fn ibc_chains(_ctx: MmArc, _req: serde_json::Value) -> IBCChainRegistriesResult {
    tendermint::get_ibc_chain_list().await
}
