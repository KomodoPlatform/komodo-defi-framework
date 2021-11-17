use crate::prelude::*;
use crate::token::SlpActivationParams;
use async_trait::async_trait;
use coins::utxo::bch::BchCoin;
use coins::utxo::slp::SlpToken;
use coins::{lp_coinfind, CoinProtocol, MmCoinEnum};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::NotSame;
use derive_more::Display;
use ser_error_derive::SerializeErrorType;
use serde_derive::Serialize;
use serde_json::Value as Json;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct TokenActivationParams<T> {
    ticker: String,
    activation_params: T,
}

pub trait TokenOf: Into<MmCoinEnum> {
    type PlatformCoin: PlatformWithTokensActivationOps;
}

#[async_trait]
pub trait TokenInitializer {
    type Token: TokenOf;
    type TokenActivationParams: Send;

    fn tokens_params_from_platform_params(
        platform_params: &<<Self::Token as TokenOf>::PlatformCoin as PlatformWithTokensActivationOps>::ActivationParams,
    ) -> Vec<TokenActivationParams<Self::TokenActivationParams>>;

    async fn init_tokens(
        self,
        params: Vec<TokenActivationParams<Self::TokenActivationParams>>,
    ) -> Result<Vec<Self::Token>, MmError<()>>;
}

#[async_trait]
pub trait TokenAsMmCoinInitializer {
    type PlatformCoin;
    type ActivationParams;

    async fn init_tokens_as_mm_coins(
        self,
        params: &Self::ActivationParams,
    ) -> Result<Vec<MmCoinEnum>, MmError<InitTokensAsMmCoinsError>>;
}

pub trait PlatformCoinWithTokensActivationOps {}

pub enum InitTokensAsMmCoinsError {
    TokenConfigIsNotFound(String),
    TokenProtocolParseError(String),
    UnexpectedTokenProtocol(CoinProtocol),
}

#[async_trait]
impl<T: TokenInitializer + Send> TokenAsMmCoinInitializer for T {
    type PlatformCoin = <T::Token as TokenOf>::PlatformCoin;
    type ActivationParams = <Self::PlatformCoin as PlatformWithTokensActivationOps>::ActivationParams;

    async fn init_tokens_as_mm_coins(
        self,
        params: &Self::ActivationParams,
    ) -> Result<Vec<MmCoinEnum>, MmError<InitTokensAsMmCoinsError>> {
        let token_params = T::tokens_params_from_platform_params(params);

        let tokens = self.init_tokens(token_params).await.unwrap();
        Ok(tokens.into_iter().map(Into::into).collect())
    }
}

#[async_trait]
pub trait PlatformWithTokensActivationOps: Into<MmCoinEnum> {
    type ActivationParams: Send + Sync;
    type PlatformProtocolInfo: TryFromCoinProtocol;
    type ActivationResult;
    type ActivationError: NotMmError;

    /// Initializes the platform coin itself
    async fn init_platform_coin(
        ticker: String,
        coin_conf: Json,
        activation_params: Self::ActivationParams,
        protocol_conf: Self::PlatformProtocolInfo,
    ) -> Result<Self, MmError<Self::ActivationError>>;

    fn token_initializers(
    ) -> Vec<Box<dyn TokenAsMmCoinInitializer<PlatformCoin = Self, ActivationParams = Self::ActivationParams>>>;
}

pub struct BchWithTokensActivationParams {
    slp_tokens_params: Vec<TokenActivationParams<SlpActivationParams>>,
}

pub struct BchProtocolInfo {
    #[allow(dead_code)]
    slp_prefix: String,
}

impl TryFromCoinProtocol for BchProtocolInfo {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::BCH { slp_prefix } => Ok(BchProtocolInfo { slp_prefix }),
            protocol => MmError::err(protocol),
        }
    }
}

#[async_trait]
impl PlatformWithTokensActivationOps for BchCoin {
    type ActivationParams = BchWithTokensActivationParams;
    type PlatformProtocolInfo = BchProtocolInfo;
    type ActivationResult = ();
    type ActivationError = ();

    async fn init_platform_coin(
        _ticker: String,
        _platform_conf: Json,
        _activation_params: Self::ActivationParams,
        _protocol_conf: Self::PlatformProtocolInfo,
    ) -> Result<Self, MmError<Self::ActivationError>> {
        unimplemented!()
    }

    fn token_initializers(
    ) -> Vec<Box<dyn TokenAsMmCoinInitializer<PlatformCoin = Self, ActivationParams = Self::ActivationParams>>> {
        vec![Box::new(SlpTokenInitializer {})]
    }
}

pub struct SlpTokenInitializer {}

impl TokenOf for SlpToken {
    type PlatformCoin = BchCoin;
}

#[async_trait]
impl TokenInitializer for SlpTokenInitializer {
    type Token = SlpToken;
    type TokenActivationParams = SlpActivationParams;

    fn tokens_params_from_platform_params(
        platform_params: &BchWithTokensActivationParams,
    ) -> Vec<TokenActivationParams<Self::TokenActivationParams>> {
        platform_params.slp_tokens_params.clone()
    }

    async fn init_tokens(
        self,
        _activation_params: Vec<TokenActivationParams<SlpActivationParams>>,
    ) -> Result<Vec<SlpToken>, MmError<()>> {
        unimplemented!()
    }
}

pub struct EnablePlatformCoinWithTokensReq<T> {
    ticker: String,
    #[allow(dead_code)]
    activation_params: T,
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum EnablePlatformCoinWithTokensError {
    PlatformIsAlreadyActivated(String),
    PlatformConfigIsNotFound(String),
    CoinProtocolParseError(String),
    #[display(fmt = "Unexpected platform protocol {:?}", _0)]
    UnexpectedPlatformProtocol(CoinProtocol),
    Transport(String),
    Internal(String),
}

impl From<CoinConfWithProtocolError> for EnablePlatformCoinWithTokensError {
    fn from(err: CoinConfWithProtocolError) -> Self {
        match err {
            CoinConfWithProtocolError::ConfigIsNotFound(ticker) => {
                EnablePlatformCoinWithTokensError::PlatformConfigIsNotFound(ticker)
            },
            CoinConfWithProtocolError::UnexpectedProtocol(proto) => {
                EnablePlatformCoinWithTokensError::UnexpectedPlatformProtocol(proto)
            },
            CoinConfWithProtocolError::CoinProtocolParseError(e) => {
                EnablePlatformCoinWithTokensError::CoinProtocolParseError(e.to_string())
            },
        }
    }
}

pub async fn enable_platform_coin_with_tokens<Platform>(
    ctx: MmArc,
    req: EnablePlatformCoinWithTokensReq<Platform::ActivationParams>,
) -> Result<Platform::ActivationResult, MmError<EnablePlatformCoinWithTokensError>>
where
    Platform: PlatformWithTokensActivationOps,
    EnablePlatformCoinWithTokensError: From<Platform::ActivationError>,
    (Platform::ActivationError, EnablePlatformCoinWithTokensError): NotSame,
{
    if let Ok(Some(_)) = lp_coinfind(&ctx, &req.ticker).await {
        return MmError::err(EnablePlatformCoinWithTokensError::PlatformIsAlreadyActivated(
            req.ticker,
        ));
    }

    let (platform_conf, platform_protocol) = coin_conf_with_protocol(&ctx, &req.ticker)?;

    let _platform_coin =
        Platform::init_platform_coin(req.ticker, platform_conf, req.activation_params, platform_protocol).await?;
    for _initializer in Platform::token_initializers() {}
    unimplemented!()
}
