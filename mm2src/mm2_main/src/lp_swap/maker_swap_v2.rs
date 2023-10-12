use std::collections::HashMap;
use std::marker::PhantomData;

use async_trait::async_trait;
use bitcrypto::{dhash160, sha256};
use coins::{ConfirmPaymentInput, FeeApproxStage, GenTakerPaymentSpendArgs, MarketCoinOps, MmCoin, SendPaymentArgs,
            SwapOpsV2, TxPreimageWithSig};
use common::log::{debug, info, warn};
use common::{bits256, Future01CompatExt, DEX_FEE_ADDR_RAW_PUBKEY};
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_number::MmNumber;
use mm2_state_machine::prelude::*;
use mm2_state_machine::storable_state_machine::*;
use primitives::hash::H256;
// This is needed to have Debug on messages
#[allow(unused_imports)] use prost::Message;
use uuid::Uuid;

use super::{NEGOTIATE_SEND_INTERVAL, NEGOTIATION_TIMEOUT_SEC};
use crate::mm2::lp_network::subscribe_to_topic;
use crate::mm2::lp_swap::swap_v2_pb::*;
use crate::mm2::lp_swap::{broadcast_swap_v2_msg_every, check_balance_for_maker_swap, recv_swap_v2_msg, SecretHashAlgo,
                          SwapConfirmationsSettings, SwapsContext, TransactionIdentifier};

/// Represents events produced by maker swap states.
#[derive(Debug, PartialEq)]
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
    },
    /// Received taker payment info.
    TakerPaymentReceived {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        taker_payment: TransactionIdentifier,
    },
    /// Sent maker payment.
    MakerPaymentSent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        maker_payment: TransactionIdentifier,
    },
    /// Something went wrong, so maker payment refund is required.
    MakerPaymentRefundRequired { maker_payment: TransactionIdentifier },
    /// Taker payment has been confirmed on-chain.
    TakerPaymentConfirmed {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
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
    Aborted { reason: String },
    /// Swap completed successfully.
    Completed,
}

/// Represents errors that can be produced by [`MakerSwapStateMachine`] run.
#[derive(Debug, Display)]
pub enum MakerSwapStateMachineError {}

/// Dummy storage for maker swap events (used temporary).
#[derive(Default)]
pub struct DummyMakerSwapStorage {
    events: HashMap<Uuid, Vec<MakerSwapEvent>>,
}

#[async_trait]
impl StateMachineStorage for DummyMakerSwapStorage {
    type MachineId = Uuid;
    type Event = MakerSwapEvent;
    type Error = MakerSwapStateMachineError;

    async fn store_event(&mut self, id: Self::MachineId, event: Self::Event) -> Result<(), Self::Error> {
        self.events.entry(id).or_insert_with(Vec::new).push(event);
        Ok(())
    }

    async fn get_unfinished(&self) -> Result<Vec<Self::MachineId>, Self::Error> {
        Ok(self.events.keys().copied().collect())
    }

    async fn mark_finished(&mut self, _id: Self::MachineId) -> Result<(), Self::Error> { Ok(()) }
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
impl<MakerCoin: MmCoin + SwapOpsV2 + Send + Sync + 'static, TakerCoin: MmCoin + SwapOpsV2 + Send + Sync + 'static> State
    for Initialize<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        subscribe_to_topic(&state_machine.ctx, state_machine.p2p_topic.clone());
        let swap_ctx = SwapsContext::from_ctx(&state_machine.ctx).expect("SwapsContext::from_ctx should not fail");
        swap_ctx.init_msg_v2_store(state_machine.uuid, bits256::default());

        let maker_coin_start_block = match state_machine.maker_coin.current_block().compat().await {
            Ok(b) => b,
            Err(e) => return Self::change_state(Aborted::new(e), state_machine).await,
        };

        let taker_coin_start_block = match state_machine.taker_coin.current_block().compat().await {
            Ok(b) => b,
            Err(e) => return Self::change_state(Aborted::new(e), state_machine).await,
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
            return Self::change_state(Aborted::new(e.to_string()), state_machine).await;
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
impl<MakerCoin: MmCoin, TakerCoin: MmCoin + SwapOpsV2> State for Initialized<MakerCoin, TakerCoin> {
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
                let next_state = Aborted::new(format!("Failed to receive TakerNegotiation: {}", e));
                return Self::change_state(next_state, state_machine).await;
            },
        };
        drop(abort_handle);

