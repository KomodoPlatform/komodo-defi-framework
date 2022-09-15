use super::TendermintCoin;
use crate::{big_decimal_from_sat_unsigned, BalanceFut, BigDecimal, CoinBalance, FeeApproxStage, FoundSwapTxSpend,
            HistorySyncState, MarketCoinOps, MmCoin, NegotiateSwapContractAddrErr, RawTransactionFut,
            RawTransactionRequest, SearchForSwapTxSpendInput, SignatureResult, SwapOps, TradeFee, TradePreimageFut,
            TradePreimageResult, TradePreimageValue, TransactionEnum, TransactionFut, TxMarshalingErr,
            UnexpectedDerivationMethod, ValidateAddressResult, ValidatePaymentInput, VerificationResult, WithdrawFut,
            WithdrawRequest};
use async_trait::async_trait;
use cosmrs::Denom;
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
pub struct TendermintIbcAsset {
    pub ticker: String,
    platform_coin: TendermintCoin,
    pub decimals: u8,
    pub denom: Denom,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IbcAssetProtocolInfo {
    pub platform: String,
    pub decimals: u8,
    pub denom: String,
}

#[derive(Clone, Deserialize)]
pub struct IbcAssetActivationParams {}

pub enum IbcAssetInitError {
    InvalidDenom(String),
}

impl TendermintIbcAsset {
    pub fn new(
        ticker: String,
        platform_coin: TendermintCoin,
        decimals: u8,
        denom: String,
    ) -> MmResult<Self, IbcAssetInitError> {
        let denom = Denom::from_str(&denom).map_to_mm(|e| IbcAssetInitError::InvalidDenom(e.to_string()))?;
        Ok(TendermintIbcAsset {
            ticker,
            platform_coin,
            decimals,
            denom,
        })
    }
}

#[async_trait]
#[allow(unused_variables)]
impl SwapOps for TendermintIbcAsset {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, uuid: &[u8]) -> TransactionFut { todo!() }

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
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
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        todo!()
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
        todo!()
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
        todo!()
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
        todo!()
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
        fee_tx: &TransactionEnum,
        expected_sender: &[u8],
        fee_addr: &[u8],
        amount: &BigDecimal,
        min_block_number: u64,
        uuid: &[u8],
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
        other_pub: &[u8],
        secret_hash: &[u8],
        search_from_block: u64,
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
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

    fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> { todo!() }

    fn negotiate_swap_contract_addr(
        &self,
        other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        todo!()
    }

    fn derive_htlc_key_pair(&self, swap_unique_data: &[u8]) -> KeyPair { todo!() }
}

#[allow(unused_variables)]
impl MarketCoinOps for TendermintIbcAsset {
    fn ticker(&self) -> &str { &self.ticker }

    fn my_address(&self) -> Result<String, String> { self.platform_coin.my_address() }

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

    fn wait_for_tx_spend(
        &self,
        transaction: &[u8],
        wait_until: u64,
        from_block: u64,
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        todo!()
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
impl MmCoin for TendermintIbcAsset {
    fn is_asset_chain(&self) -> bool { false }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut { todo!() }

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
