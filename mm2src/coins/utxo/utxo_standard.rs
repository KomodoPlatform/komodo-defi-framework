use super::*;
use crate::coin_balance::{self, EnableCoinBalanceError, EnabledCoinBalanceParams, HDAccountBalance, HDAddressBalance,
                          HDWalletBalance, HDWalletBalanceOps};
use crate::coin_errors::MyAddressError;
use crate::hd_pubkey::{ExtractExtendedPubkey, HDExtractPubkeyError, HDXPubExtractor};
use crate::hd_wallet::{AccountUpdatingError, AddressDerivingResult, HDAccountMut, NewAccountCreatingError};
use crate::hd_wallet_storage::HDWalletCoinWithStorageOps;
use crate::my_tx_history_v2::{CoinWithTxHistoryV2, MyTxHistoryErrorV2, MyTxHistoryTarget, TxHistoryStorage};
use crate::rpc_command::account_balance::{self, AccountBalanceParams, AccountBalanceRpcOps, HDAccountBalanceResponse};
use crate::rpc_command::get_new_address::{self, GetNewAddressParams, GetNewAddressResponse, GetNewAddressRpcError,
                                          GetNewAddressRpcOps};
use crate::rpc_command::hd_account_balance_rpc_error::HDAccountBalanceRpcError;
use crate::rpc_command::init_account_balance::{self, InitAccountBalanceParams, InitAccountBalanceRpcOps};
use crate::rpc_command::init_create_account::{self, CreateAccountRpcError, CreateAccountState, CreateNewAccountParams,
                                              InitCreateAccountRpcOps};
use crate::rpc_command::init_scan_for_new_addresses::{self, InitScanAddressesRpcOps, ScanAddressesParams,
                                                      ScanAddressesResponse};
use crate::rpc_command::init_withdraw::{InitWithdrawCoin, WithdrawTaskHandle};
use crate::tx_history_storage::{GetTxHistoryFilters, WalletId};
use crate::utxo::utxo_builder::{UtxoArcBuilder, UtxoCoinBuilder};
use crate::utxo::utxo_tx_history_v2::{UtxoMyAddressesHistoryError, UtxoTxDetailsError, UtxoTxDetailsParams,
                                      UtxoTxHistoryOps};
use crate::{CanRefundHtlc, CheckIfMyPaymentSentArgs, CoinBalance, CoinWithDerivationMethod, GetWithdrawSenderAddress,
            IguanaPrivKey, NegotiateSwapContractAddrErr, PaymentInstructions, PaymentInstructionsErr,
            PrivKeyBuildPolicy, SearchForSwapTxSpendInput, SendMakerPaymentArgs, SendMakerRefundsPaymentArgs,
            SendMakerSpendsTakerPaymentArgs, SendTakerPaymentArgs, SendTakerRefundsPaymentArgs,
            SendTakerSpendsMakerPaymentArgs, SignatureResult, SwapOps, TradePreimageValue, TransactionFut,
            TxMarshalingErr, ValidateAddressResult, ValidateFeeArgs, ValidateInstructionsErr, ValidateOtherPubKeyErr,
            ValidatePaymentFut, ValidatePaymentInput, VerificationResult, WatcherOps, WatcherValidatePaymentInput,
            WithdrawFut, WithdrawSenderAddress};
use crypto::Bip44Chain;
use futures::{FutureExt, TryFutureExt};
use mm2_metrics::MetricsArc;
use mm2_number::MmNumber;
use utxo_signer::UtxoSignerOps;

#[derive(Clone)]
pub struct UtxoStandardCoin {
    utxo_arc: UtxoArc,
}

impl AsRef<UtxoCoinFields> for UtxoStandardCoin {
    fn as_ref(&self) -> &UtxoCoinFields { &self.utxo_arc }
}

impl From<UtxoArc> for UtxoStandardCoin {
    fn from(coin: UtxoArc) -> UtxoStandardCoin { UtxoStandardCoin { utxo_arc: coin } }
}

impl From<UtxoStandardCoin> for UtxoArc {
    fn from(coin: UtxoStandardCoin) -> Self { coin.utxo_arc }
}

