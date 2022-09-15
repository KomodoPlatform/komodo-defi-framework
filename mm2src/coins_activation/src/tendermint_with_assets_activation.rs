use crate::platform_coin_with_tokens::{EnablePlatformCoinWithTokensError, GetPlatformBalance,
                                       InitTokensAsMmCoinsError, PlatformWithTokensActivationOps, RegisterTokenInfo,
                                       TokenActivationParams, TokenActivationRequest, TokenAsMmCoinInitializer,
                                       TokenInitializer, TokenOf};
use crate::prelude::*;
use async_trait::async_trait;
use coins::my_tx_history_v2::TxHistoryStorage;
use coins::tendermint::{IbcAssetActivationParams, IbcAssetInitError, IbcAssetProtocolInfo, TendermintCoin,
                        TendermintIbcAsset, TendermintInitError, TendermintInitErrorKind, TendermintProtocolInfo};
use coins::{CoinBalance, CoinProtocol, MarketCoinOps};
use common::Future01CompatExt;
use futures::future::AbortHandle;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_metrics::MetricsArc;
use mm2_number::BigDecimal;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::HashMap;

impl TokenOf for TendermintIbcAsset {
    type PlatformCoin = TendermintCoin;
}

impl RegisterTokenInfo<TendermintIbcAsset> for TendermintCoin {
    fn register_token_info(&self, token: &TendermintIbcAsset) {
        self.add_activated_ibc_asset_info(token.ticker.clone(), token.decimals, token.denom.clone())
    }
}

#[derive(Clone, Deserialize)]
pub struct TendermintActivationParams {
    rpc_urls: Vec<String>,
    pub ibc_assets_params: Vec<TokenActivationRequest<IbcAssetActivationParams>>,
}

impl TxHistory for TendermintActivationParams {
    fn tx_history(&self) -> bool { false }
}

struct IbcAssetInitializer {
    platform_coin: TendermintCoin,
}

#[async_trait]
impl TokenInitializer for IbcAssetInitializer {
    type Token = TendermintIbcAsset;
    type TokenActivationRequest = IbcAssetActivationParams;
    type TokenProtocol = IbcAssetProtocolInfo;
    type InitTokensError = IbcAssetInitError;

    fn tokens_requests_from_platform_request(
        platform_request: &TendermintActivationParams,
    ) -> Vec<TokenActivationRequest<Self::TokenActivationRequest>> {
        platform_request.ibc_assets_params.clone()
    }

    async fn enable_tokens(
        &self,
        params: Vec<TokenActivationParams<Self::TokenActivationRequest, Self::TokenProtocol>>,
    ) -> Result<Vec<Self::Token>, MmError<Self::InitTokensError>> {
        params
            .into_iter()
            .map(|param| {
                TendermintIbcAsset::new(
                    param.ticker,
                    self.platform_coin.clone(),
                    param.protocol.decimals,
                    param.protocol.denom,
                )
            })
            .collect()
    }

    fn platform_coin(&self) -> &<Self::Token as TokenOf>::PlatformCoin { &self.platform_coin }
}

impl TryFromCoinProtocol for TendermintProtocolInfo {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>> {
        match proto {
            CoinProtocol::TENDERMINT(proto) => Ok(proto),
            other => MmError::err(other),
        }
    }
}

impl TryFromCoinProtocol for IbcAssetProtocolInfo {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>> {
        match proto {
            CoinProtocol::TENDERMINTIBC(proto) => Ok(proto),
            other => MmError::err(other),
        }
    }
}

impl From<IbcAssetInitError> for InitTokensAsMmCoinsError {
    fn from(err: IbcAssetInitError) -> Self {
        match err {
            IbcAssetInitError::InvalidDenom(error) => InitTokensAsMmCoinsError::TokenProtocolParseError {
                ticker: "".into(),
                error,
            },
        }
    }
}

#[derive(Serialize)]
pub struct TendermintActivationResult {
    ticker: String,
    address: String,
    current_block: u64,
    balance: CoinBalance,
    ibc_assets_balances: HashMap<String, CoinBalance>,
}

impl CurrentBlock for TendermintActivationResult {
    fn current_block(&self) -> u64 { self.current_block }
}

impl GetPlatformBalance for TendermintActivationResult {
    fn get_platform_balance(&self) -> BigDecimal { self.balance.spendable.clone() }
}

impl From<TendermintInitError> for EnablePlatformCoinWithTokensError {
    fn from(err: TendermintInitError) -> Self {
        EnablePlatformCoinWithTokensError::PlatformCoinCreationError {
            ticker: err.ticker,
            error: err.kind.to_string(),
        }
    }
}

#[async_trait]
#[allow(unused_variables)]
impl PlatformWithTokensActivationOps for TendermintCoin {
    type ActivationRequest = TendermintActivationParams;
    type PlatformProtocolInfo = TendermintProtocolInfo;
    type ActivationResult = TendermintActivationResult;
    type ActivationError = TendermintInitError;

    async fn enable_platform_coin(
        _ctx: MmArc,
        ticker: String,
        _coin_conf: Json,
        activation_request: Self::ActivationRequest,
        protocol_conf: Self::PlatformProtocolInfo,
        priv_key: &[u8],
    ) -> Result<Self, MmError<Self::ActivationError>> {
        TendermintCoin::init(ticker, protocol_conf, activation_request.rpc_urls, priv_key).await
    }

    fn token_initializers(
        &self,
    ) -> Vec<Box<dyn TokenAsMmCoinInitializer<PlatformCoin = Self, ActivationRequest = Self::ActivationRequest>>> {
        vec![Box::new(IbcAssetInitializer {
            platform_coin: self.clone(),
        })]
    }

    async fn get_activation_result(&self) -> Result<Self::ActivationResult, MmError<Self::ActivationError>> {
        let current_block = self.current_block().compat().await.map_to_mm(|e| TendermintInitError {
            ticker: self.ticker().to_owned(),
            kind: TendermintInitErrorKind::RpcError(e),
        })?;

        let balances = self.all_balances().await.mm_err(|e| TendermintInitError {
            ticker: self.ticker().to_owned(),
            kind: TendermintInitErrorKind::RpcError(e.to_string()),
        })?;

        Ok(TendermintActivationResult {
            address: self.account_id.to_string(),
            current_block,
            balance: CoinBalance {
                spendable: balances.platform_balance,
                unspendable: BigDecimal::default(),
            },
            ibc_assets_balances: balances
                .ibc_assets_balances
                .into_iter()
                .map(|(ticker, balance)| {
                    (ticker, CoinBalance {
                        spendable: balance,
                        unspendable: BigDecimal::default(),
                    })
                })
                .collect(),
            ticker: self.ticker().to_owned(),
        })
    }

    fn start_history_background_fetching(
        &self,
        metrics: MetricsArc,
        storage: impl TxHistoryStorage,
        initial_balance: BigDecimal,
    ) -> AbortHandle {
        unimplemented!()
    }
}
