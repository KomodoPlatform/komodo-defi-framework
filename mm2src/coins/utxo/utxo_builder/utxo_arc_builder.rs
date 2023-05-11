use crate::utxo::rpc_clients::{ElectrumClient, ElectrumClientImpl, UtxoJsonRpcClientInfo, UtxoRpcClientEnum};
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
use common::log::{debug, error, info, warn};
use futures::compat::Future01CompatExt;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
#[cfg(test)] use mocktopus::macros::*;
use rand::Rng;
use script::Builder;
use serde_json::Value as Json;
use serialization::Reader;
use spv_validation::conf::SPVConf;
use spv_validation::helpers_validation::{validate_headers, SPVError};
use spv_validation::storage::{BlockHeaderStorageError, BlockHeaderStorageOps};
use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::{Arc, Weak};

const CHUNK_SIZE_REDUCER_VALUE: u64 = 100;
const TRY_TO_RETRIEVE_HEADERS_ATTEMPTS: u8 = 10;

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
            spawn_block_header_utxo_loop(self.ticker, &utxo_arc, sync_handle, spv_conf);
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

/// This function executes a loop to fetch, validate and store block headers from the connected electrum servers.
/// sync_status_loop_handle notifies the coin activation function of errors and if the error is temporary or not.
/// spv_conf is passed from the coin configuration and it determines how headers are validated and stored.
pub(crate) async fn block_header_utxo_loop(
    weak: Weak<ElectrumClientImpl>,
    mut sync_status_loop_handle: UtxoSyncStatusLoopHandle,
    spv_conf: SPVConf,
) {
    macro_rules! remove_server_and_break_if_no_servers_left {
        ($client:expr, $server_address:expr, $ticker:expr, $sync_status_loop_handle:expr) => {
            if let Err(e) = $client.remove_server($server_address).await {
                let msg = format!("Error {} on removing server {}!", e, $server_address);
                // Todo: Permanent error notification should lead to deactivation of coin after applying some fail-safe measures if there are on-going swaps
                $sync_status_loop_handle.notify_on_permanent_error(msg);
                break;
            }

            if $client.is_connections_pool_empty().await {
                // Todo: Permanent error notification should lead to deactivation of coin after applying some fail-safe measures if there are on-going swaps
                let msg = format!("All servers are removed for {}!", $ticker);
                $sync_status_loop_handle.notify_on_permanent_error(msg);
                break;
            }
        };
    }

    let (mut electrum_addresses, mut block_count) = match weak.upgrade() {
        Some(client) => {
            let client = ElectrumClient(client);
            match client.get_servers_with_latest_block_count().compat().await {
                Ok((electrum_addresses, block_count)) => (electrum_addresses, block_count),
                Err(err) => {
                    sync_status_loop_handle.notify_on_permanent_error(err);
                    return;
                },
            }
        },
        None => {
            sync_status_loop_handle.notify_on_permanent_error("Electrum client dropped!".to_string());
            return;
        },
    };
    let mut args = BlockHeaderUtxoLoopExtraArgs::default();
    while let Some(client) = weak.upgrade() {
        let client = &ElectrumClient(client);
        let ticker = client.coin_name();

        let storage = client.block_headers_storage();
        let last_height_in_storage = match storage.get_last_block_height().await {
            Ok(Some(height)) => height,
            Ok(None) => {
                if let Err(err) = validate_and_store_starting_header(client, ticker, storage, &spv_conf).await {
                    sync_status_loop_handle.notify_on_permanent_error(err);
                    break;
                }
                spv_conf.starting_block_header.height
            },
            Err(err) => {
                error!(
                    "Error {} on getting the height of the last stored {} header in DB!",
                    err, ticker
                );
                sync_status_loop_handle.notify_on_temp_error(err);
                Timer::sleep(args.error_sleep).await;
                continue;
            },
        };

        let mut retrieve_to = last_height_in_storage + args.chunk_size;
        if retrieve_to > block_count {
            (electrum_addresses, block_count) = match client.get_servers_with_latest_block_count().compat().await {
                Ok((electrum_addresses, block_count)) => (electrum_addresses, block_count),
                Err(e) => {
                    let msg = format!(
                        "Error {} on getting the height of the latest {} block from rpc!",
                        e, ticker
                    );
                    error!("{}", msg);
                    sync_status_loop_handle.notify_on_temp_error(msg);
                    Timer::sleep(args.error_sleep).await;
                    continue;
                },
            };

            // More than `chunk_size` blocks could have appeared since the last `get_block_count` RPC.
            // So reset `to_block_height` if only `from_block_height + chunk_size > actual_block_count`.
            if retrieve_to > block_count {
                retrieve_to = block_count;
            }
        }
        drop_mutability!(retrieve_to);

        if last_height_in_storage == block_count {
            sync_status_loop_handle.notify_sync_finished(block_count);
            Timer::sleep(args.success_sleep).await;
            continue;
        }

        // Check if there should be a limit on the number of headers stored in storage.
        if let Some(max_stored_block_headers) = spv_conf.max_stored_block_headers {
            if let Err(err) =
                remove_excessive_headers_from_storage(storage, retrieve_to, max_stored_block_headers).await
            {
                error!("Error {} on removing excessive {} headers from storage!", err, ticker);
                sync_status_loop_handle.notify_on_temp_error(err);
                Timer::sleep(args.error_sleep).await;
            };
        }

        sync_status_loop_handle.notify_blocks_headers_sync_status(last_height_in_storage + 1, retrieve_to);

        let index = rand::thread_rng().gen_range(0, electrum_addresses.len());
        let server_address = match electrum_addresses.get(index) {
            Some(address) => address,
            None => {
                let msg = "Electrum addresses are empty when there should be at least one electrum returned from get_servers_with_latest_block_count!";
                error!("{}", msg);
                sync_status_loop_handle.notify_on_temp_error(msg.to_string());
                Timer::sleep(args.error_sleep).await;
                continue;
            },
        };
        let (block_registry, block_headers) = match try_to_retrieve_headers_until_success(
            &mut args,
            client,
            server_address,
            last_height_in_storage + 1,
            retrieve_to,
        )
        .await
        {
            Ok((block_registry, block_headers)) => (block_registry, block_headers),
            Err(err) => match err.get_inner() {
                TryToRetrieveHeadersUntilSuccessError::NetworkError { .. } => {
                    error!("{}", err);
                    sync_status_loop_handle.notify_on_temp_error(err.to_string());
                    continue;
                },
                TryToRetrieveHeadersUntilSuccessError::PermanentError { .. } => {
                    error!("{}", err);
                    remove_server_and_break_if_no_servers_left!(
                        client,
                        server_address,
                        ticker,
                        sync_status_loop_handle
                    );
                    continue;
                },
            },
        };

        // Validate retrieved block headers.
        if let Err(err) = validate_headers(ticker, last_height_in_storage, &block_headers, storage, &spv_conf).await {
            error!("Error {} on validating the latest headers for {}!", err, ticker);
            // This code block handles a specific error scenario where a parent hash mismatch(chain re-org) is
            // detected in the SPV client.
            // If this error occurs, the code retrieves and revalidates the mismatching header from the SPV client..
            if let SPVError::ParentHashMismatch {
                coin,
                mismatched_block_height,
            } = &err
            {
                match resolve_possible_chain_reorg(
                    client,
                    server_address,
                    &mut args,
                    last_height_in_storage,
                    *mismatched_block_height,
                    storage,
                    &spv_conf,
                )
                .await
                {
                    Ok(()) => {
                        info!(
                            "Chain reorg detected and resolved for coin: {}, re-syncing reorganized headers!",
                            coin
                        );
                        continue;
                    },
                    Err(err) => {
                        error!("Error {} on resolving chain reorg for coin: {}!", err, coin);
                        if err.get_inner().is_network_error() {
                            sync_status_loop_handle.notify_on_temp_error(err.to_string());
                        } else {
                            remove_server_and_break_if_no_servers_left!(
                                client,
                                server_address,
                                ticker,
                                sync_status_loop_handle
                            );
                        }
                        continue;
                    },
                }
            }
            remove_server_and_break_if_no_servers_left!(client, server_address, ticker, sync_status_loop_handle);
            continue;
        }

        let sleep = args.error_sleep;
        ok_or_continue_after_sleep!(storage.add_block_headers_to_storage(block_registry).await, sleep);
    }
}