pub async fn utxo_standard_coin_with_policy(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    activation_params: &UtxoActivationParams,
    priv_key_policy: PrivKeyBuildPolicy,
) -> Result<UtxoStandardCoin, String> {
    let coin = try_s!(
        UtxoArcBuilder::new(
            ctx,
            ticker,
            conf,
            activation_params,
            priv_key_policy,
            UtxoStandardCoin::from
        )
        .build()
        .await
    );
    Ok(coin)
}

pub async fn utxo_standard_coin_with_priv_key(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    activation_params: &UtxoActivationParams,
    priv_key: IguanaPrivKey,
) -> Result<UtxoStandardCoin, String> {
    let priv_key_policy = PrivKeyBuildPolicy::IguanaPrivKey(priv_key);
    utxo_standard_coin_with_policy(ctx, ticker, conf, activation_params, priv_key_policy).await
}

// if mockable is placed before async_trait there is `munmap_chunk(): invalid pointer` error on async fn mocking attempt
#[async_trait]
#[cfg_attr(test, mockable)]
impl UtxoTxBroadcastOps for UtxoStandardCoin {
    async fn broadcast_tx(&self, tx: &UtxoTx) -> Result<H256Json, MmError<BroadcastTxErr>> {
        utxo_common::broadcast_tx(self, tx).await
    }
}

// if mockable is placed before async_trait there is `munmap_chunk(): invalid pointer` error on async fn mocking attempt
#[async_trait]
#[cfg_attr(test, mockable)]
impl UtxoTxGenerationOps for UtxoStandardCoin {
    async fn get_tx_fee(&self) -> UtxoRpcResult<ActualTxFee> { utxo_common::get_tx_fee(&self.utxo_arc).await }

    async fn calc_interest_if_required(
        &self,
        unsigned: TransactionInputSigner,
        data: AdditionalTxData,
        my_script_pub: Bytes,
    ) -> UtxoRpcResult<(TransactionInputSigner, AdditionalTxData)> {
        utxo_common::calc_interest_if_required(self, unsigned, data, my_script_pub).await
    }
}

#[async_trait]
#[cfg_attr(test, mockable)]
impl GetUtxoListOps for UtxoStandardCoin {
    async fn get_unspent_ordered_list(
        &self,
        address: &Address,
    ) -> UtxoRpcResult<(Vec<UnspentInfo>, RecentlySpentOutPointsGuard<'_>)> {
        utxo_common::get_unspent_ordered_list(self, address).await
    }

    async fn get_all_unspent_ordered_list(
        &self,
        address: &Address,
    ) -> UtxoRpcResult<(Vec<UnspentInfo>, RecentlySpentOutPointsGuard<'_>)> {
        utxo_common::get_all_unspent_ordered_list(self, address).await
    }

    async fn get_mature_unspent_ordered_list(
        &self,
        address: &Address,
    ) -> UtxoRpcResult<(MatureUnspentList, RecentlySpentOutPointsGuard<'_>)> {
        utxo_common::get_mature_unspent_ordered_list(self, address).await
    }
}

#[async_trait]
#[cfg_attr(test, mockable)]
impl GetUtxoMapOps for UtxoStandardCoin {
    async fn get_unspent_ordered_map(
        &self,
        addresses: Vec<Address>,
    ) -> UtxoRpcResult<(UnspentMap, RecentlySpentOutPointsGuard<'_>)> {
        utxo_common::get_unspent_ordered_map(self, addresses).await
    }

    async fn get_all_unspent_ordered_map(
        &self,
        addresses: Vec<Address>,
    ) -> UtxoRpcResult<(UnspentMap, RecentlySpentOutPointsGuard<'_>)> {
        utxo_common::get_all_unspent_ordered_map(self, addresses).await
    }

    async fn get_mature_unspent_ordered_map(
        &self,
        addresses: Vec<Address>,
    ) -> UtxoRpcResult<(MatureUnspentMap, RecentlySpentOutPointsGuard<'_>)> {
        utxo_common::get_mature_unspent_ordered_map(self, addresses).await
    }
}

// if mockable is placed before async_trait there is `munmap_chunk(): invalid pointer` error on async fn mocking attempt
#[async_trait]
#[cfg_attr(test, mockable)]
impl UtxoCommonOps for UtxoStandardCoin {
    async fn get_htlc_spend_fee(&self, tx_size: u64, stage: &FeeApproxStage) -> UtxoRpcResult<u64> {
        utxo_common::get_htlc_spend_fee(self, tx_size, stage).await
    }