        debug!("Received taker negotiation message {:?}", taker_negotiation);
        let taker_data = match taker_negotiation.action {
            Some(taker_negotiation::Action::Continue(data)) => data,
            Some(taker_negotiation::Action::Abort(abort)) => {
                let next_state = Aborted::new(abort.reason);
                return Self::change_state(next_state, state_machine).await;
            },
            None => {
                let next_state = Aborted::new("received invalid negotiation message from taker".into());
                return Self::change_state(next_state, state_machine).await;
            },
        };

        let next_state = WaitingForTakerPayment {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            taker_payment_locktime: taker_data.payment_locktime,
            maker_coin_htlc_pub_from_taker: taker_data.maker_coin_htlc_pub,
            taker_coin_htlc_pub_from_taker: taker_data.taker_coin_htlc_pub,
            maker_coin_swap_contract: taker_data.maker_coin_swap_contract,
            taker_coin_swap_contract: taker_data.taker_coin_swap_contract,
        };
        Self::change_state(next_state, state_machine).await
    }
}

struct WaitingForTakerPayment<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    taker_payment_locktime: u64,
    maker_coin_htlc_pub_from_taker: Vec<u8>,
    taker_coin_htlc_pub_from_taker: Vec<u8>,
    maker_coin_swap_contract: Option<Vec<u8>>,
    taker_coin_swap_contract: Option<Vec<u8>>,
}

impl<MakerCoin, TakerCoin> TransitionFrom<Initialized<MakerCoin, TakerCoin>>
    for WaitingForTakerPayment<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin, TakerCoin: MmCoin + SwapOpsV2> State for WaitingForTakerPayment<MakerCoin, TakerCoin> {
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
        let taker_payment = match recv_fut.await {
            Ok(p) => p,
            Err(e) => {
                let next_state = Aborted::new(format!("Failed to receive TakerPaymentInfo: {}", e));
                return Self::change_state(next_state, state_machine).await;
            },
        };
        drop(abort_handle);

        debug!("Received taker payment info message {:?}", taker_payment);
        let next_state = TakerPaymentReceived {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            taker_payment_locktime: self.taker_payment_locktime,
            maker_coin_htlc_pub_from_taker: self.maker_coin_htlc_pub_from_taker,
            taker_coin_htlc_pub_from_taker: self.taker_coin_htlc_pub_from_taker,
            maker_coin_swap_contract: self.maker_coin_swap_contract,
            taker_coin_swap_contract: self.taker_coin_swap_contract,
            taker_payment: TransactionIdentifier {
                tx_hex: taker_payment.tx_bytes.into(),
                tx_hash: Default::default(),
            },
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState
    for WaitingForTakerPayment<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::WaitingForTakerPayment {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
        }
    }
}

struct TakerPaymentReceived<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    taker_payment_locktime: u64,
    maker_coin_htlc_pub_from_taker: Vec<u8>,
    taker_coin_htlc_pub_from_taker: Vec<u8>,
    maker_coin_swap_contract: Option<Vec<u8>>,
    taker_coin_swap_contract: Option<Vec<u8>>,
    taker_payment: TransactionIdentifier,
}

