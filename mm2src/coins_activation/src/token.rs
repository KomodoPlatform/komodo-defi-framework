/// Contains token activation traits and their implementations for various coins
///
use async_trait::async_trait;
use coins::utxo::bch::BchCoin;
use coins::utxo::rpc_clients::UtxoRpcError;
use coins::utxo::slp::{SlpProtocolConf, SlpToken};
use coins::{coin_conf, lp_coinfind, lp_coinfind_or_err, CoinBalance, CoinProtocol, CoinsContext, MarketCoinOps,
            MmCoin, MmCoinEnum};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::{HttpStatusCode, NotSame, StatusCode};
use derive_more::Display;
use rpc::v1::types::H256 as H256Json;
use ser_error_derive::SerializeErrorType;
use serde_derive::{Deserialize, Serialize};
use serde_json::{self as json};
use std::collections::HashMap;

pub trait TryPlatformCoinFromMmCoinEnum {
    fn try_from_mm_coin(coin: MmCoinEnum) -> Option<Self>
    where
        Self: Sized;
}

pub trait TryTokenProtoFromCoinProto {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized;
}

pub trait TokenProtocolParams {
    fn platform_coin_ticker(&self) -> &str;
}

#[async_trait]
pub trait TokenActivationOps: Into<MmCoinEnum> {
    type PlatformCoin: TryPlatformCoinFromMmCoinEnum;
    type ActivationParams;
    type ProtocolInfo: TokenProtocolParams + TryTokenProtoFromCoinProto;
    type ActivationResult;
    type ActivationError: NotMmError;

