use crate::mm2::lp_swap::{check_balance_for_taker_swap, SwapConfirmationsSettings, TransactionIdentifier};
use async_trait::async_trait;
use coins::{CoinBalance, ConfirmPaymentInput, FeeApproxStage, MarketCoinOps, MmCoin, SwapOpsV2, TestCoin, TradeFee};
use common::{block_on, Future01CompatExt};
use keys::KeyPair;
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};
use mm2_number::MmNumber;
use mm2_state_machine::prelude::*;
use mm2_state_machine::storable_state_machine::*;
use rpc::v1::types::Bytes as BytesJson;
use std::collections::HashMap;
use std::marker::PhantomData;
use uuid::Uuid;

#[derive(Debug, PartialEq)]
pub enum TakerSwapEvent {
    Initialized {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
    },
    Negotiated {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        secret_hash: BytesJson,
    },
    TakerPaymentSent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        taker_payment: TransactionIdentifier,
        secret_hash: BytesJson,
    },
    TakerPaymentRefundRequired {
        taker_payment: TransactionIdentifier,
        secret_hash: BytesJson,
    },
    BothPaymentsSentAndConfirmed {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        maker_payment: TransactionIdentifier,
        taker_payment: TransactionIdentifier,
        secret_hash: BytesJson,
    },
    TakerPaymentSpent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        maker_payment: TransactionIdentifier,
        taker_payment: TransactionIdentifier,
        taker_payment_spend: TransactionIdentifier,
        secret: BytesJson,
    },
    MakerPaymentSpent {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
        maker_payment: TransactionIdentifier,
        taker_payment: TransactionIdentifier,
        taker_payment_spend: TransactionIdentifier,
        maker_payment_spend: TransactionIdentifier,
    },
    Aborted {
        reason: String,
    },
    Completed,
}

#[derive(Debug, Display)]
pub enum TakerSwapStateMachineError {}

pub struct DummyTakerSwapStorage {
    events: HashMap<Uuid, Vec<TakerSwapEvent>>,
}

impl DummyTakerSwapStorage {
    pub fn new() -> Self { DummyTakerSwapStorage { events: HashMap::new() } }
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

    async fn mark_finished(&mut self, id: Self::MachineId) -> Result<(), Self::Error> { Ok(()) }
}

pub struct TakerSwapStateMachine<MakerCoin, TakerCoin> {
    pub ctx: MmArc,
    pub storage: DummyTakerSwapStorage,
    pub maker_coin: MakerCoin,
    pub maker_volume: MmNumber,
    pub taker_coin: TakerCoin,
    pub taker_volume: MmNumber,
    pub taker_premium: MmNumber,
    pub conf_settings: SwapConfirmationsSettings,
    pub uuid: Uuid,
    pub p2p_keypair: Option<KeyPair>,
}

impl<MakerCoin, TakerCoin> TakerSwapStateMachine<MakerCoin, TakerCoin> {
    fn maker_payment_conf_timeout(&self) -> u64 { 0 }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableStateMachine
    for TakerSwapStateMachine<MakerCoin, TakerCoin>
{
    type Storage = DummyTakerSwapStorage;
    type Result = ();

    fn storage(&mut self) -> &mut Self::Storage { &mut self.storage }

    fn id(&self) -> <Self::Storage as StateMachineStorage>::MachineId { self.uuid }

    fn restore_from_storage(
        id: <Self::Storage as StateMachineStorage>::MachineId,
        storage: Self::Storage,
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
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;
}

#[async_trait]
impl<MakerCoin: MmCoin + SwapOpsV2 + Send + Sync + 'static, TakerCoin: MmCoin + SwapOpsV2 + Send + Sync + 'static> State
    for Initialize<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let maker_coin_start_block = match ctx.maker_coin.current_block().compat().await {
            Ok(b) => b,
            Err(e) => return Self::change_state(Aborted::new(e), ctx).await,
        };

        let taker_coin_start_block = match ctx.taker_coin.current_block().compat().await {
            Ok(b) => b,
            Err(e) => return Self::change_state(Aborted::new(e), ctx).await,
        };

        if let Err(e) = check_balance_for_taker_swap(
            &ctx.ctx,
            &ctx.taker_coin,
            &ctx.maker_coin,
            ctx.taker_volume.clone(),
            Some(&ctx.uuid),
            None,
            FeeApproxStage::StartSwap,
        )
        .await
        {
            return Self::change_state(Aborted::new(e.to_string()), ctx).await;
        }

        let negotiate = Initialized {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block,
            taker_coin_start_block,
        };
        Self::change_state(negotiate, ctx).await
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
impl<MakerCoin: MarketCoinOps + Send + Sync + 'static, TakerCoin: MarketCoinOps + Send + Sync + 'static> State
    for Initialized<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let next_state = Negotiated {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            secret_hash: Vec::new().into(),
        };
        Self::change_state(next_state, ctx).await
    }
}

