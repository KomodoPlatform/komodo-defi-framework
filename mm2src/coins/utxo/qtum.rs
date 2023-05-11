use super::*;
use crate::coin_balance::{self, EnableCoinBalanceError, EnabledCoinBalanceParams, HDAccountBalance, HDAddressBalance,
                          HDWalletBalance, HDWalletBalanceOps};
use crate::coin_errors::MyAddressError;
use crate::hd_confirm_address::HDConfirmAddress;
use crate::hd_pubkey::{ExtractExtendedPubkey, HDExtractPubkeyError, HDXPubExtractor};
use crate::hd_wallet::{AccountUpdatingError, AddressDerivingResult, HDAccountMut, NewAccountCreatingError,
                       NewAddressDeriveConfirmError};
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
use crate::utxo::utxo_builder::{MergeUtxoArcOps, UtxoCoinBuildError, UtxoCoinBuilder, UtxoCoinBuilderCommonOps,
                                UtxoFieldsWithGlobalHDBuilder, UtxoFieldsWithHardwareWalletBuilder,
                                UtxoFieldsWithIguanaSecretBuilder};
use crate::utxo::utxo_tx_history_v2::{UtxoMyAddressesHistoryError, UtxoTxDetailsError, UtxoTxDetailsParams,
                                      UtxoTxHistoryOps};
use crate::{eth, CanRefundHtlc, CheckIfMyPaymentSentArgs, CoinBalance, CoinWithDerivationMethod, ConfirmPaymentInput,
            DelegationError, DelegationFut, GetWithdrawSenderAddress, IguanaPrivKey, MakerSwapTakerCoin, MmCoinEnum,
            NegotiateSwapContractAddrErr, PaymentInstructionArgs, PaymentInstructions, PaymentInstructionsErr,
            PrivKeyBuildPolicy, RefundError, RefundPaymentArgs, RefundResult, SearchForSwapTxSpendInput,
            SendMakerPaymentSpendPreimageInput, SendPaymentArgs, SignatureResult, SpendPaymentArgs, StakingInfosFut,
            SwapOps, TakerSwapMakerCoin, TradePreimageValue, TransactionFut, TxMarshalingErr,
            UnexpectedDerivationMethod, ValidateAddressResult, ValidateFeeArgs, ValidateInstructionsErr,
            ValidateOtherPubKeyErr, ValidatePaymentError, ValidatePaymentFut, ValidatePaymentInput,
            VerificationResult, WaitForHTLCTxSpendArgs, WatcherOps, WatcherReward, WatcherRewardError,
            WatcherSearchForSwapTxSpendInput, WatcherValidatePaymentInput, WatcherValidateTakerFeeInput, WithdrawFut,
            WithdrawSenderAddress};
use common::executor::{AbortableSystem, AbortedError};
use crypto::Bip44Chain;
use ethereum_types::H160;
use futures::{FutureExt, TryFutureExt};
use keys::AddressHashEnum;
use mm2_metrics::MetricsArc;
use mm2_number::MmNumber;
use serde::Serialize;
use serialization::CoinVariant;
use utxo_signer::UtxoSignerOps;

#[derive(Debug, Display)]
pub enum Qrc20AddressError {
    UnexpectedDerivationMethod(String),
    ScriptHashTypeNotSupported { script_hash_type: String },
}

impl From<UnexpectedDerivationMethod> for Qrc20AddressError {
    fn from(e: UnexpectedDerivationMethod) -> Self { Qrc20AddressError::UnexpectedDerivationMethod(e.to_string()) }
}

impl From<ScriptHashTypeNotSupported> for Qrc20AddressError {
    fn from(e: ScriptHashTypeNotSupported) -> Self {
        Qrc20AddressError::ScriptHashTypeNotSupported {
            script_hash_type: e.script_hash_type,
        }
    }
}

#[derive(Debug, Display)]
pub struct ScriptHashTypeNotSupported {
    pub script_hash_type: String,
}

impl From<ScriptHashTypeNotSupported> for WithdrawError {
    fn from(e: ScriptHashTypeNotSupported) -> Self { WithdrawError::InvalidAddress(e.to_string()) }
}

#[path = "qtum_delegation.rs"] mod qtum_delegation;
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

pub trait QtumDelegationOps {
    fn add_delegation(&self, request: QtumDelegationRequest) -> DelegationFut;

