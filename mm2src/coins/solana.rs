use super::{CoinBalance, HistorySyncState, MarketCoinOps, MmCoin, SwapOps, TradeFee, TransactionEnum};
use crate::solana::solana_common::{lamports_to_sol, PrepareTransferData, SufficientBalanceError};
use crate::solana::spl::SplTokenInfo;
use crate::{BalanceError, BalanceFut, FeeApproxStage, FoundSwapTxSpend, NegotiateSwapContractAddrErr,
            RawTransactionFut, RawTransactionRequest, SearchForSwapTxSpendInput, SignatureResult, TradePreimageFut,
            TradePreimageResult, TradePreimageValue, TransactionDetails, TransactionFut, TransactionType,
            UnexpectedDerivationMethod, ValidateAddressResult, ValidatePaymentInput, VerificationResult,
            WithdrawError, WithdrawFut, WithdrawRequest, WithdrawResult};
use async_trait::async_trait;
use base58::ToBase58;
use bincode::{deserialize, serialize};
use common::{async_blocking, now_ms};
use derive_more::Display;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::{BigDecimal, MmNumber};
use rpc::v1::types::Bytes as BytesJson;
use serde_json::{self as json, Value as Json};
use solana_client::rpc_request::TokenAccountsFilter;
use solana_client::{client_error::{ClientError, ClientErrorKind},
                    rpc_client::RpcClient};
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_sdk::program_error::ProgramError;
use solana_sdk::pubkey::ParsePubkeyError;
use solana_sdk::transaction::Transaction;
use solana_sdk::{pubkey::Pubkey,
                 signature::{Keypair, Signer}};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;
use std::{convert::TryFrom,
          fmt::{Debug, Formatter, Result as FmtResult},
          ops::Deref,
          sync::Arc};

pub mod solana_common;
#[cfg(test)] mod solana_common_tests;
mod solana_decode_tx_helpers;
#[cfg(test)] mod solana_tests;
pub mod spl;
#[cfg(test)] mod spl_tests;

pub const SOLANA_DEFAULT_DECIMALS: u64 = 9;
pub const LAMPORTS_DUMMY_AMOUNT: u64 = 10;

#[async_trait]
pub trait SolanaCommonOps {
    fn rpc(&self) -> &RpcClient;

    fn is_token(&self) -> bool;

    async fn check_balance_and_prepare_transfer(
        &self,
        max: bool,
        amount: BigDecimal,
        fees: u64,
    ) -> Result<PrepareTransferData, MmError<SufficientBalanceError>>;
}

impl From<ClientError> for BalanceError {
    fn from(e: ClientError) -> Self {
        match e.kind {
            ClientErrorKind::Io(e) => BalanceError::Transport(e.to_string()),
            ClientErrorKind::Reqwest(e) => BalanceError::Transport(e.to_string()),
            ClientErrorKind::RpcError(e) => BalanceError::Transport(format!("{:?}", e)),
            ClientErrorKind::SerdeJson(e) => BalanceError::InvalidResponse(e.to_string()),
            ClientErrorKind::Custom(e) => BalanceError::Internal(e),
            ClientErrorKind::SigningError(_)
            | ClientErrorKind::TransactionError(_)
            | ClientErrorKind::FaucetError(_) => BalanceError::Internal("not_reacheable".to_string()),
        }
    }
}

impl From<ParsePubkeyError> for BalanceError {
    fn from(e: ParsePubkeyError) -> Self { BalanceError::Internal(format!("{:?}", e)) }
}

impl From<ClientError> for WithdrawError {
    fn from(e: ClientError) -> Self {
        match e.kind {
            ClientErrorKind::Io(e) => WithdrawError::Transport(e.to_string()),
            ClientErrorKind::Reqwest(e) => WithdrawError::Transport(e.to_string()),
            ClientErrorKind::RpcError(e) => WithdrawError::Transport(format!("{:?}", e)),
            ClientErrorKind::SerdeJson(e) => WithdrawError::InternalError(e.to_string()),
            ClientErrorKind::Custom(e) => WithdrawError::InternalError(e),
            ClientErrorKind::SigningError(_)
            | ClientErrorKind::TransactionError(_)
            | ClientErrorKind::FaucetError(_) => WithdrawError::InternalError("not_reacheable".to_string()),
        }
    }
}

