use crate::utxo::rpc_clients::UtxoRpcClientEnum;
use crate::utxo::utxo_builder::{UtxoCoinBuildError, UtxoCoinBuilder, UtxoCoinBuilderCommonOps,
                                UtxoFieldsWithHardwareWalletBuilder, UtxoFieldsWithIguanaPrivKeyBuilder};
use crate::utxo::{generate_and_send_tx, FeePolicy, GetUtxoListOps, UtxoArc, UtxoCommonOps, UtxoSyncStatusLoopHandle,
                  UtxoWeak};
use crate::{DerivationMethod, PrivKeyBuildPolicy, UtxoActivationParams};
use async_trait::async_trait;
use chain::TransactionOutput;
use common::executor::{AbortSettings, SpawnAbortable, Timer};
use common::log::{error, info, warn};
use futures::compat::Future01CompatExt;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use script::Builder;
use serde_json::Value as Json;
use spv_validation::helpers_validation::validate_headers;
use spv_validation::storage::BlockHeaderStorageOps;

const BLOCK_HEADERS_LOOP_SUCCESS_SLEEP_TIMER: f64 = 60.;
const BLOCK_HEADERS_LOOP_ERORR_SLEEP_TIMER: f64 = 10.;
const FETCH_BLOCK_HEADERS_ATTEMPTS: u64 = 3;
const CHUNK_SIZE_REDUCER_VALUE: u64 = 100;
const ELECTRUM_MAX_CHUNK_SIZE: u64 = 2016;

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

        self.spawn_merge_utxo_loop_if_required(&utxo_arc, self.constructor.clone());

        let result_coin = (self.constructor)(utxo_arc.clone());
        if let Some(sync_status_loop_handle) = sync_status_loop_handle {
            let block_count = result_coin
                .as_ref()
                .rpc_client
                .get_block_count()
                .compat()
                .await
                .map_err(|err| UtxoCoinBuildError::CantGetBlockCount(err.to_string()))?;
            self.spawn_block_header_utxo_loop(
                &utxo_arc,
                self.constructor.clone(),
                sync_status_loop_handle,
                block_count,
            );
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
    fn spawn_merge_utxo_loop_if_required<F>(&self, utxo_arc: &UtxoArc, constructor: F)
    where
        F: Fn(UtxoArc) -> T + Send + Sync + 'static,
    {
        let merge_params = match self.activation_params().utxo_merge_params {
            Some(ref merge_params) => merge_params,
            None => return,
        };

        let ticker = self.ticker();
        info!("Starting UTXO merge loop for coin {ticker}");

        let utxo_weak = utxo_arc.downgrade();
        let fut = merge_utxo_loop(
            utxo_weak,
            merge_params.merge_at,
            merge_params.check_every,
            merge_params.max_merge_at_once,
            constructor,
        );

        let settings = AbortSettings::info_on_abort(format!("spawn_merge_utxo_loop_if_required stopped for {ticker}"));
        utxo_arc
            .abortable_system
            .weak_spawner()
            .spawn_with_settings(fut, settings);
    }
}

