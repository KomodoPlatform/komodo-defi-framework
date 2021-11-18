use super::*;
use crate::utxo::rpc_clients::UtxoRpcFut;
use crate::utxo::slp::{parse_slp_script, SlpTokenInfo, SlpTransaction, SlpUnspent};
use crate::utxo::utxo_common::big_decimal_from_sat_unsigned;
use crate::{CanRefundHtlc, CoinBalance, CoinBalancesWithTokens, CoinProtocol, NegotiateSwapContractAddrErr, SwapOps,
            TradePreimageValue, ValidateAddressResult, WithdrawFut};
use common::log::warn;
use common::mm_metrics::MetricsArc;
use common::mm_number::MmNumber;
use derive_more::Display;
use futures::{FutureExt, TryFutureExt};
pub use keys::NetworkPrefix as CashAddrPrefix;
use serde_json::{self as json, Value as Json};
use serialization::{deserialize, CoinVariant};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BchActivationRequest {
    #[serde(default)]
    allow_slp_unsafe_conf: bool,
    bchd_urls: Vec<String>,
    with_tokens: Vec<String>,
    #[serde(flatten)]
    utxo_params: UtxoActivationParams,
}

#[derive(Debug, Display)]
pub enum BchFromLegacyReqErr {
    InvalidUtxoParams(UtxoFromLegacyReqErr),
    InvalidBchdUrls(json::Error),
}

impl From<UtxoFromLegacyReqErr> for BchFromLegacyReqErr {
    fn from(err: UtxoFromLegacyReqErr) -> Self { BchFromLegacyReqErr::InvalidUtxoParams(err) }
}

impl BchActivationRequest {
    pub fn from_legacy_req(req: &Json) -> Result<Self, MmError<BchFromLegacyReqErr>> {
        let bchd_urls = json::from_value(req["bchd_urls"].clone()).map_to_mm(BchFromLegacyReqErr::InvalidBchdUrls)?;
        let allow_slp_unsafe_conf = req["allow_slp_unsafe_conf"].as_bool().unwrap_or_default();
        let utxo_params = UtxoActivationParams::from_legacy_req(req)?;

        Ok(BchActivationRequest {
            allow_slp_unsafe_conf,
            bchd_urls,
            with_tokens: Vec::new(),
            utxo_params,
        })
    }
}

#[derive(Clone, Debug)]
pub struct BchCoin {
    utxo_arc: UtxoArc,
    slp_addr_prefix: CashAddrPrefix,
    bchd_urls: Vec<String>,
    slp_tokens_infos: Arc<Mutex<HashMap<String, SlpTokenInfo>>>,
}

pub enum IsSlpUtxoError {
    Rpc(UtxoRpcError),
    TxDeserialization(serialization::Error),
}

#[derive(Debug, Default)]
pub struct BchUnspents {
    /// Standard BCH UTXOs
    standard: Vec<UnspentInfo>,
    /// SLP related UTXOs
    slp: HashMap<H256, Vec<SlpUnspent>>,
    /// SLP minting batons outputs, DO NOT use them as MM2 doesn't support SLP minting by default
    slp_batons: Vec<UnspentInfo>,
    /// The unspents of transaction with an undetermined protocol (OP_RETURN in 0 output but not SLP)
    /// DO NOT ever use them to avoid burning users funds
    undetermined: Vec<UnspentInfo>,
}

impl BchUnspents {
    fn add_standard(&mut self, utxo: UnspentInfo) { self.standard.push(utxo) }

    fn add_slp(&mut self, token_id: H256, bch_unspent: UnspentInfo, slp_amount: u64) {
        let slp_unspent = SlpUnspent {
            bch_unspent,
            slp_amount,
        };
        self.slp.entry(token_id).or_insert_with(Vec::new).push(slp_unspent);
    }

    fn add_slp_baton(&mut self, utxo: UnspentInfo) { self.slp_batons.push(utxo) }

    fn add_undetermined(&mut self, utxo: UnspentInfo) { self.undetermined.push(utxo) }

