#![allow(dead_code)]
#![allow(unused_variables)]

use async_trait::async_trait;
use coins::{
    solana::{SolanaCoin, SolanaToken, SolanaTokenInitError, SolanaTokenProtocolInfo},
    CoinProtocol,
};
use mm2_err_handle::prelude::*;

use crate::{
    platform_coin_with_tokens::TokenOf,
    prelude::TryFromCoinProtocol,
    token::{TokenActivationOps, TokenProtocolParams},
};

pub struct SolanaTokenActivationParams {}
pub struct SolanaTokenInitResult {}

impl TokenOf for SolanaToken {
    type PlatformCoin = SolanaCoin;
}

impl TryFromCoinProtocol for SolanaTokenProtocolInfo {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>> {
        match proto {
            CoinProtocol::SOLANATOKEN(proto) => Ok(proto),
            other => MmError::err(other),
        }
    }
}

impl TokenProtocolParams for SolanaTokenProtocolInfo {
    fn platform_coin_ticker(&self) -> &str {
        &self.platform
    }
}

#[async_trait]
impl TokenActivationOps for SolanaToken {
    type ActivationParams = SolanaTokenActivationParams;
    type ProtocolInfo = SolanaTokenProtocolInfo;
    type ActivationResult = SolanaTokenInitResult;
    type ActivationError = SolanaTokenInitError;

    async fn enable_token(
        ticker: String,
        platform_coin: Self::PlatformCoin,
        _activation_params: Self::ActivationParams,
        _token_conf: serde_json::Value,
        protocol_conf: Self::ProtocolInfo,
        _is_custom: bool,
    ) -> Result<(Self, Self::ActivationResult), MmError<Self::ActivationError>> {
        todo!()
    }
}
