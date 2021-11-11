use async_trait::async_trait;
use coins::{CoinProtocol, MmCoinEnum};
use common::mm_error::prelude::*;

pub trait PlatformWithTokensActivationParams<T> {
    fn get_tokens_for_initializer(&self, initializer: &dyn TokenInitializer<PlatformCoin = T>) -> Vec<String>;
}

pub trait TryPlatformProtoFromCoinProto {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized;
}

pub trait TokenOf {
    type PlatformCoin;
}

pub trait TokenInitializer {
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

    fn token_initializers() -> Vec<Box<dyn TokenInitializer<PlatformCoin = Self>>>;
}
