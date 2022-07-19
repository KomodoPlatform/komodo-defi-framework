use super::EthCoin;
use crate::{BalanceFut, CoinBalance, FeeApproxStage, FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin,
            NegotiateSwapContractAddrErr, RawTransactionFut, RawTransactionRequest, SearchForSwapTxSpendInput,
            SignatureResult, SwapOps, TradeFee, TradePreimageFut, TradePreimageResult, TradePreimageValue,
            TransactionEnum, TransactionFut, UnexpectedDerivationMethod, ValidateAddressResult, ValidatePaymentInput,
            VerificationResult, WithdrawFut, WithdrawRequest};
use async_trait::async_trait;
use ethereum_types::Address;
use futures01::Future;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::{BigDecimal, MmNumber};
use rpc::v1::types::Bytes as BytesJson;
use serde_json::Value as Json;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Erc20TokenConf {
    pub decimals: u8,
    pub ticker: String,
    pub token_contract_address: Address,
}

#[derive(Clone, Debug)]
pub struct Erc20Token {
    pub conf: Arc<Erc20TokenConf>,
    pub platform_coin: EthCoin,
}

#[async_trait]
impl MmCoin for Erc20Token {
    fn is_asset_chain(&self) -> bool { todo!() }

    fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut { todo!() }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut { todo!() }

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

    fn set_requires_notarization(&self, _requires_nota: bool) { todo!() }

    fn swap_contract_address(&self) -> Option<BytesJson> { todo!() }

    fn mature_confirmations(&self) -> Option<u32> { todo!() }

    fn coin_protocol_info(&self) -> Vec<u8> { todo!() }

    fn is_coin_protocol_supported(&self, _info: &Option<Vec<u8>>) -> bool { todo!() }
}

#[async_trait]
impl SwapOps for Erc20Token {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, _uuid: &[u8]) -> TransactionFut { todo!() }

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
    }

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        _time_lock: u32,
        _taker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
    }

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
    }

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
    }

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        _time_lock: u32,
        _taker_pub: &[u8],
        _secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
    }

    fn validate_fee(
        &self,
        fee_tx: &TransactionEnum,
        expected_sender: &[u8],
        fee_addr: &[u8],
        amount: &BigDecimal,
        min_block_number: u64,
        _uuid: &[u8],
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        todo!()
    }

    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        todo!()
    }

    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        todo!()
    }

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        _other_pub: &[u8],
        secret_hash: &[u8],
        from_block: u64,
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
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

    fn extract_secret(&self, _secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> { todo!() }

    fn negotiate_swap_contract_addr(
        &self,
        other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        todo!()
    }

    fn derive_htlc_key_pair(&self, _swap_unique_data: &[u8]) -> keys::KeyPair { todo!() }
}

#[cfg_attr(test, mocktopus::macros::mockable)]
impl MarketCoinOps for Erc20Token {
    fn ticker(&self) -> &str { todo!() }

    fn my_address(&self) -> Result<String, String> { todo!() }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> { todo!() }

    fn sign_message_hash(&self, message: &str) -> Option<[u8; 32]> { todo!() }

    fn sign_message(&self, message: &str) -> SignatureResult<String> { todo!() }

    fn verify_message(&self, signature: &str, message: &str, address: &str) -> VerificationResult<bool> { todo!() }

    fn my_balance(&self) -> BalanceFut<CoinBalance> { todo!() }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> { todo!() }

    fn platform_ticker(&self) -> &str { todo!() }

    fn send_raw_tx(&self, mut tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> { todo!() }

    fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> { todo!() }

    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        confirmations: u64,
        _requires_nota: bool,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        todo!()
    }

    fn wait_for_tx_spend(
        &self,
        tx_bytes: &[u8],
        wait_until: u64,
        from_block: u64,
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        todo!()
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String> { todo!() }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> { todo!() }

    fn display_priv_key(&self) -> Result<String, String> { todo!() }

    fn min_tx_amount(&self) -> BigDecimal { todo!() }

    fn min_trading_vol(&self) -> MmNumber { todo!() }
}
