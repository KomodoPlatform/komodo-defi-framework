use std::{collections::HashMap,
          convert::{TryFrom, TryInto},
          fmt::Debug,
          ops::Deref,
          str::FromStr,
          sync::{Arc, Mutex}};

use async_trait::async_trait;
use base58::ToBase58;
use bincode::{deserialize, serialize};
use bitcrypto::sha256;
use common::{async_blocking,
             executor::{abortable_queue::AbortableQueue, AbortableSystem, AbortedError},
             log::error,
             now_sec};
use crypto::HDPathToCoin;
use derive_more::Display;
use futures::{compat::Future01CompatExt,
              {FutureExt, TryFutureExt}};
use futures01::Future;
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::{BigDecimal, MmNumber};
use num_traits::ToPrimitive;
use rpc::v1::types::Bytes as BytesJson;
pub use satomic_swap::{instruction::AtomicSwapInstruction, STORAGE_SPACE_ALLOCATED};
use serde_json::{self as json, Value as Json};
use solana_client::{client_error::{ClientError, ClientErrorKind},
                    rpc_client::RpcClient,
                    rpc_request::TokenAccountsFilter};
pub use solana_sdk::transaction::Transaction as SolTransaction;
use solana_sdk::{commitment_config::{CommitmentConfig, CommitmentLevel},
                 instruction::{AccountMeta, Instruction},
                 native_token::sol_to_lamports,
                 program_error::ProgramError,
                 pubkey::{ParsePubkeyError, Pubkey},
                 signature::{Keypair as SolKeypair, Signer}};
use spl_token::solana_program;

use super::{CoinBalance, HistorySyncState, MarketCoinOps, MmCoin, SwapOps, TradeFee, Transaction, TransactionEnum,
            TransactionErr, WatcherOps};
use crate::coin_errors::{MyAddressError, ValidatePaymentResult};
use crate::hd_wallet::HDPathAccountToAddressId;
use crate::solana::{solana_common::{lamports_to_sol, PrepareTransferData, SufficientBalanceError},
                    spl::SplTokenInfo};
use crate::{BalanceError, BalanceFut, CheckIfMyPaymentSentArgs, CoinFutSpawner, ConfirmPaymentInput, DexFee,
            FeeApproxStage, FoundSwapTxSpend, MakerSwapTakerCoin, MmCoinEnum, NegotiateSwapContractAddrErr,
            PaymentInstructionArgs, PaymentInstructions, PaymentInstructionsErr, PrivKeyBuildPolicy,
            PrivKeyPolicyNotAllowed, RawTransactionError, RawTransactionFut, RawTransactionRequest,
            RawTransactionResult, RefundError, RefundPaymentArgs, RefundResult, SearchForSwapTxSpendInput,
            SendMakerPaymentSpendPreimageInput, SendPaymentArgs, SignRawTransactionRequest, SignatureResult,
            SpendPaymentArgs, TakerSwapMakerCoin, TradePreimageFut, TradePreimageResult, TradePreimageValue,
            TransactionData, TransactionDetails, TransactionFut, TransactionResult, TransactionType, TxMarshalingErr,
            UnexpectedDerivationMethod, ValidateAddressResult, ValidateFeeArgs, ValidateInstructionsErr,
            ValidateOtherPubKeyErr, ValidatePaymentError, ValidatePaymentFut, ValidatePaymentInput,
            ValidateWatcherSpendInput, VerificationResult, WaitForHTLCTxSpendArgs, WatcherReward, WatcherRewardError,
            WatcherSearchForSwapTxSpendInput, WatcherValidatePaymentInput, WatcherValidateTakerFeeInput,
            WithdrawError, WithdrawFut, WithdrawRequest, WithdrawResult};

pub mod solana_common;
mod solana_decode_tx_helpers;
pub mod spl;

#[cfg(test)] mod solana_common_tests;
#[cfg(test)] mod solana_tests;
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
    #[serde(default)]
    path_to_address: HDPathAccountToAddressId,
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

