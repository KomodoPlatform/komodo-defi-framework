use super::taker_swap::MaxTakerVolumeLessThanDust;
use super::{get_locked_amount, get_locked_amount_by_other_swaps};
use bigdecimal::BigDecimal;
use coins::{BalanceError, MmCoinEnum, TradeFee, TradePreimageError};
use common::log::debug;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::mm_number::MmNumber;
use derive_more::Display;
use futures::compat::Future01CompatExt;
use uuid::Uuid;

pub type CheckBalanceResult<T> = Result<T, MmError<CheckBalanceError>>;

/// Check the coin balance before the swap has started.
///
/// `swap_uuid` is used if our swap is running already and we should except this swap locked amount from the following calculations.
pub async fn check_my_coin_balance_for_swap(
    ctx: &MmArc,
    coin: &MmCoinEnum,
    swap_uuid: Option<&Uuid>,
    volume: MmNumber,
    mut trade_fee: TradeFee,
    taker_fee: Option<TakerFeeAdditionalInfo>,
) -> CheckBalanceResult<()> {
    let ticker = coin.ticker();
    debug!("Check my_coin '{}' balance for swap", ticker);
    let balance: MmNumber = coin.my_spendable_balance().compat().await?.into();

    let locked = match swap_uuid {
        Some(u) => get_locked_amount_by_other_swaps(ctx, u, ticker),
        None => get_locked_amount(ctx, ticker),
    };

    let dex_fee = match taker_fee {
        Some(TakerFeeAdditionalInfo {
            dex_fee,
            fee_to_send_dex_fee,
        }) => {
            if fee_to_send_dex_fee.coin != trade_fee.coin {
                let err = format!(
                    "trade_fee {:?} and fee_to_send_dex_fee {:?} coins are expected to be the same",
                    trade_fee.coin, fee_to_send_dex_fee.coin
                );
                return MmError::err(CheckBalanceError::InternalError(err));
            }
            // increase `trade_fee` by the `fee_to_send_dex_fee`
            trade_fee.amount += fee_to_send_dex_fee.amount;
            dex_fee
        },
        None => MmNumber::from(0),
    };

    let total_trade_fee = if ticker == trade_fee.coin {
        trade_fee.amount
    } else {
        let base_coin_balance: MmNumber = coin.base_coin_balance().compat().await?.into();
        check_base_coin_balance_for_swap(ctx, &base_coin_balance, trade_fee, swap_uuid).await?;
        MmNumber::from(0)
    };

    debug!(
        "{} balance {:?}, locked {:?}, volume {:?}, fee {:?}, dex_fee {:?}",
        ticker,
        balance.to_fraction(),
        locked.to_fraction(),
        volume.to_fraction(),
        total_trade_fee.to_fraction(),
        dex_fee.to_fraction()
    );

    let required = volume + total_trade_fee + dex_fee;
    let available = &balance - &locked;

    if available < required {
        return MmError::err(CheckBalanceError::NotSufficientBalance {
            coin: ticker.to_owned(),
            available: available.to_decimal(),
            required: required.to_decimal(),
            locked_by_swaps: Some(locked.to_decimal()),
        });
    }

    Ok(())
}

pub async fn check_other_coin_balance_for_swap(
    ctx: &MmArc,
    coin: &MmCoinEnum,
    swap_uuid: Option<&Uuid>,
    trade_fee: TradeFee,
) -> CheckBalanceResult<()> {
    if trade_fee.paid_from_trading_vol {
        return Ok(());
    }
    let ticker = coin.ticker();
    debug!("Check other_coin '{}' balance for swap", ticker);
    let balance: MmNumber = coin.my_spendable_balance().compat().await?.into();

    let locked = match swap_uuid {
        Some(u) => get_locked_amount_by_other_swaps(ctx, u, ticker),
        None => get_locked_amount(ctx, ticker),
    };

    if ticker == trade_fee.coin {
        let available = &balance - &locked;
        let required = trade_fee.amount;
        debug!(
            "{} balance {:?}, locked {:?}, required {:?}",
            ticker,
            balance.to_fraction(),
            locked.to_fraction(),
            required.to_fraction(),
        );
        if available < required {
            return MmError::err(CheckBalanceError::NotSufficientBalance {
                coin: ticker.to_owned(),
                available: available.to_decimal(),
                required: required.to_decimal(),
                locked_by_swaps: Some(locked.to_decimal()),
            });
        }
    } else {
        let base_coin_balance: MmNumber = coin.base_coin_balance().compat().await?.into();
        check_base_coin_balance_for_swap(ctx, &base_coin_balance, trade_fee, swap_uuid).await?;
    }

    Ok(())
}

