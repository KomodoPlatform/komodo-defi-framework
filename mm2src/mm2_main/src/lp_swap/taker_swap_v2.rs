use super::{NEGOTIATE_SEND_INTERVAL, NEGOTIATION_TIMEOUT_SEC};
use crate::mm2::database::my_swaps::{get_swap_events, insert_new_swap_v2, set_swap_is_finished, update_swap_events};
use crate::mm2::lp_network::subscribe_to_topic;
use crate::mm2::lp_swap::swap_v2_pb::*;
use crate::mm2::lp_swap::{broadcast_swap_v2_msg_every, check_balance_for_taker_swap, recv_swap_v2_msg, SecretHashAlgo,
                          SwapConfirmationsSettings, SwapsContext, TransactionIdentifier, MAX_STARTED_AT_DIFF,
                          TAKER_SWAP_V2_TYPE};
use async_trait::async_trait;
use bitcrypto::{dhash160, sha256};
use coins::{CoinAssocTypes, ConfirmPaymentInput, FeeApproxStage, GenTakerFundingSpendArgs, GenTakerPaymentSpendArgs,
            MmCoin, SendTakerFundingArgs, SpendPaymentArgs, SwapOps, SwapOpsV2, ToBytes, Transaction,
            TxPreimageWithSig, ValidatePaymentInput, WaitForHTLCTxSpendArgs};
use common::log::{debug, info, warn};
use common::{bits256, Future01CompatExt, DEX_FEE_ADDR_RAW_PUBKEY};
use db_common::sqlite::rusqlite::named_params;
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
    maker_payment_locktime: u64,
    maker_secret_hash: BytesJson,
    maker_coin_htlc_pub_from_maker: BytesJson,
    taker_coin_htlc_pub_from_maker: BytesJson,
    maker_coin_swap_contract: Option<BytesJson>,
    taker_coin_swap_contract: Option<BytesJson>,
}

/// Represents events produced by taker swap states.
#[derive(Debug, Deserialize, Serialize)]
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
        negotiation_data: StoredNegotiationData,
    },
    /// Sent taker funding tx.
    TakerFundingSent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        negotiation_data: StoredNegotiationData,
        taker_funding: TransactionIdentifier,
    },
    /// Taker funding tx refund is required.
    TakerFundingRefundRequired {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        negotiation_data: StoredNegotiationData,
        taker_funding: TransactionIdentifier,
        reason: TakerFundingRefundReason,
    },
    /// Received maker payment
    MakerPaymentReceived {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        negotiation_data: StoredNegotiationData,
        taker_funding: TransactionIdentifier,
        maker_payment: TransactionIdentifier,
    },
    /// Sent taker payment.
    TakerPaymentSent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        taker_payment: TransactionIdentifier,
        negotiation_data: StoredNegotiationData,
    },
    /// Something went wrong, so taker payment refund is required.
    TakerPaymentRefundRequired {
        taker_payment: TransactionIdentifier,
        negotiation_data: StoredNegotiationData,
    },
    /// Maker payment is confirmed on-chain
    MakerPaymentConfirmed {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        maker_payment: TransactionIdentifier,
        taker_payment: TransactionIdentifier,
        negotiation_data: StoredNegotiationData,
    },
    /// Maker spent taker's payment and taker discovered the tx on-chain.
    TakerPaymentSpent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        maker_payment: TransactionIdentifier,
        taker_payment: TransactionIdentifier,
        taker_payment_spend: TransactionIdentifier,
        negotiation_data: StoredNegotiationData,
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
    Aborted { reason: AbortReason },
    /// Swap completed successfully.
    Completed,
}

/// Represents errors that can be produced by [`TakerSwapStateMachine`] run.
#[derive(Debug, Display)]
pub enum TakerSwapStateMachineError {
    StorageError(String),
    SerdeError(String),
}

/// Dummy storage for taker swap events (used temporary).
pub struct DummyTakerSwapStorage {
    ctx: MmArc,
}

impl DummyTakerSwapStorage {
    pub fn new(ctx: MmArc) -> Self { DummyTakerSwapStorage { ctx } }
}

#[async_trait]
impl StateMachineStorage for DummyTakerSwapStorage {
    type MachineId = Uuid;
    type Event = TakerSwapEvent;
    type Error = MmError<TakerSwapStateMachineError>;

