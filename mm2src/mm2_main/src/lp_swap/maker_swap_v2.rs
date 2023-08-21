use crate::mm2::lp_swap::{check_balance_for_maker_swap, SwapConfirmationsSettings, TransactionIdentifier};
use async_trait::async_trait;
use coins::coin_errors::{MyAddressError, ValidatePaymentError, ValidatePaymentFut};
use coins::{BalanceFut, CheckIfMyPaymentSentArgs, CoinBalance, CoinFutSpawner, ConfirmPaymentInput, FeeApproxStage,
            FoundSwapTxSpend, GenTakerPaymentSpendArgs, GenTakerPaymentSpendResult, HistorySyncState,
            MakerSwapTakerCoin, MarketCoinOps, MmCoin, MmCoinEnum, NegotiateSwapContractAddrErr,
            PaymentInstructionArgs, PaymentInstructions, PaymentInstructionsErr, RawTransactionFut,
            RawTransactionRequest, RefundPaymentArgs, RefundResult, SearchForSwapTxSpendInput,
            SendDexFeeWithPremiumArgs, SendMakerPaymentSpendPreimageInput, SendPaymentArgs, SignatureResult,
            SpendPaymentArgs, SwapOps, SwapOpsV2, TakerSwapMakerCoin, TestCoin, TradeFee, TradePreimageFut,
            TradePreimageResult, TradePreimageValue, TransactionEnum, TransactionFut, TransactionResult,
            TxMarshalingErr, TxPreimageWithSig, UnexpectedDerivationMethod, ValidateAddressResult,
            ValidateDexFeeResult, ValidateDexFeeSpendPreimageResult, ValidateFeeArgs, ValidateInstructionsErr,
            ValidateOtherPubKeyErr, ValidatePaymentInput, ValidateTakerPaymentArgs, VerificationResult,
            WaitForHTLCTxSpendArgs, WatcherOps, WatcherReward, WatcherRewardError, WatcherSearchForSwapTxSpendInput,
            WatcherValidatePaymentInput, WatcherValidateTakerFeeInput, WithdrawFut, WithdrawRequest};
use common::executor::AbortedError;
use common::{block_on, Future01CompatExt};
use futures01::Future;
use keys::KeyPair;
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};
use mm2_err_handle::mm_error::{MmError, MmResult};
use mm2_number::{BigDecimal, MmNumber};
use mm2_state_machine::prelude::*;
use mm2_state_machine::storable_state_machine::*;
use primitives::hash::H256;
use rpc::v1::types::Bytes as BytesJson;
use serde_json::Value as Json;
use std::collections::HashMap;
use std::marker::PhantomData;
use uuid::Uuid;

#[derive(Debug, PartialEq)]
pub enum MakerSwapEvent {
    Initialized {
        maker_coin_start_block: u64,
        taker_coin_start_block: u64,
    },
    Negotiated {
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
    pub taker_coin: TakerCoin,
    pub taker_volume: MmNumber,
    pub taker_premium: MmNumber,
    pub conf_settings: SwapConfirmationsSettings,
    pub uuid: Uuid,
    pub p2p_keypair: Option<KeyPair>,
}

impl<MakerCoin, TakerCoin> MakerSwapStateMachine<MakerCoin, TakerCoin> {
    fn taker_payment_conf_timeout(&self) -> u64 { 0 }
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

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let maker_coin_start_block = match ctx.maker_coin.current_block().compat().await {
            Ok(b) => b,
            Err(e) => return Self::change_state(Aborted::new(e), ctx).await,
        };

        let taker_coin_start_block = match ctx.taker_coin.current_block().compat().await {
            Ok(b) => b,
            Err(e) => return Self::change_state(Aborted::new(e), ctx).await,
        };

        if let Err(e) = check_balance_for_maker_swap(
            &ctx.ctx,
            &ctx.maker_coin,
            &ctx.taker_coin,
            ctx.maker_volume.clone(),
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
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::Initialized {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
        }
    }
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: MarketCoinOps + Send + Sync + 'static> State
    for Initialized<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let next_state = Negotiated {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
        };
        Self::change_state(next_state, ctx).await
    }
}

struct Negotiated<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
}

