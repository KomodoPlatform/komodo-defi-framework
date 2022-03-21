use crate::utxo::utxo_block_header_storage::BlockHeaderStorage;
use crate::utxo::utxo_builder::{UtxoCoinBuildError, UtxoCoinBuilder, UtxoCoinBuilderCommonOps,
                                UtxoFieldsWithHardwareWalletBuilder, UtxoFieldsWithIguanaPrivKeyBuilder};
use crate::utxo::utxo_common::{block_header_utxo_loop, merge_utxo_loop};
use crate::utxo::{UtxoArc, UtxoCoinFields, UtxoCommonOps, UtxoWeak};
use crate::{PrivKeyBuildPolicy, UtxoActivationParams};
use async_trait::async_trait;
use common::executor::spawn;
use common::log::info;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use futures::future::{abortable, AbortHandle};
use serde_json::Value as Json;

pub struct UtxoArcBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
{
    ctx: &'a MmArc,
    ticker: &'a str,
    conf: &'a Json,
    activation_params: &'a UtxoActivationParams,
    priv_key_policy: PrivKeyBuildPolicy<'a>,
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
        constructor: F,
    ) -> UtxoArcBuilder<'a, F, T> {
        UtxoArcBuilder {
            ctx,
            ticker,
            conf,
            activation_params,
            priv_key_policy,
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
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    type ResultCoin = T;
    type Error = UtxoCoinBuildError;

    fn priv_key_policy(&self) -> PrivKeyBuildPolicy<'_> { self.priv_key_policy.clone() }

    async fn build(self) -> MmResult<Self::ResultCoin, Self::Error> {
        let utxo = self.build_utxo_fields().await?;
        let utxo_arc = UtxoArc::new(utxo);
        let utxo_weak = utxo_arc.downgrade();
        let result_coin = (self.constructor)(utxo_arc);

        self.spawn_merge_utxo_loop_if_required(utxo_weak.clone(), self.constructor.clone());
        if let Some(abort_handler) = self.spawn_block_header_utxo_loop_if_required(
            utxo_weak,
            &result_coin.as_ref().block_headers_storage,
            self.constructor.clone(),
        ) {
            self.ctx.abort_handlers.lock().unwrap().push(abort_handler);
        }
        Ok(result_coin)
    }
}

impl<'a, F, T> MergeUtxoArcOps<T> for UtxoArcBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
}

impl<'a, F, T> BlockHeaderUtxoArcOps<T> for UtxoArcBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
}

pub trait MergeUtxoArcOps<T>: UtxoCoinBuilderCommonOps
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    fn spawn_merge_utxo_loop_if_required<F>(&self, weak: UtxoWeak, constructor: F)
    where
        F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    {
        if let Some(ref merge_params) = self.activation_params().utxo_merge_params {
            let fut = merge_utxo_loop(
                weak,
                merge_params.merge_at,
                merge_params.check_every,
                merge_params.max_merge_at_once,
                constructor,
            );
            info!("Starting UTXO merge loop for coin {}", self.ticker());
            spawn(fut);
        }
    }
}

pub trait BlockHeaderUtxoArcOps<T>: UtxoCoinBuilderCommonOps
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    fn spawn_block_header_utxo_loop_if_required<F>(
        &self,
        weak: UtxoWeak,
        maybe_storage: &Option<BlockHeaderStorage>,
        constructor: F,
    ) -> Option<AbortHandle>
    where
        F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    {
        if maybe_storage.is_some() {
            let ticker = self.ticker().to_owned();
            let (fut, abort_handle) = abortable(block_header_utxo_loop(weak, constructor));
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