    fn platform_balance(&self, decimals: u8) -> CoinBalance {
        let spendable_sat = total_unspent_value(&self.standard);

        let unspendable_slp = self.slp.iter().fold(0, |cur, (_, slp_unspents)| {
            let bch_value = total_unspent_value(slp_unspents.iter().map(|slp| &slp.bch_unspent));
            cur + bch_value
        });

        let unspendable_slp_batons = total_unspent_value(&self.slp_batons);
        let unspendable_undetermined = total_unspent_value(&self.undetermined);

        let total_unspendable = unspendable_slp + unspendable_slp_batons + unspendable_undetermined;
        CoinBalance {
            spendable: big_decimal_from_sat_unsigned(spendable_sat, decimals),
            unspendable: big_decimal_from_sat_unsigned(total_unspendable, decimals),
        }
    }
}

impl From<UtxoRpcError> for IsSlpUtxoError {
    fn from(err: UtxoRpcError) -> IsSlpUtxoError { IsSlpUtxoError::Rpc(err) }
}

impl From<serialization::Error> for IsSlpUtxoError {
    fn from(err: serialization::Error) -> IsSlpUtxoError { IsSlpUtxoError::TxDeserialization(err) }
}

impl BchCoin {
    pub fn slp_prefix(&self) -> &CashAddrPrefix { &self.slp_addr_prefix }

    pub fn bchd_urls(&self) -> &[String] { &self.bchd_urls }

    async fn utxos_into_bch_unspents(&self, utxos: Vec<UnspentInfo>) -> UtxoRpcResult<BchUnspents> {
        let mut result = BchUnspents::default();
        for unspent in utxos {
            if unspent.outpoint.index == 0 {
                // zero output is reserved for OP_RETURN of specific protocols
                // so if we get it we can safely consider this as standard BCH UTXO
                result.add_standard(unspent);
                continue;
            }

            let prev_tx_bytes = self
                .get_verbose_transaction_from_cache_or_rpc(unspent.outpoint.hash.reversed().into())
                .compat()
                .await?
                .into_inner();
            let prev_tx: UtxoTx = match deserialize(prev_tx_bytes.hex.as_slice()) {
                Ok(b) => b,
                Err(e) => {
                    warn!(
                        "Failed to deserialize prev_tx {:?} with error {:?}, considering {:?} as undetermined",
                        prev_tx_bytes, e, unspent
                    );
                    result.add_undetermined(unspent);
                    continue;
                },
            };

            if prev_tx.outputs.is_empty() {
                warn!(
                    "Prev_tx {:?} outputs are empty, considering {:?} as undetermined",
                    prev_tx_bytes, unspent
                );
                result.add_undetermined(unspent);
                continue;
            }

            let zero_out_script: Script = prev_tx.outputs[0].script_pubkey.clone().into();
            if zero_out_script.is_pay_to_public_key()
                || zero_out_script.is_pay_to_public_key_hash()
                || zero_out_script.is_pay_to_script_hash()
            {
                result.add_standard(unspent);
            } else {
                match parse_slp_script(&prev_tx.outputs[0].script_pubkey) {
                    Ok(slp_data) => match slp_data.transaction {
                        SlpTransaction::Send { token_id, amounts } => {
                            match amounts.get(unspent.outpoint.index as usize - 1) {
                                Some(slp_amount) => result.add_slp(token_id, unspent, *slp_amount),
                                None => result.add_standard(unspent),
                            }
                        },
                        SlpTransaction::Genesis {
                            initial_token_mint_quantity,
                            mint_baton_vout,
                            ..
                        } => {
                            if unspent.outpoint.index == 1 {
                                let token_id = prev_tx.hash().reversed();
                                result.add_slp(token_id, unspent, initial_token_mint_quantity);
                            } else if Some(unspent.outpoint.index) == mint_baton_vout.map(|u| u as u32) {
                                result.add_slp_baton(unspent);
                            } else {
                                result.add_standard(unspent);
                            }
                        },
                        SlpTransaction::Mint {
                            token_id,
                            additional_token_quantity,
                            mint_baton_vout,
                        } => {
                            if unspent.outpoint.index == 1 {
                                result.add_slp(token_id, unspent, additional_token_quantity);
                            } else if Some(unspent.outpoint.index) == mint_baton_vout.map(|u| u as u32) {
                                result.add_slp_baton(unspent);
                            } else {
                                result.add_standard(unspent);
                            }
                        },
                    },
                    Err(e) => {
                        warn!(
                            "Error {} parsing script {:?} as SLP, considering {:?} as undetermined",
                            e, prev_tx.outputs[0].script_pubkey, unspent
                        );
                        result.undetermined.push(unspent);
                    },
                };
            }
        }
        Ok(result)
    }

