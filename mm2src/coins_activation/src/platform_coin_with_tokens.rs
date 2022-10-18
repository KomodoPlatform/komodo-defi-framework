use crate::prelude::*;
use async_trait::async_trait;
use coins::my_tx_history_v2::TxHistoryStorage;
use coins::tx_history_storage::{CreateTxHistoryStorageError, TxHistoryStorageBuilder};
use coins::{lp_coinfind, CoinProtocol, CoinsContext, MmCoinEnum};
use common::{log, HttpStatusCode, StatusCode};
use derive_more::Display;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_metrics::MetricsArc;
use mm2_number::BigDecimal;
use ser_error_derive::SerializeErrorType;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::convert::Infallible;

#[derive(Clone, Debug, Deserialize)]
pub struct TokenActivationRequest<Req> {
    ticker: String,
    #[serde(flatten)]
    request: Req,
}

pub trait TokenOf: Into<MmCoinEnum> {
    type PlatformCoin: PlatformWithTokensActivationOps + RegisterTokenInfo<Self>;
}

pub struct TokenActivationParams<Req, Protocol> {
    pub(crate) ticker: String,
    pub(crate) activation_request: Req,
    pub(crate) protocol: Protocol,
}

#[async_trait]
pub trait TokenInitializer {
    type Token: TokenOf;
    type TokenActivationRequest: Send;
    type TokenProtocol: TryFromCoinProtocol + Send;
    type InitTokensError: NotMmError;

    fn tokens_requests_from_platform_request(
        platform_request: &<<Self::Token as TokenOf>::PlatformCoin as PlatformWithTokensActivationOps>::ActivationRequest,
    ) -> Vec<TokenActivationRequest<Self::TokenActivationRequest>>;

    async fn enable_tokens(
        &self,
        params: Vec<TokenActivationParams<Self::TokenActivationRequest, Self::TokenProtocol>>,
    ) -> Result<Vec<Self::Token>, MmError<Self::InitTokensError>>;

    fn platform_coin(&self) -> &<Self::Token as TokenOf>::PlatformCoin;
}

#[async_trait]
pub trait TokenAsMmCoinInitializer: Send + Sync {
    type PlatformCoin;
    type ActivationRequest;

    async fn enable_tokens_as_mm_coins(
        &self,
        ctx: MmArc,
        request: &Self::ActivationRequest,
    ) -> Result<Vec<MmCoinEnum>, MmError<InitTokensAsMmCoinsError>>;
}

pub enum InitTokensAsMmCoinsError {
    TokenConfigIsNotFound(String),
    InvalidPubkey(String),
    CouldNotFetchBalance(String),
    Internal(String),
    TokenProtocolParseError { ticker: String, error: String },
    UnexpectedTokenProtocol { ticker: String, protocol: CoinProtocol },
}

impl From<CoinConfWithProtocolError> for InitTokensAsMmCoinsError {
    fn from(err: CoinConfWithProtocolError) -> Self {
        match err {
            CoinConfWithProtocolError::ConfigIsNotFound(e) => InitTokensAsMmCoinsError::TokenConfigIsNotFound(e),
            CoinConfWithProtocolError::CoinProtocolParseError { ticker, err } => {
                InitTokensAsMmCoinsError::TokenProtocolParseError {
                    ticker,
                    error: err.to_string(),
                }
            },
            CoinConfWithProtocolError::UnexpectedProtocol { ticker, protocol } => {
                InitTokensAsMmCoinsError::UnexpectedTokenProtocol { ticker, protocol }
            },
        }
    }
}

pub trait RegisterTokenInfo<T: TokenOf<PlatformCoin = Self>> {
    fn register_token_info(&self, token: &T);
}

impl From<std::convert::Infallible> for InitTokensAsMmCoinsError {
    fn from(e: Infallible) -> Self { match e {} }
}

