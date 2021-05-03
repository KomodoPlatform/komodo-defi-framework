use super::check_balance::CheckBalanceError;
use super::{maker_swap_trade_preimage, taker_swap_trade_preimage, MakerTradePreimage, TakerTradePreimage};
use bigdecimal::BigDecimal;
use coins::{is_wallet_only_ticker, lp_coinfind_or_err, BalanceError, CoinFindError, TradeFee, TradePreimageError};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::mm_number::MmNumber;
use common::HttpStatusCode;
use derive_more::Display;
use http::StatusCode;
use std::collections::HashMap;

construct_detailed!(DetailedAmount, amount);
construct_detailed!(DetailedVolume, volume);
construct_detailed!(DetailedRequiredBalance, required_balance);

pub type TradePreimageRpcResult<T> = Result<T, MmError<TradePreimageRpcError>>;

pub async fn trade_preimage_rpc(
    ctx: MmArc,
    req: TradePreimageRequest,
) -> TradePreimageRpcResult<TradePreimageResponse> {
    if is_wallet_only_ticker(&ctx, &req.base) {
        return MmError::err(TradePreimageRpcError::CoinIsWalletOnly { coin: req.base });
    }
    if is_wallet_only_ticker(&ctx, &req.rel) {
        return MmError::err(TradePreimageRpcError::CoinIsWalletOnly { coin: req.rel });
    }

    let base_coin = lp_coinfind_or_err(&ctx, &req.base).await?;
    let rel_coin = lp_coinfind_or_err(&ctx, &req.rel).await?;

    match req.swap_method {
        TradePreimageMethod::SetPrice => maker_swap_trade_preimage(&ctx, req, base_coin, rel_coin)
            .await
            .map(TradePreimageResponse::from),
        TradePreimageMethod::Buy | TradePreimageMethod::Sell => {
            taker_swap_trade_preimage(&ctx, req, base_coin, rel_coin)
                .await
                .map(TradePreimageResponse::from)
        },
    }
}

#[derive(Deserialize)]
pub struct TradePreimageRequest {
    /// The base currency of the request.
    pub base: String,
    /// The rel currency of the request.
    pub rel: String,
    /// The name of the method whose preimage is requested.
    pub swap_method: TradePreimageMethod,
    /// The price in `rel` the user is willing to receive per one unit of the `base` coin.
    #[serde(default)]
    pub price: MmNumber,
    /// The amount the user is willing to trade.
    /// Ignored if `max = true`.
    #[serde(default)]
    pub volume: MmNumber,
    /// Whether to return the maximum available volume for setprice method
    #[serde(default)]
    pub max: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TradePreimageMethod {
    SetPrice,
    Buy,
    Sell,
}

#[derive(Serialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum TradePreimageResponse {
    MakerPreimage {
        base_coin_fee: TradeFeeResponse,
        rel_coin_fee: TradeFeeResponse,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(flatten)]
        volume: Option<DetailedVolume>,
        total_fees: Vec<TotalTradeFeeResponse>,
    },
    TakerPreimage {
        base_coin_fee: TradeFeeResponse,
        rel_coin_fee: TradeFeeResponse,
        taker_fee: TradeFeeResponse,
        fee_to_send_taker_fee: TradeFeeResponse,
        total_fees: Vec<TotalTradeFeeResponse>,
    },
}

impl From<MakerTradePreimage> for TradePreimageResponse {
    fn from(maker: MakerTradePreimage) -> Self {
        let mut total_fees = HashMap::new();

        TradePreimageResponse::accumulate_total_fees(&mut total_fees, maker.base_coin_fee.clone());
        let base_coin_fee = TradeFeeResponse::from(maker.base_coin_fee);

        TradePreimageResponse::accumulate_total_fees(&mut total_fees, maker.rel_coin_fee.clone());
        let rel_coin_fee = TradeFeeResponse::from(maker.rel_coin_fee);

        let total_fees = total_fees
            .into_iter()
            .filter_map(TradePreimageResponse::filter_zero_total_fees)
            .collect();
        let volume = maker.volume.map(DetailedVolume::from);
        TradePreimageResponse::MakerPreimage {
            base_coin_fee,
            rel_coin_fee,
            volume,
            total_fees,
        }
    }
}

