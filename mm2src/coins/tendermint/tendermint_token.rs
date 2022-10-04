/// Module containing implementation for Tendermint Tokens. They include native assets + IBC
use super::{upper_hex, TendermintCoin, TendermintFeeDetails, GAS_LIMIT_DEFAULT, TIMEOUT_HEIGHT_DELTA};
use crate::tendermint::TX_DEFAULT_MEMO;
use crate::{big_decimal_from_sat_unsigned, utxo::sat_from_big_decimal, BalanceFut, BigDecimal, CoinBalance,
            FeeApproxStage, FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin, MyAddressError,
            NegotiateSwapContractAddrErr, RawTransactionFut, RawTransactionRequest, SearchForSwapTxSpendInput,
            SignatureResult, SwapOps, TradeFee, TradePreimageFut, TradePreimageResult, TradePreimageValue,
            TransactionDetails, TransactionEnum, TransactionErr, TransactionFut, TransactionType, TxFeeDetails,
            TxMarshalingErr, UnexpectedDerivationMethod, ValidateAddressResult, ValidatePaymentError,
            ValidatePaymentFut, ValidatePaymentInput, VerificationResult, WatcherValidatePaymentInput, WithdrawError,
            WithdrawFut, WithdrawRequest};
use async_trait::async_trait;
use bitcrypto::sha256;
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
use std::str::FromStr;

#[allow(dead_code)]
#[derive(Clone)]
pub struct TendermintToken {
    pub ticker: String,
    platform_coin: TendermintCoin,
    pub decimals: u8,
    pub denom: Denom,
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
    InvalidDenom(String),
    MyAddressError(String),
    CouldNotFetchBalance(String),
}

impl From<MyAddressError> for TendermintTokenInitError {
    fn from(err: MyAddressError) -> Self { TendermintTokenInitError::MyAddressError(err.to_string()) }
}

impl TendermintToken {
    pub fn new(
        ticker: String,
        platform_coin: TendermintCoin,
        decimals: u8,
        denom: String,
    ) -> MmResult<Self, TendermintTokenInitError> {
        let denom = Denom::from_str(&denom).map_to_mm(|e| TendermintTokenInitError::InvalidDenom(e.to_string()))?;
        Ok(TendermintToken {
            ticker,
            platform_coin,
            decimals,
            denom,
        })
    }
}

#[async_trait]
#[allow(unused_variables)]
impl SwapOps for TendermintToken {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, uuid: &[u8]) -> TransactionFut {
        self.platform_coin
            .send_taker_fee_for_denom(fee_addr, amount, self.denom.clone(), self.decimals, uuid)
    }

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        self.platform_coin.send_htlc_for_denom(
            time_lock,
            taker_pub,
            secret_hash,
            amount,
            self.denom.clone(),
            self.decimals,
        )
    }

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        self.platform_coin.send_htlc_for_denom(
            time_lock,
            maker_pub,
            secret_hash,
            amount,
            self.denom.clone(),
            self.decimals,
        )
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        self.platform_coin.send_maker_spends_taker_payment(
            taker_payment_tx,
            time_lock,
            taker_pub,
            secret,
            secret_hash,
            swap_contract_address,
            swap_unique_data,
        )
    }

    fn create_taker_spends_maker_payment_preimage(
        &self,
        _maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
    }

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        self.platform_coin.send_taker_spends_maker_payment(
            maker_payment_tx,
            time_lock,
            maker_pub,
            secret,
            secret_hash,
            swap_contract_address,
            swap_unique_data,
        )
    }

    fn send_taker_spends_maker_payment_preimage(&self, preimage: &[u8], secret: &[u8]) -> TransactionFut { todo!() }

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        Box::new(futures01::future::err(TransactionErr::Plain(
            "Doesn't need transaction broadcast to be refunded".into(),
        )))
    }

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        Box::new(futures01::future::err(TransactionErr::Plain(
            "Doesn't need transaction broadcast to be refunded".into(),
        )))
    }

    fn validate_fee(
        &self,
        fee_tx: &TransactionEnum,
        expected_sender: &[u8],
        fee_addr: &[u8],
        amount: &BigDecimal,
        min_block_number: u64,
        uuid: &[u8],
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        self.platform_coin.validate_fee_for_denom(
            fee_tx,
            expected_sender,
            fee_addr,
            amount,
            self.decimals,
            uuid,
            self.denom.to_string(),
        )
    }

    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        let fut = async move { Ok(()) };
        Box::new(fut.boxed().compat())
    }

    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        let fut = async move { Ok(()) };
        Box::new(fut.boxed().compat())
    }

    fn watcher_validate_taker_payment(
        &self,
        _input: WatcherValidatePaymentInput,
    ) -> Box<dyn Future<Item = (), Error = MmError<ValidatePaymentError>> + Send> {
        todo!()
    }

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        search_from_block: u64,
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        let fut = async move { Ok(None) };
        Box::new(fut.boxed().compat())
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

    fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> {
        self.platform_coin.extract_secret(secret_hash, spend_tx)
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
}

