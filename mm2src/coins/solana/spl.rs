use super::{CoinBalance, HistorySyncState, MarketCoinOps, MmCoin, SwapOps, TradeFee, TransactionEnum, WatcherOps};
use crate::coin_errors::MyAddressError;
use crate::solana::solana_common::{ui_amount_to_amount, PrepareTransferData, SufficientBalanceError};
use crate::solana::{solana_common, AccountError, SolanaCommonOps, SolanaFeeDetails};
use crate::{BalanceFut, CheckIfMyPaymentSentArgs, CoinFutSpawner, ConfirmPaymentInput, DexFee, FeeApproxStage,
            FoundSwapTxSpend, MakerSwapTakerCoin, MmCoinEnum, NegotiateSwapContractAddrErr, PaymentInstructionArgs,
            PaymentInstructions, PaymentInstructionsErr, RawTransactionFut, RawTransactionRequest, RefundError,
            RefundPaymentArgs, RefundResult, SearchForSwapTxSpendInput, SendMakerPaymentSpendPreimageInput,
            SendPaymentArgs, SignatureResult, SolanaCoin, SpendPaymentArgs, TakerSwapMakerCoin, TradePreimageFut,
            TradePreimageResult, TradePreimageValue, TransactionDetails, TransactionFut, TransactionResult,
            TransactionType, TxMarshalingErr, UnexpectedDerivationMethod, ValidateAddressResult, ValidateFeeArgs,
            ValidateInstructionsErr, ValidateOtherPubKeyErr, ValidatePaymentError, ValidatePaymentFut,
            ValidatePaymentInput, ValidateWatcherSpendInput, VerificationResult, WaitForHTLCTxSpendArgs,
            WatcherReward, WatcherRewardError, WatcherSearchForSwapTxSpendInput, WatcherValidatePaymentInput,
            WatcherValidateTakerFeeInput, WithdrawError, WithdrawFut, WithdrawRequest, WithdrawResult};
use async_trait::async_trait;
use bincode::serialize;
use common::executor::{abortable_queue::AbortableQueue, AbortableSystem, AbortedError};
use common::{async_blocking, now_sec};
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::{BigDecimal, MmNumber};
use rpc::v1::types::Bytes as BytesJson;
use serde_json::Value as Json;
use solana_client::{rpc_client::RpcClient, rpc_request::TokenAccountsFilter};
use solana_sdk::message::Message;
use solana_sdk::transaction::Transaction;
use solana_sdk::{pubkey::Pubkey, signature::Signer};
use spl_associated_token_account::{create_associated_token_account, get_associated_token_address};
use std::{convert::TryFrom,
          fmt::{Debug, Formatter, Result as FmtResult},
          str::FromStr,
          sync::Arc};

#[derive(Debug)]
pub enum SplTokenCreationError {
    InvalidPubkey(String),
    Internal(String),
}

impl From<AbortedError> for SplTokenCreationError {
    fn from(e: AbortedError) -> Self { SplTokenCreationError::Internal(e.to_string()) }
}

pub struct SplTokenFields {
    pub decimals: u8,
    pub ticker: String,
    pub token_contract_address: Pubkey,
    pub abortable_system: AbortableQueue,
}

#[derive(Clone, Debug)]
pub struct SplTokenInfo {
    pub token_contract_address: Pubkey,
    pub decimals: u8,
}

#[derive(Debug)]
pub struct SplProtocolConf {
    pub platform_coin_ticker: String,
    pub decimals: u8,
    pub token_contract_address: String,
}

#[derive(Clone)]
pub struct SplToken {
    pub conf: Arc<SplTokenFields>,
    pub platform_coin: SolanaCoin,
}

impl Debug for SplToken {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult { f.write_str(&self.conf.ticker) }
}

impl SplToken {
    pub fn new(
        decimals: u8,
        ticker: String,
        token_address: String,
        platform_coin: SolanaCoin,
    ) -> MmResult<SplToken, SplTokenCreationError> {
        let token_contract_address = solana_sdk::pubkey::Pubkey::from_str(&token_address)
            .map_err(|e| MmError::new(SplTokenCreationError::InvalidPubkey(format!("{:?}", e))))?;

        // Create an abortable system linked to the `platform_coin` so if the platform coin is disabled,
        // all spawned futures related to `SplToken` will be aborted as well.
        let abortable_system = platform_coin.abortable_system.create_subsystem()?;

        let conf = Arc::new(SplTokenFields {
            decimals,
            ticker,
            token_contract_address,
            abortable_system,
        });
        Ok(SplToken { conf, platform_coin })
    }

    pub fn get_info(&self) -> SplTokenInfo {
        SplTokenInfo {
            token_contract_address: self.conf.token_contract_address,
            decimals: self.decimals(),
        }
    }
}

