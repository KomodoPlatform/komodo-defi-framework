use super::{coin_conf, lp_coinfind, CoinProtocol};
use crate::qrc20::Qrc20ActivationParams;
use crate::utxo::bch::BchActivationParams;
use crate::utxo::UtxoActivationParams;
use crate::{lp_coinfind_or_err, BalanceError, CoinActivationOps, CoinActivationParamsOps, CoinBalance, CoinsContext,
            MmCoinEnum, TokenCreationError};
use async_trait::async_trait;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::NotSame;
use derive_more::Display;
use ethereum_types::Address;
use futures::compat::Future01CompatExt;
use serde_json::{self as json};
use std::collections::HashMap;
use std::convert::TryFrom;

#[derive(Debug, Deserialize, Serialize)]
pub enum ZcoinActivationMode {
    Native,
    // TODO extend in the future
    Zlite,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "protocol_name", content = "protocol_data")]
pub enum EnableProtocolParams {
    Bch(BchActivationParams),
    SlpToken,
    Eth {
        urls: Vec<String>,
        swap_contract_address: Address,
        fallback_swap_contract: Option<Address>,
        with_tokens: Vec<String>,
    },
    Erc20,
    Utxo(UtxoActivationParams),
    Qrc20(Qrc20ActivationParams),
    Zcoin {
        mode: ZcoinActivationMode,
    },
}

#[derive(Debug, Deserialize)]
pub struct EnableRpcRequest {
    coin: String,
    protocol_params: EnableProtocolParams,
}

#[derive(Debug, Serialize)]
pub struct EnableResult<T> {
    current_block: u64,
    platform_coin_balances: HashMap<String, CoinBalance>,
    token_balances: HashMap<String, HashMap<String, CoinBalance>>,
    resulting_coin_config: T,
}

#[derive(Debug, Serialize)]
pub struct EnableTokenResult<T> {
    balances: HashMap<String, CoinBalance>,
    resulting_token_config: T,
}

#[derive(Debug, Display)]
#[allow(clippy::large_enum_variant)]
pub enum EnableError {
    /// With ticker
    CoinIsAlreadyActivated(String),
    /// With ticker
    CoinConfigNotFound(String),
    /// With ticker
    TokenConfigNotFound(String),
    TokenCreationError(TokenCreationError),
    InvalidCoinProtocolConf(serde_json::Error),
    #[display(
        fmt = "Request {:?} is not compatible with protocol {:?}",
        request,
        protocol_from_conf
    )]
    RequestNotCompatibleWithProtocol {
        request: EnableProtocolParams,
        protocol_from_conf: CoinProtocol,
    },
    GetCurrentBlockError(String),
    GetBalanceError(BalanceError),
}

impl From<TokenCreationError> for EnableError {
    fn from(err: TokenCreationError) -> Self { EnableError::TokenCreationError(err) }
}

impl From<BalanceError> for EnableError {
    fn from(err: BalanceError) -> Self { EnableError::GetBalanceError(err) }
}

pub async fn enable_coin<T>(
    ctx: &MmArc,
    ticker: &str,
    params: T::ActivationParams,
) -> Result<EnableResult<()>, MmError<EnableError>>
where
    T: CoinActivationOps,
    EnableError: From<T::ActivationError>,
    (T::ActivationError, EnableError): NotSame,
{
    if let Ok(Some(_)) = lp_coinfind(&ctx, ticker).await {
        return MmError::err(EnableError::CoinIsAlreadyActivated(ticker.to_owned()));
    }

    let conf = coin_conf(&ctx, ticker);
    if conf.is_null() {
        return MmError::err(EnableError::CoinConfigNotFound(ticker.to_owned()));
    }

    let token_tickers = params.activate_with_tokens();
    let platform_coin = T::activate(ctx, ticker, &conf, params).await?;
    let tokens: Vec<_> = token_tickers
        .into_iter()
        .map(|token| {
            let conf = coin_conf(&ctx, &token);
            if conf.is_null() {
                return MmError::err(EnableError::TokenConfigNotFound(token));
            }
            let token_as_coin = platform_coin.activate_token(&token, &conf)?;
            Ok(token_as_coin)
        })
        .collect::<Result<_, _>>()?;
    let coins_ctx = CoinsContext::from_ctx(ctx).unwrap();
    let mut coins = coins_ctx.coins.lock().await;

    let platform_coin = platform_coin.into();

    let current_block = platform_coin
        .current_block()
        .compat()
        .await
        .map_to_mm(EnableError::GetCurrentBlockError)?;

    let balances = platform_coin.get_balances_with_tokens().compat().await?;

    if coins.contains_key(ticker) {
        return MmError::err(EnableError::CoinIsAlreadyActivated(ticker.to_owned()));
    }

    coins.insert(ticker.to_owned(), platform_coin);
    for token in tokens {
        coins.insert(token.ticker().to_owned(), token);
    }

    Ok(EnableResult {
        current_block,
        platform_coin_balances: balances.platform_coin_balances,
        token_balances: balances.token_balances,
        resulting_coin_config: (),
    })
}