struct Negotiated<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    secret_hash: BytesJson,
}

impl<MakerCoin, TakerCoin> TransitionFrom<Initialized<MakerCoin, TakerCoin>> for Negotiated<MakerCoin, TakerCoin> {}

#[async_trait]
impl<MakerCoin: MarketCoinOps + Send + Sync + 'static, TakerCoin: MarketCoinOps + Send + Sync + 'static> State
    for Negotiated<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let next_state = TakerPaymentSent {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            taker_payment: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
            secret_hash: self.secret_hash,
        };
        Self::change_state(next_state, ctx).await
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
    secret_hash: BytesJson,
}

impl<MakerCoin, TakerCoin> TransitionFrom<Negotiated<MakerCoin, TakerCoin>> for TakerPaymentSent<MakerCoin, TakerCoin> {}

#[async_trait]
impl<MakerCoin: MarketCoinOps + Send + Sync + 'static, TakerCoin: Send + Sync + 'static> State
    for TakerPaymentSent<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let maker_payment = TransactionIdentifier {
            tx_hex: Default::default(),
            tx_hash: Default::default(),
        };

        let input = ConfirmPaymentInput {
            payment_tx: maker_payment.tx_hex.0.clone(),
            confirmations: ctx.conf_settings.taker_coin_confs,
            requires_nota: ctx.conf_settings.taker_coin_nota,
            wait_until: ctx.maker_payment_conf_timeout(),
            check_every: 10,
        };
        if let Err(e) = ctx.maker_coin.wait_for_confirmations(input).compat().await {
            let next_state = TakerPaymentRefundRequired {
                maker_coin: Default::default(),
                taker_coin: Default::default(),
                taker_payment: self.taker_payment,
                secret_hash: self.secret_hash,
            };
            return Self::change_state(next_state, ctx).await;
        }

        let next_state = BothPaymentsSentAndConfirmed {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment,
            taker_payment: self.taker_payment,
            secret_hash: self.secret_hash,
        };
        Self::change_state(next_state, ctx).await
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState for TakerPaymentSent<MakerCoin, TakerCoin> {
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::TakerPaymentSent {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            taker_payment: self.taker_payment.clone(),
            secret_hash: self.secret_hash.clone(),
        }
    }
}

struct TakerPaymentRefundRequired<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    taker_payment: TransactionIdentifier,
    secret_hash: BytesJson,
}

impl<MakerCoin, TakerCoin> TransitionFrom<TakerPaymentSent<MakerCoin, TakerCoin>>
    for TakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: Send + Sync + 'static> State
    for TakerPaymentRefundRequired<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
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
            secret_hash: self.secret_hash.clone(),
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
    secret_hash: BytesJson,
}

