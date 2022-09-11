/// This module is named bch_and_slp_tx_history temporary. We will most likely use the same approach for every
/// supported UTXO coin.
use super::RequestTxHistoryResult;
use crate::my_tx_history_v2::{CoinWithTxHistoryV2, TxHistoryStorage};
use crate::utxo::bch::BchCoin;
use crate::utxo::utxo_common;
use crate::{BalanceResult, BlockHeightAndTime, HistorySyncState, MarketCoinOps, TransactionDetails, UtxoRpcError};
use async_trait::async_trait;
use common::executor::Timer;
use common::log::{error, info};
use common::state_machine::prelude::*;
use keys::Address;
use mm2_err_handle::prelude::*;
use mm2_metrics::MetricsArc;
use mm2_number::BigDecimal;
use rpc::v1::types::H256 as H256Json;
use std::collections::{hash_map::Entry, HashMap, HashSet};
use std::str::FromStr;

pub struct UtxoTxDetailsParams<'a, Storage> {
    pub hash: &'a H256Json,
    pub block_height_and_time: Option<BlockHeightAndTime>,
    pub storage: &'a Storage,
    pub my_addresses: &'a HashSet<Address>,
}

#[async_trait]
pub trait UtxoTxHistoryOps: CoinWithTxHistoryV2 + MarketCoinOps + Send + Sync + 'static {
    /// # Todo
    ///
    /// Consider returning a specific error.
    async fn my_addresses(&self) -> Result<HashSet<Address>, String>;

    /// Gets tx details by hash requesting the coin RPC if required.
    ///
    /// # Todo
    ///
    /// Consider returning a specific error.
    /// In the meantime, return the string as an error since we just output it to the log.
    async fn tx_details_by_hash<T>(
        &self,
        params: UtxoTxDetailsParams<'_, T>,
    ) -> Result<Vec<TransactionDetails>, String>
    where
        T: TxHistoryStorage;

    /// Requests transaction history.
    async fn request_tx_history(&self, metrics: MetricsArc) -> RequestTxHistoryResult;

    /// Requests timestamp of the given block.
    async fn get_block_timestamp(&self, height: u64) -> MmResult<u64, UtxoRpcError>;

    /// Requests balances of all activated coin's addresses.
    async fn get_addresses_balances(&self) -> BalanceResult<HashMap<String, BigDecimal>>;

    /// Sets the history sync state.
    fn set_history_sync_state(&self, new_state: HistorySyncState);
}

struct UtxoTxHistoryCtx<Coin: UtxoTxHistoryOps, Storage: TxHistoryStorage> {
    coin: Coin,
    storage: Storage,
    metrics: MetricsArc,
    /// Last requested balances of the activated coin's addresses.
    /// TODO add a `CoinBalanceState` structure and replace [`HashMap<String, BigDecimal>`] everywhere.
    balances: HashMap<String, BigDecimal>,
}

// States have to be generic over storage type because BchAndSlpHistoryCtx is generic over it
struct Init<Coin, Storage> {
    phantom: std::marker::PhantomData<(Coin, Storage)>,
}

impl<Coin, Storage> Init<Coin, Storage> {
    fn new() -> Self {
        Init {
            phantom: Default::default(),
        }
    }
}

impl<Coin, Storage> TransitionFrom<Init<Coin, Storage>> for Stopped<Coin, Storage> {}

#[async_trait]
impl<Coin, Storage> State for Init<Coin, Storage>
where
    Coin: UtxoTxHistoryOps,
    Storage: TxHistoryStorage,
{
    type Ctx = UtxoTxHistoryCtx<Coin, Storage>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut Self::Ctx) -> StateResult<Self::Ctx, Self::Result> {
        ctx.coin.set_history_sync_state(HistorySyncState::NotStarted);

        if let Err(e) = ctx.storage.init(&ctx.coin.history_wallet_id()).await {
            return Self::change_state(Stopped::storage_error(e));
        }

        Self::change_state(FetchingTxHashes::new())
    }
}

