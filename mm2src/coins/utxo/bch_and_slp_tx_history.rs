/// This module is named bch_and_slp_tx_history temporary. We will most likely use the same approach for every
/// supported UTXO coin.
use super::RequestTxHistoryResult;
use crate::my_tx_history_v2::{CoinWithTxHistoryV2, TxHistoryStorage};
use crate::utxo::bch::BchCoin;
use crate::utxo::utxo_common;
use crate::utxo::UtxoStandardOps;
use crate::{BlockHeightAndTime, HistorySyncState, MarketCoinOps};
use async_trait::async_trait;
use common::executor::Timer;
use common::log::{error, info};
use common::mm_metrics::MetricsArc;
use common::mm_number::BigDecimal;
use common::state_machine::prelude::*;
use futures::compat::Future01CompatExt;
use rpc::v1::types::H256 as H256Json;

struct BchAndSlpHistoryCtx<Storage: TxHistoryStorage> {
    coin: BchCoin,
    storage: Storage,
    metrics: MetricsArc,
    current_balance: BigDecimal,
}

// States have to be generic over storage type because BchAndSlpHistoryCtx is generic over it
struct Init<T> {
    phantom: std::marker::PhantomData<T>,
}

impl<T> Init<T> {
    fn new() -> Self {
        Init {
            phantom: Default::default(),
        }
    }
}

impl<T, E> TransitionFrom<Init<T>> for Stopped<T, E> {}

#[async_trait]
impl<T: TxHistoryStorage> State for Init<T> {
    type Ctx = BchAndSlpHistoryCtx<T>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut BchAndSlpHistoryCtx<T>) -> StateResult<BchAndSlpHistoryCtx<T>, ()> {
        *ctx.coin.as_ref().history_sync_state.lock().unwrap() = HistorySyncState::NotStarted;

        if let Err(e) = ctx.storage.init(&ctx.coin.history_wallet_id()).await {
            return Self::change_state(Stopped::storage_error(e));
        }

        Self::change_state(FetchingTxHashes::new())
    }
}

// States have to be generic over storage type because BchAndSlpHistoryCtx is generic over it
struct FetchingTxHashes<T> {
    phantom: std::marker::PhantomData<T>,
}

impl<T> FetchingTxHashes<T> {
    fn new() -> Self {
        FetchingTxHashes {
            phantom: Default::default(),
        }
    }
}

impl<T> TransitionFrom<Init<T>> for FetchingTxHashes<T> {}
impl<T> TransitionFrom<OnIoErrorCooldown<T>> for FetchingTxHashes<T> {}
impl<T> TransitionFrom<WaitForHistoryUpdateTrigger<T>> for FetchingTxHashes<T> {}

#[async_trait]
impl<T: TxHistoryStorage> State for FetchingTxHashes<T> {
    type Ctx = BchAndSlpHistoryCtx<T>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut BchAndSlpHistoryCtx<T>) -> StateResult<BchAndSlpHistoryCtx<T>, ()> {
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
                    *ctx.coin.as_ref().history_sync_state.lock().unwrap() =
                        HistorySyncState::InProgress(json!({ "transactions_left": txes_left }));
                }

                Self::change_state(UpdatingUnconfirmedTxes::new(all_tx_ids_with_height))
            },
            RequestTxHistoryResult::HistoryTooLarge => Self::change_state(Stopped::<T, T::Error>::history_too_large()),
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
                Self::change_state(Stopped::<T, T::Error>::unknown(e))
            },
        }
    }
}

// States have to be generic over storage type because BchAndSlpHistoryCtx is generic over it
struct OnIoErrorCooldown<T> {
    phantom: std::marker::PhantomData<T>,
}

impl<T> OnIoErrorCooldown<T> {
    fn new() -> Self {
        OnIoErrorCooldown {
            phantom: Default::default(),
        }
    }
}

impl<T> TransitionFrom<FetchingTxHashes<T>> for OnIoErrorCooldown<T> {}
impl<T> TransitionFrom<FetchingTransactionsData<T>> for OnIoErrorCooldown<T> {}
impl<T> TransitionFrom<UpdatingUnconfirmedTxes<T>> for OnIoErrorCooldown<T> {}

#[async_trait]
impl<T: TxHistoryStorage> State for OnIoErrorCooldown<T> {
    type Ctx = BchAndSlpHistoryCtx<T>;
    type Result = ();

    async fn on_changed(self: Box<Self>, _ctx: &mut BchAndSlpHistoryCtx<T>) -> StateResult<BchAndSlpHistoryCtx<T>, ()> {
        Timer::sleep(30.).await;
        Self::change_state(FetchingTxHashes::new())
    }
}

// States have to be generic over storage type because BchAndSlpHistoryCtx is generic over it
struct WaitForHistoryUpdateTrigger<T> {
    phantom: std::marker::PhantomData<T>,
}

impl<T> WaitForHistoryUpdateTrigger<T> {
    fn new() -> Self {
        WaitForHistoryUpdateTrigger {
            phantom: Default::default(),
        }
    }
}

impl<T> TransitionFrom<FetchingTransactionsData<T>> for WaitForHistoryUpdateTrigger<T> {}