    async fn store_event(&mut self, id: Self::MachineId, event: Self::Event) -> Result<(), Self::Error> {
        let id_str = id.to_string();
        let events_json = get_swap_events(&self.ctx.sqlite_connection(), &id_str)
            .map_to_mm(|e| TakerSwapStateMachineError::StorageError(e.to_string()))?;
        let mut events: Vec<TakerSwapEvent> =
            serde_json::from_str(&events_json).map_to_mm(|e| TakerSwapStateMachineError::SerdeError(e.to_string()))?;
        events.push(event);
        drop_mutability!(events);
        let serialized_events =
            serde_json::to_string(&events).map_to_mm(|e| TakerSwapStateMachineError::SerdeError(e.to_string()))?;
        update_swap_events(&self.ctx.sqlite_connection(), &id_str, &serialized_events)
            .map_to_mm(|e| TakerSwapStateMachineError::StorageError(e.to_string()))?;
        Ok(())
    }

    async fn get_unfinished(&self) -> Result<Vec<Self::MachineId>, Self::Error> { todo!() }

    async fn mark_finished(&mut self, id: Self::MachineId) -> Result<(), Self::Error> {
        set_swap_is_finished(&self.ctx.sqlite_connection(), &id.to_string())
            .map_to_mm(|e| TakerSwapStateMachineError::StorageError(e.to_string()))
    }
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
    /// Algorithm used to hash swap secrets.
    pub secret_hash_algo: SecretHashAlgo,
    /// Swap transactions' confirmations settings.
    pub conf_settings: SwapConfirmationsSettings,
    /// UUID of the swap.
    pub uuid: Uuid,
    /// The gossipsub topic used for peer-to-peer communication in swap process.
    pub p2p_topic: String,
    /// If Some, used to sign P2P messages of this swap.
    pub p2p_keypair: Option<KeyPair>,
    /// The secret used for immediate taker funding tx reclaim if maker back-outs
    pub taker_secret: H256,
}

impl<MakerCoin, TakerCoin> TakerSwapStateMachine<MakerCoin, TakerCoin> {
    fn maker_payment_conf_timeout(&self) -> u64 { self.started_at + self.lock_duration * 2 / 3 }

    fn taker_funding_locktime(&self) -> u64 { self.started_at + self.lock_duration * 3 }

    fn taker_payment_locktime(&self) -> u64 { self.started_at + self.lock_duration }

    fn unique_data(&self) -> Vec<u8> { self.uuid.as_bytes().to_vec() }