// States have to be generic over storage type because BchAndSlpHistoryCtx is generic over it
struct FetchingTxHashes<Coin, Storage> {
    phantom: std::marker::PhantomData<(Coin, Storage)>,
}

impl<Coin, Storage> FetchingTxHashes<Coin, Storage> {
    fn new() -> Self {
        FetchingTxHashes {
            phantom: Default::default(),
        }
    }
}

impl<Coin, Storage> TransitionFrom<Init<Coin, Storage>> for FetchingTxHashes<Coin, Storage> {}
impl<Coin, Storage> TransitionFrom<OnIoErrorCooldown<Coin, Storage>> for FetchingTxHashes<Coin, Storage> {}
impl<Coin, Storage> TransitionFrom<WaitForHistoryUpdateTrigger<Coin, Storage>> for FetchingTxHashes<Coin, Storage> {}

#[async_trait]
impl<Coin, Storage> State for FetchingTxHashes<Coin, Storage>
where
    Coin: UtxoTxHistoryOps,
    Storage: TxHistoryStorage,
{
    type Ctx = UtxoTxHistoryCtx<Coin, Storage>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut Self::Ctx) -> StateResult<Self::Ctx, Self::Result> {
        let wallet_id = ctx.coin.history_wallet_id();
        if let Err(e) = ctx.storage.init(&wallet_id).await {
            return Self::change_state(Stopped::storage_error(e));
        }

        let maybe_tx_ids = ctx.coin.request_tx_history(ctx.metrics.clone()).await;
        match maybe_tx_ids {
            RequestTxHistoryResult::Ok(all_tx_ids_with_height) => {
                let in_storage = match ctx.storage.unique_tx_hashes_num_in_history(&wallet_id).await {
                    Ok(num) => num,
                    Err(e) => return Self::change_state(Stopped::storage_error(e)),
                };
                if all_tx_ids_with_height.len() > in_storage {
                    let txes_left = all_tx_ids_with_height.len() - in_storage;
                    let new_state_json = json!({ "transactions_left": txes_left });
                    ctx.coin
                        .set_history_sync_state(HistorySyncState::InProgress(new_state_json));
                }

                Self::change_state(UpdatingUnconfirmedTxes::new(all_tx_ids_with_height))
            },
            RequestTxHistoryResult::HistoryTooLarge => Self::change_state(Stopped::history_too_large()),
            RequestTxHistoryResult::Retry { error } => {
                error!("Error {} on requesting tx history for {}", error, ctx.coin.ticker());
                Self::change_state(OnIoErrorCooldown::new())
            },
            RequestTxHistoryResult::CriticalError(e) => {
                error!(
                    "Critical error {} on requesting tx history for {}",
                    e,
                    ctx.coin.ticker()
                );
                Self::change_state(Stopped::unknown(e))
            },
        }
    }
}

// States have to be generic over storage type because BchAndSlpHistoryCtx is generic over it
struct OnIoErrorCooldown<Coin, Storage> {
    phantom: std::marker::PhantomData<(Coin, Storage)>,
}

impl<Coin, Storage> OnIoErrorCooldown<Coin, Storage> {
    fn new() -> Self {
        OnIoErrorCooldown {
            phantom: Default::default(),
        }
    }
}

impl<Coin, Storage> TransitionFrom<FetchingTxHashes<Coin, Storage>> for OnIoErrorCooldown<Coin, Storage> {}
impl<Coin, Storage> TransitionFrom<FetchingTransactionsData<Coin, Storage>> for OnIoErrorCooldown<Coin, Storage> {}
impl<Coin, Storage> TransitionFrom<UpdatingUnconfirmedTxes<Coin, Storage>> for OnIoErrorCooldown<Coin, Storage> {}

#[async_trait]
impl<Coin, Storage> State for OnIoErrorCooldown<Coin, Storage>
where
    Coin: UtxoTxHistoryOps,
    Storage: TxHistoryStorage,
{
    type Ctx = UtxoTxHistoryCtx<Coin, Storage>;
    type Result = ();

    async fn on_changed(self: Box<Self>, _ctx: &mut Self::Ctx) -> StateResult<Self::Ctx, Self::Result> {
        Timer::sleep(30.).await;
        Self::change_state(FetchingTxHashes::new())
    }
}

