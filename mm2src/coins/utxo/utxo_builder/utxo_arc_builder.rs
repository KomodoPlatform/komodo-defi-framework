use crate::utxo::rpc_clients::{ElectrumClient, UtxoRpcClientEnum};
use crate::utxo::utxo_block_header_storage::BlockHeaderStorage;
use crate::utxo::utxo_builder::{UtxoCoinBuildError, UtxoCoinBuilder, UtxoCoinBuilderCommonOps,
                                UtxoFieldsWithGlobalHDBuilder, UtxoFieldsWithHardwareWalletBuilder,
                                UtxoFieldsWithIguanaSecretBuilder};
use crate::utxo::{generate_and_send_tx, FeePolicy, GetUtxoListOps, UtxoArc, UtxoCommonOps, UtxoSyncStatusLoopHandle,
                  UtxoWeak};
use crate::{DerivationMethod, PrivKeyBuildPolicy, UtxoActivationParams};
use async_trait::async_trait;
use chain::{BlockHeader, TransactionOutput};
use common::executor::{AbortSettings, SpawnAbortable, Timer};
use common::log::{error, info, warn};
use futures::compat::Future01CompatExt;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
#[cfg(test)] use mocktopus::macros::*;
use primitives::hash::H256;
use script::Builder;
use serde_json::Value as Json;
use serialization::Reader;
use spv_validation::conf::SPVConf;
use spv_validation::helpers_validation::validate_headers;
use spv_validation::storage::{BlockHeaderStorageError, BlockHeaderStorageOps};
use std::collections::HashMap;
use std::num::NonZeroU64;

const FETCH_BLOCK_HEADERS_ATTEMPTS: u64 = 3;
const CHUNK_SIZE_REDUCER_VALUE: u64 = 100;

