use super::{NEGOTIATE_SEND_INTERVAL, NEGOTIATION_TIMEOUT_SEC};
use crate::mm2::database::my_swaps::{get_swap_events, insert_new_swap_v2, set_swap_is_finished, update_swap_events};
use crate::mm2::lp_network::subscribe_to_topic;
use crate::mm2::lp_swap::swap_v2_pb::*;
use crate::mm2::lp_swap::{broadcast_swap_v2_msg_every, check_balance_for_maker_swap, recv_swap_v2_msg, SecretHashAlgo,
                          SwapConfirmationsSettings, SwapsContext, TransactionIdentifier, MAKER_SWAP_V2_TYPE,
                          MAX_STARTED_AT_DIFF};
use async_trait::async_trait;
use bitcrypto::{dhash160, sha256};
use coins::{CoinAssocTypes, ConfirmPaymentInput, FeeApproxStage, GenTakerPaymentSpendArgs, MarketCoinOps, MmCoin,
            SendPaymentArgs, SwapOpsV2, ToBytes, Transaction, TxPreimageWithSig};
use common::log::{debug, info, warn};
use common::{bits256, Future01CompatExt, DEX_FEE_ADDR_RAW_PUBKEY};
use db_common::sqlite::rusqlite::params;
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::MmNumber;
use mm2_state_machine::prelude::*;
use mm2_state_machine::storable_state_machine::*;
use primitives::hash::H256;
use rpc::v1::types::Bytes as BytesJson;
use std::marker::PhantomData;
use uuid::Uuid;

// This is needed to have Debug on messages
#[allow(unused_imports)] use prost::Message;

/// Negotiation data representation to be stored in DB.
#[derive(Debug, Deserialize, Serialize)]
pub struct StoredNegotiationData {
    taker_payment_locktime: u64,
    maker_coin_htlc_pub_from_taker: BytesJson,
    taker_coin_htlc_pub_from_taker: BytesJson,
    maker_coin_swap_contract: Option<BytesJson>,
    taker_coin_swap_contract: Option<BytesJson>,
}