// States have to be generic over storage type because BchAndSlpHistoryCtx is generic over it
struct WaitForHistoryUpdateTrigger<Coin, Storage> {
    phantom: std::marker::PhantomData<(Coin, Storage)>,
}

impl<Coin, Storage> WaitForHistoryUpdateTrigger<Coin, Storage> {
    fn new() -> Self {
        WaitForHistoryUpdateTrigger {
            phantom: Default::default(),
        }
    }
}

impl<Coin, Storage> TransitionFrom<FetchingTransactionsData<Coin, Storage>>
    for WaitForHistoryUpdateTrigger<Coin, Storage>
{
}

#[async_trait]
impl<Coin, Storage> State for WaitForHistoryUpdateTrigger<Coin, Storage>
where
    Coin: UtxoTxHistoryOps,
    Storage: TxHistoryStorage,
{
    type Ctx = UtxoTxHistoryCtx<Coin, Storage>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut Self::Ctx) -> StateResult<Self::Ctx, Self::Result> {
        let wallet_id = ctx.coin.history_wallet_id();
        loop {
            Timer::sleep(30.).await;
            match ctx.storage.history_contains_unconfirmed_txes(&wallet_id).await {
                Ok(contains) => {
                    if contains {
                        return Self::change_state(FetchingTxHashes::new());
                    }
                },
                Err(e) => return Self::change_state(Stopped::storage_error(e)),
            }

            let current_balances = match ctx.coin.get_addresses_balances().await {
                Ok(balances) => balances,
                Err(e) => {
                    error!("Error {e:?} on balance fetching for the coin {}", ctx.coin.ticker());
                    continue;
                },
            };

            let mut to_update = false;
            for (address, current_balance) in current_balances {
                match ctx.balances.entry(address) {
                    // Do nothing if the balance hasn't been changed.
                    Entry::Occupied(entry) if *entry.get() == current_balance => {},
                    Entry::Occupied(mut entry) => {
                        to_update = true;
                        entry.insert(current_balance);
                    },
                    Entry::Vacant(entry) => {
                        to_update = true;
                        entry.insert(current_balance);
                    },
                }
            }

            if to_update {
                return Self::change_state(FetchingTxHashes::new());
            }
        }
    }
}

// States have to be generic over storage type because BchAndSlpHistoryCtx is generic over it
struct UpdatingUnconfirmedTxes<Coin, Storage> {
    phantom: std::marker::PhantomData<(Coin, Storage)>,
    all_tx_ids_with_height: Vec<(H256Json, u64)>,
}

impl<Coin, Storage> UpdatingUnconfirmedTxes<Coin, Storage> {
    fn new(all_tx_ids_with_height: Vec<(H256Json, u64)>) -> Self {
        UpdatingUnconfirmedTxes {
            phantom: Default::default(),
            all_tx_ids_with_height,
        }
    }
}

impl<Coin, Storage> TransitionFrom<FetchingTxHashes<Coin, Storage>> for UpdatingUnconfirmedTxes<Coin, Storage> {}

