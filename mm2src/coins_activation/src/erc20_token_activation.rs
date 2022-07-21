use crate::{prelude::TryPlatformCoinFromMmCoinEnum,
            token::{EnableTokenError, TokenActivationOps, TokenProtocolParams}};
use async_trait::async_trait;
use coins::{coin_conf,
            eth::{eth_coin_from_conf_and_request_v2, EthActivationV2Error, EthActivationV2Request, EthCoin},
            CoinBalance, CoinProtocol, MarketCoinOps, MmCoinEnum};
use common::Future01CompatExt;
use crypto::CryptoCtx;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::MmError;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct Erc20InitResult {
    balances: HashMap<String, CoinBalance>,
    pubkey: String,
    platform_coin: String,
}

impl TokenProtocolParams for CoinProtocol {
    fn platform_coin_ticker(&self) -> &str {
        match self {
            CoinProtocol::ERC20 { platform, .. } => platform,
            _ => "ETH",
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

impl From<EthActivationV2Error> for EnableTokenError {
    fn from(err: EthActivationV2Error) -> Self {
        match err {
            EthActivationV2Error::InvalidPayload(e) => EnableTokenError::InvalidPayload(e),
            EthActivationV2Error::ActivationFailed { ticker, error } => {
                EnableTokenError::Internal(format!("{} activation is failed. {}", ticker, error))
            },
            EthActivationV2Error::AtLeastOneNodeRequired(e)
            | EthActivationV2Error::UnreachableNodes(e)
            | EthActivationV2Error::CouldNotFetchBalance(e) => EnableTokenError::Transport(e),
            EthActivationV2Error::InternalError(e) => EnableTokenError::Internal(e),
        }
    }
}

#[async_trait]
impl TokenActivationOps for EthCoin {
    type PlatformCoin = EthCoin;
    type ActivationParams = EthActivationV2Request;
    type ProtocolInfo = CoinProtocol;
    type ActivationResult = Erc20InitResult;
    type ActivationError = EthActivationV2Error;

    async fn enable_token(
        ticker: String,
        platform_coin: Self::PlatformCoin,
        activation_params: Self::ActivationParams,
        protocol_conf: Self::ProtocolInfo,
    ) -> Result<(Self, Self::ActivationResult), MmError<Self::ActivationError>> {
        let ctx = MmArc::from_weak(&platform_coin.ctx())
            .ok_or("No context")
            .map_err(|e| EthActivationV2Error::InternalError(e.to_string()))?;

        let secret = CryptoCtx::from_ctx(&ctx)
            .map_err(|e| EthActivationV2Error::InternalError(e.to_string()))?
            .iguana_ctx()
            .secp256k1_privkey_bytes()
            .to_vec();

        let coins_en = coin_conf(&ctx, &ticker);
        let token: EthCoin = eth_coin_from_conf_and_request_v2(
            &ctx,
            &ticker,
            &coins_en,
            activation_params.clone(),
            &secret,
            protocol_conf,
        )
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