/// Represents events produced by maker swap states.
#[derive(Debug, Deserialize, Serialize)]
pub enum MakerSwapEvent {
    /// Swap has been successfully initialized.
    Initialized {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
    },
    /// Started waiting for taker payment.
    WaitingForTakerPayment {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        negotiation_data: StoredNegotiationData,
    },
    /// Received taker payment info.
    TakerPaymentReceived {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        negotiation_data: StoredNegotiationData,
        taker_payment: TransactionIdentifier,
    },
    /// Sent maker payment.
    MakerPaymentSent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        negotiation_data: StoredNegotiationData,
        maker_payment: TransactionIdentifier,
    },
    /// Something went wrong, so maker payment refund is required.
    MakerPaymentRefundRequired {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        negotiation_data: StoredNegotiationData,
        maker_payment: TransactionIdentifier,
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

/// Represents errors that can be produced by [`MakerSwapStateMachine`] run.
#[derive(Debug, Display)]
pub enum MakerSwapStateMachineError {
    StorageError(String),
    SerdeError(String),
}

/// Dummy storage for maker swap events (used temporary).
pub struct DummyMakerSwapStorage {
    ctx: MmArc,
}

impl DummyMakerSwapStorage {
    pub fn new(ctx: MmArc) -> Self { DummyMakerSwapStorage { ctx } }
}

#[async_trait]
impl StateMachineStorage for DummyMakerSwapStorage {
    type MachineId = Uuid;
    type Event = MakerSwapEvent;
    type Error = MmError<MakerSwapStateMachineError>;

    async fn store_event(&mut self, id: Self::MachineId, event: Self::Event) -> Result<(), Self::Error> {
        let id_str = id.to_string();
        let events_json = get_swap_events(&self.ctx.sqlite_connection(), &id_str)
            .map_to_mm(|e| MakerSwapStateMachineError::StorageError(e.to_string()))?;
        let mut events: Vec<MakerSwapEvent> =
            serde_json::from_str(&events_json).map_to_mm(|e| MakerSwapStateMachineError::SerdeError(e.to_string()))?;
        events.push(event);
        drop_mutability!(events);
        let serialized_events =
            serde_json::to_string(&events).map_to_mm(|e| MakerSwapStateMachineError::SerdeError(e.to_string()))?;
        update_swap_events(&self.ctx.sqlite_connection(), &id_str, &serialized_events)
            .map_to_mm(|e| MakerSwapStateMachineError::StorageError(e.to_string()))?;
        Ok(())
    }

    async fn get_unfinished(&self) -> Result<Vec<Self::MachineId>, Self::Error> { todo!() }

    async fn mark_finished(&mut self, id: Self::MachineId) -> Result<(), Self::Error> {
        set_swap_is_finished(&self.ctx.sqlite_connection(), &id.to_string())
            .map_to_mm(|e| MakerSwapStateMachineError::StorageError(e.to_string()))
    }
}

/// Represents the state machine for maker's side of the Trading Protocol Upgrade swap (v2).
pub struct MakerSwapStateMachine<MakerCoin, TakerCoin> {
    /// MM2 context
    pub ctx: MmArc,
    /// Storage
    pub storage: DummyMakerSwapStorage,
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
}

impl<MakerCoin, TakerCoin> MakerSwapStateMachine<MakerCoin, TakerCoin> {
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

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableStateMachine
    for MakerSwapStateMachine<MakerCoin, TakerCoin>
{
    type Storage = DummyMakerSwapStorage;
    type Result = ();

    fn storage(&mut self) -> &mut Self::Storage { &mut self.storage }

    fn id(&self) -> <Self::Storage as StateMachineStorage>::MachineId { self.uuid }

    fn restore_from_storage(
        _id: <Self::Storage as StateMachineStorage>::MachineId,
        _storage: Self::Storage,
    ) -> Result<RestoredMachine<Self>, <Self::Storage as StateMachineStorage>::Error> {
        todo!()
    }
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

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> InitialState for Initialize<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State for Initialize<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        {
            let sql_params = params![
                state_machine.maker_coin.ticker(),
                state_machine.taker_coin.ticker(),
                state_machine.uuid.to_string(),
                state_machine.started_at,
                MAKER_SWAP_V2_TYPE,
                state_machine.maker_volume.to_fraction_string(),
                state_machine.taker_volume.to_fraction_string(),
                state_machine.taker_premium.to_fraction_string(),
                state_machine.dex_fee_amount.to_fraction_string(),
                state_machine.secret.take(),
                state_machine.secret_hash(),
                state_machine.secret_hash_algo as u8,
                state_machine.p2p_keypair.map(|k| k.private_bytes()).unwrap_or_default(),
                state_machine.lock_duration,
                state_machine.conf_settings.maker_coin_confs,
                state_machine.conf_settings.maker_coin_nota,
                state_machine.conf_settings.taker_coin_confs,
                state_machine.conf_settings.taker_coin_nota
            ];
            insert_new_swap_v2(&state_machine.ctx, sql_params).unwrap();
        }

        subscribe_to_topic(&state_machine.ctx, state_machine.p2p_topic.clone());
        let swap_ctx = SwapsContext::from_ctx(&state_machine.ctx).expect("SwapsContext::from_ctx should not fail");
        swap_ctx.init_msg_v2_store(state_machine.uuid, bits256::default());

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

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState for Initialized<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
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

        let expected_taker_payment_locktime = taker_data.started_at + state_machine.lock_duration;
        if taker_data.payment_locktime != expected_taker_payment_locktime {
            let reason = AbortReason::TakerProvidedInvalidLocktime(taker_data.payment_locktime);
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

        let next_state = WaitingForTakerPayment {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: NegotiationData {
                taker_payment_locktime: expected_taker_payment_locktime,
                maker_coin_htlc_pub_from_taker,
                taker_coin_htlc_pub_from_taker,
                maker_coin_swap_contract: taker_data.maker_coin_swap_contract,
                taker_coin_swap_contract: taker_data.taker_coin_swap_contract,
            },
        };
        Self::change_state(next_state, state_machine).await
    }
}

struct NegotiationData<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    taker_payment_locktime: u64,
    maker_coin_htlc_pub_from_taker: MakerCoin::Pubkey,
    taker_coin_htlc_pub_from_taker: TakerCoin::Pubkey,
    maker_coin_swap_contract: Option<Vec<u8>>,
    taker_coin_swap_contract: Option<Vec<u8>>,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> NegotiationData<MakerCoin, TakerCoin> {
    fn to_stored_data(&self) -> StoredNegotiationData {
        StoredNegotiationData {
            taker_payment_locktime: self.taker_payment_locktime,
            maker_coin_htlc_pub_from_taker: self.maker_coin_htlc_pub_from_taker.to_bytes().into(),
            taker_coin_htlc_pub_from_taker: self.taker_coin_htlc_pub_from_taker.to_bytes().into(),
            maker_coin_swap_contract: self.maker_coin_swap_contract.clone().map(|b| b.into()),
            taker_coin_swap_contract: self.taker_coin_swap_contract.clone().map(|b| b.into()),
        }
    }
}

struct WaitingForTakerPayment<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<Initialized<MakerCoin, TakerCoin>>
    for WaitingForTakerPayment<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for WaitingForTakerPayment<MakerCoin, TakerCoin>
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
            |store| store.taker_payment.take(),
            &state_machine.uuid,
            NEGOTIATION_TIMEOUT_SEC,
        );
        let taker_payment_info = match recv_fut.await {
            Ok(p) => p,
            Err(e) => {
                let reason = AbortReason::DidNotReceiveTakerPaymentInfo(e);
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };
        drop(abort_handle);

        debug!("Received taker payment info message {:?}", taker_payment_info);
        let taker_payment = match state_machine.taker_coin.parse_tx(&taker_payment_info.tx_bytes) {
            Ok(tx) => tx,
            Err(e) => {
                let reason = AbortReason::FailedToParseTakerPayment(e.to_string());
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };
        let next_state = TakerPaymentReceived {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data,
            taker_payment,
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: CoinAssocTypes + Send + 'static, TakerCoin: SwapOpsV2> StorableState
    for WaitingForTakerPayment<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::WaitingForTakerPayment {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data.to_stored_data(),
        }
    }
}

struct TakerPaymentReceived<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
    taker_payment: TakerCoin::Tx,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<WaitingForTakerPayment<MakerCoin, TakerCoin>>
    for TakerPaymentReceived<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for TakerPaymentReceived<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let args = SendPaymentArgs {
            time_lock_duration: state_machine.lock_duration,
            time_lock: state_machine.maker_payment_locktime(),
            other_pubkey: &self.negotiation_data.maker_coin_htlc_pub_from_taker.to_bytes(),
            secret_hash: &state_machine.secret_hash(),
            amount: state_machine.maker_volume.to_decimal(),
            swap_contract_address: &None,
            swap_unique_data: &state_machine.unique_data(),
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
        let next_state = MakerPaymentSent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data,
            taker_payment: self.taker_payment,
            maker_payment: TransactionIdentifier {
                tx_hex: maker_payment.tx_hex().into(),
                tx_hash: maker_payment.tx_hash(),
            },
        };

        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: CoinAssocTypes + Send + 'static, TakerCoin: SwapOpsV2> StorableState
    for TakerPaymentReceived<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::TakerPaymentReceived {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data.to_stored_data(),
            taker_payment: TransactionIdentifier {
                tx_hex: self.taker_payment.tx_hex().into(),
                tx_hash: self.taker_payment.tx_hash(),
            },
        }
    }
}

