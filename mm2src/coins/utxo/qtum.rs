use super::*;
use crate::{SwapOps, ValidateAddressResult};
use common::mm_metrics::MetricsArc;
use futures::{FutureExt, TryFutureExt};

pub const QTUM_STANDARD_DUST: u64 = 1000;

#[derive(Clone, Debug)]
pub struct QtumCoin {
    utxo_arc: UtxoArc,
}

impl AsRef<UtxoCoinFields> for QtumCoin {
    fn as_ref(&self) -> &UtxoCoinFields { &self.utxo_arc }
}

impl From<UtxoArc> for QtumCoin {
    fn from(coin: UtxoArc) -> QtumCoin { QtumCoin { utxo_arc: coin } }
}

impl From<QtumCoin> for UtxoArc {
    fn from(coin: QtumCoin) -> Self { coin.utxo_arc }
}

pub async fn qtum_coin_from_conf_and_request(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    req: &Json,
    priv_key: &[u8],
) -> Result<QtumCoin, String> {
    let inner = try_s!(utxo_arc_from_conf_and_request(ctx, ticker, conf, req, priv_key, QTUM_STANDARD_DUST).await);
    Ok(inner.into())
}

#[cfg_attr(test, mockable)]
#[async_trait]
impl UtxoCommonOps for QtumCoin {
    async fn get_tx_fee(&self) -> Result<ActualTxFee, JsonRpcError> { utxo_common::get_tx_fee(&self.utxo_arc).await }

    async fn get_htlc_spend_fee(&self) -> Result<u64, String> { utxo_common::get_htlc_spend_fee(self).await }

    fn addresses_from_script(&self, script: &Script) -> Result<Vec<Address>, String> {
        utxo_common::addresses_from_script(&self.utxo_arc, script)
    }

    fn denominate_satoshis(&self, satoshi: i64) -> f64 { utxo_common::denominate_satoshis(&self.utxo_arc, satoshi) }

    fn my_public_key(&self) -> &Public { self.utxo_arc.key_pair.public() }

    fn display_address(&self, address: &Address) -> Result<String, String> {
        utxo_common::display_address(&self.utxo_arc, address)
    }

    fn address_from_str(&self, address: &str) -> Result<Address, String> {
        utxo_common::address_from_str(&self.utxo_arc, address)
    }

    async fn get_current_mtp(&self) -> Result<u32, String> { utxo_common::get_current_mtp(&self.utxo_arc).await }

    fn is_unspent_mature(&self, output: &RpcTransaction) -> bool {
        is_qtum_unspent_mature(self.utxo_arc.mature_confirmations, output)
    }

    async fn generate_transaction(
        &self,
        utxos: Vec<UnspentInfo>,
        outputs: Vec<TransactionOutput>,
        fee_policy: FeePolicy,
        fee: Option<ActualTxFee>,
        gas_fee: Option<u64>,
    ) -> Result<(TransactionInputSigner, AdditionalTxData), GenerateTransactionError> {
        utxo_common::generate_transaction(self, utxos, outputs, fee_policy, fee, gas_fee).await
    }

    async fn calc_interest_if_required(
        &self,
        unsigned: TransactionInputSigner,
        data: AdditionalTxData,
        my_script_pub: Bytes,
    ) -> Result<(TransactionInputSigner, AdditionalTxData), String> {
        utxo_common::calc_interest_if_required(self, unsigned, data, my_script_pub).await
    }

    fn p2sh_spending_tx(
        &self,
        prev_transaction: UtxoTx,
        redeem_script: Bytes,
        outputs: Vec<TransactionOutput>,
        script_data: Script,
        sequence: u32,
    ) -> Result<UtxoTx, String> {
        utxo_common::p2sh_spending_tx(
            &self.utxo_arc,
            prev_transaction,
            redeem_script,
            outputs,
            script_data,
            sequence,
        )
    }

    fn ordered_mature_unspents(
        &self,
        address: &Address,
    ) -> Box<dyn Future<Item = Vec<UnspentInfo>, Error = String> + Send> {
        Box::new(
            utxo_common::ordered_mature_unspents(self.clone(), address.clone())
                .boxed()
                .compat(),
        )
    }

    fn get_verbose_transaction_from_cache_or_rpc(
        &self,
        txid: H256Json,
    ) -> Box<dyn Future<Item = VerboseTransactionFrom, Error = String> + Send> {
        let selfi = self.clone();
        let fut = async move { utxo_common::get_verbose_transaction_from_cache_or_rpc(&selfi.utxo_arc, txid).await };
        Box::new(fut.boxed().compat())
    }

    async fn cache_transaction_if_possible(&self, tx: &RpcTransaction) -> Result<(), String> {
        utxo_common::cache_transaction_if_possible(&self.utxo_arc, tx).await
    }

