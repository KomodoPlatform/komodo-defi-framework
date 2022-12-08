/// Module containing implementation for Tendermint Tokens. They include native assets + IBC
use super::{TendermintCoin, TendermintFeeDetails, GAS_LIMIT_DEFAULT, MIN_TX_SATOSHIS, TIMEOUT_HEIGHT_DELTA,
            TX_DEFAULT_MEMO};
use crate::utxo::utxo_common::big_decimal_from_sat;
use crate::{big_decimal_from_sat_unsigned, utxo::sat_from_big_decimal, BalanceFut, BigDecimal,
            CheckIfMyPaymentSentArgs, CoinBalance, CoinFutSpawner, FeeApproxStage, FoundSwapTxSpend, HistorySyncState,
            MarketCoinOps, MmCoin, MyAddressError, NegotiateSwapContractAddrErr, PaymentInstructions,
            PaymentInstructionsErr, RawTransactionFut, RawTransactionRequest, SearchForSwapTxSpendInput,
            SendMakerPaymentArgs, SendMakerRefundsPaymentArgs, SendMakerSpendsTakerPaymentArgs, SendTakerPaymentArgs,
            SendTakerRefundsPaymentArgs, SendTakerSpendsMakerPaymentArgs, SignatureResult, SwapOps, TradeFee,
            TradePreimageFut, TradePreimageResult, TradePreimageValue, TransactionDetails, TransactionEnum,
            TransactionErr, TransactionFut, TransactionType, TxFeeDetails, TxMarshalingErr,
            UnexpectedDerivationMethod, ValidateAddressResult, ValidateFeeArgs, ValidateInstructionsErr,
            ValidateOtherPubKeyErr, ValidatePaymentFut, ValidatePaymentInput, VerificationResult, WatcherOps,
            WatcherValidatePaymentInput, WithdrawError, WithdrawFut, WithdrawRequest};
use async_trait::async_trait;
use bitcrypto::sha256;
use common::executor::abortable_queue::AbortableQueue;
use common::executor::{AbortableSystem, AbortedError};
use common::log::warn;
use common::Future01CompatExt;
use cosmrs::{bank::MsgSend,
             tx::{Fee, Msg},
             AccountId, Coin, Denom};
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::MmNumber;
use rpc::v1::types::Bytes as BytesJson;
use serde_json::Value as Json;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;

pub struct TendermintTokenImpl {
    pub ticker: String,
    pub platform_coin: TendermintCoin,
    pub decimals: u8,
    pub denom: Denom,
    /// This spawner is used to spawn coin's related futures that should be aborted on coin deactivation
    /// or on [`MmArc::stop`].
    abortable_system: AbortableQueue,
}

#[derive(Clone)]
pub struct TendermintToken(Arc<TendermintTokenImpl>);

impl Deref for TendermintToken {
    type Target = TendermintTokenImpl;

    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TendermintTokenProtocolInfo {
    pub platform: String,
    pub decimals: u8,
    pub denom: String,
}

#[derive(Clone, Deserialize)]
pub struct TendermintTokenActivationParams {}

pub enum TendermintTokenInitError {
    Internal(String),
    InvalidDenom(String),
    MyAddressError(String),
    CouldNotFetchBalance(String),
}

impl From<MyAddressError> for TendermintTokenInitError {
    fn from(err: MyAddressError) -> Self { TendermintTokenInitError::MyAddressError(err.to_string()) }
}

impl From<AbortedError> for TendermintTokenInitError {
    fn from(e: AbortedError) -> Self { TendermintTokenInitError::Internal(e.to_string()) }
}

impl TendermintToken {
    pub fn new(
        ticker: String,
        platform_coin: TendermintCoin,
        decimals: u8,
        denom: String,
    ) -> MmResult<Self, TendermintTokenInitError> {
        let denom = Denom::from_str(&denom).map_to_mm(|e| TendermintTokenInitError::InvalidDenom(e.to_string()))?;
        let token_impl = TendermintTokenImpl {
            abortable_system: platform_coin.abortable_system.create_subsystem()?,
            ticker,
            platform_coin,
            decimals,
            denom,
        };
        Ok(TendermintToken(Arc::new(token_impl)))
    }
}

#[async_trait]
#[allow(unused_variables)]
impl SwapOps for TendermintToken {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, uuid: &[u8]) -> TransactionFut {
        self.platform_coin
            .send_taker_fee_for_denom(fee_addr, amount, self.denom.clone(), self.decimals, uuid)
    }

