use super::{NEGOTIATE_SEND_INTERVAL, NEGOTIATION_TIMEOUT_SEC};
use crate::mm2::lp_network::subscribe_to_topic;
use crate::mm2::lp_swap::swap_v2_pb::*;
use crate::mm2::lp_swap::{broadcast_swap_v2_msg_every, check_balance_for_taker_swap, recv_swap_v2_msg,
                          SwapConfirmationsSettings, SwapsContext, TransactionIdentifier};
use async_trait::async_trait;
use coins::{ConfirmPaymentInput, FeeApproxStage, GenTakerPaymentSpendArgs, MmCoin, SendCombinedTakerPaymentArgs,
            SpendPaymentArgs, SwapOpsV2, WaitForHTLCTxSpendArgs};
use common::log::{debug, info, warn};
use common::{bits256, Future01CompatExt, DEX_FEE_ADDR_RAW_PUBKEY};
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_number::{BigDecimal, MmNumber};
use mm2_state_machine::prelude::*;
use mm2_state_machine::storable_state_machine::*;
use rpc::v1::types::Bytes as BytesJson;
use std::collections::HashMap;
use std::marker::PhantomData;
use uuid::Uuid;

// This is needed to have Debug on messages
#[allow(unused_imports)] use prost::Message;

/// Represents events produced by taker swap states.
#[derive(Debug, PartialEq)]
pub enum TakerSwapEvent {
    /// Swap has been successfully initialized.
    Initialized {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
    },
    /// Negotiated swap data with maker.
    Negotiated {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        secret_hash: BytesJson,
    },
    /// Sent taker payment.
    TakerPaymentSent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        taker_payment: TransactionIdentifier,
        secret_hash: BytesJson,
    },
    /// Something went wrong, so taker payment refund is required.
    TakerPaymentRefundRequired {
        taker_payment: TransactionIdentifier,
        secret_hash: BytesJson,
    },
    /// Both payments are confirmed on-chain
    BothPaymentsSentAndConfirmed {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        maker_payment: TransactionIdentifier,
        taker_payment: TransactionIdentifier,
        secret_hash: BytesJson,
    },
    /// Maker spent taker's payment and taker discovered the tx on-chain.
    TakerPaymentSpent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        maker_payment: TransactionIdentifier,
        taker_payment: TransactionIdentifier,
        taker_payment_spend: TransactionIdentifier,
        secret: BytesJson,
    },
    /// Taker spent maker's payment.
    MakerPaymentSpent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        maker_payment: TransactionIdentifier,
        taker_payment: TransactionIdentifier,
        taker_payment_spend: TransactionIdentifier,
        maker_payment_spend: TransactionIdentifier,
    },
    /// Swap has been aborted before taker payment was sent.
    Aborted { reason: String },
    /// Swap completed successfully.
    Completed,
}

/// Represents errors that can be produced by [`TakerSwapStateMachine`] run.
#[derive(Debug, Display)]
pub enum TakerSwapStateMachineError {}

/// Dummy storage for taker swap events (used temporary).
#[derive(Default)]
pub struct DummyTakerSwapStorage {
    events: HashMap<Uuid, Vec<TakerSwapEvent>>,
}

#[async_trait]
impl StateMachineStorage for DummyTakerSwapStorage {
    type MachineId = Uuid;
    type Event = TakerSwapEvent;
    type Error = TakerSwapStateMachineError;

    async fn store_event(&mut self, id: Self::MachineId, event: Self::Event) -> Result<(), Self::Error> {
        self.events.entry(id).or_insert_with(Vec::new).push(event);
        Ok(())
    }

    async fn get_unfinished(&self) -> Result<Vec<Self::MachineId>, Self::Error> {
        Ok(self.events.keys().copied().collect())
    }

    async fn mark_finished(&mut self, _id: Self::MachineId) -> Result<(), Self::Error> { Ok(()) }
}