    fn addresses_from_script(&self, script: &Script) -> Result<Vec<Address>, String> {
        utxo_common::addresses_from_script(self, script)
    }

    fn denominate_satoshis(&self, satoshi: i64) -> f64 { utxo_common::denominate_satoshis(&self.utxo_arc, satoshi) }

    fn my_public_key(&self) -> Result<&Public, MmError<UnexpectedDerivationMethod>> {
        utxo_common::my_public_key(self.as_ref())
    }

    fn address_from_str(&self, address: &str) -> MmResult<Address, AddrFromStrError> {
        utxo_common::checked_address_from_str(self, address)
    }

    async fn get_current_mtp(&self) -> UtxoRpcResult<u32> {
        utxo_common::get_current_mtp(&self.utxo_arc, self.ticker().into()).await
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

    async fn p2sh_spending_tx(&self, input: utxo_common::P2SHSpendingTxInput<'_>) -> Result<UtxoTx, String> {
        utxo_common::p2sh_spending_tx(self, input).await
    }

    fn get_verbose_transactions_from_cache_or_rpc(
        &self,
        tx_ids: HashSet<H256Json>,
    ) -> UtxoRpcFut<HashMap<H256Json, VerboseTransactionFrom>> {
        let selfi = self.clone();
        let fut = async move { utxo_common::get_verbose_transactions_from_cache_or_rpc(&selfi.utxo_arc, tx_ids).await };
        Box::new(fut.boxed().compat())
    }

    async fn preimage_trade_fee_required_to_send_outputs(
        &self,
        outputs: Vec<TransactionOutput>,
        fee_policy: FeePolicy,
        gas_fee: Option<u64>,
        stage: &FeeApproxStage,
    ) -> TradePreimageResult<BigDecimal> {
        utxo_common::preimage_trade_fee_required_to_send_outputs(
            self,
            self.ticker(),
            outputs,
            fee_policy,
            gas_fee,
            stage,
        )
        .await
    }

    fn increase_dynamic_fee_by_stage(&self, dynamic_fee: u64, stage: &FeeApproxStage) -> u64 {
        utxo_common::increase_dynamic_fee_by_stage(self, dynamic_fee, stage)
    }

    async fn p2sh_tx_locktime(&self, htlc_locktime: u32) -> Result<u32, MmError<UtxoRpcError>> {
        utxo_common::p2sh_tx_locktime(self, &self.utxo_arc.conf.ticker, htlc_locktime).await
    }

    fn addr_format(&self) -> &UtxoAddressFormat { utxo_common::addr_format(self) }

    fn addr_format_for_standard_scripts(&self) -> UtxoAddressFormat {
        utxo_common::addr_format_for_standard_scripts(self)
    }

    fn address_from_pubkey(&self, pubkey: &Public) -> Address {
        let conf = &self.utxo_arc.conf;
        utxo_common::address_from_pubkey(
            pubkey,
            conf.pub_addr_prefix,
            conf.pub_t_addr_prefix,
            conf.checksum_type,
            conf.bech32_hrp.clone(),
            self.addr_format().clone(),
        )
    }
}

#[async_trait]
impl UtxoStandardOps for UtxoStandardCoin {
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

#[async_trait]
impl SwapOps for UtxoStandardCoin {
    #[inline]
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, _uuid: &[u8]) -> TransactionFut {
        utxo_common::send_taker_fee(self.clone(), fee_addr, amount)
    }

    #[inline]
    fn send_maker_payment(&self, maker_payment_args: SendMakerPaymentArgs) -> TransactionFut {
        utxo_common::send_maker_payment(
            self.clone(),
            maker_payment_args.time_lock,
            maker_payment_args.other_pubkey,
            maker_payment_args.secret_hash,
            maker_payment_args.amount,
            maker_payment_args.swap_unique_data,
        )
    }

    #[inline]
    fn send_taker_payment(&self, taker_payment_args: SendTakerPaymentArgs) -> TransactionFut {
        utxo_common::send_taker_payment(
            self.clone(),
            taker_payment_args.time_lock,
            taker_payment_args.other_pubkey,
            taker_payment_args.secret_hash,
            taker_payment_args.amount,
            taker_payment_args.swap_unique_data,
        )
    }