async fn withdraw_spl_token_impl(coin: SplToken, req: WithdrawRequest) -> WithdrawResult {
    let (hash, fees) = coin.platform_coin.estimate_withdraw_fees().await?;
    let res = coin
        .check_balance_and_prepare_transfer(req.max, req.amount.clone(), fees)
        .await?;
    let system_destination_pubkey = solana_sdk::pubkey::Pubkey::try_from(&*req.to)?;
    let contract_key = coin.get_underlying_contract_pubkey();
    let auth_key = coin.platform_coin.key_pair.pubkey();
    let funding_address = coin.get_pubkey().await?;
    let dest_token_address = get_associated_token_address(&system_destination_pubkey, &contract_key);
    let mut instructions = Vec::with_capacity(1);
    let account_info = async_blocking({
        let coin = coin.clone();
        move || coin.rpc().get_account(&dest_token_address)
    })
    .await;
    if account_info.is_err() {
        let instruction_creation = create_associated_token_account(&auth_key, &dest_token_address, &contract_key);
        instructions.push(instruction_creation);
    }
    let amount = ui_amount_to_amount(req.amount, coin.conf.decimals)?;
    let instruction_transfer_checked = spl_token::instruction::transfer_checked(
        &spl_token::id(),
        &funding_address,
        &contract_key,
        &dest_token_address,
        &auth_key,
        &[&auth_key],
        amount,
        coin.conf.decimals,
    )?;
    instructions.push(instruction_transfer_checked);
    let msg = Message::new(&instructions, Some(&auth_key));
    let signers = vec![&coin.platform_coin.key_pair];
    let tx = Transaction::new(&signers, msg, hash);
    let serialized_tx = serialize(&tx).map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;
    let received_by_me = if req.to == coin.platform_coin.my_address {
        res.to_send.clone()
    } else {
        0.into()
    };
    Ok(TransactionDetails {
        tx_hex: serialized_tx.into(),
        tx_hash: tx.signatures[0].to_string(),
        from: vec![coin.platform_coin.my_address.clone()],
        to: vec![req.to],
        total_amount: res.to_send.clone(),
        spent_by_me: res.to_send.clone(),
        my_balance_change: &received_by_me - &res.to_send,
        received_by_me,
        block_height: 0,
        timestamp: now_sec(),
        fee_details: Some(
            SolanaFeeDetails {
                amount: res.sol_required,
            }
            .into(),
        ),
        coin: coin.conf.ticker.clone(),
        internal_id: vec![].into(),
        kmd_rewards: None,
        transaction_type: TransactionType::StandardTransfer,
        memo: None,
    })
}

async fn withdraw_impl(coin: SplToken, req: WithdrawRequest) -> WithdrawResult {
    let validate_address_result = coin.validate_address(&req.to);
    if !validate_address_result.is_valid {
        return MmError::err(WithdrawError::InvalidAddress(
            validate_address_result.reason.unwrap_or_else(|| "Unknown".to_string()),
        ));
    }
    withdraw_spl_token_impl(coin, req).await
}

#[async_trait]
impl SolanaCommonOps for SplToken {
    fn rpc(&self) -> &RpcClient { &self.platform_coin.client }

    fn is_token(&self) -> bool { true }

    async fn check_balance_and_prepare_transfer(
        &self,
        max: bool,
        amount: BigDecimal,
        fees: u64,
    ) -> Result<PrepareTransferData, MmError<SufficientBalanceError>> {
        solana_common::check_balance_and_prepare_transfer(self, max, amount, fees).await
    }
}

impl SplToken {
    fn get_underlying_contract_pubkey(&self) -> Pubkey { self.conf.token_contract_address }

    async fn get_pubkey(&self) -> Result<Pubkey, MmError<AccountError>> {
        let coin = self.clone();
        let token_accounts = async_blocking(move || {
            coin.rpc().get_token_accounts_by_owner(
                &coin.platform_coin.key_pair.pubkey(),
                TokenAccountsFilter::Mint(coin.get_underlying_contract_pubkey()),
            )
        })
        .await?;
        if token_accounts.is_empty() {
            return MmError::err(AccountError::NotFundedError("account_not_funded".to_string()));
        }
        Ok(Pubkey::from_str(&token_accounts[0].pubkey)?)
    }

    fn my_balance_impl(&self) -> BalanceFut<CoinBalance> {
        let coin = self.clone();
        let fut = async move {
            coin.platform_coin
                .my_balance_spl(SplTokenInfo {
                    token_contract_address: coin.conf.token_contract_address,
                    decimals: coin.conf.decimals,
                })
                .await
        };
        Box::new(fut.boxed().compat())
    }
}

impl MarketCoinOps for SplToken {
    fn ticker(&self) -> &str { &self.conf.ticker }

    fn my_address(&self) -> MmResult<String, MyAddressError> { Ok(self.platform_coin.my_address.clone()) }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> { unimplemented!() }