#[derive(Debug, Display)]
enum TryToRetrieveHeadersUntilSuccessError {
    #[display(
        fmt = "Network error: {}, on retrieving headers from server {}",
        error,
        server_address
    )]
    NetworkError { error: String, server_address: String },
    #[display(
        fmt = "Permanent Error: {}, on retrieving headers from server {}",
        error,
        server_address
    )]
    PermanentError { error: String, server_address: String },
}

/// Loops until the headers are retrieved successfully.
async fn try_to_retrieve_headers_until_success(
    args: &mut BlockHeaderUtxoLoopExtraArgs,
    client: &ElectrumClient,
    server_address: &str,
    retrieve_from: u64,
    retrieve_to: u64,
) -> Result<(HashMap<u64, BlockHeader>, Vec<BlockHeader>), MmError<TryToRetrieveHeadersUntilSuccessError>> {
    let mut attempts: u8 = TRY_TO_RETRIEVE_HEADERS_ATTEMPTS;
    loop {
        match client
            .retrieve_headers_from(server_address, retrieve_from, retrieve_to)
            .compat()
            .await
        {
            Ok(res) => break Ok(res),
            Err(err) => {
                let err_inner = err.get_inner();
                if err_inner.is_network_error() {
                    if attempts == 0 {
                        break Err(MmError::new(TryToRetrieveHeadersUntilSuccessError::NetworkError {
                            error: format!(
                                "Max attempts of {} reached, will try to retrieve headers from a random server again!",
                                TRY_TO_RETRIEVE_HEADERS_ATTEMPTS
                            ),
                            server_address: server_address.to_string(),
                        }));
                    }
                    attempts -= 1;
                    error!(
                        "Network Error: {}, Will try fetching block headers again from {} after 10 secs",
                        err, server_address,
                    );
                    Timer::sleep(args.error_sleep).await;
                    continue;
                };

                // If electrum returns response too large error, we will reduce the requested headers by CHUNK_SIZE_REDUCER_VALUE in every loop until we arrive at a reasonable value.
                if err_inner.is_response_too_large() && args.chunk_size > CHUNK_SIZE_REDUCER_VALUE {
                    args.chunk_size -= CHUNK_SIZE_REDUCER_VALUE;
                    continue;
                }

                break Err(MmError::new(TryToRetrieveHeadersUntilSuccessError::PermanentError {
                    error: err.to_string(),
                    server_address: server_address.to_string(),
                }));
            },
        }
    }
}

