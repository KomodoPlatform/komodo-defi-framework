use std::collections::HashMap;

use async_trait::async_trait;
use coins::coin_errors::MyAddressError;
use coins::solana::spl::{SplProtocolConf, SplTokenCreationError};
use coins::{BalanceError, CoinBalance, CoinProtocol, MarketCoinOps, MmCoinEnum, SolanaCoin, SplToken};
use common::Future01CompatExt;
use mm2_err_handle::prelude::*;
use serde_derive::{Deserialize, Serialize};

use crate::prelude::{TryFromCoinProtocol, TryPlatformCoinFromMmCoinEnum};
use crate::token::{EnableTokenError, TokenActivationOps, TokenProtocolParams};

impl TryPlatformCoinFromMmCoinEnum for SolanaCoin {
    fn try_from_mm_coin(coin: MmCoinEnum) -> Option<Self>
    where
        Self: Sized,
    {
        match coin {
            MmCoinEnum::SolanaCoin(coin) => Some(coin),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct SplActivationRequest {}

impl TryFromCoinProtocol for SplProtocolConf {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::SPLTOKEN {
                platform,
                token_contract_address,
                decimals,
            } => Ok(SplProtocolConf {
                platform_coin_ticker: platform,
                decimals,
                token_contract_address,
            }),
            proto => MmError::err(proto),
        }
    }
}

impl TokenProtocolParams for SplProtocolConf {
    fn platform_coin_ticker(&self) -> &str { &self.platform_coin_ticker }
}

#[derive(Debug, Serialize)]
pub struct SplInitResult {
    balances: HashMap<String, CoinBalance>,
    token_contract_address: String,
    platform_coin: String,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum SplInitError {
    GetBalanceError(BalanceError),
    TokenCreationFailed(SplTokenCreationError),
    MyAddressError(String),
}

impl From<MyAddressError> for SplInitError {
    fn from(err: MyAddressError) -> Self { Self::MyAddressError(err.to_string()) }
}

impl From<SplTokenCreationError> for SplInitError {
    fn from(e: SplTokenCreationError) -> Self { SplInitError::TokenCreationFailed(e) }
}

impl From<SplInitError> for EnableTokenError {
    fn from(err: SplInitError) -> Self {
        match err {
            SplInitError::GetBalanceError(rpc_err) => rpc_err.into(),
            SplInitError::TokenCreationFailed(e) => EnableTokenError::Internal(format! {"{:?}", e}),
            SplInitError::MyAddressError(e) => EnableTokenError::Internal(e),
        }
    }
}

#[async_trait]
impl TokenActivationOps for SplToken {
    type ActivationParams = SplActivationRequest;
    type ProtocolInfo = SplProtocolConf;
    type ActivationResult = SplInitResult;
    type ActivationError = SplInitError;

    async fn enable_token(
        ticker: String,
        platform_coin: Self::PlatformCoin,
        _activation_params: Self::ActivationParams,
        protocol_conf: Self::ProtocolInfo,
    ) -> Result<(Self, Self::ActivationResult), MmError<Self::ActivationError>> {
        let token = Self::new(
            protocol_conf.decimals,
            ticker,
            protocol_conf.token_contract_address,
            platform_coin,
        )?;
        let balance = token
            .my_balance()
            .compat()
            .await
            .map_err(|e| SplInitError::GetBalanceError(e.into_inner()))?;
        let my_address = token.my_address()?;
        let mut balances = HashMap::new();
        balances.insert(my_address, balance);
        let init_result = SplInitResult {
            balances,
            token_contract_address: token.conf.token_contract_address.to_string(),
            platform_coin: token.platform_coin.ticker().to_owned(),
        };
        Ok((token, init_result))
    }
}