/// Represents the state machine for taker's side of the Trading Protocol Upgrade swap (v2).
pub struct TakerSwapStateMachine<MakerCoin, TakerCoin> {
    /// MM2 context.
    pub ctx: MmArc,
    /// Storage.
    pub storage: DummyTakerSwapStorage,
    /// The timestamp when the swap was started.
    pub started_at: u64,
    /// The duration of HTLC timelock in seconds.
    pub lock_duration: u64,
    /// Maker coin.
    pub maker_coin: MakerCoin,
    /// The amount swapped by maker.
    pub maker_volume: MmNumber,
    /// Taker coin.
    pub taker_coin: TakerCoin,
    /// The amount swapped by taker.
    pub taker_volume: MmNumber,
    /// DEX fee amount.
    pub dex_fee: MmNumber,
    /// Premium amount, which might be paid to maker as additional reward.
    pub taker_premium: MmNumber,
    /// Swap transactions' confirmations settings.
    pub conf_settings: SwapConfirmationsSettings,
    /// UUID of the swap.
    pub uuid: Uuid,
    /// The gossipsub topic used for peer-to-peer communication in swap process.
    pub p2p_topic: String,
    /// If Some, used to sign P2P messages of this swap.
    pub p2p_keypair: Option<KeyPair>,
}

impl<MakerCoin, TakerCoin> TakerSwapStateMachine<MakerCoin, TakerCoin> {
    fn maker_payment_conf_timeout(&self) -> u64 { self.started_at + self.lock_duration * 2 / 3 }

    fn taker_payment_locktime(&self) -> u64 { self.started_at + self.lock_duration }

    fn unique_data(&self) -> Vec<u8> { self.uuid.as_bytes().to_vec() }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableStateMachine
    for TakerSwapStateMachine<MakerCoin, TakerCoin>
{
    type Storage = DummyTakerSwapStorage;
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

/// Represents a state used to start a new taker swap.
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
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;
}

#[async_trait]
impl<MakerCoin: MmCoin + SwapOpsV2 + Send + Sync + 'static, TakerCoin: MmCoin + SwapOpsV2 + Send + Sync + 'static> State
    for Initialize<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

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

        if let Err(e) = check_balance_for_taker_swap(
            &state_machine.ctx,
            &state_machine.taker_coin,
            &state_machine.maker_coin,
            state_machine.taker_volume.clone(),
            Some(&state_machine.uuid),
            None,
            FeeApproxStage::StartSwap,
        )
        .await
        {
            return Self::change_state(Aborted::new(e.to_string()), state_machine).await;
        }

        info!("Taker swap {} has successfully started", state_machine.uuid);
        let next_state = Initialized {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block,
            taker_coin_start_block,
        };
        Self::change_state(next_state, state_machine).await
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
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::Initialized {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
        }
    }
}

#[async_trait]
impl<MakerCoin: MmCoin + Send + Sync + 'static, TakerCoin: MmCoin + SwapOpsV2 + Send + Sync + 'static> State
    for Initialized<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let recv_fut = recv_swap_v2_msg(
            state_machine.ctx.clone(),
            |store| store.maker_negotiation.take(),
            &state_machine.uuid,
            NEGOTIATION_TIMEOUT_SEC,
        );

        let maker_negotiation = match recv_fut.await {
            Ok(d) => d,
            Err(e) => {
                let next_state = Aborted::new(format!("Failed to receive MakerNegotiation: {}", e));
                return Self::change_state(next_state, state_machine).await;
            },
        };

        debug!("Received maker negotiation message {:?}", maker_negotiation);

        let unique_data = state_machine.unique_data();
        let taker_negotiation = TakerNegotiation {
            action: Some(taker_negotiation::Action::Continue(TakerNegotiationData {
                started_at: state_machine.started_at,
                payment_locktime: state_machine.taker_payment_locktime(),
                maker_coin_htlc_pub: state_machine.maker_coin.derive_htlc_pubkey(&unique_data),
                taker_coin_htlc_pub: state_machine.taker_coin.derive_htlc_pubkey(&unique_data),
                maker_coin_swap_contract: state_machine.maker_coin.swap_contract_address().map(|bytes| bytes.0),
                taker_coin_swap_contract: state_machine.taker_coin.swap_contract_address().map(|bytes| bytes.0),
            })),
        };

        let swap_msg = SwapMessage {
            inner: Some(swap_message::Inner::TakerNegotiation(taker_negotiation)),
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
            |store| store.maker_negotiated.take(),
            &state_machine.uuid,
            NEGOTIATION_TIMEOUT_SEC,
        );

        let maker_negotiated = match recv_fut.await {
            Ok(d) => d,
            Err(e) => {
                let next_state = Aborted::new(format!("Failed to receive MakerNegotiated: {}", e));
                return Self::change_state(next_state, state_machine).await;
            },
        };
        drop(abort_handle);

        debug!("Received maker negotiated message {:?}", maker_negotiated);
        if !maker_negotiated.negotiated {
            let next_state = Aborted::new(format!(
                "Maker did not negotiate with the reason: {}",
                maker_negotiated.reason.unwrap_or_default()
            ));
            return Self::change_state(next_state, state_machine).await;
        }

        let next_state = Negotiated {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            secret_hash: maker_negotiation.secret_hash,
            maker_payment_locktime: maker_negotiation.payment_locktime,
            maker_coin_htlc_pub_from_maker: maker_negotiation.maker_coin_htlc_pub,
            taker_coin_htlc_pub_from_maker: maker_negotiation.taker_coin_htlc_pub,
            maker_coin_swap_contract: maker_negotiation.maker_coin_swap_contract,
            taker_coin_swap_contract: maker_negotiation.taker_coin_swap_contract,
        };
        Self::change_state(next_state, state_machine).await
    }
}

