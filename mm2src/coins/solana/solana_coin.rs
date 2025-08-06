use std::ops::Deref;
use std::sync::Arc;

use common::executor::{abortable_queue::WeakSpawner, AbortedError};
use futures01::Future;
use mm2_core::mm_ctx::MmArc;
use mm2_number::MmNumber;
use rpc::v1::types::Bytes as RpcBytes;
use async_trait::async_trait;

use crate::{
    DexFee, FeeApproxStage, HistorySyncState, MmCoin, RawTransactionFut, RawTransactionRequest, TradeFee,
    TradePreimageResult, TradePreimageValue, ValidateAddressResult, WithdrawFut, WithdrawRequest,
};

#[derive(Clone)]
pub struct SolanaCoin(Arc<SolanaCoinFields>);

pub struct SolanaCoinFields {}

impl Deref for SolanaCoin {
    type Target = SolanaCoinFields;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[async_trait]
impl MmCoin for SolanaCoin {
    fn is_asset_chain(&self) -> bool {
        todo!()
    }

    fn wallet_only(&self, ctx: &MmArc) -> bool {
        todo!()
    }

    fn spawner(&self) -> WeakSpawner {
        todo!()
    }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        todo!()
    }

    fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut {
        todo!()
    }

    fn get_tx_hex_by_hash(&self, tx_hash: Vec<u8>) -> RawTransactionFut {
        todo!()
    }

    fn decimals(&self) -> u8 {
        todo!()
    }

    fn convert_to_address(&self, from: &str, to_address_format: serde_json::Value) -> Result<String, String> {
        todo!()
    }

    fn validate_address(&self, address: &str) -> ValidateAddressResult {
        todo!()
    }

    fn process_history_loop(&self, ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> {
        todo!()
    }

    fn history_sync_status(&self) -> HistorySyncState {
        todo!()
    }

    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> {
        todo!()
    }

    async fn get_sender_trade_fee(
        &self,
        value: TradePreimageValue,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        todo!()
    }

    fn get_receiver_trade_fee(&self, stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        todo!()
    }

    async fn get_fee_to_send_taker_fee(
        &self,
        dex_fee_amount: DexFee,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        todo!()
    }

    fn required_confirmations(&self) -> u64 {
        todo!()
    }

    fn requires_notarization(&self) -> bool {
        todo!()
    }

    fn set_required_confirmations(&self, confirmations: u64) {
        todo!()
    }

    fn set_requires_notarization(&self, requires_nota: bool) {
        todo!()
    }

    fn swap_contract_address(&self) -> Option<RpcBytes> {
        todo!()
    }

    fn fallback_swap_contract(&self) -> Option<RpcBytes> {
        todo!()
    }

    fn mature_confirmations(&self) -> Option<u32> {
        todo!()
    }

    fn coin_protocol_info(&self, amount_to_receive: Option<MmNumber>) -> Vec<u8> {
        todo!()
    }

    fn is_coin_protocol_supported(
        &self,
        info: &Option<Vec<u8>>,
        amount_to_send: Option<MmNumber>,
        locktime: u64,
        is_maker: bool,
    ) -> bool {
        todo!()
    }

    fn on_disabled(&self) -> Result<(), AbortedError> {
        todo!()
    }

    fn on_token_deactivated(&self, ticker: &str) {
        todo!()
    }
}