    async fn list_unspent_ordered<'a>(
        &'a self,
        address: &Address,
    ) -> Result<(Vec<UnspentInfo>, AsyncMutexGuard<'a, RecentlySpentOutPoints>), String> {
        utxo_common::list_unspent_ordered(self, address).await
    }
}

#[async_trait]
impl UtxoStandardOps for QtumCoin {
    async fn tx_details_by_hash(&self, hash: &[u8]) -> Result<TransactionDetails, String> {
        utxo_common::tx_details_by_hash(self, hash).await
    }

    async fn request_tx_history(&self, metrics: MetricsArc) -> RequestTxHistoryResult {
        utxo_common::request_tx_history(self, metrics).await
    }
}

impl SwapOps for QtumCoin {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal) -> TransactionFut {
        utxo_common::send_taker_fee(self.clone(), fee_addr, amount)
    }

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
    ) -> TransactionFut {
        utxo_common::send_maker_payment(self.clone(), time_lock, taker_pub, secret_hash, amount)
    }

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
    ) -> TransactionFut {
        utxo_common::send_taker_payment(self.clone(), time_lock, maker_pub, secret_hash, amount)
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret: &[u8],
    ) -> TransactionFut {
        utxo_common::send_maker_spends_taker_payment(self.clone(), taker_payment_tx, time_lock, taker_pub, secret)
    }

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret: &[u8],
    ) -> TransactionFut {
        utxo_common::send_taker_spends_maker_payment(self.clone(), maker_payment_tx, time_lock, maker_pub, secret)
    }

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
    ) -> TransactionFut {
        utxo_common::send_taker_refunds_payment(self.clone(), taker_payment_tx, time_lock, maker_pub, secret_hash)
    }

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
    ) -> TransactionFut {
        utxo_common::send_maker_refunds_payment(self.clone(), maker_payment_tx, time_lock, taker_pub, secret_hash)
    }

    fn validate_fee(
        &self,
        fee_tx: &TransactionEnum,
        fee_addr: &[u8],
        amount: &BigDecimal,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        utxo_common::validate_fee(self.clone(), fee_tx, fee_addr, amount)
    }

    fn validate_maker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        utxo_common::validate_maker_payment(self, payment_tx, time_lock, maker_pub, priv_bn_hash, amount)
    }

    fn validate_taker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        utxo_common::validate_taker_payment(self, payment_tx, time_lock, taker_pub, priv_bn_hash, amount)
    }

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        _search_from_block: u64,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        utxo_common::check_if_my_payment_sent(self.clone(), time_lock, other_pub, secret_hash)
    }

    fn search_for_swap_tx_spend_my(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        tx: &[u8],
        search_from_block: u64,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        utxo_common::search_for_swap_tx_spend_my(
            &self.utxo_arc,
            time_lock,
            other_pub,
            secret_hash,
            tx,
            search_from_block,
        )
    }

    fn search_for_swap_tx_spend_other(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        tx: &[u8],
        search_from_block: u64,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        utxo_common::search_for_swap_tx_spend_other(
            &self.utxo_arc,
            time_lock,
            other_pub,
            secret_hash,
            tx,
            search_from_block,
        )
    }

    fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> {
        utxo_common::extract_secret(secret_hash, spend_tx)
    }
}

impl MarketCoinOps for QtumCoin {
    fn ticker(&self) -> &str { &self.utxo_arc.ticker }

    fn my_address(&self) -> Result<String, String> { utxo_common::my_address(self) }

    fn my_balance(&self) -> Box<dyn Future<Item = BigDecimal, Error = String> + Send> {
        utxo_common::my_balance(&self.utxo_arc)
    }

    fn base_coin_balance(&self) -> Box<dyn Future<Item = BigDecimal, Error = String> + Send> {
        utxo_common::base_coin_balance(self)
    }

    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
        utxo_common::send_raw_tx(&self.utxo_arc, tx)
    }

    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        confirmations: u64,
        requires_nota: bool,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        utxo_common::wait_for_confirmations(
            &self.utxo_arc,
            tx,
            confirmations,
            requires_nota,
            wait_until,
            check_every,
        )
    }

    fn wait_for_tx_spend(&self, transaction: &[u8], wait_until: u64, from_block: u64) -> TransactionFut {
        utxo_common::wait_for_tx_spend(&self.utxo_arc, transaction, wait_until, from_block)
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String> {
        utxo_common::tx_enum_from_bytes(bytes)
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        utxo_common::current_block(&self.utxo_arc)
    }

    fn address_from_pubkey_str(&self, pubkey: &str) -> Result<String, String> {
        utxo_common::address_from_pubkey_str(self, pubkey)
    }

    fn display_priv_key(&self) -> String { utxo_common::display_priv_key(&self.utxo_arc) }
}

impl MmCoin for QtumCoin {
    fn is_asset_chain(&self) -> bool { utxo_common::is_asset_chain(&self.utxo_arc) }