#[async_trait]
impl<T: TxHistoryStorage> State for WaitForHistoryUpdateTrigger<T> {
    type Ctx = BchAndSlpHistoryCtx<T>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut BchAndSlpHistoryCtx<T>) -> StateResult<Self::Ctx, Self::Result> {
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

            match ctx.coin.my_balance().compat().await {
                Ok(balance) => {
                    let total_balance = balance.into_total();
                    if ctx.current_balance != total_balance {
                        ctx.current_balance = total_balance;
                        return Self::change_state(FetchingTxHashes::new());
                    }
                },
                Err(e) => {
                    error!("Error {} on balance fetching for the coin {}", e, ctx.coin.ticker());
                },
            }
        }
    }
}

// States have to be generic over storage type because BchAndSlpHistoryCtx is generic over it
struct UpdatingUnconfirmedTxes<T> {
    phantom: std::marker::PhantomData<T>,
    all_tx_ids_with_height: Vec<(H256Json, u64)>,
}

impl<T> UpdatingUnconfirmedTxes<T> {
    fn new(all_tx_ids_with_height: Vec<(H256Json, u64)>) -> Self {
        UpdatingUnconfirmedTxes {
            phantom: Default::default(),
            all_tx_ids_with_height,
        }
    }
}

impl<T> TransitionFrom<FetchingTxHashes<T>> for UpdatingUnconfirmedTxes<T> {}

#[async_trait]
impl<T: TxHistoryStorage> State for UpdatingUnconfirmedTxes<T> {
    type Ctx = BchAndSlpHistoryCtx<T>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut BchAndSlpHistoryCtx<T>) -> StateResult<BchAndSlpHistoryCtx<T>, ()> {
        let wallet_id = ctx.coin.history_wallet_id();
        match ctx.storage.get_unconfirmed_txes_from_history(&wallet_id).await {
            Ok(unconfirmed) => {
                for mut tx in unconfirmed {
                    let found = self
                        .all_tx_ids_with_height
                        .iter()
                        .find(|(hash, _)| hash.0.as_ref() == tx.tx_hash.as_bytes());
                    match found {
                        Some((_, height)) => {
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
                            // We should remove it from storage too.
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
struct FetchingTransactionsData<T> {
    phantom: std::marker::PhantomData<T>,
    all_tx_ids_with_height: Vec<(H256Json, u64)>,
}

impl<T> TransitionFrom<UpdatingUnconfirmedTxes<T>> for FetchingTransactionsData<T> {}

impl<T> FetchingTransactionsData<T> {
    fn new(all_tx_ids_with_height: Vec<(H256Json, u64)>) -> Self {
        FetchingTransactionsData {
            phantom: Default::default(),
            all_tx_ids_with_height,
        }
    }
}

#[async_trait]
impl<T: TxHistoryStorage> State for FetchingTransactionsData<T> {
    type Ctx = BchAndSlpHistoryCtx<T>;
    type Result = ();

    async fn on_changed(self: Box<Self>, ctx: &mut BchAndSlpHistoryCtx<T>) -> StateResult<BchAndSlpHistoryCtx<T>, ()> {
        let wallet_id = ctx.coin.history_wallet_id();
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
            let tx_details = match ctx
                .coin
                .transaction_details_with_token_transfers(&tx_hash, block_height_and_time, &ctx.storage)
                .await
            {
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
        *ctx.coin.as_ref().history_sync_state.lock().unwrap() = HistorySyncState::Finished;
        Self::change_state(WaitForHistoryUpdateTrigger::new())
    }
}

#[derive(Debug)]
enum StopReason<E> {
    HistoryTooLarge,
    StorageError(E),
    UnknownError(String),
}

struct Stopped<T, E> {
    phantom: std::marker::PhantomData<T>,
    stop_reason: StopReason<E>,
}

impl<T, E> Stopped<T, E> {
    fn history_too_large() -> Self {
        Stopped {
            phantom: Default::default(),
            stop_reason: StopReason::HistoryTooLarge,
        }
    }

    fn storage_error(e: E) -> Self {
        Stopped {
            phantom: Default::default(),
            stop_reason: StopReason::StorageError(e),
        }
    }

    fn unknown(e: String) -> Self {
        Stopped {
            phantom: Default::default(),
            stop_reason: StopReason::UnknownError(e),
        }
    }
}

impl<T, E> TransitionFrom<FetchingTxHashes<T>> for Stopped<T, E> {}
impl<T, E> TransitionFrom<UpdatingUnconfirmedTxes<T>> for Stopped<T, E> {}
impl<T, E> TransitionFrom<WaitForHistoryUpdateTrigger<T>> for Stopped<T, E> {}
impl<T, E> TransitionFrom<FetchingTransactionsData<T>> for Stopped<T, E> {}

#[async_trait]
impl<T: TxHistoryStorage, E: std::fmt::Debug + Send + 'static> LastState for Stopped<T, E> {
    type Ctx = BchAndSlpHistoryCtx<T>;
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
        *ctx.coin.as_ref().history_sync_state.lock().unwrap() = HistorySyncState::Error(new_state_json);
    }
}

pub async fn bch_and_slp_history_loop(
    coin: BchCoin,
    storage: impl TxHistoryStorage,
    metrics: MetricsArc,
    current_balance: BigDecimal,
) {
    let ctx = BchAndSlpHistoryCtx {
        coin,
        storage,
        metrics,
        current_balance,
    };
    let state_machine: StateMachine<_, ()> = StateMachine::from_ctx(ctx);
    state_machine.run(Init::new()).await;
}
