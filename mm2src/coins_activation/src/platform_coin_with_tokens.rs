use crate::prelude::*;
use crate::token::SlpActivationRequest;
use async_trait::async_trait;
use coins::utxo::bch::{bch_coin_from_conf_and_params, BchActivationRequest, BchCoin, CashAddrPrefix};
use coins::utxo::slp::{SlpProtocolConf, SlpToken};
use coins::{lp_coinfind, CoinBalance, CoinProtocol, CoinsContext, MarketCoinOps, MmCoin, MmCoinEnum};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::NotSame;
use derive_more::Display;
use ser_error_derive::SerializeErrorType;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::HashMap;
use std::convert::Infallible;
use std::str::FromStr;

#[derive(Clone, Debug, Deserialize)]
pub struct TokenActivationRequest<Req> {
    ticker: String,
    request: Req,
}

pub trait TokenOf: Into<MmCoinEnum> {
    type PlatformCoin: PlatformWithTokensActivationOps + RegisterTokenInfo<Self>;
}

pub struct TokenActivationParams<Req, Protocol> {
    ticker: String,
    activation_request: Req,
    protocol: Protocol,
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

    async fn init_tokens(
        &self,
        params: Vec<TokenActivationParams<Self::TokenActivationRequest, Self::TokenProtocol>>,
    ) -> Result<Vec<Self::Token>, MmError<Self::InitTokensError>>;

    fn platform_coin(&self) -> &<Self::Token as TokenOf>::PlatformCoin;
}

#[async_trait]
pub trait TokenAsMmCoinInitializer {
    type PlatformCoin;
    type ActivationRequest;

    async fn init_tokens_as_mm_coins(
        &self,
        ctx: MmArc,
        request: &Self::ActivationRequest,
    ) -> Result<Vec<MmCoinEnum>, MmError<InitTokensAsMmCoinsError>>;
}

pub trait PlatformCoinWithTokensActivationOps {}

pub enum InitTokensAsMmCoinsError {
    TokenConfigIsNotFound(String),
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
    (T::InitTokensError, InitTokensAsMmCoinsError): NotSame,
{
    type PlatformCoin = <T::Token as TokenOf>::PlatformCoin;
    type ActivationRequest = <Self::PlatformCoin as PlatformWithTokensActivationOps>::ActivationRequest;

    async fn init_tokens_as_mm_coins(
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

        let tokens = self.init_tokens(token_params).await?;
        for token in tokens.iter() {
            self.platform_coin().register_token_info(token);
        }
        Ok(tokens.into_iter().map(Into::into).collect())
    }
}

#[async_trait]
pub trait PlatformWithTokensActivationOps: Into<MmCoinEnum> {
    type ActivationRequest: Clone + Send + Sync;
    type PlatformProtocolInfo: TryFromCoinProtocol;
    type ActivationResult;
    type ActivationError: NotMmError;

    /// Initializes the platform coin itself
    async fn init_platform_coin(
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
}

#[derive(Clone, Debug, Deserialize)]
pub struct BchWithTokensActivationRequest {
    #[serde(flatten)]
    platform_request: BchActivationRequest,
    slp_tokens_requests: Vec<TokenActivationRequest<SlpActivationRequest>>,
}

pub struct BchProtocolInfo {
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

#[derive(Debug)]
pub enum DerivationMethod {
    /// Legacy iguana's privkey derivation, used by default
    Iguana,
    /// HD wallet derivation path, String is temporary here
    HDWallet(String),
}

#[derive(Debug)]
pub struct CoinAddressInfo<Balance> {
    derivation_method: DerivationMethod,
    pubkey: String,
    balances: Balance,
}

pub type TokenBalances = HashMap<String, CoinBalance>;

#[derive(Debug)]
pub struct BchWithTokensActivationResult {
    current_block: u64,
    bch_addresses_infos: HashMap<String, CoinAddressInfo<CoinBalance>>,
    slp_addresses_infos: HashMap<String, CoinAddressInfo<TokenBalances>>,
}

#[derive(Debug)]
pub enum BchWithTokensActivationError {
    PlatformCoinCreationError {
        ticker: String,
        error: String,
    },
    InvalidSlpPrefix {
        ticker: String,
        prefix: String,
        error: String,
    },
}

#[async_trait]
impl PlatformWithTokensActivationOps for BchCoin {
    type ActivationRequest = BchWithTokensActivationRequest;
    type PlatformProtocolInfo = BchProtocolInfo;
    type ActivationResult = BchWithTokensActivationResult;
    type ActivationError = BchWithTokensActivationError;

