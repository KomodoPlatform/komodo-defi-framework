use crate::utxo::rpc_clients::{UtxoRpcClientEnum, UtxoRpcError};
use crate::utxo::utxo_builder::{UtxoCoinBuildError, UtxoCoinBuilder, UtxoCoinBuilderCommonOps,
                                UtxoFieldsWithHardwareWalletBuilder, UtxoFieldsWithIguanaPrivKeyBuilder};
use crate::utxo::{generate_and_send_tx, FeePolicy, GetUtxoListOps, UtxoArc, UtxoCommonOps, UtxoSyncStatusLoopHandle,
                  UtxoWeak};
use crate::{DerivationMethod, PrivKeyBuildPolicy, UtxoActivationParams};
use async_trait::async_trait;
use chain::TransactionOutput;
use common::executor::{spawn, Timer};
use common::jsonrpc_client::{JsonRpcError, JsonRpcErrorType};
use common::log::{error, info, warn};
use futures::compat::Future01CompatExt;
use futures::future::{abortable, AbortHandle};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use script::Builder;
use serde_json::Value as Json;
use spv_validation::helpers_validation::validate_headers;
use spv_validation::storage::BlockHeaderStorageOps;

const BLOCK_HEADERS_LOOP_INTERVAL: f64 = 60.;
const BLOCK_HEADERS_MAX_CHUNK_SIZE: u64 = 500;

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
    T: UtxoCommonOps + GetUtxoListOps,
{
    type ResultCoin = T;
    type Error = UtxoCoinBuildError;

    fn priv_key_policy(&self) -> PrivKeyBuildPolicy<'_> { self.priv_key_policy.clone() }

    async fn build(self) -> MmResult<Self::ResultCoin, Self::Error> {
        let utxo = self.build_utxo_fields().await?;
        let sync_status_loop_handle = utxo.block_headers_status_notifier.clone();
        let utxo_arc = UtxoArc::new(utxo);
        let utxo_weak = utxo_arc.downgrade();
        let result_coin = (self.constructor)(utxo_arc);

        if let Some(abort_handler) = self.spawn_merge_utxo_loop_if_required(utxo_weak.clone(), self.constructor.clone())
        {
            self.ctx.abort_handlers.lock().unwrap().push(abort_handler);
        }

        if let Some(sync_status_loop_handle) = sync_status_loop_handle {
            let abort_handler =
                self.spawn_block_header_utxo_loop(utxo_weak, self.constructor.clone(), sync_status_loop_handle);
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

async fn merge_utxo_loop<T>(
    weak: UtxoWeak,
    merge_at: usize,
    check_every: f64,
    max_merge_at_once: usize,
    constructor: impl Fn(UtxoArc) -> T,
) where
    T: UtxoCommonOps + GetUtxoListOps,
{
    loop {
        Timer::sleep(check_every).await;

        let coin = match weak.upgrade() {
            Some(arc) => constructor(arc),
            None => break,
        };

        let my_address = match coin.as_ref().derivation_method {
            DerivationMethod::Iguana(ref my_address) => my_address,
            DerivationMethod::HDWallet(_) => {
                warn!("'merge_utxo_loop' is currently not used for HD wallets");
                return;
            },
        };

        let ticker = &coin.as_ref().conf.ticker;
        let (unspents, recently_spent) = match coin.get_unspent_ordered_list(my_address).await {
            Ok((unspents, recently_spent)) => (unspents, recently_spent),
            Err(e) => {
                error!("Error {} on get_unspent_ordered_list of coin {}", e, ticker);
                continue;
            },
        };
        if unspents.len() >= merge_at {
            let unspents: Vec<_> = unspents.into_iter().take(max_merge_at_once).collect();
            info!("Trying to merge {} UTXOs of coin {}", unspents.len(), ticker);
            let value = unspents.iter().fold(0, |sum, unspent| sum + unspent.value);
            let script_pubkey = Builder::build_p2pkh(&my_address.hash).to_bytes();
            let output = TransactionOutput { value, script_pubkey };
            let merge_tx_fut = generate_and_send_tx(
                &coin,
                unspents,
                None,
                FeePolicy::DeductFromOutput(0),
                recently_spent,
                vec![output],
            );
            match merge_tx_fut.await {
                Ok(tx) => info!(
                    "UTXO merge successful for coin {}, tx_hash {:?}",
                    ticker,
                    tx.hash().reversed()
                ),
                Err(e) => error!("Error {:?} on UTXO merge attempt for coin {}", e, ticker),
            }
        }
    }
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

macro_rules! sync_status_notice_on_error {
    ($sync_status_loop_handle:ident, $delay: expr, $error: ident) => {
        $sync_status_loop_handle.notify_on_temp_error($error.to_string());
        Timer::sleep($delay).await;
        continue;
    };
}

macro_rules! block_header_utxo_loop_validate_headers {
    ($coin: expr, $ticker: ident, $temporary_from: ident, $block_headers: expr, $storage: ident, $sync_status_loop_handle:ident) => {
         if let Some(params) = &$coin.as_ref().conf.block_headers_verification_params {
            if let Err(e) = validate_headers($ticker, $temporary_from, $block_headers, $storage, &params).await {
                error!("Error {} on validating the latest headers!", e);
                // Todo: remove this electrum server and use another in this case since the headers from this server are invalid
                $sync_status_loop_handle.notify_on_permanent_error(e.to_string());
                break;
            }
        }
    };
}

async fn block_header_utxo_loop<T: UtxoCommonOps>(
    weak: UtxoWeak,
    constructor: impl Fn(UtxoArc) -> T,
    mut sync_status_loop_handle: UtxoSyncStatusLoopHandle,
) {
    while let Some(arc) = weak.upgrade() {
        let coin = constructor(arc);
        let ticker = coin.as_ref().conf.ticker.as_str();
        let client = match &coin.as_ref().rpc_client {
            UtxoRpcClientEnum::Native(_) => break,
            UtxoRpcClientEnum::Electrum(client) => client,
        };

        let storage = client.block_headers_storage();
        let from_block_height = match storage.get_last_block_height().await {
            Ok(h) => h,
            Err(e) => {
                error!("Error {} on getting the height of the last stored header in DB!", e);
                sync_status_loop_handle.notify_on_temp_error(e.to_string());
                Timer::sleep(10.).await;
                continue;
            },
        };

        let to_block_height = match coin.as_ref().rpc_client.get_block_count().compat().await {
            Ok(h) => h,
            Err(e) => {
                error!("Error {} on getting the height of the latest block from rpc!", e);
                sync_status_loop_handle.notify_on_temp_error(e.to_string());
                Timer::sleep(10.).await;
                continue;
            },
        };

        // Todo: Add code for the case if a chain reorganization happens
        if from_block_height == to_block_height {
            Timer::sleep(BLOCK_HEADERS_LOOP_INTERVAL).await;
            continue;
        }

        sync_status_loop_handle.notify_blocks_headers_sync_status(from_block_height + 1, to_block_height);

        let notify_sync =
            async move |last_height: u64, to_block_height: u64, mut sync_handle: UtxoSyncStatusLoopHandle| {
                if last_height == to_block_height {
                    sync_handle.notify_sync_finished(to_block_height);
                    Timer::sleep(BLOCK_HEADERS_LOOP_INTERVAL).await;
                }
            };

        let (block_registry, block_headers, last_retrieved_height) = match client
            .retrieve_headers(from_block_height + 1, to_block_height)
            .compat()
            .await
        {
            Ok(res) => res,
            Err(e) => {
                error!("Error {} on retrieving the latest headers from rpc!", e);
                if !e.to_string().contains("response too large") {
                    error!("error {:?}", e);
                    sync_status_notice_on_error!(sync_status_loop_handle, 10., e);
                }

                log!("Now retrieving the latest headers from rpc in chunks!");
                let mut temporary_from = from_block_height + 1;
                let mut temporary_to = from_block_height + BLOCK_HEADERS_MAX_CHUNK_SIZE;

                // While (temporary to value) is less or equal to incoming original (to value), we will collect the headers in chunk of BLOCK_HEADERS_MAX_CHUNK_SIZE at a single request/loop.
                return while temporary_to <= to_block_height {
                    match client.retrieve_headers(temporary_from, temporary_to).compat().await {
                        Ok((block_registry, block_headers, last_retrieved_height)) => {
                            // Validate retrieved block headers
                            block_header_utxo_loop_validate_headers!(
                                coin,
                                ticker,
                                temporary_from,
                                block_headers.clone(),
                                storage,
                                sync_status_loop_handle
                            );

                            // Add headers to storage
                            ok_or_continue_after_sleep!(
                                storage.add_block_headers_to_storage(block_registry).await,
                                BLOCK_HEADERS_LOOP_INTERVAL
                            );

                            // blockchain.block.headers will returns a maximum of 500 headers so the loop needs to continue until we have all headers up to the current one.
                            notify_sync(last_retrieved_height, to_block_height, sync_status_loop_handle.clone()).await;

                            temporary_from += BLOCK_HEADERS_MAX_CHUNK_SIZE;
                            temporary_to += BLOCK_HEADERS_MAX_CHUNK_SIZE;
                        },
                        Err(err) => {
                            // keep retrying if network error
                            if let UtxoRpcError::Transport(JsonRpcError {
                                error: JsonRpcErrorType::Transport(_err),
                                ..
                            }) = err.get_inner()
                            {
                                log!("Will try fetching block headers again after 10 secs");
                                Timer::sleep(10.).await;
                                continue;
                            };
                            error!("Error {} on retrieving the latest headers from rpc!", err);
                            sync_status_notice_on_error!(sync_status_loop_handle, 10., e);
                        },
                    };
                };
            },
        };

        // Validate retrieved block headers
        block_header_utxo_loop_validate_headers!(
            coin,
            ticker,
            from_block_height,
            block_headers.clone(),
            storage,
            sync_status_loop_handle
        );

        ok_or_continue_after_sleep!(
            storage.add_block_headers_to_storage(block_registry).await,
            BLOCK_HEADERS_LOOP_INTERVAL
        );

        // blockchain.block.headers returns a maximum of 2016 headers (tested for btc) so the loop needs to continue until we have all headers up to the current one.
        notify_sync(last_retrieved_height, to_block_height, sync_status_loop_handle.clone()).await;
    }
}

pub trait BlockHeaderUtxoArcOps<T>: UtxoCoinBuilderCommonOps {
    fn spawn_block_header_utxo_loop<F>(
        &self,
        weak: UtxoWeak,
        constructor: F,
        sync_status_loop_handle: UtxoSyncStatusLoopHandle,
    ) -> AbortHandle
    where
        F: Fn(UtxoArc) -> T + Send + Sync + 'static,
        T: UtxoCommonOps,
    {
        let ticker = self.ticker().to_owned();
        let (fut, abort_handle) = abortable(block_header_utxo_loop(weak, constructor, sync_status_loop_handle));
        info!("Starting UTXO block header loop for coin {}", ticker);
        spawn(async move {
            if let Err(e) = fut.await {
                info!("spawn_block_header_utxo_loop stopped for {}, reason {}", ticker, e);
            }
        });
        abort_handle
    }
}
