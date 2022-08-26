use crate::utxo::utxo_builder::{UtxoCoinBuildError, UtxoCoinBuilder, UtxoCoinBuilderCommonOps,
                                UtxoFieldsWithHardwareWalletBuilder, UtxoFieldsWithIguanaPrivKeyBuilder};
use crate::utxo::utxo_common::{block_header_utxo_loop, merge_utxo_loop};
use crate::utxo::{GetUtxoListOps, UtxoArc, UtxoCommonOps, UtxoWeak};
use crate::{PrivKeyBuildPolicy, UtxoActivationParams};
use async_trait::async_trait;
use common::executor::spawn;
use common::log::{info, LogOnError};
use futures::channel::mpsc::Sender as AsyncSender;
use futures::future::{abortable, AbortHandle};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use serde_json::Value as Json;

pub enum UtxoSyncStatus {
    SyncingBlockHeaders {
        current_scanned_block: u64,
        last_block: u64,
    },
    TemporaryError(String),
    PermanentError(String),
    Finished {
        block_number: u64,
    },
}

#[derive(Clone)]
pub struct UtxoSyncStatusLoopHandle(AsyncSender<UtxoSyncStatus>);

impl UtxoSyncStatusLoopHandle {
    pub fn new(sync_status_notifier: AsyncSender<UtxoSyncStatus>) -> Self {
        UtxoSyncStatusLoopHandle(sync_status_notifier)
    }

    pub fn notify_blocks_headers_sync_status(&mut self, current_scanned_block: u64, last_block: u64) {
        self.0
            .try_send(UtxoSyncStatus::SyncingBlockHeaders {
                current_scanned_block,
                last_block,
            })
            .debug_log_with_msg("No one seems interested in UtxoSyncStatus");
    }

    pub fn notify_on_temp_error(&mut self, error: String) {
        self.0
            .try_send(UtxoSyncStatus::TemporaryError(error))
            .debug_log_with_msg("No one seems interested in UtxoSyncStatus");
    }

    pub fn notify_on_permanent_error(&mut self, error: String) {
        self.0
            .try_send(UtxoSyncStatus::PermanentError(error))
            .debug_log_with_msg("No one seems interested in UtxoSyncStatus");
    }

    pub fn notify_sync_finished(&mut self, block_number: u64) {
        self.0
            .try_send(UtxoSyncStatus::Finished { block_number })
            .debug_log_with_msg("No one seems interested in UtxoSyncStatus");
    }
}

pub struct UtxoArcBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
{
    ctx: &'a MmArc,
    ticker: &'a str,
    conf: &'a Json,
    activation_params: &'a UtxoActivationParams,
    priv_key_policy: PrivKeyBuildPolicy<'a>,
    sync_status_loop_handle: Option<UtxoSyncStatusLoopHandle>,
    constructor: F,
}

impl<'a, F, T> UtxoArcBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
{
    pub fn new(
        ctx: &'a MmArc,
        ticker: &'a str,
        conf: &'a Json,
        activation_params: &'a UtxoActivationParams,
        priv_key_policy: PrivKeyBuildPolicy<'a>,
        sync_status_loop_handle: Option<UtxoSyncStatusLoopHandle>,
        constructor: F,
    ) -> UtxoArcBuilder<'a, F, T> {
        UtxoArcBuilder {
            ctx,
            ticker,
            conf,
            activation_params,
            priv_key_policy,
            sync_status_loop_handle,
            constructor,
        }
    }
}

#[async_trait]
impl<'a, F, T> UtxoCoinBuilderCommonOps for UtxoArcBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
{
    fn ctx(&self) -> &MmArc { self.ctx }

    fn conf(&self) -> &Json { self.conf }

    fn activation_params(&self) -> &UtxoActivationParams { self.activation_params }

    fn ticker(&self) -> &str { self.ticker }

    fn sync_status_loop_handle(&self) -> Option<UtxoSyncStatusLoopHandle> { self.sync_status_loop_handle.clone() }
}