    /// Returns secret hash generated using selected [SecretHashAlgo].
    fn taker_secret_hash(&self) -> Vec<u8> {
        match self.secret_hash_algo {
            SecretHashAlgo::DHASH160 => dhash160(self.taker_secret.as_slice()).take().into(),
            SecretHashAlgo::SHA256 => sha256(self.taker_secret.as_slice()).take().into(),
        }
    }
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
        {
            let sql_params = named_params! {
                ":my_coin": state_machine.taker_coin.ticker(),
                ":other_coin": state_machine.maker_coin.ticker(),
                ":uuid": state_machine.uuid.to_string(),
                ":started_at": state_machine.started_at,
                ":swap_type": TAKER_SWAP_V2_TYPE,
                ":maker_volume": state_machine.maker_volume.to_fraction_string(),
                ":taker_volume": state_machine.taker_volume.to_fraction_string(),
                ":premium": state_machine.taker_premium.to_fraction_string(),
                ":dex_fee": state_machine.dex_fee.to_fraction_string(),
                ":secret": state_machine.taker_secret.take(),
                ":secret_hash": state_machine.taker_secret_hash(),
                ":secret_hash_algo": state_machine.secret_hash_algo as u8,
                ":p2p_privkey": state_machine.p2p_keypair.map(|k| k.private_bytes()).unwrap_or_default(),
                ":lock_duration": state_machine.lock_duration,
                ":maker_coin_confs": state_machine.conf_settings.maker_coin_confs,
                ":maker_coin_nota": state_machine.conf_settings.maker_coin_nota,
                ":taker_coin_confs": state_machine.conf_settings.taker_coin_confs,
                ":taker_coin_nota": state_machine.conf_settings.taker_coin_nota
            };
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
            let reason = AbortReason::BalanceCheckFailure(e.to_string());
            return Self::change_state(Aborted::new(reason), state_machine).await;
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
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State for Initialized<MakerCoin, TakerCoin> {
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
                let reason = AbortReason::DidNotReceiveMakerNegotiation(e);
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };

        debug!("Received maker negotiation message {:?}", maker_negotiation);

        let started_at_diff = state_machine.started_at.abs_diff(maker_negotiation.started_at);
        if started_at_diff > MAX_STARTED_AT_DIFF {
            let reason = AbortReason::TooLargeStartedAtDiff(started_at_diff);
            return Self::change_state(Aborted::new(reason), state_machine).await;
        }

        if !(maker_negotiation.secret_hash.len() == 20 || maker_negotiation.secret_hash.len() == 32) {
            let reason = AbortReason::SecretHashUnexpectedLen(maker_negotiation.secret_hash.len());
            return Self::change_state(Aborted::new(reason), state_machine).await;
        }

        let expected_maker_payment_locktime = maker_negotiation.started_at + 2 * state_machine.lock_duration;
        if maker_negotiation.payment_locktime != expected_maker_payment_locktime {
            let reason = AbortReason::MakerProvidedInvalidLocktime(maker_negotiation.payment_locktime);
            return Self::change_state(Aborted::new(reason), state_machine).await;
        }

        let maker_coin_htlc_pub_from_maker = match state_machine
            .maker_coin
            .parse_pubkey(&maker_negotiation.maker_coin_htlc_pub)
        {
            Ok(p) => p,
            Err(e) => {
                let reason = AbortReason::FailedToParsePubkey(e.to_string());
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };

        let taker_coin_htlc_pub_from_maker = match state_machine
            .taker_coin
            .parse_pubkey(&maker_negotiation.taker_coin_htlc_pub)
        {
            Ok(p) => p,
            Err(e) => {
                let reason = AbortReason::FailedToParsePubkey(e.to_string());
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };

        let unique_data = state_machine.unique_data();
        let taker_negotiation = TakerNegotiation {
            action: Some(taker_negotiation::Action::Continue(TakerNegotiationData {
                started_at: state_machine.started_at,
                funding_locktime: state_machine.taker_funding_locktime(),
                payment_locktime: state_machine.taker_payment_locktime(),
                taker_secret_hash: state_machine.taker_secret_hash(),
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
                let reason = AbortReason::DidNotReceiveMakerNegotiated(e);
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };
        drop(abort_handle);

        debug!("Received maker negotiated message {:?}", maker_negotiated);
        if !maker_negotiated.negotiated {
            let reason = AbortReason::MakerDidNotNegotiate(maker_negotiated.reason.unwrap_or_default());
            return Self::change_state(Aborted::new(reason), state_machine).await;
        }

        let next_state = Negotiated {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: NegotiationData {
                maker_secret_hash: maker_negotiation.secret_hash,
                maker_payment_locktime: expected_maker_payment_locktime,
                maker_coin_htlc_pub_from_maker,
                taker_coin_htlc_pub_from_maker,
                maker_coin_swap_contract: maker_negotiation.maker_coin_swap_contract,
                taker_coin_swap_contract: maker_negotiation.taker_coin_swap_contract,
            },
        };
        Self::change_state(next_state, state_machine).await
    }
}

struct NegotiationData<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_secret_hash: Vec<u8>,
    maker_payment_locktime: u64,
    maker_coin_htlc_pub_from_maker: MakerCoin::Pubkey,
    taker_coin_htlc_pub_from_maker: TakerCoin::Pubkey,
    maker_coin_swap_contract: Option<Vec<u8>>,
    taker_coin_swap_contract: Option<Vec<u8>>,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> NegotiationData<MakerCoin, TakerCoin> {
    fn to_stored_data(&self) -> StoredNegotiationData {
        StoredNegotiationData {
            maker_payment_locktime: self.maker_payment_locktime,
            maker_secret_hash: self.maker_secret_hash.clone().into(),
            maker_coin_htlc_pub_from_maker: self.maker_coin_htlc_pub_from_maker.to_bytes().into(),
            taker_coin_htlc_pub_from_maker: self.taker_coin_htlc_pub_from_maker.to_bytes().into(),
            maker_coin_swap_contract: self.maker_coin_swap_contract.clone().map(|b| b.into()),
            taker_coin_swap_contract: self.taker_coin_swap_contract.clone().map(|b| b.into()),
        }
    }
}

struct Negotiated<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: SwapOpsV2> TransitionFrom<Initialized<MakerCoin, TakerCoin>>
    for Negotiated<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State for Negotiated<MakerCoin, TakerCoin> {
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let args = SendTakerFundingArgs {
            time_lock: state_machine.taker_funding_locktime(),
            taker_secret_hash: &state_machine.taker_secret_hash(),
            maker_pub: &self.negotiation_data.taker_coin_htlc_pub_from_maker.to_bytes(),
            dex_fee_amount: state_machine.dex_fee.to_decimal(),
            premium_amount: state_machine.taker_premium.to_decimal(),
            trading_amount: state_machine.taker_volume.to_decimal(),
            swap_unique_data: &state_machine.unique_data(),
        };

        let taker_funding = match state_machine.taker_coin.send_taker_funding(args).await {
            Ok(tx) => tx,
            Err(e) => {
                let reason = AbortReason::FailedToSendTakerFunding(format!("{:?}", e));
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };

        info!(
            "Sent taker funding {} tx {:02x} during swap {}",
            state_machine.taker_coin.ticker(),
            taker_funding.tx_hash(),
            state_machine.uuid
        );

        let next_state = TakerFundingSent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            taker_funding,
            negotiation_data: self.negotiation_data,
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: CoinAssocTypes + Send + 'static, TakerCoin: SwapOpsV2> StorableState
    for Negotiated<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::Negotiated {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data.to_stored_data(),
        }
    }
}

struct TakerFundingSent<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    taker_funding: TakerCoin::Tx,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for TakerFundingSent<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let taker_funding_info = TakerFundingInfo {
            tx_bytes: self.taker_funding.tx_hex(),
            next_step_instructions: None,
        };

        let swap_msg = SwapMessage {
            inner: Some(swap_message::Inner::TakerFundingInfo(taker_funding_info)),
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
                let next_state = TakerFundingRefundRequired {
                    maker_coin_start_block: self.maker_coin_start_block,
                    taker_coin_start_block: self.taker_coin_start_block,
                    taker_funding: self.taker_funding,
                    negotiation_data: self.negotiation_data,
                    reason: TakerFundingRefundReason::DidNotReceiveMakerPayment(e),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };
        drop(abort_handle);

        debug!("Received maker payment info message {:?}", maker_payment_info);

        let preimage_tx = match state_machine
            .taker_coin
            .parse_preimage(&maker_payment_info.funding_preimage_tx)
        {
            Ok(p) => p,
            Err(e) => {
                let next_state = TakerFundingRefundRequired {
                    maker_coin_start_block: self.maker_coin_start_block,
                    taker_coin_start_block: self.taker_coin_start_block,
                    taker_funding: self.taker_funding,
                    negotiation_data: self.negotiation_data,
                    reason: TakerFundingRefundReason::FailedToParseFundingSpendPreimg(e.to_string()),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };

        let preimage_sig = match state_machine
            .taker_coin
            .parse_signature(&maker_payment_info.funding_preimage_sig)
        {
            Ok(p) => p,
            Err(e) => {
                let next_state = TakerFundingRefundRequired {
                    maker_coin_start_block: self.maker_coin_start_block,
                    taker_coin_start_block: self.taker_coin_start_block,
                    taker_funding: self.taker_funding,
                    negotiation_data: self.negotiation_data,
                    reason: TakerFundingRefundReason::FailedToParseFundingSpendSig(e.to_string()),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };

        let next_state = MakerPaymentAndFundingSpendPreimgReceived {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data,
            taker_funding: self.taker_funding,
            funding_spend_preimage: TxPreimageWithSig {
                preimage: preimage_tx,
                signature: preimage_sig,
            },
            maker_payment: TransactionIdentifier {
                tx_hex: maker_payment_info.tx_bytes.into(),
                tx_hash: Default::default(),
            },
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<Negotiated<MakerCoin, TakerCoin>>
    for TakerFundingSent<MakerCoin, TakerCoin>
{
}

impl<MakerCoin: CoinAssocTypes + Send + 'static, TakerCoin: SwapOpsV2> StorableState
    for TakerFundingSent<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::TakerFundingSent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            taker_funding: TransactionIdentifier {
                tx_hex: self.taker_funding.tx_hex().into(),
                tx_hash: self.taker_funding.tx_hash(),
            },
            negotiation_data: self.negotiation_data.to_stored_data(),
        }
    }
}

struct MakerPaymentAndFundingSpendPreimgReceived<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
    taker_funding: TakerCoin::Tx,
    funding_spend_preimage: TxPreimageWithSig<TakerCoin>,
    maker_payment: TransactionIdentifier,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<TakerFundingSent<MakerCoin, TakerCoin>>
    for MakerPaymentAndFundingSpendPreimgReceived<MakerCoin, TakerCoin>
{
}

impl<MakerCoin: CoinAssocTypes + Send + 'static, TakerCoin: SwapOpsV2> StorableState
    for MakerPaymentAndFundingSpendPreimgReceived<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::MakerPaymentReceived {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            negotiation_data: self.negotiation_data.to_stored_data(),
            taker_funding: TransactionIdentifier {
                tx_hex: self.taker_funding.tx_hex().into(),
                tx_hash: self.taker_funding.tx_hash(),
            },
            maker_payment: self.maker_payment.clone(),
        }
    }
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for MakerPaymentAndFundingSpendPreimgReceived<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let unique_data = state_machine.unique_data();

        let input = ValidatePaymentInput {
            payment_tx: self.maker_payment.tx_hex.0.clone(),
            time_lock_duration: state_machine.lock_duration,
            time_lock: self.negotiation_data.maker_payment_locktime,
            other_pub: self.negotiation_data.maker_coin_htlc_pub_from_maker.to_bytes(),
            secret_hash: self.negotiation_data.maker_secret_hash.clone(),
            amount: state_machine.maker_volume.to_decimal(),
            swap_contract_address: None,
            try_spv_proof_until: state_machine.maker_payment_conf_timeout(),
            confirmations: state_machine.conf_settings.maker_coin_confs,
            unique_swap_data: unique_data.clone(),
            watcher_reward: None,
        };
        if let Err(e) = state_machine.maker_coin.validate_maker_payment(input).compat().await {
            let next_state = TakerFundingRefundRequired {
                maker_coin_start_block: self.maker_coin_start_block,
                taker_coin_start_block: self.taker_coin_start_block,
                taker_funding: self.taker_funding,
                negotiation_data: self.negotiation_data,
                reason: TakerFundingRefundReason::MakerPaymentValidationFailed(e.to_string()),
            };
            return Self::change_state(next_state, state_machine).await;
        };

        let args = GenTakerFundingSpendArgs {
            funding_tx: &self.taker_funding,
            maker_pub: &self.negotiation_data.taker_coin_htlc_pub_from_maker,
            taker_pub: &state_machine.taker_coin.derive_htlc_pubkey_v2(&unique_data),
            funding_time_lock: state_machine.taker_funding_locktime(),
            taker_secret_hash: &state_machine.taker_secret_hash(),
            taker_payment_time_lock: state_machine.taker_payment_locktime(),
            maker_secret_hash: &self.negotiation_data.maker_secret_hash,
        };

        if let Err(e) = state_machine
            .taker_coin
            .validate_taker_funding_spend_preimage(&args, &self.funding_spend_preimage)
            .await
        {
            let next_state = TakerFundingRefundRequired {
                maker_coin_start_block: self.maker_coin_start_block,
                taker_coin_start_block: self.taker_coin_start_block,
                taker_funding: self.taker_funding,
                negotiation_data: self.negotiation_data,
                reason: TakerFundingRefundReason::FundingSpendPreimageValidationFailed(format!("{:?}", e)),
            };
            return Self::change_state(next_state, state_machine).await;
        }

        let taker_payment = match state_machine
            .taker_coin
            .sign_and_send_taker_funding_spend(&self.funding_spend_preimage, &args, &unique_data)
            .await
        {
            Ok(tx) => tx,
            Err(e) => {
                let next_state = TakerFundingRefundRequired {
                    maker_coin_start_block: self.maker_coin_start_block,
                    taker_coin_start_block: self.taker_coin_start_block,
                    taker_funding: self.taker_funding,
                    negotiation_data: self.negotiation_data,
                    reason: TakerFundingRefundReason::FailedToSendTakerPayment(format!("{:?}", e)),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };

        info!(
            "Sent taker payment {} tx {:02x} during swap {}",
            state_machine.taker_coin.ticker(),
            taker_payment.tx_hash(),
            state_machine.uuid
        );

        let next_state = TakerPaymentSent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            taker_payment,
            maker_payment: self.maker_payment,
            negotiation_data: self.negotiation_data,
        };
        Self::change_state(next_state, state_machine).await
    }
}

struct TakerPaymentSent<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    taker_payment: TakerCoin::Tx,
    maker_payment: TransactionIdentifier,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes>
    TransitionFrom<MakerPaymentAndFundingSpendPreimgReceived<MakerCoin, TakerCoin>>
    for TakerPaymentSent<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for TakerPaymentSent<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let taker_payment_info = TakerPaymentInfo {
            tx_bytes: self.taker_payment.tx_hex(),
            next_step_instructions: None,
        };
        let swap_msg = SwapMessage {
            inner: Some(swap_message::Inner::TakerPaymentInfo(taker_payment_info)),
        };
        let _abort_handle = broadcast_swap_v2_msg_every(
            state_machine.ctx.clone(),
            state_machine.p2p_topic.clone(),
            swap_msg,
            600.,
            state_machine.p2p_keypair,
        );

        let input = ConfirmPaymentInput {
            payment_tx: self.maker_payment.tx_hex.0.clone(),
            confirmations: state_machine.conf_settings.taker_coin_confs,
            requires_nota: state_machine.conf_settings.taker_coin_nota,
            wait_until: state_machine.maker_payment_conf_timeout(),
            check_every: 10,
        };

        if let Err(e) = state_machine.maker_coin.wait_for_confirmations(input).compat().await {
            let next_state = TakerPaymentRefundRequired {
                taker_payment: self.taker_payment,
                negotiation_data: self.negotiation_data,
                reason: TakerPaymentRefundReason::MakerPaymentNotConfirmedInTime(e),
            };
            return Self::change_state(next_state, state_machine).await;
        }

        let next_state = MakerPaymentConfirmed {
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
    for TakerPaymentSent<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::TakerPaymentSent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            taker_payment: TransactionIdentifier {
                tx_hex: self.taker_payment.tx_hex().into(),
                tx_hash: self.taker_payment.tx_hash(),
            },
            negotiation_data: self.negotiation_data.to_stored_data(),
        }
    }
}

/// Represents the reason taker funding refund
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum TakerFundingRefundReason {
    DidNotReceiveMakerPayment(String),
    FailedToParseFundingSpendPreimg(String),
    FailedToParseFundingSpendSig(String),
    FailedToSendTakerPayment(String),
    MakerPaymentValidationFailed(String),
    FundingSpendPreimageValidationFailed(String),
}

struct TakerFundingRefundRequired<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    taker_funding: TakerCoin::Tx,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
    reason: TakerFundingRefundReason,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<TakerFundingSent<MakerCoin, TakerCoin>>
    for TakerFundingRefundRequired<MakerCoin, TakerCoin>
{
}
impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes>
    TransitionFrom<MakerPaymentAndFundingSpendPreimgReceived<MakerCoin, TakerCoin>>
    for TakerFundingRefundRequired<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: SwapOps + CoinAssocTypes + Send + 'static, TakerCoin: MmCoin + SwapOpsV2> State
    for TakerFundingRefundRequired<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, _state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        todo!()
    }
}

impl<MakerCoin: CoinAssocTypes + Send + 'static, TakerCoin: CoinAssocTypes + Send + 'static> StorableState
    for TakerFundingRefundRequired<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::TakerFundingRefundRequired {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            taker_funding: TransactionIdentifier {
                tx_hex: self.taker_funding.tx_hex().into(),
                tx_hash: self.taker_funding.tx_hash(),
            },
            negotiation_data: self.negotiation_data.to_stored_data(),
            reason: self.reason.clone(),
        }
    }
}

