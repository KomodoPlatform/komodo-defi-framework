use crate::context::CoinsActivationContext;
use crate::prelude::TryFromCoinProtocol;
use crate::standalone_coin::{InitStandaloneCoinActivationOps, InitStandaloneCoinTaskHandle,
                             InitStandaloneCoinTaskManagerShared};
use crate::utxo_activation::common_impl::{get_activation_result, priv_key_build_policy};
use crate::utxo_activation::init_utxo_standard_activation_error::InitUtxoStandardError;
use crate::utxo_activation::init_utxo_standard_statuses::{UtxoStandardAwaitingStatus, UtxoStandardInProgressStatus,
                                                          UtxoStandardUserAction};
use crate::utxo_activation::utxo_standard_activation_result::UtxoStandardActivationResult;
use async_trait::async_trait;
use coins::utxo::utxo_builder::{UtxoArcBuilder, UtxoCoinBuilder, UtxoSyncStatus, UtxoSyncStatusLoopHandle};
use coins::utxo::utxo_standard::UtxoStandardCoin;
use coins::utxo::UtxoActivationParams;
use coins::CoinProtocol;
use crypto::CryptoCtx;
use futures::channel::mpsc::channel;
use futures::StreamExt;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
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
        activation_request: &Self::ActivationRequest,
        _protocol_info: Self::StandaloneProtocol,
        task_handle: &UtxoStandardRpcTaskHandle,
    ) -> MmResult<Self, InitUtxoStandardError> {
        let crypto_ctx = CryptoCtx::from_ctx(&ctx)?;
        let priv_key_policy = priv_key_build_policy(&crypto_ctx, activation_request.priv_key_policy);

        // todo: add a function for this instead of coin.as_ref().conf.enable_spv_proof && !coin.as_ref().rpc_client.is_native()
        // Todo: should this step be used before this or maybe inside UtxoArcBuilder??
        // Todo: channel can be used to send errors after coin init to rotate_servers etc..
        let (sync_status_loop_handle, maybe_sync_watcher) =
            if coin_conf["enable_spv_proof"].as_bool().unwrap_or(false) && !activation_request.mode.is_native() {
                let (sync_status_notifier, sync_watcher) = channel(1);
                (
                    Some(UtxoSyncStatusLoopHandle::new(sync_status_notifier)),
                    Some(sync_watcher),
                )
            } else {
                (None, None)
            };

        let coin = UtxoArcBuilder::new(
            &ctx,
            &ticker,
            &coin_conf,
            activation_request,
            priv_key_policy,
            sync_status_loop_handle,
            UtxoStandardCoin::from,
        )
        .build()
        .await
        .mm_err(|e| InitUtxoStandardError::from_build_err(e, ticker.clone()))?;

        if let Some(mut sync_watcher) = maybe_sync_watcher {
            loop {
                let in_progress_status =
                    match sync_watcher
                        .next()
                        .await
                        .ok_or(InitUtxoStandardError::CoinCreationError {
                            ticker: ticker.clone(),
                            error: "Error waiting for block headers synchronization status!".into(),
                        })? {
                        UtxoSyncStatus::SyncingBlockHeaders {
                            current_scanned_block,
                            last_block,
                        } => UtxoStandardInProgressStatus::SyncingBlockHeaders {
                            current_scanned_block,
                            last_block,
                        },
                        UtxoSyncStatus::TemporaryError(e) => UtxoStandardInProgressStatus::TemporaryError(e),
                        UtxoSyncStatus::PermanentError(e) => {
                            return Err(InitUtxoStandardError::CoinCreationError {
                                ticker: ticker.clone(),
                                error: e,
                            }
                            .into())
                        },
                        UtxoSyncStatus::Finished { .. } => break,
                    };
                task_handle.update_in_progress_status(in_progress_status)?;
            }
        }

        Ok(coin)
    }

    async fn get_activation_result(
        &self,
        ctx: MmArc,
        task_handle: &UtxoStandardRpcTaskHandle,
        activation_request: &Self::ActivationRequest,
    ) -> MmResult<Self::ActivationResult, InitUtxoStandardError> {
        get_activation_result(&ctx, self, task_handle, activation_request).await
    }
}