    /// Returns unspents to calculate balance, use for displaying purposes only!
    /// DO NOT USE to build transactions, it can lead to double spending attempt and also have other unpleasant consequences
    pub async fn bch_unspents_for_display(&self, address: &Address) -> UtxoRpcResult<BchUnspents> {
        // ordering is not required to display balance to we can simply call "normal" list_unspent
        let all_unspents = self
            .utxo_arc
            .rpc_client
            .list_unspent(address, self.utxo_arc.decimals)
            .compat()
            .await?;
        self.utxos_into_bch_unspents(all_unspents).await
    }

    /// Locks recently spent cache to safely return UTXOs for spending
    pub async fn bch_unspents_for_spend(
        &self,
        address: &Address,
    ) -> UtxoRpcResult<(BchUnspents, AsyncMutexGuard<'_, RecentlySpentOutPoints>)> {
        let (all_unspents, recently_spent) = utxo_common::list_unspent_ordered(self, address).await?;
        let result = self.utxos_into_bch_unspents(all_unspents).await?;

        Ok((result, recently_spent))
    }

    pub async fn get_token_utxos_for_spend(
        &self,
        token_id: &H256,
    ) -> UtxoRpcResult<(
        Vec<SlpUnspent>,
        Vec<UnspentInfo>,
        AsyncMutexGuard<'_, RecentlySpentOutPoints>,
    )> {
        let (mut bch_unspents, recently_spent) = self.bch_unspents_for_spend(&self.as_ref().my_address).await?;
        let (mut slp_unspents, standard_utxos) = (
            bch_unspents.slp.remove(token_id).unwrap_or_default(),
            bch_unspents.standard,
        );

        slp_unspents.sort_by(|a, b| a.slp_amount.cmp(&b.slp_amount));
        Ok((slp_unspents, standard_utxos, recently_spent))
    }

    pub async fn get_token_utxos_for_display(
        &self,
        token_id: &H256,
    ) -> UtxoRpcResult<(Vec<SlpUnspent>, Vec<UnspentInfo>)> {
        let mut bch_unspents = self.bch_unspents_for_display(&self.as_ref().my_address).await?;
        let (mut slp_unspents, standard_utxos) = (
            bch_unspents.slp.remove(token_id).unwrap_or_default(),
            bch_unspents.standard,
        );

        slp_unspents.sort_by(|a, b| a.slp_amount.cmp(&b.slp_amount));
        Ok((slp_unspents, standard_utxos))
    }

    pub fn add_slp_token_info(&self, ticker: String, info: SlpTokenInfo) {
        self.slp_tokens_infos.lock().unwrap().insert(ticker, info);
    }
}

impl AsRef<UtxoCoinFields> for BchCoin {
    fn as_ref(&self) -> &UtxoCoinFields { &self.utxo_arc }
}

pub async fn bch_coin_from_conf_and_params(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    params: BchActivationRequest,
    slp_addr_prefix: CashAddrPrefix,
    priv_key: &[u8],
) -> Result<BchCoin, String> {
    if params.bchd_urls.is_empty() && !params.allow_slp_unsafe_conf {
        return Err("Using empty bchd_urls is unsafe for SLP users!".into());
    }

    let bchd_urls = params.bchd_urls;
    let slp_tokens_infos = Arc::new(Mutex::new(HashMap::new()));
    let constructor = {
        move |utxo_arc| BchCoin {
            utxo_arc,
            slp_addr_prefix: slp_addr_prefix.clone(),
            bchd_urls: bchd_urls.clone(),
            slp_tokens_infos: Arc::clone(&slp_tokens_infos),
        }
    };
    let coin: BchCoin = try_s!(
        utxo_common::utxo_arc_from_conf_and_params(ctx, ticker, conf, params.utxo_params, priv_key, constructor).await
    );
    Ok(coin)
}