#[derive(Debug)]
enum TakerPaymentRefundReason {
    MakerPaymentNotConfirmedInTime(String),
    FailedToGenerateSpendPreimage(String),
    MakerDidNotSpendInTime(String),
}

struct TakerPaymentRefundRequired<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    taker_payment: TakerCoin::Tx,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
    reason: TakerPaymentRefundReason,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<TakerPaymentSent<MakerCoin, TakerCoin>>
    for TakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
}
impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<MakerPaymentConfirmed<MakerCoin, TakerCoin>>
    for TakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: CoinAssocTypes + Send + Sync + 'static, TakerCoin: CoinAssocTypes + Send + Sync + 'static> State
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

impl<MakerCoin: CoinAssocTypes + Send + 'static, TakerCoin: CoinAssocTypes + Send + 'static> StorableState
    for TakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::TakerPaymentRefundRequired {
            taker_payment: TransactionIdentifier {
                tx_hex: self.taker_payment.tx_hex().into(),
                tx_hash: self.taker_payment.tx_hash(),
            },
            negotiation_data: self.negotiation_data.to_stored_data(),
        }
    }
}

struct MakerPaymentConfirmed<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    maker_payment: TransactionIdentifier,
    taker_payment: TakerCoin::Tx,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<TakerPaymentSent<MakerCoin, TakerCoin>>
    for MakerPaymentConfirmed<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for MakerPaymentConfirmed<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let unique_data = state_machine.unique_data();

        let args = GenTakerPaymentSpendArgs {
            taker_tx: &self.taker_payment,
            time_lock: state_machine.taker_payment_locktime(),
            secret_hash: &self.negotiation_data.maker_secret_hash,
            maker_pub: &self.negotiation_data.taker_coin_htlc_pub_from_maker,
            taker_pub: &state_machine.taker_coin.derive_htlc_pubkey_v2(&unique_data),
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
                    taker_payment: self.taker_payment,
                    negotiation_data: self.negotiation_data,
                    reason: TakerPaymentRefundReason::FailedToGenerateSpendPreimage(e.to_string()),
                };
                return Self::change_state(next_state, state_machine).await;
            },
        };

        let preimage_msg = TakerPaymentSpendPreimage {
            signature: preimage.signature.to_bytes(),
            tx_preimage: preimage.preimage.to_bytes(),
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
            tx_bytes: &self.taker_payment.tx_hex(),
            secret_hash: &self.negotiation_data.maker_secret_hash,
            wait_until: state_machine.taker_payment_locktime(),
            from_block: self.taker_coin_start_block,
            swap_contract_address: &self
                .negotiation_data
                .taker_coin_swap_contract
                .clone()
                .map(|bytes| bytes.into()),
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
                    taker_payment: self.taker_payment,
                    negotiation_data: self.negotiation_data,
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
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment,
            taker_payment: self.taker_payment,
            taker_payment_spend: TransactionIdentifier {
                tx_hex: taker_payment_spend.tx_hex().into(),
                tx_hash: taker_payment_spend.tx_hash(),
            },
            negotiation_data: self.negotiation_data,
        };
        Self::change_state(next_state, state_machine).await
    }
}