#[async_trait]
impl<Coin, Storage> State for UpdatingUnconfirmedTxes<Coin, Storage>
where
    Coin: UtxoTxHistoryOps,
    Storage: TxHistoryStorage,
{
    type Ctx = UtxoTxHistoryCtx<Coin, Storage>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut Self::Ctx) -> StateResult<Self::Ctx, Self::Result> {
        let wallet_id = ctx.coin.history_wallet_id();
        match ctx.storage.get_unconfirmed_txes_from_history(&wallet_id).await {
            Ok(unconfirmed) => {
                let txs_with_height: HashMap<H256Json, u64> = self.all_tx_ids_with_height.clone().into_iter().collect();
                for mut tx in unconfirmed {
                    let found = match H256Json::from_str(&tx.tx_hash) {
                        Ok(unconfirmed_tx_hash) => txs_with_height.get(&unconfirmed_tx_hash),
                        Err(_) => None,
                    };

                    match found {
                        Some(height) => {
                            if *height > 0 {
                                match ctx.coin.get_block_timestamp(*height).await {
                                    Ok(time) => tx.timestamp = time,
                                    Err(_) => return Self::change_state(OnIoErrorCooldown::new()),
                                };
                                tx.block_height = *height;
                                if let Err(e) = ctx.storage.update_tx_in_history(&wallet_id, &tx).await {
                                    return Self::change_state(Stopped::storage_error(e));
                                }
                            }
                        },
                        None => {
                            // This can potentially happen when unconfirmed tx is removed from mempool for some reason.
                            // Or if the hash is undecodable. We should remove it from storage too.
                            if let Err(e) = ctx.storage.remove_tx_from_history(&wallet_id, &tx.internal_id).await {
                                return Self::change_state(Stopped::storage_error(e));
                            }
                        },
                    }
                }
                Self::change_state(FetchingTransactionsData::new(self.all_tx_ids_with_height))
            },
            Err(e) => Self::change_state(Stopped::storage_error(e)),
        }
    }
}

// States have to be generic over storage type because BchAndSlpHistoryCtx is generic over it
struct FetchingTransactionsData<Coin, Storage> {
    phantom: std::marker::PhantomData<(Coin, Storage)>,
    all_tx_ids_with_height: Vec<(H256Json, u64)>,
}

impl<Coin, Storage> TransitionFrom<UpdatingUnconfirmedTxes<Coin, Storage>> for FetchingTransactionsData<Coin, Storage> {}

impl<Coin, Storage> FetchingTransactionsData<Coin, Storage> {
    fn new(all_tx_ids_with_height: Vec<(H256Json, u64)>) -> Self {
        FetchingTransactionsData {
            phantom: Default::default(),
            all_tx_ids_with_height,
        }
    }
}

#[async_trait]
impl<Coin, Storage> State for FetchingTransactionsData<Coin, Storage>
where
    Coin: UtxoTxHistoryOps,
    Storage: TxHistoryStorage,
{
    type Ctx = UtxoTxHistoryCtx<Coin, Storage>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut Self::Ctx) -> StateResult<Self::Ctx, Self::Result> {
        let wallet_id = ctx.coin.history_wallet_id();
        let my_addresses = match ctx.coin.my_addresses().await {
            Ok(addresses) => addresses,
            Err(e) => return Self::change_state(Stopped::unknown(format!("Error on getting my addresses: {e}"))),
        };

        for (tx_hash, height) in self.all_tx_ids_with_height {
            let tx_hash_string = format!("{:02x}", tx_hash);
            match ctx.storage.history_has_tx_hash(&wallet_id, &tx_hash_string).await {
                Ok(true) => continue,
                Ok(false) => (),
                Err(e) => return Self::change_state(Stopped::storage_error(e)),
            }

            let block_height_and_time = if height > 0 {
                let timestamp = match ctx.coin.get_block_timestamp(height).await {
                    Ok(time) => time,
                    Err(_) => return Self::change_state(OnIoErrorCooldown::new()),
                };
                Some(BlockHeightAndTime { height, timestamp })
            } else {
                None
            };
            let params = UtxoTxDetailsParams {
                hash: &tx_hash,
                block_height_and_time,
                storage: &ctx.storage,
                my_addresses: &my_addresses,
            };
            let tx_details = match ctx.coin.tx_details_by_hash(params).await {
                Ok(tx) => tx,
                Err(e) => {
                    error!(
                        "Error {:?} on getting {} tx details for hash {:02x}",
                        e,
                        ctx.coin.ticker(),
                        tx_hash
                    );
                    return Self::change_state(OnIoErrorCooldown::new());
                },
            };

            if let Err(e) = ctx.storage.add_transactions_to_history(&wallet_id, tx_details).await {
                return Self::change_state(Stopped::storage_error(e));
            }

            // wait for for one second to reduce the number of requests to electrum servers
            Timer::sleep(1.).await;
        }
        info!("Tx history fetching finished for {}", ctx.coin.ticker());
        ctx.coin.set_history_sync_state(HistorySyncState::Finished);
        Self::change_state(WaitForHistoryUpdateTrigger::new())
    }
}

