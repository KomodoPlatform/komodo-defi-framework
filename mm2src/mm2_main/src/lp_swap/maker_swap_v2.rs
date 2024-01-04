use super::swap_v2_common::*;
use super::{swap_v2_topic, NEGOTIATE_SEND_INTERVAL, NEGOTIATION_TIMEOUT_SEC};
use crate::mm2::lp_swap::swap_lock::SwapLock;
use crate::mm2::lp_swap::swap_v2_pb::*;
use crate::mm2::lp_swap::{broadcast_swap_v2_msg_every, check_balance_for_maker_swap, recv_swap_v2_msg, SecretHashAlgo,
                          SwapConfirmationsSettings, TransactionIdentifier, MAKER_SWAP_V2_TYPE, MAX_STARTED_AT_DIFF};
use async_trait::async_trait;
use bitcrypto::{dhash160, sha256};
use coins::{CanRefundHtlc, CoinAssocTypes, ConfirmPaymentInput, FeeApproxStage, GenTakerFundingSpendArgs,
            GenTakerPaymentSpendArgs, MmCoin, RefundPaymentArgs, SendPaymentArgs, SwapOpsV2, ToBytes, Transaction,
            TxPreimageWithSig, ValidateTakerFundingArgs};
use common::executor::abortable_queue::AbortableQueue;
use common::executor::{AbortableSystem, Timer};
use common::log::{debug, error, info, warn};
use common::{Future01CompatExt, DEX_FEE_ADDR_RAW_PUBKEY};
use crypto::privkey::SerializableSecp256k1Keypair;
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::MmNumber;
use mm2_state_machine::prelude::*;
use mm2_state_machine::storable_state_machine::*;
use primitives::hash::H256;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use std::convert::TryInto;
use std::marker::PhantomData;
use uuid::Uuid;

cfg_native!(
    use crate::mm2::database::my_swaps::{insert_new_swap_v2, SELECT_MY_SWAP_V2_BY_UUID};
    use common::async_blocking;
    use db_common::sqlite::rusqlite::{named_params, Error as SqlError, Result as SqlResult, Row};
    use db_common::sqlite::rusqlite::types::Type as SqlType;
);

cfg_wasm32!(
    use crate::mm2::lp_swap::SwapsContext;
    use crate::mm2::lp_swap::swap_wasm_db::{MySwapsFiltersTable, SavedSwapTable};
);

// This is needed to have Debug on messages
#[allow(unused_imports)] use prost::Message;

/// Negotiation data representation to be stored in DB.
#[derive(Debug, Deserialize, Serialize)]
pub struct StoredNegotiationData {
    taker_payment_locktime: u64,
    taker_funding_locktime: u64,
    maker_coin_htlc_pub_from_taker: BytesJson,
    taker_coin_htlc_pub_from_taker: BytesJson,
    maker_coin_swap_contract: Option<BytesJson>,
    taker_coin_swap_contract: Option<BytesJson>,
    taker_secret_hash: BytesJson,
}

/// Represents events produced by maker swap states.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "event_type", content = "event_data")]
pub enum MakerSwapEvent {
    /// Swap has been successfully initialized.
    Initialized {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
    },
    /// Started waiting for taker funding tx.
    WaitingForTakerFunding {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        negotiation_data: StoredNegotiationData,
    },
    /// Received taker funding info.
    TakerFundingReceived {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        negotiation_data: StoredNegotiationData,
        taker_funding: TransactionIdentifier,
    },
    /// Sent maker payment and generated funding spend preimage.
    MakerPaymentSentFundingSpendGenerated {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        negotiation_data: StoredNegotiationData,
        maker_payment: TransactionIdentifier,
        funding_spend_preimage: StoredTxPreimage,
    },
    /// Something went wrong, so maker payment refund is required.
    MakerPaymentRefundRequired {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        negotiation_data: StoredNegotiationData,
        maker_payment: TransactionIdentifier,
        reason: MakerPaymentRefundReason,
    },
    /// Maker payment has been refunded
    MakerPaymentRefunded {
        maker_payment: TransactionIdentifier,
        maker_payment_refund: TransactionIdentifier,
        reason: MakerPaymentRefundReason,
    },
    /// Taker payment has been confirmed on-chain.
    TakerPaymentConfirmed {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        negotiation_data: StoredNegotiationData,
        maker_payment: TransactionIdentifier,
        taker_payment: TransactionIdentifier,
    },
    /// Maker successfully spent taker's payment.
    TakerPaymentSpent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        maker_payment: TransactionIdentifier,
        taker_payment: TransactionIdentifier,
        taker_payment_spend: TransactionIdentifier,
    },
    /// Swap has been aborted before maker payment was sent.
    Aborted { reason: AbortReason },
    /// Swap completed successfully.
    Completed,
}

/// Storage for maker swaps.
#[derive(Clone)]
pub struct MakerSwapStorage {
    ctx: MmArc,
}

impl MakerSwapStorage {
    pub fn new(ctx: MmArc) -> Self { MakerSwapStorage { ctx } }
}

#[async_trait]
impl StateMachineStorage for MakerSwapStorage {
    type MachineId = Uuid;
    type DbRepr = MakerSwapDbRepr;
    type Error = MmError<SwapStateMachineError>;

    #[cfg(not(target_arch = "wasm32"))]
    async fn store_repr(&mut self, _id: Self::MachineId, repr: Self::DbRepr) -> Result<(), Self::Error> {
        let ctx = self.ctx.clone();

        async_blocking(move || {
            let sql_params = named_params! {
                ":my_coin": &repr.maker_coin,
                ":other_coin": &repr.taker_coin,
                ":uuid": repr.uuid.to_string(),
                ":started_at": repr.started_at,
                ":swap_type": MAKER_SWAP_V2_TYPE,
                ":maker_volume": repr.maker_volume.to_fraction_string(),
                ":taker_volume": repr.taker_volume.to_fraction_string(),
                ":premium": repr.taker_premium.to_fraction_string(),
                ":dex_fee": repr.dex_fee_amount.to_fraction_string(),
                ":secret": repr.maker_secret.0,
                ":secret_hash": repr.maker_secret_hash.0,
                ":secret_hash_algo": repr.secret_hash_algo as u8,
                ":p2p_privkey": repr.p2p_keypair.map(|k| k.priv_key()).unwrap_or_default(),
                ":lock_duration": repr.lock_duration,
                ":maker_coin_confs": repr.conf_settings.maker_coin_confs,
                ":maker_coin_nota": repr.conf_settings.maker_coin_nota,
                ":taker_coin_confs": repr.conf_settings.taker_coin_confs,
                ":taker_coin_nota": repr.conf_settings.taker_coin_nota
            };
            insert_new_swap_v2(&ctx, sql_params)?;
            Ok(())
        })
        .await
    }