fn generate_keypair_from_slice(priv_key: &[u8]) -> Result<SolKeypair, MmError<KeyPairCreationError>> {
    let secret_key = ed25519_dalek::SecretKey::from_bytes(priv_key)?;
    let public_key = ed25519_dalek::PublicKey::from(&secret_key);
    let key_pair = ed25519_dalek::Keypair {
        secret: secret_key,
        public: public_key,
    };
    solana_sdk::signature::keypair_from_seed(key_pair.to_bytes().as_ref())
        .map_to_mm(|e| KeyPairCreationError::KeyPairFromSeed(e.to_string()))
}

pub async fn solana_coin_with_policy(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    params: SolanaActivationParams,
    priv_key_policy: PrivKeyBuildPolicy,
) -> Result<SolanaCoin, String> {
    let client = RpcClient::new_with_commitment(params.client_url.clone(), CommitmentConfig {
        commitment: params.confirmation_commitment,
    });
    let decimals = conf["decimals"].as_u64().unwrap_or(SOLANA_DEFAULT_DECIMALS) as u8;

    let priv_key = match priv_key_policy {
        PrivKeyBuildPolicy::IguanaPrivKey(priv_key) => priv_key,
        PrivKeyBuildPolicy::GlobalHDAccount(global_hd) => {
            let path_to_coin: HDPathToCoin = try_s!(json::from_value(conf["derivation_path"].clone()));
            let derivation_path = try_s!(params.path_to_address.to_derivation_path(&path_to_coin));
            try_s!(global_hd.derive_secp256k1_secret(&derivation_path))
        },
        PrivKeyBuildPolicy::Trezor => return ERR!("{}", PrivKeyPolicyNotAllowed::HardwareWalletNotSupported),
    };

    let key_pair = try_s!(generate_keypair_from_slice(priv_key.as_slice()));
    let my_address = key_pair.pubkey().to_string();
    let spl_tokens_infos = Arc::new(Mutex::new(HashMap::new()));

    // Create an abortable system linked to the `MmCtx` so if the context is stopped via `MmArc::stop`,
    // all spawned futures related to `SolanaCoin` will be aborted as well.
    let abortable_system: AbortableQueue = try_s!(ctx.abortable_system.create_subsystem());

    let solana_coin = SolanaCoin(Arc::new(SolanaCoinImpl {
        my_address,
        key_pair,
        ticker: ticker.to_string(),
        client,
        decimals,
        spl_tokens_infos,
        abortable_system,
    }));
    Ok(solana_coin)
}

/// pImpl idiom.
pub struct SolanaCoinImpl {
    ticker: String,
    key_pair: SolKeypair,
    client: RpcClient,
    decimals: u8,
    my_address: String,
    spl_tokens_infos: Arc<Mutex<HashMap<String, SplTokenInfo>>>,
    /// This spawner is used to spawn coin's related futures that should be aborted on coin deactivation
    /// and on [`MmArc::stop`].
    pub abortable_system: AbortableQueue,
}

#[derive(Clone)]
pub struct SolanaCoin(Arc<SolanaCoinImpl>);
impl Deref for SolanaCoin {
    type Target = SolanaCoinImpl;
    fn deref(&self) -> &SolanaCoinImpl { &self.0 }
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
        tx: TransactionData::new_signed(serialized_tx.into(), tx.signatures[0].to_string()),
        from: vec![coin.my_address.clone()],
        to: vec![req.to],
        total_amount: spent_by_me.clone(),
        my_balance_change: &received_by_me - &spent_by_me,
        spent_by_me,
        received_by_me,
        block_height: 0,
        timestamp: now_sec(),
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
        memo: None,
    })
}

async fn withdraw_impl(coin: SolanaCoin, req: WithdrawRequest) -> WithdrawResult {
    let validate_address_result = coin.validate_address(&req.to);
    if !validate_address_result.is_valid {
        return MmError::err(WithdrawError::InvalidAddress(
            validate_address_result.reason.unwrap_or_else(|| "Unknown".to_string()),
        ));
    }
    withdraw_base_coin_impl(coin, req).await
}

type SolTxFut = Box<dyn Future<Item = SolTransaction, Error = TransactionErr> + Send + 'static>;