    fn get_delegation_infos(&self) -> StakingInfosFut;

    fn remove_delegation(&self) -> DelegationFut;

    #[allow(clippy::result_large_err)]
    fn generate_pod(&self, addr_hash: AddressHashEnum) -> Result<keys::Signature, MmError<DelegationError>>;
}

#[async_trait]
pub trait QtumBasedCoin: UtxoCommonOps + MarketCoinOps {
    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> {
        let to_address_format: QtumAddressFormat =
            json::from_value(to_address_format).map_err(|e| ERRL!("Error on parse Qtum address format {:?}", e))?;
        let from_address = try_s!(self.utxo_address_from_any_format(from));
        match to_address_format {
            QtumAddressFormat::Wallet => Ok(from_address.to_string()),
            QtumAddressFormat::Contract => Ok(try_s!(display_as_contract_address(from_address))),
        }
    }

    /// Try to parse address from either wallet (UTXO) format or contract format.
    fn utxo_address_from_any_format(&self, from: &str) -> Result<Address, String> {
        let utxo_err = match Address::from_str(from) {
            Ok(addr) => {
                let is_p2pkh = addr.prefix == self.as_ref().conf.pub_addr_prefix
                    && addr.t_addr_prefix == self.as_ref().conf.pub_t_addr_prefix;
                if is_p2pkh {
                    return Ok(addr);
                }
                "Address has invalid prefixes".to_string()
            },
            Err(e) => e.to_string(),
        };
        let utxo_segwit_err = match Address::from_segwitaddress(
            from,
            self.as_ref().conf.checksum_type,
            self.as_ref().conf.pub_addr_prefix,
            self.as_ref().conf.pub_t_addr_prefix,
        ) {
            Ok(addr) => {
                let is_segwit =
                    addr.hrp.is_some() && addr.hrp == self.as_ref().conf.bech32_hrp && self.as_ref().conf.segwit;
                if is_segwit {
                    return Ok(addr);
                }
                "Address has invalid hrp".to_string()
            },
            Err(e) => e,
        };
        let contract_err = match contract_addr_from_str(from) {
            Ok(contract_addr) => return Ok(self.utxo_addr_from_contract_addr(contract_addr)),
            Err(e) => e,
        };
        ERR!(
            "error on parse wallet address: {:?}, {:?}, error on parse contract address: {:?}",
            utxo_err,
            utxo_segwit_err,
            contract_err,
        )
    }

    fn utxo_addr_from_contract_addr(&self, address: H160) -> Address {
        let utxo = self.as_ref();
        Address {
            prefix: utxo.conf.pub_addr_prefix,
            t_addr_prefix: utxo.conf.pub_t_addr_prefix,
            hash: AddressHashEnum::AddressHash(address.0.into()),
            checksum_type: utxo.conf.checksum_type,
            hrp: utxo.conf.bech32_hrp.clone(),
            addr_format: self.addr_format().clone(),
        }
    }

    fn my_addr_as_contract_addr(&self) -> MmResult<H160, Qrc20AddressError> {
        let my_address = self.as_ref().derivation_method.single_addr_or_err()?.clone();
        contract_addr_from_utxo_addr(my_address).mm_err(Qrc20AddressError::from)
    }

    fn utxo_address_from_contract_addr(&self, address: H160) -> Address {
        let utxo = self.as_ref();
        Address {
            prefix: utxo.conf.pub_addr_prefix,
            t_addr_prefix: utxo.conf.pub_t_addr_prefix,
            hash: AddressHashEnum::AddressHash(address.0.into()),
            checksum_type: utxo.conf.checksum_type,
            hrp: utxo.conf.bech32_hrp.clone(),
            addr_format: self.addr_format().clone(),
        }
    }

    fn contract_address_from_raw_pubkey(&self, pubkey: &[u8]) -> Result<H160, String> {
        let utxo = self.as_ref();
        let qtum_address = try_s!(utxo_common::address_from_raw_pubkey(
            pubkey,
            utxo.conf.pub_addr_prefix,
            utxo.conf.pub_t_addr_prefix,
            utxo.conf.checksum_type,
            utxo.conf.bech32_hrp.clone(),
            self.addr_format().clone()
        ));
        let contract_addr = try_s!(contract_addr_from_utxo_addr(qtum_address));
        Ok(contract_addr)
    }