#[derive(Debug)]
enum StopReason {
    HistoryTooLarge,
    StorageError(String),
    UnknownError(String),
}

struct Stopped<Coin, Storage> {
    phantom: std::marker::PhantomData<(Coin, Storage)>,
    stop_reason: StopReason,
}

impl<Coin, Storage> Stopped<Coin, Storage> {
    fn history_too_large() -> Self {
        Stopped {
            phantom: Default::default(),
            stop_reason: StopReason::HistoryTooLarge,
        }
    }

    fn storage_error<E>(e: E) -> Self
    where
        E: std::fmt::Debug,
    {
        Stopped {
            phantom: Default::default(),
            stop_reason: StopReason::StorageError(format!("{:?}", e)),
        }
    }

    fn unknown(e: String) -> Self {
        Stopped {
            phantom: Default::default(),
            stop_reason: StopReason::UnknownError(e),
        }
    }
}

impl<Coin, Storage> TransitionFrom<FetchingTxHashes<Coin, Storage>> for Stopped<Coin, Storage> {}
impl<Coin, Storage> TransitionFrom<UpdatingUnconfirmedTxes<Coin, Storage>> for Stopped<Coin, Storage> {}
impl<Coin, Storage> TransitionFrom<WaitForHistoryUpdateTrigger<Coin, Storage>> for Stopped<Coin, Storage> {}
impl<Coin, Storage> TransitionFrom<FetchingTransactionsData<Coin, Storage>> for Stopped<Coin, Storage> {}

#[async_trait]
impl<Coin, Storage> LastState for Stopped<Coin, Storage>
where
    Coin: UtxoTxHistoryOps,
    Storage: TxHistoryStorage,
{
    type Ctx = UtxoTxHistoryCtx<Coin, Storage>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut Self::Ctx) -> Self::Result {
        info!(
            "Stopping tx history fetching for {}. Reason: {:?}",
            ctx.coin.ticker(),
            self.stop_reason
        );
        let new_state_json = match self.stop_reason {
            StopReason::HistoryTooLarge => json!({
                "code": utxo_common::HISTORY_TOO_LARGE_ERR_CODE,
                "message": "Got `history too large` error from Electrum server. History is not available",
            }),
            reason => json!({
                "message": format!("{:?}", reason),
            }),
        };
        ctx.coin.set_history_sync_state(HistorySyncState::Error(new_state_json));
    }
}

pub async fn bch_and_slp_history_loop(
    coin: BchCoin,
    storage: impl TxHistoryStorage,
    metrics: MetricsArc,
    current_balance: BigDecimal,
) {
    let my_address = match coin.my_address() {
        Ok(my_address) => my_address,
        Err(e) => {
            error!("{}", e);
            return;
        },
    };
    let mut balances = HashMap::new();
    balances.insert(my_address, current_balance);
    drop_mutability!(balances);

    let ctx = UtxoTxHistoryCtx {
        coin,
        storage,
        metrics,
        balances,
    };
    let state_machine: StateMachine<_, ()> = StateMachine::from_ctx(ctx);
    state_machine.run(Init::new()).await;
}

pub async fn utxo_history_loop<Coin, Storage>(
    coin: Coin,
    storage: Storage,
    metrics: MetricsArc,
    current_balances: HashMap<String, BigDecimal>,
) where
    Coin: UtxoTxHistoryOps,
    Storage: TxHistoryStorage,
{
    let ctx = UtxoTxHistoryCtx {
        coin,
        storage,
        metrics,
        balances: current_balances,
    };
    let state_machine: StateMachine<_, ()> = StateMachine::from_ctx(ctx);
    state_machine.run(Init::new()).await;
}
