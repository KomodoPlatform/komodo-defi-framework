use crate::{prelude::TryPlatformCoinFromMmCoinEnum,
            token::{EnableTokenError, TokenActivationOps, TokenProtocolParams}};
use async_trait::async_trait;
use coins::{tendermint::{TendermintCoin, TendermintToken, TendermintTokenActivationParams, TendermintTokenInitError,
                         TendermintTokenProtocolInfo},
            CoinBalance, MarketCoinOps, MmCoinEnum};
use common::Future01CompatExt;
use mm2_err_handle::prelude::{MapMmError, MmError};
use serde::Serialize;
use std::collections::HashMap;

impl From<TendermintTokenInitError> for EnableTokenError {
    fn from(err: TendermintTokenInitError) -> Self {
        match err {
            TendermintTokenInitError::InvalidDenom(e) => EnableTokenError::InvalidConfig(e),
            TendermintTokenInitError::MyAddressError(e) | TendermintTokenInitError::Internal(e) => {
                EnableTokenError::Internal(e)
            },
            TendermintTokenInitError::CouldNotFetchBalance(e) => EnableTokenError::CouldNotFetchBalance(e),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TendermintTokenInitResult {
    balances: HashMap<String, CoinBalance>,
    platform_coin: String,
}

impl TryPlatformCoinFromMmCoinEnum for TendermintCoin {
    fn try_from_mm_coin(coin: MmCoinEnum) -> Option<Self>
    where
        Self: Sized,
    {
        match coin {
            MmCoinEnum::Tendermint(coin) => Some(coin),
            _ => None,
        }
    }
}

impl TokenProtocolParams for TendermintTokenProtocolInfo {
    fn platform_coin_ticker(&self) -> &str { &self.platform }
}

#[async_trait]
impl TokenActivationOps for TendermintToken {
    type ActivationParams = TendermintTokenActivationParams;
    type ProtocolInfo = TendermintTokenProtocolInfo;
    type ActivationResult = TendermintTokenInitResult;
    type ActivationError = TendermintTokenInitError;

    async fn enable_token(
        ticker: String,
        platform_coin: Self::PlatformCoin,
        _activation_params: Self::ActivationParams,
        protocol_conf: Self::ProtocolInfo,
    ) -> Result<(Self, Self::ActivationResult), MmError<Self::ActivationError>> {
        let token = TendermintToken::new(ticker, platform_coin, protocol_conf.decimals, protocol_conf.denom)?;

        let balance = token
            .my_balance()
            .compat()
            .await
            .mm_err(|e| TendermintTokenInitError::CouldNotFetchBalance(e.to_string()))?;

        let my_address = token.my_address()?;
        let mut balances = HashMap::new();
        balances.insert(my_address, balance);

        let init_result = TendermintTokenInitResult {
            balances,
            platform_coin: token.platform_ticker().into(),
        };

        Ok((token, init_result))
    }
}