    #[cfg(target_arch = "wasm32")]
    async fn store_repr(&mut self, uuid: Self::MachineId, repr: Self::DbRepr) -> Result<(), Self::Error> {
        let swaps_ctx = SwapsContext::from_ctx(&self.ctx).expect("SwapsContext::from_ctx should not fail");
        let db = swaps_ctx.swap_db().await?;
        let transaction = db.transaction().await?;

        let filters_table = transaction.table::<MySwapsFiltersTable>().await?;

        let item = MySwapsFiltersTable {
            uuid,
            my_coin: repr.maker_coin.clone(),
            other_coin: repr.taker_coin.clone(),
            started_at: repr.started_at as u32,
            is_finished: false.into(),
            swap_type: MAKER_SWAP_V2_TYPE,
        };
        filters_table.add_item(&item).await?;

        let table = transaction.table::<SavedSwapTable>().await?;
        let item = SavedSwapTable {
            uuid,
            saved_swap: serde_json::to_value(repr)?,
        };
        table.add_item(&item).await?;
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    async fn get_repr(&self, id: Self::MachineId) -> Result<Self::DbRepr, Self::Error> {
        let ctx = self.ctx.clone();
        let id_str = id.to_string();

        async_blocking(move || {
            Ok(ctx.sqlite_connection().query_row(
                SELECT_MY_SWAP_V2_BY_UUID,
                &[(":uuid", &id_str)],
                MakerSwapDbRepr::from_sql_row,
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

    async fn store_event(&mut self, id: Self::MachineId, event: MakerSwapEvent) -> Result<(), Self::Error> {
        store_swap_event::<MakerSwapDbRepr>(self.ctx.clone(), id, event).await
    }

    async fn get_unfinished(&self) -> Result<Vec<Self::MachineId>, Self::Error> {
        get_unfinished_swaps_uuids(self.ctx.clone(), MAKER_SWAP_V2_TYPE).await
    }

    async fn mark_finished(&mut self, id: Self::MachineId) -> Result<(), Self::Error> {
        mark_swap_as_finished(self.ctx.clone(), id).await
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MakerSwapDbRepr {
    /// Maker coin
    pub maker_coin: String,
    /// The amount swapped by maker.
    pub maker_volume: MmNumber,
    /// The secret used in HTLC hashlock.
    pub maker_secret: H256Json,
    /// The secret's hash in HTLC hashlock.
    pub maker_secret_hash: BytesJson,
    /// Algorithm used to hash the swap secret.
    pub secret_hash_algo: SecretHashAlgo,
    /// The timestamp when the swap was started.
    pub started_at: u64,
    /// The duration of HTLC timelock in seconds.
    pub lock_duration: u64,
    /// Taker coin
    pub taker_coin: String,
    /// The amount swapped by taker.
    pub taker_volume: MmNumber,
    /// Premium amount, which might be paid to maker as additional reward.
    pub taker_premium: MmNumber,
    /// DEX fee amount
    pub dex_fee_amount: MmNumber,
    /// Swap transactions' confirmations settings
    pub conf_settings: SwapConfirmationsSettings,
    /// UUID of the swap
    pub uuid: Uuid,
    /// If Some, used to sign P2P messages of this swap.
    pub p2p_keypair: Option<SerializableSecp256k1Keypair>,
    /// Swap events
    pub events: Vec<MakerSwapEvent>,
}

impl StateMachineDbRepr for MakerSwapDbRepr {
    type Event = MakerSwapEvent;

    fn add_event(&mut self, event: Self::Event) { self.events.push(event) }
}

impl GetSwapCoins for MakerSwapDbRepr {
    fn maker_coin(&self) -> &str { &self.maker_coin }

    fn taker_coin(&self) -> &str { &self.taker_coin }
}

#[cfg(not(target_arch = "wasm32"))]
impl MakerSwapDbRepr {
    fn from_sql_row(row: &Row) -> SqlResult<Self> {
        Ok(MakerSwapDbRepr {
            maker_coin: row.get(0)?,
            taker_coin: row.get(1)?,
            uuid: row
                .get::<_, String>(2)?
                .parse()
                .map_err(|e| SqlError::FromSqlConversionFailure(2, SqlType::Text, Box::new(e)))?,
            started_at: row.get(3)?,
            maker_secret: row.get::<_, [u8; 32]>(4)?.into(),
            maker_secret_hash: row.get::<_, Vec<u8>>(5)?.into(),
            secret_hash_algo: row
                .get::<_, u8>(6)?
                .try_into()
                .map_err(|e| SqlError::FromSqlConversionFailure(6, SqlType::Integer, Box::new(e)))?,
            events: serde_json::from_str(&row.get::<_, String>(7)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(7, SqlType::Text, Box::new(e)))?,
            maker_volume: MmNumber::from_fraction_string(&row.get::<_, String>(8)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(8, SqlType::Text, Box::new(e)))?,
            taker_volume: MmNumber::from_fraction_string(&row.get::<_, String>(9)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(9, SqlType::Text, Box::new(e)))?,
            taker_premium: MmNumber::from_fraction_string(&row.get::<_, String>(10)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(10, SqlType::Text, Box::new(e)))?,
            dex_fee_amount: MmNumber::from_fraction_string(&row.get::<_, String>(11)?)
                .map_err(|e| SqlError::FromSqlConversionFailure(11, SqlType::Text, Box::new(e)))?,
            lock_duration: row.get(12)?,
            conf_settings: SwapConfirmationsSettings {
                maker_coin_confs: row.get(13)?,
                maker_coin_nota: row.get(14)?,
                taker_coin_confs: row.get(15)?,
                taker_coin_nota: row.get(16)?,
            },
            p2p_keypair: row.get::<_, [u8; 32]>(17).and_then(|maybe_key| {
                if maybe_key == [0; 32] {
                    Ok(None)
                } else {
                    Ok(Some(SerializableSecp256k1Keypair::new(maybe_key).map_err(|e| {
                        SqlError::FromSqlConversionFailure(17, SqlType::Blob, Box::new(e))
                    })?))
                }
            })?,
        })
    }
}

/// Represents the state machine for maker's side of the Trading Protocol Upgrade swap (v2).
pub struct MakerSwapStateMachine<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> {
    /// MM2 context
    pub ctx: MmArc,
    /// Storage
    pub storage: MakerSwapStorage,
    /// Maker coin
    pub maker_coin: MakerCoin,
    /// The amount swapped by maker.
    pub maker_volume: MmNumber,
    /// The secret used in HTLC hashlock.
    pub secret: H256,
    /// Algorithm used to hash the swap secret.
    pub secret_hash_algo: SecretHashAlgo,
    /// The timestamp when the swap was started.
    pub started_at: u64,
    /// The duration of HTLC timelock in seconds.
    pub lock_duration: u64,
    /// Taker coin
    pub taker_coin: TakerCoin,
    /// The amount swapped by taker.
    pub taker_volume: MmNumber,
    /// Premium amount, which might be paid to maker as additional reward.
    pub taker_premium: MmNumber,
    /// DEX fee amount
    pub dex_fee_amount: MmNumber,
    /// Swap transactions' confirmations settings
    pub conf_settings: SwapConfirmationsSettings,
    /// UUID of the swap
    pub uuid: Uuid,
    /// The gossipsub topic used for peer-to-peer communication in swap process.
    pub p2p_topic: String,
    /// If Some, used to sign P2P messages of this swap.
    pub p2p_keypair: Option<KeyPair>,
    /// Abortable queue used to spawn related activities
    pub abortable_system: AbortableQueue,
}

impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> MakerSwapStateMachine<MakerCoin, TakerCoin> {
    /// Timeout for taker payment's on-chain confirmation.
    #[inline]
    fn taker_payment_conf_timeout(&self) -> u64 { self.started_at + self.lock_duration * 2 / 3 }

    /// Returns timestamp of maker payment's locktime.
    #[inline]
    fn maker_payment_locktime(&self) -> u64 { self.started_at + 2 * self.lock_duration }

    /// Returns secret hash generated using selected [SecretHashAlgo].
    fn secret_hash(&self) -> Vec<u8> {
        match self.secret_hash_algo {
            SecretHashAlgo::DHASH160 => dhash160(self.secret.as_slice()).take().into(),
            SecretHashAlgo::SHA256 => sha256(self.secret.as_slice()).take().into(),
        }
    }

    /// Returns data that is unique for this swap.
    #[inline]
    fn unique_data(&self) -> Vec<u8> { self.secret_hash() }
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> StorableStateMachine
    for MakerSwapStateMachine<MakerCoin, TakerCoin>
{
    type Storage = MakerSwapStorage;
    type Result = ();
    type Error = MmError<SwapStateMachineError>;
    type ReentrancyLock = SwapLock;
    type RecreateCtx = SwapRecreateCtx<MakerCoin, TakerCoin>;
    type RecreateError = MmError<SwapRecreateError>;

    fn to_db_repr(&self) -> MakerSwapDbRepr {
        MakerSwapDbRepr {
            maker_coin: self.maker_coin.ticker().into(),
            maker_volume: self.maker_volume.clone(),
            maker_secret: self.secret.into(),
            maker_secret_hash: self.secret_hash().into(),
            secret_hash_algo: self.secret_hash_algo,
            started_at: self.started_at,
            lock_duration: self.lock_duration,
            taker_coin: self.taker_coin.ticker().into(),
            taker_volume: self.taker_volume.clone(),
            taker_premium: self.taker_premium.clone(),
            dex_fee_amount: self.dex_fee_amount.clone(),
            conf_settings: self.conf_settings,
            uuid: self.uuid,
            p2p_keypair: self.p2p_keypair.map(Into::into),
            events: Vec::new(),
        }
    }

    fn storage(&mut self) -> &mut Self::Storage { &mut self.storage }

    fn id(&self) -> <Self::Storage as StateMachineStorage>::MachineId { self.uuid }

    async fn recreate_machine(
        uuid: Uuid,
        storage: MakerSwapStorage,
        mut repr: MakerSwapDbRepr,
        recreate_ctx: Self::RecreateCtx,
    ) -> Result<RestoredMachine<Self>, Self::RecreateError> {
        if repr.events.is_empty() {
            return MmError::err(SwapRecreateError::ReprEventsEmpty);
        }

        let current_state: Box<dyn State<StateMachine = Self>> = match repr.events.remove(repr.events.len() - 1) {
            MakerSwapEvent::Initialized {
                maker_coin_start_block,
                taker_coin_start_block,
            } => Box::new(Initialized {
                maker_coin: Default::default(),
                taker_coin: Default::default(),
                maker_coin_start_block,
                taker_coin_start_block,
            }),
            MakerSwapEvent::WaitingForTakerFunding {
                maker_coin_start_block,
                taker_coin_start_block,
                negotiation_data,
            } => Box::new(WaitingForTakerFunding {
                maker_coin_start_block,
                taker_coin_start_block,
                negotiation_data: NegotiationData::from_stored_data(
                    negotiation_data,
                    &recreate_ctx.maker_coin,
                    &recreate_ctx.taker_coin,
                )?,
            }),
            MakerSwapEvent::TakerFundingReceived {
                maker_coin_start_block,
                taker_coin_start_block,
                negotiation_data,
                taker_funding,
            } => Box::new(TakerFundingReceived {
                maker_coin_start_block,
                taker_coin_start_block,
                negotiation_data: NegotiationData::from_stored_data(
                    negotiation_data,
                    &recreate_ctx.maker_coin,
                    &recreate_ctx.taker_coin,
                )?,
                taker_funding: recreate_ctx
                    .taker_coin
                    .parse_tx(&taker_funding.tx_hex.0)
                    .map_err(|e| SwapRecreateError::FailedToParseData(e.to_string()))?,
            }),
            MakerSwapEvent::MakerPaymentSentFundingSpendGenerated {
                maker_coin_start_block,
                taker_coin_start_block,
                negotiation_data,
                maker_payment,
                funding_spend_preimage,
            } => Box::new(MakerPaymentSentFundingSpendGenerated {
                maker_coin_start_block,
                taker_coin_start_block,
                negotiation_data: NegotiationData::from_stored_data(
                    negotiation_data,
                    &recreate_ctx.maker_coin,
                    &recreate_ctx.taker_coin,
                )?,
                funding_spend_preimage: TxPreimageWithSig {
                    preimage: recreate_ctx
                        .taker_coin
                        .parse_preimage(&funding_spend_preimage.preimage.0)
                        .map_err(|e| SwapRecreateError::FailedToParseData(e.to_string()))?,
                    signature: recreate_ctx
                        .taker_coin
                        .parse_signature(&funding_spend_preimage.signature.0)
                        .map_err(|e| SwapRecreateError::FailedToParseData(e.to_string()))?,
                },
                maker_payment,
            }),
            MakerSwapEvent::MakerPaymentRefundRequired {
                maker_coin_start_block,
                taker_coin_start_block,
                negotiation_data,
                maker_payment,
                reason,
            } => Box::new(MakerPaymentRefundRequired {
                maker_coin_start_block,
                taker_coin_start_block,
                negotiation_data: NegotiationData::from_stored_data(
                    negotiation_data,
                    &recreate_ctx.maker_coin,
                    &recreate_ctx.taker_coin,
                )?,
                maker_payment,
                reason,
            }),
            MakerSwapEvent::TakerPaymentConfirmed {
                maker_coin_start_block,
                taker_coin_start_block,
                negotiation_data,
                maker_payment,
                taker_payment,
            } => Box::new(TakerPaymentReceived {
                maker_coin_start_block,
                taker_coin_start_block,
                maker_payment,
                taker_payment: recreate_ctx
                    .taker_coin
                    .parse_tx(&taker_payment.tx_hex.0)
                    .map_err(|e| SwapRecreateError::FailedToParseData(e.to_string()))?,
                negotiation_data: NegotiationData::from_stored_data(
                    negotiation_data,
                    &recreate_ctx.maker_coin,
                    &recreate_ctx.taker_coin,
                )?,
            }),
            MakerSwapEvent::TakerPaymentSpent {
                maker_coin_start_block,
                taker_coin_start_block,
                maker_payment,
                taker_payment,
                taker_payment_spend,
            } => Box::new(TakerPaymentSpent {
                maker_coin: Default::default(),
                maker_coin_start_block,
                taker_coin_start_block,
                maker_payment,
                taker_payment: recreate_ctx
                    .taker_coin
                    .parse_tx(&taker_payment.tx_hex.0)
                    .map_err(|e| SwapRecreateError::FailedToParseData(e.to_string()))?,
                taker_payment_spend,
            }),
            MakerSwapEvent::Aborted { .. } => return MmError::err(SwapRecreateError::SwapAborted),
            MakerSwapEvent::Completed => return MmError::err(SwapRecreateError::SwapCompleted),
            MakerSwapEvent::MakerPaymentRefunded { .. } => {
                return MmError::err(SwapRecreateError::SwapFinishedWithRefund)
            },
        };

        let machine = MakerSwapStateMachine {
            ctx: storage.ctx.clone(),
            abortable_system: storage
                .ctx
                .abortable_system
                .create_subsystem()
                .expect("create_subsystem should not fail"),
            storage,
            maker_coin: recreate_ctx.maker_coin,
            maker_volume: repr.maker_volume,
            secret: repr.maker_secret.into(),
            secret_hash_algo: repr.secret_hash_algo,
            started_at: repr.started_at,
            lock_duration: repr.lock_duration,
            taker_coin: recreate_ctx.taker_coin,
            taker_volume: repr.taker_volume,
            taker_premium: repr.taker_premium,
            dex_fee_amount: repr.dex_fee_amount,
            conf_settings: repr.conf_settings,
            p2p_topic: swap_v2_topic(&uuid),
            uuid,
            p2p_keypair: repr.p2p_keypair.map(|k| k.into_inner()),
        };

        Ok(RestoredMachine { machine, current_state })
    }

    fn init_additional_context(&mut self) {
        init_additional_context_impl(&self.ctx, ActiveSwapV2Info {
            uuid: self.uuid,
            maker_coin: self.maker_coin.ticker().into(),
            taker_coin: self.taker_coin.ticker().into(),
        })
    }

    async fn acquire_reentrancy_lock(&self) -> Result<Self::ReentrancyLock, Self::Error> {
        acquire_reentrancy_lock_impl(&self.ctx, self.uuid).await
    }

    fn spawn_reentrancy_lock_renew(&mut self, guard: Self::ReentrancyLock) {
        spawn_reentrancy_lock_renew_impl(&self.abortable_system, self.uuid, guard)
    }

    fn clean_up_context(&mut self) { clean_up_context_impl(&self.ctx, &self.uuid) }
}

/// Represents a state used to start a new maker swap.
pub struct Initialize<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
}

impl<MakerCoin, TakerCoin> Default for Initialize<MakerCoin, TakerCoin> {
    fn default() -> Self {
        Initialize {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
        }
    }
}

impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> InitialState
    for Initialize<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State for Initialize<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let maker_coin_start_block = match state_machine.maker_coin.current_block().compat().await {
            Ok(b) => b,
            Err(e) => {
                let reason = AbortReason::FailedToGetMakerCoinBlock(e);
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };

        let taker_coin_start_block = match state_machine.taker_coin.current_block().compat().await {
            Ok(b) => b,
            Err(e) => {
                let reason = AbortReason::FailedToGetTakerCoinBlock(e);
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };

        if let Err(e) = check_balance_for_maker_swap(
            &state_machine.ctx,
            &state_machine.maker_coin,
            &state_machine.taker_coin,
            state_machine.maker_volume.clone(),
            Some(&state_machine.uuid),
            None,
            FeeApproxStage::StartSwap,
        )
        .await
        {
            let reason = AbortReason::BalanceCheckFailure(e.to_string());
            return Self::change_state(Aborted::new(reason), state_machine).await;
        }

        info!("Maker swap {} has successfully started", state_machine.uuid);
        let negotiate = Initialized {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block,
            taker_coin_start_block,
        };
        Self::change_state(negotiate, state_machine).await
    }
}

struct Initialized<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
}

impl<MakerCoin, TakerCoin> TransitionFrom<Initialize<MakerCoin, TakerCoin>> for Initialized<MakerCoin, TakerCoin> {}

impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> StorableState
    for Initialized<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> MakerSwapEvent {
        MakerSwapEvent::Initialized {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
        }
    }
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State for Initialized<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let unique_data = state_machine.unique_data();

        let maker_negotiation_msg = MakerNegotiation {
            started_at: state_machine.started_at,
            payment_locktime: state_machine.maker_payment_locktime(),
            secret_hash: state_machine.secret_hash(),
            maker_coin_htlc_pub: state_machine.maker_coin.derive_htlc_pubkey(&unique_data),
            taker_coin_htlc_pub: state_machine.taker_coin.derive_htlc_pubkey(&unique_data),
            maker_coin_swap_contract: state_machine.maker_coin.swap_contract_address().map(|bytes| bytes.0),
            taker_coin_swap_contract: state_machine.taker_coin.swap_contract_address().map(|bytes| bytes.0),
        };
        debug!("Sending maker negotiation message {:?}", maker_negotiation_msg);
        let swap_msg = SwapMessage {
            inner: Some(swap_message::Inner::MakerNegotiation(maker_negotiation_msg)),
        };
        let abort_handle = broadcast_swap_v2_msg_every(
            state_machine.ctx.clone(),
            state_machine.p2p_topic.clone(),
            swap_msg,
            NEGOTIATE_SEND_INTERVAL,
            state_machine.p2p_keypair,
        );

        let recv_fut = recv_swap_v2_msg(
            state_machine.ctx.clone(),
            |store| store.taker_negotiation.take(),
            &state_machine.uuid,
            NEGOTIATION_TIMEOUT_SEC,
        );
        let taker_negotiation = match recv_fut.await {
            Ok(d) => d,
            Err(e) => {
                let reason = AbortReason::DidNotReceiveTakerNegotiation(e);
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };
        drop(abort_handle);

        debug!("Received taker negotiation message {:?}", taker_negotiation);
        let taker_data = match taker_negotiation.action {
            Some(taker_negotiation::Action::Continue(data)) => data,
            Some(taker_negotiation::Action::Abort(abort)) => {
                let reason = AbortReason::TakerAbortedNegotiation(abort.reason);
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
            None => {
                let reason = AbortReason::ReceivedInvalidTakerNegotiation;
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };

        let started_at_diff = state_machine.started_at.abs_diff(taker_data.started_at);
        if started_at_diff > MAX_STARTED_AT_DIFF {
            let reason = AbortReason::TooLargeStartedAtDiff(started_at_diff);
            return Self::change_state(Aborted::new(reason), state_machine).await;
        }

        let expected_taker_funding_locktime = taker_data.started_at + 3 * state_machine.lock_duration;
        if taker_data.funding_locktime != expected_taker_funding_locktime {
            let reason = AbortReason::TakerProvidedInvalidFundingLocktime(taker_data.funding_locktime);
            return Self::change_state(Aborted::new(reason), state_machine).await;
        }

        let expected_taker_payment_locktime = taker_data.started_at + state_machine.lock_duration;
        if taker_data.payment_locktime != expected_taker_payment_locktime {
            let reason = AbortReason::TakerProvidedInvalidPaymentLocktime(taker_data.payment_locktime);
            return Self::change_state(Aborted::new(reason), state_machine).await;
        }

        let taker_coin_htlc_pub_from_taker =
            match state_machine.taker_coin.parse_pubkey(&taker_data.taker_coin_htlc_pub) {
                Ok(p) => p,
                Err(e) => {
                    let reason = AbortReason::FailedToParsePubkey(e.to_string());
                    return Self::change_state(Aborted::new(reason), state_machine).await;
                },
            };

        let maker_coin_htlc_pub_from_taker =
            match state_machine.maker_coin.parse_pubkey(&taker_data.maker_coin_htlc_pub) {
                Ok(p) => p,
                Err(e) => {
                    let reason = AbortReason::FailedToParsePubkey(e.to_string());
                    return Self::change_state(Aborted::new(reason), state_machine).await;
                },
            };

        let next_state = WaitingForTakerFunding {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: NegotiationData {
                taker_payment_locktime: expected_taker_payment_locktime,
                taker_funding_locktime: expected_taker_funding_locktime,
                maker_coin_htlc_pub_from_taker,
                taker_coin_htlc_pub_from_taker,
                maker_coin_swap_contract: taker_data.maker_coin_swap_contract,
                taker_coin_swap_contract: taker_data.taker_coin_swap_contract,
                taker_secret_hash: taker_data.taker_secret_hash,
            },
        };
        Self::change_state(next_state, state_machine).await
    }
}

struct NegotiationData<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    taker_payment_locktime: u64,
    taker_funding_locktime: u64,
    maker_coin_htlc_pub_from_taker: MakerCoin::Pubkey,
    taker_coin_htlc_pub_from_taker: TakerCoin::Pubkey,
    maker_coin_swap_contract: Option<Vec<u8>>,
    taker_coin_swap_contract: Option<Vec<u8>>,
    taker_secret_hash: Vec<u8>,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> NegotiationData<MakerCoin, TakerCoin> {
    fn to_stored_data(&self) -> StoredNegotiationData {
        StoredNegotiationData {
            taker_payment_locktime: self.taker_payment_locktime,
            taker_funding_locktime: self.taker_funding_locktime,
            maker_coin_htlc_pub_from_taker: self.maker_coin_htlc_pub_from_taker.to_bytes().into(),
            taker_coin_htlc_pub_from_taker: self.taker_coin_htlc_pub_from_taker.to_bytes().into(),
            maker_coin_swap_contract: self.maker_coin_swap_contract.clone().map(|b| b.into()),
            taker_coin_swap_contract: self.taker_coin_swap_contract.clone().map(|b| b.into()),
            taker_secret_hash: self.taker_secret_hash.clone().into(),
        }
    }

    fn from_stored_data(
        stored: StoredNegotiationData,
        maker_coin: &MakerCoin,
        taker_coin: &TakerCoin,
    ) -> Result<Self, MmError<SwapRecreateError>> {
        Ok(NegotiationData {
            taker_payment_locktime: stored.taker_payment_locktime,
            taker_funding_locktime: stored.taker_funding_locktime,
            maker_coin_htlc_pub_from_taker: maker_coin
                .parse_pubkey(&stored.maker_coin_htlc_pub_from_taker.0)
                .map_err(|e| SwapRecreateError::FailedToParseData(e.to_string()))?,
            taker_coin_htlc_pub_from_taker: taker_coin
                .parse_pubkey(&stored.taker_coin_htlc_pub_from_taker.0)
                .map_err(|e| SwapRecreateError::FailedToParseData(e.to_string()))?,
            maker_coin_swap_contract: None,
            taker_coin_swap_contract: None,
            taker_secret_hash: stored.taker_secret_hash.into(),
        })
    }
}

struct WaitingForTakerFunding<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<Initialized<MakerCoin, TakerCoin>>
    for WaitingForTakerFunding<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for WaitingForTakerFunding<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let maker_negotiated_msg = MakerNegotiated {
            negotiated: true,
            reason: None,
        };
        debug!("Sending maker negotiated message {:?}", maker_negotiated_msg);
        let swap_msg = SwapMessage {
            inner: Some(swap_message::Inner::MakerNegotiated(maker_negotiated_msg)),
        };
        let abort_handle = broadcast_swap_v2_msg_every(
            state_machine.ctx.clone(),
            state_machine.p2p_topic.clone(),
            swap_msg,
            NEGOTIATE_SEND_INTERVAL,
            state_machine.p2p_keypair,
        );

        let recv_fut = recv_swap_v2_msg(
            state_machine.ctx.clone(),
            |store| store.taker_funding.take(),
            &state_machine.uuid,
            NEGOTIATION_TIMEOUT_SEC,
        );
        let taker_funding_info = match recv_fut.await {
            Ok(p) => p,
            Err(e) => {
                let reason = AbortReason::DidNotReceiveTakerFundingInfo(e);
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };
        drop(abort_handle);

        debug!("Received taker funding info message {:?}", taker_funding_info);
        let taker_funding = match state_machine.taker_coin.parse_tx(&taker_funding_info.tx_bytes) {
            Ok(tx) => tx,
            Err(e) => {
                let reason = AbortReason::FailedToParseTakerFunding(e.to_string());
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };
        let next_state = TakerFundingReceived {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data,
            taker_funding,
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> StorableState
    for WaitingForTakerFunding<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> MakerSwapEvent {
        MakerSwapEvent::WaitingForTakerFunding {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data.to_stored_data(),
        }
    }
}

struct TakerFundingReceived<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
    taker_funding: TakerCoin::Tx,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<WaitingForTakerFunding<MakerCoin, TakerCoin>>
    for TakerFundingReceived<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for TakerFundingReceived<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let unique_data = state_machine.unique_data();

        let validation_args = ValidateTakerFundingArgs {
            funding_tx: &self.taker_funding,
            time_lock: self.negotiation_data.taker_funding_locktime,
            taker_secret_hash: &self.negotiation_data.taker_secret_hash,
            other_pub: &self.negotiation_data.taker_coin_htlc_pub_from_taker,
            dex_fee_amount: state_machine.dex_fee_amount.to_decimal(),
            premium_amount: state_machine.taker_premium.to_decimal(),
            trading_amount: state_machine.taker_volume.to_decimal(),
            swap_unique_data: &unique_data,
        };

        if let Err(e) = state_machine.taker_coin.validate_taker_funding(validation_args).await {
            let reason = AbortReason::TakerFundingValidationFailed(e.to_string());
            return Self::change_state(Aborted::new(reason), state_machine).await;
        }

        let args = GenTakerFundingSpendArgs {
            funding_tx: &self.taker_funding,
            maker_pub: &state_machine.taker_coin.derive_htlc_pubkey_v2(&unique_data),
            taker_pub: &self.negotiation_data.taker_coin_htlc_pub_from_taker,
            funding_time_lock: self.negotiation_data.taker_funding_locktime,
            taker_secret_hash: &self.negotiation_data.taker_secret_hash,
            taker_payment_time_lock: self.negotiation_data.taker_payment_locktime,
            maker_secret_hash: &state_machine.secret_hash(),
        };
        let funding_spend_preimage = match state_machine
            .taker_coin
            .gen_taker_funding_spend_preimage(&args, &unique_data)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                let reason = AbortReason::FailedToGenerateFundingSpend(e.to_string());
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };

        let args = SendPaymentArgs {
            time_lock_duration: state_machine.lock_duration,
            time_lock: state_machine.maker_payment_locktime(),
            other_pubkey: &self.negotiation_data.maker_coin_htlc_pub_from_taker.to_bytes(),
            secret_hash: &state_machine.secret_hash(),
            amount: state_machine.maker_volume.to_decimal(),
            swap_contract_address: &None,
            swap_unique_data: &unique_data,
            payment_instructions: &None,
            watcher_reward: None,
            wait_for_confirmation_until: 0,
        };
        let maker_payment = match state_machine.maker_coin.send_maker_payment(args).compat().await {
            Ok(tx) => tx,
            Err(e) => {
                let reason = AbortReason::FailedToSendMakerPayment(format!("{:?}", e));
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };
        info!(
            "Sent maker payment {} tx {:02x} during swap {}",
            state_machine.maker_coin.ticker(),
            maker_payment.tx_hash(),
            state_machine.uuid
        );
        let next_state = MakerPaymentSentFundingSpendGenerated {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data,
            funding_spend_preimage,
            maker_payment: TransactionIdentifier {
                tx_hex: maker_payment.tx_hex().into(),
                tx_hash: maker_payment.tx_hash(),
            },
        };

        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> StorableState
    for TakerFundingReceived<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> MakerSwapEvent {
        MakerSwapEvent::TakerFundingReceived {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data.to_stored_data(),
            taker_funding: TransactionIdentifier {
                tx_hex: self.taker_funding.tx_hex().into(),
                tx_hash: self.taker_funding.tx_hash(),
            },
        }
    }
}

struct MakerPaymentSentFundingSpendGenerated<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
    funding_spend_preimage: TxPreimageWithSig<TakerCoin>,
    maker_payment: TransactionIdentifier,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<TakerFundingReceived<MakerCoin, TakerCoin>>
    for MakerPaymentSentFundingSpendGenerated<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for MakerPaymentSentFundingSpendGenerated<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let maker_payment_info = MakerPaymentInfo {
            tx_bytes: self.maker_payment.tx_hex.0.clone(),
            next_step_instructions: None,
            funding_preimage_sig: self.funding_spend_preimage.signature.to_bytes(),
            funding_preimage_tx: self.funding_spend_preimage.preimage.to_bytes(),
        };
        let swap_msg = SwapMessage {
            inner: Some(swap_message::Inner::MakerPaymentInfo(maker_payment_info)),
        };

        debug!("Sending maker payment info message {:?}", swap_msg);
        let abort_handle = broadcast_swap_v2_msg_every(
            state_machine.ctx.clone(),
            state_machine.p2p_topic.clone(),
            swap_msg,
            600.,
            state_machine.p2p_keypair,
        );

        let recv_fut = recv_swap_v2_msg(
            state_machine.ctx.clone(),
            |store| store.taker_payment.take(),
            &state_machine.uuid,
            NEGOTIATION_TIMEOUT_SEC,
        );
        let taker_payment_info = match recv_fut.await {
            Ok(p) => p,
            Err(e) => {
                let next_state = MakerPaymentRefundRequired {
                    maker_coin_start_block: self.maker_coin_start_block,
                    taker_coin_start_block: self.taker_coin_start_block,
                    negotiation_data: self.negotiation_data,
                    maker_payment: self.maker_payment,
                    reason: MakerPaymentRefundReason::DidNotGetTakerPayment(e),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };
        drop(abort_handle);

        let taker_payment = match state_machine.taker_coin.parse_tx(&taker_payment_info.tx_bytes) {
            Ok(tx) => tx,
            Err(e) => {
                let next_state = MakerPaymentRefundRequired {
                    maker_coin_start_block: self.maker_coin_start_block,
                    taker_coin_start_block: self.taker_coin_start_block,
                    negotiation_data: self.negotiation_data,
                    maker_payment: self.maker_payment,
                    reason: MakerPaymentRefundReason::FailedToParseTakerPayment(e.to_string()),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };

        let next_state = TakerPaymentReceived {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment,
            taker_payment,
            negotiation_data: self.negotiation_data,
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> StorableState
    for MakerPaymentSentFundingSpendGenerated<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> MakerSwapEvent {
        MakerSwapEvent::MakerPaymentSentFundingSpendGenerated {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data.to_stored_data(),
            maker_payment: self.maker_payment.clone(),
            funding_spend_preimage: StoredTxPreimage {
                preimage: self.funding_spend_preimage.preimage.to_bytes().into(),
                signature: self.funding_spend_preimage.signature.to_bytes().into(),
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum MakerPaymentRefundReason {
    DidNotGetTakerPayment(String),
    FailedToParseTakerPayment(String),
    TakerPaymentNotConfirmedInTime(String),
    DidNotGetTakerPaymentSpendPreimage(String),
    TakerPaymentSpendPreimageIsNotValid(String),
    FailedToParseTakerPreimage(String),
    FailedToParseTakerSignature(String),
    TakerPaymentSpendBroadcastFailed(String),
}

struct MakerPaymentRefundRequired<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
    maker_payment: TransactionIdentifier,
    reason: MakerPaymentRefundReason,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: SwapOpsV2>
    TransitionFrom<MakerPaymentSentFundingSpendGenerated<MakerCoin, TakerCoin>>
    for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
}
impl<MakerCoin: CoinAssocTypes, TakerCoin: SwapOpsV2> TransitionFrom<TakerPaymentReceived<MakerCoin, TakerCoin>>
    for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        warn!(
            "Entered MakerPaymentRefundRequired state for swap {} with reason {:?}",
            state_machine.uuid, self.reason
        );

        loop {
            match state_machine
                .maker_coin
                .can_refund_htlc(state_machine.maker_payment_locktime())
                .compat()
                .await
            {
                Ok(CanRefundHtlc::CanRefundNow) => break,
                Ok(CanRefundHtlc::HaveToWait(to_sleep)) => Timer::sleep(to_sleep as f64).await,
                Err(e) => {
                    error!("Error {} on can_refund_htlc, retrying in 30 seconds", e);
                    Timer::sleep(30.).await;
                },
            }
        }

        let other_pub = self.negotiation_data.maker_coin_htlc_pub_from_taker.to_bytes();
        let unique_data = state_machine.unique_data();
        let secret_hash = state_machine.secret_hash();

        let refund_args = RefundPaymentArgs {
            payment_tx: &self.maker_payment.tx_hex.0,
            time_lock: state_machine.maker_payment_locktime(),
            other_pubkey: &other_pub,
            secret_hash: &secret_hash,
            swap_contract_address: &None,
            swap_unique_data: &unique_data,
            watcher_reward: false,
        };

        let refund_tx = match state_machine.maker_coin.send_maker_refunds_payment(refund_args).await {
            Ok(tx) => tx,
            Err(e) => {
                let reason = AbortReason::MakerPaymentRefundFailed(e.get_plain_text_format());
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };

        let next_state = MakerPaymentRefunded {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_payment: self.maker_payment,
            maker_payment_refund: TransactionIdentifier {
                tx_hex: refund_tx.tx_hex().into(),
                tx_hash: refund_tx.tx_hash(),
            },
            reason: self.reason,
        };

        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> StorableState
    for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> MakerSwapEvent {
        MakerSwapEvent::MakerPaymentRefundRequired {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data.to_stored_data(),
            maker_payment: self.maker_payment.clone(),
            reason: self.reason.clone(),
        }
    }
}

struct TakerPaymentReceived<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    maker_payment: TransactionIdentifier,
    taker_payment: TakerCoin::Tx,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes>
    TransitionFrom<MakerPaymentSentFundingSpendGenerated<MakerCoin, TakerCoin>>
    for TakerPaymentReceived<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for TakerPaymentReceived<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let input = ConfirmPaymentInput {
            payment_tx: self.taker_payment.tx_hex(),
            confirmations: state_machine.conf_settings.taker_coin_confs,
            requires_nota: state_machine.conf_settings.taker_coin_nota,
            wait_until: state_machine.taker_payment_conf_timeout(),
            check_every: 10,
        };

        if let Err(e) = state_machine.taker_coin.wait_for_confirmations(input).compat().await {
            let next_state = MakerPaymentRefundRequired {
                maker_coin_start_block: self.maker_coin_start_block,
                taker_coin_start_block: self.taker_coin_start_block,
                negotiation_data: self.negotiation_data,
                maker_payment: self.maker_payment,
                reason: MakerPaymentRefundReason::TakerPaymentNotConfirmedInTime(e),
            };
            return Self::change_state(next_state, state_machine).await;
        }

        let recv_fut = recv_swap_v2_msg(
            state_machine.ctx.clone(),
            |store| store.taker_payment_spend_preimage.take(),
            &state_machine.uuid,
            state_machine.taker_payment_conf_timeout(),
        );
        let preimage_data = match recv_fut.await {
            Ok(preimage) => preimage,
            Err(e) => {
                let next_state = MakerPaymentRefundRequired {
                    maker_coin_start_block: self.maker_coin_start_block,
                    taker_coin_start_block: self.taker_coin_start_block,
                    negotiation_data: self.negotiation_data,
                    maker_payment: self.maker_payment,
                    reason: MakerPaymentRefundReason::DidNotGetTakerPaymentSpendPreimage(e),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };
        debug!("Received taker payment spend preimage message {:?}", preimage_data);

        let unique_data = state_machine.unique_data();

        let gen_args = GenTakerPaymentSpendArgs {
            taker_tx: &self.taker_payment,
            time_lock: self.negotiation_data.taker_payment_locktime,
            secret_hash: &state_machine.secret_hash(),
            maker_pub: &state_machine.taker_coin.derive_htlc_pubkey_v2(&unique_data),
            taker_pub: &self.negotiation_data.taker_coin_htlc_pub_from_taker,
            dex_fee_amount: state_machine.dex_fee_amount.to_decimal(),
            premium_amount: Default::default(),
            trading_amount: state_machine.taker_volume.to_decimal(),
            dex_fee_pub: &DEX_FEE_ADDR_RAW_PUBKEY,
        };

        let preimage = match state_machine.taker_coin.parse_preimage(&preimage_data.tx_preimage) {
            Ok(p) => p,
            Err(e) => {
                let next_state = MakerPaymentRefundRequired {
                    maker_coin_start_block: self.maker_coin_start_block,
                    taker_coin_start_block: self.taker_coin_start_block,
                    negotiation_data: self.negotiation_data,
                    maker_payment: self.maker_payment,
                    reason: MakerPaymentRefundReason::FailedToParseTakerPreimage(e.to_string()),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };
        let signature = match state_machine.taker_coin.parse_signature(&preimage_data.signature) {
            Ok(s) => s,
            Err(e) => {
                let next_state = MakerPaymentRefundRequired {
                    maker_coin_start_block: self.maker_coin_start_block,
                    taker_coin_start_block: self.taker_coin_start_block,
                    negotiation_data: self.negotiation_data,
                    maker_payment: self.maker_payment,
                    reason: MakerPaymentRefundReason::FailedToParseTakerSignature(e.to_string()),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };

        let tx_preimage = TxPreimageWithSig { preimage, signature };
        if let Err(e) = state_machine
            .taker_coin
            .validate_taker_payment_spend_preimage(&gen_args, &tx_preimage)
            .await
        {
            let next_state = MakerPaymentRefundRequired {
                maker_coin_start_block: self.maker_coin_start_block,
                taker_coin_start_block: self.taker_coin_start_block,
                negotiation_data: self.negotiation_data,
                maker_payment: self.maker_payment,
                reason: MakerPaymentRefundReason::TakerPaymentSpendPreimageIsNotValid(e.to_string()),
            };
            return Self::change_state(next_state, state_machine).await;
        }

        let taker_payment_spend = match state_machine
            .taker_coin
            .sign_and_broadcast_taker_payment_spend(
                &tx_preimage,
                &gen_args,
                state_machine.secret.as_slice(),
                &unique_data,
            )
            .await
        {
            Ok(tx) => tx,
            Err(e) => {
                let next_state = MakerPaymentRefundRequired {
                    maker_coin_start_block: self.maker_coin_start_block,
                    taker_coin_start_block: self.taker_coin_start_block,
                    negotiation_data: self.negotiation_data,
                    maker_payment: self.maker_payment,
                    reason: MakerPaymentRefundReason::TakerPaymentSpendBroadcastFailed(format!("{:?}", e)),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };
        info!(
            "Spent taker payment {} tx {:02x} during swap {}",
            state_machine.taker_coin.ticker(),
            taker_payment_spend.tx_hash(),
            state_machine.uuid
        );
        let next_state = TakerPaymentSpent {
            maker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment,
            taker_payment: self.taker_payment,
            taker_payment_spend: TransactionIdentifier {
                tx_hex: taker_payment_spend.tx_hex().into(),
                tx_hash: taker_payment_spend.tx_hash(),
            },
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> StorableState
    for TakerPaymentReceived<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> MakerSwapEvent {
        MakerSwapEvent::TakerPaymentConfirmed {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data.to_stored_data(),
            maker_payment: self.maker_payment.clone(),
            taker_payment: TransactionIdentifier {
                tx_hex: self.taker_payment.tx_hex().into(),
                tx_hash: self.taker_payment.tx_hash(),
            },
        }
    }
}

struct TakerPaymentSpent<MakerCoin, TakerCoin: CoinAssocTypes> {
    maker_coin: PhantomData<MakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    maker_payment: TransactionIdentifier,
    taker_payment: TakerCoin::Tx,
    taker_payment_spend: TransactionIdentifier,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<TakerPaymentReceived<MakerCoin, TakerCoin>>
    for TakerPaymentSpent<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for TakerPaymentSpent<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        Self::change_state(Completed::new(), state_machine).await
    }
}

impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> StorableState
    for TakerPaymentSpent<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> MakerSwapEvent {
        MakerSwapEvent::TakerPaymentSpent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment.clone(),
            taker_payment: TransactionIdentifier {
                tx_hex: self.taker_payment.tx_hex().into(),
                tx_hash: self.taker_payment.tx_hash(),
            },
            taker_payment_spend: self.taker_payment_spend.clone(),
        }
    }
}

/// Represents possible reasons of maker swap being aborted
#[derive(Clone, Debug, Deserialize, Display, Serialize)]
pub enum AbortReason {
    FailedToGetMakerCoinBlock(String),
    FailedToGetTakerCoinBlock(String),
    BalanceCheckFailure(String),
    DidNotReceiveTakerNegotiation(String),
    TakerAbortedNegotiation(String),
    ReceivedInvalidTakerNegotiation,
    DidNotReceiveTakerFundingInfo(String),
    FailedToParseTakerFunding(String),
    TakerFundingValidationFailed(String),
    FailedToGenerateFundingSpend(String),
    FailedToSendMakerPayment(String),
    TooLargeStartedAtDiff(u64),
    TakerProvidedInvalidFundingLocktime(u64),
    TakerProvidedInvalidPaymentLocktime(u64),
    FailedToParsePubkey(String),
    MakerPaymentRefundFailed(String),
}

struct Aborted<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    reason: AbortReason,
}

impl<MakerCoin, TakerCoin> Aborted<MakerCoin, TakerCoin> {
    fn new(reason: AbortReason) -> Aborted<MakerCoin, TakerCoin> {
        Aborted {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            reason,
        }
    }
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> LastState for Aborted<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(
        self: Box<Self>,
        state_machine: &mut Self::StateMachine,
    ) -> <Self::StateMachine as StateMachineTrait>::Result {
        warn!("Swap {} was aborted with reason {}", state_machine.uuid, self.reason);
    }
}

impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> StorableState
    for Aborted<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> MakerSwapEvent {
        MakerSwapEvent::Aborted {
            reason: self.reason.clone(),
        }
    }
}

impl<MakerCoin, TakerCoin> TransitionFrom<Initialize<MakerCoin, TakerCoin>> for Aborted<MakerCoin, TakerCoin> {}
impl<MakerCoin, TakerCoin> TransitionFrom<Initialized<MakerCoin, TakerCoin>> for Aborted<MakerCoin, TakerCoin> {}
impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<WaitingForTakerFunding<MakerCoin, TakerCoin>>
    for Aborted<MakerCoin, TakerCoin>
{
}
impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<TakerFundingReceived<MakerCoin, TakerCoin>>
    for Aborted<MakerCoin, TakerCoin>
{
}
impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes>
    TransitionFrom<MakerPaymentRefundRequired<MakerCoin, TakerCoin>> for Aborted<MakerCoin, TakerCoin>
{
}

struct Completed<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
}

impl<MakerCoin, TakerCoin> Completed<MakerCoin, TakerCoin> {
    fn new() -> Completed<MakerCoin, TakerCoin> {
        Completed {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
        }
    }
}

impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> StorableState
    for Completed<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> MakerSwapEvent { MakerSwapEvent::Completed }
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> LastState for Completed<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(
        self: Box<Self>,
        state_machine: &mut Self::StateMachine,
    ) -> <Self::StateMachine as StateMachineTrait>::Result {
        info!("Swap {} has been completed", state_machine.uuid);
    }
}

impl<MakerCoin, TakerCoin: CoinAssocTypes> TransitionFrom<TakerPaymentSpent<MakerCoin, TakerCoin>>
    for Completed<MakerCoin, TakerCoin>
{
}

struct MakerPaymentRefunded<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_payment: TransactionIdentifier,
    maker_payment_refund: TransactionIdentifier,
    reason: MakerPaymentRefundReason,
}

impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> StorableState
    for MakerPaymentRefunded<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> MakerSwapEvent {
        MakerSwapEvent::MakerPaymentRefunded {
            maker_payment: self.maker_payment.clone(),
            maker_payment_refund: self.maker_payment_refund.clone(),
            reason: self.reason.clone(),
        }
    }
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> LastState
    for MakerPaymentRefunded<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(
        self: Box<Self>,
        state_machine: &mut Self::StateMachine,
    ) -> <Self::StateMachine as StateMachineTrait>::Result {
        info!(
            "Swap {} has been finished with maker payment refund",
            state_machine.uuid
        );
    }
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes>
    TransitionFrom<MakerPaymentRefundRequired<MakerCoin, TakerCoin>> for MakerPaymentRefunded<MakerCoin, TakerCoin>
{
}
