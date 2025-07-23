//! State machine for taker aggregated swap: liquidity routing swap (opt) + atomic swap + liquidity routing swap (opt)

use super::lr_errors::LrSwapError;
use super::lr_helpers;
use super::{AtomicSwapParams, LrSwapParams};
use crate::common::executor::SpawnAbortable;
use crate::common::log::LogOnError;
use crate::lp_ordermatch::lp_auto_buy;
use crate::lp_swap::swap_lock::SwapLock;
use crate::lp_swap::swap_v2_common::*;
use crate::lp_swap::swap_v2_rpcs::{my_swap_status_rpc, MySwapStatusError, MySwapStatusRequest, SwapRpcData};
use crate::lp_swap::{check_balance_for_taker_swap, AGG_TAKER_SWAP_TYPE};
use async_trait::async_trait;
use coins::eth::{u256_from_big_decimal, u256_to_big_decimal, EthCoinType};
use coins::hd_wallet::DisplayAddress;
use coins::{lp_coinfind_or_err, ConfirmPaymentInput, FeeApproxStage, MarketCoinOps, SignEthTransactionParams,
            SignRawTransactionEnum, SignRawTransactionRequest};
use coins::{DexFee, MmCoin};
use coins::{MmCoinEnum, Ticker};
use common::executor::abortable_queue::AbortableQueue;
use common::executor::AbortSettings;
use common::executor::AbortableSystem;
use common::executor::Timer;
use common::log::{debug, info, warn};
use common::Future01CompatExt;
use common::{new_uuid, now_sec};
use derive_more::Display;
use ethereum_types::{Address as EthAddress, U256};
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::map_mm_error::MapMmError;
use mm2_err_handle::prelude::*;
use mm2_number::MmNumber;
use mm2_rpc::data::legacy::{Mm2RpcResult, SellBuyResponse, TakerAction};
use mm2_state_machine::prelude::*;
use mm2_state_machine::storable_state_machine::*;
use num_traits::CheckedDiv;
use rpc::v1::types::Bytes as BytesJson;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::OnceLock;
use trading_api::one_inch_api::classic_swap_types::ClassicSwapData;
use trading_api::one_inch_api::client::{ApiClient, SwapApiMethods, SwapUrlBuilder};
use uuid::Uuid;

const LR_SWAP_CONFIRMATIONS: u64 = 1;
const LR_SWAP_WAIT_CONFIRM_TIMEOUT_SEC: u64 = 60;
const LR_SWAP_WAIT_CONFIRM_INTERVAL_SEC: u64 = 15;

cfg_native!(
    use common::async_blocking;
    use db_common::sqlite::rusqlite::{named_params, Error as SqlError, Result as SqlResult, Row};
    use db_common::sqlite::rusqlite::types::Type as SqlType;
    use crate::database::my_lr_swaps::insert_new_lr_swap;
    use crate::database::my_lr_swaps::SELECT_LR_SWAP_BY_UUID;
);

cfg_wasm32!(
    use crate::lp_swap::SwapsContext;
    use crate::lp_swap::swap_wasm_db::{MySwapsFiltersTable, SavedSwapTable};
    use crate::swap_versioning::legacy_swap_version;
);

const LOG_PREFIX: &str = "Taker swap with LR";

/// Represents events produced by aggregated taker swap states.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "event_type", content = "event_data")]
pub enum AggTakerSwapEvent {
    /// Run LR-swap before atomic swap and get its result
    RunLrSwap0 {},
    /// Waiting for LR tx 0 to confirm
    WaitingForLrTxConfirmation0 {
        coin: Ticker,
        tx_bytes: BytesJson,
        dst_amount: MmNumber,
        slippage: f32,
    },
    /// Atomic swap has been successfully started
    StartAtomicSwap {
        volume: MmNumber,
        action: TakerAction,
        slippage: f32,
    },
    /// Waiting for running atomic swap
    WaitingForAtomicSwap {
        atomic_swap_uuid: Uuid,
        maker_volume: MmNumber,
    },
    /// Run LR-swap after atomic swap and get its result
    RunLrSwap1 {},
    /// Waiting for LR tx 1 to confirm
    WaitingForLrTxConfirmation1 { coin: Ticker, tx_bytes: BytesJson },
    /// Aggregated swap has been aborted before any payment was sent.
    Aborted { reason: states::AbortReason },
    /// Aggregated swap completed successfully.
    Completed,
}

/// Storage for aggregated swaps
#[derive(Clone)]
pub struct AggTakerSwapStorage {
    ctx: MmArc,
}

impl AggTakerSwapStorage {
    pub fn new(ctx: MmArc) -> Self { AggTakerSwapStorage { ctx } }
}

#[async_trait]
impl StateMachineStorage for AggTakerSwapStorage {
    type MachineId = Uuid;
    type DbRepr = AggTakerSwapDbRepr;
    type Error = MmError<SwapStateMachineError>;

    #[cfg(not(target_arch = "wasm32"))]
    async fn store_repr(&mut self, _id: Self::MachineId, repr: Self::DbRepr) -> Result<(), Self::Error> {
        let ctx = self.ctx.clone();

        async_blocking(move || {
            let lr_swap_0_ser = if let Some(lr_swap_0) = repr.lr_swap_0 {
                serde_json::to_string(&lr_swap_0)
                    .map_to_mm(|ser_err| SwapStateMachineError::SerdeError(ser_err.to_string()))?
            } else {
                Default::default() // We should insert empty string to let sqlite know about the field type
            };
            let sell_buy_req_ser = serde_json::to_string(&repr.atomic_swap)
                .map_to_mm(|ser_err| SwapStateMachineError::SerdeError(ser_err.to_string()))?;
            let lr_swap_1_ser = if let Some(lr_swap_1) = repr.lr_swap_1 {
                serde_json::to_string(&lr_swap_1)
                    .map_to_mm(|ser_err| SwapStateMachineError::SerdeError(ser_err.to_string()))?
            } else {
                Default::default() // We should insert empty string to let sqlite know about the field type
            };
            let sql_params = named_params! {
                ":my_coin": repr.taker_coin,
                ":other_coin": repr.maker_coin,
                ":uuid": repr.uuid.to_string(),
                ":started_at": repr.started_at,
                ":swap_type": AGG_TAKER_SWAP_TYPE,
                ":swap_version": repr.swap_version,
                ":taker_volume": repr.source_volume.to_fraction_string(),
                ":maker_volume": repr.destination_volume.to_fraction_string(),
                ":lr_swap_0": lr_swap_0_ser,
                ":sell_buy_req": sell_buy_req_ser,
                ":lr_swap_1": lr_swap_1_ser,
            };
            insert_new_lr_swap(&ctx, sql_params)?;
            Ok(())
        })
        .await
    }