impl<MakerCoin: CoinAssocTypes + Send + 'static, TakerCoin: SwapOpsV2> StorableState
    for MakerPaymentConfirmed<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::MakerPaymentConfirmed {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment.clone(),
            taker_payment: TransactionIdentifier {
                tx_hex: self.taker_payment.tx_hex().into(),
                tx_hash: self.taker_payment.tx_hash(),
            },
            negotiation_data: self.negotiation_data.to_stored_data(),
        }
    }
}

#[allow(dead_code)]
struct TakerPaymentSpent<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> {
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    maker_payment: TransactionIdentifier,
    taker_payment: TakerCoin::Tx,
    taker_payment_spend: TransactionIdentifier,
    negotiation_data: NegotiationData<MakerCoin, TakerCoin>,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: CoinAssocTypes> TransitionFrom<MakerPaymentConfirmed<MakerCoin, TakerCoin>>
    for TakerPaymentSpent<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: MmCoin + CoinAssocTypes, TakerCoin: MmCoin + SwapOpsV2> State
    for TakerPaymentSpent<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let secret = match state_machine
            .taker_coin
            .extract_secret(
                &self.negotiation_data.maker_secret_hash,
                &self.taker_payment_spend.tx_hex.0,
                false,
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                let reason = AbortReason::CouldNotExtractSecret(e);
                return Self::change_state(Aborted::new(reason), state_machine).await;
            },
        };

        let args = SpendPaymentArgs {
            other_payment_tx: &self.maker_payment.tx_hex.0,
            time_lock: self.negotiation_data.maker_payment_locktime,
            other_pubkey: &self.negotiation_data.maker_coin_htlc_pub_from_maker.to_bytes(),
            secret: &secret,
            secret_hash: &self.negotiation_data.maker_secret_hash,
            swap_contract_address: &self
                .negotiation_data
                .maker_coin_swap_contract
                .clone()
                .map(|bytes| bytes.into()),
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
                let reason = AbortReason::FailedToSpendMakerPayment(format!("{:?}", e));
                return Self::change_state(Aborted::new(reason), state_machine).await;
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

impl<MakerCoin: CoinAssocTypes + Send + 'static, TakerCoin: SwapOpsV2> StorableState
    for TakerPaymentSpent<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::TakerPaymentSpent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment.clone(),
            taker_payment: TransactionIdentifier {
                tx_hex: self.taker_payment.tx_hex().into(),
                tx_hash: self.taker_payment.tx_hash(),
            },
            taker_payment_spend: self.taker_payment_spend.clone(),
            negotiation_data: self.negotiation_data.to_stored_data(),
        }
    }
}

