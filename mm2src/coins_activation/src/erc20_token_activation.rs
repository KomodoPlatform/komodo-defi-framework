use crate::{prelude::{TryFromCoinProtocol, TryPlatformCoinFromMmCoinEnum},
            token::{EnableTokenError, TokenActivationOps, TokenProtocolParams}};
use async_trait::async_trait;
use coins::{eth::{valid_addr_from_str, Erc20Protocol, Erc20TokenRequest, EthActivationV2Error, EthCoin},
            CoinBalance, CoinProtocol, MarketCoinOps, MmCoinEnum};
use common::Future01CompatExt;
use mm2_err_handle::prelude::MmError;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct Erc20InitResult {
    balances: HashMap<String, CoinBalance>,
    pubkey: String,
    platform_coin: String,
}

impl From<EthActivationV2Error> for EnableTokenError {
    fn from(err: EthActivationV2Error) -> Self {
        match err {
            EthActivationV2Error::InvalidPayload(e) => EnableTokenError::InvalidPayload(e),
            EthActivationV2Error::ActivationFailed { ticker, error } => {
                EnableTokenError::Internal(format!("{} activation is failed. {}", ticker, error))
            },
            EthActivationV2Error::AtLeastOneNodeRequired => {
                EnableTokenError::Transport("Enable request for ETH coin must have at least 1 node".to_string())
            },
            EthActivationV2Error::UnreachableNodes(e) | EthActivationV2Error::CouldNotFetchBalance(e) => {
                EnableTokenError::Transport(e)
            },
            EthActivationV2Error::InternalError(e) => EnableTokenError::Internal(e),
        }
    }
}

impl TryPlatformCoinFromMmCoinEnum for EthCoin {
    fn try_from_mm_coin(coin: MmCoinEnum) -> Option<Self>
    where
        Self: Sized,
    {
        match coin {
            MmCoinEnum::EthCoin(coin) => Some(coin),
            _ => None,
        }
    }
}

impl TryFromCoinProtocol for Erc20Protocol {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::ERC20 {
                platform,
                contract_address,
            } => {
                let token_addr = valid_addr_from_str(&contract_address).map_err(|_| CoinProtocol::ERC20 {
                    platform: platform.clone(),
                    contract_address,
                })?;

                Ok(Erc20Protocol { platform, token_addr })
            },
            proto => MmError::err(proto),
        }
    }
}

impl TokenProtocolParams for Erc20Protocol {
    fn platform_coin_ticker(&self) -> &str { &self.platform }
}

#[async_trait]
impl TokenActivationOps for EthCoin {
    type PlatformCoin = EthCoin;
    type ActivationParams = Erc20TokenRequest;
    type ProtocolInfo = Erc20Protocol;
    type ActivationResult = Erc20InitResult;
    type ActivationError = EthActivationV2Error;

    async fn enable_token(
        ticker: String,
        platform_coin: Self::PlatformCoin,
        activation_params: Self::ActivationParams,
        protocol_conf: Self::ProtocolInfo,
    ) -> Result<(Self, Self::ActivationResult), MmError<Self::ActivationError>> {
        let token = platform_coin
            .initialize_erc20_token(activation_params, protocol_conf, ticker)
            .await?;

        let my_address = token.my_address().map_err(EthActivationV2Error::InternalError)?;

        let balance = token
            .my_balance()
            .compat()
            .await
            .map_err(|e| EthActivationV2Error::CouldNotFetchBalance(e.to_string()))?;

        let balances = HashMap::from([(my_address.clone(), balance)]);

        let init_result = Erc20InitResult {
            balances,
            pubkey: my_address,
            platform_coin: token.platform_ticker().to_owned(),
        };

        Ok((token, init_result))
    }
}