impl<MakerCoin, TakerCoin> TransitionFrom<WaitingForTakerPayment<MakerCoin, TakerCoin>>
    for TakerPaymentReceived<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin, TakerCoin: MmCoin + SwapOpsV2> State for TakerPaymentReceived<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let args = SendPaymentArgs {
            time_lock_duration: state_machine.lock_duration,
            time_lock: state_machine.maker_payment_locktime(),
            other_pubkey: &self.maker_coin_htlc_pub_from_taker,
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
                let next_state = Aborted::new(format!("Failed to send maker payment {:?}", e));
                return Self::change_state(next_state, state_machine).await;
            },
        };
        info!(
            "Sent maker payment {} tx {:02x} during swap {}",
            state_machine.maker_coin.ticker(),
            maker_payment.tx_hash(),
            state_machine.uuid
        );
        let next_state = MakerPaymentSent {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            taker_payment_locktime: self.taker_payment_locktime,
            maker_coin_htlc_pub_from_taker: self.maker_coin_htlc_pub_from_taker,
            taker_coin_htlc_pub_from_taker: self.taker_coin_htlc_pub_from_taker,
            maker_coin_swap_contract: self.maker_coin_swap_contract,
            taker_coin_swap_contract: self.taker_coin_swap_contract,
            taker_payment: self.taker_payment,
            maker_payment: TransactionIdentifier {
                tx_hex: maker_payment.tx_hex().into(),
                tx_hash: maker_payment.tx_hash(),
            },
        };

        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState
    for TakerPaymentReceived<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::TakerPaymentReceived {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            taker_payment: self.taker_payment.clone(),
        }
    }
}

struct MakerPaymentSent<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    taker_payment_locktime: u64,
    maker_coin_htlc_pub_from_taker: Vec<u8>,
    taker_coin_htlc_pub_from_taker: Vec<u8>,
    maker_coin_swap_contract: Option<Vec<u8>>,
    taker_coin_swap_contract: Option<Vec<u8>>,
    taker_payment: TransactionIdentifier,
    maker_payment: TransactionIdentifier,
}

impl<MakerCoin, TakerCoin> TransitionFrom<TakerPaymentReceived<MakerCoin, TakerCoin>>
    for MakerPaymentSent<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin, TakerCoin: MmCoin + SwapOpsV2> State for MakerPaymentSent<MakerCoin, TakerCoin> {
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
            payment_tx: self.taker_payment.tx_hex.0.clone(),
            confirmations: state_machine.conf_settings.taker_coin_confs,
            requires_nota: state_machine.conf_settings.taker_coin_nota,
            wait_until: state_machine.taker_payment_conf_timeout(),
            check_every: 10,
        };
        if let Err(e) = state_machine.taker_coin.wait_for_confirmations(input).compat().await {
            let next_state = MakerPaymentRefundRequired {
                maker_coin: Default::default(),
                taker_coin: Default::default(),
                maker_payment: self.maker_payment,
                reason: MakerPaymentRefundReason::TakerPaymentNotConfirmedInTime(e),
            };
            return Self::change_state(next_state, state_machine).await;
        }

        let next_state = TakerPaymentConfirmed {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment,
            taker_payment: self.taker_payment,
            taker_payment_locktime: self.taker_payment_locktime,
            maker_coin_htlc_pub_from_taker: self.maker_coin_htlc_pub_from_taker,
            taker_coin_htlc_pub_from_taker: self.taker_coin_htlc_pub_from_taker,
            maker_coin_swap_contract: self.maker_coin_swap_contract,
            taker_coin_swap_contract: self.taker_coin_swap_contract,
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState for MakerPaymentSent<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::MakerPaymentSent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment.clone(),
        }
    }
}

#[derive(Debug)]
enum MakerPaymentRefundReason {
    TakerPaymentNotConfirmedInTime(String),
    DidNotGetTakerPaymentSpendPreimage(String),
    TakerPaymentSpendPreimageIsNotValid(String),
    TakerPaymentSpendBroadcastFailed(String),
}

struct MakerPaymentRefundRequired<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_payment: TransactionIdentifier,
    reason: MakerPaymentRefundReason,
}

impl<MakerCoin, TakerCoin> TransitionFrom<MakerPaymentSent<MakerCoin, TakerCoin>>
    for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
}
impl<MakerCoin, TakerCoin> TransitionFrom<TakerPaymentConfirmed<MakerCoin, TakerCoin>>
    for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: MarketCoinOps + Send + Sync + 'static> State
    for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
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

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState
    for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::MakerPaymentRefundRequired {
            maker_payment: self.maker_payment.clone(),
        }
    }
}