struct MakerPaymentSpent<MakerCoin, TakerCoin: CoinAssocTypes> {
    maker_coin: PhantomData<MakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    maker_payment: TransactionIdentifier,
    taker_payment: TakerCoin::Tx,
    taker_payment_spend: TransactionIdentifier,
    maker_payment_spend: TransactionIdentifier,
}

impl<MakerCoin: CoinAssocTypes, TakerCoin: SwapOpsV2> TransitionFrom<TakerPaymentSpent<MakerCoin, TakerCoin>>
    for MakerPaymentSpent<MakerCoin, TakerCoin>
{
}

impl<MakerCoin: Send + 'static, TakerCoin: CoinAssocTypes + Send + 'static> StorableState
    for MakerPaymentSpent<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::MakerPaymentSpent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment.clone(),
            taker_payment: TransactionIdentifier {
                tx_hex: self.taker_payment.tx_hex().into(),
                tx_hash: self.taker_payment.tx_hash(),
            },
            taker_payment_spend: self.taker_payment_spend.clone(),
            maker_payment_spend: self.maker_payment_spend.clone(),
        }
    }
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: CoinAssocTypes + Send + Sync + 'static> State
    for MakerPaymentSpent<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, state_machine: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        Self::change_state(Completed::new(), state_machine).await
    }
}