#[derive(Debug)]
pub enum BchActivationError {
    CoinInitError(String),
    TokenConfIsNotFound {
        token: String,
    },
    TokenCoinProtocolParseError {
        token: String,
        error: json::Error,
    },
    TokenCoinProtocolIsNotSlp {
        token: String,
        protocol: CoinProtocol,
    },
    TokenPlatformCoinIsInvalidInConf {
        token: String,
        expected_platform: String,
        actual_platform: String,
    },
    RpcError(UtxoRpcError),
    SlpPrefixParseError(String),
}

impl From<UtxoRpcError> for BchActivationError {
    fn from(e: UtxoRpcError) -> Self { BchActivationError::RpcError(e) }
}

// if mockable is placed before async_trait there is `munmap_chunk(): invalid pointer` error on async fn mocking attempt
#[async_trait]
#[cfg_attr(test, mockable)]
impl UtxoTxBroadcastOps for BchCoin {
    async fn broadcast_tx(&self, tx: &UtxoTx) -> Result<H256Json, MmError<BroadcastTxErr>> {
        utxo_common::broadcast_tx(self, tx).await
    }
}

// if mockable is placed before async_trait there is `munmap_chunk(): invalid pointer` error on async fn mocking attempt
#[async_trait]
#[cfg_attr(test, mockable)]
impl UtxoTxGenerationOps for BchCoin {
    async fn get_tx_fee(&self) -> Result<ActualTxFee, JsonRpcError> { utxo_common::get_tx_fee(&self.utxo_arc).await }

    async fn calc_interest_if_required(
        &self,
        unsigned: TransactionInputSigner,
        data: AdditionalTxData,
        my_script_pub: Bytes,
    ) -> UtxoRpcResult<(TransactionInputSigner, AdditionalTxData)> {
        utxo_common::calc_interest_if_required(self, unsigned, data, my_script_pub).await
    }
}

// if mockable is placed before async_trait there is `munmap_chunk(): invalid pointer` error on async fn mocking attempt
#[async_trait]
#[cfg_attr(test, mockable)]
impl UtxoCommonOps for BchCoin {
    async fn get_htlc_spend_fee(&self, tx_size: u64) -> UtxoRpcResult<u64> {
        utxo_common::get_htlc_spend_fee(self, tx_size).await
    }

    fn addresses_from_script(&self, script: &Script) -> Result<Vec<Address>, String> {
        utxo_common::addresses_from_script(self, script)
    }

    fn denominate_satoshis(&self, satoshi: i64) -> f64 { utxo_common::denominate_satoshis(&self.utxo_arc, satoshi) }

    fn my_public_key(&self) -> &Public { self.utxo_arc.key_pair.public() }

    fn address_from_str(&self, address: &str) -> Result<Address, String> {
        utxo_common::checked_address_from_str(&self.utxo_arc, address)
    }

    async fn get_current_mtp(&self) -> UtxoRpcResult<u32> {
        utxo_common::get_current_mtp(&self.utxo_arc, CoinVariant::Standard).await
    }

    fn is_unspent_mature(&self, output: &RpcTransaction) -> bool {
        utxo_common::is_unspent_mature(self.utxo_arc.conf.mature_confirmations, output)
    }

    async fn calc_interest_of_tx(&self, tx: &UtxoTx, input_transactions: &mut HistoryUtxoTxMap) -> UtxoRpcResult<u64> {
        utxo_common::calc_interest_of_tx(self, tx, input_transactions).await
    }