async fn block_header_utxo_loop<T: UtxoCommonOps>(
    weak: UtxoWeak,
    constructor: impl Fn(UtxoArc) -> T,
    mut sync_status_loop_handle: UtxoSyncStatusLoopHandle,
    mut block_count: u64,
) {
    let mut chunk_size = ELECTRUM_MAX_CHUNK_SIZE;
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
                error!("Error {e:?} on getting the height of the last stored {ticker} header in DB!",);
                sync_status_loop_handle.notify_on_temp_error(e.to_string());
                Timer::sleep(BLOCK_HEADERS_LOOP_ERORR_SLEEP_TIMER).await;
                continue;
            },
        };

        let mut to_block_height = from_block_height + chunk_size;
        if to_block_height > block_count {
            block_count = match coin.as_ref().rpc_client.get_block_count().compat().await {
                Ok(h) => h,
                Err(e) => {
                    error!("Error {e:} on getting the height of the latest {ticker} block from rpc!");
                    sync_status_loop_handle.notify_on_temp_error(e.to_string());
                    Timer::sleep(BLOCK_HEADERS_LOOP_ERORR_SLEEP_TIMER).await;
                    continue;
                },
            };

            // More than `chunk_size` blocks could have appeared since the last `get_block_count` RPC.
            // So reset `to_block_height` if only `from_block_height + chunk_size > actual_block_count`.
            if to_block_height > block_count {
                to_block_height = block_count;
            }
        }
        drop_mutability!(to_block_height);

        // Todo: Add code for the case if a chain reorganization happens
        if from_block_height == block_count {
            sync_status_loop_handle.notify_sync_finished(to_block_height);
            Timer::sleep(BLOCK_HEADERS_LOOP_SUCCESS_SLEEP_TIMER).await;
            continue;
        }

        sync_status_loop_handle.notify_blocks_headers_sync_status(from_block_height + 1, to_block_height);

        let mut fetch_blocker_headers_attempts = FETCH_BLOCK_HEADERS_ATTEMPTS;
        let (block_registry, block_headers) = match client
            .retrieve_headers(from_block_height + 1, to_block_height)
            .compat()
            .await
        {
            Ok(res) => res,
            Err(error) => {
                if error.get_inner().is_network_error() {
                    log!("Network Error: Will try fetching {ticker} block headers again after 10 secs");
                    sync_status_loop_handle.notify_on_temp_error(error.to_string());
                    Timer::sleep(BLOCK_HEADERS_LOOP_ERORR_SLEEP_TIMER).await;
                    continue;
                };

                // If electrum returns response too large error, we will reduce the requested headers by CHUNK_SIZE_REDUCER_VALUE in every loop until we arrive at a reasonable value.
                if error.get_inner().is_response_too_large() && chunk_size > CHUNK_SIZE_REDUCER_VALUE {
                    chunk_size -= CHUNK_SIZE_REDUCER_VALUE;
                    continue;
                }

                if fetch_blocker_headers_attempts > 0 {
                    fetch_blocker_headers_attempts -= 1;
                    error!("Error {error:?} on retrieving latest {ticker} headers from rpc! {fetch_blocker_headers_attempts} attempts left");
                    // Todo: remove this electrum server and use another in this case since the headers from this server can't be retrieved
                    sync_status_loop_handle.notify_on_temp_error(error.to_string());
                    Timer::sleep(BLOCK_HEADERS_LOOP_ERORR_SLEEP_TIMER).await;
                    continue;
                };

                error!(
                    "Error {error:?} on retrieving latest {ticker} headers from rpc after {FETCH_BLOCK_HEADERS_ATTEMPTS} attempts"
                );
                // Todo: remove this electrum server and use another in this case since the headers from this server can't be retrieved
                sync_status_loop_handle.notify_on_permanent_error(error.to_string());
                break;
            },
        };

        // Validate retrieved block headers
        if let Some(params) = &coin.as_ref().conf.block_headers_verification_params {
            if let Err(e) = validate_headers(ticker, from_block_height, block_headers, storage, params).await {
                error!("Error {e:?} on validating the latest headers for {ticker}!");
                // Todo: remove this electrum server and use another in this case since the headers from this server are invalid
                sync_status_loop_handle.notify_on_permanent_error(e.to_string());
                break;
            }
        };

        ok_or_continue_after_sleep!(
            storage.add_block_headers_to_storage(block_registry).await,
            BLOCK_HEADERS_LOOP_SUCCESS_SLEEP_TIMER
        );
    }
}

pub trait BlockHeaderUtxoArcOps<T>: UtxoCoinBuilderCommonOps {
    fn spawn_block_header_utxo_loop<F>(
        &self,
        utxo_arc: &UtxoArc,
        constructor: F,
        sync_status_loop_handle: UtxoSyncStatusLoopHandle,
        block_count: u64,
    ) where
        F: Fn(UtxoArc) -> T + Send + Sync + 'static,
        T: UtxoCommonOps,
    {
        let ticker = self.ticker();
        info!("Starting UTXO block header loop for coin {ticker}");

        let utxo_weak = utxo_arc.downgrade();
        let fut = block_header_utxo_loop(utxo_weak, constructor, sync_status_loop_handle, block_count);

        let settings = AbortSettings::info_on_abort(format!("spawn_block_header_utxo_loop stopped for {ticker}"));
        utxo_arc
            .abortable_system
            .weak_spawner()
            .spawn_with_settings(fut, settings);
    }
}