    fn can_i_spend_other_payment(&self) -> Box<dyn Future<Item = (), Error = String> + Send> {
        utxo_common::can_i_spend_other_payment()
    }

    fn wallet_only(&self) -> bool { false }

    fn withdraw(&self, req: WithdrawRequest) -> Box<dyn Future<Item = TransactionDetails, Error = String> + Send> {
        Box::new(utxo_common::withdraw(self.clone(), req).boxed().compat())
    }

    fn decimals(&self) -> u8 { utxo_common::decimals(&self.utxo_arc) }

    /// Check if the `to_address_format` is standard and if the `from` address is standard UTXO address.
    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> {
        convert_qtum_address(&self.utxo_arc.ticker, from, to_address_format)
    }

    fn validate_address(&self, address: &str) -> ValidateAddressResult { utxo_common::validate_address(self, address) }

    fn process_history_loop(&self, ctx: MmArc) { utxo_common::process_history_loop(self, ctx) }

    fn history_sync_status(&self) -> HistorySyncState { utxo_common::history_sync_status(&self.utxo_arc) }

    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> {
        utxo_common::get_trade_fee(self.clone())
    }

    fn required_confirmations(&self) -> u64 { utxo_common::required_confirmations(&self.utxo_arc) }

    fn requires_notarization(&self) -> bool { utxo_common::requires_notarization(&self.utxo_arc) }

    fn set_required_confirmations(&self, confirmations: u64) {
        utxo_common::set_required_confirmations(&self.utxo_arc, confirmations)
    }

    fn set_requires_notarization(&self, requires_nota: bool) {
        utxo_common::set_requires_notarization(&self.utxo_arc, requires_nota)
    }

    fn my_unspendable_balance(&self) -> Box<dyn Future<Item = BigDecimal, Error = String> + Send> {
        Box::new(utxo_common::my_unspendable_balance(self.clone()).boxed().compat())
    }
}

pub fn is_qtum_unspent_mature(mature_confirmations: u32, output: &RpcTransaction) -> bool {
    let is_qrc20_coinbase = output.vout.iter().any(|x| x.is_empty());
    let is_coinbase = output.is_coinbase() || is_qrc20_coinbase;
    !is_coinbase || output.confirmations >= mature_confirmations
}

pub fn convert_qtum_address(coin: &str, from: &str, to_address_format: Json) -> Result<String, String> {
    let to_address_format: UtxoAddressFormat =
        json::from_value(to_address_format).map_err(|e| ERRL!("Error on parse UTXO address format {:?}", e))?;
    match to_address_format {
        UtxoAddressFormat::Standard => (),
        _ => return ERR!("{} supports standard UTXO address format only", coin),
    }

    let from_address = try_s!(Address::from_str(from));
    Ok(from_address.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpc::v1::types::{ScriptType, SignedTransactionOutput, TransactionOutputScript};

    #[test]
    fn test_is_unspent_mature() {
        let empty_output = SignedTransactionOutput {
            value: 0.,
            n: 0,
            script: TransactionOutputScript {
                asm: "".into(),
                hex: "".into(),
                req_sigs: 0,
                script_type: ScriptType::NonStandard,
                addresses: vec![],
            },
        };
        let real_output = SignedTransactionOutput {
            value: 117.02430015,
            n: 1,
            script: TransactionOutputScript {
                asm: "03e71b9c152bb233ddfe58f20056715c51b054a1823e0aba108e6f1cea0ceb89c8 OP_CHECKSIG".into(),
                hex: "2103e71b9c152bb233ddfe58f20056715c51b054a1823e0aba108e6f1cea0ceb89c8ac".into(),
                req_sigs: 0,
                script_type: ScriptType::PubKey,
                addresses: vec![],
            },
        };

        let mut tx = RpcTransaction {
            hex: Default::default(),
            txid: "47d983175720ba2a67f36d0e1115a129351a2f340bdde6ecb6d6029e138fe920".into(),
            hash: None,
            size: Default::default(),
            vsize: Default::default(),
            version: 2,
            locktime: 0,
            vin: vec![],
            vout: vec![empty_output, real_output],
            blockhash: "c23882939ff695be36546ea998eb585e962b043396e4d91959477b9796ceb9e1".into(),
            confirmations: 421,
            rawconfirmations: None,
            time: 1590671504,
            blocktime: 1590671504,
            height: None,
        };

        // output is coinbase and has confirmations < QTUM_MATURE_CONFIRMATIONS
        assert_eq!(is_qtum_unspent_mature(500, &tx), false);

        tx.confirmations = 501;
        // output is coinbase but has confirmations > QTUM_MATURE_CONFIRMATIONS
        assert!(is_qtum_unspent_mature(500, &tx));

        tx.confirmations = 421;
        // remove empty output
        tx.vout.remove(0);
        // output is not coinbase
        assert!(is_qtum_unspent_mature(500, &tx));
    }
}