/// Represents possible reasons of taker swap being aborted
#[derive(Clone, Debug, Deserialize, Display, Serialize)]
pub enum AbortReason {
    FailedToGetMakerCoinBlock(String),
    FailedToGetTakerCoinBlock(String),
    BalanceCheckFailure(String),
    DidNotReceiveMakerNegotiation(String),
    TooLargeStartedAtDiff(u64),
    FailedToParsePubkey(String),
    MakerProvidedInvalidLocktime(u64),
    SecretHashUnexpectedLen(usize),
    DidNotReceiveMakerNegotiated(String),
    MakerDidNotNegotiate(String),
    FailedToSendTakerFunding(String),
    CouldNotExtractSecret(String),
    FailedToSpendMakerPayment(String),
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
impl<MakerCoin: CoinAssocTypes, TakerCoin: SwapOpsV2> TransitionFrom<Negotiated<MakerCoin, TakerCoin>>
    for Aborted<MakerCoin, TakerCoin>
{
}
impl<MakerCoin: CoinAssocTypes, TakerCoin: SwapOpsV2> TransitionFrom<TakerPaymentSpent<MakerCoin, TakerCoin>>
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

impl<MakerCoin, TakerCoin: CoinAssocTypes> TransitionFrom<MakerPaymentSpent<MakerCoin, TakerCoin>>
    for Completed<MakerCoin, TakerCoin>
{
}
