use crate::context::CoinsActivationContext;
use crate::prelude::TryFromCoinProtocol;
use crate::standalone_coin::{InitStandaloneCoinActivationOps, InitStandaloneCoinTaskHandle,
                             InitStandaloneCoinTaskManagerShared};
use crate::utxo_activation::common_impl::{get_activation_result, xpub_extractor_rpc_statuses};
use crate::utxo_activation::init_utxo_standard_activation_error::InitUtxoStandardError;
use crate::utxo_activation::init_utxo_standard_statuses::{UtxoStandardAwaitingStatus, UtxoStandardInProgressStatus,
                                                          UtxoStandardUserAction};
use crate::utxo_activation::utxo_standard_activation_result::UtxoStandardActivationResult;
use async_trait::async_trait;
use coins::hd_pubkey::RpcTaskXPubExtractor;
use coins::utxo::utxo_builder::{UtxoArcBuilder, UtxoCoinBuilder};
use coins::utxo::utxo_standard::UtxoStandardCoin;
use coins::utxo::UtxoActivationParams;
use coins::{lp_register_coin, CoinProtocol, MmCoinEnum, PrivKeyBuildPolicy, RegisterCoinParams};
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use serde_json::Value as Json;

pub type UtxoStandardTaskManagerShared = InitStandaloneCoinTaskManagerShared<UtxoStandardCoin>;
pub type UtxoStandardRpcTaskHandle = InitStandaloneCoinTaskHandle<UtxoStandardCoin>;

pub struct UtxoStandardProtocolInfo;

impl TryFromCoinProtocol for UtxoStandardProtocolInfo {
    fn try_from_coin_protocol(proto: CoinProtocol) -> Result<Self, MmError<CoinProtocol>>
    where
        Self: Sized,
    {
        match proto {
            CoinProtocol::UTXO => Ok(UtxoStandardProtocolInfo),
            protocol => MmError::err(protocol),
        }
    }
}

#[async_trait]
impl InitStandaloneCoinActivationOps for UtxoStandardCoin {
    type ActivationRequest = UtxoActivationParams;
    type StandaloneProtocol = UtxoStandardProtocolInfo;
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
    ) -> MmResult<Self, InitUtxoStandardError> {
        // Construct an Xpub extractor without checking if the MarketMaker supports HD wallet ops.
        // If the coin builder tries to extract an extended public key despite HD wallet is not supported,
        // [`UtxoCoinBuilder::build`] fails with the [`UtxoCoinBuildError::IguanaPrivKeyNotAllowed`] error.
        let xpub_extractor = RpcTaskXPubExtractor::new_unchecked(&ctx, task_handle, xpub_extractor_rpc_statuses());
        let tx_history = activation_request.tx_history;
        let coin = UtxoArcBuilder::new(
            &ctx,
            &ticker,
            &coin_conf,
            &activation_request,
            priv_key_policy,
            xpub_extractor,
            UtxoStandardCoin::from,
        )
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
        task_handle: &UtxoStandardRpcTaskHandle,
    ) -> MmResult<Self::ActivationResult, InitUtxoStandardError> {
        get_activation_result(self, task_handle).await
    }
}