#[async_trait]
impl<T> TokenAsMmCoinInitializer for T
where
    T: TokenInitializer + Send + Sync,
    InitTokensAsMmCoinsError: From<T::InitTokensError>,
    (T::InitTokensError, InitTokensAsMmCoinsError): NotEqual,
{
    type PlatformCoin = <T::Token as TokenOf>::PlatformCoin;
    type ActivationRequest = <Self::PlatformCoin as PlatformWithTokensActivationOps>::ActivationRequest;

    async fn enable_tokens_as_mm_coins(
        &self,
        ctx: MmArc,
        request: &Self::ActivationRequest,
    ) -> Result<Vec<MmCoinEnum>, MmError<InitTokensAsMmCoinsError>> {
        let tokens_requests = T::tokens_requests_from_platform_request(request);
        let token_params = tokens_requests
            .into_iter()
            .map(|req| -> Result<_, MmError<CoinConfWithProtocolError>> {
                let (_, protocol): (_, T::TokenProtocol) = coin_conf_with_protocol(&ctx, &req.ticker)?;
                Ok(TokenActivationParams {
                    ticker: req.ticker,
                    activation_request: req.request,
                    protocol,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let tokens = self.enable_tokens(token_params).await?;
        for token in tokens.iter() {
            self.platform_coin().register_token_info(token);
        }
        Ok(tokens.into_iter().map(Into::into).collect())
    }
}

pub trait GetPlatformBalance {
    fn get_platform_balance(&self) -> BigDecimal;
}

#[async_trait]
pub trait PlatformWithTokensActivationOps: Into<MmCoinEnum> {
    type ActivationRequest: Clone + Send + Sync + TxHistory;
    type PlatformProtocolInfo: TryFromCoinProtocol;
    type ActivationResult: GetPlatformBalance + CurrentBlock;
    type ActivationError: NotMmError;

    /// Initializes the platform coin itself
    async fn enable_platform_coin(
        ctx: MmArc,
        ticker: String,
        coin_conf: Json,
        activation_request: Self::ActivationRequest,
        protocol_conf: Self::PlatformProtocolInfo,
        priv_key: &[u8],
    ) -> Result<Self, MmError<Self::ActivationError>>;

    fn token_initializers(
        &self,
    ) -> Vec<Box<dyn TokenAsMmCoinInitializer<PlatformCoin = Self, ActivationRequest = Self::ActivationRequest>>>;

    async fn get_activation_result(&self) -> Result<Self::ActivationResult, MmError<Self::ActivationError>>;

    fn start_history_background_fetching(
        &self,
        metrics: MetricsArc,
        storage: impl TxHistoryStorage,
        initial_balance: BigDecimal,
    );
}

#[derive(Debug, Deserialize)]
pub struct EnablePlatformCoinWithTokensReq<T: Clone> {
    ticker: String,
    #[serde(flatten)]
    request: T,
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum EnablePlatformCoinWithTokensError {
    PlatformIsAlreadyActivated(String),
    #[display(fmt = "Platform {} config is not found", _0)]
    PlatformConfigIsNotFound(String),
    #[display(fmt = "Platform coin {} protocol parsing failed: {}", ticker, error)]
    CoinProtocolParseError {
        ticker: String,
        error: String,
    },
    #[display(fmt = "Unexpected platform protocol {:?} for {}", protocol, ticker)]
    UnexpectedPlatformProtocol {
        ticker: String,
        protocol: CoinProtocol,
    },
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
    #[display(fmt = "Error on platform coin {} creation: {}", ticker, error)]
    PlatformCoinCreationError {
        ticker: String,
        error: String,
    },
    #[display(fmt = "Private key is not allowed: {}", _0)]
    PrivKeyNotAllowed(String),
    #[display(fmt = "Unexpected derivation method: {}", _0)]
    UnexpectedDerivationMethod(String),
    Transport(String),
    AtLeastOneNodeRequired(String),
    InvalidPayload(String),
    Internal(String),
}

impl From<CoinConfWithProtocolError> for EnablePlatformCoinWithTokensError {
    fn from(err: CoinConfWithProtocolError) -> Self {
        match err {
            CoinConfWithProtocolError::ConfigIsNotFound(ticker) => {
                EnablePlatformCoinWithTokensError::PlatformConfigIsNotFound(ticker)
            },
            CoinConfWithProtocolError::UnexpectedProtocol { ticker, protocol } => {
                EnablePlatformCoinWithTokensError::UnexpectedPlatformProtocol { ticker, protocol }
            },
            CoinConfWithProtocolError::CoinProtocolParseError { ticker, err } => {
                EnablePlatformCoinWithTokensError::CoinProtocolParseError {
                    ticker,
                    error: err.to_string(),
                }
            },
        }
    }
}

impl From<InitTokensAsMmCoinsError> for EnablePlatformCoinWithTokensError {
    fn from(err: InitTokensAsMmCoinsError) -> Self {
        match err {
            InitTokensAsMmCoinsError::TokenConfigIsNotFound(ticker) => {
                EnablePlatformCoinWithTokensError::TokenConfigIsNotFound(ticker)
            },
            InitTokensAsMmCoinsError::TokenProtocolParseError { ticker, error } => {
                EnablePlatformCoinWithTokensError::TokenProtocolParseError { ticker, error }
            },
            InitTokensAsMmCoinsError::UnexpectedTokenProtocol { ticker, protocol } => {
                EnablePlatformCoinWithTokensError::UnexpectedTokenProtocol { ticker, protocol }
            },
            InitTokensAsMmCoinsError::InvalidPubkey(e) | InitTokensAsMmCoinsError::Internal(e) => {
                EnablePlatformCoinWithTokensError::Internal(e)
            },
            InitTokensAsMmCoinsError::CouldNotFetchBalance(e) => EnablePlatformCoinWithTokensError::Transport(e),
        }
    }
}

impl From<CreateTxHistoryStorageError> for EnablePlatformCoinWithTokensError {
    fn from(e: CreateTxHistoryStorageError) -> Self {
        match e {
            CreateTxHistoryStorageError::Internal(internal) => EnablePlatformCoinWithTokensError::Internal(internal),
        }
    }
}

impl HttpStatusCode for EnablePlatformCoinWithTokensError {
    fn status_code(&self) -> StatusCode {
        match self {
            EnablePlatformCoinWithTokensError::CoinProtocolParseError { .. }
            | EnablePlatformCoinWithTokensError::TokenProtocolParseError { .. }
            | EnablePlatformCoinWithTokensError::PlatformCoinCreationError { .. }
            | EnablePlatformCoinWithTokensError::PrivKeyNotAllowed(_)
            | EnablePlatformCoinWithTokensError::UnexpectedDerivationMethod(_)
            | EnablePlatformCoinWithTokensError::Transport(_)
            | EnablePlatformCoinWithTokensError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            EnablePlatformCoinWithTokensError::PlatformIsAlreadyActivated(_)
            | EnablePlatformCoinWithTokensError::PlatformConfigIsNotFound(_)
            | EnablePlatformCoinWithTokensError::TokenConfigIsNotFound(_)
            | EnablePlatformCoinWithTokensError::UnexpectedPlatformProtocol { .. }
            | EnablePlatformCoinWithTokensError::InvalidPayload { .. }
            | EnablePlatformCoinWithTokensError::AtLeastOneNodeRequired(_)
            | EnablePlatformCoinWithTokensError::UnexpectedTokenProtocol { .. } => StatusCode::BAD_REQUEST,
        }
    }
}

pub async fn enable_platform_coin_with_tokens<Platform>(
    ctx: MmArc,
    req: EnablePlatformCoinWithTokensReq<Platform::ActivationRequest>,
) -> Result<Platform::ActivationResult, MmError<EnablePlatformCoinWithTokensError>>
where
    Platform: PlatformWithTokensActivationOps,
    EnablePlatformCoinWithTokensError: From<Platform::ActivationError>,
    (Platform::ActivationError, EnablePlatformCoinWithTokensError): NotEqual,
{
    if let Ok(Some(_)) = lp_coinfind(&ctx, &req.ticker).await {
        return MmError::err(EnablePlatformCoinWithTokensError::PlatformIsAlreadyActivated(
            req.ticker,
        ));
    }

    let (platform_conf, platform_protocol) = coin_conf_with_protocol(&ctx, &req.ticker)?;

    let priv_key = &*ctx.secp256k1_key_pair().private().secret;

    let platform_coin = Platform::enable_platform_coin(
        ctx.clone(),
        req.ticker.clone(),
        platform_conf,
        req.request.clone(),
        platform_protocol,
        priv_key,
    )
    .await?;
    let mut mm_tokens = Vec::new();
    for initializer in platform_coin.token_initializers() {
        let tokens = initializer.enable_tokens_as_mm_coins(ctx.clone(), &req.request).await?;
        mm_tokens.extend(tokens);
    }

    let activation_result = platform_coin.get_activation_result().await?;
    log::info!("{} current block {}", req.ticker, activation_result.current_block());

    if req.request.tx_history() {
        platform_coin.start_history_background_fetching(
            ctx.metrics.clone(),
            TxHistoryStorageBuilder::new(&ctx).build()?,
            activation_result.get_platform_balance(),
        );
    }

    let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
    coins_ctx
        .add_platform_with_tokens(platform_coin.into(), mm_tokens)
        .await
        .mm_err(|e| EnablePlatformCoinWithTokensError::PlatformIsAlreadyActivated(e.ticker))?;

    Ok(activation_result)
}