    async fn get_mut_verbose_transaction_from_map_or_rpc<'a, 'b>(
        &'a self,
        tx_hash: H256Json,
        utxo_tx_map: &'b mut HistoryUtxoTxMap,
    ) -> UtxoRpcResult<&'b mut HistoryUtxoTx> {
        utxo_common::get_mut_verbose_transaction_from_map_or_rpc(self, tx_hash, utxo_tx_map).await
    }

    async fn p2sh_spending_tx(
        &self,
        prev_transaction: UtxoTx,
        redeem_script: Bytes,
        outputs: Vec<TransactionOutput>,
        script_data: Script,
        sequence: u32,
        lock_time: u32,
    ) -> Result<UtxoTx, String> {
        utxo_common::p2sh_spending_tx(
            self,
            prev_transaction,
            redeem_script,
            outputs,
            script_data,
            sequence,
            lock_time,
        )
        .await
    }

    async fn ordered_mature_unspents<'a>(
        &'a self,
        address: &Address,
    ) -> UtxoRpcResult<(Vec<UnspentInfo>, AsyncMutexGuard<'a, RecentlySpentOutPoints>)> {
        self.list_unspent_ordered(address).await
    }

    fn get_verbose_transaction_from_cache_or_rpc(&self, txid: H256Json) -> UtxoRpcFut<VerboseTransactionFrom> {
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
    ) -> UtxoRpcResult<(Vec<UnspentInfo>, AsyncMutexGuard<'a, RecentlySpentOutPoints>)> {
        let (bch_unspents, recently_spent) = self.bch_unspents_for_spend(address).await?;
        Ok((bch_unspents.standard, recently_spent))
    }

    async fn preimage_trade_fee_required_to_send_outputs(
        &self,
        outputs: Vec<TransactionOutput>,
        fee_policy: FeePolicy,
        gas_fee: Option<u64>,
        stage: &FeeApproxStage,
    ) -> TradePreimageResult<BigDecimal> {
        utxo_common::preimage_trade_fee_required_to_send_outputs(self, outputs, fee_policy, gas_fee, stage).await
    }

    fn increase_dynamic_fee_by_stage(&self, dynamic_fee: u64, stage: &FeeApproxStage) -> u64 {
        utxo_common::increase_dynamic_fee_by_stage(self, dynamic_fee, stage)
    }

    async fn p2sh_tx_locktime(&self, htlc_locktime: u32) -> Result<u32, MmError<UtxoRpcError>> {
        utxo_common::p2sh_tx_locktime(self, &self.utxo_arc.conf.ticker, htlc_locktime).await
    }

    fn addr_format_for_standard_scripts(&self) -> UtxoAddressFormat {
        utxo_common::addr_format_for_standard_scripts(self)
    }
}

impl SwapOps for BchCoin {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, _uuid: &[u8]) -> TransactionFut {
        utxo_common::send_taker_fee(self.clone(), fee_addr, amount)
    }

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        utxo_common::send_maker_payment(self.clone(), time_lock, taker_pub, secret_hash, amount)
    }

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        utxo_common::send_taker_payment(self.clone(), time_lock, maker_pub, secret_hash, amount)
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret: &[u8],
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        utxo_common::send_maker_spends_taker_payment(self.clone(), taker_payment_tx, time_lock, taker_pub, secret)
    }

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret: &[u8],
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        utxo_common::send_taker_spends_maker_payment(self.clone(), maker_payment_tx, time_lock, maker_pub, secret)
    }

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        utxo_common::send_taker_refunds_payment(self.clone(), taker_payment_tx, time_lock, maker_pub, secret_hash)
    }

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        utxo_common::send_maker_refunds_payment(self.clone(), maker_payment_tx, time_lock, taker_pub, secret_hash)
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
        let tx = match fee_tx {
            TransactionEnum::UtxoTx(tx) => tx.clone(),
            _ => panic!(),
        };
        utxo_common::validate_fee(
            self.clone(),
            tx,
            utxo_common::DEFAULT_FEE_VOUT,
            expected_sender,
            amount,
            min_block_number,
            fee_addr,
        )
    }

    fn validate_maker_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        priv_bn_hash: &[u8],
        amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
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
        _swap_contract_address: &Option<BytesJson>,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        utxo_common::validate_taker_payment(self, payment_tx, time_lock, taker_pub, priv_bn_hash, amount)
    }

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        _search_from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
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
        _swap_contract_address: &Option<BytesJson>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        utxo_common::search_for_swap_tx_spend_my(
            &self.utxo_arc,
            time_lock,
            other_pub,
            secret_hash,
            tx,
            utxo_common::DEFAULT_SWAP_VOUT,
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
        _swap_contract_address: &Option<BytesJson>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        utxo_common::search_for_swap_tx_spend_other(
            &self.utxo_arc,
            time_lock,
            other_pub,
            secret_hash,
            tx,
            utxo_common::DEFAULT_SWAP_VOUT,
            search_from_block,
        )
    }

    fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> {
        utxo_common::extract_secret(secret_hash, spend_tx)
    }

    fn can_refund_htlc(&self, locktime: u64) -> Box<dyn Future<Item = CanRefundHtlc, Error = String> + Send + '_> {
        Box::new(
            utxo_common::can_refund_htlc(self, locktime)
                .boxed()
                .map_err(|e| ERRL!("{}", e))
                .compat(),
        )
    }

    fn negotiate_swap_contract_addr(
        &self,
        _other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        Ok(None)
    }
}

