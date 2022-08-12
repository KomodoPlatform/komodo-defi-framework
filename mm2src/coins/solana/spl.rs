use super::{CoinBalance, HistorySyncState, MarketCoinOps, MmCoin, SwapOps, TradeFee, TransactionEnum};
use crate::solana::solana_common::{ui_amount_to_amount, PrepareTransferData, SufficientBalanceError};
use crate::solana::{solana_common, AccountError, SolanaCommonOps, SolanaFeeDetails};
use crate::{BalanceFut, FeeApproxStage, FoundSwapTxSpend, NegotiateSwapContractAddrErr, RawTransactionFut,
            RawTransactionRequest, SearchForSwapTxSpendInput, SignatureResult, SolanaCoin, TradePreimageFut,
            TradePreimageResult, TradePreimageValue, TransactionDetails, TransactionFut, TransactionType,
            TxMarshalingErr, UnexpectedDerivationMethod, ValidateAddressResult, ValidatePaymentInput,
            VerificationResult, WithdrawError, WithdrawFut, WithdrawRequest, WithdrawResult};
use async_trait::async_trait;
use bincode::serialize;
use common::{async_blocking, now_ms};
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
}

#[derive(Debug)]
pub struct SplTokenConf {
    pub decimals: u8,
    pub ticker: String,
    pub token_contract_address: Pubkey,
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
    pub conf: Arc<SplTokenConf>,
    pub platform_coin: SolanaCoin,
}

impl Debug for SplToken {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult { f.write_str(&*self.conf.ticker) }
}

impl SplToken {
    pub fn new(
        decimals: u8,
        ticker: String,
        token_address: String,
        platform_coin: SolanaCoin,
    ) -> Result<SplToken, MmError<SplTokenCreationError>> {
        let token_contract_address = solana_sdk::pubkey::Pubkey::from_str(&token_address)
            .map_err(|e| MmError::new(SplTokenCreationError::InvalidPubkey(format!("{:?}", e))))?;
        let conf = Arc::new(SplTokenConf {
            decimals,
            ticker,
            token_contract_address,
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
        timestamp: now_ms() / 1000,
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
    })
}

async fn withdraw_impl(coin: SplToken, req: WithdrawRequest) -> WithdrawResult {
    let validate_address_result = coin.validate_address(&*req.to);
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
        Ok(Pubkey::from_str(&*token_accounts[0].pubkey)?)
    }

    fn my_balance_impl(&self) -> BalanceFut<CoinBalance> {
        let coin = self.clone();
        let fut = async move {
            coin.platform_coin
                .my_balance_spl(&SplTokenInfo {
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

    fn my_address(&self) -> Result<String, String> { Ok(self.platform_coin.my_address.clone()) }

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

    fn wait_for_confirmations(
        &self,
        _tx: &[u8],
        _confirmations: u64,
        _requires_nota: bool,
        _wait_until: u64,
        _check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn wait_for_tx_spend(
        &self,
        _transaction: &[u8],
        _wait_until: u64,
        _from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        unimplemented!()
    }

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

#[allow(clippy::forget_ref, clippy::forget_copy, clippy::cast_ref_to_mut)]
#[async_trait]
impl SwapOps for SplToken {
    fn send_taker_fee(&self, _fee_addr: &[u8], amount: BigDecimal, _uuid: &[u8]) -> TransactionFut { unimplemented!() }

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!()
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
        unimplemented!()
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!()
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
        todo!()
    }

    fn validate_fee(
        &self,
        _fee_tx: &TransactionEnum,
        _expected_sender: &[u8],
        _fee_addr: &[u8],
        _amount: &BigDecimal,
        _min_block_number: u64,
        _uuid: &[u8],
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
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

    fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> { unimplemented!() }

    fn negotiate_swap_contract_addr(
        &self,
        _other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        unimplemented!()
    }

    fn derive_htlc_key_pair(&self, _swap_unique_data: &[u8]) -> KeyPair { todo!() }
}

#[allow(clippy::forget_ref, clippy::forget_copy, clippy::cast_ref_to_mut)]
#[async_trait]
impl MmCoin for SplToken {
    fn is_asset_chain(&self) -> bool { false }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        Box::new(Box::pin(withdraw_impl(self.clone(), req)).compat())
    }

    fn get_raw_transaction(&self, _req: RawTransactionRequest) -> RawTransactionFut { unimplemented!() }

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
        _dex_fee_amount: BigDecimal,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        unimplemented!()
    }

    fn required_confirmations(&self) -> u64 { 1 }

    fn requires_notarization(&self) -> bool { false }

    fn set_required_confirmations(&self, _confirmations: u64) { unimplemented!() }

    fn set_requires_notarization(&self, _requires_nota: bool) { unimplemented!() }

    fn swap_contract_address(&self) -> Option<BytesJson> { unimplemented!() }

    fn mature_confirmations(&self) -> Option<u32> { Some(1) }

    fn coin_protocol_info(&self) -> Vec<u8> { Vec::new() }

    fn is_coin_protocol_supported(&self, _info: &Option<Vec<u8>>) -> bool { true }
}