struct Negotiated<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    secret_hash: Vec<u8>,
    maker_payment_locktime: u64,
    maker_coin_htlc_pub_from_maker: Vec<u8>,
    taker_coin_htlc_pub_from_maker: Vec<u8>,
    maker_coin_swap_contract: Option<Vec<u8>>,
    taker_coin_swap_contract: Option<Vec<u8>>,
}

impl<MakerCoin, TakerCoin> TransitionFrom<Initialized<MakerCoin, TakerCoin>> for Negotiated<MakerCoin, TakerCoin> {}

#[async_trait]
impl<MakerCoin: MmCoin, TakerCoin: MmCoin + SwapOpsV2> State for Negotiated<MakerCoin, TakerCoin> {
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let args = SendCombinedTakerPaymentArgs {
            time_lock: state_machine.taker_payment_locktime(),
            secret_hash: &self.secret_hash,
            other_pub: &self.taker_coin_htlc_pub_from_maker,
            dex_fee_amount: state_machine.dex_fee.to_decimal(),
            premium_amount: BigDecimal::from(0),
            trading_amount: state_machine.taker_volume.to_decimal(),
            swap_unique_data: &state_machine.unique_data(),
        };

        let taker_payment = match state_machine.taker_coin.send_combined_taker_payment(args).await {
            Ok(tx) => tx,
            Err(e) => {
                let next_state = Aborted::new(format!("Failed to send taker payment {:?}", e));
                return Self::change_state(next_state, state_machine).await;
            },
        };
        info!(
            "Sent combined taker payment {} tx {:02x} during swap {}",
            state_machine.taker_coin.ticker(),
            taker_payment.tx_hash(),
            state_machine.uuid
        );

        let next_state = TakerPaymentSent {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            taker_payment: TransactionIdentifier {
                tx_hex: taker_payment.tx_hex().into(),
                tx_hash: taker_payment.tx_hash(),
            },
            secret_hash: self.secret_hash,
            maker_payment_locktime: self.maker_payment_locktime,
            maker_coin_htlc_pub_from_maker: self.maker_coin_htlc_pub_from_maker,
            taker_coin_htlc_pub_from_maker: self.taker_coin_htlc_pub_from_maker,
            maker_coin_swap_contract: self.maker_coin_swap_contract,
            taker_coin_swap_contract: self.taker_coin_swap_contract,
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState for Negotiated<MakerCoin, TakerCoin> {
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::Negotiated {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            secret_hash: Default::default(),
        }
    }
}

struct TakerPaymentSent<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    taker_payment: TransactionIdentifier,
    secret_hash: Vec<u8>,
    maker_payment_locktime: u64,
    maker_coin_htlc_pub_from_maker: Vec<u8>,
    taker_coin_htlc_pub_from_maker: Vec<u8>,
    maker_coin_swap_contract: Option<Vec<u8>>,
    taker_coin_swap_contract: Option<Vec<u8>>,
}

impl<MakerCoin, TakerCoin> TransitionFrom<Negotiated<MakerCoin, TakerCoin>> for TakerPaymentSent<MakerCoin, TakerCoin> {}

#[async_trait]
impl<MakerCoin: MmCoin, TakerCoin: MmCoin + SwapOpsV2> State for TakerPaymentSent<MakerCoin, TakerCoin> {
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let taker_payment_info = TakerPaymentInfo {
            tx_bytes: self.taker_payment.tx_hex.clone().0,
            next_step_instructions: None,
        };
        let swap_msg = SwapMessage {
            inner: Some(swap_message::Inner::TakerPaymentInfo(taker_payment_info)),
        };
        let abort_handle = broadcast_swap_v2_msg_every(
            state_machine.ctx.clone(),
            state_machine.p2p_topic.clone(),
            swap_msg,
            600.,
            state_machine.p2p_keypair,
        );

