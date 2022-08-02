use crate::{prelude::{TryFromCoinProtocol, TryPlatformCoinFromMmCoinEnum},
            token::{EnableTokenError, TokenActivationOps, TokenProtocolParams}};
use async_trait::async_trait;
use coins::{eth::{v2_activation::{Erc20Protocol, Erc20TokenActivationError, Erc20TokenActivationRequest},
                  valid_addr_from_str, EthCoin},
            CoinBalance, CoinProtocol, MarketCoinOps, MmCoin, MmCoinEnum};
use common::Future01CompatExt;
use mm2_err_handle::prelude::MmError;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct Erc20InitResult {
    balances: HashMap<String, CoinBalance>,
    platform_coin: String,
    token_contract_address: String,
    required_confirmations: u64,
}

impl From<Erc20TokenActivationError> for EnableTokenError {
    fn from(err: Erc20TokenActivationError) -> Self {
        match err {
            Erc20TokenActivationError::InternalError(e) => EnableTokenError::Internal(e),
            Erc20TokenActivationError::CouldNotFetchBalance(e) => EnableTokenError::Transport(e),
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
    type ActivationParams = Erc20TokenActivationRequest;
    type ProtocolInfo = Erc20Protocol;
    type ActivationResult = Erc20InitResult;
    type ActivationError = Erc20TokenActivationError;

    async fn enable_token(
        ticker: String,
        platform_coin: Self::PlatformCoin,
        activation_params: Self::ActivationParams,
        protocol_conf: Self::ProtocolInfo,
    ) -> Result<(Self, Self::ActivationResult), MmError<Self::ActivationError>> {
        let token = platform_coin
            .initialize_erc20_token(activation_params, protocol_conf, ticker)
            .await?;

        let address = token.my_address().map_err(Erc20TokenActivationError::InternalError)?;
        let token_contract_address = token
            .erc20_token_address()
            .ok_or_else(|| Erc20TokenActivationError::InternalError("Token contract address is missing".to_string()))?;

        let balance = token
            .my_balance()
            .compat()
            .await
            .map_err(|e| Erc20TokenActivationError::CouldNotFetchBalance(e.to_string()))?;

        let balances = HashMap::from([(address, balance)]);

        let init_result = Erc20InitResult {
            balances,
            platform_coin: token.platform_ticker().to_owned(),
            required_confirmations: token.required_confirmations(),
            token_contract_address: format!("{:#02x}", token_contract_address),
        };

        Ok((token, init_result))
    }
}
