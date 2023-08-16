use crate::mm2::lp_swap::check_balance_for_maker_swap;
use async_trait::async_trait;
use coins::coin_errors::{MyAddressError, ValidatePaymentError, ValidatePaymentFut};
use coins::{BalanceFut, CheckIfMyPaymentSentArgs, CoinBalance, CoinFutSpawner, ConfirmPaymentInput, FeeApproxStage,
            FoundSwapTxSpend, HistorySyncState, MakerSwapTakerCoin, MarketCoinOps, MmCoin, MmCoinEnum,
            NegotiateSwapContractAddrErr, PaymentInstructionArgs, PaymentInstructions, PaymentInstructionsErr,
            RawTransactionFut, RawTransactionRequest, RefundPaymentArgs, RefundResult, SearchForSwapTxSpendInput,
            SendMakerPaymentSpendPreimageInput, SendPaymentArgs, SignatureResult, SpendPaymentArgs, SwapOps,
            TakerSwapMakerCoin, TradeFee, TradePreimageFut, TradePreimageResult, TradePreimageValue, TransactionEnum,
            TransactionFut, TransactionResult, TxMarshalingErr, UnexpectedDerivationMethod, ValidateAddressResult,
            ValidateFeeArgs, ValidateInstructionsErr, ValidateOtherPubKeyErr, ValidatePaymentInput,
            VerificationResult, WaitForHTLCTxSpendArgs, WatcherOps, WatcherReward, WatcherRewardError,
            WatcherSearchForSwapTxSpendInput, WatcherValidatePaymentInput, WatcherValidateTakerFeeInput, WithdrawFut,
            WithdrawRequest};
use common::block_on;
use common::executor::AbortedError;
use futures01::Future;
use keys::KeyPair;
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};
use mm2_err_handle::mm_error::{MmError, MmResult};
use mm2_number::{BigDecimal, MmNumber};
use mm2_state_machine::prelude::*;
use mm2_state_machine::storable_state_machine::*;
use rpc::v1::types::Bytes;
use serde_json::Value;
use std::collections::HashMap;
use std::marker::PhantomData;
use uuid::Uuid;

pub enum MakerSwapEvent {
    Finished,
}

pub enum MakerSwapStateMachineError {}

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

    async fn mark_finished(&mut self, id: Self::MachineId) -> Result<(), Self::Error> {
        self.events.remove(&id);
        Ok(())
    }
}

pub struct MakerSwapStateMachine<MakerCoin, TakerCoin> {
    ctx: MmArc,
    maker_coin: MakerCoin,
    maker_volume: MmNumber,
    taker_coin: TakerCoin,
    uuid: Uuid,
    storage: DummyMakerSwapStorage,
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

struct Initialize<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> InitialState for Initialize<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;
}

struct Finish<MakerCoin, TakerCoin> {
    maker_coin: PhantomData<MakerCoin>,
    taker_coin: PhantomData<TakerCoin>,
}

impl<MakerCoin: Send + 'static, TakerCoin: Send + 'static> StorableState for Finish<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    fn get_event(&self) -> <<Self::StateMachine as StorableStateMachine>::Storage as StateMachineStorage>::Event {
        MakerSwapEvent::Finished
    }
}

#[async_trait]
impl<MakerCoin: Send + Sync + 'static, TakerCoin: Send + Sync + 'static> LastState for Finish<MakerCoin, TakerCoin> {
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(
        self: Box<Self>,
        _ctx: &mut Self::StateMachine,
    ) -> <Self::StateMachine as StateMachineTrait>::Result {
        //TODO just log something here?
    }
}

impl<MakerCoin, TakerCoin> TransitionFrom<Initialize<MakerCoin, TakerCoin>> for Finish<MakerCoin, TakerCoin> {}