    fn sign_message_hash(&self, _message: &str) -> Option<[u8; 32]> { unimplemented!() }

    fn sign_message(&self, message: &str) -> SignatureResult<String> {
        solana_common::sign_message(&self.platform_coin, message)
    }

    fn verify_message(&self, signature: &str, message: &str, pubkey_bs58: &str) -> VerificationResult<bool> {
        solana_common::verify_message(&self.platform_coin, signature, message, pubkey_bs58)
    }

    fn my_balance(&self) -> BalanceFut<CoinBalance> {
        let fut = self.my_balance_impl().and_then(Ok);
        Box::new(fut)
    }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> { self.platform_coin.base_coin_balance() }

    fn platform_ticker(&self) -> &str { self.platform_coin.ticker() }

    #[inline(always)]
    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
        self.platform_coin.send_raw_tx(tx)
    }

    #[inline(always)]
    fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> {
        self.platform_coin.send_raw_tx_bytes(tx)
    }

    fn wait_for_confirmations(&self, _input: ConfirmPaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn wait_for_htlc_tx_spend(&self, args: WaitForHTLCTxSpendArgs<'_>) -> TransactionFut { unimplemented!() }

    fn tx_enum_from_bytes(&self, _bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        MmError::err(TxMarshalingErr::NotSupported(
            "tx_enum_from_bytes is not supported for Spl yet.".to_string(),
        ))
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> { self.platform_coin.current_block() }

    fn display_priv_key(&self) -> Result<String, String> { self.platform_coin.display_priv_key() }

    fn min_tx_amount(&self) -> BigDecimal { BigDecimal::from(0) }

    fn min_trading_vol(&self) -> MmNumber { MmNumber::from("0.00777") }
}

#[async_trait]
impl SwapOps for SplToken {
    fn send_taker_fee(&self, _fee_addr: &[u8], dex_fee: DexFee, _uuid: &[u8]) -> TransactionFut { unimplemented!() }

    fn send_maker_payment(&self, _maker_payment_args: SendPaymentArgs) -> TransactionFut { unimplemented!() }

    fn send_taker_payment(&self, _taker_payment_args: SendPaymentArgs) -> TransactionFut { unimplemented!() }

    fn send_maker_spends_taker_payment(&self, _maker_spends_payment_args: SpendPaymentArgs) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_spends_maker_payment(&self, _taker_spends_payment_args: SpendPaymentArgs) -> TransactionFut {
        unimplemented!()
    }

    async fn send_taker_refunds_payment(
        &self,
        _taker_refunds_payment_args: RefundPaymentArgs<'_>,
    ) -> TransactionResult {
        unimplemented!()
    }

    async fn send_maker_refunds_payment(
        &self,
        _maker_refunds_payment_args: RefundPaymentArgs<'_>,
    ) -> TransactionResult {
        todo!()
    }

    fn validate_fee(&self, _validate_fee_args: ValidateFeeArgs) -> ValidatePaymentFut<()> { unimplemented!() }

    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> { unimplemented!() }

    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> { unimplemented!() }

    fn check_if_my_payment_sent(
        &self,
        _if_my_payment_sent_args: CheckIfMyPaymentSentArgs,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        unimplemented!()
    }

    async fn search_for_swap_tx_spend_my(
        &self,
        _: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!()
    }

    async fn search_for_swap_tx_spend_other(
        &self,
        _: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!()
    }

    fn check_tx_signed_by_pub(&self, tx: &[u8], expected_pub: &[u8]) -> Result<bool, MmError<ValidatePaymentError>> {
        unimplemented!();
    }

    async fn extract_secret(
        &self,
        secret_hash: &[u8],
        spend_tx: &[u8],
        watcher_reward: bool,
    ) -> Result<Vec<u8>, String> {
        unimplemented!()
    }

    fn is_auto_refundable(&self) -> bool { false }

    async fn wait_for_htlc_refund(&self, _tx: &[u8], _locktime: u64) -> RefundResult<()> {
        MmError::err(RefundError::Internal(
            "wait_for_htlc_refund is not supported for this coin!".into(),
        ))
    }

    fn negotiate_swap_contract_addr(
        &self,
        _other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        unimplemented!()
    }

    #[inline]
    fn derive_htlc_key_pair(&self, _swap_unique_data: &[u8]) -> KeyPair { todo!() }

    #[inline]
    fn derive_htlc_pubkey(&self, swap_unique_data: &[u8]) -> Vec<u8> {
        self.derive_htlc_key_pair(swap_unique_data).public_slice().to_vec()
    }

    fn validate_other_pubkey(&self, _raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> { unimplemented!() }

    async fn maker_payment_instructions(
        &self,
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        unimplemented!()
    }

    async fn taker_payment_instructions(
        &self,
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        unimplemented!()
    }

    fn validate_maker_payment_instructions(
        &self,
        _instructions: &[u8],
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        unimplemented!()
    }

    fn validate_taker_payment_instructions(
        &self,
        _instructions: &[u8],
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        unimplemented!()
    }
}

#[async_trait]
impl TakerSwapMakerCoin for SplToken {
    async fn on_taker_payment_refund_start(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_taker_payment_refund_success(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
impl MakerSwapTakerCoin for SplToken {
    async fn on_maker_payment_refund_start(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_maker_payment_refund_success(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
impl WatcherOps for SplToken {
    fn send_maker_payment_spend_preimage(&self, _input: SendMakerPaymentSpendPreimageInput) -> TransactionFut {
        unimplemented!();
    }

    fn create_taker_payment_refund_preimage(
        &self,
        _taker_payment_tx: &[u8],
        _time_lock: u64,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn create_maker_payment_spend_preimage(
        &self,
        _maker_payment_tx: &[u8],
        _time_lock: u64,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_taker_payment_refund_preimage(&self, _watcher_refunds_payment_args: RefundPaymentArgs) -> TransactionFut {
        unimplemented!();
    }

    fn watcher_validate_taker_fee(&self, _input: WatcherValidateTakerFeeInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    fn watcher_validate_taker_payment(&self, _input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    fn taker_validates_payment_spend_or_refund(&self, _input: ValidateWatcherSpendInput) -> ValidatePaymentFut<()> {
        unimplemented!()
    }

    async fn watcher_search_for_swap_tx_spend(
        &self,
        input: WatcherSearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!();
    }

    async fn get_taker_watcher_reward(
        &self,
        other_coin: &MmCoinEnum,
        coin_amount: Option<BigDecimal>,
        other_coin_amount: Option<BigDecimal>,
        reward_amount: Option<BigDecimal>,
        wait_until: u64,
    ) -> Result<WatcherReward, MmError<WatcherRewardError>> {
        unimplemented!();
    }

    async fn get_maker_watcher_reward(
        &self,
        other_coin: &MmCoinEnum,
        reward_amount: Option<BigDecimal>,
        wait_until: u64,
    ) -> Result<Option<WatcherReward>, MmError<WatcherRewardError>> {
        unimplemented!();
    }
}

#[async_trait]
impl MmCoin for SplToken {
    fn is_asset_chain(&self) -> bool { false }

    fn spawner(&self) -> CoinFutSpawner { CoinFutSpawner::new(&self.conf.abortable_system) }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        Box::new(Box::pin(withdraw_impl(self.clone(), req)).compat())
    }

    fn get_raw_transaction(&self, _req: RawTransactionRequest) -> RawTransactionFut { unimplemented!() }

    fn get_tx_hex_by_hash(&self, tx_hash: Vec<u8>) -> RawTransactionFut { unimplemented!() }

    fn decimals(&self) -> u8 { self.conf.decimals }

    fn convert_to_address(&self, _from: &str, _to_address_format: Json) -> Result<String, String> { unimplemented!() }

    fn validate_address(&self, address: &str) -> ValidateAddressResult { self.platform_coin.validate_address(address) }

    fn process_history_loop(&self, _ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> { unimplemented!() }

    fn history_sync_status(&self) -> HistorySyncState { unimplemented!() }

    /// Get fee to be paid per 1 swap transaction
    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> { unimplemented!() }

    async fn get_sender_trade_fee(
        &self,
        _value: TradePreimageValue,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        unimplemented!()
    }

    fn get_receiver_trade_fee(&self, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> { unimplemented!() }

    async fn get_fee_to_send_taker_fee(
        &self,
        _dex_fee_amount: DexFee,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        unimplemented!()
    }

    fn required_confirmations(&self) -> u64 { 1 }

    fn requires_notarization(&self) -> bool { false }

    fn set_required_confirmations(&self, _confirmations: u64) { unimplemented!() }

    fn set_requires_notarization(&self, _requires_nota: bool) { unimplemented!() }

    fn swap_contract_address(&self) -> Option<BytesJson> { unimplemented!() }

    fn fallback_swap_contract(&self) -> Option<BytesJson> { unimplemented!() }

    fn mature_confirmations(&self) -> Option<u32> { Some(1) }

    fn coin_protocol_info(&self, _amount_to_receive: Option<MmNumber>) -> Vec<u8> { Vec::new() }

    fn is_coin_protocol_supported(
        &self,
        _info: &Option<Vec<u8>>,
        _amount_to_send: Option<MmNumber>,
        _locktime: u64,
        _is_maker: bool,
    ) -> bool {
        true
    }

    fn on_disabled(&self) -> Result<(), AbortedError> { self.conf.abortable_system.abort_all() }

    fn on_token_deactivated(&self, _ticker: &str) {}
}