    fn send_maker_payment(&self, maker_payment_args: SendMakerPaymentArgs) -> TransactionFut {
        self.platform_coin.send_htlc_for_denom(
            maker_payment_args.time_lock_duration,
            maker_payment_args.other_pubkey,
            maker_payment_args.secret_hash,
            maker_payment_args.amount,
            self.denom.clone(),
            self.decimals,
        )
    }

    fn send_taker_payment(&self, taker_payment_args: SendTakerPaymentArgs) -> TransactionFut {
        self.platform_coin.send_htlc_for_denom(
            taker_payment_args.time_lock_duration,
            taker_payment_args.other_pubkey,
            taker_payment_args.secret_hash,
            taker_payment_args.amount,
            self.denom.clone(),
            self.decimals,
        )
    }

    fn send_maker_spends_taker_payment(
        &self,
        maker_spends_payment_args: SendMakerSpendsTakerPaymentArgs,
    ) -> TransactionFut {
        self.platform_coin
            .send_maker_spends_taker_payment(maker_spends_payment_args)
    }

    fn send_taker_spends_maker_payment(
        &self,
        taker_spends_payment_args: SendTakerSpendsMakerPaymentArgs,
    ) -> TransactionFut {
        self.platform_coin
            .send_taker_spends_maker_payment(taker_spends_payment_args)
    }

    fn send_taker_refunds_payment(&self, taker_refunds_payment_args: SendTakerRefundsPaymentArgs) -> TransactionFut {
        Box::new(futures01::future::err(TransactionErr::Plain(
            "Doesn't need transaction broadcast to be refunded".into(),
        )))
    }

    fn send_maker_refunds_payment(&self, maker_refunds_payment_args: SendMakerRefundsPaymentArgs) -> TransactionFut {
        Box::new(futures01::future::err(TransactionErr::Plain(
            "Doesn't need transaction broadcast to be refunded".into(),
        )))
    }

    fn validate_fee(&self, validate_fee_args: ValidateFeeArgs) -> Box<dyn Future<Item = (), Error = String> + Send> {
        self.platform_coin.validate_fee_for_denom(
            validate_fee_args.fee_tx,
            validate_fee_args.expected_sender,
            validate_fee_args.fee_addr,
            validate_fee_args.amount,
            self.decimals,
            validate_fee_args.uuid,
            self.denom.to_string(),
        )
    }

    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        self.platform_coin
            .validate_payment_for_denom(input, self.denom.clone(), self.decimals)
    }

    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        self.platform_coin
            .validate_payment_for_denom(input, self.denom.clone(), self.decimals)
    }

    fn check_if_my_payment_sent(
        &self,
        if_my_payment_spent_args: CheckIfMyPaymentSentArgs,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        self.platform_coin.check_if_my_payment_sent_for_denom(
            self.decimals,
            self.denom.clone(),
            if_my_payment_spent_args.other_pub,
            if_my_payment_spent_args.secret_hash,
            if_my_payment_spent_args.amount,
        )
    }

    async fn search_for_swap_tx_spend_my(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        self.platform_coin.search_for_swap_tx_spend_my(input).await
    }

    async fn search_for_swap_tx_spend_other(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        self.platform_coin.search_for_swap_tx_spend_other(input).await
    }

    async fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> {
        self.platform_coin.extract_secret(secret_hash, spend_tx).await
    }

    fn check_tx_signed_by_pub(&self, tx: &[u8], expected_pub: &[u8]) -> Result<bool, String> {
        unimplemented!();
    }

    fn negotiate_swap_contract_addr(
        &self,
        other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        self.platform_coin.negotiate_swap_contract_addr(other_side_address)
    }

    fn derive_htlc_key_pair(&self, swap_unique_data: &[u8]) -> KeyPair {
        self.platform_coin.derive_htlc_key_pair(swap_unique_data)
    }

    fn validate_other_pubkey(&self, raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> {
        self.platform_coin.validate_other_pubkey(raw_pubkey)
    }

    async fn maker_payment_instructions(
        &self,
        _secret_hash: &[u8],
        _amount: &BigDecimal,
        _maker_lock_duration: u64,
        _expires_in: u64,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        Ok(None)
    }

    async fn taker_payment_instructions(
        &self,
        _secret_hash: &[u8],
        _amount: &BigDecimal,
        _expires_in: u64,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        Ok(None)
    }

    fn validate_maker_payment_instructions(
        &self,
        _instructions: &[u8],
        _secret_hash: &[u8],
        _amount: BigDecimal,
        _maker_lock_duration: u64,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        MmError::err(ValidateInstructionsErr::UnsupportedCoin(self.ticker().to_string()))
    }

    fn validate_taker_payment_instructions(
        &self,
        _instructions: &[u8],
        _secret_hash: &[u8],
        _amount: BigDecimal,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        MmError::err(ValidateInstructionsErr::UnsupportedCoin(self.ticker().to_string()))
    }

    fn is_supported_by_watchers(&self) -> bool { false }
}