fn total_unspent_value<'a>(unspents: impl IntoIterator<Item = &'a UnspentInfo>) -> u64 {
    unspents.into_iter().fold(0, |cur, unspent| cur + unspent.value)
}

impl MarketCoinOps for BchCoin {
    fn ticker(&self) -> &str { &self.utxo_arc.conf.ticker }

    fn my_address(&self) -> Result<String, String> { utxo_common::my_address(self) }

    fn my_balance(&self) -> BalanceFut<CoinBalance> {
        let coin = self.clone();
        let fut = async move {
            let bch_unspents = coin.bch_unspents_for_display(&coin.as_ref().my_address).await?;
            Ok(bch_unspents.platform_balance(coin.as_ref().decimals))
        };
        Box::new(fut.boxed().compat())
    }

    fn get_balances_with_tokens(&self) -> BalanceFut<CoinBalancesWithTokens> { unimplemented!() }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> { utxo_common::base_coin_balance(self) }

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

    fn wait_for_tx_spend(
        &self,
        transaction: &[u8],
        wait_until: u64,
        from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        utxo_common::wait_for_output_spend(
            &self.utxo_arc,
            transaction,
            utxo_common::DEFAULT_SWAP_VOUT,
            from_block,
            wait_until,
        )
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String> {
        utxo_common::tx_enum_from_bytes(self.as_ref(), bytes)
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        utxo_common::current_block(&self.utxo_arc)
    }

    fn display_priv_key(&self) -> String { utxo_common::display_priv_key(&self.utxo_arc) }

    fn min_tx_amount(&self) -> BigDecimal { utxo_common::min_tx_amount(self.as_ref()) }

    fn min_trading_vol(&self) -> MmNumber { utxo_common::min_trading_vol(self.as_ref()) }
}

#[async_trait]
impl UtxoStandardOps for BchCoin {
    async fn tx_details_by_hash(
        &self,
        hash: &[u8],
        input_transactions: &mut HistoryUtxoTxMap,
    ) -> Result<TransactionDetails, String> {
        utxo_common::tx_details_by_hash(self, hash, input_transactions).await
    }

    async fn request_tx_history(&self, metrics: MetricsArc) -> RequestTxHistoryResult {
        utxo_common::request_tx_history(self, metrics).await
    }

    async fn update_kmd_rewards(
        &self,
        tx_details: &mut TransactionDetails,
        input_transactions: &mut HistoryUtxoTxMap,
    ) -> UtxoRpcResult<()> {
        utxo_common::update_kmd_rewards(self, tx_details, input_transactions).await
    }
}

impl MmCoin for BchCoin {
    fn is_asset_chain(&self) -> bool { utxo_common::is_asset_chain(&self.utxo_arc) }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        Box::new(utxo_common::withdraw(self.clone(), req).boxed().compat())
    }

    fn decimals(&self) -> u8 { utxo_common::decimals(&self.utxo_arc) }

    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> {
        utxo_common::convert_to_address(self, from, to_address_format)
    }

    fn validate_address(&self, address: &str) -> ValidateAddressResult { utxo_common::validate_address(self, address) }

