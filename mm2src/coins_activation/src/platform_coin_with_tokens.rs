use async_trait::async_trait;
use coins::utxo::bch::BchCoin;
use coins::utxo::slp::SlpToken;
use coins::{coin_conf, lp_coinfind, CoinProtocol, MmCoinEnum};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::NotSame;
use derive_more::Display;
use ser_error_derive::SerializeErrorType;
use serde_derive::Serialize;
use serde_json::{self as json};

pub trait PlatformWithTokensActivationParams<T> {
    fn get_tokens_for_initializer(&self, initializer: &dyn TokenAsMmCoinInitializer<PlatformCoin = T>) -> Vec<String>;
}

pub trait TryPlatformProtoFromCoinProto {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized;
}

pub trait TokenOf: Into<MmCoinEnum> {
    type PlatformCoin;
}

#[async_trait]
pub trait TokenInitializer {
    type Token: TokenOf;

    async fn init_tokens(self) -> Result<Vec<Self::Token>, MmError<()>>;
}

pub trait TokenAsMmCoinInitializer {
    type PlatformCoin;

    fn init_tokens(self) -> Vec<MmCoinEnum>;
}

#[async_trait]
pub trait PlatformWithTokensActivationOps: Into<MmCoinEnum> {
    type ActivationParams: PlatformWithTokensActivationParams<Self>;
    type PlatformProtocolInfo: TryPlatformProtoFromCoinProto;
    type ActivationResult;
    type ActivationError: NotMmError;

    /// Initializes the platform coin itself
    async fn init_platform_coin(
        ticker: String,
        activation_params: Self::ActivationParams,
        protocol_conf: Self::PlatformProtocolInfo,
    ) -> Result<Self, MmError<Self::ActivationError>>;

    fn token_initializers() -> Vec<Box<dyn TokenAsMmCoinInitializer<PlatformCoin = Self>>>;
}

struct SlpTokenInitializer {
    #[allow(dead_code)]
    platform_coin: BchCoin,
}

impl TokenOf for SlpToken {
    type PlatformCoin = BchCoin;
}

#[async_trait]
impl TokenInitializer for SlpTokenInitializer {
    type Token = SlpToken;

    async fn init_tokens(self) -> Result<Vec<SlpToken>, MmError<()>> { unimplemented!() }
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
    InvalidPlatformProtocolConf(String),
    #[display(fmt = "Invalid coin protocol {:?}", _0)]
    InvalidPlatformProtocol(CoinProtocol),
    Transport(String),
    Internal(String),
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

    let conf = coin_conf(&ctx, &req.ticker);
    if conf.is_null() {
        return MmError::err(EnablePlatformCoinWithTokensError::PlatformConfigIsNotFound(req.ticker));
    }

    let coin_protocol: CoinProtocol = json::from_value(conf["protocol"].clone())
        .map_to_mm(|e| EnablePlatformCoinWithTokensError::InvalidPlatformProtocolConf(e.to_string()))?;

    let platform_protocol = Platform::PlatformProtocolInfo::try_from_coin_protocol(coin_protocol)
        .mm_err(EnablePlatformCoinWithTokensError::InvalidPlatformProtocol)?;

    let _platform_coin = Platform::init_platform_coin(req.ticker, req.activation_params, platform_protocol).await?;
    for _initializer in Platform::token_initializers() {}
    unimplemented!()
}
