use crate::platform_coin_with_tokens::{EnablePlatformCoinWithTokensError, GetPlatformBalance,
                                       PlatformWithTokensActivationOps, TokenAsMmCoinInitializer};
use crate::prelude::*;
use async_trait::async_trait;
use coins::my_tx_history_v2::TxHistoryStorage;
use coins::tendermint::{TendermintActivationParams, TendermintCoin, TendermintInitError, TendermintInitErrorKind,
                        TendermintProtocolInfo};
use coins::{CoinBalance, CoinProtocol, MarketCoinOps};
use common::Future01CompatExt;
use futures::future::AbortHandle;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_metrics::MetricsArc;
use mm2_number::BigDecimal;
use serde::Serialize;
use serde_json::Value as Json;

impl TxHistory for TendermintActivationParams {
    fn tx_history(&self) -> bool { false }
}

impl TryFromCoinProtocol for TendermintProtocolInfo {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>> {
        match proto {
            CoinProtocol::TENDERMINT(proto) => Ok(proto),
            other => MmError::err(other),
        }
    }
}

#[derive(Serialize)]
pub struct TendermintActivationResult {
    address: String,
    current_block: u64,
    balance: CoinBalance,
    ticker: String,
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
        TendermintCoin::init(ticker, protocol_conf, activation_request, priv_key).await
    }

    fn token_initializers(
        &self,
    ) -> Vec<Box<dyn TokenAsMmCoinInitializer<PlatformCoin = Self, ActivationRequest = Self::ActivationRequest>>> {
        Vec::new()
    }

    async fn get_activation_result(&self) -> Result<Self::ActivationResult, MmError<Self::ActivationError>> {
        let current_block = self.current_block().compat().await.map_to_mm(|e| TendermintInitError {
            ticker: self.ticker().to_owned(),
            kind: TendermintInitErrorKind::RpcError(e),
        })?;

        let balance = self.my_balance().compat().await.mm_err(|e| TendermintInitError {
            ticker: self.ticker().to_owned(),
            kind: TendermintInitErrorKind::RpcError(e.to_string()),
        })?;

        Ok(TendermintActivationResult {
            address: self.account_id.to_string(),
            current_block,
            balance,
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