    #[inline]
    fn send_maker_spends_taker_payment(
        &self,
        maker_spends_payment_args: SendMakerSpendsTakerPaymentArgs,
    ) -> TransactionFut {
        utxo_common::send_maker_spends_taker_payment(
            self.clone(),
            maker_spends_payment_args.other_payment_tx,
            maker_spends_payment_args.time_lock,
            maker_spends_payment_args.other_pubkey,
            maker_spends_payment_args.secret,
            maker_spends_payment_args.secret_hash,
            maker_spends_payment_args.swap_unique_data,
        )
    }

    #[inline]
    fn send_taker_spends_maker_payment(
        &self,
        taker_spends_payment_args: SendTakerSpendsMakerPaymentArgs,
    ) -> TransactionFut {
        utxo_common::send_taker_spends_maker_payment(
            self.clone(),
            taker_spends_payment_args.other_payment_tx,
            taker_spends_payment_args.time_lock,
            taker_spends_payment_args.other_pubkey,
            taker_spends_payment_args.secret,
            taker_spends_payment_args.secret_hash,
            taker_spends_payment_args.swap_unique_data,
        )
    }

    #[inline]
    fn send_taker_refunds_payment(&self, taker_refunds_payment_args: SendTakerRefundsPaymentArgs) -> TransactionFut {
        utxo_common::send_taker_refunds_payment(
            self.clone(),
            taker_refunds_payment_args.payment_tx,
            taker_refunds_payment_args.time_lock,
            taker_refunds_payment_args.other_pubkey,
            taker_refunds_payment_args.secret_hash,
            taker_refunds_payment_args.swap_unique_data,
        )
    }

    #[inline]
    fn send_maker_refunds_payment(&self, maker_refunds_payment_args: SendMakerRefundsPaymentArgs) -> TransactionFut {
        utxo_common::send_maker_refunds_payment(
            self.clone(),
            maker_refunds_payment_args.payment_tx,
            maker_refunds_payment_args.time_lock,
            maker_refunds_payment_args.other_pubkey,
            maker_refunds_payment_args.secret_hash,
            maker_refunds_payment_args.swap_unique_data,
        )
    }

    fn validate_fee(&self, validate_fee_args: ValidateFeeArgs) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let tx = match validate_fee_args.fee_tx {
            TransactionEnum::UtxoTx(tx) => tx.clone(),
            _ => panic!(),
        };
        utxo_common::validate_fee(
            self.clone(),
            tx,
            utxo_common::DEFAULT_FEE_VOUT,
            validate_fee_args.expected_sender,
            validate_fee_args.amount,
            validate_fee_args.min_block_number,
            validate_fee_args.fee_addr,
        )
    }

    #[inline]
    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        utxo_common::validate_maker_payment(self, input)
    }

    #[inline]
    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        utxo_common::validate_taker_payment(self, input)
    }

    #[inline]
    fn check_if_my_payment_sent(
        &self,
        if_my_payment_spent_args: CheckIfMyPaymentSentArgs,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        utxo_common::check_if_my_payment_sent(
            self.clone(),
            if_my_payment_spent_args.time_lock,
            if_my_payment_spent_args.other_pub,
            if_my_payment_spent_args.secret_hash,
            if_my_payment_spent_args.swap_unique_data,
        )
    }

    #[inline]
    async fn search_for_swap_tx_spend_my(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        utxo_common::search_for_swap_tx_spend_my(self, input, utxo_common::DEFAULT_SWAP_VOUT).await
    }

    #[inline]
    async fn search_for_swap_tx_spend_other(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        utxo_common::search_for_swap_tx_spend_other(self, input, utxo_common::DEFAULT_SWAP_VOUT).await
    }

    #[inline]
    fn check_tx_signed_by_pub(&self, tx: &[u8], expected_pub: &[u8]) -> Result<bool, String> {
        utxo_common::check_all_inputs_signed_by_pub(tx, expected_pub)
    }

    #[inline]
    async fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> {
        utxo_common::extract_secret(secret_hash, spend_tx)
    }

    #[inline]
    fn can_refund_htlc(&self, locktime: u64) -> Box<dyn Future<Item = CanRefundHtlc, Error = String> + Send + '_> {
        Box::new(
            utxo_common::can_refund_htlc(self, locktime)
                .boxed()
                .map_err(|e| ERRL!("{}", e))
                .compat(),
        )
    }

    #[inline]
    fn negotiate_swap_contract_addr(
        &self,
        _other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        Ok(None)
    }

    #[inline]
    fn derive_htlc_key_pair(&self, swap_unique_data: &[u8]) -> KeyPair {
        utxo_common::derive_htlc_key_pair(self.as_ref(), swap_unique_data)
    }

    #[inline]
    fn validate_other_pubkey(&self, raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> {
        utxo_common::validate_other_pubkey(raw_pubkey)
    }

    async fn payment_instructions(
        &self,
        _secret_hash: &[u8],
        _amount: &BigDecimal,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        Ok(None)
    }

    fn validate_instructions(
        &self,
        _instructions: &[u8],
        _secret_hash: &[u8],
        _amount: BigDecimal,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        MmError::err(ValidateInstructionsErr::UnsupportedCoin(self.ticker().to_string()))
    }

    fn is_supported_by_watchers(&self) -> bool { true }
}

