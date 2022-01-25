use crate::context::CoinsActivationContext;
use crate::prelude::TryFromCoinProtocol;
use crate::standalone_coin::InitStandaloneCoinActivationOps;
use crate::utxo_activation::init_utxo_standard_activation::{UtxoStandardRpcTaskHandle, UtxoStandardTaskManagerShared};
use crate::utxo_activation::init_utxo_standard_activation_error::InitUtxoStandardError;
use crate::utxo_activation::init_utxo_standard_statuses::{UtxoStandardAwaitingStatus, UtxoStandardInProgressStatus,
                                                          UtxoStandardUserAction};
use crate::utxo_activation::utxo_standard_activation_result::UtxoStandardActivationResult;
use crate::utxo_activation::utxo_standard_coin_hw_ops::UtxoStandardCoinHwOps;
use async_trait::async_trait;
use coins::coin_balance::EnableCoinBalanceOps;
use coins::utxo::qtum::{QtumCoin, QtumCoinBuilder};
use coins::utxo::utxo_builder::UtxoCoinBuilder;
use coins::utxo::UtxoActivationParams;
use coins::{lp_register_coin, CoinProtocol, MarketCoinOps, MmCoinEnum, PrivKeyBuildPolicy, RegisterCoinParams};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::Future01CompatExt;
use serde_json::Value as Json;

pub struct QtumProtocolInfo;

impl TryFromCoinProtocol for QtumProtocolInfo {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::QTUM => Ok(QtumProtocolInfo),
            protocol => MmError::err(protocol),
        }
    }
}

#[async_trait]
impl InitStandaloneCoinActivationOps for QtumCoin {
    type ActivationRequest = UtxoActivationParams;
    type StandaloneProtocol = QtumProtocolInfo;
    type ActivationResult = UtxoStandardActivationResult;
    type ActivationError = InitUtxoStandardError;
    type InProgressStatus = UtxoStandardInProgressStatus;
    type AwaitingStatus = UtxoStandardAwaitingStatus;
    type UserAction = UtxoStandardUserAction;

    fn rpc_task_manager(activation_ctx: &CoinsActivationContext) -> &UtxoStandardTaskManagerShared {
        &activation_ctx.init_utxo_standard_task_manager
    }

    async fn init_standalone_coin(
        ctx: MmArc,
        ticker: String,
        coin_conf: Json,
        activation_request: Self::ActivationRequest,
        _protocol_info: Self::StandaloneProtocol,
        priv_key_policy: PrivKeyBuildPolicy<'_>,
        task_handle: &UtxoStandardRpcTaskHandle,
    ) -> Result<Self, MmError<Self::ActivationError>> {
        let hw_ops = UtxoStandardCoinHwOps::new(&ctx, task_handle);
        let tx_history = activation_request.tx_history;
        let coin = QtumCoinBuilder::new(&ctx, &ticker, &coin_conf, &activation_request, priv_key_policy, hw_ops)
            .build()
            .await
            .mm_err(|e| InitUtxoStandardError::from_build_err(e, ticker.clone()))?;
        lp_register_coin(&ctx, MmCoinEnum::from(coin.clone()), RegisterCoinParams {
            ticker: ticker.clone(),
            tx_history,
        })
        .await
        .mm_err(|e| InitUtxoStandardError::from_register_err(e, ticker))?;
        Ok(coin)
    }

    async fn get_activation_result(
        &self,
        _task_handle: &UtxoStandardRpcTaskHandle,
    ) -> Result<Self::ActivationResult, MmError<Self::ActivationError>> {
        let current_block =
            self.current_block()
                .compat()
                .await
                .map_to_mm(|error| InitUtxoStandardError::CoinCreationError {
                    ticker: self.ticker().to_owned(),
                    error,
                })?;
        let wallet_balance =
            self.enable_coin_balance()
                .await
                .mm_err(|error| InitUtxoStandardError::CoinCreationError {
                    ticker: self.ticker().to_owned(),
                    error: error.to_string(),
                })?;
        let result = UtxoStandardActivationResult {
            current_block,
            wallet_balance,
        };
        Ok(result)
    }
}