impl<'a, F, T> UtxoFieldsWithIguanaPrivKeyBuilder for UtxoArcBuilder<'a, F, T> where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static
{
}

impl<'a, F, T> UtxoFieldsWithHardwareWalletBuilder for UtxoArcBuilder<'a, F, T> where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static
{
}

#[async_trait]
impl<'a, F, T> UtxoCoinBuilder for UtxoArcBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Clone + Send + Sync + 'static,
    T: UtxoCommonOps + GetUtxoListOps,
{
    type ResultCoin = T;
    type Error = UtxoCoinBuildError;

    fn priv_key_policy(&self) -> PrivKeyBuildPolicy<'_> { self.priv_key_policy.clone() }

    async fn build(self) -> MmResult<Self::ResultCoin, Self::Error> {
        let utxo = self.build_utxo_fields().await?;
        let utxo_arc = UtxoArc::new(utxo);
        let utxo_weak = utxo_arc.downgrade();
        let result_coin = (self.constructor)(utxo_arc);

        if let Some(abort_handler) = self.spawn_merge_utxo_loop_if_required(utxo_weak.clone(), self.constructor.clone())
        {
            self.ctx.abort_handlers.lock().unwrap().push(abort_handler);
        }

        // Todo: find a better way for this
        if let Some(abort_handler) = self.spawn_block_header_utxo_loop_if_required(utxo_weak, self.constructor.clone())
        {
            self.ctx.abort_handlers.lock().unwrap().push(abort_handler);
        }
        Ok(result_coin)
    }
}

impl<'a, F, T> MergeUtxoArcOps<T> for UtxoArcBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    T: UtxoCommonOps + GetUtxoListOps,
{
}

impl<'a, F, T> BlockHeaderUtxoArcOps<T> for UtxoArcBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    T: UtxoCommonOps,
{
}

pub trait MergeUtxoArcOps<T: UtxoCommonOps + GetUtxoListOps>: UtxoCoinBuilderCommonOps {
    fn spawn_merge_utxo_loop_if_required<F>(&self, weak: UtxoWeak, constructor: F) -> Option<AbortHandle>
    where
        F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    {
        if let Some(ref merge_params) = self.activation_params().utxo_merge_params {
            let (fut, abort_handle) = abortable(merge_utxo_loop(
                weak,
                merge_params.merge_at,
                merge_params.check_every,
                merge_params.max_merge_at_once,
                constructor,
            ));
            let ticker = self.ticker().to_owned();
            info!("Starting UTXO merge loop for coin {}", ticker);
            spawn(async move {
                if let Err(e) = fut.await {
                    info!("spawn_merge_utxo_loop_if_required stopped for {}, reason {}", ticker, e);
                }
            });
            return Some(abort_handle);
        }
        None
    }
}

pub trait BlockHeaderUtxoArcOps<T>: UtxoCoinBuilderCommonOps {
    // Todo: this should be called only if storing headers is enabled and should be called after syncing the latest header on coin activation
    // Todo: probably this function needs to be refactored
    fn spawn_block_header_utxo_loop_if_required<F>(&self, weak: UtxoWeak, constructor: F) -> Option<AbortHandle>
    where
        F: Fn(UtxoArc) -> T + Send + Sync + 'static,
        T: UtxoCommonOps,
    {
        // Todo:  add condition for enable_spv_proof (should block headers be saved when enable_spv_proof is true only? what about for getting tx height?)
        if let Some(sync_status_loop_handle) = self.sync_status_loop_handle() {
            let ticker = self.ticker().to_owned();
            let (fut, abort_handle) = abortable(block_header_utxo_loop(weak, constructor, sync_status_loop_handle));
            info!("Starting UTXO block header loop for coin {}", ticker);
            spawn(async move {
                if let Err(e) = fut.await {
                    info!(
                        "spawn_block_header_utxo_loop_if_required stopped for {}, reason {}",
                        ticker, e
                    );
                }
            });
            return Some(abort_handle);
        }

        None
    }
}