#[async_trait]
impl WatcherOps for UtxoStandardCoin {
    #[inline]
    fn create_taker_refunds_payment_preimage(
        &self,
        taker_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        utxo_common::create_taker_refunds_payment_preimage(
            self.clone(),
            taker_tx,
            time_lock,
            maker_pub,
            secret_hash,
            swap_unique_data,
        )
    }

    #[inline]
    fn create_taker_spends_maker_payment_preimage(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        utxo_common::create_taker_spends_maker_payment_preimage(
            self.clone(),
            maker_payment_tx,
            time_lock,
            maker_pub,
            secret_hash,
            swap_unique_data,
        )
    }

    #[inline]
    fn send_watcher_refunds_taker_payment_preimage(&self, preimage: &[u8]) -> TransactionFut {
        utxo_common::send_watcher_refunds_taker_payment_preimage(self.clone(), preimage)
    }

    #[inline]
    fn send_taker_spends_maker_payment_preimage(&self, preimage: &[u8], secret: &[u8]) -> TransactionFut {
        utxo_common::send_taker_spends_maker_payment_preimage(self.clone(), preimage, secret)
    }

    #[inline]
    fn watcher_validate_taker_fee(&self, taker_fee_hash: Vec<u8>, verified_pub: Vec<u8>) -> ValidatePaymentFut<()> {
        utxo_common::watcher_validate_taker_fee(self.clone(), taker_fee_hash, verified_pub)
    }

    #[inline]
    fn watcher_validate_taker_payment(&self, input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
        utxo_common::watcher_validate_taker_payment(self, input)
    }
}

impl MarketCoinOps for UtxoStandardCoin {
    fn ticker(&self) -> &str { &self.utxo_arc.conf.ticker }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> {
        let pubkey = utxo_common::my_public_key(&self.utxo_arc)?;
        Ok(pubkey.to_string())
    }

    fn my_address(&self) -> MmResult<String, MyAddressError> { utxo_common::my_address(self) }

    fn sign_message_hash(&self, message: &str) -> Option<[u8; 32]> {
        utxo_common::sign_message_hash(self.as_ref(), message)
    }

    fn sign_message(&self, message: &str) -> SignatureResult<String> {
        utxo_common::sign_message(self.as_ref(), message)
    }

    fn verify_message(&self, signature_base64: &str, message: &str, address: &str) -> VerificationResult<bool> {
        utxo_common::verify_message(self, signature_base64, message, address)
    }

    fn my_balance(&self) -> BalanceFut<CoinBalance> { utxo_common::my_balance(self.clone()) }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> { utxo_common::base_coin_balance(self) }

    fn platform_ticker(&self) -> &str { self.ticker() }

