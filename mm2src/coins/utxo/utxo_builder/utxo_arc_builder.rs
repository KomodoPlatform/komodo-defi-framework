use crate::hd_pubkey::HDXPubExtractor;
use crate::utxo::utxo_builder::{UtxoCoinBuildError, UtxoCoinBuilder, UtxoCoinBuilderCommonOps,
                                UtxoCoinWithIguanaPrivKeyBuilder, UtxoFieldsWithHardwareWalletBuilder,
                                UtxoFieldsWithIguanaPrivKeyBuilder};
use crate::utxo::utxo_common::merge_utxo_loop;
use crate::utxo::{UtxoArc, UtxoCoinFields, UtxoCommonOps, UtxoWeak};
use crate::{PrivKeyBuildPolicy, UtxoActivationParams};
use async_trait::async_trait;
use common::executor::spawn;
use common::log::info;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use serde_json::Value as Json;

pub struct UtxoArcBuilder<'a, F, T, XPubExtractor>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    XPubExtractor: HDXPubExtractor + Send + Sync,
{
    ctx: &'a MmArc,
    ticker: &'a str,
    conf: &'a Json,
    activation_params: &'a UtxoActivationParams,
    priv_key_policy: PrivKeyBuildPolicy<'a>,
    xpub_extractor: XPubExtractor,
    constructor: F,
}

impl<'a, F, T, XPubExtractor> UtxoArcBuilder<'a, F, T, XPubExtractor>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    XPubExtractor: HDXPubExtractor + Send + Sync,
{
    pub fn new(
        ctx: &'a MmArc,
        ticker: &'a str,
        conf: &'a Json,
        activation_params: &'a UtxoActivationParams,
        priv_key_policy: PrivKeyBuildPolicy<'a>,
        xpub_extractor: XPubExtractor,
        constructor: F,
    ) -> UtxoArcBuilder<'a, F, T, XPubExtractor> {
        UtxoArcBuilder {
            ctx,
            ticker,
            conf,
            activation_params,
            priv_key_policy,
            xpub_extractor,
            constructor,
        }
    }
}

#[async_trait]
impl<'a, F, T, XPubExtractor> UtxoCoinBuilderCommonOps for UtxoArcBuilder<'a, F, T, XPubExtractor>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    XPubExtractor: HDXPubExtractor + Send + Sync,
{
    fn ctx(&self) -> &MmArc { self.ctx }

    fn conf(&self) -> &Json { self.conf }

    fn activation_params(&self) -> &UtxoActivationParams { self.activation_params }

    fn ticker(&self) -> &str { self.ticker }
}

impl<'a, F, T, XPubExtractor> UtxoFieldsWithIguanaPrivKeyBuilder for UtxoArcBuilder<'a, F, T, XPubExtractor>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    XPubExtractor: HDXPubExtractor + Send + Sync,
{
}

impl<'a, F, T, XPubExtractor> UtxoFieldsWithHardwareWalletBuilder<XPubExtractor>
    for UtxoArcBuilder<'a, F, T, XPubExtractor>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    XPubExtractor: HDXPubExtractor + Send + Sync,
{
}

#[async_trait]
impl<'a, F, T, XPubExtractor> UtxoCoinBuilder<XPubExtractor> for UtxoArcBuilder<'a, F, T, XPubExtractor>
where
    F: Fn(UtxoArc) -> T + Clone + Send + Sync + 'static,
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
    XPubExtractor: HDXPubExtractor + Send + Sync,
{
    type ResultCoin = T;
    type Error = UtxoCoinBuildError;

    fn priv_key_policy(&self) -> PrivKeyBuildPolicy<'_> { self.priv_key_policy.clone() }

    fn xpub_extractor(&self) -> &XPubExtractor { &self.xpub_extractor }

    async fn build(self) -> MmResult<Self::ResultCoin, Self::Error> {
        let utxo = self.build_utxo_fields().await?;
        let utxo_arc = UtxoArc::new(utxo);
        let utxo_weak = utxo_arc.downgrade();
        let result_coin = (self.constructor)(utxo_arc);

        self.spawn_merge_utxo_loop_if_required(utxo_weak, self.constructor.clone());
        Ok(result_coin)
    }
}

impl<'a, F, T, XPubExtractor> MergeUtxoArcOps<T> for UtxoArcBuilder<'a, F, T, XPubExtractor>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
    XPubExtractor: HDXPubExtractor + Send + Sync,
{
}

pub struct UtxoArcWithIguanaPrivKeyBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
{
    ctx: &'a MmArc,
    ticker: &'a str,
    conf: &'a Json,
    activation_params: &'a UtxoActivationParams,
    priv_key: &'a [u8],
    constructor: F,
}

impl<'a, F, T> UtxoCoinBuilderCommonOps for UtxoArcWithIguanaPrivKeyBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
{
    fn ctx(&self) -> &MmArc { self.ctx }

    fn conf(&self) -> &Json { self.conf }

    fn activation_params(&self) -> &UtxoActivationParams { self.activation_params }

    fn ticker(&self) -> &str { self.ticker }
}

impl<'a, F, T> UtxoFieldsWithIguanaPrivKeyBuilder for UtxoArcWithIguanaPrivKeyBuilder<'a, F, T> where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static
{
}

impl<'a, F, T> MergeUtxoArcOps<T> for UtxoArcWithIguanaPrivKeyBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
}

#[async_trait]
impl<'a, F, T> UtxoCoinWithIguanaPrivKeyBuilder for UtxoArcWithIguanaPrivKeyBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Clone + Send + Sync + 'static,
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    type ResultCoin = T;
    type Error = UtxoCoinBuildError;

    fn priv_key(&self) -> &[u8] { self.priv_key }

    async fn build(self) -> MmResult<Self::ResultCoin, Self::Error> {
        let utxo = self.build_utxo_fields_with_iguana_priv_key(self.priv_key()).await?;
        let utxo_arc = UtxoArc::new(utxo);
        let utxo_weak = utxo_arc.downgrade();
        let result_coin = (self.constructor)(utxo_arc);

        self.spawn_merge_utxo_loop_if_required(utxo_weak, self.constructor.clone());
        Ok(result_coin)
    }
}

impl<'a, F, T> UtxoArcWithIguanaPrivKeyBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    T: AsRef<UtxoCoinFields> + UtxoCommonOps + Send + Sync + 'static,
{
    pub fn new(
        ctx: &'a MmArc,
        ticker: &'a str,
        conf: &'a Json,
        activation_params: &'a UtxoActivationParams,
        priv_key: &'a [u8],
        constructor: F,
    ) -> UtxoArcWithIguanaPrivKeyBuilder<'a, F, T> {
        UtxoArcWithIguanaPrivKeyBuilder {
            ctx,
            ticker,
            conf,
            activation_params,
            priv_key,
            constructor,
        }
    }
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