pub struct UtxoArcBuilder<'a, F, T>
where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static,
{
    ctx: &'a MmArc,
    ticker: &'a str,
    conf: &'a Json,
    activation_params: &'a UtxoActivationParams,
    priv_key_policy: PrivKeyBuildPolicy,
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
        priv_key_policy: PrivKeyBuildPolicy,
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

impl<'a, F, T> UtxoFieldsWithIguanaSecretBuilder for UtxoArcBuilder<'a, F, T> where
    F: Fn(UtxoArc) -> T + Send + Sync + 'static
{
}

impl<'a, F, T> UtxoFieldsWithGlobalHDBuilder for UtxoArcBuilder<'a, F, T> where
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

    fn priv_key_policy(&self) -> PrivKeyBuildPolicy { self.priv_key_policy.clone() }

    async fn build(self) -> MmResult<Self::ResultCoin, Self::Error> {
        let utxo = self.build_utxo_fields().await?;
        let sync_status_loop_handle = utxo.block_headers_status_notifier.clone();
        let spv_conf = utxo.conf.spv_conf.clone();
        let utxo_arc = UtxoArc::new(utxo);

        self.spawn_merge_utxo_loop_if_required(&utxo_arc, self.constructor.clone());

        let result_coin = (self.constructor)(utxo_arc.clone());

        if let (Some(spv_conf), Some(sync_handle)) = (spv_conf, sync_status_loop_handle) {
            spv_conf.validate(self.ticker).map_to_mm(UtxoCoinBuildError::SPVError)?;

            let block_count = result_coin
                .as_ref()
                .rpc_client
                .get_block_count()
                .compat()
                .await
                .map_err(|err| UtxoCoinBuildError::CantGetBlockCount(err.to_string()))?;
            self.spawn_block_header_utxo_loop(&utxo_arc, self.constructor.clone(), sync_handle, block_count, spv_conf);
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
            DerivationMethod::SingleAddress(ref my_address) => my_address,
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

pub(crate) struct BlockHeaderUtxoLoopExtraArgs {
    pub(crate) chunk_size: u64,
    pub(crate) error_sleep: f64,
    pub(crate) success_sleep: f64,
}

#[cfg_attr(test, mockable)]
impl Default for BlockHeaderUtxoLoopExtraArgs {
    fn default() -> Self {
        Self {
            chunk_size: 2016,
            error_sleep: 10.,
            success_sleep: 60.,
        }
    }
}

pub(crate) async fn block_header_utxo_loop<T: UtxoCommonOps>(
    weak: UtxoWeak,
    constructor: impl Fn(UtxoArc) -> T,
    mut sync_status_loop_handle: UtxoSyncStatusLoopHandle,
    mut block_count: u64,
    spv_conf: SPVConf,
) {
    let args = BlockHeaderUtxoLoopExtraArgs::default();
    let mut chunk_size = args.chunk_size;
    while let Some(arc) = weak.upgrade() {
        let coin = constructor(arc);
        let ticker = coin.as_ref().conf.ticker.as_str();
        let client = match &coin.as_ref().rpc_client {
            UtxoRpcClientEnum::Native(_) => break,
            UtxoRpcClientEnum::Electrum(client) => client,
        };

        let storage = client.block_headers_storage();
        let from_block_height = match storage.get_last_block_height().await {
            Ok(Some(height)) => height,
            Ok(None) => {
                if let Err(err) = validate_and_store_starting_header(client, ticker, storage, &spv_conf).await {
                    sync_status_loop_handle.notify_on_permanent_error(err);
                    break;
                }
                spv_conf.starting_block_header.height
            },
            Err(err) => {
                error!("Error {err:?} on getting the height of the last stored {ticker} header in DB!",);
                sync_status_loop_handle.notify_on_temp_error(err);
                Timer::sleep(args.error_sleep).await;
                continue;
            },
        };

        let mut to_block_height = from_block_height + chunk_size;
        if to_block_height > block_count {
            block_count = match coin.as_ref().rpc_client.get_block_count().compat().await {
                Ok(h) => h,
                Err(err) => {
                    error!("Error {err:} on getting the height of the latest {ticker} block from rpc!");
                    sync_status_loop_handle.notify_on_temp_error(err);
                    Timer::sleep(args.error_sleep).await;
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
            Timer::sleep(args.success_sleep).await;
            continue;
        }

        // Check if there should be a limit on the number of headers stored in storage.
        if let Some(max_stored_block_headers) = spv_conf.max_stored_block_headers {
            if let Err(err) =
                remove_excessive_headers_from_storage(storage, to_block_height, max_stored_block_headers).await
            {
                sync_status_loop_handle.notify_on_temp_error(err);
                Timer::sleep(args.error_sleep).await;
            };
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
                let err_inner = error.get_inner();
                if err_inner.is_network_error() {
                    log!("Network Error: Will try fetching {ticker} block headers again after 10 secs");
                    sync_status_loop_handle.notify_on_temp_error(error);
                    Timer::sleep(args.error_sleep).await;
                    continue;
                };

                // If electrum returns response too large error, we will reduce the requested headers by CHUNK_SIZE_REDUCER_VALUE in every loop until we arrive at a reasonable value.
                if err_inner.is_response_too_large() && chunk_size > CHUNK_SIZE_REDUCER_VALUE {
                    chunk_size -= CHUNK_SIZE_REDUCER_VALUE;
                    continue;
                }

                if fetch_blocker_headers_attempts > 0 {
                    fetch_blocker_headers_attempts -= 1;
                    error!("Error {error:?} on retrieving latest {ticker} headers from rpc! {fetch_blocker_headers_attempts} attempts left");
                    // Todo: remove this electrum server and use another in this case since the headers from this server can't be retrieved
                    sync_status_loop_handle.notify_on_temp_error(error);
                    Timer::sleep(args.error_sleep).await;
                    continue;
                };

                error!(
                    "Error {error:?} on retrieving latest {ticker} headers from rpc after {FETCH_BLOCK_HEADERS_ATTEMPTS} attempts"
                );
                // Todo: remove this electrum server and use another in this case since the headers from this server can't be retrieved
                sync_status_loop_handle.notify_on_permanent_error(error);
                break;
            },
        };

        // Scan header for chain reorg
        match scan_headers_for_chain_reorg(ticker, from_block_height, storage, &block_headers).await {
            Ok(_) => (),
            Err(err) => {
                error!("{err:?}!");
                // Todo: remove bad headers from storage starting from the `height` at which re-org occured.
                sync_status_loop_handle.notify_on_temp_error(err);
                continue;
            },
        };

        // Validate retrieved block headers.
        if let Err(err) = validate_headers(ticker, from_block_height, block_headers, storage, &spv_conf).await {
            error!("Error {err:?} on validating the latest headers for {ticker}!");
            // Todo: remove this electrum server and use another in this case since the headers from this server are invalid
            sync_status_loop_handle.notify_on_permanent_error(err);
            break;
        };

        let sleep = args.success_sleep;
        ok_or_continue_after_sleep!(storage.add_block_headers_to_storage(block_registry).await, sleep);
    }
}

// Check for a chain reorg given a block header and a list of headers
// downloaded from an RPC node
pub async fn scan_headers_for_chain_reorg(
    coin: &str,
    last_block_height: u64,
    storage: &BlockHeaderStorage,
    rpc_headers: &[BlockHeader],
) -> Result<(), BlockHeaderStorageError> {
    let latest_header = storage.get_block_header(last_block_height).await?;

    if let Some(latest_header) = latest_header {
        let headers_to_check = rpc_headers.len();
        let local_headers = storage.get_block_headers_from_height(headers_to_check as u64).await?;
        let common_header = common_block_headers(&local_headers, rpc_headers).await;
        if let Some(common_header) = common_header {
            return if common_header == latest_header.previous_header_hash {
                Ok(())
            } else {
                let height = storage.get_block_height_by_hash(common_header).await?;
                Err(BlockHeaderStorageError::ChainReOrgDetected {
                    coin: coin.to_string(),
                    height: height.unwrap_or_default() as u64,
                })?
            };
        }
    };

    Ok(())
}

// Find the common ancestor block header given two lists of headers
async fn common_block_headers(headers1: &[BlockHeader], headers2: &[BlockHeader]) -> Option<H256> {
    for header1 in headers1.iter().rev() {
        for header2 in headers2.iter().rev() {
            if header1.hash() == header2.hash() {
                return Some(header1.hash());
            }
        }
    }

    None
}

#[derive(Display)]
enum StartingHeaderValidationError {
    #[display(fmt = "Can't decode/deserialize from storage for {} - reason: {}", coin, reason)]
    DecodeErr {
        coin: String,
        reason: String,
    },
    RpcError(String),
    StorageError(String),
    #[display(fmt = "Error validating starting header for {} - reason: {}", coin, reason)]
    ValidationError {
        coin: String,
        reason: String,
    },
}

async fn validate_and_store_starting_header(
    client: &ElectrumClient,
    ticker: &str,
    storage: &BlockHeaderStorage,
    spv_conf: &SPVConf,
) -> MmResult<(), StartingHeaderValidationError> {
    let height = spv_conf.starting_block_header.height;
    let header_bytes = client
        .blockchain_block_header(height)
        .compat()
        .await
        .map_to_mm(|err| StartingHeaderValidationError::RpcError(err.to_string()))?;

    let mut reader = Reader::new_with_coin_variant(&header_bytes, ticker.into());
    let header = reader
        .read()
        .map_to_mm(|err| StartingHeaderValidationError::DecodeErr {
            coin: ticker.to_string(),
            reason: err.to_string(),
        })?;

    spv_conf
        .validate_rpc_starting_header(height, &header)
        .map_to_mm(|err| StartingHeaderValidationError::ValidationError {
            coin: ticker.to_string(),
            reason: err.to_string(),
        })?;

    storage
        .add_block_headers_to_storage(HashMap::from([(height, header)]))
        .await
        .map_to_mm(|err| StartingHeaderValidationError::StorageError(err.to_string()))
}

async fn remove_excessive_headers_from_storage(
    storage: &BlockHeaderStorage,
    last_height_to_be_added: u64,
    max_allowed_headers: NonZeroU64,
) -> Result<(), BlockHeaderStorageError> {
    let max_allowed_headers = max_allowed_headers.get();
    if last_height_to_be_added > max_allowed_headers {
        return storage
            .remove_headers_up_to_height(last_height_to_be_added - max_allowed_headers)
            .await;
    }

    Ok(())
}

pub trait BlockHeaderUtxoArcOps<T>: UtxoCoinBuilderCommonOps {
    fn spawn_block_header_utxo_loop<F>(
        &self,
        utxo_arc: &UtxoArc,
        constructor: F,
        sync_status_loop_handle: UtxoSyncStatusLoopHandle,
        block_count: u64,
        spv_conf: SPVConf,
    ) where
        F: Fn(UtxoArc) -> T + Send + Sync + 'static,
        T: UtxoCommonOps,
    {
        let ticker = self.ticker();
        info!("Starting UTXO block header loop for coin {ticker}");

        let utxo_weak = utxo_arc.downgrade();
        let fut = block_header_utxo_loop(utxo_weak, constructor, sync_status_loop_handle, block_count, spv_conf);

        let settings = AbortSettings::info_on_abort(format!("spawn_block_header_utxo_loop stopped for {ticker}"));
        utxo_arc
            .abortable_system
            .weak_spawner()
            .spawn_with_settings(fut, settings);
    }
}
