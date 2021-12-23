use crate::prelude::*;
use crate::token::{EnableTokenError, TokenActivationOps, TokenProtocolParams};
use async_trait::async_trait;
use coins::utxo::bch::BchCoin;
use coins::utxo::rpc_clients::UtxoRpcError;
use coins::utxo::slp::{SlpProtocolConf, SlpToken};
use coins::{CoinBalance, CoinProtocol, MarketCoinOps, MmCoin, MmCoinEnum};
use common::mm_error::prelude::*;
use rpc::v1::types::H256 as H256Json;
use serde_derive::{Deserialize, Serialize};
use std::collections::HashMap;

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

#[derive(Clone, Debug, Deserialize)]
pub struct SlpActivationRequest {
    pub required_confirmations: Option<u64>,
}

impl TryFromCoinProtocol for SlpProtocolConf {
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

impl From<SlpInitError> for EnableTokenError {
    fn from(err: SlpInitError) -> Self {
        match err {
            SlpInitError::GetBalanceError(rpc_err) => rpc_err.into(),
            SlpInitError::MyAddressError(e) => EnableTokenError::Internal(e),
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
#[allow(clippy::large_enum_variant)]
pub enum SlpInitError {
    GetBalanceError(UtxoRpcError),
    MyAddressError(String),
}

#[async_trait]
impl TokenActivationOps for SlpToken {
    type PlatformCoin = BchCoin;
    type ActivationParams = SlpActivationRequest;
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