// Represents the different types of errors that can occur while retrieving block headers from the Electrum client.
#[derive(Debug, Display)]
enum PossibleChainReorgError {
    #[display(fmt = "Preconfigured starting_block_header is bad or invalid. Please reconfigure.")]
    BadStartingHeaderChain,
    #[display(fmt = "Validation Error: {}", _0)]
    ValidationError(String),
    #[display(fmt = "Error retrieving headers: {}", _0)]
    HeadersRetrievalError(TryToRetrieveHeadersUntilSuccessError),
}

impl PossibleChainReorgError {
    fn is_network_error(&self) -> bool {
        matches!(
            self,
            PossibleChainReorgError::HeadersRetrievalError(TryToRetrieveHeadersUntilSuccessError::NetworkError { .. })
        )
    }
}

/// Retrieves block headers from the specified client within the given height range and revalidate against [`SPVError::ParentHashMismatch`] .
async fn resolve_possible_chain_reorg(
    client: &ElectrumClient,
    server_address: &str,
    args: &mut BlockHeaderUtxoLoopExtraArgs,
    last_height_in_storage: u64,
    mismatched_block_height: u64,
    storage: &dyn BlockHeaderStorageOps,
    spv_conf: &SPVConf,
) -> Result<(), MmError<PossibleChainReorgError>> {
    let ticker = client.coin_name();
    let mut retrieve_from = mismatched_block_height;
    let mut retrieve_to = retrieve_from + args.chunk_size;

    loop {
        debug!(
            "Possible chain reorganization for coin:{} at block height {}!",
            ticker, retrieve_from
        );
        // Attempt to retrieve the headers and validate them.
        let (_, headers_to_validate) =
            match try_to_retrieve_headers_until_success(args, client, server_address, retrieve_from, retrieve_to).await
            {
                Ok(res) => res,
                Err(err) => {
                    break Err(MmError::new(PossibleChainReorgError::HeadersRetrievalError(
                        err.into_inner(),
                    )))
                },
            };
        // If the headers are successfully retrieved and validated, remove the headers from storage and continue the outer loop.
        match validate_headers(ticker, retrieve_from - 1, &headers_to_validate, storage, spv_conf).await {
            Ok(_) => {
                // Headers are valid, remove saved headers and continue outer loop
                let sleep = args.error_sleep;
                return Ok(ok_or_continue_after_sleep!(
                    storage
                        .remove_headers_from_storage(retrieve_from, last_height_in_storage)
                        .await,
                    sleep
                ));
            },
            Err(err) => {
                if let SPVError::ParentHashMismatch {
                    mismatched_block_height,
                    ..
                } = err
                {
                    // There is another parent hash mismatch, retrieve the chunk right before this mismatched block height.
                    retrieve_to = mismatched_block_height - 1;
                    // Check if the height to retrieve up to is equal to the height of the preconfigured starting block header.
                    // If it is, it indicates a bad chain, and we return an error of type `RetrieveHeadersError::BadStartingHeaderChain`.
                    if retrieve_to == spv_conf.starting_block_header.height {
                        // Bad chain for preconfigured starting header detected, reconfigure.
                        return Err(MmError::new(PossibleChainReorgError::BadStartingHeaderChain));
                    };
                    // Calculate the height to retrieve from on next iteration based on the the height we will retrieve up to and the chunk size.
                    // If the current height is below or equal to the starting block header height, use the block header
                    // height after the starting one.
                    retrieve_from = retrieve_to
                        .saturating_sub(args.chunk_size)
                        .max(spv_conf.starting_block_header.height + 1);
                } else {
                    return Err(MmError::new(PossibleChainReorgError::ValidationError(err.to_string())));
                }
            },
        }
    }
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
    storage: &dyn BlockHeaderStorageOps,
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
            .remove_headers_from_storage(0, last_height_to_be_added - max_allowed_headers)
            .await;
    }

    Ok(())
}

fn spawn_block_header_utxo_loop(
    ticker: &str,
    utxo_arc: &UtxoArc,
    sync_status_loop_handle: UtxoSyncStatusLoopHandle,
    spv_conf: SPVConf,
) {
    let client = match &utxo_arc.rpc_client {
        UtxoRpcClientEnum::Native(_) => return,
        UtxoRpcClientEnum::Electrum(client) => client,
    };
    info!("Starting UTXO block header loop for coin {ticker}");

    let electrum_weak = Arc::downgrade(&client.0);
    let fut = block_header_utxo_loop(electrum_weak, sync_status_loop_handle, spv_conf);

    let settings = AbortSettings::info_on_abort(format!("spawn_block_header_utxo_loop stopped for {ticker}"));
    utxo_arc
        .abortable_system
        .weak_spawner()
        .spawn_with_settings(fut, settings);
}