impl<MakerCoin, TakerCoin> TransitionFrom<Initialized<MakerCoin, TakerCoin>> for Negotiated<MakerCoin, TakerCoin> {}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: MarketCoinOps + Send + Sync + 'static> State
    for Negotiated<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let next_state = MakerPaymentSent {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
        };
        Self::change_state(next_state, ctx).await
    }
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState for Negotiated<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::Negotiated {
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
        }
    }
}

struct MakerPaymentSent<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
    maker_coin_start_block: u64,
    taker_coin_start_block: u64,
    maker_payment: TransactionIdentifier,
}

impl<MakerCoin, TakerCoin> TransitionFrom<Negotiated<MakerCoin, TakerCoin>> for MakerPaymentSent<MakerCoin, TakerCoin> {}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: MarketCoinOps + Send + Sync + 'static> State
    for MakerPaymentSent<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        let taker_payment = TransactionIdentifier {
            tx_hex: Default::default(),
            tx_hash: Default::default(),
        };

        let input = ConfirmPaymentInput {
            payment_tx: taker_payment.tx_hex.0.clone(),
            confirmations: ctx.conf_settings.taker_coin_confs,
            requires_nota: ctx.conf_settings.taker_coin_nota,
            wait_until: ctx.taker_payment_conf_timeout(),
            check_every: 10,
        };
        if let Err(e) = ctx.taker_coin.wait_for_confirmations(input).compat().await {
            let next_state = MakerPaymentRefundRequired {
                maker_coin: Default::default(),
                taker_coin: Default::default(),
                maker_payment: self.maker_payment,
            };
            return Self::change_state(next_state, ctx).await;
        }

        let next_state = BothPaymentsSentAndConfirmed {
            maker_coin: Default::default(),
            taker_coin: Default::default(),
            maker_coin_start_block: self.maker_coin_start_block,
            taker_coin_start_block: self.taker_coin_start_block,
            maker_payment: self.maker_payment,
            taker_payment,
        };
        Self::change_state(next_state, ctx).await
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

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
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

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        Self::change_state(Completed::new(), ctx).await
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
        _ctx: &mut Self::StateMachine,
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
        _ctx: &mut Self::StateMachine,
    ) -> <Self::StateMachine as StateMachineTrait>::Result {
        //TODO just log something here?
    }
}

impl<MakerCoin, TakerCoin> TransitionFrom<TakerPaymentSpent<MakerCoin, TakerCoin>> for Completed<MakerCoin, TakerCoin> {}

#[test]
fn just_run_it() {
    use mocktopus::mocking::{MockResult, Mockable};
    TestCoin::current_block.mock_safe(|_| MockResult::Return(Box::new(futures01::future::ok(1000))));
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
    let mut machine = MakerSwapStateMachine {
        ctx,
        maker_coin: TestCoin::default(),
        maker_volume: Default::default(),
        conf_settings: SwapConfirmationsSettings {
            maker_coin_confs: 0,
            maker_coin_nota: false,
            taker_coin_confs: 0,
            taker_coin_nota: false,
        },
        taker_coin: TestCoin::default(),
        taker_volume: Default::default(),
        uuid,
        storage: DummyMakerSwapStorage {
            events: Default::default(),
        },
        taker_premium: Default::default(),
        secret: Default::default(),
        p2p_keypair: None,
    };

    block_on(machine.run(Box::new(Initialize {
        maker_coin: Default::default(),
        taker_coin: Default::default(),
    })))
    .unwrap();

    let expected_events = vec![
        MakerSwapEvent::Initialized {
            maker_coin_start_block: 1000,
            taker_coin_start_block: 1000,
        },
        MakerSwapEvent::Negotiated {
            maker_coin_start_block: 1000,
            taker_coin_start_block: 1000,
        },
        MakerSwapEvent::MakerPaymentSent {
            maker_coin_start_block: 1000,
            taker_coin_start_block: 1000,
            maker_payment: TransactionIdentifier {
                tx_hex: Default::default(),
                tx_hash: Default::default(),
            },
        },
        MakerSwapEvent::BothPaymentsSentAndConfirmed {
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
        },
        MakerSwapEvent::TakerPaymentSpent {
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
        },
        MakerSwapEvent::Completed,
    ];
    assert_eq!(expected_events, *machine.storage.events.get(&uuid).unwrap());
}
