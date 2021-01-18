use super::*;
use crate::{eth, SwapOps, ValidateAddressResult};
use common::mm_metrics::MetricsArc;
use ethereum_types::H160;
use futures::{FutureExt, TryFutureExt};

pub const QTUM_STANDARD_DUST: u64 = 1000;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "format")]
pub enum QtumAddressFormat {
    /// Standard Qtum/UTXO address format.
    #[serde(rename = "wallet")]
    Wallet,
    /// Contract address format. The same as used in ETH/ERC20.
    /// Note starts with "0x" prefix.
    #[serde(rename = "contract")]
    Contract,
}

pub trait QtumBasedCoin: AsRef<UtxoCoinFields> {
    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> {
        let to_address_format: QtumAddressFormat =
            json::from_value(to_address_format).map_err(|e| ERRL!("Error on parse Qtum address format {:?}", e))?;
        let from_address = try_s!(self.utxo_address_from_any_format(from));
        match to_address_format {
            QtumAddressFormat::Wallet => Ok(from_address.to_string()),
            QtumAddressFormat::Contract => Ok(display_as_contract_address(from_address)),
        }
    }

    /// Try to parse address from either wallet (UTXO) format or contract format.
    fn utxo_address_from_any_format(&self, from: &str) -> Result<Address, String> {
        let utxo_err = match Address::from_str(from) {
            Ok(addr) => {
                let is_p2pkh = addr.prefix == self.as_ref().pub_addr_prefix
                    && addr.t_addr_prefix == self.as_ref().pub_t_addr_prefix;
                if is_p2pkh {
                    return Ok(addr);
                }
                "Address has invalid prefixes".to_string()
            },
            Err(e) => e.to_string(),
        };
        let contract_err = match contract_addr_from_str(from) {
            Ok(contract_addr) => return Ok(self.utxo_addr_from_contract_addr(contract_addr)),
            Err(e) => e,
        };
        ERR!(
            "error on parse wallet address: {:?}, error on parse contract address: {:?}",
            utxo_err,
            contract_err,
        )
    }

    fn utxo_addr_from_contract_addr(&self, address: H160) -> Address {
        let utxo = self.as_ref();
        Address {
            prefix: utxo.pub_addr_prefix,
            t_addr_prefix: utxo.pub_t_addr_prefix,
            hash: address.0.into(),
            checksum_type: utxo.checksum_type,
        }
    }

    fn my_addr_as_contract_addr(&self) -> H160 { contract_addr_from_utxo_addr(self.as_ref().my_address.clone()) }

    fn utxo_address_from_contract_addr(&self, address: H160) -> Address {
        let utxo = self.as_ref();
        Address {
            prefix: utxo.pub_addr_prefix,
            t_addr_prefix: utxo.pub_t_addr_prefix,
            hash: address.0.into(),
            checksum_type: utxo.checksum_type,
        }
    }

    fn contract_address_from_raw_pubkey(&self, pubkey: &[u8]) -> Result<H160, String> {
        let utxo = self.as_ref();
        let qtum_address = try_s!(utxo_common::address_from_raw_pubkey(
            pubkey,
            utxo.pub_addr_prefix,
            utxo.pub_t_addr_prefix,
            utxo.checksum_type
        ));
        Ok(qtum::contract_addr_from_utxo_addr(qtum_address))
    }

    fn is_qtum_unspent_mature(&self, output: &RpcTransaction) -> bool {
        let is_qrc20_coinbase = output.vout.iter().any(|x| x.is_empty());
        let is_coinbase = output.is_coinbase() || is_qrc20_coinbase;
        !is_coinbase || output.confirmations >= self.as_ref().mature_confirmations
    }
}

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
    let coin: QtumCoin = try_s!(utxo_common::utxo_arc_from_conf_and_request(ctx, ticker, conf, req, priv_key).await);
    Ok(coin)
}

impl QtumBasedCoin for QtumCoin {}

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

    fn is_unspent_mature(&self, output: &RpcTransaction) -> bool { self.is_qtum_unspent_mature(output) }

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

    fn wait_for_tx_spend(
        &self,
        transaction: &[u8],
        wait_until: u64,
        from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
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
        QtumBasedCoin::convert_to_address(self, from, to_address_format)
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

    fn swap_contract_address(&self) -> Option<BytesJson> { utxo_common::swap_contract_address() }
}

/// Parse contract address (H160) from string.
/// Qtum Contract addresses have another checksum verification algorithm, because of this do not use [`eth::valid_addr_from_str`].
pub fn contract_addr_from_str(addr: &str) -> Result<H160, String> { eth::addr_from_str(addr) }

pub fn contract_addr_from_utxo_addr(address: Address) -> H160 { address.hash.take().into() }

pub fn display_as_contract_address(address: Address) -> String {
    let address = qtum::contract_addr_from_utxo_addr(address);
    format!("{:#02x}", address)
}