        let recv_fut = recv_swap_v2_msg(
            state_machine.ctx.clone(),
            |store| store.maker_payment.take(),
            &state_machine.uuid,
            NEGOTIATION_TIMEOUT_SEC,
        );

        let maker_payment_info = match recv_fut.await {
            Ok(p) => p,
            Err(e) => {
                let next_state = TakerPaymentRefundRequired {
                    maker_coin: Default::default(),
                    taker_coin: Default::default(),
                    taker_payment: self.taker_payment,
                    secret_hash: self.secret_hash,
                    reason: TakerPaymentRefundReason::DidNotReceiveMakerPayment(e),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };
        drop(abort_handle);
        debug!("Received maker payment info message {:?}", maker_payment_info);

        let input = ConfirmPaymentInput {
            payment_tx: maker_payment_info.tx_bytes.clone(),
            confirmations: state_machine.conf_settings.taker_coin_confs,
            requires_nota: state_machine.conf_settings.taker_coin_nota,
            wait_until: state_machine.maker_payment_conf_timeout(),
            check_every: 10,
        };

        if let Err(e) = state_machine.maker_coin.wait_for_confirmations(input).compat().await {
            let next_state = TakerPaymentRefundRequired {
                maker_coin: Default::default(),
                taker_coin: Default::default(),
                taker_payment: self.taker_payment,
                secret_hash: self.secret_hash,
                reason: TakerPaymentRefundReason::MakerPaymentNotConfirmedInTime(e),
            };
            return Self::change_state(next_state, state_machine).await;
        }

        let next_state = MakerPaymentConfirmed {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: TransactionIdentifier {
                tx_hex: maker_payment_info.tx_bytes.into(),
                tx_hash: Default::default(),
            },
            taker_payment: self.taker_payment,
            secret_hash: self.secret_hash,
            maker_payment_locktime: self.maker_payment_locktime,
            maker_coin_htlc_pub_from_maker: self.maker_coin_htlc_pub_from_maker,
            taker_coin_htlc_pub_from_maker: self.taker_coin_htlc_pub_from_maker,
            maker_coin_swap_contract: self.maker_coin_swap_contract,
            taker_coin_swap_contract: self.taker_coin_swap_contract,
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState for TakerPaymentSent<MakerCoin, TakerCoin> {
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::TakerPaymentSent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            taker_payment: self.taker_payment.clone(),
            secret_hash: self.secret_hash.clone().into(),
        }
    }
}

#[derive(Debug)]
enum TakerPaymentRefundReason {
    DidNotReceiveMakerPayment(String),
    MakerPaymentNotConfirmedInTime(String),
    FailedToGenerateSpendPreimage(String),
    MakerDidNotSpendInTime(String),
}

struct TakerPaymentRefundRequired<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    taker_payment: TransactionIdentifier,
    secret_hash: Vec<u8>,
    reason: TakerPaymentRefundReason,
}

impl<MakerCoin, TakerCoin> TransitionFrom<TakerPaymentSent<MakerCoin, TakerCoin>>
    for TakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
}
impl<MakerCoin, TakerCoin> TransitionFrom<MakerPaymentConfirmed<MakerCoin, TakerCoin>>
    for TakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: Send + Sync + 'static> State
    for TakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        warn!(
            "Entered TakerPaymentRefundRequired state for swap {} with reason {:?}",
            state_machine.uuid, self.reason
        );
        unimplemented!()
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState
    for TakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::TakerPaymentRefundRequired {
            taker_payment: self.taker_payment.clone(),
            secret_hash: self.secret_hash.clone().into(),
        }
    }
}

struct MakerPaymentConfirmed<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    maker_payment: TransactionIdentifier,
    taker_payment: TransactionIdentifier,
    secret_hash: Vec<u8>,
    maker_payment_locktime: u64,
    maker_coin_htlc_pub_from_maker: Vec<u8>,
    taker_coin_htlc_pub_from_maker: Vec<u8>,
    maker_coin_swap_contract: Option<Vec<u8>>,
    taker_coin_swap_contract: Option<Vec<u8>>,
}