    async fn init_platform_coin(
        ctx: MmArc,
        ticker: String,
        platform_conf: Json,
        activation_request: Self::ActivationRequest,
        protocol_conf: Self::PlatformProtocolInfo,
        priv_key: &[u8],
    ) -> Result<Self, MmError<Self::ActivationError>> {
        let slp_prefix = CashAddrPrefix::from_str(&protocol_conf.slp_prefix).map_to_mm(|error| {
            BchWithTokensActivationError::InvalidSlpPrefix {
                ticker: ticker.clone(),
                prefix: protocol_conf.slp_prefix,
                error,
            }
        })?;

        let platform_coin = bch_coin_from_conf_and_params(
            &ctx,
            &ticker,
            &platform_conf,
            activation_request.platform_request,
            slp_prefix,
            priv_key,
        )
        .await
        .map_to_mm(|error| BchWithTokensActivationError::PlatformCoinCreationError { ticker, error })?;
        Ok(platform_coin)
    }

    fn token_initializers(
        &self,
    ) -> Vec<Box<dyn TokenAsMmCoinInitializer<PlatformCoin = Self, ActivationRequest = Self::ActivationRequest>>> {
        vec![Box::new(SlpTokenInitializer {
            platform_coin: self.clone(),
        })]
    }

    async fn get_activation_result(&self) -> Result<BchWithTokensActivationResult, MmError<Self::ActivationError>> {
        todo!()
    }
}

pub struct SlpTokenInitializer {
    platform_coin: BchCoin,
}

impl TokenOf for SlpToken {
    type PlatformCoin = BchCoin;
}

#[async_trait]
impl TokenInitializer for SlpTokenInitializer {
    type Token = SlpToken;
    type TokenActivationRequest = SlpActivationRequest;
    type TokenProtocol = SlpProtocolConf;
    type InitTokensError = std::convert::Infallible;

    fn tokens_requests_from_platform_request(
        platform_params: &BchWithTokensActivationRequest,
    ) -> Vec<TokenActivationRequest<Self::TokenActivationRequest>> {
        platform_params.slp_tokens_requests.clone()
    }

    async fn init_tokens(
        &self,
        activation_params: Vec<TokenActivationParams<SlpActivationRequest, SlpProtocolConf>>,
    ) -> Result<Vec<SlpToken>, MmError<std::convert::Infallible>> {
        let tokens = activation_params
            .into_iter()
            .map(|params| {
                // confirmation settings from RPC request have the highest priority
                let required_confirmations = params.activation_request.required_confirmations.unwrap_or_else(|| {
                    params
                        .protocol
                        .required_confirmations
                        .unwrap_or_else(|| self.platform_coin.required_confirmations())
                });

                SlpToken::new(
                    params.protocol.decimals,
                    params.ticker,
                    params.protocol.token_id,
                    self.platform_coin.clone(),
                    required_confirmations,
                )
            })
            .collect();

        Ok(tokens)
    }

    fn platform_coin(&self) -> &BchCoin { &self.platform_coin }
}

pub struct EnablePlatformCoinWithTokensReq<T: Clone> {
    ticker: String,
    request: T,
}

impl RegisterTokenInfo<SlpToken> for BchCoin {
    fn register_token_info(&self, token: &SlpToken) { self.add_slp_token_info(token.ticker().into(), token.get_info()) }
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
    Transport(String),
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
    (Platform::ActivationError, EnablePlatformCoinWithTokensError): NotSame,
{
    if let Ok(Some(_)) = lp_coinfind(&ctx, &req.ticker).await {
        return MmError::err(EnablePlatformCoinWithTokensError::PlatformIsAlreadyActivated(
            req.ticker,
        ));
    }

    let (platform_conf, platform_protocol) = coin_conf_with_protocol(&ctx, &req.ticker)?;

    let priv_key = &*ctx.secp256k1_key_pair().private().secret;

    let platform_coin = Platform::init_platform_coin(
        ctx.clone(),
        req.ticker,
        platform_conf,
        req.request.clone(),
        platform_protocol,
        priv_key,
    )
    .await?;
    let mut mm_tokens = Vec::new();
    for initializer in platform_coin.token_initializers() {
        let tokens = initializer.init_tokens_as_mm_coins(ctx.clone(), &req.request).await?;
        mm_tokens.extend(tokens)
    }

    let activation_result = platform_coin.get_activation_result().await?;
    let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
    coins_ctx
        .add_platform_with_tokens(platform_coin.into(), mm_tokens)
        .await
        .mm_err(|e| EnablePlatformCoinWithTokensError::PlatformIsAlreadyActivated(e.ticker))?;

    Ok(activation_result)
}