    fn process_history_loop(&self, ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> {
        Box::new(
            utxo_common::process_history_loop(self.clone(), ctx)
                .map(|_| Ok(()))
                .boxed()
                .compat(),
        )
    }

    fn history_sync_status(&self) -> HistorySyncState { utxo_common::history_sync_status(&self.utxo_arc) }

    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> {
        utxo_common::get_trade_fee(self.clone())
    }

    fn get_sender_trade_fee(&self, value: TradePreimageValue, stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        utxo_common::get_sender_trade_fee(self.clone(), value, stage)
    }

    fn get_receiver_trade_fee(&self, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        utxo_common::get_receiver_trade_fee(self.clone())
    }

    fn get_fee_to_send_taker_fee(
        &self,
        dex_fee_amount: BigDecimal,
        stage: FeeApproxStage,
    ) -> TradePreimageFut<TradeFee> {
        utxo_common::get_fee_to_send_taker_fee(self.clone(), dex_fee_amount, stage)
    }

    fn required_confirmations(&self) -> u64 { utxo_common::required_confirmations(&self.utxo_arc) }

    fn requires_notarization(&self) -> bool { utxo_common::requires_notarization(&self.utxo_arc) }

    fn set_required_confirmations(&self, confirmations: u64) {
        utxo_common::set_required_confirmations(&self.utxo_arc, confirmations)
    }

    fn set_requires_notarization(&self, requires_nota: bool) {
        utxo_common::set_requires_notarization(&self.utxo_arc, requires_nota)
    }

    fn swap_contract_address(&self) -> Option<BytesJson> { utxo_common::swap_contract_address() }

    fn mature_confirmations(&self) -> Option<u32> { Some(self.utxo_arc.conf.mature_confirmations) }

    fn coin_protocol_info(&self) -> Vec<u8> { utxo_common::coin_protocol_info(&self.utxo_arc) }

    fn is_coin_protocol_supported(&self, info: &Option<Vec<u8>>) -> bool {
        utxo_common::is_coin_protocol_supported(&self.utxo_arc, info)
    }
}

// testnet
#[cfg(test)]
pub fn tbch_coin_for_test() -> BchCoin {
    use common::block_on;
    use common::mm_ctx::MmCtxBuilder;
    use common::privkey::key_pair_from_seed;

    let ctx = MmCtxBuilder::default().into_mm_arc();
    let keypair = key_pair_from_seed("BCH SLP test").unwrap();

    let conf = json!({"coin":"BCH","pubtype":0,"p2shtype":5,"mm2":1,"fork_id":"0x40","protocol":{"type":"UTXO"},
         "address_format":{"format":"cashaddress","network":"bchtest"}});
    let req = json!({
        "method": "electrum",
        "coin": "BCH",
        "servers": [{"url":"blackie.c3-soft.com:60001"},{"url":"testnet.imaginary.cash:50001"},{"url":"tbch.loping.net:60001"},{"url":"electroncash.de:50003"}],
        "bchd_urls": ["https://bchd-testnet.greyh.at:18335"],
        "allow_slp_unsafe_conf": false,
    });

    let params = BchActivationRequest::from_legacy_req(&req).unwrap();
    block_on(bch_coin_from_conf_and_params(
        &ctx,
        "BCH",
        &conf,
        params,
        CashAddrPrefix::SlpTest,
        &*keypair.private().secret,
    ))
    .unwrap()
}

// mainnet
#[cfg(test)]
pub fn bch_coin_for_test() -> BchCoin {
    use common::block_on;
    use common::mm_ctx::MmCtxBuilder;
    use common::privkey::key_pair_from_seed;

    let ctx = MmCtxBuilder::default().into_mm_arc();
    let keypair = key_pair_from_seed("BCH SLP test").unwrap();

    let conf = json!({"coin":"BCH","pubtype":0,"p2shtype":5,"mm2":1,"fork_id":"0x40","protocol":{"type":"UTXO"},
         "address_format":{"format":"cashaddress","network":"bitcoincash"}});
    let req = json!({
        "method": "electrum",
        "coin": "BCH",
        "servers": [{"url":"electrum1.cipig.net:10055"},{"url":"electrum2.cipig.net:10055"},{"url":"electrum3.cipig.net:10055"}],
        "bchd_urls": [],
        "allow_slp_unsafe_conf": true,
    });

    let params = BchActivationRequest::from_legacy_req(&req).unwrap();
    block_on(bch_coin_from_conf_and_params(
        &ctx,
        "BCH",
        &conf,
        params,
        CashAddrPrefix::SimpleLedger,
        &*keypair.private().secret,
    ))
    .unwrap()
}