impl<MakerCoin, TakerCoin> TransitionFrom<TakerPaymentSent<MakerCoin, TakerCoin>>
    for BothPaymentsSentAndConfirmed<MakerCoin, TakerCoin>
{
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: Send + Sync + 'static> State
    for BothPaymentsSentAndConfirmed<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
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
        Self::change_state(next_state, ctx).await
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState
    for BothPaymentsSentAndConfirmed<MakerCoin, TakerCoin>
{
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        TakerSwapEvent::BothPaymentsSentAndConfirmed {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment.clone(),
            taker_payment: self.taker_payment.clone(),
            secret_hash: self.secret_hash.clone(),
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
    type StateMachine = TakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let next_state = MakerPaymentSpent {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment,
            taker_payment: self.taker_payment,
            taker_payment_spend: self.taker_payment_spend,
            maker_payment_spend: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
        };
        Self::change_state(next_state, ctx).await
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

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        Self::change_state(Completed::new(), ctx).await
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
        _ctx: &mut Self::StateMachine,
    ) -> <Self::StateMachine as StateMachineTrait>::Result {
        //TODO just log something here?
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
        _ctx: &mut Self::StateMachine,
    ) -> <Self::StateMachine as StateMachineTrait>::Result {
        //TODO just log something here?
    }
}

impl<MakerCoin, TakerCoin> TransitionFrom<MakerPaymentSpent<MakerCoin, TakerCoin>> for Completed<MakerCoin, TakerCoin> {}

#[test]
fn just_run_it() {
    use mocktopus::mocking::{MockResult, Mockable};
    TestCoin::current_block.mock_safe(|_| MockResult::Return(Box::new(futures01::future::ok(1000))));
    TestCoin::min_tx_amount.mock_safe(|_| MockResult::Return(0.into()));
    TestCoin::get_fee_to_send_taker_fee.mock_safe(|_, _, _| {
        MockResult::Return(Box::pin(futures::future::ok(TradeFee {
            coin: "test".to_string(),
            amount: Default::default(),
            paid_from_trading_vol: false,
        })))
    });
    TestCoin::get_sender_trade_fee.mock_safe(|_, _, _| {
        MockResult::Return(Box::pin(futures::future::ok(TradeFee {
            coin: "test".to_string(),
            amount: Default::default(),
            paid_from_trading_vol: false,
        })))
    });

    TestCoin::get_receiver_trade_fee.mock_safe(|_, _| {
        MockResult::Return(Box::new(futures01::future::ok(TradeFee {
            coin: "test".to_string(),
            amount: Default::default(),
            paid_from_trading_vol: false,
        })))
    });

    TestCoin::my_balance.mock_safe(|_| {
        MockResult::Return(Box::new(futures01::future::ok(CoinBalance {
            spendable: 100.into(),
            unspendable: Default::default(),
        })))
    });

    TestCoin::wait_for_confirmations.mock_safe(|_, _| MockResult::Return(Box::new(futures01::future::ok(()))));

    let ctx = MmCtxBuilder::default().into_mm_arc();
    let uuid = Uuid::default();

    let mut state_machine = TakerSwapStateMachine {
        ctx,
        storage: DummyTakerSwapStorage::new(),
        maker_coin: TestCoin::default(),
        maker_volume: 0.into(),
        taker_coin: TestCoin::default(),
        taker_volume: 0.into(),
        taker_premium: 0.into(),
        conf_settings: SwapConfirmationsSettings {
            maker_coin_confs: 0,
            maker_coin_nota: false,
            taker_coin_confs: 0,
            taker_coin_nota: false,
        },
        uuid,
        p2p_keypair: None,
    };

    block_on(state_machine.run(Box::new(Initialize::new()))).unwrap();

    let expected_events = vec![
        TakerSwapEvent::Initialized {
            maker_coin_start_block: 1000,
            taker_coin_start_block: 1000,
        },
        TakerSwapEvent::Negotiated {
            maker_coin_start_block: 1000,
            taker_coin_start_block: 1000,
            secret_hash: Default::default(),
        },
        TakerSwapEvent::TakerPaymentSent {
            maker_coin_start_block: 1000,
            taker_coin_start_block: 1000,
            taker_payment: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
            secret_hash: Default::default(),
        },
        TakerSwapEvent::BothPaymentsSentAndConfirmed {
            maker_coin_start_block: 1000,
            taker_coin_start_block: 1000,
            maker_payment: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
            taker_payment: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
            secret_hash: Default::default(),
        },
        TakerSwapEvent::TakerPaymentSpent {
            maker_coin_start_block: 1000,
            taker_coin_start_block: 1000,
            maker_payment: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
            taker_payment: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
            taker_payment_spend: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
            secret: Default::default(),
        },
        TakerSwapEvent::MakerPaymentSpent {
            maker_coin_start_block: 1000,
            taker_coin_start_block: 1000,
            maker_payment: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
            taker_payment: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
            taker_payment_spend: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
            maker_payment_spend: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
        },
        TakerSwapEvent::Completed,
    ];
    assert_eq!(expected_events, *state_machine.storage.events.get(&uuid).unwrap());
}