    #[inline(always)]
    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
        utxo_common::send_raw_tx(&self.utxo_arc, tx)
    }

    #[inline(always)]
    fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> {
        utxo_common::send_raw_tx_bytes(&self.utxo_arc, tx)
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

    fn wait_for_htlc_tx_spend(
        &self,
        transaction: &[u8],
        _secret_hash: &[u8],
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

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        utxo_common::tx_enum_from_bytes(self.as_ref(), bytes)
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        utxo_common::current_block(&self.utxo_arc)
    }

    fn display_priv_key(&self) -> Result<String, String> { utxo_common::display_priv_key(&self.utxo_arc) }

    fn min_tx_amount(&self) -> BigDecimal { utxo_common::min_tx_amount(self.as_ref()) }

    fn min_trading_vol(&self) -> MmNumber { utxo_common::min_trading_vol(self.as_ref()) }
}

#[async_trait]
impl MmCoin for UtxoStandardCoin {
    fn is_asset_chain(&self) -> bool { utxo_common::is_asset_chain(&self.utxo_arc) }

    fn spawner(&self) -> CoinFutSpawner { CoinFutSpawner::new(&self.as_ref().abortable_system) }

    fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut {
        Box::new(utxo_common::get_raw_transaction(&self.utxo_arc, req).boxed().compat())
    }

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

    async fn get_sender_trade_fee(
        &self,
        value: TradePreimageValue,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        utxo_common::get_sender_trade_fee(self, value, stage).await
    }

    fn get_receiver_trade_fee(&self, _send_amount: BigDecimal, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        utxo_common::get_receiver_trade_fee(self.clone())
    }

    async fn get_fee_to_send_taker_fee(
        &self,
        dex_fee_amount: BigDecimal,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        utxo_common::get_fee_to_send_taker_fee(self, dex_fee_amount, stage).await
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

    fn fallback_swap_contract(&self) -> Option<BytesJson> { utxo_common::fallback_swap_contract() }

    fn mature_confirmations(&self) -> Option<u32> { Some(self.utxo_arc.conf.mature_confirmations) }

    fn coin_protocol_info(&self) -> Vec<u8> { utxo_common::coin_protocol_info(self) }

    fn is_coin_protocol_supported(&self, info: &Option<Vec<u8>>) -> bool {
        utxo_common::is_coin_protocol_supported(self, info)
    }
}

#[async_trait]
impl GetWithdrawSenderAddress for UtxoStandardCoin {
    type Address = Address;
    type Pubkey = Public;

    async fn get_withdraw_sender_address(
        &self,
        req: &WithdrawRequest,
    ) -> MmResult<WithdrawSenderAddress<Self::Address, Self::Pubkey>, WithdrawError> {
        utxo_common::get_withdraw_from_address(self, req).await
    }
}

#[async_trait]
impl InitWithdrawCoin for UtxoStandardCoin {
    async fn init_withdraw(
        &self,
        ctx: MmArc,
        req: WithdrawRequest,
        task_handle: &WithdrawTaskHandle,
    ) -> Result<TransactionDetails, MmError<WithdrawError>> {
        utxo_common::init_withdraw(ctx, self.clone(), req, task_handle).await
    }
}

impl UtxoSignerOps for UtxoStandardCoin {
    type TxGetter = UtxoRpcClientEnum;

    fn trezor_coin(&self) -> UtxoSignTxResult<String> {
        self.utxo_arc
            .conf
            .trezor_coin
            .clone()
            .or_mm_err(|| UtxoSignTxError::CoinNotSupportedWithTrezor {
                coin: self.utxo_arc.conf.ticker.clone(),
            })
    }

    fn fork_id(&self) -> u32 { self.utxo_arc.conf.fork_id }

    fn branch_id(&self) -> u32 { self.utxo_arc.conf.consensus_branch_id }

    fn tx_provider(&self) -> Self::TxGetter { self.utxo_arc.rpc_client.clone() }
}

impl CoinWithDerivationMethod for UtxoStandardCoin {
    type Address = Address;
    type HDWallet = UtxoHDWallet;

    fn derivation_method(&self) -> &DerivationMethod<Self::Address, Self::HDWallet> {
        utxo_common::derivation_method(self.as_ref())
    }
}

#[async_trait]
impl ExtractExtendedPubkey for UtxoStandardCoin {
    type ExtendedPublicKey = Secp256k1ExtendedPublicKey;