impl<MakerCoin, TakerCoin> TransitionFrom<TakerPaymentSent<MakerCoin, TakerCoin>>
    for MakerPaymentConfirmed<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin, TakerCoin: MmCoin + SwapOpsV2> State for MakerPaymentConfirmed<MakerCoin, TakerCoin> {
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let unique_data = state_machine.unique_data();

        let args = GenTakerPaymentSpendArgs {
            taker_tx: &self.taker_payment.tx_hex.0,
            time_lock: state_machine.taker_payment_locktime(),
            secret_hash: &self.secret_hash,
            maker_pub: &self.maker_coin_htlc_pub_from_maker,
            taker_pub: &state_machine.taker_coin.derive_htlc_pubkey(&unique_data),
            dex_fee_pub: &DEX_FEE_ADDR_RAW_PUBKEY,
            dex_fee_amount: state_machine.dex_fee.to_decimal(),
            premium_amount: Default::default(),
            trading_amount: state_machine.taker_volume.to_decimal(),
        };

        let preimage = match state_machine
            .taker_coin
            .gen_taker_payment_spend_preimage(&args, &unique_data)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                let next_state = TakerPaymentRefundRequired {
                    maker_coin: Default::default(),
                    taker_coin: Default::default(),
                    taker_payment: self.taker_payment,
                    secret_hash: self.secret_hash,
                    reason: TakerPaymentRefundReason::FailedToGenerateSpendPreimage(e.to_string()),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };

        let preimage_msg = TakerPaymentSpendPreimage {
            signature: preimage.signature,
            tx_preimage: if !preimage.preimage.is_empty() {
                Some(preimage.preimage)
            } else {
                None
            },
        };
        let swap_msg = SwapMessage {
            inner: Some(swap_message::Inner::TakerPaymentSpendPreimage(preimage_msg)),
        };

        let _abort_handle = broadcast_swap_v2_msg_every(
            state_machine.ctx.clone(),
            state_machine.p2p_topic.clone(),
            swap_msg,
            600.,
            state_machine.p2p_keypair,
        );

        let wait_args = WaitForHTLCTxSpendArgs {
            tx_bytes: &self.taker_payment.tx_hex.0,
            secret_hash: &self.secret_hash,
            wait_until: state_machine.taker_payment_locktime(),
            from_block: self.taker_coin_start_block,
            swap_contract_address: &self.taker_coin_swap_contract.clone().map(|bytes| bytes.into()),
            check_every: 10.0,
            watcher_reward: false,
        };
        let taker_payment_spend = match state_machine
            .taker_coin
            .wait_for_htlc_tx_spend(wait_args)
            .compat()
            .await
        {
            Ok(tx) => tx,
            Err(e) => {
                let next_state = TakerPaymentRefundRequired {
                    maker_coin: Default::default(),
                    taker_coin: Default::default(),
                    taker_payment: self.taker_payment,
                    secret_hash: self.secret_hash,
                    reason: TakerPaymentRefundReason::MakerDidNotSpendInTime(format!("{:?}", e)),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };
        info!(
            "Found taker payment spend {} tx {:02x} during swap {}",
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
            secret_hash: self.secret_hash,
            maker_payment_locktime: self.maker_payment_locktime,
            maker_coin_htlc_pub_from_maker: self.maker_coin_htlc_pub_from_maker,
            taker_coin_htlc_pub_from_maker: self.taker_coin_htlc_pub_from_maker,
            maker_coin_swap_contract: self.maker_coin_swap_contract,
            taker_coin_swap_contract: self.taker_coin_swap_contract,
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState
    for MakerPaymentConfirmed<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::BothPaymentsSentAndConfirmed {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment.clone(),
            taker_payment: self.taker_payment.clone(),
            secret_hash: self.secret_hash.clone().into(),
        }
    }
}

#[allow(dead_code)]
struct TakerPaymentSpent<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    maker_payment: TransactionIdentifier,
    taker_payment: TransactionIdentifier,
    taker_payment_spend: TransactionIdentifier,
    secret_hash: Vec<u8>,
    maker_payment_locktime: u64,
    maker_coin_htlc_pub_from_maker: Vec<u8>,
    taker_coin_htlc_pub_from_maker: Vec<u8>,
    maker_coin_swap_contract: Option<Vec<u8>>,
    taker_coin_swap_contract: Option<Vec<u8>>,
}

impl<MakerCoin, TakerCoin> TransitionFrom<MakerPaymentConfirmed<MakerCoin, TakerCoin>>
    for TakerPaymentSpent<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + Send + Sync + 'static, TakerCoin: MmCoin + SwapOpsV2> State
    for TakerPaymentSpent<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let secret = match state_machine
            .taker_coin
            .extract_secret(&self.secret_hash, &self.taker_payment_spend.tx_hex.0, false)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                let next_state = Aborted::new(format!("Couldn't extract secret from taker payment spend {}", e));
                return Self::change_state(next_state, state_machine).await;
            },
        };

        let args = SpendPaymentArgs {
            other_payment_tx: &self.maker_payment.tx_hex.0,
            time_lock: self.maker_payment_locktime,
            other_pubkey: &self.maker_coin_htlc_pub_from_maker,
            secret: &secret,
            secret_hash: &self.secret_hash,
            swap_contract_address: &self.maker_coin_swap_contract.clone().map(|bytes| bytes.into()),
            swap_unique_data: &state_machine.unique_data(),
            watcher_reward: false,
        };
        let maker_payment_spend = match state_machine
            .maker_coin
            .send_taker_spends_maker_payment(args)
            .compat()
            .await
        {
            Ok(tx) => tx,
            Err(e) => {
                let next_state = Aborted::new(format!("Failed to spend maker payment {:?}", e));
                return Self::change_state(next_state, state_machine).await;
            },
        };
        info!(
            "Spent maker payment {} tx {:02x} during swap {}",
            state_machine.maker_coin.ticker(),
            maker_payment_spend.tx_hash(),
            state_machine.uuid
        );
        let next_state = MakerPaymentSpent {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment,
            taker_payment: self.taker_payment,
            taker_payment_spend: self.taker_payment_spend,
            maker_payment_spend: TransactionIdentifier {
                tx_hex: maker_payment_spend.tx_hex().into(),
                tx_hash: maker_payment_spend.tx_hash(),
            },
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState for TakerPaymentSpent<MakerCoin, TakerCoin> {
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::TakerPaymentSpent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment.clone(),
            taker_payment: self.taker_payment.clone(),
            taker_payment_spend: self.taker_payment_spend.clone(),
            secret: Vec::new().into(),
        }
    }
}