struct MakerPaymentSent<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
    taker_payment: TakerCoin::Tx,
    maker_payment: TransactionIdentifier,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<TakerPaymentReceived<MakerCoin, TakerCoin>>
    for MakerPaymentSent<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for MakerPaymentSent<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let maker_payment_info = MakerPaymentInfo {
            tx_bytes: self.maker_payment.tx_hex.0.clone(),
            next_step_instructions: None,
        };
        let swap_msg = SwapMessage {
            inner: Some(swap_message::Inner::MakerPaymentInfo(maker_payment_info)),
        };

        debug!("Sending maker payment info message {:?}", swap_msg);
        let _abort_handle = broadcast_swap_v2_msg_every(
            state_machine.ctx.clone(),
            state_machine.p2p_topic.clone(),
            swap_msg,
            600.,
            state_machine.p2p_keypair,
        );
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

        let next_state = TakerPaymentConfirmed {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment,
            taker_payment: self.taker_payment,
            negotiation_data: self.negotiation_data,
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: CoinAssocTypes + Send + 'static, TakerCoin: SwapOpsV2> StorableState
    for MakerPaymentSent<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::MakerPaymentSent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data.to_stored_data(),
            maker_payment: self.maker_payment.clone(),
        }
    }
}

#[derive(Debug)]
enum MakerPaymentRefundReason {
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

impl<MakerCoin: CoinAssocTypes, TakerCoin: SwapOpsV2> TransitionFrom<MakerPaymentSent<MakerCoin, TakerCoin>>
    for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
}
impl<MakerCoin: CoinAssocTypes, TakerCoin: SwapOpsV2> TransitionFrom<TakerPaymentConfirmed<MakerCoin, TakerCoin>>
    for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<
        MakerCoin: CoinAssocTypes + Send + Sync + 'static,
        TakerCoin: MarketCoinOps + CoinAssocTypes + Send + Sync + 'static,
    > State for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        warn!(
            "Entered MakerPaymentRefundRequired state for swap {} with reason {:?}",
            state_machine.uuid, self.reason
        );
        unimplemented!()
    }
}

