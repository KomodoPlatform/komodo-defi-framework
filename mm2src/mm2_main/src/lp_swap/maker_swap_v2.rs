use crate::mm2::lp_network::subscribe_to_topic;
use crate::mm2::lp_swap::swap_v2_pb::*;
use crate::mm2::lp_swap::{broadcast_swap_v2_msg_every, check_balance_for_maker_swap, recv_swap_v2_msg, SecretHashAlgo,
                          SwapConfirmationsSettings, SwapsContext, TransactionIdentifier};
use async_trait::async_trait;
use bitcrypto::{dhash160, sha256};
use coins::{ConfirmPaymentInput, FeeApproxStage, MarketCoinOps, MmCoin, SwapOpsV2};
use common::log::{debug, info};
use common::{bits256, Future01CompatExt};
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_number::MmNumber;
use mm2_state_machine::prelude::*;
use mm2_state_machine::storable_state_machine::*;
use primitives::hash::H256;
use std::collections::HashMap;
use std::marker::PhantomData;
use uuid::Uuid;

// This is needed to have Debug on messages
#[allow(unused_imports)] use prost::Message;

const NEGOTIATION_TIMEOUT_SEC: u64 = 90;

#[derive(Debug, PartialEq)]
pub enum MakerSwapEvent {
    Initialized {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
    },
    WaitingForTakerPayment {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
    },
    MakerPaymentSent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        maker_payment: TransactionIdentifier,
    },
    MakerPaymentRefundRequired {
        maker_payment: TransactionIdentifier,
    },
    BothPaymentsSentAndConfirmed {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        maker_payment: TransactionIdentifier,
        taker_payment: TransactionIdentifier,
    },
    TakerPaymentSpent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        maker_payment: TransactionIdentifier,
        taker_payment: TransactionIdentifier,
        taker_payment_spend: TransactionIdentifier,
    },
    Aborted {
        reason: String,
    },
    Completed,
}

#[derive(Debug, Display)]
pub enum MakerSwapStateMachineError {}

pub struct DummyMakerSwapStorage {
    events: HashMap<Uuid, Vec<MakerSwapEvent>>,
}

impl DummyMakerSwapStorage {
    pub fn new() -> Self { DummyMakerSwapStorage { events: HashMap::new() } }
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

    async fn mark_finished(&mut self, id: Self::MachineId) -> Result<(), Self::Error> { Ok(()) }
}

pub struct MakerSwapStateMachine<MakerCoin, TakerCoin> {
    pub ctx: MmArc,
    pub storage: DummyMakerSwapStorage,
    pub maker_coin: MakerCoin,
    pub maker_volume: MmNumber,
    pub secret: H256,
    pub secret_hash_algo: SecretHashAlgo,
    pub started_at: u64,
    pub lock_duration: u64,
    pub taker_coin: TakerCoin,
    pub taker_volume: MmNumber,
    pub taker_premium: MmNumber,
    pub conf_settings: SwapConfirmationsSettings,
    pub uuid: Uuid,
    pub p2p_topic: String,
    pub p2p_keypair: Option<KeyPair>,
}

impl<MakerCoin, TakerCoin> MakerSwapStateMachine<MakerCoin, TakerCoin> {
    #[inline]
    fn taker_payment_conf_timeout(&self) -> u64 { 0 }

    #[inline]
    fn maker_payment_locktime(&self) -> u64 { self.started_at + self.lock_duration }

    fn secret_hash(&self) -> Vec<u8> {
        match self.secret_hash_algo {
            SecretHashAlgo::DHASH160 => dhash160(self.secret.as_slice()).take().into(),
            SecretHashAlgo::SHA256 => sha256(self.secret.as_slice()).take().into(),
        }
    }

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

pub struct Initialize<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
}

impl<MakerCoin, TakerCoin> Initialize<MakerCoin, TakerCoin> {
    pub fn new() -> Self {
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
impl<MakerCoin: MmCoin + Send + Sync + 'static, TakerCoin: MmCoin + Send + Sync + 'static> State
    for Initialized<MakerCoin, TakerCoin>
{
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
            30.,
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
impl<MakerCoin: Send + Sync + 'static, TakerCoin: MarketCoinOps + Send + Sync + 'static> State
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
            30.,
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
impl<MakerCoin: Send + Sync + 'static, TakerCoin: MarketCoinOps + Send + Sync + 'static> State
    for TakerPaymentReceived<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        unimplemented!()
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState
    for TakerPaymentReceived<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        unimplemented!()
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
impl<MakerCoin: Send + Sync + 'static, TakerCoin: MarketCoinOps + Send + Sync + 'static> State
    for MakerPaymentSent<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let taker_payment = TransactionIdentifier {
            tx_hex: Default::default(),
            tx_hash: Default::default(),
        };

        let input = ConfirmPaymentInput {
            payment_tx: taker_payment.tx_hex.0.clone(),
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
            };
            return Self::change_state(next_state, state_machine).await;
        }

        let next_state = BothPaymentsSentAndConfirmed {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment,
            taker_payment,
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

struct MakerPaymentRefundRequired<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_payment: TransactionIdentifier,
}

impl<MakerCoin, TakerCoin> TransitionFrom<MakerPaymentSent<MakerCoin, TakerCoin>>
    for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: MarketCoinOps + Send + Sync + 'static> State
    for MakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
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

struct BothPaymentsSentAndConfirmed<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    maker_payment: TransactionIdentifier,
    taker_payment: TransactionIdentifier,
}

impl<MakerCoin, TakerCoin> TransitionFrom<MakerPaymentSent<MakerCoin, TakerCoin>>
    for BothPaymentsSentAndConfirmed<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: Send + Sync + 'static> State
    for BothPaymentsSentAndConfirmed<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let next_state = TakerPaymentSpent {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment,
            taker_payment: self.taker_payment,
            taker_payment_spend: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState
    for BothPaymentsSentAndConfirmed<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::BothPaymentsSentAndConfirmed {
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

impl<MakerCoin, TakerCoin> TransitionFrom<BothPaymentsSentAndConfirmed<MakerCoin, TakerCoin>>
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
        _state_machine: &mut Self::StateMachine,
    ) -> <Self::StateMachine as StateMachineTrait>::Result {
        //TODO just log something here?
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
        _state_machine: &mut Self::StateMachine,
    ) -> <Self::StateMachine as StateMachineTrait>::Result {
        //TODO just log something here?
    }
}

impl<MakerCoin, TakerCoin> TransitionFrom<TakerPaymentSpent<MakerCoin, TakerCoin>> for Completed<MakerCoin, TakerCoin> {}