pub async fn enable_coin_with_tokens<T>(
    ctx: &MmArc,
    ticker: &str,
    params: T::ActivationParams,
) -> Result<EnableResult<()>, MmError<EnableError>>
where
    T: CoinActivationOps,
    EnableError: From<T::ActivationError>,
    (T::ActivationError, EnableError): NotSame,
{
    if let Ok(Some(_)) = lp_coinfind(&ctx, ticker).await {
        return MmError::err(EnableError::CoinIsAlreadyActivated(ticker.to_owned()));
    }

    let conf = coin_conf(&ctx, ticker);
    if conf.is_null() {
        return MmError::err(EnableError::CoinConfigNotFound(ticker.to_owned()));
    }

    let token_tickers = params.activate_with_tokens();
    let platform_coin = T::activate(ctx, ticker, &conf, params).await?;
    let tokens: Vec<_> = token_tickers
        .into_iter()
        .map(|token| {
            let conf = coin_conf(&ctx, &token);
            if conf.is_null() {
                return MmError::err(EnableError::TokenConfigNotFound(token));
            }
            let token_as_coin = platform_coin.activate_token(&token, &conf)?;
            Ok(token_as_coin)
        })
        .collect::<Result<_, _>>()?;

    let platform_coin = platform_coin.into();

    let current_block = platform_coin
        .current_block()
        .compat()
        .await
        .map_to_mm(EnableError::GetCurrentBlockError)?;

    let balances = platform_coin.get_balances_with_tokens().compat().await?;

    let coins_ctx = CoinsContext::from_ctx(ctx).unwrap();
    let mut coins = coins_ctx.coins.lock().await;

    if coins.contains_key(ticker) {
        return MmError::err(EnableError::CoinIsAlreadyActivated(ticker.to_owned()));
    }

    coins.insert(ticker.to_owned(), platform_coin);
    for token in tokens {
        coins.insert(token.ticker().to_owned(), token);
    }

    Ok(EnableResult {
        current_block,
        platform_coin_balances: balances.platform_coin_balances,
        token_balances: balances.token_balances,
        resulting_coin_config: (),
    })
}

pub trait TokenActivationParams {
    fn platform_coin_ticker(&self) -> &str;
}

#[async_trait]
pub trait TokenOf: Into<MmCoinEnum> {
    type PlatformCoin: TryFrom<MmCoinEnum, Error: Sized + Into<EnableTokenError<Self::ActivationError>>>;
    type ActivationParams;
    type ProtocolInfo: TokenActivationParams + TryFrom<CoinProtocol>;
    type ActivationResult;
    type ActivationError: NotMmError + Sized + Into<EnableTokenError<Self::ActivationError>>;

    async fn new(
        platform_coin: Self::PlatformCoin,
        activation_params: Self::ActivationParams,
        protocol_conf: Self::ProtocolInfo,
    ) -> Result<(Self, Self::ActivationResult), MmError<Self::ActivationError>>;
}

pub enum EnableTokenError<T: NotMmError> {
    TokenIsAlreadyActivated(String),
    TokenConfigIsNotFound(String),
    InvalidTokenProtocolConf(json::Error),
    TokenCreationError(T),
}

pub async fn enable_token<T>(
    ctx: &MmArc,
    ticker: &str,
    params: T::ActivationParams,
) -> Result<T::ActivationResult, MmError<EnableTokenError<T::ActivationError>>>
where
    T: TokenOf,
{
    if let Ok(Some(_)) = lp_coinfind(&ctx, ticker).await {
        return MmError::err(EnableTokenError::TokenIsAlreadyActivated(ticker.to_owned()));
    }

    let conf = coin_conf(&ctx, ticker);
    if conf.is_null() {
        return MmError::err(EnableTokenError::TokenConfigIsNotFound(ticker.to_owned()));
    }

    let coin_protocol: CoinProtocol =
        json::from_value(conf["protocol"].clone()).map_to_mm(EnableTokenError::InvalidTokenProtocolConf)?;
    let token_protocol = T::ProtocolInfo::try_from(coin_protocol)?;

    let platform_coin = lp_coinfind_or_err(ctx, token_protocol.platform_coin_ticker()).await?;
    let platform_coin = T::PlatformCoin::try_from(platform_coin)?;

    let (token, activation_result) = T::new(platform_coin, params, token_protocol).await?;

    let coins_ctx = CoinsContext::from_ctx(ctx).unwrap();
    let mut coins = coins_ctx.coins.lock().await;

    if coins.contains_key(ticker) {
        return MmError::err(EnableTokenError::TokenIsAlreadyActivated(ticker.to_owned()));
    }

    coins.insert(ticker.to_owned(), token.into());

    Ok(activation_result)
}