impl<MakerCoin: CoinAssocTypes + Send + 'static, TakerCoin: CoinAssocTypes + Send + 'static> StorableState
    for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::MakerPaymentRefundRequired {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data.to_stored_data(),
            maker_payment: self.maker_payment.clone(),
        }
    }
}

#[allow(dead_code)]
struct TakerPaymentConfirmed<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    maker_payment: TransactionIdentifier,
    taker_payment: TakerCoin::Tx,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<MakerPaymentSent<MakerCoin, TakerCoin>>
    for TakerPaymentConfirmed<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for TakerPaymentConfirmed<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
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

impl<MakerCoin: CoinAssocTypes + Send + 'static, TakerCoin: SwapOpsV2> StorableState
    for TakerPaymentConfirmed<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
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

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<TakerPaymentConfirmed<MakerCoin, TakerCoin>>
    for TakerPaymentSpent<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: CoinAssocTypes + Send + Sync + 'static> State
    for TakerPaymentSpent<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        Self::change_state(Completed::new(), state_machine).await
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: CoinAssocTypes + Send + 'static> StorableState
    for TakerPaymentSpent<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
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
    DidNotReceiveTakerPaymentInfo(String),
    FailedToParseTakerPayment(String),
    FailedToSendMakerPayment(String),
    TooLargeStartedAtDiff(u64),
    TakerProvidedInvalidLocktime(u64),
    FailedToParsePubkey(String),
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
impl<MakerCoin: Send + Sync + 'static, TakerCoin: Send + Sync + 'static> LastState for Aborted<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(
        self: Box<Self>,
        state_machine: &mut Self::StateMachine,
    ) -> <Self::StateMachine as StateMachineTrait>::Result {
        warn!("Swap {} was aborted with reason {}", state_machine.uuid, self.reason);
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState for Aborted<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::Aborted {
            reason: self.reason.clone(),
        }
    }
}

impl<MakerCoin, TakerCoin> TransitionFrom<Initialize<MakerCoin, TakerCoin>> for Aborted<MakerCoin, TakerCoin> {}
impl<MakerCoin, TakerCoin> TransitionFrom<Initialized<MakerCoin, TakerCoin>> for Aborted<MakerCoin, TakerCoin> {}
impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<WaitingForTakerPayment<MakerCoin, TakerCoin>>
    for Aborted<MakerCoin, TakerCoin>
{
}
impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<TakerPaymentReceived<MakerCoin, TakerCoin>>
    for Aborted<MakerCoin, TakerCoin>
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

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState for Completed<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::Completed
    }
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: Send + Sync + 'static> LastState for Completed<MakerCoin, TakerCoin> {
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