#[allow(dead_code)]
struct TakerPaymentConfirmed<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    maker_payment: TransactionIdentifier,
    taker_payment: TransactionIdentifier,
    taker_payment_locktime: u64,
    maker_coin_htlc_pub_from_taker: Vec<u8>,
    taker_coin_htlc_pub_from_taker: Vec<u8>,
    maker_coin_swap_contract: Option<Vec<u8>>,
    taker_coin_swap_contract: Option<Vec<u8>>,
}

impl<MakerCoin, TakerCoin> TransitionFrom<MakerPaymentSent<MakerCoin, TakerCoin>>
    for TakerPaymentConfirmed<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin, TakerCoin: MmCoin + SwapOpsV2> State for TakerPaymentConfirmed<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let recv_fut = recv_swap_v2_msg(
            state_machine.ctx.clone(),
            |store| store.taker_payment_spend_preimage.take(),
            &state_machine.uuid,
            state_machine.taker_payment_conf_timeout(),
        );
        let preimage = match recv_fut.await {
            Ok(preimage) => preimage,
            Err(e) => {
                let next_state = MakerPaymentRefundRequired {
                    maker_coin: Default::default(),
                    taker_coin: Default::default(),
                    maker_payment: self.maker_payment,
                    reason: MakerPaymentRefundReason::DidNotGetTakerPaymentSpendPreimage(e),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };
        debug!("Received taker payment spend preimage message {:?}", preimage);

        let unique_data = state_machine.unique_data();

        let gen_args = GenTakerPaymentSpendArgs {
            taker_tx: &self.taker_payment.tx_hex.0,
            time_lock: self.taker_payment_locktime,
            secret_hash: &state_machine.secret_hash(),
            maker_pub: &state_machine.maker_coin.derive_htlc_pubkey(&unique_data),
            taker_pub: &self.taker_coin_htlc_pub_from_taker,
            dex_fee_amount: state_machine.dex_fee_amount.to_decimal(),
            premium_amount: Default::default(),
            trading_amount: state_machine.taker_volume.to_decimal(),
            dex_fee_pub: &DEX_FEE_ADDR_RAW_PUBKEY,
        };
        let tx_preimage = TxPreimageWithSig {
            preimage: preimage.tx_preimage.unwrap_or_default(),
            signature: preimage.signature,
        };
        if let Err(e) = state_machine
            .taker_coin
            .validate_taker_payment_spend_preimage(&gen_args, &tx_preimage)
            .await
        {
            let next_state = MakerPaymentRefundRequired {
                maker_coin: Default::default(),
                taker_coin: Default::default(),
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
                    maker_coin: Default::default(),
                    taker_coin: Default::default(),
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
            taker_coin: Default::default(),
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

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState
    for TakerPaymentConfirmed<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::TakerPaymentConfirmed {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment.clone(),
            taker_payment: self.taker_payment.clone(),
        }
    }
}

struct TakerPaymentSpent<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    maker_payment: TransactionIdentifier,
    taker_payment: TransactionIdentifier,
    taker_payment_spend: TransactionIdentifier,
}

impl<MakerCoin, TakerCoin> TransitionFrom<TakerPaymentConfirmed<MakerCoin, TakerCoin>>
    for TakerPaymentSpent<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: Send + Sync + 'static> State
    for TakerPaymentSpent<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        Self::change_state(Completed::new(), state_machine).await
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState for TakerPaymentSpent<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::TakerPaymentSpent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment.clone(),
            taker_payment: self.taker_payment.clone(),
            taker_payment_spend: self.taker_payment_spend.clone(),
        }
    }
}

struct Aborted<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    reason: String,
}

impl<MakerCoin, TakerCoin> Aborted<MakerCoin, TakerCoin> {
    fn new(reason: String) -> Aborted<MakerCoin, TakerCoin> {
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
impl<MakerCoin, TakerCoin> TransitionFrom<WaitingForTakerPayment<MakerCoin, TakerCoin>>
    for Aborted<MakerCoin, TakerCoin>
{
}
impl<MakerCoin, TakerCoin> TransitionFrom<TakerPaymentReceived<MakerCoin, TakerCoin>>
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

impl<MakerCoin, TakerCoin> TransitionFrom<TakerPaymentSpent<MakerCoin, TakerCoin>> for Completed<MakerCoin, TakerCoin> {}
