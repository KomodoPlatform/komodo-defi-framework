use common::{HttpStatusCode, PagingOptions, StatusCode};
use cosmrs::staking::{Commission, Description, Validator};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;

use crate::{lp_coinfind_or_err, tendermint::TendermintCoinRpcError, MmCoinEnum};

#[derive(Default, Deserialize)]
pub(crate) enum ValidatorStatus {
    All,
    #[default]
    Bonded,
    Unbonded,
}

impl ToString for ValidatorStatus {
    fn to_string(&self) -> String {
        match self {
            ValidatorStatus::All => String::default(),
            ValidatorStatus::Bonded => "BOND_STATUS_BONDED".into(),
            ValidatorStatus::Unbonded => "BOND_STATUS_UNBONDED".into(),
        }
    }
}

#[derive(Deserialize)]
pub struct ValidatorsRPC {
    #[serde(rename = "ticker")]
    coin: String,
    #[serde(flatten)]
    paging: PagingOptions,
    #[serde(default)]
    filter_by_status: ValidatorStatus,
}

#[derive(Clone, Serialize)]
pub struct ValidatorsRPCResponse {
    validators: Vec<serde_json::Value>,
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
            ValidatorsRPCError::CoinNotFound { .. } => StatusCode::NOT_FOUND,
            ValidatorsRPCError::UnexpectedCoinType { .. } => StatusCode::BAD_REQUEST,
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

pub async fn validators_rpc(
    ctx: MmArc,
    req: ValidatorsRPC,
) -> Result<ValidatorsRPCResponse, MmError<ValidatorsRPCError>> {
    fn maybe_jsonize_description(description: Option<Description>) -> Option<serde_json::Value> {
        description.map(|d| {
            json!({
                "moniker": d.moniker,
                "identity": d.identity,
                "website": d.website,
                "security_contact": d.security_contact,
                "details": d.details,
            })
        })
    }

    fn maybe_jsonize_commission(commission: Option<Commission>) -> Option<serde_json::Value> {
        commission.map(|c| {
            let rates = c.commission_rates.map(|cr| {
                json!({
                    "rate": cr.rate,
                    "max_rate": cr.max_rate,
                    "max_change_rate": cr.max_change_rate
                })
            });

            json!({
                "commission_rates": rates,
                "update_time": c.update_time
            })
        })
    }

    fn jsonize_validator(v: Validator) -> serde_json::Value {
        json!({
            "operator_address": v.operator_address,
            "consensus_pubkey": v.consensus_pubkey,
            "jailed": v.jailed,
            "status": v.status,
            "tokens": v.tokens,
            "delegator_shares": v.delegator_shares,
            "description": maybe_jsonize_description(v.description),
            "unbonding_height": v.unbonding_height,
            "unbonding_time": v.unbonding_time,
            "commission": maybe_jsonize_commission(v.commission),
            "min_self_delegation": v.min_self_delegation,
        })
    }

    let validators = match lp_coinfind_or_err(&ctx, &req.coin).await {
        Ok(MmCoinEnum::Tendermint(coin)) => coin.validators_list(req.filter_by_status, req.paging).await?,
        Ok(MmCoinEnum::TendermintToken(token)) => {
            token
                .platform_coin
                .validators_list(req.filter_by_status, req.paging)
                .await?
        },
        Ok(_) => return MmError::err(ValidatorsRPCError::UnexpectedCoinType { ticker: req.coin }),
        Err(_) => return MmError::err(ValidatorsRPCError::UnexpectedCoinType { ticker: req.coin }),
    };

    Ok(ValidatorsRPCResponse {
        validators: validators.into_iter().map(jsonize_validator).collect(),
    })
}