#[async_trait]
#[allow(unused_variables)]
impl WatcherOps for TendermintToken {
    fn create_taker_spends_maker_payment_preimage(
        &self,
        _maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_taker_spends_maker_payment_preimage(&self, preimage: &[u8], secret: &[u8]) -> TransactionFut {
        unimplemented!();
    }

    fn create_taker_refunds_payment_preimage(
        &self,
        _taker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_watcher_refunds_taker_payment_preimage(&self, _taker_refunds_payment: &[u8]) -> TransactionFut {
        unimplemented!();
    }

    fn watcher_validate_taker_fee(&self, _taker_fee_hash: Vec<u8>, _verified_pub: Vec<u8>) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    fn watcher_validate_taker_payment(&self, _input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }
}

impl MarketCoinOps for TendermintToken {
    fn ticker(&self) -> &str { &self.ticker }

    fn my_address(&self) -> MmResult<String, MyAddressError> { self.platform_coin.my_address() }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> {
        self.platform_coin.get_public_key()
    }

    fn sign_message_hash(&self, message: &str) -> Option<[u8; 32]> { self.platform_coin.sign_message_hash(message) }

    fn sign_message(&self, message: &str) -> SignatureResult<String> { self.platform_coin.sign_message(message) }

    fn verify_message(&self, signature: &str, message: &str, address: &str) -> VerificationResult<bool> {
        self.platform_coin.verify_message(signature, message, address)
    }

    fn my_balance(&self) -> BalanceFut<CoinBalance> {
        let coin = self.clone();
        let fut = async move {
            let balance_denom = coin.platform_coin.balance_for_denom(coin.denom.to_string()).await?;
            Ok(CoinBalance {
                spendable: big_decimal_from_sat_unsigned(balance_denom, coin.decimals),
                unspendable: BigDecimal::default(),
            })
        };
        Box::new(fut.boxed().compat())
    }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> { self.platform_coin.my_spendable_balance() }

    fn platform_ticker(&self) -> &str { self.platform_coin.ticker() }

    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
        self.platform_coin.send_raw_tx(tx)
    }

    fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> {
        self.platform_coin.send_raw_tx_bytes(tx)
    }

    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        confirmations: u64,
        requires_nota: bool,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        self.platform_coin
            .wait_for_confirmations(tx, confirmations, requires_nota, wait_until, check_every)
    }