impl From<TakerTradePreimage> for TradePreimageResponse {
    fn from(taker: TakerTradePreimage) -> Self {
        let mut total_fees = HashMap::new();

        TradePreimageResponse::accumulate_total_fees(&mut total_fees, taker.base_coin_fee.clone());
        let base_coin_fee = TradeFeeResponse::from(taker.base_coin_fee);

        TradePreimageResponse::accumulate_total_fees(&mut total_fees, taker.rel_coin_fee.clone());
        let rel_coin_fee = TradeFeeResponse::from(taker.rel_coin_fee);

        TradePreimageResponse::accumulate_total_fees(&mut total_fees, taker.taker_fee.clone());
        let taker_fee = TradeFeeResponse::from(taker.taker_fee);

        TradePreimageResponse::accumulate_total_fees(&mut total_fees, taker.fee_to_send_taker_fee.clone());
        let fee_to_send_taker_fee = TradeFeeResponse::from(taker.fee_to_send_taker_fee);

        let total_fees = total_fees
            .into_iter()
            .filter_map(TradePreimageResponse::filter_zero_total_fees)
            .collect();
        TradePreimageResponse::TakerPreimage {
            base_coin_fee,
            rel_coin_fee,
            taker_fee,
            fee_to_send_taker_fee,
            total_fees,
        }
    }
}

impl TradePreimageResponse {
    fn accumulate_total_fees(total_fees: &mut HashMap<String, TotalTradeFee>, fee: TradeFee) {
        use std::collections::hash_map::Entry;
        match total_fees.entry(fee.coin.clone()) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().add_trade_fee(fee.amount, fee.paid_from_trading_vol);
            },
            Entry::Vacant(entry) => {
                entry.insert(fee.into());
            },
        }
    }

    fn filter_zero_total_fees((_coin, fee): (String, TotalTradeFee)) -> Option<TotalTradeFeeResponse> {
        if fee.amount.is_zero() {
            None
        } else {
            Some(TotalTradeFeeResponse::from(fee))
        }
    }
}

/// The extended `coins::TradePreimageError` error.
#[derive(Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum TradePreimageRpcError {
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
    #[display(fmt = "Max volume {} less than minimum transaction amount", volume)]
    MaxVolumeLessThanDust { volume: BigDecimal },
    #[display(fmt = "The volume {} is too small", volume)]
    VolumeIsTooSmall { volume: BigDecimal },
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(fmt = "Coin {} is wallet only", coin)]
    CoinIsWalletOnly { coin: String },
    #[display(fmt = "Incorrect use of the '{}' parameter: {}", param, reason)]
    InvalidParam { param: String, reason: String },
    #[display(fmt = "Expected non-zero 'price'")]
    ZeroPrice,
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

