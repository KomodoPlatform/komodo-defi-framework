use common::{HttpStatusCode, PagingOptions, StatusCode};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;

use crate::{lp_coinfind_or_err, tendermint::TendermintCoinRpcError, MmCoinEnum};

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
    coin: String,
    #[serde(flatten)]
    paging: PagingOptions,
    #[serde(default)]
    pub(crate) filter_by_status: ValidatorStatus,
}

#[derive(Clone, Serialize)]
pub struct ValidatorsRPCResponse {
    pub(crate) validators: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Display, Serialize, SerializeErrorType, PartialEq)]
#[serde(tag = "error_type", content = "error_data")]
pub enum ValidatorsRPCError {
    #[display(fmt = "Coin '{ticker}' could not be found in coins configuration.")]
    CoinNotFound { ticker: String },
    #[display(fmt = "'{ticker}' is not a Cosmos coin.")]
    UnexpectedCoinType { ticker: String },
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

impl HttpStatusCode for ValidatorsRPCError {
    fn status_code(&self) -> common::StatusCode {
        match self {
            ValidatorsRPCError::Transport(_) => StatusCode::SERVICE_UNAVAILABLE,
            ValidatorsRPCError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ValidatorsRPCError::UnexpectedCoinType { .. } | ValidatorsRPCError::CoinNotFound { .. } => {
                StatusCode::BAD_REQUEST
            },
        }
    }
}

impl From<TendermintCoinRpcError> for ValidatorsRPCError {
    fn from(e: TendermintCoinRpcError) -> Self {
        match e {
            TendermintCoinRpcError::InvalidResponse(e)
            | TendermintCoinRpcError::PerformError(e)
            | TendermintCoinRpcError::RpcClientError(e) => ValidatorsRPCError::Transport(e),
            TendermintCoinRpcError::Prost(e) | TendermintCoinRpcError::InternalError(e) => ValidatorsRPCError::InternalError(e),
            TendermintCoinRpcError::UnexpectedAccountType { .. } => ValidatorsRPCError::InternalError(
                "RPC client got an unexpected error 'TendermintCoinRpcError::UnexpectedAccountType', this isn't normal."
                    .into(),
            ),
        }
    }
}

#[inline(always)]
pub async fn validators_rpc(ctx: MmArc, req: ValidatorsRPC) -> ValidatorsRPCResult {
    let validators = match lp_coinfind_or_err(&ctx, &req.coin).await {
        Ok(MmCoinEnum::Tendermint(coin)) => coin.validators_list(req.filter_by_status, req.paging).await?,
        Ok(MmCoinEnum::TendermintToken(token)) => {
            token
                .platform_coin
                .validators_list(req.filter_by_status, req.paging)
                .await?
        },
        Ok(_) => todo!(),
        Err(_) => todo!(),
    };

    let validators_json = validators
        .into_iter()
        .map(|v| {
            let serializable_description = v.description.map(|d| {
                json!({
                    "moniker": d.moniker,
                    "identity": d.identity,
                    "website": d.website,
                    "security_contact": d.security_contact,
                    "details": d.details,
                })
            });

            let serializable_commission = v.commission.map(|c| {
                let serializable_commission_rates = c.commission_rates.map(|cr| {
                    json!({
                        "rate": cr.rate,
                        "max_rate": cr.max_rate,
                        "max_change_rate": cr.max_change_rate
                    })
                });

                json!({
                    "commission_rates": serializable_commission_rates,
                    "update_time": c.update_time
                })
            });

            json!({
                "operator_address": v.operator_address,
                "consensus_pubkey": v.consensus_pubkey,
                "jailed": v.jailed,
                "status": v.status,
                "tokens": v.tokens,
                "delegator_shares": v.delegator_shares,
                "description": serializable_description,
                "unbonding_height": v.unbonding_height,
                "unbonding_time": v.unbonding_time,
                "commission": serializable_commission,
                "min_self_delegation": v.min_self_delegation,
            })
        })
        .collect();

    Ok(ValidatorsRPCResponse {
        validators: validators_json,
    })
}