    fn is_qtum_unspent_mature(&self, output: &RpcTransaction) -> bool {
        let is_qrc20_coinbase = output.vout.iter().any(|x| x.is_empty());
        let is_coinbase = output.is_coinbase() || is_qrc20_coinbase;
        !is_coinbase || output.confirmations >= self.as_ref().conf.mature_confirmations
    }
}

pub struct QtumCoinBuilder<'a> {
    ctx: &'a MmArc,
    ticker: &'a str,
    conf: &'a Json,
    activation_params: &'a UtxoActivationParams,
    priv_key_policy: PrivKeyBuildPolicy,
}

#[async_trait]
impl<'a> UtxoCoinBuilderCommonOps for QtumCoinBuilder<'a> {
    fn ctx(&self) -> &MmArc { self.ctx }

    fn conf(&self) -> &Json { self.conf }

    fn activation_params(&self) -> &UtxoActivationParams { self.activation_params }

    fn ticker(&self) -> &str { self.ticker }

    fn check_utxo_maturity(&self) -> bool { self.activation_params().check_utxo_maturity.unwrap_or(true) }
}

impl<'a> UtxoFieldsWithIguanaSecretBuilder for QtumCoinBuilder<'a> {}

impl<'a> UtxoFieldsWithGlobalHDBuilder for QtumCoinBuilder<'a> {}

impl<'a> UtxoFieldsWithHardwareWalletBuilder for QtumCoinBuilder<'a> {}

#[async_trait]
impl<'a> UtxoCoinBuilder for QtumCoinBuilder<'a> {
    type ResultCoin = QtumCoin;
    type Error = UtxoCoinBuildError;

    fn priv_key_policy(&self) -> PrivKeyBuildPolicy { self.priv_key_policy.clone() }

    async fn build(self) -> MmResult<Self::ResultCoin, Self::Error> {
        let utxo = self.build_utxo_fields().await?;
        let utxo_arc = UtxoArc::new(utxo);

        self.spawn_merge_utxo_loop_if_required(&utxo_arc, QtumCoin::from);
        Ok(QtumCoin::from(utxo_arc))
    }
}

impl<'a> MergeUtxoArcOps<QtumCoin> for QtumCoinBuilder<'a> {}

impl<'a> QtumCoinBuilder<'a> {
    pub fn new(
        ctx: &'a MmArc,
        ticker: &'a str,
        conf: &'a Json,
        activation_params: &'a UtxoActivationParams,
        priv_key_policy: PrivKeyBuildPolicy,
    ) -> Self {
        QtumCoinBuilder {
            ctx,
            ticker,
            conf,
            activation_params,
            priv_key_policy,
        }
    }
}

#[derive(Clone)]
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

pub async fn qtum_coin_with_policy(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    activation_params: &UtxoActivationParams,
    priv_key_policy: PrivKeyBuildPolicy,
) -> Result<QtumCoin, String> {
    let coin = try_s!(
        QtumCoinBuilder::new(ctx, ticker, conf, activation_params, priv_key_policy)
            .build()
            .await
    );
    Ok(coin)
}

pub async fn qtum_coin_with_priv_key(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    activation_params: &UtxoActivationParams,
    priv_key: IguanaPrivKey,
) -> Result<QtumCoin, String> {
    let priv_key_policy = PrivKeyBuildPolicy::IguanaPrivKey(priv_key);
    qtum_coin_with_policy(ctx, ticker, conf, activation_params, priv_key_policy).await
}

impl QtumBasedCoin for QtumCoin {}

#[derive(Clone, Debug, Deserialize)]
pub struct QtumDelegationRequest {
    pub address: String,
    pub fee: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QtumStakingInfosDetails {
    pub amount: BigDecimal,
    pub staker: Option<String>,
    pub am_i_staking: bool,
    pub is_staking_supported: bool,
}

// if mockable is placed before async_trait there is `munmap_chunk(): invalid pointer` error on async fn mocking attempt
#[async_trait]
#[cfg_attr(test, mockable)]
impl UtxoTxBroadcastOps for QtumCoin {
    async fn broadcast_tx(&self, tx: &UtxoTx) -> Result<H256Json, MmError<BroadcastTxErr>> {
        utxo_common::broadcast_tx(self, tx).await
    }
}

#[async_trait]
#[cfg_attr(test, mockable)]
impl UtxoTxGenerationOps for QtumCoin {
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
impl GetUtxoListOps for QtumCoin {
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
impl GetUtxoMapOps for QtumCoin {
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

#[async_trait]
#[cfg_attr(test, mockable)]
impl UtxoCommonOps for QtumCoin {
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
        utxo_common::get_current_mtp(&self.utxo_arc, CoinVariant::Qtum).await
    }