impl HttpStatusCode for TradePreimageRpcError {
    fn status_code(&self) -> StatusCode {
        match self {
            TradePreimageRpcError::NotSufficientBalance { .. }
            | TradePreimageRpcError::NotSufficientBaseCoinBalance { .. }
            | TradePreimageRpcError::MaxVolumeLessThanDust { .. }
            | TradePreimageRpcError::VolumeIsTooSmall { .. }
            | TradePreimageRpcError::NoSuchCoin { .. }
            | TradePreimageRpcError::CoinIsWalletOnly { .. }
            | TradePreimageRpcError::InvalidParam { .. }
            | TradePreimageRpcError::ZeroPrice => StatusCode::BAD_REQUEST,
            TradePreimageRpcError::Transport(_) | TradePreimageRpcError::InternalError(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
        }
    }
}

impl From<BalanceError> for TradePreimageRpcError {
    fn from(e: BalanceError) -> Self {
        match e {
            BalanceError::Transport(transport) | BalanceError::InvalidResponse(transport) => {
                TradePreimageRpcError::Transport(transport)
            },
            BalanceError::Internal(internal) => TradePreimageRpcError::InternalError(internal),
        }
    }
}

impl From<CheckBalanceError> for TradePreimageRpcError {
    fn from(e: CheckBalanceError) -> Self {
        match e {
            CheckBalanceError::NotSufficientBalance {
                coin,
                available,
                required,
                locked_by_swaps,
            } => TradePreimageRpcError::NotSufficientBalance {
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
            } => TradePreimageRpcError::NotSufficientBaseCoinBalance {
                coin,
                available,
                required,
                locked_by_swaps,
            },
            CheckBalanceError::MaxVolumeLessThanDust { volume } => {
                TradePreimageRpcError::MaxVolumeLessThanDust { volume }
            },
            CheckBalanceError::VolumeIsTooSmall { volume } => TradePreimageRpcError::VolumeIsTooSmall { volume },
            CheckBalanceError::Transport(transport) => TradePreimageRpcError::Transport(transport),
            CheckBalanceError::InternalError(internal) => TradePreimageRpcError::InternalError(internal),
        }
    }
}

impl From<CoinFindError> for TradePreimageRpcError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => TradePreimageRpcError::NoSuchCoin { coin },
        }
    }
}

impl TradePreimageRpcError {
    /// Construct [`TradePreimageRpcError`] from [`coins::TradePreimageError`] using the additional `ticker` argument.
    /// `ticker` is used to identify whether the `NotSufficientBalance` or `NotSufficientBaseCoinBalance` has occurred.
    pub fn from_trade_preimage_error(trade_preimage_err: TradePreimageError, ticker: &str) -> TradePreimageRpcError {
        // `CheckBalanceError` has similar variants as `TradePreimageRpcError` and can be obtained from `TradePreimageError`,
        // so avoid unnecessary boilerplate code.
        TradePreimageRpcError::from(CheckBalanceError::from_trade_preimage_error(trade_preimage_err, ticker))
    }
}

#[derive(Clone, Serialize)]
pub struct TradeFeeResponse {
    coin: String,
    #[serde(flatten)]
    amount: DetailedAmount,
    paid_from_trading_vol: bool,
}

impl From<TradeFee> for TradeFeeResponse {
    fn from(orig: TradeFee) -> Self {
        TradeFeeResponse {
            coin: orig.coin,
            amount: DetailedAmount::from(orig.amount),
            paid_from_trading_vol: orig.paid_from_trading_vol,
        }
    }
}

#[derive(Clone)]
struct TotalTradeFee {
    coin: String,
    amount: MmNumber,
    required_balance: MmNumber,
}

impl TotalTradeFee {
    fn add_trade_fee(&mut self, amount: MmNumber, paid_from_trading_vol: bool) {
        self.amount += &amount;
        if !paid_from_trading_vol {
            self.required_balance += amount;
        }
    }
}

impl From<TradeFee> for TotalTradeFee {
    fn from(orig: TradeFee) -> TotalTradeFee {
        let required_balance = if orig.paid_from_trading_vol {
            0.into()
        } else {
            orig.amount.clone()
        };
        TotalTradeFee {
            coin: orig.coin,
            amount: orig.amount,
            required_balance,
        }
    }
}

#[derive(Clone, Serialize)]
pub struct TotalTradeFeeResponse {
    coin: String,
    #[serde(flatten)]
    amount: DetailedAmount,
    #[serde(flatten)]
    required_balance: DetailedRequiredBalance,
}

impl From<TotalTradeFee> for TotalTradeFeeResponse {
    fn from(orig: TotalTradeFee) -> Self {
        TotalTradeFeeResponse {
            coin: orig.coin,
            amount: orig.amount.into(),
            required_balance: orig.required_balance.into(),
        }
    }
}
