use coins::{lp_coinfind_or_err, CoinFindError};
use common::HttpStatusCode;
use derive_more::Display;
use http::StatusCode;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::{BigDecimal, MmNumberMultiRepr};
use ser_error_derive::SerializeErrorType;

use crate::mm2::lp_swap::{get_max_maker_vol, CheckBalanceError, CoinVolumeInfo};

#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum MaxMakerVolRpcError {
    #[display(
        fmt = "Not enough {} for swap: available {}, required at least {}, locked by swaps {:?}",
        coin,
        available,
        required,
        locked_by_swaps
    )]
    NotSufficientBalance {
        coin: String,
        available: BigDecimal,
        required: BigDecimal,
        #[serde(skip_serializing_if = "Option::is_none")]
        locked_by_swaps: Option<BigDecimal>,
    },
    #[display(
        fmt = "Not enough base coin {} balance for swap: available {}, required at least {}, locked by swaps {:?}",
        coin,
        available,
        required,
        locked_by_swaps
    )]
    NotSufficientBaseCoinBalance {
        coin: String,
        available: BigDecimal,
        required: BigDecimal,
        #[serde(skip_serializing_if = "Option::is_none")]
        locked_by_swaps: Option<BigDecimal>,
    },
    #[display(
        fmt = "The volume {} of the {} coin less than minimum transaction amount {}",
        volume,
        coin,
        threshold
    )]
    VolumeTooLow {
        coin: String,
        volume: BigDecimal,
        threshold: BigDecimal,
    },
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(fmt = "Coin {} is wallet only", coin)]
    CoinIsWalletOnly { coin: String },
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

impl From<CoinFindError> for MaxMakerVolRpcError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => MaxMakerVolRpcError::NoSuchCoin { coin },
        }
    }
}

impl From<CheckBalanceError> for MaxMakerVolRpcError {
    fn from(e: CheckBalanceError) -> Self {
        match e {
            CheckBalanceError::NotSufficientBalance {
                coin,
                available,
                required,
                locked_by_swaps,
            } => MaxMakerVolRpcError::NotSufficientBalance {
                coin,
                available,
                required,
                locked_by_swaps,
            },
            CheckBalanceError::NotSufficientBaseCoinBalance {
                coin,
                available,
                required,
                locked_by_swaps,
            } => MaxMakerVolRpcError::NotSufficientBaseCoinBalance {
                coin,
                available,
                required,
                locked_by_swaps,
            },
            CheckBalanceError::VolumeTooLow {
                coin,
                volume,
                threshold,
            } => MaxMakerVolRpcError::VolumeTooLow {
                coin,
                volume,
                threshold,
            },
            CheckBalanceError::Transport(transport) => MaxMakerVolRpcError::Transport(transport),
            CheckBalanceError::InternalError(internal) => MaxMakerVolRpcError::InternalError(internal),
        }
    }
}

impl HttpStatusCode for MaxMakerVolRpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            MaxMakerVolRpcError::NotSufficientBalance { .. }
            | MaxMakerVolRpcError::NotSufficientBaseCoinBalance { .. }
            | MaxMakerVolRpcError::VolumeTooLow { .. }
            | MaxMakerVolRpcError::NoSuchCoin { .. }
            | MaxMakerVolRpcError::CoinIsWalletOnly { .. } => StatusCode::BAD_REQUEST,
            MaxMakerVolRpcError::Transport(_) | MaxMakerVolRpcError::InternalError(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
        }
    }
}

#[derive(Deserialize)]
pub struct MaxMakerVolRequest {
    coin: String,
}

#[derive(Debug, Serialize)]
pub struct MaxMakerVolResponse {
    coin: String,
    volume: MmNumberMultiRepr,
    balance: MmNumberMultiRepr,
    locked_by_swaps: MmNumberMultiRepr,
}

pub async fn max_maker_vol(ctx: MmArc, req: MaxMakerVolRequest) -> MmResult<MaxMakerVolResponse, MaxMakerVolRpcError> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    if coin.wallet_only(&ctx) {
        return MmError::err(MaxMakerVolRpcError::CoinIsWalletOnly { coin: req.coin });
    }
    let CoinVolumeInfo {
        volume,
        balance,
        locked_by_swaps,
    } = get_max_maker_vol(&ctx, &coin).await?;
    Ok(MaxMakerVolResponse {
        coin: req.coin,
        volume: MmNumberMultiRepr::from(volume),
        balance: MmNumberMultiRepr::from(balance),
        locked_by_swaps: MmNumberMultiRepr::from(locked_by_swaps),
    })
}
