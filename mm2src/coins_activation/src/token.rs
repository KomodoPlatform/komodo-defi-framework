/// Contains token activation traits and their implementations for various coins
///
use crate::prelude::*;
use async_trait::async_trait;
use coins::utxo::rpc_clients::UtxoRpcError;
use coins::{lp_coinfind, lp_coinfind_or_err, BalanceError, CoinProtocol, CoinsContext, MmCoinEnum,
            UnexpectedDerivationMethod};
use common::{HttpStatusCode, StatusCode};
use derive_more::Display;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use ser_error_derive::SerializeErrorType;
use serde_derive::{Deserialize, Serialize};

pub trait TokenProtocolParams {
    fn platform_coin_ticker(&self) -> &str;
}

#[async_trait]
pub trait TokenActivationOps: Into<MmCoinEnum> {
    type PlatformCoin: TryPlatformCoinFromMmCoinEnum;
    type ActivationParams;
    type ProtocolInfo: TokenProtocolParams + TryFromCoinProtocol;
    type ActivationResult;
    type ActivationError: NotMmError;

    async fn enable_token(
        ticker: String,
        platform_coin: Self::PlatformCoin,
        activation_params: Self::ActivationParams,
        protocol_conf: Self::ProtocolInfo,
    ) -> Result<(Self, Self::ActivationResult), MmError<Self::ActivationError>>;
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum EnableTokenError {
    #[display(fmt = "Token {} is already activated", _0)]
    TokenIsAlreadyActivated(String),
    #[display(fmt = "Token {} config is not found", _0)]
    TokenConfigIsNotFound(String),
    #[display(fmt = "Token {} protocol parsing failed: {}", ticker, error)]
    TokenProtocolParseError {
        ticker: String,
        error: String,
    },
    #[display(fmt = "Unexpected token protocol {:?} for {}", protocol, ticker)]
    UnexpectedTokenProtocol {
        ticker: String,
        protocol: CoinProtocol,
    },
    #[display(fmt = "Platform coin {} is not activated", _0)]
    PlatformCoinIsNotActivated(String),
    #[display(fmt = "{} is not a platform coin for token {}", platform_coin_ticker, token_ticker)]
    UnsupportedPlatformCoin {
        platform_coin_ticker: String,
        token_ticker: String,
    },
    #[display(fmt = "{}", _0)]
    UnexpectedDerivationMethod(UnexpectedDerivationMethod),
    CouldNotFetchBalance(String),
    Transport(String),
    Internal(String),
}

impl From<CoinConfWithProtocolError> for EnableTokenError {
    fn from(err: CoinConfWithProtocolError) -> Self {
        match err {
            CoinConfWithProtocolError::ConfigIsNotFound(ticker) => EnableTokenError::TokenConfigIsNotFound(ticker),
            CoinConfWithProtocolError::CoinProtocolParseError { ticker, err } => {
                EnableTokenError::TokenProtocolParseError {
                    ticker,
                    error: err.to_string(),
                }
            },
            CoinConfWithProtocolError::UnexpectedProtocol { ticker, protocol } => {
                EnableTokenError::UnexpectedTokenProtocol { ticker, protocol }
            },
        }
    }
}

impl From<BalanceError> for EnableTokenError {
    fn from(e: BalanceError) -> Self {
        match e {
            BalanceError::Transport(e) | BalanceError::InvalidResponse(e) => EnableTokenError::Transport(e),
            BalanceError::UnexpectedDerivationMethod(e) => EnableTokenError::UnexpectedDerivationMethod(e),
            BalanceError::Internal(e) | BalanceError::WalletStorageError(e) => EnableTokenError::Internal(e),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EnableTokenRequest<T> {
    ticker: String,
    activation_params: T,
}

pub async fn enable_token<Token>(
    ctx: MmArc,
    req: EnableTokenRequest<Token::ActivationParams>,
) -> Result<Token::ActivationResult, MmError<EnableTokenError>>
where
    Token: TokenActivationOps,
    EnableTokenError: From<Token::ActivationError>,
    (Token::ActivationError, EnableTokenError): NotEqual,
{
    if let Ok(Some(_)) = lp_coinfind(&ctx, &req.ticker).await {
        return MmError::err(EnableTokenError::TokenIsAlreadyActivated(req.ticker));
    }

    let (_, token_protocol): (_, Token::ProtocolInfo) = coin_conf_with_protocol(&ctx, &req.ticker)?;

    let platform_coin = lp_coinfind_or_err(&ctx, token_protocol.platform_coin_ticker())
        .await
        .mm_err(|_| EnableTokenError::PlatformCoinIsNotActivated(token_protocol.platform_coin_ticker().to_owned()))?;

    let platform_coin = Token::PlatformCoin::try_from_mm_coin(platform_coin).or_mm_err(|| {
        EnableTokenError::UnsupportedPlatformCoin {
            platform_coin_ticker: token_protocol.platform_coin_ticker().into(),
            token_ticker: req.ticker.clone(),
        }
    })?;

    let (token, activation_result) =
        Token::enable_token(req.ticker, platform_coin, req.activation_params, token_protocol).await?;

    let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
    coins_ctx
        .add_coin(token.into())
        .await
        .mm_err(|e| EnableTokenError::TokenIsAlreadyActivated(e.ticker))?;

    Ok(activation_result)
}

impl From<UtxoRpcError> for EnableTokenError {
    fn from(err: UtxoRpcError) -> Self {
        match err {
            UtxoRpcError::Transport(e) | UtxoRpcError::ResponseParseError(e) => {
                EnableTokenError::Transport(e.to_string())
            },
            UtxoRpcError::InvalidResponse(e) => EnableTokenError::Transport(e),
            UtxoRpcError::Internal(e) => EnableTokenError::Internal(e),
        }
    }
}

impl HttpStatusCode for EnableTokenError {
    fn status_code(&self) -> StatusCode {
        match self {
            EnableTokenError::TokenIsAlreadyActivated(_)
            | EnableTokenError::PlatformCoinIsNotActivated(_)
            | EnableTokenError::TokenConfigIsNotFound { .. }
            | EnableTokenError::UnexpectedTokenProtocol { .. } => StatusCode::BAD_REQUEST,
            EnableTokenError::TokenProtocolParseError { .. }
            | EnableTokenError::UnsupportedPlatformCoin { .. }
            | EnableTokenError::UnexpectedDerivationMethod(_)
            | EnableTokenError::Transport(_)
            | EnableTokenError::CouldNotFetchBalance(_)
            | EnableTokenError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