#[allow(unused_variables)]
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

    /// !! This function includes dummy implementation for P.O.C work
    fn min_tx_amount(&self) -> BigDecimal { BigDecimal::from(0) }

    /// !! This function includes dummy implementation for P.O.C work
    fn min_trading_vol(&self) -> MmNumber { MmNumber::from("0.00777") }
}

#[async_trait]
#[allow(unused_variables)]
impl MmCoin for TendermintToken {
    fn is_asset_chain(&self) -> bool { false }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        let coin = self.platform_coin.clone();
        let token = self.clone();
        let fut = async move {
            let to_address =
                AccountId::from_str(&req.to).map_to_mm(|e| WithdrawError::InvalidAddress(e.to_string()))?;
            if to_address.prefix() != coin.account_prefix {
                return MmError::err(WithdrawError::InvalidAddress(format!(
                    "expected {} address prefix",
                    coin.account_prefix
                )));
            }

            let base_denom_balance = coin.balance_for_denom(coin.denom.to_string()).await?;
            let base_denom_balance_dec = big_decimal_from_sat_unsigned(base_denom_balance, token.decimals());

            // TODO calculate current fee instead of using hard-coded value
            let fee_denom = 50000;
            let fee_amount_dec = big_decimal_from_sat_unsigned(fee_denom, coin.decimals());

            if base_denom_balance < fee_denom {
                return MmError::err(WithdrawError::NotSufficientBalanceForFee {
                    coin: coin.ticker().to_string(),
                    available: base_denom_balance_dec,
                    required: fee_amount_dec,
                });
            }

            let balance_denom = coin.balance_for_denom(token.denom.to_string()).await?;
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

            let received_by_me = if to_address == coin.account_id {
                amount_dec
            } else {
                BigDecimal::default()
            };

            let msg_send = MsgSend {
                from_address: coin.account_id.clone(),
                to_address,
                amount: vec![Coin {
                    denom: token.denom.clone(),
                    amount: amount_denom.into(),
                }],
            }
            .to_any()
            .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let current_block = token
                .current_block()
                .compat()
                .await
                .map_to_mm(WithdrawError::Transport)?;

            let _sequence_lock = coin.sequence_lock.lock().await;
            let account_info = coin.my_account_info().await?;

            let fee_amount = Coin {
                denom: coin.denom.clone(),
                amount: fee_denom.into(),
            };

            let fee = Fee::from_amount_and_gas(fee_amount, GAS_LIMIT_DEFAULT);
            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

            let tx_raw = coin
                .any_to_signed_raw_tx(account_info, msg_send, fee, timeout_height, TX_DEFAULT_MEMO.into())
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let tx_bytes = tx_raw
                .to_bytes()
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let hash = sha256(&tx_bytes);
            Ok(TransactionDetails {
                tx_hash: upper_hex(hash.as_slice()),
                tx_hex: tx_bytes.into(),
                from: vec![coin.account_id.to_string()],
                to: vec![req.to],
                my_balance_change: &received_by_me - &total_amount,
                spent_by_me: total_amount.clone(),
                total_amount,
                received_by_me,
                block_height: 0,
                timestamp: 0,
                fee_details: Some(TxFeeDetails::Tendermint(TendermintFeeDetails {
                    coin: coin.ticker().to_string(),
                    amount: fee_amount_dec,
                    gas_limit: GAS_LIMIT_DEFAULT,
                })),
                coin: token.ticker,
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
        Ok(TradeFee {
            coin: self.platform_coin.ticker().into(),
            amount: "0.0002".into(),
            paid_from_trading_vol: false,
        })
    }

    fn get_receiver_trade_fee(&self, stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        Box::new(futures01::future::ok(TradeFee {
            coin: self.platform_coin.ticker().into(),
            amount: "0.0002".into(),
            paid_from_trading_vol: false,
        }))
    }

    async fn get_fee_to_send_taker_fee(
        &self,
        dex_fee_amount: BigDecimal,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        Ok(TradeFee {
            coin: self.platform_coin.ticker().into(),
            amount: "0.0002".into(),
            paid_from_trading_vol: false,
        })
    }

    fn required_confirmations(&self) -> u64 { self.platform_coin.required_confirmations() }

    fn requires_notarization(&self) -> bool { self.platform_coin.requires_notarization() }

    fn set_required_confirmations(&self, confirmations: u64) { todo!() }

    fn set_requires_notarization(&self, requires_nota: bool) { todo!() }

    fn swap_contract_address(&self) -> Option<BytesJson> { None }

    fn mature_confirmations(&self) -> Option<u32> { None }

    fn coin_protocol_info(&self) -> Vec<u8> { self.platform_coin.coin_protocol_info() }

    fn is_coin_protocol_supported(&self, info: &Option<Vec<u8>>) -> bool {
        self.platform_coin.is_coin_protocol_supported(info)
    }
}