pub async fn check_base_coin_balance_for_swap(
    ctx: &MmArc,
    balance: &MmNumber,
    trade_fee: TradeFee,
    swap_uuid: Option<&Uuid>,
) -> CheckBalanceResult<()> {
    let ticker = trade_fee.coin.as_str();
    let trade_fee_fraction = trade_fee.amount.to_fraction();
    debug!(
        "Check if the base coin '{}' has sufficient balance to pay the trade fee {:?}",
        ticker, trade_fee_fraction
    );

    let required = trade_fee.amount;
    let locked = match swap_uuid {
        Some(uuid) => get_locked_amount_by_other_swaps(ctx, uuid, ticker),
        None => get_locked_amount(ctx, ticker),
    };
    let available = balance - &locked;

    debug!(
        "{} balance {:?}, locked {:?}",
        ticker,
        balance.to_fraction(),
        locked.to_fraction()
    );
    if available < required {
        MmError::err(CheckBalanceError::NotSufficientBaseCoinBalance {
            coin: ticker.to_owned(),
            available: available.to_decimal(),
            required: required.to_decimal(),
            locked_by_swaps: Some(locked.to_decimal()),
        })
    } else {
        Ok(())
    }
}

pub struct TakerFeeAdditionalInfo {
    pub dex_fee: MmNumber,
    pub fee_to_send_dex_fee: TradeFee,
}

#[derive(Debug, Display)]
pub enum CheckBalanceError {
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
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

impl From<BalanceError> for CheckBalanceError {
    fn from(e: BalanceError) -> Self {
        match e {
            BalanceError::Transport(transport) | BalanceError::InvalidResponse(transport) => {
                CheckBalanceError::Transport(transport)
            },
            e @ BalanceError::UnexpectedDerivationMethod(_) | e @ BalanceError::WalletStorageError(_) => {
                CheckBalanceError::InternalError(e.to_string())
            },
            BalanceError::Internal(internal) => CheckBalanceError::InternalError(internal),
        }
    }
}

impl CheckBalanceError {
    pub fn not_sufficient_balance(&self) -> bool {
        matches!(
            self,
            CheckBalanceError::NotSufficientBalance { .. } | CheckBalanceError::NotSufficientBaseCoinBalance { .. }
        )
    }

    /// Construct [`CheckBalanceError`] from [`coins::TradePreimageError`] using the additional `ticker` argument.
    /// `ticker` is used to identify whether the `NotSufficientBalance` or `NotSufficientBaseCoinBalance` has occurred.
    pub fn from_trade_preimage_error(trade_preimage_err: TradePreimageError, ticker: &str) -> CheckBalanceError {
        match trade_preimage_err {
            TradePreimageError::NotSufficientBalance {
                coin,
                available,
                required,
            } => {
                if coin == ticker {
                    CheckBalanceError::NotSufficientBalance {
                        coin,
                        available,
                        locked_by_swaps: None,
                        required,
                    }
                } else {
                    CheckBalanceError::NotSufficientBaseCoinBalance {
                        coin,
                        available,
                        locked_by_swaps: None,
                        required,
                    }
                }
            },
            TradePreimageError::AmountIsTooSmall { amount, threshold } => CheckBalanceError::VolumeTooLow {
                coin: ticker.to_owned(),
                volume: amount,
                threshold,
            },
            TradePreimageError::Transport(transport) => CheckBalanceError::Transport(transport),
            TradePreimageError::InternalError(internal) => CheckBalanceError::InternalError(internal),
        }
    }

    pub fn from_max_taker_vol_error(
        max_vol_err: MaxTakerVolumeLessThanDust,
        coin: String,
        locked_by_swaps: BigDecimal,
    ) -> CheckBalanceError {
        CheckBalanceError::NotSufficientBalance {
            coin,
            available: max_vol_err.max_vol.to_decimal(),
            required: max_vol_err.min_tx_amount.to_decimal(),
            locked_by_swaps: Some(locked_by_swaps),
        }
    }
}