    fn wait_for_htlc_tx_spend(
        &self,
        transaction: &[u8],
        secret_hash: &[u8],
        wait_until: u64,
        from_block: u64,
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        self.platform_coin.wait_for_htlc_tx_spend(
            transaction,
            secret_hash,
            wait_until,
            from_block,
            swap_contract_address,
        )
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        self.platform_coin.tx_enum_from_bytes(bytes)
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> { self.platform_coin.current_block() }

    fn display_priv_key(&self) -> Result<String, String> { self.platform_coin.display_priv_key() }

    fn min_tx_amount(&self) -> BigDecimal { big_decimal_from_sat(MIN_TX_SATOSHIS, self.decimals) }

    /// !! This function includes dummy implementation for P.O.C work
    fn min_trading_vol(&self) -> MmNumber { MmNumber::from("0.00777") }
}

#[async_trait]
#[allow(unused_variables)]
impl MmCoin for TendermintToken {
    fn is_asset_chain(&self) -> bool { false }

    fn spawner(&self) -> CoinFutSpawner { CoinFutSpawner::new(&self.abortable_system) }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        let platform = self.platform_coin.clone();
        let token = self.clone();
        let fut = async move {
            let to_address =
                AccountId::from_str(&req.to).map_to_mm(|e| WithdrawError::InvalidAddress(e.to_string()))?;
            if to_address.prefix() != platform.account_prefix {
                return MmError::err(WithdrawError::InvalidAddress(format!(
                    "expected {} address prefix",
                    platform.account_prefix
                )));
            }

            let base_denom_balance = platform.balance_for_denom(platform.denom.to_string()).await?;
            let base_denom_balance_dec = big_decimal_from_sat_unsigned(base_denom_balance, token.decimals());

            let balance_denom = platform.balance_for_denom(token.denom.to_string()).await?;
            let balance_dec = big_decimal_from_sat_unsigned(balance_denom, token.decimals());

            let (amount_denom, amount_dec, total_amount) = if req.max {
                (
                    balance_denom,
                    big_decimal_from_sat_unsigned(balance_denom, token.decimals),
                    balance_dec,
                )
            } else {
                if balance_dec < req.amount {
                    return MmError::err(WithdrawError::NotSufficientBalance {
                        coin: token.ticker.clone(),
                        available: balance_dec,
                        required: req.amount,
                    });
                }

                (
                    sat_from_big_decimal(&req.amount, token.decimals())?,
                    req.amount.clone(),
                    req.amount,
                )
            };

            if !platform.is_tx_amount_enough(token.decimals, &amount_dec) {
                return MmError::err(WithdrawError::AmountTooLow {
                    amount: amount_dec,
                    threshold: token.min_tx_amount(),
                });
            }

            let received_by_me = if to_address == platform.account_id {
                amount_dec
            } else {
                BigDecimal::default()
            };

            let msg_send = MsgSend {
                from_address: platform.account_id.clone(),
                to_address,
                amount: vec![Coin {
                    denom: token.denom.clone(),
                    amount: amount_denom.into(),
                }],
            }
            .to_any()
            .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let memo = req.memo.unwrap_or_else(|| TX_DEFAULT_MEMO.into());
            let current_block = token
                .current_block()
                .compat()
                .await
                .map_to_mm(WithdrawError::Transport)?;

            let _sequence_lock = platform.sequence_lock.lock().await;
            let account_info = platform.my_account_info().await?;

            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

            let simulated_tx = platform
                .gen_simulated_tx(account_info.clone(), msg_send.clone(), timeout_height, memo.clone())
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let fee_amount_u64 = platform.calculate_fee_amount_as_u64(simulated_tx).await?;
            let fee_amount_dec = big_decimal_from_sat_unsigned(fee_amount_u64, platform.decimals());

            if base_denom_balance < fee_amount_u64 {
                return MmError::err(WithdrawError::NotSufficientPlatformBalanceForFee {
                    coin: platform.ticker().to_string(),
                    available: base_denom_balance_dec,
                    required: fee_amount_dec,
                });
            }

            let fee_amount = Coin {
                denom: platform.denom.clone(),
                amount: fee_amount_u64.into(),
            };

            let fee = Fee::from_amount_and_gas(fee_amount, GAS_LIMIT_DEFAULT);

            let tx_raw = platform
                .any_to_signed_raw_tx(account_info, msg_send, fee, timeout_height, memo)
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let tx_bytes = tx_raw
                .to_bytes()
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let hash = sha256(&tx_bytes);
            Ok(TransactionDetails {
                tx_hash: hex::encode_upper(hash.as_slice()),
                tx_hex: tx_bytes.into(),
                from: vec![platform.account_id.to_string()],
                to: vec![req.to],
                my_balance_change: &received_by_me - &total_amount,
                spent_by_me: total_amount.clone(),
                total_amount,
                received_by_me,
                block_height: 0,
                timestamp: 0,
                fee_details: Some(TxFeeDetails::Tendermint(TendermintFeeDetails {
                    coin: platform.ticker().to_string(),
                    amount: fee_amount_dec,
                    uamount: fee_amount_u64,
                    gas_limit: GAS_LIMIT_DEFAULT,
                })),
                coin: token.ticker.clone(),
                internal_id: hash.to_vec().into(),
                kmd_rewards: None,
                transaction_type: TransactionType::default(),
            })
        };
        Box::new(fut.boxed().compat())
    }

    fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut {
        self.platform_coin.get_raw_transaction(req)
    }

    fn decimals(&self) -> u8 { self.decimals }

    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> {
        self.platform_coin.convert_to_address(from, to_address_format)
    }

    fn validate_address(&self, address: &str) -> ValidateAddressResult { self.platform_coin.validate_address(address) }

    fn process_history_loop(&self, ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> {
        warn!("process_history_loop is deprecated, tendermint uses tx_history_v2");
        Box::new(futures01::future::err(()))
    }

    fn history_sync_status(&self) -> HistorySyncState { HistorySyncState::NotEnabled }

    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> {
        Box::new(futures01::future::err("Not implemented".into()))
    }

    async fn get_sender_trade_fee(
        &self,
        value: TradePreimageValue,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        let amount = match value {
            TradePreimageValue::Exact(decimal) | TradePreimageValue::UpperBound(decimal) => decimal,
        };

        self.platform_coin
            .get_sender_trade_fee_for_denom(self.ticker.clone(), self.denom.clone(), self.decimals, amount)
            .await
    }

    fn get_receiver_trade_fee(&self, send_amount: BigDecimal, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        let token = self.clone();
        let fut = async move {
            // We can't simulate Claim Htlc without having information about broadcasted htlc tx.
            // Since create and claim htlc fees are almost same, we can simply simulate create htlc tx.
            token
                .platform_coin
                .get_sender_trade_fee_for_denom(token.ticker.clone(), token.denom.clone(), token.decimals, send_amount)
                .await
        };
        Box::new(fut.boxed().compat())
    }

    async fn get_fee_to_send_taker_fee(
        &self,
        dex_fee_amount: BigDecimal,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        self.platform_coin
            .get_fee_to_send_taker_fee_for_denom(self.ticker.clone(), self.denom.clone(), self.decimals, dex_fee_amount)
            .await
    }

    fn required_confirmations(&self) -> u64 { self.platform_coin.required_confirmations() }

    fn requires_notarization(&self) -> bool { self.platform_coin.requires_notarization() }

    fn set_required_confirmations(&self, confirmations: u64) {
        warn!("set_required_confirmations is not supported for tendermint")
    }

    fn set_requires_notarization(&self, requires_nota: bool) {
        self.platform_coin.set_requires_notarization(requires_nota)
    }

    fn swap_contract_address(&self) -> Option<BytesJson> { self.platform_coin.swap_contract_address() }

    fn fallback_swap_contract(&self) -> Option<BytesJson> { self.platform_coin.fallback_swap_contract() }

    fn mature_confirmations(&self) -> Option<u32> { None }

    fn coin_protocol_info(&self) -> Vec<u8> { self.platform_coin.coin_protocol_info() }

    fn is_coin_protocol_supported(&self, info: &Option<Vec<u8>>) -> bool {
        self.platform_coin.is_coin_protocol_supported(info)
    }

    fn on_disabled(&self) -> Result<(), AbortedError> { AbortableSystem::abort_all(&self.abortable_system) }

    fn on_token_deactivated(&self, _ticker: &str) {}
}