struct MakerPaymentSpent<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    maker_payment: TransactionIdentifier,
    taker_payment: TransactionIdentifier,
    taker_payment_spend: TransactionIdentifier,
    maker_payment_spend: TransactionIdentifier,
}

impl<MakerCoin, TakerCoin> TransitionFrom<TakerPaymentSpent<MakerCoin, TakerCoin>>
    for MakerPaymentSpent<MakerCoin, TakerCoin>
{
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState for MakerPaymentSpent<MakerCoin, TakerCoin> {
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::MakerPaymentSpent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment.clone(),
            taker_payment: self.taker_payment.clone(),
            taker_payment_spend: self.taker_payment_spend.clone(),
            maker_payment_spend: self.maker_payment_spend.clone(),
        }
    }
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: Send + Sync + 'static> State
    for MakerPaymentSpent<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        Self::change_state(Completed::new(), state_machine).await
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
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(
        self: Box<Self>,
        state_machine: &mut Self::StateMachine,
    ) -> <Self::StateMachine as StateMachineTrait>::Result {
        warn!("Swap {} was aborted with reason {}", state_machine.uuid, self.reason);
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState for Aborted<MakerCoin, TakerCoin> {
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::Aborted {
            reason: self.reason.clone(),
        }
    }
}

impl<MakerCoin, TakerCoin> TransitionFrom<Initialize<MakerCoin, TakerCoin>> for Aborted<MakerCoin, TakerCoin> {}
impl<MakerCoin, TakerCoin> TransitionFrom<Initialized<MakerCoin, TakerCoin>> for Aborted<MakerCoin, TakerCoin> {}
impl<MakerCoin, TakerCoin> TransitionFrom<Negotiated<MakerCoin, TakerCoin>> for Aborted<MakerCoin, TakerCoin> {}
impl<MakerCoin, TakerCoin> TransitionFrom<TakerPaymentSpent<MakerCoin, TakerCoin>> for Aborted<MakerCoin, TakerCoin> {}

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
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::Completed
    }
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: Send + Sync + 'static> LastState for Completed<MakerCoin, TakerCoin> {
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(
        self: Box<Self>,
        state_machine: &mut Self::StateMachine,
    ) -> <Self::StateMachine as StateMachineTrait>::Result {
        info!("Swap {} has been completed", state_machine.uuid);
    }
}

impl<MakerCoin, TakerCoin> TransitionFrom<MakerPaymentSpent<MakerCoin, TakerCoin>> for Completed<MakerCoin, TakerCoin> {}