    async fn extract_extended_pubkey<XPubExtractor>(
        &self,
        xpub_extractor: &XPubExtractor,
        derivation_path: DerivationPath,
    ) -> MmResult<Self::ExtendedPublicKey, HDExtractPubkeyError>
    where
        XPubExtractor: HDXPubExtractor + Sync,
    {
        utxo_common::extract_extended_pubkey(&self.utxo_arc.conf, xpub_extractor, derivation_path).await
    }
}

#[async_trait]
impl HDWalletCoinOps for UtxoStandardCoin {
    type Address = Address;
    type Pubkey = Public;
    type HDWallet = UtxoHDWallet;
    type HDAccount = UtxoHDAccount;

    async fn derive_addresses<Ids>(
        &self,
        hd_account: &Self::HDAccount,
        address_ids: Ids,
    ) -> AddressDerivingResult<Vec<HDAddress<Self::Address, Self::Pubkey>>>
    where
        Ids: Iterator<Item = HDAddressId> + Send,
    {
        utxo_common::derive_addresses(self, hd_account, address_ids).await
    }

    async fn create_new_account<'a, XPubExtractor>(
        &self,
        hd_wallet: &'a Self::HDWallet,
        xpub_extractor: &XPubExtractor,
    ) -> MmResult<HDAccountMut<'a, Self::HDAccount>, NewAccountCreatingError>
    where
        XPubExtractor: HDXPubExtractor + Sync,
    {
        utxo_common::create_new_account(self, hd_wallet, xpub_extractor).await
    }

    async fn set_known_addresses_number(
        &self,
        hd_wallet: &Self::HDWallet,
        hd_account: &mut Self::HDAccount,
        chain: Bip44Chain,
        new_known_addresses_number: u32,
    ) -> MmResult<(), AccountUpdatingError> {
        utxo_common::set_known_addresses_number(self, hd_wallet, hd_account, chain, new_known_addresses_number).await
    }
}

#[async_trait]
impl GetNewAddressRpcOps for UtxoStandardCoin {
    async fn get_new_address_rpc(
        &self,
        params: GetNewAddressParams,
    ) -> MmResult<GetNewAddressResponse, GetNewAddressRpcError> {
        get_new_address::common_impl::get_new_address_rpc(self, params).await
    }
}

#[async_trait]
impl HDWalletBalanceOps for UtxoStandardCoin {
    type HDAddressScanner = UtxoAddressScanner;

    async fn produce_hd_address_scanner(&self) -> BalanceResult<Self::HDAddressScanner> {
        utxo_common::produce_hd_address_scanner(self).await
    }

    async fn enable_hd_wallet<XPubExtractor>(
        &self,
        hd_wallet: &Self::HDWallet,
        xpub_extractor: &XPubExtractor,
        params: EnabledCoinBalanceParams,
    ) -> MmResult<HDWalletBalance, EnableCoinBalanceError>
    where
        XPubExtractor: HDXPubExtractor + Sync,
    {
        coin_balance::common_impl::enable_hd_wallet(self, hd_wallet, xpub_extractor, params).await
    }

    async fn scan_for_new_addresses(
        &self,
        hd_wallet: &Self::HDWallet,
        hd_account: &mut Self::HDAccount,
        address_scanner: &Self::HDAddressScanner,
        gap_limit: u32,
    ) -> BalanceResult<Vec<HDAddressBalance>> {
        utxo_common::scan_for_new_addresses(self, hd_wallet, hd_account, address_scanner, gap_limit).await
    }

    async fn all_known_addresses_balances(&self, hd_account: &Self::HDAccount) -> BalanceResult<Vec<HDAddressBalance>> {
        utxo_common::all_known_addresses_balances(self, hd_account).await
    }

    async fn known_address_balance(&self, address: &Self::Address) -> BalanceResult<CoinBalance> {
        utxo_common::address_balance(self, address).await
    }

    async fn known_addresses_balances(
        &self,
        addresses: Vec<Self::Address>,
    ) -> BalanceResult<Vec<(Self::Address, CoinBalance)>> {
        utxo_common::addresses_balances(self, addresses).await
    }
}

impl HDWalletCoinWithStorageOps for UtxoStandardCoin {
    fn hd_wallet_storage<'a>(&self, hd_wallet: &'a Self::HDWallet) -> &'a HDWalletCoinStorage {
        &hd_wallet.hd_wallet_storage
    }
}