impl Transaction for SolTransaction {
    fn tx_hex(&self) -> Vec<u8> {
        serialize(self).unwrap_or_else(|e| {
            error!("Error serializing SolTransaction: {}", e);
            vec![]
        })
    }

    fn tx_hash_as_bytes(&self) -> BytesJson {
        let hash = match self.signatures.get(0) {
            Some(signature) => signature,
            None => {
                error!("No signature found in SolTransaction");
                return BytesJson(Vec::new());
            },
        };
        BytesJson(Vec::from(hash.as_ref()))
    }
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

    pub async fn my_balance_spl(&self, infos: SplTokenInfo) -> Result<CoinBalance, MmError<BalanceError>> {
        let token_accounts = async_blocking({
            let coin = self.clone();
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
            Pubkey::from_str(&token_accounts[0].pubkey).map_err(|e| BalanceError::Internal(format!("{:?}", e)))?;
        let amount = async_blocking({
            let coin = self.clone();
            move || coin.rpc().get_token_account_balance(&actual_token_pubkey)
        })
        .await?;
        let balance =
            BigDecimal::from_str(&amount.ui_amount_string).map_to_mm(|e| BalanceError::Internal(e.to_string()))?;
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

    /// WARNING
    /// Be very careful using this function since it returns dereferenced clone
    /// of value behind the MutexGuard and makes it non-thread-safe.
    pub fn get_spl_tokens_infos(&self) -> HashMap<String, SplTokenInfo> {
        let guard = self.spl_tokens_infos.lock().unwrap();
        (*guard).clone()
    }

    fn send_hash_time_locked_payment(&self, args: SendPaymentArgs<'_>) -> SolTxFut {
        let receiver = Pubkey::new(args.other_pubkey);
        let swap_program_id = Pubkey::new(try_tx_fus_opt!(
            args.swap_contract_address,
            format!(
                "Unable to extract Bytes from args.swap_contract_address ( {:?} )",
                args.swap_contract_address
            )
        ));
        let amount = sol_to_lamports(try_tx_fus_opt!(
            args.amount.to_f64(),
            format!("Unable to extract value from args.amount ( {:?} )", args.amount)
        ));
        let secret_hash: [u8; 32] = try_tx_fus!(<[u8; 32]>::try_from(args.secret_hash));
        let (vault_pda, vault_pda_data, vault_bump_seed, vault_bump_seed_data, rent_exemption_lamports) =
            try_tx_fus!(self.create_vaults(args.time_lock, secret_hash, swap_program_id, STORAGE_SPACE_ALLOCATED));
        let swap_instruction = AtomicSwapInstruction::LamportsPayment {
            secret_hash,
            lock_time: args.time_lock,
            amount,
            receiver,
            rent_exemption_lamports,
            vault_bump_seed,
            vault_bump_seed_data,
        };

        let accounts = vec![
            AccountMeta::new(self.key_pair.pubkey(), true),
            AccountMeta::new(vault_pda_data, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(solana_program::system_program::id(), false),
        ];
        self.sign_and_send_swap_transaction_fut(swap_program_id, accounts, swap_instruction.pack())
    }

    fn spend_hash_time_locked_payment(&self, args: SpendPaymentArgs) -> SolTxFut {
        let sender = Pubkey::new(args.other_pubkey);
        let swap_program_id = Pubkey::new(try_tx_fus_opt!(
            args.swap_contract_address.as_ref(),
            format!(
                "Unable to extract Bytes from args.swap_contract_address ( {:?} )",
                args.swap_contract_address
            )
        ));
        let secret: [u8; 32] = try_tx_fus!(<[u8; 32]>::try_from(args.secret));
        let secret_hash = sha256(secret.as_slice()).take();
        let (lock_time, tx_secret_hash, amount, token_program) =
            try_tx_fus!(self.get_swap_transaction_details(args.other_payment_tx));
        if secret_hash != tx_secret_hash {
            try_tx_fus_err!(format!(
                "Provided secret_hash {:?} does not match transaction secret_hash {:?}",
                secret_hash, tx_secret_hash
            ));
        }
        let (vault_pda, vault_pda_data, vault_bump_seed, vault_bump_seed_data, _rent_exemption_lamports) =
            try_tx_fus!(self.create_vaults(lock_time, secret_hash, swap_program_id, STORAGE_SPACE_ALLOCATED));
        let swap_instruction = AtomicSwapInstruction::ReceiverSpend {
            secret,
            lock_time,
            amount,
            sender,
            token_program,
            vault_bump_seed,
            vault_bump_seed_data,
        };
        let accounts = vec![
            AccountMeta::new(self.key_pair.pubkey(), true),
            AccountMeta::new(vault_pda_data, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(solana_program::system_program::id(), false),
        ];
        self.sign_and_send_swap_transaction_fut(swap_program_id, accounts, swap_instruction.pack())
    }

    fn refund_hash_time_locked_payment(&self, args: RefundPaymentArgs) -> SolTxFut {
        let receiver = Pubkey::new(args.other_pubkey);
        let swap_program_id = Pubkey::new(try_tx_fus_opt!(
            args.swap_contract_address.as_ref(),
            format!(
                "Unable to extract Bytes from args.swap_contract_address ( {:?} )",
                args.swap_contract_address
            )
        ));
        let (lock_time, secret_hash, amount, token_program) =
            try_tx_fus!(self.get_swap_transaction_details(args.payment_tx));
        let (vault_pda, vault_pda_data, vault_bump_seed, vault_bump_seed_data, _rent_exemption_lamports) =
            try_tx_fus!(self.create_vaults(lock_time, secret_hash, swap_program_id, STORAGE_SPACE_ALLOCATED));
        let swap_instruction = AtomicSwapInstruction::SenderRefund {
            secret_hash,
            lock_time,
            amount,
            receiver,
            token_program,
            vault_bump_seed,
            vault_bump_seed_data,
        };
        let accounts = vec![
            AccountMeta::new(self.key_pair.pubkey(), true), // Marked as signer
            AccountMeta::new(vault_pda_data, false),        // Not a signer
            AccountMeta::new(vault_pda, false),             // Not a signer
            AccountMeta::new(solana_program::system_program::id(), false), //system_program must be included
        ];
        self.sign_and_send_swap_transaction_fut(swap_program_id, accounts, swap_instruction.pack())
    }

    fn get_swap_transaction_details(&self, tx_hex: &[u8]) -> Result<(u64, [u8; 32], u64, Pubkey), Box<TransactionErr>> {
        let transaction: SolTransaction = deserialize(tx_hex)
            .map_err(|e| Box::new(TransactionErr::Plain(ERRL!("error deserializing tx_hex: {:?}", e))))?;

        let instruction = transaction
            .message
            .instructions
            .get(0)
            .ok_or_else(|| Box::new(TransactionErr::Plain(ERRL!("Instruction not found in message"))))?;

        let instruction_data = &instruction.data[..];
        let instruction = AtomicSwapInstruction::unpack(instruction_data)
            .map_err(|e| Box::new(TransactionErr::Plain(ERRL!("error unpacking tx data: {:?}", e))))?;

        match instruction {
            AtomicSwapInstruction::LamportsPayment {
                secret_hash,
                lock_time,
                amount,
                ..
            } => Ok((lock_time, secret_hash, amount, Pubkey::new_from_array([0; 32]))),
            AtomicSwapInstruction::SPLTokenPayment {
                secret_hash,
                lock_time,
                amount,
                token_program,
                ..
            } => Ok((lock_time, secret_hash, amount, token_program)),
            AtomicSwapInstruction::ReceiverSpend {
                secret,
                lock_time,
                amount,
                token_program,
                ..
            } => Ok((lock_time, sha256(&secret).take(), amount, token_program)),
            AtomicSwapInstruction::SenderRefund {
                secret_hash,
                lock_time,
                amount,
                token_program,
                ..
            } => Ok((lock_time, secret_hash, amount, token_program)),
        }
    }

    fn sign_and_send_swap_transaction_fut(
        &self,
        program_id: Pubkey,
        accounts: Vec<AccountMeta>,
        data: Vec<u8>,
    ) -> SolTxFut {
        let coin = self.clone();
        Box::new(
            async move { coin.sign_and_send_swap_transaction(program_id, accounts, data).await }
                .boxed()
                .compat(),
        )
    }

    pub async fn sign_and_send_swap_transaction(
        &self,
        program_id: Pubkey,
        accounts: Vec<AccountMeta>,
        data: Vec<u8>,
    ) -> Result<SolTransaction, TransactionErr> {
        // Construct the instruction to send to the program
        // The parameters here depend on your specific program's requirements
        let instruction = Instruction {
            program_id,
            accounts, // Specify account metas here
            data,     // Pass data to the program here
        };

        // Create a transaction
        let recent_blockhash = self
            .client
            .get_latest_blockhash()
            .map_err(|e| TransactionErr::Plain(format!("Failed to get recent blockhash: {:?}", e)))?;

        let transaction: SolTransaction = SolTransaction::new_signed_with_payer(
            &[instruction],
            Some(&self.key_pair.pubkey()), //payer pubkey
            &[&self.key_pair],             //payer
            recent_blockhash,
        );

        // Send the transaction
        let tx = self
            .client
            .send_and_confirm_transaction(&transaction)
            .map(|_signature| transaction)
            .map_err(|e| TransactionErr::Plain(ERRL!("Solana ClientError: {:?}", e)))?;

        Ok(tx)
    }

    fn create_vaults(
        &self,
        lock_time: u64,
        secret_hash: [u8; 32],
        program_id: Pubkey,
        space: u64,
    ) -> Result<(Pubkey, Pubkey, u8, u8, u64), Box<TransactionErr>> {
        let seeds: &[&[u8]] = &[b"swap", &lock_time.to_le_bytes()[..], &secret_hash[..]];
        let (vault_pda, bump_seed) = Pubkey::find_program_address(seeds, &program_id);

        let seeds_data: &[&[u8]] = &[b"swap_data", &lock_time.to_le_bytes()[..], &secret_hash[..]];
        let (vault_pda_data, bump_seed_data) = Pubkey::find_program_address(seeds_data, &program_id);

        let rent_exemption_lamports = self
            .client
            .get_minimum_balance_for_rent_exemption(
                space
                    .try_into()
                    .map_err(|e| Box::new(TransactionErr::Plain(ERRL!("unable to convert space: {:?}", e))))?,
            )
            .map_err(|e| {
                Box::new(TransactionErr::Plain(ERRL!(
                    "error get_minimum_balance_for_rent_exemption: {:?}",
                    e
                )))
            })?;

        Ok((
            vault_pda,
            vault_pda_data,
            bump_seed,
            bump_seed_data,
            rent_exemption_lamports,
        ))
    }
}

#[async_trait]
impl MarketCoinOps for SolanaCoin {
    fn ticker(&self) -> &str { &self.ticker }

    fn my_address(&self) -> MmResult<String, MyAddressError> { Ok(self.my_address.clone()) }

    async fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> {
        Ok(self.key_pair.pubkey().to_string())
    }

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
            let tx: SolTransaction = deserialize(bytes.as_slice())
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

    #[inline(always)]
    async fn sign_raw_tx(&self, _args: &SignRawTransactionRequest) -> RawTransactionResult {
        MmError::err(RawTransactionError::NotImplemented {
            coin: self.ticker().to_string(),
        })
    }

    fn wait_for_confirmations(&self, _input: ConfirmPaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn wait_for_htlc_tx_spend(&self, args: WaitForHTLCTxSpendArgs<'_>) -> TransactionFut { unimplemented!() }

    fn tx_enum_from_bytes(&self, _bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        MmError::err(TxMarshalingErr::NotSupported(
            "tx_enum_from_bytes is not supported for Solana yet.".to_string(),
        ))
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        let coin = self.clone();
        let fut = async_blocking(move || coin.rpc().get_block_height().map_err(|e| format!("{:?}", e)));
        Box::new(fut.boxed().compat())
    }

    fn display_priv_key(&self) -> Result<String, String> { Ok(self.key_pair.secret().to_bytes()[..].to_base58()) }

    fn min_tx_amount(&self) -> BigDecimal { BigDecimal::from(0) }

    fn min_trading_vol(&self) -> MmNumber { MmNumber::from("0.00777") }

    fn is_trezor(&self) -> bool { unimplemented!() }
}

#[async_trait]
impl SwapOps for SolanaCoin {
    fn send_taker_fee(&self, _fee_addr: &[u8], dex_fee: DexFee, _uuid: &[u8], _expire_at: u64) -> TransactionFut {
        unimplemented!()
    }

    fn send_maker_payment(&self, maker_payment: SendPaymentArgs) -> TransactionFut {
        Box::new(
            self.send_hash_time_locked_payment(maker_payment)
                .map(TransactionEnum::from),
        )
    }

    fn send_taker_payment(&self, taker_payment: SendPaymentArgs) -> TransactionFut {
        Box::new(
            self.send_hash_time_locked_payment(taker_payment)
                .map(TransactionEnum::from),
        )
    }

    async fn send_maker_spends_taker_payment(
        &self,
        maker_spends_payment_args: SpendPaymentArgs<'_>,
    ) -> TransactionResult {
        self.spend_hash_time_locked_payment(maker_spends_payment_args)
            .compat()
            .await
            .map(TransactionEnum::from)
    }

    async fn send_taker_spends_maker_payment(
        &self,
        taker_spends_payment_args: SpendPaymentArgs<'_>,
    ) -> TransactionResult {
        self.spend_hash_time_locked_payment(taker_spends_payment_args)
            .compat()
            .await
            .map(TransactionEnum::from)
    }

    async fn send_taker_refunds_payment(&self, taker_refunds_payment_args: RefundPaymentArgs<'_>) -> TransactionResult {
        self.refund_hash_time_locked_payment(taker_refunds_payment_args)
            .map(TransactionEnum::from)
            .compat()
            .await
    }

    async fn send_maker_refunds_payment(&self, maker_refunds_payment_args: RefundPaymentArgs<'_>) -> TransactionResult {
        self.refund_hash_time_locked_payment(maker_refunds_payment_args)
            .map(TransactionEnum::from)
            .compat()
            .await
    }

    fn validate_fee(&self, _validate_fee_args: ValidateFeeArgs) -> ValidatePaymentFut<()> { unimplemented!() }

    async fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentResult<()> {
        unimplemented!()
    }

    async fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentResult<()> {
        unimplemented!()
    }

    fn check_if_my_payment_sent(
        &self,
        _if_my_payment_sent_args: CheckIfMyPaymentSentArgs,
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

    fn check_tx_signed_by_pub(&self, tx: &[u8], expected_pub: &[u8]) -> Result<bool, MmError<ValidatePaymentError>> {
        unimplemented!();
    }

    async fn extract_secret(
        &self,
        secret_hash: &[u8],
        spend_tx: &[u8],
        watcher_reward: bool,
    ) -> Result<Vec<u8>, String> {
        unimplemented!()
    }

    fn is_auto_refundable(&self) -> bool { false }

    async fn wait_for_htlc_refund(&self, _tx: &[u8], _locktime: u64) -> RefundResult<()> {
        MmError::err(RefundError::Internal(
            "wait_for_htlc_refund is not supported for this coin!".into(),
        ))
    }

    fn negotiate_swap_contract_addr(
        &self,
        _other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        unimplemented!()
    }

    #[inline]
    fn derive_htlc_key_pair(&self, _swap_unique_data: &[u8]) -> KeyPair { todo!() }

    #[inline]
    fn derive_htlc_pubkey(&self, swap_unique_data: &[u8]) -> Vec<u8> {
        self.derive_htlc_key_pair(swap_unique_data).public_slice().to_vec()
    }

    fn validate_other_pubkey(&self, _raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> { unimplemented!() }

    async fn maker_payment_instructions(
        &self,
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        unimplemented!()
    }

    async fn taker_payment_instructions(
        &self,
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        unimplemented!()
    }

    fn validate_maker_payment_instructions(
        &self,
        _instructions: &[u8],
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        unimplemented!()
    }

    fn validate_taker_payment_instructions(
        &self,
        _instructions: &[u8],
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        unimplemented!()
    }
}

#[async_trait]
impl TakerSwapMakerCoin for SolanaCoin {
    async fn on_taker_payment_refund_start(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_taker_payment_refund_success(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
impl MakerSwapTakerCoin for SolanaCoin {
    async fn on_maker_payment_refund_start(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_maker_payment_refund_success(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
impl WatcherOps for SolanaCoin {
    fn create_maker_payment_spend_preimage(
        &self,
        _maker_payment_tx: &[u8],
        _time_lock: u64,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_maker_payment_spend_preimage(&self, _input: SendMakerPaymentSpendPreimageInput) -> TransactionFut {
        unimplemented!();
    }

    fn create_taker_payment_refund_preimage(
        &self,
        _taker_payment_tx: &[u8],
        _time_lock: u64,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_taker_payment_refund_preimage(&self, _watcher_refunds_payment_args: RefundPaymentArgs) -> TransactionFut {
        unimplemented!();
    }

    fn watcher_validate_taker_fee(&self, input: WatcherValidateTakerFeeInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    fn watcher_validate_taker_payment(&self, _input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    fn taker_validates_payment_spend_or_refund(&self, _input: ValidateWatcherSpendInput) -> ValidatePaymentFut<()> {
        unimplemented!()
    }

    async fn watcher_search_for_swap_tx_spend(
        &self,
        input: WatcherSearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!();
    }

    async fn get_taker_watcher_reward(
        &self,
        other_coin: &MmCoinEnum,
        coin_amount: Option<BigDecimal>,
        other_coin_amount: Option<BigDecimal>,
        reward_amount: Option<BigDecimal>,
        wait_until: u64,
    ) -> Result<WatcherReward, MmError<WatcherRewardError>> {
        unimplemented!();
    }

    async fn get_maker_watcher_reward(
        &self,
        other_coin: &MmCoinEnum,
        reward_amount: Option<BigDecimal>,
        wait_until: u64,
    ) -> Result<Option<WatcherReward>, MmError<WatcherRewardError>> {
        unimplemented!();
    }
}

#[async_trait]
impl MmCoin for SolanaCoin {
    fn is_asset_chain(&self) -> bool { false }

    fn spawner(&self) -> CoinFutSpawner { CoinFutSpawner::new(&self.abortable_system) }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        Box::new(Box::pin(withdraw_impl(self.clone(), req)).compat())
    }

    fn get_raw_transaction(&self, _req: RawTransactionRequest) -> RawTransactionFut { unimplemented!() }

    fn get_tx_hex_by_hash(&self, tx_hash: Vec<u8>) -> RawTransactionFut { unimplemented!() }

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
        _value: TradePreimageValue,
        _stage: FeeApproxStage,
        _include_refund_fee: bool,
    ) -> TradePreimageResult<TradeFee> {
        unimplemented!()
    }

    fn get_receiver_trade_fee(&self, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> { unimplemented!() }

    async fn get_fee_to_send_taker_fee(
        &self,
        _dex_fee_amount: DexFee,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        unimplemented!()
    }

    fn required_confirmations(&self) -> u64 { 1 }

    fn requires_notarization(&self) -> bool { false }

    fn set_required_confirmations(&self, _confirmations: u64) { unimplemented!() }

    fn set_requires_notarization(&self, _requires_nota: bool) { unimplemented!() }

    fn swap_contract_address(&self) -> Option<BytesJson> { unimplemented!() }

    fn fallback_swap_contract(&self) -> Option<BytesJson> { unimplemented!() }

    fn mature_confirmations(&self) -> Option<u32> { None }

    fn coin_protocol_info(&self, _amount_to_receive: Option<MmNumber>) -> Vec<u8> { Vec::new() }

    fn is_coin_protocol_supported(
        &self,
        _info: &Option<Vec<u8>>,
        _amount_to_send: Option<MmNumber>,
        _locktime: u64,
        _is_maker: bool,
    ) -> bool {
        true
    }

    fn on_disabled(&self) -> Result<(), AbortedError> { AbortableSystem::abort_all(&self.abortable_system) }

    fn on_token_deactivated(&self, _ticker: &str) { unimplemented!() }
}