    fn is_unspent_mature(&self, output: &RpcTransaction) -> bool { self.is_qtum_unspent_mature(output) }

    async fn calc_interest_of_tx(
        &self,
        _tx: &UtxoTx,
        _input_transactions: &mut HistoryUtxoTxMap,
    ) -> UtxoRpcResult<u64> {
        MmError::err(UtxoRpcError::Internal(
            "QTUM coin doesn't support transaction rewards".to_owned(),
        ))
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
impl UtxoStandardOps for QtumCoin {
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
impl SwapOps for QtumCoin {
    #[inline]
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, _uuid: &[u8]) -> TransactionFut {
        utxo_common::send_taker_fee(self.clone(), fee_addr, amount)
    }

    #[inline]
    fn send_maker_payment(&self, maker_payment_args: SendPaymentArgs) -> TransactionFut {
        utxo_common::send_maker_payment(self.clone(), maker_payment_args)
    }

    #[inline]
    fn send_taker_payment(&self, taker_payment_args: SendPaymentArgs) -> TransactionFut {
        utxo_common::send_taker_payment(self.clone(), taker_payment_args)
    }

    #[inline]
    fn send_maker_spends_taker_payment(&self, maker_spends_payment_args: SpendPaymentArgs) -> TransactionFut {
        utxo_common::send_maker_spends_taker_payment(self.clone(), maker_spends_payment_args)
    }

    #[inline]
    fn send_taker_spends_maker_payment(&self, taker_spends_payment_args: SpendPaymentArgs) -> TransactionFut {
        utxo_common::send_taker_spends_maker_payment(self.clone(), taker_spends_payment_args)
    }

    #[inline]
    fn send_taker_refunds_payment(&self, taker_refunds_payment_args: RefundPaymentArgs) -> TransactionFut {
        utxo_common::send_taker_refunds_payment(self.clone(), taker_refunds_payment_args)
    }

    #[inline]
    fn send_maker_refunds_payment(&self, maker_refunds_payment_args: RefundPaymentArgs) -> TransactionFut {
        utxo_common::send_maker_refunds_payment(self.clone(), maker_refunds_payment_args)
    }

    fn validate_fee(&self, validate_fee_args: ValidateFeeArgs) -> ValidatePaymentFut<()> {
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
        if_my_payment_sent_args: CheckIfMyPaymentSentArgs,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        utxo_common::check_if_my_payment_sent(
            self.clone(),
            if_my_payment_sent_args.time_lock,
            if_my_payment_sent_args.other_pub,
            if_my_payment_sent_args.secret_hash,
            if_my_payment_sent_args.swap_unique_data,
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
    fn check_tx_signed_by_pub(&self, tx: &[u8], expected_pub: &[u8]) -> Result<bool, MmError<ValidatePaymentError>> {
        utxo_common::check_all_inputs_signed_by_pub(tx, expected_pub)
    }

    #[inline]
    async fn extract_secret(
        &self,
        secret_hash: &[u8],
        spend_tx: &[u8],
        _watcher_reward: bool,
    ) -> Result<Vec<u8>, String> {
        utxo_common::extract_secret(secret_hash, spend_tx)
    }

    fn is_auto_refundable(&self) -> bool { false }

    async fn wait_for_htlc_refund(&self, _tx: &[u8], _locktime: u64) -> RefundResult<()> {
        MmError::err(RefundError::Internal(
            "wait_for_htlc_refund is not supported for this coin!".into(),
        ))
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

    fn derive_htlc_key_pair(&self, swap_unique_data: &[u8]) -> KeyPair {
        utxo_common::derive_htlc_key_pair(self.as_ref(), swap_unique_data)
    }

    fn derive_htlc_pubkey(&self, swap_unique_data: &[u8]) -> Vec<u8> {
        utxo_common::derive_htlc_pubkey(self, swap_unique_data)
    }

    #[inline]
    fn validate_other_pubkey(&self, raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> {
        utxo_common::validate_other_pubkey(raw_pubkey)
    }

    async fn maker_payment_instructions(
        &self,
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        Ok(None)
    }

    async fn taker_payment_instructions(
        &self,
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        Ok(None)
    }

    fn validate_maker_payment_instructions(
        &self,
        _instructions: &[u8],
        _args: PaymentInstructionArgs,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        MmError::err(ValidateInstructionsErr::UnsupportedCoin(self.ticker().to_string()))
    }

    fn validate_taker_payment_instructions(
        &self,
        _instructions: &[u8],
        _args: PaymentInstructionArgs,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        MmError::err(ValidateInstructionsErr::UnsupportedCoin(self.ticker().to_string()))
    }

    fn is_supported_by_watchers(&self) -> bool { true }
}

#[async_trait]
impl TakerSwapMakerCoin for QtumCoin {
    async fn on_taker_payment_refund_start(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_taker_payment_refund_success(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
impl MakerSwapTakerCoin for QtumCoin {
    async fn on_maker_payment_refund_start(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_maker_payment_refund_success(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
impl WatcherOps for QtumCoin {
    #[inline]
    fn create_maker_payment_spend_preimage(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        utxo_common::create_maker_payment_spend_preimage(
            self,
            maker_payment_tx,
            time_lock,
            maker_pub,
            secret_hash,
            swap_unique_data,
        )
    }

    #[inline]
    fn send_maker_payment_spend_preimage(&self, input: SendMakerPaymentSpendPreimageInput) -> TransactionFut {
        utxo_common::send_maker_payment_spend_preimage(self, input)
    }

    #[inline]
    fn create_taker_payment_refund_preimage(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        utxo_common::create_taker_payment_refund_preimage(
            self,
            taker_payment_tx,
            time_lock,
            maker_pub,
            secret_hash,
            swap_unique_data,
        )
    }

    #[inline]
    fn send_taker_payment_refund_preimage(&self, watcher_refunds_payment_args: RefundPaymentArgs) -> TransactionFut {
        utxo_common::send_taker_payment_refund_preimage(self, watcher_refunds_payment_args)
    }

    #[inline]
    fn watcher_validate_taker_fee(&self, input: WatcherValidateTakerFeeInput) -> ValidatePaymentFut<()> {
        utxo_common::watcher_validate_taker_fee(self, input, utxo_common::DEFAULT_FEE_VOUT)
    }

    #[inline]
    fn watcher_validate_taker_payment(&self, input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
        utxo_common::watcher_validate_taker_payment(self, input)
    }

    async fn watcher_search_for_swap_tx_spend(
        &self,
        input: WatcherSearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        utxo_common::watcher_search_for_swap_tx_spend(self, input, utxo_common::DEFAULT_SWAP_VOUT).await
    }

    async fn get_taker_watcher_reward(
        &self,
        _other_coin: &MmCoinEnum,
        _coin_amount: Option<BigDecimal>,
        _other_coin_amount: Option<BigDecimal>,
        _reward_amount: Option<BigDecimal>,
        _wait_until: u64,
    ) -> Result<WatcherReward, MmError<WatcherRewardError>> {
        unimplemented!()
    }

    async fn get_maker_watcher_reward(
        &self,
        _other_coin: &MmCoinEnum,
        _reward_amount: Option<BigDecimal>,
        _wait_until: u64,
    ) -> Result<Option<WatcherReward>, MmError<WatcherRewardError>> {
        unimplemented!()
    }
}

impl MarketCoinOps for QtumCoin {
    fn ticker(&self) -> &str { &self.utxo_arc.conf.ticker }

    fn my_address(&self) -> MmResult<String, MyAddressError> { utxo_common::my_address(self) }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> {
        let pubkey = utxo_common::my_public_key(&self.utxo_arc)?;
        Ok(pubkey.to_string())
    }

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

    fn wait_for_confirmations(&self, input: ConfirmPaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        utxo_common::wait_for_confirmations(&self.utxo_arc, input)
    }

    fn wait_for_htlc_tx_spend(&self, args: WaitForHTLCTxSpendArgs<'_>) -> TransactionFut {
        utxo_common::wait_for_output_spend(
            &self.utxo_arc,
            args.tx_bytes,
            utxo_common::DEFAULT_SWAP_VOUT,
            args.from_block,
            args.wait_until,
            args.check_every,
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
impl MmCoin for QtumCoin {
    fn is_asset_chain(&self) -> bool { utxo_common::is_asset_chain(&self.utxo_arc) }

    fn spawner(&self) -> CoinFutSpawner { CoinFutSpawner::new(&self.as_ref().abortable_system) }

    fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut {
        Box::new(utxo_common::get_raw_transaction(&self.utxo_arc, req).boxed().compat())
    }

    fn get_tx_hex_by_hash(&self, tx_hash: Vec<u8>) -> RawTransactionFut {
        Box::new(
            utxo_common::get_tx_hex_by_hash(&self.utxo_arc, tx_hash)
                .boxed()
                .compat(),
        )
    }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        Box::new(utxo_common::withdraw(self.clone(), req).boxed().compat())
    }

    fn decimals(&self) -> u8 { utxo_common::decimals(&self.utxo_arc) }

    /// Check if the `to_address_format` is standard and if the `from` address is standard UTXO address.
    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> {
        QtumBasedCoin::convert_to_address(self, from, to_address_format)
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

    fn get_receiver_trade_fee(&self, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
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

    fn coin_protocol_info(&self, _amount_to_receive: Option<MmNumber>) -> Vec<u8> {
        utxo_common::coin_protocol_info(self)
    }

    fn is_coin_protocol_supported(
        &self,
        info: &Option<Vec<u8>>,
        _amount_to_send: Option<MmNumber>,
        _locktime: u64,
        _is_maker: bool,
    ) -> bool {
        utxo_common::is_coin_protocol_supported(self, info)
    }

    fn on_disabled(&self) -> Result<(), AbortedError> { AbortableSystem::abort_all(&self.as_ref().abortable_system) }

    fn on_token_deactivated(&self, _ticker: &str) {}
}

#[async_trait]
impl GetWithdrawSenderAddress for QtumCoin {
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
impl InitWithdrawCoin for QtumCoin {
    async fn init_withdraw(
        &self,
        ctx: MmArc,
        req: WithdrawRequest,
        task_handle: &WithdrawTaskHandle,
    ) -> Result<TransactionDetails, MmError<WithdrawError>> {
        utxo_common::init_withdraw(ctx, self.clone(), req, task_handle).await
    }
}

impl UtxoSignerOps for QtumCoin {
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

impl CoinWithDerivationMethod for QtumCoin {
    type Address = Address;
    type HDWallet = UtxoHDWallet;

    fn derivation_method(&self) -> &DerivationMethod<Self::Address, Self::HDWallet> {
        utxo_common::derivation_method(self.as_ref())
    }
}

#[async_trait]
impl ExtractExtendedPubkey for QtumCoin {
    type ExtendedPublicKey = Secp256k1ExtendedPublicKey;

    async fn extract_extended_pubkey<XPubExtractor>(
        &self,
        xpub_extractor: &XPubExtractor,
        derivation_path: DerivationPath,
    ) -> MmResult<Self::ExtendedPublicKey, HDExtractPubkeyError>
    where
        XPubExtractor: HDXPubExtractor,
    {
        utxo_common::extract_extended_pubkey(&self.utxo_arc.conf, xpub_extractor, derivation_path).await
    }
}

#[async_trait]
impl HDWalletCoinOps for QtumCoin {
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

    async fn generate_and_confirm_new_address<ConfirmAddress>(
        &self,
        hd_wallet: &Self::HDWallet,
        hd_account: &mut Self::HDAccount,
        chain: Bip44Chain,
        confirm_address: &ConfirmAddress,
    ) -> MmResult<HDAddress<Self::Address, Self::Pubkey>, NewAddressDeriveConfirmError>
    where
        ConfirmAddress: HDConfirmAddress,
    {
        utxo_common::generate_and_confirm_new_address(self, hd_wallet, hd_account, chain, confirm_address).await
    }

    async fn create_new_account<'a, XPubExtractor>(
        &self,
        hd_wallet: &'a Self::HDWallet,
        xpub_extractor: &XPubExtractor,
    ) -> MmResult<HDAccountMut<'a, Self::HDAccount>, NewAccountCreatingError>
    where
        XPubExtractor: HDXPubExtractor,
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
impl HDWalletBalanceOps for QtumCoin {
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
        XPubExtractor: HDXPubExtractor,
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

impl HDWalletCoinWithStorageOps for QtumCoin {
    fn hd_wallet_storage<'a>(&self, hd_wallet: &'a Self::HDWallet) -> &'a HDWalletCoinStorage {
        &hd_wallet.hd_wallet_storage
    }
}

#[async_trait]
impl GetNewAddressRpcOps for QtumCoin {
    async fn get_new_address_rpc_without_conf(
        &self,
        params: GetNewAddressParams,
    ) -> MmResult<GetNewAddressResponse, GetNewAddressRpcError> {
        get_new_address::common_impl::get_new_address_rpc_without_conf(self, params).await
    }

    async fn get_new_address_rpc<ConfirmAddress>(
        &self,
        params: GetNewAddressParams,
        confirm_address: &ConfirmAddress,
    ) -> MmResult<GetNewAddressResponse, GetNewAddressRpcError>
    where
        ConfirmAddress: HDConfirmAddress,
    {
        get_new_address::common_impl::get_new_address_rpc(self, params, confirm_address).await
    }
}

#[async_trait]
impl AccountBalanceRpcOps for QtumCoin {
    async fn account_balance_rpc(
        &self,
        params: AccountBalanceParams,
    ) -> MmResult<HDAccountBalanceResponse, HDAccountBalanceRpcError> {
        account_balance::common_impl::account_balance_rpc(self, params).await
    }
}

#[async_trait]
impl InitAccountBalanceRpcOps for QtumCoin {
    async fn init_account_balance_rpc(
        &self,
        params: InitAccountBalanceParams,
    ) -> MmResult<HDAccountBalance, HDAccountBalanceRpcError> {
        init_account_balance::common_impl::init_account_balance_rpc(self, params).await
    }
}

#[async_trait]
impl InitScanAddressesRpcOps for QtumCoin {
    async fn init_scan_for_new_addresses_rpc(
        &self,
        params: ScanAddressesParams,
    ) -> MmResult<ScanAddressesResponse, HDAccountBalanceRpcError> {
        init_scan_for_new_addresses::common_impl::scan_for_new_addresses_rpc(self, params).await
    }
}

#[async_trait]
impl InitCreateAccountRpcOps for QtumCoin {
    async fn init_create_account_rpc<XPubExtractor>(
        &self,
        params: CreateNewAccountParams,
        state: CreateAccountState,
        xpub_extractor: &XPubExtractor,
    ) -> MmResult<HDAccountBalance, CreateAccountRpcError>
    where
        XPubExtractor: HDXPubExtractor,
    {
        init_create_account::common_impl::init_create_new_account_rpc(self, params, state, xpub_extractor).await
    }

    async fn revert_creating_account(&self, account_id: u32) {
        init_create_account::common_impl::revert_creating_account(self, account_id).await
    }
}

#[async_trait]
impl CoinWithTxHistoryV2 for QtumCoin {
    fn history_wallet_id(&self) -> WalletId { utxo_common::utxo_tx_history_v2_common::history_wallet_id(self.as_ref()) }

    async fn get_tx_history_filters(
        &self,
        target: MyTxHistoryTarget,
    ) -> MmResult<GetTxHistoryFilters, MyTxHistoryErrorV2> {
        utxo_common::utxo_tx_history_v2_common::get_tx_history_filters(self, target).await
    }
}

#[async_trait]
impl UtxoTxHistoryOps for QtumCoin {
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

/// Parse contract address (H160) from string.
/// Qtum Contract addresses have another checksum verification algorithm, because of this do not use [`eth::valid_addr_from_str`].
pub fn contract_addr_from_str(addr: &str) -> Result<H160, String> { eth::addr_from_str(addr) }

pub fn contract_addr_from_utxo_addr(address: Address) -> MmResult<H160, ScriptHashTypeNotSupported> {
    match address.hash {
        AddressHashEnum::AddressHash(h) => Ok(h.take().into()),
        AddressHashEnum::WitnessScriptHash(_) => MmError::err(ScriptHashTypeNotSupported {
            script_hash_type: "Witness".to_owned(),
        }),
    }
}

pub fn display_as_contract_address(address: Address) -> MmResult<String, ScriptHashTypeNotSupported> {
    let address = qtum::contract_addr_from_utxo_addr(address)?;
    Ok(format!("{:#02x}", address))
}
