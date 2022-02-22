/// Contains L2 activation traits and their implementations for various coins
///
use crate::prelude::*;
use async_trait::async_trait;
use coins::{lp_coinfind, lp_coinfind_or_err, CoinProtocol, CoinsContext, MmCoinEnum};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::{HttpStatusCode, NotSame, StatusCode};
use derive_more::Display;
use ser_error_derive::SerializeErrorType;
use serde_derive::{Deserialize, Serialize};

pub trait L2ProtocolParams {
    fn platform_coin_ticker(&self) -> &str;
}

#[async_trait]
pub trait L2ActivationOps: Into<MmCoinEnum> {
    type PlatformCoin: TryPlatformCoinFromMmCoinEnum;
    type ActivationParams;
    type ProtocolInfo: L2ProtocolParams + TryFromCoinProtocol;
    type ValidatedParams;
    type ActivationResult;
    type ActivationError: NotMmError;

    fn validate_platform_configuration(
        platform_coin: &Self::PlatformCoin,
    ) -> Result<(), MmError<Self::ActivationError>>;

    fn validate_activation_params(
        activation_params: Self::ActivationParams,
    ) -> Result<Self::ValidatedParams, MmError<Self::ActivationError>>;

    async fn enable_l2(
        ctx: &MmArc,
        ticker: String,
        platform_coin: Self::PlatformCoin,
        validated_params: Self::ValidatedParams,
        protocol_conf: Self::ProtocolInfo,
    ) -> Result<(Self, Self::ActivationResult), MmError<Self::ActivationError>>;
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum EnableL2Error {
    #[display(fmt = "Layer 2 {} is already activated", _0)]
    L2IsAlreadyActivated(String),
    #[display(fmt = "Layer 2 {} config is not found", _0)]
    L2ConfigIsNotFound(String),
    #[display(fmt = "Layer 2 {} protocol parsing failed: {}", ticker, error)]
    L2ProtocolParseError {
        ticker: String,
        error: String,
    },
    #[display(fmt = "Unexpected layer 2 protocol {:?} for {}", protocol, ticker)]
    UnexpectedL2Protocol {
        ticker: String,
        protocol: CoinProtocol,
    },
    #[display(fmt = "Platform coin {} is not activated", _0)]
    PlatformCoinIsNotActivated(String),
    #[display(fmt = "{} is not a platform coin for layer 2 {}", platform_coin_ticker, l2_ticker)]
    UnsupportedPlatformCoin {
        platform_coin_ticker: String,
        l2_ticker: String,
    },
    Transport(String),
    Internal(String),
}

impl From<CoinConfWithProtocolError> for EnableL2Error {
    fn from(err: CoinConfWithProtocolError) -> Self {
        match err {
            CoinConfWithProtocolError::ConfigIsNotFound(ticker) => EnableL2Error::L2ConfigIsNotFound(ticker),
            CoinConfWithProtocolError::CoinProtocolParseError { ticker, err } => EnableL2Error::L2ProtocolParseError {
                ticker,
                error: err.to_string(),
            },
            CoinConfWithProtocolError::UnexpectedProtocol { ticker, protocol } => {
                EnableL2Error::UnexpectedL2Protocol { ticker, protocol }
            },
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EnableL2Request<T> {
    ticker: String,
    activation_params: T,
}

pub async fn enable_l2<L2>(
    ctx: MmArc,
    req: EnableL2Request<L2::ActivationParams>,
) -> Result<L2::ActivationResult, MmError<EnableL2Error>>
where
    L2: L2ActivationOps,
    EnableL2Error: From<L2::ActivationError>,
    (L2::ActivationError, EnableL2Error): NotSame,
{
    if let Ok(Some(_)) = lp_coinfind(&ctx, &req.ticker).await {
        return MmError::err(EnableL2Error::L2IsAlreadyActivated(req.ticker));
    }

    let (_, l2_protocol): (_, L2::ProtocolInfo) = coin_conf_with_protocol(&ctx, &req.ticker)?;

    let platform_coin = lp_coinfind_or_err(&ctx, l2_protocol.platform_coin_ticker())
        .await
        .mm_err(|_| EnableL2Error::PlatformCoinIsNotActivated(req.ticker.clone()))?;

    let platform_coin =
        L2::PlatformCoin::try_from_mm_coin(platform_coin).or_mm_err(|| EnableL2Error::UnsupportedPlatformCoin {
            platform_coin_ticker: l2_protocol.platform_coin_ticker().into(),
            l2_ticker: req.ticker.clone(),
        })?;

    L2::validate_platform_configuration(&platform_coin)?;

    let validated_params = L2::validate_activation_params(req.activation_params)?;

    let (l2, activation_result) = L2::enable_l2(&ctx, req.ticker, platform_coin, validated_params, l2_protocol).await?;

    let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
    coins_ctx
        .add_coin(l2.into())
        .await
        .mm_err(|e| EnableL2Error::L2IsAlreadyActivated(e.ticker))?;

    Ok(activation_result)
}

impl HttpStatusCode for EnableL2Error {
    fn status_code(&self) -> StatusCode {
        match self {
            EnableL2Error::L2IsAlreadyActivated(_)
            | EnableL2Error::PlatformCoinIsNotActivated(_)
            | EnableL2Error::L2ConfigIsNotFound { .. }
            | EnableL2Error::UnexpectedL2Protocol { .. } => StatusCode::BAD_REQUEST,
            EnableL2Error::L2ProtocolParseError { .. }
            | EnableL2Error::UnsupportedPlatformCoin { .. }
            | EnableL2Error::Transport(_)
            | EnableL2Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