#[async_trait]
impl AccountBalanceRpcOps for UtxoStandardCoin {
    async fn account_balance_rpc(
        &self,
        params: AccountBalanceParams,
    ) -> MmResult<HDAccountBalanceResponse, HDAccountBalanceRpcError> {
        account_balance::common_impl::account_balance_rpc(self, params).await
    }
}

#[async_trait]
impl InitAccountBalanceRpcOps for UtxoStandardCoin {
    async fn init_account_balance_rpc(
        &self,
        params: InitAccountBalanceParams,
    ) -> MmResult<HDAccountBalance, HDAccountBalanceRpcError> {
        init_account_balance::common_impl::init_account_balance_rpc(self, params).await
    }
}

#[async_trait]
impl InitScanAddressesRpcOps for UtxoStandardCoin {
    async fn init_scan_for_new_addresses_rpc(
        &self,
        params: ScanAddressesParams,
    ) -> MmResult<ScanAddressesResponse, HDAccountBalanceRpcError> {
        init_scan_for_new_addresses::common_impl::scan_for_new_addresses_rpc(self, params).await
    }
}

#[async_trait]
impl InitCreateAccountRpcOps for UtxoStandardCoin {
    async fn init_create_account_rpc<XPubExtractor>(
        &self,
        params: CreateNewAccountParams,
        state: CreateAccountState,
        xpub_extractor: &XPubExtractor,
    ) -> MmResult<HDAccountBalance, CreateAccountRpcError>
    where
        XPubExtractor: HDXPubExtractor + Sync,
    {
        init_create_account::common_impl::init_create_new_account_rpc(self, params, state, xpub_extractor).await
    }

    async fn revert_creating_account(&self, account_id: u32) {
        init_create_account::common_impl::revert_creating_account(self, account_id).await
    }
}

#[async_trait]
impl CoinWithTxHistoryV2 for UtxoStandardCoin {
    fn history_wallet_id(&self) -> WalletId { utxo_common::utxo_tx_history_v2_common::history_wallet_id(self.as_ref()) }

    async fn get_tx_history_filters(
        &self,
        target: MyTxHistoryTarget,
    ) -> MmResult<GetTxHistoryFilters, MyTxHistoryErrorV2> {
        utxo_common::utxo_tx_history_v2_common::get_tx_history_filters(self, target).await
    }
}

#[async_trait]
impl UtxoTxHistoryOps for UtxoStandardCoin {
    async fn my_addresses(&self) -> MmResult<HashSet<Address>, UtxoMyAddressesHistoryError> {
        utxo_common::utxo_tx_history_v2_common::my_addresses(self).await
    }

    async fn tx_details_by_hash<Storage>(
        &self,
        params: UtxoTxDetailsParams<'_, Storage>,
    ) -> MmResult<Vec<TransactionDetails>, UtxoTxDetailsError>
    where
        Storage: TxHistoryStorage,
    {
        utxo_common::utxo_tx_history_v2_common::tx_details_by_hash(self, params).await
    }

    async fn tx_from_storage_or_rpc<Storage: TxHistoryStorage>(
        &self,
        tx_hash: &H256Json,
        storage: &Storage,
    ) -> MmResult<UtxoTx, UtxoTxDetailsError> {
        utxo_common::utxo_tx_history_v2_common::tx_from_storage_or_rpc(self, tx_hash, storage).await
    }

    async fn request_tx_history(
        &self,
        metrics: MetricsArc,
        for_addresses: &HashSet<Address>,
    ) -> RequestTxHistoryResult {
        utxo_common::utxo_tx_history_v2_common::request_tx_history(self, metrics, for_addresses).await
    }

    async fn get_block_timestamp(&self, height: u64) -> MmResult<u64, GetBlockHeaderError> {
        self.as_ref().rpc_client.get_block_timestamp(height).await
    }

    async fn my_addresses_balances(&self) -> BalanceResult<HashMap<String, BigDecimal>> {
        utxo_common::utxo_tx_history_v2_common::my_addresses_balances(self).await
    }

    fn address_from_str(&self, address: &str) -> MmResult<Address, AddrFromStrError> {
        utxo_common::checked_address_from_str(self, address)
    }

    fn set_history_sync_state(&self, new_state: HistorySyncState) {
        *self.as_ref().history_sync_state.lock().unwrap() = new_state;
    }
}