#[async_trait]
impl<MakerCoin: MmCoin + Send + Sync + 'static, TakerCoin: MmCoin + Send + Sync + 'static> State
    for Initialize<MakerCoin, TakerCoin>
{
    type StateMachine = MakerSwapStateMachine<MakerCoin, TakerCoin>;

    async fn on_changed(self: Box<Self>, ctx: &mut Self::StateMachine) -> StateResult<Self::StateMachine> {
        if check_balance_for_maker_swap(
            &ctx.ctx,
            &ctx.maker_coin,
            &ctx.taker_coin,
            ctx.maker_volume.clone(),
            Some(&ctx.uuid),
            None,
            FeeApproxStage::StartSwap,
        )
        .await
        .is_err()
        {
            return Self::change_state(
                Finish {
                    maker_coin: Default::default(),
                    taker_coin: Default::default(),
                },
                ctx,
            )
            .await;
        }
        unimplemented!()
    }
}

#[test]
fn just_run_it() {
    struct Coin {};

    #[async_trait]
    impl SwapOps for MakerCoin {
        fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, uuid: &[u8]) -> TransactionFut { todo!() }

        fn send_maker_payment(&self, maker_payment_args: SendPaymentArgs<'_>) -> TransactionFut { todo!() }

        fn send_taker_payment(&self, taker_payment_args: SendPaymentArgs<'_>) -> TransactionFut { todo!() }

        fn send_maker_spends_taker_payment(&self, maker_spends_payment_args: SpendPaymentArgs<'_>) -> TransactionFut {
            todo!()
        }

        fn send_taker_spends_maker_payment(&self, taker_spends_payment_args: SpendPaymentArgs<'_>) -> TransactionFut {
            todo!()
        }

        async fn send_taker_refunds_payment(
            &self,
            taker_refunds_payment_args: RefundPaymentArgs<'_>,
        ) -> TransactionResult {
            todo!()
        }

        async fn send_maker_refunds_payment(
            &self,
            maker_refunds_payment_args: RefundPaymentArgs<'_>,
        ) -> TransactionResult {
            todo!()
        }

        fn validate_fee(&self, validate_fee_args: ValidateFeeArgs<'_>) -> ValidatePaymentFut<()> { todo!() }

        fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> { todo!() }

        fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> { todo!() }

        fn check_if_my_payment_sent(
            &self,
            if_my_payment_sent_args: CheckIfMyPaymentSentArgs<'_>,
        ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
            todo!()
        }

        async fn search_for_swap_tx_spend_my(
            &self,
            input: SearchForSwapTxSpendInput<'_>,
        ) -> Result<Option<FoundSwapTxSpend>, String> {
            todo!()
        }

        async fn search_for_swap_tx_spend_other(
            &self,
            input: SearchForSwapTxSpendInput<'_>,
        ) -> Result<Option<FoundSwapTxSpend>, String> {
            todo!()
        }

        async fn extract_secret(
            &self,
            secret_hash: &[u8],
            spend_tx: &[u8],
            watcher_reward: bool,
        ) -> Result<Vec<u8>, String> {
            todo!()
        }

        fn check_tx_signed_by_pub(
            &self,
            tx: &[u8],
            expected_pub: &[u8],
        ) -> Result<bool, MmError<ValidatePaymentError>> {
            todo!()
        }

        fn is_auto_refundable(&self) -> bool { todo!() }

        async fn wait_for_htlc_refund(&self, _tx: &[u8], _locktime: u64) -> RefundResult<()> { todo!() }

        fn negotiate_swap_contract_addr(
            &self,
            other_side_address: Option<&[u8]>,
        ) -> Result<Option<Bytes>, MmError<NegotiateSwapContractAddrErr>> {
            todo!()
        }

        fn derive_htlc_key_pair(&self, swap_unique_data: &[u8]) -> KeyPair { todo!() }

        fn derive_htlc_pubkey(&self, swap_unique_data: &[u8]) -> Vec<u8> { todo!() }

        fn validate_other_pubkey(&self, raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> { todo!() }

        async fn maker_payment_instructions(
            &self,
            args: PaymentInstructionArgs<'_>,
        ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
            todo!()
        }

        async fn taker_payment_instructions(
            &self,
            args: PaymentInstructionArgs<'_>,
        ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
            todo!()
        }

        fn validate_maker_payment_instructions(
            &self,
            instructions: &[u8],
            args: PaymentInstructionArgs<'_>,
        ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
            todo!()
        }

        fn validate_taker_payment_instructions(
            &self,
            instructions: &[u8],
            args: PaymentInstructionArgs<'_>,
        ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
            todo!()
        }
    }

    #[async_trait]
    impl TakerSwapMakerCoin for MakerCoin {
        async fn on_taker_payment_refund_start(&self, maker_payment: &[u8]) -> RefundResult<()> { todo!() }

        async fn on_taker_payment_refund_success(&self, maker_payment: &[u8]) -> RefundResult<()> { todo!() }
    }

    #[async_trait]
    impl MakerSwapTakerCoin for MakerCoin {
        async fn on_maker_payment_refund_start(&self, taker_payment: &[u8]) -> RefundResult<()> { todo!() }

        async fn on_maker_payment_refund_success(&self, taker_payment: &[u8]) -> RefundResult<()> { todo!() }
    }

    #[async_trait]
    impl WatcherOps for MakerCoin {
        fn send_maker_payment_spend_preimage(&self, input: SendMakerPaymentSpendPreimageInput) -> TransactionFut {
            todo!()
        }

        fn send_taker_payment_refund_preimage(
            &self,
            watcher_refunds_payment_args: RefundPaymentArgs,
        ) -> TransactionFut {
            todo!()
        }

        fn create_taker_payment_refund_preimage(
            &self,
            _taker_payment_tx: &[u8],
            _time_lock: u32,
            _maker_pub: &[u8],
            _secret_hash: &[u8],
            _swap_contract_address: &Option<Bytes>,
            _swap_unique_data: &[u8],
        ) -> TransactionFut {
            todo!()
        }

        fn create_maker_payment_spend_preimage(
            &self,
            _maker_payment_tx: &[u8],
            _time_lock: u32,
            _maker_pub: &[u8],
            _secret_hash: &[u8],
            _swap_unique_data: &[u8],
        ) -> TransactionFut {
            todo!()
        }

        fn watcher_validate_taker_fee(&self, input: WatcherValidateTakerFeeInput) -> ValidatePaymentFut<()> { todo!() }

        fn watcher_validate_taker_payment(&self, _input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
            todo!()
        }

        async fn watcher_search_for_swap_tx_spend(
            &self,
            input: WatcherSearchForSwapTxSpendInput<'_>,
        ) -> Result<Option<FoundSwapTxSpend>, String> {
            todo!()
        }

        async fn get_taker_watcher_reward(
            &self,
            other_coin: &MmCoinEnum,
            coin_amount: Option<BigDecimal>,
            other_coin_amount: Option<BigDecimal>,
            reward_amount: Option<BigDecimal>,
            wait_until: u64,
        ) -> Result<WatcherReward, MmError<WatcherRewardError>> {
            todo!()
        }

        async fn get_maker_watcher_reward(
            &self,
            other_coin: &MmCoinEnum,
            reward_amount: Option<BigDecimal>,
            wait_until: u64,
        ) -> Result<Option<WatcherReward>, MmError<WatcherRewardError>> {
            todo!()
        }
    }

    impl MarketCoinOps for MakerCoin {
        fn ticker(&self) -> &str { todo!() }

        fn my_address(&self) -> MmResult<String, MyAddressError> { todo!() }

        fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> { todo!() }

        fn sign_message_hash(&self, _message: &str) -> Option<[u8; 32]> { todo!() }

        fn sign_message(&self, _message: &str) -> SignatureResult<String> { todo!() }

        fn verify_message(&self, _signature: &str, _message: &str, _address: &str) -> VerificationResult<bool> {
            todo!()
        }

        fn my_balance(&self) -> BalanceFut<CoinBalance> { todo!() }

        fn base_coin_balance(&self) -> BalanceFut<BigDecimal> { todo!() }

        fn platform_ticker(&self) -> &str { todo!() }

        fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> { todo!() }

        fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> { todo!() }

        fn wait_for_confirmations(
            &self,
            input: ConfirmPaymentInput,
        ) -> Box<dyn Future<Item = (), Error = String> + Send> {
            todo!()
        }

        fn wait_for_htlc_tx_spend(&self, args: WaitForHTLCTxSpendArgs<'_>) -> TransactionFut { todo!() }

        fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> { todo!() }

        fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> { todo!() }

        fn display_priv_key(&self) -> Result<String, String> { todo!() }

        fn min_tx_amount(&self) -> BigDecimal { todo!() }

        fn min_trading_vol(&self) -> MmNumber { todo!() }
    }

    #[async_trait]
    impl MmCoin for MakerCoin {
        fn is_asset_chain(&self) -> bool { todo!() }

        fn spawner(&self) -> CoinFutSpawner { todo!() }

        fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut { todo!() }

        fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut { todo!() }

        fn get_tx_hex_by_hash(&self, tx_hash: Vec<u8>) -> RawTransactionFut { todo!() }

        fn decimals(&self) -> u8 { todo!() }

        fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> { todo!() }

        fn validate_address(&self, address: &str) -> ValidateAddressResult { todo!() }

        fn process_history_loop(&self, ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> { todo!() }

        fn history_sync_status(&self) -> HistorySyncState { todo!() }

        fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> { todo!() }

        async fn get_sender_trade_fee(
            &self,
            value: TradePreimageValue,
            stage: FeeApproxStage,
        ) -> TradePreimageResult<TradeFee> {
            todo!()
        }

        fn get_receiver_trade_fee(&self, stage: FeeApproxStage) -> TradePreimageFut<TradeFee> { todo!() }

        async fn get_fee_to_send_taker_fee(
            &self,
            dex_fee_amount: BigDecimal,
            stage: FeeApproxStage,
        ) -> TradePreimageResult<TradeFee> {
            todo!()
        }

        fn required_confirmations(&self) -> u64 { todo!() }

        fn requires_notarization(&self) -> bool { todo!() }

        fn set_required_confirmations(&self, confirmations: u64) { todo!() }

        fn set_requires_notarization(&self, requires_nota: bool) { todo!() }

        fn swap_contract_address(&self) -> Option<BytesJson> { todo!() }

        fn fallback_swap_contract(&self) -> Option<BytesJson> { todo!() }

        fn mature_confirmations(&self) -> Option<u32> { todo!() }

        fn coin_protocol_info(&self, amount_to_receive: Option<MmNumber>) -> Vec<u8> { todo!() }

        fn is_coin_protocol_supported(
            &self,
            info: &Option<Vec<u8>>,
            amount_to_send: Option<MmNumber>,
            locktime: u64,
            is_maker: bool,
        ) -> bool {
            todo!()
        }

        fn on_disabled(&self) -> Result<(), AbortedError> { todo!() }

        fn on_token_deactivated(&self, ticker: &str) { todo!() }
    }

    let ctx = MmCtxBuilder::default().into_mm_arc();
    let mut machine = MakerSwapStateMachine {
        ctx,
        maker_coin: Coin {},
        maker_volume: Default::default(),
        taker_coin: Coin {},
        uuid: Default::default(),
        storage: DummyMakerSwapStorage {
            events: Default::default(),
        },
    };

    block_on(machine.run(Box::new(Initialize {
        maker_coin: Default::default(),
        taker_coin: Default::default(),
    })))
    .unwrap();
}