    #[cfg(target_arch = "wasm32")]
    async fn store_repr(&mut self, uuid: Self::MachineId, repr: Self::DbRepr) -> Result<(), Self::Error> {
        let swaps_ctx = SwapsContext::from_ctx(&self.ctx).expect("SwapsContext::from_ctx should not fail");
        let db = swaps_ctx.swap_db().await.map_mm_err()?;
        let transaction = db.transaction().await.map_mm_err()?;
        let filters_table = transaction.table::<MySwapsFiltersTable>().await.map_mm_err()?;

        let item = MySwapsFiltersTable {
            uuid,
            my_coin: repr.taker_coin.clone(),
            other_coin: repr.maker_coin.clone(),
            started_at: repr.started_at as u32,
            is_finished: false.into(),
            swap_type: AGG_TAKER_SWAP_TYPE,
        };
        filters_table.add_item(&item).await.map_mm_err()?;

        let table = transaction.table::<SavedSwapTable>().await.map_mm_err()?;
        let item = SavedSwapTable {
            uuid,
            saved_swap: serde_json::to_value(repr)?,
        };
        table.add_item(&item).await.map_mm_err()?;
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    async fn get_repr(&self, id: Self::MachineId) -> Result<Self::DbRepr, Self::Error> {
        let ctx = self.ctx.clone();
        let id_str = id.to_string();

        async_blocking(move || {
            Ok(ctx.sqlite_connection().query_row(
                SELECT_LR_SWAP_BY_UUID,
                &[(":uuid", &id_str)],
                Self::DbRepr::from_sql_row,
            )?)
        })
        .await
    }

    #[cfg(target_arch = "wasm32")]
    async fn get_repr(&self, id: Self::MachineId) -> Result<Self::DbRepr, Self::Error> {
        get_swap_repr(&self.ctx, id).await
    }

    async fn has_record_for(&mut self, id: &Self::MachineId) -> Result<bool, Self::Error> {
        has_db_record_for(self.ctx.clone(), id).await
    }

    async fn store_event(&mut self, id: Self::MachineId, event: AggTakerSwapEvent) -> Result<(), Self::Error> {
        store_swap_event::<AggTakerSwapDbRepr>(self.ctx.clone(), id, event).await
    }

    async fn get_unfinished(&self) -> Result<Vec<Self::MachineId>, Self::Error> {
        get_unfinished_swaps_uuids(self.ctx.clone(), AGG_TAKER_SWAP_TYPE).await
    }

    async fn mark_finished(&mut self, id: Self::MachineId) -> Result<(), Self::Error> {
        mark_swap_as_finished(self.ctx.clone(), id).await
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AggTakerSwapDbRepr {
    pub maker_coin: String,
    pub taker_coin: String,
    pub uuid: Uuid,
    pub started_at: u64,
    /// Is swap finished, the value is set only from db
    pub is_finished: bool,
    pub source_volume: MmNumber,
    pub destination_volume: MmNumber,
    pub lr_swap_0: Option<LrSwapParams>,
    pub lr_swap_1: Option<LrSwapParams>,
    pub atomic_swap: AtomicSwapParams,
    pub events: Vec<AggTakerSwapEvent>,
    pub swap_version: u8,
}

impl StateMachineDbRepr for AggTakerSwapDbRepr {
    type Event = AggTakerSwapEvent;

    fn add_event(&mut self, event: Self::Event) { self.events.push(event) }
}

#[cfg(not(target_arch = "wasm32"))]
impl AggTakerSwapDbRepr {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn get_repr_impl(ctx: &MmArc, id: &Uuid) -> SqlResult<Self> {
        let id_str = id.to_string();
        let ctx = ctx.clone();

        async_blocking(move || {
            ctx.sqlite_connection()
                .query_row(SELECT_LR_SWAP_BY_UUID, &[(":uuid", &id_str)], Self::from_sql_row)
        })
        .await
    }

    fn from_sql_row(row: &Row) -> SqlResult<Self> {
        Ok(AggTakerSwapDbRepr {
            taker_coin: row.get(0)?,
            maker_coin: row.get(1)?,
            uuid: row
                .get::<_, String>(2)?
                .parse()
                .map_err(|e| SqlError::FromSqlConversionFailure(2, SqlType::Text, Box::new(e)))?,
            started_at: row.get(3)?,
            is_finished: row.get(4)?,
            events: serde_json::from_str(&row.get::<_, String>(5)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(5, SqlType::Text, Box::new(e)))?,
            swap_version: row.get(6)?,
            source_volume: MmNumber::from_fraction_string(&row.get::<_, String>(7)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(7, SqlType::Text, Box::new(e)))?,
            destination_volume: MmNumber::from_fraction_string(&row.get::<_, String>(8)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(8, SqlType::Text, Box::new(e)))?,
            lr_swap_0: serde_json::from_str::<LrSwapParams>(&row.get::<_, String>(9)?).ok(),
            atomic_swap: serde_json::from_str::<AtomicSwapParams>(&row.get::<_, String>(10)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(10, SqlType::Text, Box::new(e)))?,
            lr_swap_1: serde_json::from_str::<LrSwapParams>(&row.get::<_, String>(11)?).ok(),
        })
    }
}

impl AggTakerSwapDbRepr {
    pub(crate) fn source_coin(&self) -> Ticker {
        if let Some(ref lr_swap_0) = self.lr_swap_0 {
            lr_swap_0.src.clone()
        } else {
            self.atomic_swap.taker_coin()
        }
    }

    pub(crate) fn destination_coin(&self) -> Ticker {
        if let Some(ref lr_swap_1) = self.lr_swap_1 {
            lr_swap_1.dst.clone()
        } else {
            self.atomic_swap.maker_coin()
        }
    }

    pub(crate) fn routing_coin_0(&self) -> Option<Ticker> { self.lr_swap_0.as_ref().map(|p| p.dst.clone()) }

    pub(crate) fn routing_coin_1(&self) -> Option<Ticker> { self.lr_swap_1.as_ref().map(|p| p.dst.clone()) }
}

/// Represents the state machine for maker's side of the Trading Protocol Upgrade swap (v2).
pub struct AggTakerSwapStateMachine {
    /// MM2 context
    ctx: MmArc,
    /// Storage
    storage: AggTakerSwapStorage,
    /// Abortable queue used to spawn related activities
    abortable_system: AbortableQueue,
    /// Params for the LR swap step running before the atomic swap (optional)
    lr_swap_0: Option<LrSwapParams>,
    /// Params for the LR swap step running after the atomic swap (optional)
    lr_swap_1: Option<LrSwapParams>,
    /// Sell or buy params for the atomic swap
    atomic_swap: AtomicSwapParams,
    /// Atomic swap step volume.
    /// TODO: for future use
    atomic_swap_maker_volume: OnceLock<MmNumber>,
    /// The UUID of the atomic swap
    atomic_swap_uuid: OnceLock<Uuid>,
    /// unique ID of the aggregated swap for stopping or querying status.
    uuid: Uuid,
    /// The timestamp when the aggregated swap was started.
    started_at: u64,
    /// Swap impl version
    swap_version: u8,
}

#[async_trait]
impl StorableStateMachine for AggTakerSwapStateMachine {
    type Storage = AggTakerSwapStorage;
    type Result = ();
    type Error = MmError<SwapStateMachineError>;
    type ReentrancyLock = SwapLock;
    type RecreateCtx = ();
    type RecreateError = MmError<SwapRecreateError>;

    fn to_db_repr(&self) -> AggTakerSwapDbRepr {
        AggTakerSwapDbRepr {
            maker_coin: self.atomic_swap_maker_coin(), // TODO: maybe we don't need these two fields and rely on lr_swap_0 lr_swap_1 and sell_buy_req
            taker_coin: self.atomic_swap_taker_coin(),
            uuid: self.uuid,
            started_at: self.started_at,
            source_volume: self.source_volume().unwrap_or_default(), // Safe as swap must be aborted if no source_volume
            destination_volume: self.destination_volume(),
            is_finished: false,
            lr_swap_0: self.lr_swap_0.clone(),
            lr_swap_1: self.lr_swap_1.clone(),
            atomic_swap: self.atomic_swap.clone(),
            events: Vec::new(),
            swap_version: self.swap_version,
        }
    }

    fn storage(&mut self) -> &mut Self::Storage { &mut self.storage }

    fn id(&self) -> <Self::Storage as StateMachineStorage>::MachineId { self.uuid }

    async fn recreate_machine(
        uuid: Uuid,
        storage: AggTakerSwapStorage,
        mut repr: AggTakerSwapDbRepr,
        _recreate_ctx: Self::RecreateCtx,
    ) -> Result<(RestoredMachine<Self>, Box<dyn RestoredState<StateMachine = Self>>), Self::RecreateError> {
        let current_state: Box<dyn RestoredState<StateMachine = Self>> = match repr
            .events
            .pop()
            .ok_or(MmError::new(SwapRecreateError::ReprEventsEmpty))?
        {
            AggTakerSwapEvent::StartAtomicSwap {
                volume,
                action,
                slippage,
            } => Box::new(states::StartAtomicSwap {
                volume,
                action,
                slippage,
            }),
            AggTakerSwapEvent::RunLrSwap0 {} => Box::new(states::RunLrSwap0 {
                lr_swap_params: repr
                    .lr_swap_0
                    .as_ref()
                    .ok_or(SwapRecreateError::FailedToParseData("LR data empty".to_string()))?
                    .clone(),
            }),
            AggTakerSwapEvent::RunLrSwap1 {} => Box::new(states::RunLrSwap1 {
                lr_swap_params: repr
                    .lr_swap_1
                    .as_ref()
                    .ok_or(SwapRecreateError::FailedToParseData("LR data empty".to_string()))?
                    .clone(),
                src_amount: Default::default(), // TODO:
            }),

            AggTakerSwapEvent::Aborted { .. } => return MmError::err(SwapRecreateError::SwapAborted),
            AggTakerSwapEvent::Completed => return MmError::err(SwapRecreateError::SwapCompleted),
            _ => return MmError::err(SwapRecreateError::FailedToParseData("".to_string())),
        };

        let machine = AggTakerSwapStateMachine {
            ctx: storage.ctx.clone(),
            abortable_system: storage
                .ctx
                .abortable_system
                .create_subsystem()
                .expect("create_subsystem should not fail"),
            storage,
            lr_swap_0: repr.lr_swap_0.clone(),
            lr_swap_1: repr.lr_swap_1.clone(),
            atomic_swap: repr.atomic_swap.clone(),
            atomic_swap_uuid: OnceLock::default(),
            atomic_swap_maker_volume: OnceLock::default(),
            uuid,
            swap_version: AggTakerSwapStateMachine::AGG_SWAP_VERSION,
            started_at: 0,
        };

        if let Some(atomic_swap_uuid) = Self::find_atomic_swap_uuid_in_events(&repr.events) {
            let _ = machine.atomic_swap_uuid.set(atomic_swap_uuid);
        }
        if let Some(atomic_swap_maker_volume) = Self::find_atomic_swap_maker_volume_in_events(&repr.events) {
            let _ = machine.atomic_swap_maker_volume.set(atomic_swap_maker_volume);
        }
        Ok((RestoredMachine::new(machine), current_state))
    }

    async fn acquire_reentrancy_lock(&self) -> Result<Self::ReentrancyLock, Self::Error> {
        acquire_reentrancy_lock_impl(&self.ctx, self.uuid).await
    }

    fn spawn_reentrancy_lock_renew(&mut self, guard: Self::ReentrancyLock) {
        spawn_reentrancy_lock_renew_impl(&self.abortable_system, self.uuid, guard)
    }

    fn init_additional_context(&mut self) {
        let swap_info = ActiveSwapV2Info {
            uuid: self.uuid,
            maker_coin: self.atomic_swap_maker_coin(),
            taker_coin: self.atomic_swap_taker_coin(),
            swap_type: AGG_TAKER_SWAP_TYPE,
        };
        init_agg_swap_context_impl(&self.ctx, swap_info);
    }

    fn clean_up_context(&mut self) { clean_up_agg_swap_context_impl(&self.ctx, &self.uuid); }

    fn on_event(&mut self, event: &AggTakerSwapEvent) {
        match event {
            AggTakerSwapEvent::StartAtomicSwap { .. } => {
                // TODO: No need to lock LR swap amounts?
            },
            AggTakerSwapEvent::Aborted { .. } | AggTakerSwapEvent::Completed => (),
            _ => (),
        }

        // TODO: add streamer call
        // Send a notification to the swap status streamer about a new event.
        /*self.ctx
        .event_stream_manager
        .send_fn(SwapStatusStreamer::derive_streamer_id(), || SwapStatusEvent::AggTaker {
            uuid: self.uuid,
            event: event.clone(),
        })
        .ok();*/
    }

    fn on_kickstart_event(&mut self, _event: AggTakerSwapEvent) {}
}

mod states {
    use super::*;

    /// Represents a state used to start a new aggregated taker swap.
    #[derive(Default)]
    pub struct Initialize {}

    impl InitialState for Initialize {
        type StateMachine = AggTakerSwapStateMachine;
    }

    #[async_trait]
    impl State for Initialize {
        type StateMachine = AggTakerSwapStateMachine;

        async fn on_changed(
            self: Box<Self>,
            state_machine: &mut Self::StateMachine,
        ) -> StateResult<Self::StateMachine> {
            info!(
                "{LOG_PREFIX} uuid: {} for {}/{} with amount {} starting...",
                state_machine.uuid,
                state_machine.source_coin(),
                state_machine.destination_coin(),
                state_machine.source_volume().unwrap_or_default().to_decimal()
            );
            if let Some(ref lr_swap_params) = state_machine.lr_swap_0 {
                let run_lr_swap = RunLrSwap0 {
                    lr_swap_params: lr_swap_params.clone(),
                };
                return Self::change_state(run_lr_swap, state_machine).await;
            }
            let Ok(volume) = state_machine.source_volume() else {
                info!("{LOG_PREFIX} failed: {}", "internal error: no source volume");
                let next_state = Aborted {
                    reason: AbortReason::SomeReason("internal error: no source volume".to_owned()),
                };
                return Self::change_state(next_state, state_machine).await;
            };
            let run_atomic_swap = StartAtomicSwap {
                volume,
                action: state_machine.atomic_swap.action.clone(),
                slippage: 0.0,
            };
            Self::change_state(run_atomic_swap, state_machine).await
        }
    }

    /// State to start atomic swap step
    pub(super) struct StartAtomicSwap {
        pub(super) volume: MmNumber,
        pub(super) action: TakerAction,
        pub(super) slippage: f32,
    }

    #[async_trait]
    impl State for StartAtomicSwap {
        type StateMachine = AggTakerSwapStateMachine;

        async fn on_changed(
            self: Box<Self>,
            state_machine: &mut Self::StateMachine,
        ) -> StateResult<Self::StateMachine> {
            match state_machine
                .start_lp_auto_buy(self.volume.clone(), self.slippage)
                .await
            {
                Ok(resp) => {
                    let _ = state_machine
                        .atomic_swap_uuid
                        .set(resp.request.uuid)
                        .expect("Atomic swap UUID should be empty");
                    // Get other party maker volume
                    let maker_volume = match resp.request.action {
                        TakerAction::Buy => resp.request.base_amount_rat.into(),
                        TakerAction::Sell => resp.request.rel_amount_rat.into(),
                    };
                    let next_state = WaitingForAtomicSwap {
                        atomic_swap_uuid: resp.request.uuid,
                        maker_volume,
                    };
                    Self::change_state(next_state, state_machine).await
                },
                Err(err) => {
                    info!("{LOG_PREFIX} failed: {}", err);
                    let next_state = Aborted {
                        reason: AbortReason::SomeReason(err.get_inner().to_string()),
                    };
                    Self::change_state(next_state, state_machine).await
                },
            }
        }
    }

    impl StorableState for StartAtomicSwap {
        type StateMachine = AggTakerSwapStateMachine;

        fn get_event(&self) -> AggTakerSwapEvent {
            AggTakerSwapEvent::StartAtomicSwap {
                volume: self.volume.clone(),
                action: self.action.clone(),
                slippage: self.slippage,
            }
        }
    }

    impl TransitionFrom<Initialize> for StartAtomicSwap {}
    impl TransitionFrom<WaitForLrTxConfirmation0> for StartAtomicSwap {}

    /// State to wait for the atomic swap step to finish
    pub(super) struct WaitingForAtomicSwap {
        atomic_swap_uuid: Uuid,
        maker_volume: MmNumber,
    }

    #[async_trait]
    impl State for WaitingForAtomicSwap {
        type StateMachine = AggTakerSwapStateMachine;

        async fn on_changed(
            self: Box<Self>,
            state_machine: &mut Self::StateMachine,
        ) -> StateResult<Self::StateMachine> {
            if let Err(err) = state_machine.wait_for_atomic_swap_finished().await {
                info!("{LOG_PREFIX} failed: {}", err);
                let next_state = Aborted {
                    reason: AbortReason::SomeReason(err.get_inner().to_string()),
                };
                return Self::change_state(next_state, state_machine).await;
            }
            if let Some(ref lr_swap) = state_machine.lr_swap_1 {
                let run_lr_swap = RunLrSwap1 {
                    lr_swap_params: lr_swap.clone(),
                    src_amount: self.maker_volume,
                };
                return Self::change_state(run_lr_swap, state_machine).await;
            }
            let completed = Completed {};
            Self::change_state(completed, state_machine).await
        }
    }

    impl StorableState for WaitingForAtomicSwap {
        type StateMachine = AggTakerSwapStateMachine;

        fn get_event(&self) -> AggTakerSwapEvent {
            AggTakerSwapEvent::WaitingForAtomicSwap {
                atomic_swap_uuid: self.atomic_swap_uuid,
                maker_volume: self.maker_volume.clone(),
            }
        }
    }

    impl TransitionFrom<StartAtomicSwap> for WaitingForAtomicSwap {}

    /// State to create and send a LR swap tx (before atomic swap)
    pub(super) struct RunLrSwap0 {
        pub lr_swap_params: LrSwapParams,
    }

    #[async_trait]
    impl State for RunLrSwap0 {
        type StateMachine = AggTakerSwapStateMachine;

        async fn on_changed(
            self: Box<Self>,
            state_machine: &mut Self::StateMachine,
        ) -> StateResult<Self::StateMachine> {
            let (base_ticker, tx_bytes, dst_amount) = match state_machine.run_lr_swap(&self.lr_swap_params, None).await
            {
                // No src_amount to be set on LR swap 0
                Ok((base_ticker, tx_bytes, dst_amount)) => (base_ticker, tx_bytes, dst_amount),
                Err(err) => {
                    info!("{LOG_PREFIX} failed: {}", err);
                    let aborted = Aborted {
                        reason: AbortReason::SomeReason(err.get_inner().to_string()),
                    };
                    return Self::change_state(aborted, state_machine).await;
                },
            };
            let next_state = WaitForLrTxConfirmation0 {
                coin: base_ticker,
                tx_bytes,
                dst_amount,
                slippage: self.lr_swap_params.slippage,
            };
            Self::change_state(next_state, state_machine).await
        }
    }

    impl StorableState for RunLrSwap0 {
        type StateMachine = AggTakerSwapStateMachine;

        fn get_event(&self) -> AggTakerSwapEvent { AggTakerSwapEvent::RunLrSwap0 {} }
    }

    impl TransitionFrom<Initialize> for RunLrSwap0 {}

    /// State to wait for confirmation of LR swap tx (before atomic swap)
    pub(super) struct WaitForLrTxConfirmation0 {
        coin: Ticker,
        tx_bytes: BytesJson,
        dst_amount: MmNumber,
        slippage: f32,
    }

    #[async_trait]
    impl State for WaitForLrTxConfirmation0 {
        type StateMachine = AggTakerSwapStateMachine;

        async fn on_changed(
            self: Box<Self>,
            state_machine: &mut Self::StateMachine,
        ) -> StateResult<Self::StateMachine> {
            if let Err(err) = state_machine
                .wait_for_lr_tx_confirmation(&self.coin, self.tx_bytes.clone())
                .await
            {
                info!("{LOG_PREFIX}: liquidity routing tx confirmation failed: {}", err);
                let aborted = Aborted {
                    reason: AbortReason::SomeReason(format!("LR tx confirmation failed: {}", err.get_inner())),
                };
                return Self::change_state(aborted, state_machine).await;
            }

            let atomic_swap_volume = match state_machine.deduct_fees(self.dst_amount.clone()).await {
                Ok(atomic_swap_volume) => atomic_swap_volume,
                Err(err) => {
                    info!("{LOG_PREFIX} failed: {}", err);
                    let aborted = Aborted {
                        reason: AbortReason::SomeReason(err.get_inner().to_string()),
                    };
                    return Self::change_state(aborted, state_machine).await;
                },
            };

            let run_atomic_swap = StartAtomicSwap {
                volume: atomic_swap_volume,
                action: state_machine.atomic_swap.action.clone(),
                slippage: self.slippage,
            };
            Self::change_state(run_atomic_swap, state_machine).await
        }
    }

    impl StorableState for WaitForLrTxConfirmation0 {
        type StateMachine = AggTakerSwapStateMachine;

        fn get_event(&self) -> AggTakerSwapEvent {
            AggTakerSwapEvent::WaitingForLrTxConfirmation0 {
                coin: self.coin.clone(),
                tx_bytes: self.tx_bytes.clone(),
                dst_amount: self.dst_amount.clone(),
                slippage: self.slippage,
            }
        }
    }

    impl TransitionFrom<RunLrSwap0> for WaitForLrTxConfirmation0 {}

    /// State to create and send a LR swap tx (after atomic swap)
    pub(super) struct RunLrSwap1 {
        pub lr_swap_params: LrSwapParams,
        pub src_amount: MmNumber,
    }

    #[async_trait]
    impl State for RunLrSwap1 {
        type StateMachine = AggTakerSwapStateMachine;

        async fn on_changed(
            self: Box<Self>,
            state_machine: &mut Self::StateMachine,
        ) -> StateResult<Self::StateMachine> {
            let (base_ticker, tx_bytes, _) = match state_machine
                .run_lr_swap(&self.lr_swap_params, Some(self.src_amount))
                .await
            {
                // Set LR_swap_1 src amount as the atomic swap maker_volume
                Ok((base_ticker, tx_bytes, dst_amount)) => (base_ticker, tx_bytes, dst_amount),
                Err(err) => {
                    info!("{LOG_PREFIX} failed: {}", err);
                    let aborted = Aborted {
                        reason: AbortReason::SomeReason(err.get_inner().to_string()),
                    };
                    return Self::change_state(aborted, state_machine).await;
                },
            };
            let wait_for_conf = WaitForLrTxConfirmation1 {
                coin: base_ticker,
                tx_bytes,
            };
            Self::change_state(wait_for_conf, state_machine).await
        }
    }

    impl StorableState for RunLrSwap1 {
        type StateMachine = AggTakerSwapStateMachine;

        fn get_event(&self) -> AggTakerSwapEvent { AggTakerSwapEvent::RunLrSwap1 {} }
    }

    impl TransitionFrom<WaitingForAtomicSwap> for RunLrSwap1 {}

    /// State to wait for confirmation of LR swap tx (after atomic swap)
    pub(super) struct WaitForLrTxConfirmation1 {
        coin: Ticker,
        tx_bytes: BytesJson,
    }

    #[async_trait]
    impl State for WaitForLrTxConfirmation1 {
        type StateMachine = AggTakerSwapStateMachine;

        async fn on_changed(
            self: Box<Self>,
            state_machine: &mut Self::StateMachine,
        ) -> StateResult<Self::StateMachine> {
            if let Err(err) = state_machine
                .wait_for_lr_tx_confirmation(&self.coin, self.tx_bytes.clone())
                .await
            {
                info!("{LOG_PREFIX} failed: {}", err);
                let aborted = Aborted {
                    reason: AbortReason::SomeReason(err.get_inner().to_string()),
                };
                return Self::change_state(aborted, state_machine).await;
            }
            let completed = Completed {};
            Self::change_state(completed, state_machine).await
        }
    }

    impl StorableState for WaitForLrTxConfirmation1 {
        type StateMachine = AggTakerSwapStateMachine;

        fn get_event(&self) -> AggTakerSwapEvent {
            AggTakerSwapEvent::WaitingForLrTxConfirmation1 {
                coin: self.coin.clone(),
                tx_bytes: self.tx_bytes.clone(),
            }
        }
    }

    impl TransitionFrom<RunLrSwap1> for WaitForLrTxConfirmation1 {}

    /// Aggregated taker swap with LR completed state
    pub(super) struct Completed {}

    impl StorableState for Completed {
        type StateMachine = AggTakerSwapStateMachine;

        fn get_event(&self) -> AggTakerSwapEvent { AggTakerSwapEvent::Completed }
    }

    #[async_trait]
    impl LastState for Completed {
        type StateMachine = AggTakerSwapStateMachine;

        async fn on_changed(
            self: Box<Self>,
            state_machine: &mut Self::StateMachine,
        ) -> <Self::StateMachine as StateMachineTrait>::Result {
            info!("{LOG_PREFIX} {} has been completed", state_machine.uuid);
        }
    }

    impl TransitionFrom<WaitForLrTxConfirmation1> for Completed {}
    impl TransitionFrom<WaitingForAtomicSwap> for Completed {}

    /// Represents possible reasons of taker swap being aborted
    /// TODO: add reasons
    #[derive(Clone, Debug, Deserialize, Display, Serialize)]
    pub enum AbortReason {
        SomeReason(String),
    }

    /// Aggregated taker swap with LR aborted state
    struct Aborted {
        reason: AbortReason,
    }

    #[async_trait]
    impl LastState for Aborted {
        type StateMachine = AggTakerSwapStateMachine;

        async fn on_changed(
            self: Box<Self>,
            state_machine: &mut Self::StateMachine,
        ) -> <Self::StateMachine as StateMachineTrait>::Result {
            warn!("Swap {} was aborted with reason {}", state_machine.uuid, self.reason);
        }
    }

    impl StorableState for Aborted {
        type StateMachine = AggTakerSwapStateMachine;

        fn get_event(&self) -> AggTakerSwapEvent {
            AggTakerSwapEvent::Aborted {
                reason: self.reason.clone(),
            }
        }
    }

    impl TransitionFrom<Initialize> for Aborted {}
    impl TransitionFrom<RunLrSwap0> for Aborted {}
    impl TransitionFrom<WaitForLrTxConfirmation0> for Aborted {}
    impl TransitionFrom<RunLrSwap1> for Aborted {}
    impl TransitionFrom<WaitForLrTxConfirmation1> for Aborted {}
    impl TransitionFrom<WaitingForAtomicSwap> for Aborted {}
    impl TransitionFrom<StartAtomicSwap> for Aborted {}
}

impl AggTakerSwapStateMachine {
    /// Current agg swap version
    const AGG_SWAP_VERSION: u8 = 0;

    #[allow(clippy::result_large_err)]
    pub(crate) fn source_volume(&self) -> Result<MmNumber, LrSwapError> {
        if let Some(ref lr_swap_0) = self.lr_swap_0 {
            Ok(lr_swap_0.src_amount.clone())
        } else {
            self.atomic_swap.taker_volume()
        }
    }

    pub(crate) fn destination_volume(&self) -> MmNumber {
        Default::default() // TODO: estimate
    }

    /// Source coin or token ticker in aggregated swap (before the first liquidity routing)
    fn source_coin(&self) -> Ticker {
        if let Some(ref lr_swap_0) = self.lr_swap_0 {
            lr_swap_0.src.clone()
        } else {
            self.atomic_swap.taker_coin()
        }
    }

    /// Destination coin or token ticker in aggregated swap (after the final liquidity routing)
    fn destination_coin(&self) -> Ticker {
        if let Some(ref lr_swap_1) = self.lr_swap_1 {
            lr_swap_1.dst.clone()
        } else {
            self.atomic_swap.maker_coin()
        }
    }

    fn atomic_swap_maker_coin(&self) -> Ticker { self.atomic_swap.maker_coin() }

    fn atomic_swap_taker_coin(&self) -> Ticker { self.atomic_swap.taker_coin() }

    pub(crate) fn find_atomic_swap_uuid_in_events(events: &[AggTakerSwapEvent]) -> Option<Uuid> {
        events.iter().find_map(|event| {
            if let AggTakerSwapEvent::WaitingForAtomicSwap { atomic_swap_uuid, .. } = event {
                Some(*atomic_swap_uuid)
            } else {
                None
            }
        })
    }

    fn find_atomic_swap_maker_volume_in_events(events: &[AggTakerSwapEvent]) -> Option<MmNumber> {
        events.iter().find_map(|event| {
            if let AggTakerSwapEvent::WaitingForAtomicSwap { maker_volume, .. } = event {
                Some(maker_volume.clone())
            } else {
                None
            }
        })
    }

    /// Execute liquidity routing interim swap.
    /// The source amount of the LR swap may be set from the previous step by the volume_from_prev_step param.
    async fn run_lr_swap(
        &self,
        lr_swap_params: &LrSwapParams,
        src_amount_from_prev_step: Option<MmNumber>,
    ) -> MmResult<(Ticker, BytesJson, MmNumber), LrSwapError> {
        let (src_coin, _) = lr_helpers::get_coin_for_one_inch(&self.ctx, &lr_swap_params.src).await?;
        let base_chain_id = src_coin.chain_id().ok_or(LrSwapError::ChainNotSupported)?;
        let src_amount = if let Some(src_amount_from_prev_step) = src_amount_from_prev_step {
            src_amount_from_prev_step
        } else {
            lr_swap_params.src_amount.clone()
        };
        info!(
            "{LOG_PREFIX} starting liquidity routing step for {}/{} for amount {}",
            lr_swap_params.src,
            lr_swap_params.dst,
            src_amount.to_decimal()
        );
        let src_amount = u256_from_big_decimal(&src_amount.to_decimal(), lr_swap_params.src_decimals)
            .mm_err(|err| LrSwapError::InvalidParam(err.to_string()))?;

        if let EthCoinType::Erc20 { .. } = src_coin.coin_type {
            let classic_swap_contract =
                EthAddress::from_str(ApiClient::classic_swap_contract()).expect("valid eth address");
            src_coin
                .handle_allowance(
                    classic_swap_contract,
                    src_amount,
                    now_sec() + LR_SWAP_WAIT_CONFIRM_TIMEOUT_SEC,
                ) // TODO: Refactor as std::time::Instant;
                .await?
        }

        let lr_swap_request = lr_helpers::make_classic_swap_create_params(
            lr_swap_params.src_contract,
            lr_swap_params.dst_contract,
            src_amount,
            lr_swap_params.from,
            lr_swap_params.slippage,
            Default::default(),
        );
        let query_params = lr_swap_request.build_query_params().map_mm_err()?;
        let url = SwapUrlBuilder::create_api_url_builder(&self.ctx, base_chain_id, SwapApiMethods::ClassicSwapCreate)
            .map_mm_err()?
            .with_query_params(query_params)
            .build()
            .map_mm_err()?;
        let swap_with_tx: ClassicSwapData = ApiClient::call_api(url).await.map_mm_err()?;
        let tx_fields = swap_with_tx
            .tx
            .ok_or(LrSwapError::InternalError("TxFields empty".to_string()))?;

        let sign_params = SignRawTransactionEnum::ETH(SignEthTransactionParams {
            value: Some(u256_to_big_decimal(U256::from_dec_str(&tx_fields.value)?, src_coin.decimals()).map_mm_err()?),
            to: Some(tx_fields.to.display_address()),
            data: Some(tx_fields.data),
            gas_limit: U256::from(tx_fields.gas),
            pay_for_gas: None, // TODO: enable when the gas policy refactoring PR merged
        });

        // TODO: maybe add another sign and send tx impl in trading_api?
        // actually I use sign_raw_tx instead of eth.rs's sign_and_send_transaction to avoid bringing eth types here
        // TODO: refactor: use SignRawTransactionEnum as the param instead of SignRawTransactionRequest (coin unneeded)
        let tx_bytes = src_coin
            .sign_raw_tx(&SignRawTransactionRequest {
                coin: src_coin.ticker().to_owned(),
                tx: sign_params,
            })
            .await
            .map_mm_err()?;
        let txid = src_coin
            .send_raw_tx_bytes(&tx_bytes.tx_hex)
            .compat()
            .await
            .map_to_mm(LrSwapError::TransactionError)?;
        info!("{LOG_PREFIX}: liquidity routing tx {} sent okay", txid);

        let dst_amount = U256::from_dec_str(&swap_with_tx.dst_amount)?;
        let dst_amount = u256_to_big_decimal(dst_amount, lr_swap_params.dst_decimals).map_mm_err()?;
        Ok((lr_swap_params.src.clone(), tx_bytes.tx_hex, dst_amount.into()))
    }

    /// Start nested atomic swap by calling lp_auto_buy. The actual volume is determined on previous states
    /// The volume is always in the taker sell coins.
    /// TODO: add slippage support
    async fn start_lp_auto_buy(&self, volume: MmNumber, _slippage: f32) -> MmResult<SellBuyResponse, LrSwapError> {
        const TAKER_ACTION: TakerAction = TakerAction::Sell;

        let base_coin = lp_coinfind_or_err(&self.ctx, &self.atomic_swap.base)
            .await
            .map_mm_err()?;
        let rel_coin = lp_coinfind_or_err(&self.ctx, &self.atomic_swap.rel)
            .await
            .map_mm_err()?;
        if base_coin.wallet_only(&self.ctx) {
            return MmError::err(LrSwapError::InvalidParam(format!(
                "Base coin {} is wallet only",
                self.atomic_swap.base
            )));
        }
        if rel_coin.wallet_only(&self.ctx) {
            return MmError::err(LrSwapError::InvalidParam(format!(
                "Rel coin {} is wallet only",
                self.atomic_swap.rel
            )));
        }
        debug!(
            "{LOG_PREFIX} volume={} self.atomic_swap.price={} action={:?} base={} rel={}",
            volume.to_decimal(),
            self.atomic_swap.price.to_decimal(),
            TAKER_ACTION,
            self.atomic_swap.base,
            self.atomic_swap.rel
        );
        let (taker_amount, sell_base, sell_rel) = match TAKER_ACTION {
            TakerAction::Buy => (&volume * &self.atomic_swap.price, rel_coin.clone(), base_coin.clone()),
            TakerAction::Sell => (volume.clone(), base_coin.clone(), rel_coin.clone()),
        };

        debug!(
            "{LOG_PREFIX} checking taker balance for atomic swap for {}/{} taker_amount: {}",
            sell_base.ticker(),
            sell_rel.ticker(),
            taker_amount.to_decimal(),
        );
        check_balance_for_taker_swap(
            &self.ctx,
            sell_base.deref(),
            sell_rel.deref(),
            taker_amount,
            None,
            None,
            FeeApproxStage::OrderIssue,
        )
        .await
        .map_mm_err()?;
        info!(
            "{LOG_PREFIX} starting atomic swap for {}/{}, action {:?}, amount {}",
            self.atomic_swap.base,
            self.atomic_swap.rel,
            TAKER_ACTION,
            volume.to_decimal()
        );
        let sell_buy_req = lr_helpers::make_atomic_swap_request(
            self.atomic_swap.base.clone(),
            self.atomic_swap.rel.clone(),
            self.atomic_swap.price.clone(),
            volume,
            TAKER_ACTION,
            self.atomic_swap.match_by.clone(),
            self.atomic_swap.order_type.clone(),
        );
        let res_bytes = lp_auto_buy(&self.ctx, &base_coin, &rel_coin, sell_buy_req)
            .await
            .map_to_mm(LrSwapError::InternalError)?;
        let rpc_res: Mm2RpcResult<SellBuyResponse> = serde_json::from_slice(res_bytes.as_slice())?;
        Ok(rpc_res.result)
    }

    #[allow(clippy::result_large_err)]
    fn check_atomic_swap_status(swap_result: &MmResult<SwapRpcData, MySwapStatusError>) -> MmResult<bool, LrSwapError> {
        let swap_status = match swap_result {
            Ok(swap_status) => swap_status,
            Err(mm_err) => {
                match mm_err.get_inner() {
                    // TODO: now considering that swap has not been started yet and we don't have non-existing uuids,
                    // but maybe we could throw an error after some time
                    MySwapStatusError::NoSwapWithUuid(_) => return Ok(false),
                    other_err => {
                        return MmError::err(LrSwapError::InternalError(format!(
                            "Failed to get swap status: {}",
                            other_err
                        )))
                    },
                }
            },
        };
        match swap_status {
            SwapRpcData::TakerV1(swap_status) => {
                if !swap_status.is_finished() {
                    return Ok(false);
                }
                match swap_status.is_success() {
                    Ok(true) => Ok(true),
                    Ok(false) | Err(_) => MmError::err(LrSwapError::AtomicSwapError("Atomic swap failed".to_string())),
                }
            },
            SwapRpcData::TakerV2(swap_status) => {
                if let Some(reason) = swap_status.is_aborted() {
                    return MmError::err(LrSwapError::AtomicSwapError(reason.to_string()));
                }
                Ok(swap_status.is_completed())
            },
            SwapRpcData::AggTaker(_) | SwapRpcData::MakerV1(_) | SwapRpcData::MakerV2(_) => {
                MmError::err(LrSwapError::InternalError("incorrect atomic swap type".to_string()))
            },
        }
    }

    async fn wait_for_atomic_swap_finished(&self) -> MmResult<(), LrSwapError> {
        let atomic_swap_uuid = self
            .atomic_swap_uuid
            .get()
            .ok_or(LrSwapError::InternalError("atomic swap uuid not set".to_string()))?;
        info!(
            "{LOG_PREFIX} {}: waiting for interim atomic swap uuid {} to finish",
            self.uuid, atomic_swap_uuid
        );
        // We do not have any time limits for waiting the atomic swap to finish,
        // assuming the atomic swap code ensures it won't hang.
        loop {
            let swap_result = my_swap_status_rpc(self.ctx.clone(), MySwapStatusRequest {
                uuid: *atomic_swap_uuid,
            })
            .await;
            if Self::check_atomic_swap_status(&swap_result)? {
                break;
            }
            Timer::sleep(5.).await;
        }
        info!(
            "{LOG_PREFIX} {}: interim atomic swap uuid {} finished",
            self.uuid, atomic_swap_uuid
        );
        Ok(())
    }

    async fn wait_for_lr_tx_confirmation(&self, coin: &Ticker, tx_bytes: BytesJson) -> MmResult<(), LrSwapError> {
        match lp_coinfind_or_err(&self.ctx, coin).await.map_mm_err()? {
            MmCoinEnum::EthCoin(eth_coin) => {
                info!("{LOG_PREFIX}: waiting for liquidity routing tx to confirm");
                let confirm_lr_swap_input = ConfirmPaymentInput {
                    payment_tx: tx_bytes.0,
                    confirmations: LR_SWAP_CONFIRMATIONS,
                    requires_nota: false,
                    wait_until: now_sec() + LR_SWAP_WAIT_CONFIRM_TIMEOUT_SEC,
                    check_every: LR_SWAP_WAIT_CONFIRM_INTERVAL_SEC,
                };
                let _ = eth_coin
                    .wait_for_confirmations(confirm_lr_swap_input)
                    .compat()
                    .await
                    .map_to_mm(LrSwapError::TransactionError)?;
                info!("{LOG_PREFIX}: liquidity routing tx confirmed");
                Ok(())
            },
            _ => MmError::err(LrSwapError::CoinTypeError),
        }
    }

    /// Deduct fees from the input volume (which is the LR swap destination amount)
    /// and convert it to the atomic swap buy or sell volume
    async fn deduct_fees(&self, volume_with_fees: MmNumber) -> MmResult<MmNumber, LrSwapError> {
        let taker_ticker = self.atomic_swap.taker_coin();
        let maker_ticker = self.atomic_swap.maker_coin();
        // Try to estimate the original taker volume. This works for EVM as there no min tx amount
        let dex_fee_rate = DexFee::dex_fee_rate(&taker_ticker, &maker_ticker);
        // Recalculate swap_volume with the tarde fees removed:
        let taker_volume = volume_with_fees.clone() / (MmNumber::from("1") + dex_fee_rate);
        debug!(
            "{maker_ticker}/{taker_ticker} deduct_fees volume_with_fees={volume_with_fees} taker_volume={taker_volume} dex_fee_diff={}",
            &volume_with_fees - &taker_volume
        );

        let swap_volume = match self.atomic_swap.action {
            TakerAction::Buy => taker_volume
                .checked_div(&self.atomic_swap.price) // make the amount how it was in the buy order
                .ok_or(MmError::new(LrSwapError::ConversionError(
                    "Could not calculate swap amount".to_string(),
                )))?,
            TakerAction::Sell => taker_volume,
        };
        Ok(swap_volume)
    }
}

async fn agg_taker_swap_state_machine_runner(
    ctx: &MmArc,
    lr_swap_0: Option<LrSwapParams>,
    lr_swap_1: Option<LrSwapParams>,
    atomic_swap: AtomicSwapParams,
    uuid: Uuid,
) {
    let mut state_machine = AggTakerSwapStateMachine {
        storage: AggTakerSwapStorage::new(ctx.clone()),
        abortable_system: ctx
            .abortable_system
            .create_subsystem()
            .expect("create_subsystem should not fail"),
        ctx: ctx.clone(),
        lr_swap_0,
        lr_swap_1,
        atomic_swap,
        atomic_swap_maker_volume: OnceLock::new(),
        uuid,
        started_at: now_sec(),
        swap_version: AggTakerSwapStateMachine::AGG_SWAP_VERSION,
        atomic_swap_uuid: OnceLock::default(),
    };
    #[allow(clippy::box_default)]
    state_machine
        .run(Box::new(states::Initialize::default()))
        .await
        .error_log();
}

pub(crate) async fn lp_start_agg_taker_swap(
    ctx: MmArc,
    lr_swap_0: Option<LrSwapParams>,
    lr_swap_1: Option<LrSwapParams>,
    atomic_swap: AtomicSwapParams,
) -> MmResult<Uuid, LrSwapError> {
    let spawner = ctx.spawner();
    let uuid = new_uuid(); // For a aggregated swap we need a new uuid, different from the atomic swap uuid, to distinguish the aggregated swap as dedicated in rpcs, statuses etc

    let fut = async move {
        agg_taker_swap_state_machine_runner(&ctx, lr_swap_0, lr_swap_1, atomic_swap, uuid).await;
    };

    let settings = AbortSettings::info_on_abort(format!("swap {uuid} stopped!"));
    spawner.spawn_with_settings(fut, settings);
    Ok(uuid)
}
