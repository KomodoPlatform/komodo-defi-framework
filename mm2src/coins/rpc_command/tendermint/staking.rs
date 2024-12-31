use common::{HttpStatusCode, PagingOptions};
use cosmrs::proto::tendermint::types::Validator;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;

use crate::tendermint;

pub type ValidatorsRPCResult = Result<ValidatorsRPCResponse, MmError<ValidatorsRPCError>>;

#[derive(Default, Deserialize)]
pub enum ValidatorStatus {
    All,
    #[default]
    Active,
    Jailed,
}

impl ToString for ValidatorStatus {
    fn to_string(&self) -> String {
        match self {
            ValidatorStatus::All => "".into(),
            ValidatorStatus::Active => "Bonded".into(),
            ValidatorStatus::Jailed => "Unbonded".into(),
        }
    }
}


#[derive(Deserialize)]
pub struct ValidatorsRPC {
    #[serde(flatten)]
    paging: PagingOptions,
    #[serde(default)]
    pub(crate) filter_by_status: ValidatorStatus,
}

#[derive(Clone, Serialize)]
pub struct ValidatorsRPCResponse {
    pub(crate) chain_registry_list: Vec<Validator>,
}

#[derive(Clone, Debug, Display, Serialize, SerializeErrorType, PartialEq)]
#[serde(tag = "error_type", content = "error_data")]
pub enum ValidatorsRPCError {
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

impl HttpStatusCode for ValidatorsRPCError {
    fn status_code(&self) -> common::StatusCode {
        match self {
            ValidatorsRPCError::Transport(_) => common::StatusCode::SERVICE_UNAVAILABLE,
            ValidatorsRPCError::InternalError(_) => common::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[inline(always)]
pub async fn validators_list_rpc(ctx: MmArc, req: ValidatorsRPC) -> ValidatorsRPCResult {
    //
    todo!()
}
