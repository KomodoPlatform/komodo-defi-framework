use bigdecimal::BigDecimal;
use common::mm_ctx::MmArc;
use common::mm_number::MmNumber;
use crate::{TradeInfo, FoundSwapTxSpend, WithdrawRequest};
use futures01::Future;
use mocktopus::macros::*;
use std::borrow::Cow;
use super::{HistorySyncState, MarketCoinOps, MmCoin, SwapOps, TradeFee, TransactionDetails, TransactionEnum, TransactionFut};

/// Dummy coin struct used in tests which functions are unimplemented but then mocked
/// in specific test to emulate the required behavior
#[derive(Clone, Debug)]
pub struct TestCoin {}

#[mockable]
impl MarketCoinOps for TestCoin {
    fn ticker (&self) -> &str {
        unimplemented!()
    }

    fn my_address(&self) -> Cow<str> {
        unimplemented!()
    }

    fn my_balance(&self) -> Box<dyn Future<Item=BigDecimal, Error=String> + Send> {
        unimplemented!()
    }

    /// Receives raw transaction bytes in hexadecimal format as input and returns tx hash in hexadecimal format
    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item=String, Error=String> + Send> {
        unimplemented!()
    }

    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        confirmations: u64,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item=(), Error=String> + Send> {
        unimplemented!()
    }

    fn wait_for_tx_spend(&self, transaction: &[u8], wait_until: u64, from_block: u64) -> TransactionFut {
        unimplemented!()
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String> {
        unimplemented!()
    }

    fn current_block(&self) -> Box<dyn Future<Item=u64, Error=String> + Send> {
        unimplemented!()
    }

    fn address_from_pubkey_str(&self, pubkey: &str) -> Result<String, String> {
        unimplemented!()
    }

    fn display_priv_key(&self) -> String {
        unimplemented!()
    }
}

#[mockable]
impl SwapOps for TestCoin {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal) -> TransactionFut {
        unimplemented!()
    }

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret: &[u8],
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret: &[u8],
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
    ) -> TransactionFut {
        unimplemented!()
    }

    fn validate_fee(
        &self,
        fee_tx: &TransactionEnum,
        fee_addr: &[u8],
        amount: &BigDecimal,
    ) -> Box<dyn Future<Item=(), Error=String> + Send> {
        unimplemented!()
    }

    fn validate_maker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
    ) -> Box<dyn Future<Item=(), Error=String> + Send> {
        unimplemented!()
    }

    fn validate_taker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
    ) -> Box<dyn Future<Item=(), Error=String> + Send> {
        unimplemented!()
    }

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        search_from_block: u64,
    ) -> Box<dyn Future<Item=Option<TransactionEnum>, Error=String> + Send> {
        unimplemented!()
    }

    fn search_for_swap_tx_spend_my(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        tx: &[u8],
        search_from_block: u64,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!()
    }

    fn search_for_swap_tx_spend_other(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        tx: &[u8],
        search_from_block: u64,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!()
    }
}

#[mockable]
impl MmCoin for TestCoin {
    fn is_asset_chain(&self) -> bool {
        unimplemented!()
    }

    fn check_i_have_enough_to_trade(&self, amount: &MmNumber, balance: &MmNumber, trade_info: TradeInfo) -> Box<dyn Future<Item=(), Error=String> + Send> {
        unimplemented!()
    }

    fn can_i_spend_other_payment(&self) -> Box<dyn Future<Item=(), Error=String> + Send> {
        unimplemented!()
    }

    fn withdraw(&self, req: WithdrawRequest) -> Box<dyn Future<Item=TransactionDetails, Error=String> + Send> {
        unimplemented!()
    }

    fn decimals(&self) -> u8 {
        unimplemented!()
    }

    fn process_history_loop(&self, ctx: MmArc) {
        unimplemented!()
    }

    fn tx_details_by_hash(&self, hash: &[u8]) -> Box<dyn Future<Item=TransactionDetails, Error=String> + Send> {
        unimplemented!()
    }

    fn history_sync_status(&self) -> HistorySyncState {
        unimplemented!()
    }

    /// Get fee to be paid per 1 swap transaction
    fn get_trade_fee(&self) -> Box<dyn Future<Item=TradeFee, Error=String> + Send> {
        unimplemented!()
    }

    fn required_confirmations(&self) -> u64 {
        unimplemented!()
    }

    fn requires_notarization(&self) -> bool { unimplemented!() }

    fn set_required_confirmations(&self, _confirmations: u64) {
        unimplemented!()
    }

    fn set_requires_notarization(&self, _requires_nota: bool) { unimplemented!() }
}