impl From<ParsePubkeyError> for WithdrawError {
    fn from(e: ParsePubkeyError) -> Self { WithdrawError::InvalidAddress(format!("{:?}", e)) }
}

impl From<ProgramError> for WithdrawError {
    fn from(e: ProgramError) -> Self { WithdrawError::InternalError(format!("{:?}", e)) }
}

#[derive(Debug)]
pub enum AccountError {
    NotFundedError(String),
    ParsePubKeyError(String),
    ClientError(ClientErrorKind),
}

impl From<ClientError> for AccountError {
    fn from(e: ClientError) -> Self { AccountError::ClientError(e.kind) }
}

impl From<ParsePubkeyError> for AccountError {
    fn from(e: ParsePubkeyError) -> Self { AccountError::ParsePubKeyError(format!("{:?}", e)) }
}

impl From<AccountError> for WithdrawError {
    fn from(e: AccountError) -> Self {
        match e {
            AccountError::NotFundedError(_) => WithdrawError::ZeroBalanceToWithdrawMax,
            AccountError::ParsePubKeyError(err) => WithdrawError::InternalError(err),
            AccountError::ClientError(e) => WithdrawError::Transport(format!("{:?}", e)),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SolanaActivationParams {
    confirmation_commitment: CommitmentLevel,
    client_url: String,
}

#[derive(Debug, Display)]
pub enum SolanaFromLegacyReqErr {
    InvalidCommitmentLevel(String),
    InvalidClientParsing(json::Error),
    ClientNoAvailableNodes(String),
}

#[derive(Debug, Display)]
pub enum KeyPairCreationError {
    #[display(fmt = "Signature error: {}", _0)]
    SignatureError(ed25519_dalek::SignatureError),
    #[display(fmt = "KeyPairFromSeed error: {}", _0)]
    KeyPairFromSeed(String),
}

impl From<ed25519_dalek::SignatureError> for KeyPairCreationError {
    fn from(e: ed25519_dalek::SignatureError) -> Self { KeyPairCreationError::SignatureError(e) }
}

fn generate_keypair_from_slice(priv_key: &[u8]) -> Result<Keypair, MmError<KeyPairCreationError>> {
    let secret_key = ed25519_dalek::SecretKey::from_bytes(priv_key)?;
    let public_key = ed25519_dalek::PublicKey::from(&secret_key);
    let key_pair = ed25519_dalek::Keypair {
        secret: secret_key,
        public: public_key,
    };
    solana_sdk::signature::keypair_from_seed(key_pair.to_bytes().as_ref())
        .map_to_mm(|e| KeyPairCreationError::KeyPairFromSeed(e.to_string()))
}

pub async fn solana_coin_from_conf_and_params(
    ticker: &str,
    conf: &Json,
    params: SolanaActivationParams,
    priv_key: &[u8],
) -> Result<SolanaCoin, String> {
    let client = RpcClient::new_with_commitment(params.client_url.clone(), CommitmentConfig {
        commitment: params.confirmation_commitment,
    });
    let decimals = conf["decimals"].as_u64().unwrap_or(SOLANA_DEFAULT_DECIMALS) as u8;
    let key_pair = try_s!(generate_keypair_from_slice(priv_key));
    let my_address = key_pair.pubkey().to_string();
    let spl_tokens_infos = Arc::new(Mutex::new(HashMap::new()));
    let solana_coin = SolanaCoin(Arc::new(SolanaCoinImpl {
        my_address,
        key_pair,
        ticker: ticker.to_string(),
        client,
        decimals,
        spl_tokens_infos,
    }));
    Ok(solana_coin)
}

/// pImpl idiom.
pub struct SolanaCoinImpl {
    ticker: String,
    key_pair: Keypair,
    client: RpcClient,
    decimals: u8,
    my_address: String,
    spl_tokens_infos: Arc<Mutex<HashMap<String, SplTokenInfo>>>,
}

impl Debug for SolanaCoinImpl {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult { f.write_str(&*self.ticker) }
}

#[derive(Clone, Debug)]
pub struct SolanaCoin(Arc<SolanaCoinImpl>);
impl Deref for SolanaCoin {
    type Target = SolanaCoinImpl;
    fn deref(&self) -> &SolanaCoinImpl { &*self.0 }
}

#[async_trait]
impl SolanaCommonOps for SolanaCoin {
    fn rpc(&self) -> &RpcClient { &self.client }

    fn is_token(&self) -> bool { false }

    async fn check_balance_and_prepare_transfer(
        &self,
        max: bool,
        amount: BigDecimal,
        fees: u64,
    ) -> Result<PrepareTransferData, MmError<SufficientBalanceError>> {
        solana_common::check_balance_and_prepare_transfer(self, max, amount, fees).await
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SolanaFeeDetails {
    pub amount: BigDecimal,
}

async fn withdraw_base_coin_impl(coin: SolanaCoin, req: WithdrawRequest) -> WithdrawResult {
    let (hash, fees) = coin.estimate_withdraw_fees().await?;
    let res = coin
        .check_balance_and_prepare_transfer(req.max, req.amount.clone(), fees)
        .await?;
    let to = solana_sdk::pubkey::Pubkey::try_from(&*req.to)?;
    let tx = solana_sdk::system_transaction::transfer(&coin.key_pair, &to, res.lamports_to_send, hash);
    let serialized_tx = serialize(&tx).map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;
    let total_amount = lamports_to_sol(res.lamports_to_send);
    let received_by_me = if req.to == coin.my_address {
        total_amount.clone()
    } else {
        0.into()
    };
    let spent_by_me = &total_amount + &res.sol_required;
    Ok(TransactionDetails {
        tx_hex: serialized_tx.into(),
        tx_hash: tx.signatures[0].to_string(),
        from: vec![coin.my_address.clone()],
        to: vec![req.to],
        total_amount: spent_by_me.clone(),
        my_balance_change: &received_by_me - &spent_by_me,
        spent_by_me,
        received_by_me,
        block_height: 0,
        timestamp: now_ms() / 1000,
        fee_details: Some(
            SolanaFeeDetails {
                amount: res.sol_required,
            }
            .into(),
        ),
        coin: coin.ticker.clone(),
        internal_id: vec![].into(),
        kmd_rewards: None,
        transaction_type: TransactionType::StandardTransfer,
    })
}

async fn withdraw_impl(coin: SolanaCoin, req: WithdrawRequest) -> WithdrawResult {
    let validate_address_result = coin.validate_address(&*req.to);
    if !validate_address_result.is_valid {
        return MmError::err(WithdrawError::InvalidAddress(
            validate_address_result.reason.unwrap_or_else(|| "Unknown".to_string()),
        ));
    }
    withdraw_base_coin_impl(coin, req).await
}

impl SolanaCoin {
    pub async fn estimate_withdraw_fees(&self) -> Result<(solana_sdk::hash::Hash, u64), MmError<ClientError>> {
        let hash = async_blocking({
            let coin = self.clone();
            move || coin.rpc().get_latest_blockhash()
        })
        .await?;
        let to = self.key_pair.pubkey();

        let tx = solana_sdk::system_transaction::transfer(&self.key_pair, &to, LAMPORTS_DUMMY_AMOUNT, hash);
        let fees = async_blocking({
            let coin = self.clone();
            move || coin.rpc().get_fee_for_message(tx.message())
        })
        .await?;
        Ok((hash, fees))
    }

    pub async fn my_balance_spl(&self, infos: &SplTokenInfo) -> Result<CoinBalance, MmError<BalanceError>> {
        let token_accounts = async_blocking({
            let coin = self.clone();
            let infos = infos.clone();
            move || {
                coin.rpc().get_token_accounts_by_owner(
                    &coin.key_pair.pubkey(),
                    TokenAccountsFilter::Mint(infos.token_contract_address),
                )
            }
        })
        .await?;
        if token_accounts.is_empty() {
            return Ok(CoinBalance {
                spendable: Default::default(),
                unspendable: Default::default(),
            });
        }
        let actual_token_pubkey =
            Pubkey::from_str(&*token_accounts[0].pubkey).map_err(|e| BalanceError::Internal(format!("{:?}", e)))?;
        let amount = async_blocking({
            let coin = self.clone();
            move || coin.rpc().get_token_account_balance(&actual_token_pubkey)
        })
        .await?;
        let balance =
            BigDecimal::from_str(&*amount.ui_amount_string).map_to_mm(|e| BalanceError::Internal(e.to_string()))?;
        Ok(CoinBalance {
            spendable: balance,
            unspendable: Default::default(),
        })
    }

    fn my_balance_impl(&self) -> BalanceFut<BigDecimal> {
        let coin = self.clone();
        let fut = async_blocking(move || {
            // this is blocking IO
            let res = coin.rpc().get_balance(&coin.key_pair.pubkey())?;
            Ok(lamports_to_sol(res))
        });
        Box::new(fut.boxed().compat())
    }

    pub fn add_spl_token_info(&self, ticker: String, info: SplTokenInfo) {
        self.spl_tokens_infos.lock().unwrap().insert(ticker, info);
    }

    pub fn get_spl_tokens_infos(&self) -> HashMap<String, SplTokenInfo> {
        let guard = self.spl_tokens_infos.lock().unwrap();
        (*guard).clone()
    }
}

impl MarketCoinOps for SolanaCoin {
    fn ticker(&self) -> &str { &self.ticker }

    fn my_address(&self) -> Result<String, String> { Ok(self.my_address.clone()) }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> { unimplemented!() }

    fn sign_message_hash(&self, _message: &str) -> Option<[u8; 32]> { unimplemented!() }

    fn sign_message(&self, message: &str) -> SignatureResult<String> { solana_common::sign_message(self, message) }

    fn verify_message(&self, signature: &str, message: &str, pubkey_bs58: &str) -> VerificationResult<bool> {
        solana_common::verify_message(self, signature, message, pubkey_bs58)
    }

    fn my_balance(&self) -> BalanceFut<CoinBalance> {
        let decimals = self.decimals as u64;
        let fut = self.my_balance_impl().and_then(move |result| {
            Ok(CoinBalance {
                spendable: result.with_prec(decimals),
                unspendable: 0.into(),
            })
        });
        Box::new(fut)
    }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> {
        let decimals = self.decimals as u64;
        let fut = self
            .my_balance_impl()
            .and_then(move |result| Ok(result.with_prec(decimals)));
        Box::new(fut)
    }

    fn platform_ticker(&self) -> &str { self.ticker() }

    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
        let coin = self.clone();
        let tx = tx.to_owned();
        let fut = async_blocking(move || {
            let bytes = hex::decode(tx).map_to_mm(|e| e).map_err(|e| format!("{:?}", e))?;
            let tx: Transaction = deserialize(bytes.as_slice())
                .map_to_mm(|e| e)
                .map_err(|e| format!("{:?}", e))?;
            // this is blocking IO
            let signature = coin.rpc().send_transaction(&tx).map_err(|e| format!("{:?}", e))?;
            Ok(signature.to_string())
        });
        Box::new(fut.boxed().compat())
    }

    fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> {
        let coin = self.clone();
        let tx = tx.to_owned();
        let fut = async_blocking(move || {
            let tx = try_s!(deserialize(tx.as_slice()));
            // this is blocking IO
            let signature = coin.rpc().send_transaction(&tx).map_err(|e| format!("{:?}", e))?;
            Ok(signature.to_string())
        });
        Box::new(fut.boxed().compat())
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

    fn tx_enum_from_bytes(&self, _bytes: &[u8]) -> Result<TransactionEnum, String> { unimplemented!() }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        let coin = self.clone();
        let fut = async_blocking(move || coin.rpc().get_block_height().map_err(|e| format!("{:?}", e)));
        Box::new(fut.boxed().compat())
    }

    fn display_priv_key(&self) -> Result<String, String> { Ok(self.key_pair.secret().to_bytes()[..].to_base58()) }

    fn min_tx_amount(&self) -> BigDecimal { BigDecimal::from(0) }

    fn min_trading_vol(&self) -> MmNumber { MmNumber::from("0.00777") }
}

#[allow(clippy::forget_ref, clippy::forget_copy, clippy::cast_ref_to_mut)]
#[async_trait]
impl SwapOps for SolanaCoin {
    fn send_taker_fee(&self, _fee_addr: &[u8], amount: BigDecimal, _uuid: &[u8]) -> TransactionFut { unimplemented!() }

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
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
        _swap_unique_data: &[u8],
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
        _swap_unique_data: &[u8],
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
        _swap_unique_data: &[u8],
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
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!()
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
        my_pub: &[u8],
        other_pub: &[u8],
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
impl MmCoin for SolanaCoin {
    fn is_asset_chain(&self) -> bool { false }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        Box::new(Box::pin(withdraw_impl(self.clone(), req)).compat())
    }

    fn get_raw_transaction(&self, _req: RawTransactionRequest) -> RawTransactionFut { unimplemented!() }

    fn decimals(&self) -> u8 { self.decimals }

    fn convert_to_address(&self, _from: &str, _to_address_format: Json) -> Result<String, String> { unimplemented!() }

    fn validate_address(&self, address: &str) -> ValidateAddressResult {
        if address.len() != 44 {
            return ValidateAddressResult {
                is_valid: false,
                reason: Some("Invalid address length".to_string()),
            };
        }
        let result = Pubkey::try_from(address);
        match result {
            Ok(pubkey) => {
                if pubkey.is_on_curve() {
                    ValidateAddressResult {
                        is_valid: true,
                        reason: None,
                    }
                } else {
                    ValidateAddressResult {
                        is_valid: false,
                        reason: Some("not_on_curve".to_string()),
                    }
                }
            },
            Err(err) => ValidateAddressResult {
                is_valid: false,
                reason: Some(format!("{:?}", err)),
            },
        }
    }

    fn process_history_loop(&self, _ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> { unimplemented!() }

    fn history_sync_status(&self) -> HistorySyncState { unimplemented!() }

    /// Get fee to be paid per 1 swap transaction
    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> { unimplemented!() }

    async fn get_sender_trade_fee(
        &self,
        value: TradePreimageValue,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        unimplemented!()
    }

    fn get_receiver_trade_fee(&self, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> { unimplemented!() }

    async fn get_fee_to_send_taker_fee(
        &self,
        dex_fee_amount: BigDecimal,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        unimplemented!()
    }

    fn required_confirmations(&self) -> u64 { 1 }

    fn requires_notarization(&self) -> bool { false }

    fn set_required_confirmations(&self, _confirmations: u64) { unimplemented!() }

    fn set_requires_notarization(&self, _requires_nota: bool) { unimplemented!() }

    fn swap_contract_address(&self) -> Option<BytesJson> { unimplemented!() }

    fn mature_confirmations(&self) -> Option<u32> { None }

    fn coin_protocol_info(&self) -> Vec<u8> { Vec::new() }

    fn is_coin_protocol_supported(&self, _info: &Option<Vec<u8>>) -> bool { true }
}