    async fn init_token(
        ticker: String,
        platform_coin: Self::PlatformCoin,
        activation_params: Self::ActivationParams,
        protocol_conf: Self::ProtocolInfo,
    ) -> Result<(Self, Self::ActivationResult), MmError<Self::ActivationError>>;
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum EnableTokenError {
    TokenIsAlreadyActivated(String),
    TokenConfigIsNotFound(String),
    InvalidTokenProtocolConf(String),
    #[display(fmt = "Invalid coin protocol {:?}", _0)]
    InvalidCoinProtocol(CoinProtocol),
    PlatformCoinIsNotActivated(String),
    #[display(fmt = "{} is not a platform coin for token {}", platform_coin_ticker, token_ticker)]
    UnsupportedPlatformCoin {
        platform_coin_ticker: String,
        token_ticker: String,
    },
    Transport(String),
    Internal(String),
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
    (Token::ActivationError, EnableTokenError): NotSame,
{
    if let Ok(Some(_)) = lp_coinfind(&ctx, &req.ticker).await {
        return MmError::err(EnableTokenError::TokenIsAlreadyActivated(req.ticker));
    }

    let conf = coin_conf(&ctx, &req.ticker);
    if conf.is_null() {
        return MmError::err(EnableTokenError::TokenConfigIsNotFound(req.ticker));
    }

    let coin_protocol: CoinProtocol = json::from_value(conf["protocol"].clone())
        .map_to_mm(|e| EnableTokenError::InvalidTokenProtocolConf(e.to_string()))?;
    let token_protocol =
        Token::ProtocolInfo::try_from_coin_protocol(coin_protocol).mm_err(EnableTokenError::InvalidCoinProtocol)?;

    let platform_coin = lp_coinfind_or_err(&ctx, token_protocol.platform_coin_ticker())
        .await
        .mm_err(|_| EnableTokenError::PlatformCoinIsNotActivated(req.ticker.clone()))?;

    let platform_coin = Token::PlatformCoin::try_from_mm_coin(platform_coin).or_mm_err(|| {
        EnableTokenError::UnsupportedPlatformCoin {
            platform_coin_ticker: token_protocol.platform_coin_ticker().into(),
            token_ticker: req.ticker.clone(),
        }
    })?;

    let (token, activation_result) =
        Token::init_token(req.ticker, platform_coin, req.activation_params, token_protocol).await?;

    let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
    coins_ctx
        .add_coin(token.into())
        .await
        .mm_err(|e| EnableTokenError::TokenIsAlreadyActivated(e.ticker))?;

    Ok(activation_result)
}

impl TryPlatformCoinFromMmCoinEnum for BchCoin {
    fn try_from_mm_coin(coin: MmCoinEnum) -> Option<Self>
    where
        Self: Sized,
    {
        match coin {
            MmCoinEnum::Bch(coin) => Some(coin),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SlpActivationParams {
    required_confirmations: Option<u64>,
}

impl TryTokenProtoFromCoinProto for SlpProtocolConf {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::SLPTOKEN {
                platform,
                token_id,
                decimals,
                required_confirmations,
            } => Ok(SlpProtocolConf {
                platform_coin_ticker: platform,
                token_id: token_id.into(),
                decimals,
                required_confirmations,
            }),
            proto => MmError::err(proto),
        }
    }
}

impl TokenProtocolParams for SlpProtocolConf {
    fn platform_coin_ticker(&self) -> &str { &self.platform_coin_ticker }
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

impl From<SlpInitError> for EnableTokenError {
    fn from(err: SlpInitError) -> Self {
        match err {
            SlpInitError::GetBalanceError(rpc_err) => rpc_err.into(),
            SlpInitError::MyAddressError(e) => EnableTokenError::Internal(e),
        }
    }
}

impl HttpStatusCode for EnableTokenError {
    fn status_code(&self) -> StatusCode {
        match self {
            EnableTokenError::TokenIsAlreadyActivated(_)
            | EnableTokenError::PlatformCoinIsNotActivated(_)
            | EnableTokenError::TokenConfigIsNotFound(_)
            | EnableTokenError::InvalidTokenProtocolConf(_) => StatusCode::BAD_REQUEST,
            EnableTokenError::InvalidCoinProtocol(_)
            | EnableTokenError::UnsupportedPlatformCoin { .. }
            | EnableTokenError::Transport(_)
            | EnableTokenError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SlpInitResult {
    balances: HashMap<String, CoinBalance>,
    token_id: H256Json,
    platform_coin: String,
    required_confirmations: u64,
}

#[derive(Debug)]
pub enum SlpInitError {
    GetBalanceError(UtxoRpcError),
    MyAddressError(String),
}

#[async_trait]
impl TokenActivationOps for SlpToken {
    type PlatformCoin = BchCoin;
    type ActivationParams = SlpActivationParams;
    type ProtocolInfo = SlpProtocolConf;
    type ActivationResult = SlpInitResult;
    type ActivationError = SlpInitError;

    async fn init_token(
        ticker: String,
        platform_coin: Self::PlatformCoin,
        activation_params: Self::ActivationParams,
        protocol_conf: Self::ProtocolInfo,
    ) -> Result<(Self, Self::ActivationResult), MmError<Self::ActivationError>> {
        // confirmation settings from activation params have the highest priority
        let required_confirmations = activation_params.required_confirmations.unwrap_or_else(|| {
            protocol_conf
                .required_confirmations
                .unwrap_or_else(|| platform_coin.required_confirmations())
        });

        let token = Self::new(
            protocol_conf.decimals,
            ticker,
            protocol_conf.token_id,
            platform_coin,
            required_confirmations,
        );
        let balance = token.my_coin_balance().await.mm_err(SlpInitError::GetBalanceError)?;
        let my_address = token.my_address().map_to_mm(SlpInitError::MyAddressError)?;
        let mut balances = HashMap::new();
        balances.insert(my_address, balance);
        let init_result = SlpInitResult {
            balances,
            token_id: (*token.token_id()).into(),
            platform_coin: token.platform_ticker().into(),
            required_confirmations: token.required_confirmations(),
        };
        Ok((token, init_result))
    }
}
