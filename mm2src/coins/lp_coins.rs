/******************************************************************************
 * Copyright © 2023 Pampex LTD and TillyHK LTD              *
 *                                                                            *
 * See the CONTRIBUTOR-LICENSE-AGREEMENT, COPYING, LICENSE-COPYRIGHT-NOTICE   *
 * and DEVELOPER-CERTIFICATE-OF-ORIGIN files in the LEGAL directory in        *
 * the top-level directory of this distribution for the individual copyright  *
 * holder information and the developer policies on copyright and licensing.  *
 *                                                                            *
 * Unless otherwise agreed in a custom licensing agreement, no part of the    *
 * Komodo DeFi Framework software, including this file may be copied, modified, propagated*
 * or distributed except according to the terms contained in the              *
 * LICENSE-COPYRIGHT-NOTICE file.                                             *
 *                                                                            *
 * Removal or modification of this copyright notice is prohibited.            *
 *                                                                            *
 ******************************************************************************/
//
//  coins.rs
//  marketmaker
//

// `mockable` implementation uses these
#![allow(
    clippy::forget_ref,
    clippy::forget_copy,
    clippy::swap_ptr_to_ref,
    clippy::forget_non_drop
)]
#![allow(uncommon_codepoints)]
#![feature(integer_atomics)]
#![feature(async_closure)]
#![feature(hash_raw_entry)]
#![feature(stmt_expr_attributes)]
#![feature(result_flattening)]

#[macro_use] extern crate common;
#[macro_use] extern crate gstuff;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate mm2_metrics;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate ser_error_derive;

use async_trait::async_trait;
use base58::FromBase58Error;
use bip32::ExtendedPrivateKey;
use common::custom_futures::timeout::TimeoutError;
use common::executor::{abortable_queue::{AbortableQueue, WeakSpawner},
                       AbortSettings, AbortedError, SpawnAbortable, SpawnFuture};
use common::log::{warn, LogOnError};
use common::{calc_total_pages, now_sec, ten, HttpStatusCode};
use crypto::{derive_secp256k1_secret, Bip32Error, CryptoCtx, CryptoCtxError, DerivationPath, GlobalHDAccountArc,
             HwRpcError, KeyPairPolicy, Secp256k1Secret, StandardHDCoinAddress, StandardHDPathToCoin, WithHwRpcError};
use derive_more::Display;
use enum_from::{EnumFromStringify, EnumFromTrait};
use ethereum_types::H256;
use futures::compat::Future01CompatExt;
use futures::lock::Mutex as AsyncMutex;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use hex::FromHexError;
use http::{Response, StatusCode};
use keys::{AddressFormat as UtxoAddressFormat, KeyPair, NetworkPrefix as CashAddrPrefix};
use mm2_core::mm_ctx::{from_ctx, MmArc};
use mm2_err_handle::prelude::*;
use mm2_metrics::MetricsWeak;
use mm2_number::{bigdecimal::{BigDecimal, ParseBigDecimalError, Zero},
                 MmNumber};
use mm2_rpc::data::legacy::{EnabledCoin, GetEnabledResponse, Mm2RpcResult};
use parking_lot::Mutex as PaMutex;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{self as json, Value as Json};
use std::cmp::Ordering;
use std::collections::hash_map::{HashMap, RawEntryMut};
use std::collections::HashSet;
use std::fmt;
use std::future::Future as Future03;
use std::num::NonZeroUsize;
use std::ops::{Add, Deref};
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::Arc;
use std::time::Duration;
use utxo_signer::with_key_pair::UtxoSignWithKeyPairError;
use zcash_primitives::transaction::Transaction as ZTransaction;

cfg_native! {
    use crate::lightning::LightningCoin;
    use crate::lightning::ln_conf::PlatformCoinConfirmationTargets;
    use ::lightning::ln::PaymentHash as LightningPayment;
    use async_std::fs;
    use futures::AsyncWriteExt;
    use lightning_invoice::{Invoice, ParseOrSemanticError};
    use std::io;
    use std::path::PathBuf;
}

cfg_wasm32! {
    use ethereum_types::{H264 as EthH264, H520 as EthH520};
    use hd_wallet_storage::HDWalletDb;
    use mm2_db::indexed_db::{ConstructibleDb, DbLocked, SharedDb};
    use tx_history_storage::wasm::{clear_tx_history, load_tx_history, save_tx_history, TxHistoryDb};
    pub type TxHistoryDbLocked<'a> = DbLocked<'a, TxHistoryDb>;
}

// using custom copy of try_fus as futures crate was renamed to futures01
macro_rules! try_fus {
    ($e: expr) => {
        match $e {
            Ok(ok) => ok,
            Err(err) => return Box::new(futures01::future::err(ERRL!("{}", err))),
        }
    };
}

macro_rules! try_f {
    ($e: expr) => {
        match $e {
            Ok(ok) => ok,
            Err(e) => return Box::new(futures01::future::err(e.into())),
        }
    };
}

/// `TransactionErr` compatible `try_fus` macro.
macro_rules! try_tx_fus {
    ($e: expr) => {
        match $e {
            Ok(ok) => ok,
            Err(err) => return Box::new(futures01::future::err(crate::TransactionErr::Plain(ERRL!("{:?}", err)))),
        }
    };
    ($e: expr, $tx: expr) => {
        match $e {
            Ok(ok) => ok,
            Err(err) => {
                return Box::new(futures01::future::err(crate::TransactionErr::TxRecoverable(
                    TransactionEnum::from($tx),
                    ERRL!("{:?}", err),
                )))
            },
        }
    };
}

/// `TransactionErr` compatible `try_s` macro.
macro_rules! try_tx_s {
    ($e: expr) => {
        match $e {
            Ok(ok) => ok,
            Err(err) => {
                return Err(crate::TransactionErr::Plain(format!(
                    "{}:{}] {:?}",
                    file!(),
                    line!(),
                    err
                )))
            },
        }
    };
    ($e: expr, $tx: expr) => {
        match $e {
            Ok(ok) => ok,
            Err(err) => {
                return Err(crate::TransactionErr::TxRecoverable(
                    TransactionEnum::from($tx),
                    format!("{}:{}] {:?}", file!(), line!(), err),
                ))
            },
        }
    };
}

/// `TransactionErr:Plain` compatible `ERR` macro.
macro_rules! TX_PLAIN_ERR {
    ($format: expr, $($args: tt)+) => { Err(crate::TransactionErr::Plain((ERRL!($format, $($args)+)))) };
    ($format: expr) => { Err(crate::TransactionErr::Plain(ERRL!($format))) }
}

/// `TransactionErr:TxRecoverable` compatible `ERR` macro.
#[allow(unused_macros)]
macro_rules! TX_RECOVERABLE_ERR {
    ($tx: expr, $format: expr, $($args: tt)+) => {
        Err(crate::TransactionErr::TxRecoverable(TransactionEnum::from($tx), ERRL!($format, $($args)+)))
    };
    ($tx: expr, $format: expr) => {
        Err(crate::TransactionErr::TxRecoverable(TransactionEnum::from($tx), ERRL!($format)))
    };
}

macro_rules! ok_or_continue_after_sleep {
    ($e:expr, $delay: ident) => {
        match $e {
            Ok(res) => res,
            Err(e) => {
                error!("error {:?}", e);
                Timer::sleep($delay).await;
                continue;
            },
        }
    };
}

pub mod coin_balance;
pub mod lp_price;
pub mod watcher_common;

pub mod coin_errors;
use coin_errors::{MyAddressError, ValidatePaymentError, ValidatePaymentFut};

#[doc(hidden)]
#[cfg(test)]
pub mod coins_tests;

pub mod eth;
use eth::GetValidEthWithdrawAddError;
use eth::{eth_coin_from_conf_and_request, get_eth_address, EthCoin, EthGasDetailsErr, EthTxFeeDetails,
          GetEthAddressError, SignedEthTx};
use ethereum_types::U256;

pub mod hd_confirm_address;
pub mod hd_pubkey;

pub mod hd_wallet;
use hd_wallet::{HDAccountAddressId, HDAddress};

pub mod hd_wallet_storage;
#[cfg(not(target_arch = "wasm32"))] pub mod lightning;
#[cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]
pub mod my_tx_history_v2;

pub mod qrc20;
use qrc20::{qrc20_coin_with_policy, Qrc20ActivationParams, Qrc20Coin, Qrc20FeeDetails};

pub mod rpc_command;
use rpc_command::{get_new_address::{GetNewAddressTaskManager, GetNewAddressTaskManagerShared},
                  init_account_balance::{AccountBalanceTaskManager, AccountBalanceTaskManagerShared},
                  init_create_account::{CreateAccountTaskManager, CreateAccountTaskManagerShared},
                  init_scan_for_new_addresses::{ScanAddressesTaskManager, ScanAddressesTaskManagerShared},
                  init_withdraw::{WithdrawTaskManager, WithdrawTaskManagerShared}};

pub mod tendermint;
use tendermint::{CosmosTransaction, CustomTendermintMsgType, TendermintCoin, TendermintFeeDetails,
                 TendermintProtocolInfo, TendermintToken, TendermintTokenProtocolInfo};

#[doc(hidden)]
#[allow(unused_variables)]
pub mod test_coin;
pub use test_coin::TestCoin;

pub mod tx_history_storage;

#[doc(hidden)]
#[allow(unused_variables)]
#[cfg(all(
    feature = "enable-solana",
    not(target_os = "ios"),
    not(target_os = "android"),
    not(target_arch = "wasm32")
))]
pub mod solana;
#[cfg(all(
    feature = "enable-solana",
    not(target_os = "ios"),
    not(target_os = "android"),
    not(target_arch = "wasm32")
))]
pub use solana::spl::SplToken;
#[cfg(all(
    feature = "enable-solana",
    not(target_os = "ios"),
    not(target_os = "android"),
    not(target_arch = "wasm32")
))]
pub use solana::{SolanaActivationParams, SolanaCoin, SolanaFeeDetails};

pub mod utxo;
use utxo::bch::{bch_coin_with_policy, BchActivationRequest, BchCoin};
use utxo::qtum::{self, qtum_coin_with_policy, Qrc20AddressError, QtumCoin, QtumDelegationOps, QtumDelegationRequest,
                 QtumStakingInfosDetails, ScriptHashTypeNotSupported};
use utxo::rpc_clients::UtxoRpcError;
use utxo::slp::SlpToken;
use utxo::slp::{slp_addr_from_pubkey_str, SlpFeeDetails};
use utxo::utxo_common::big_decimal_from_sat_unsigned;
use utxo::utxo_standard::{utxo_standard_coin_with_policy, UtxoStandardCoin};
use utxo::UtxoActivationParams;
use utxo::{BlockchainNetwork, GenerateTxError, UtxoFeeDetails, UtxoTx};

pub mod nft;
use nft::nft_errors::GetNftInfoError;

pub mod z_coin;
use z_coin::{ZCoin, ZcoinProtocolInfo};

pub type TransactionFut = Box<dyn Future<Item = TransactionEnum, Error = TransactionErr> + Send>;
pub type TransactionResult = Result<TransactionEnum, TransactionErr>;
pub type BalanceResult<T> = Result<T, MmError<BalanceError>>;
pub type BalanceFut<T> = Box<dyn Future<Item = T, Error = MmError<BalanceError>> + Send>;
pub type NonZeroBalanceFut<T> = Box<dyn Future<Item = T, Error = MmError<GetNonZeroBalance>> + Send>;
pub type NumConversResult<T> = Result<T, MmError<NumConversError>>;
pub type StakingInfosResult = Result<StakingInfos, MmError<StakingInfosError>>;
pub type StakingInfosFut = Box<dyn Future<Item = StakingInfos, Error = MmError<StakingInfosError>> + Send>;
pub type DelegationResult = Result<TransactionDetails, MmError<DelegationError>>;
pub type DelegationFut = Box<dyn Future<Item = TransactionDetails, Error = MmError<DelegationError>> + Send>;
pub type WithdrawResult = Result<TransactionDetails, MmError<WithdrawError>>;
pub type WithdrawFut = Box<dyn Future<Item = TransactionDetails, Error = MmError<WithdrawError>> + Send>;
pub type TradePreimageResult<T> = Result<T, MmError<TradePreimageError>>;
pub type TradePreimageFut<T> = Box<dyn Future<Item = T, Error = MmError<TradePreimageError>> + Send>;
pub type CoinFindResult<T> = Result<T, MmError<CoinFindError>>;
pub type TxHistoryFut<T> = Box<dyn Future<Item = T, Error = MmError<TxHistoryError>> + Send>;
pub type TxHistoryResult<T> = Result<T, MmError<TxHistoryError>>;
pub type RawTransactionResult = Result<RawTransactionRes, MmError<RawTransactionError>>;
pub type RawTransactionFut<'a> =
    Box<dyn Future<Item = RawTransactionRes, Error = MmError<RawTransactionError>> + Send + 'a>;
pub type RefundResult<T> = Result<T, MmError<RefundError>>;
/// Helper type used for swap transactions' spend preimage generation result
pub type GenPreimageResult<Coin> = MmResult<TxPreimageWithSig<Coin>, TxGenError>;
/// Helper type used for taker funding's validation result
pub type ValidateTakerFundingResult = MmResult<(), ValidateTakerFundingError>;
/// Helper type used for taker funding's spend preimage validation result
pub type ValidateTakerFundingSpendPreimageResult = MmResult<(), ValidateTakerFundingSpendPreimageError>;
/// Helper type used for taker payment's spend preimage validation result
pub type ValidateTakerPaymentSpendPreimageResult = MmResult<(), ValidateTakerPaymentSpendPreimageError>;

pub type IguanaPrivKey = Secp256k1Secret;

// Constants for logs used in tests
pub const INVALID_SENDER_ERR_LOG: &str = "Invalid sender";
pub const EARLY_CONFIRMATION_ERR_LOG: &str = "Early confirmation";
pub const OLD_TRANSACTION_ERR_LOG: &str = "Old transaction";
pub const INVALID_RECEIVER_ERR_LOG: &str = "Invalid receiver";
pub const INVALID_CONTRACT_ADDRESS_ERR_LOG: &str = "Invalid contract address";
pub const INVALID_PAYMENT_STATE_ERR_LOG: &str = "Invalid payment state";
pub const INVALID_SWAP_ID_ERR_LOG: &str = "Invalid swap id";
pub const INVALID_SCRIPT_ERR_LOG: &str = "Invalid script";
pub const INVALID_REFUND_TX_ERR_LOG: &str = "Invalid refund transaction";

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum RawTransactionError {
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(fmt = "Invalid  hash: {}", _0)]
    InvalidHashError(String),
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Hash does not exist: {}", _0)]
    HashNotExist(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
    #[display(fmt = "Transaction decode error: {}", _0)]
    DecodeError(String),
    #[display(fmt = "Invalid param: {}", _0)]
    InvalidParam(String),
    #[display(fmt = "Non-existent previous output: {}", _0)]
    NonExistentPrevOutputError(String),
    #[display(fmt = "Signing error: {}", _0)]
    SigningError(String),
    #[display(fmt = "Not implemented for this coin {}", coin)]
    NotImplemented { coin: String },
    #[display(fmt = "Transaction error {}", _0)]
    TransactionError(String),
}

impl HttpStatusCode for RawTransactionError {
    fn status_code(&self) -> StatusCode {
        match self {
            RawTransactionError::Transport(_)
            | RawTransactionError::InternalError(_)
            | RawTransactionError::SigningError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            RawTransactionError::NoSuchCoin { .. }
            | RawTransactionError::InvalidHashError(_)
            | RawTransactionError::HashNotExist(_)
            | RawTransactionError::DecodeError(_)
            | RawTransactionError::InvalidParam(_)
            | RawTransactionError::NonExistentPrevOutputError(_)
            | RawTransactionError::TransactionError(_) => StatusCode::BAD_REQUEST,
            RawTransactionError::NotImplemented { .. } => StatusCode::NOT_IMPLEMENTED,
        }
    }
}

impl From<CoinFindError> for RawTransactionError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => RawTransactionError::NoSuchCoin { coin },
        }
    }
}

impl From<NumConversError> for RawTransactionError {
    fn from(e: NumConversError) -> Self { RawTransactionError::InvalidParam(e.to_string()) }
}

impl From<FromHexError> for RawTransactionError {
    fn from(e: FromHexError) -> Self { RawTransactionError::InvalidParam(e.to_string()) }
}

#[derive(Clone, Debug, Deserialize, Display, EnumFromStringify, PartialEq, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum GetMyAddressError {
    CoinsConfCheckError(String),
    CoinIsNotSupported(String),
    #[from_stringify("CryptoCtxError")]
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
    #[from_stringify("serde_json::Error")]
    #[display(fmt = "Invalid request error error: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Get Eth address error: {}", _0)]
    GetEthAddressError(GetEthAddressError),
}

impl From<GetEthAddressError> for GetMyAddressError {
    fn from(e: GetEthAddressError) -> Self { GetMyAddressError::GetEthAddressError(e) }
}

impl HttpStatusCode for GetMyAddressError {
    fn status_code(&self) -> StatusCode {
        match self {
            GetMyAddressError::CoinsConfCheckError(_)
            | GetMyAddressError::CoinIsNotSupported(_)
            | GetMyAddressError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            GetMyAddressError::Internal(_) | GetMyAddressError::GetEthAddressError(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
        }
    }
}

#[derive(Deserialize)]
pub struct RawTransactionRequest {
    pub coin: String,
    pub tx_hash: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RawTransactionRes {
    /// Raw bytes of signed transaction in hexadecimal string, this should be return hexadecimal encoded signed transaction for get_raw_transaction
    pub tx_hex: BytesJson,
}

/// Previous utxo transaction data for signing
#[derive(Clone, Debug, Deserialize)]
pub struct PrevTxns {
    /// transaction hash
    tx_hash: String,
    /// transaction output index
    index: u32,
    /// transaction output script pub key
    script_pub_key: String,
    // TODO: implement if needed:
    // redeem script for P2SH script pubkey
    // pub redeem_script: Option<String>,
    /// transaction output amount
    amount: BigDecimal,
}

/// sign_raw_transaction RPC request's params for signing raw utxo transactions
#[derive(Clone, Debug, Deserialize)]
pub struct SignUtxoTransactionParams {
    /// unsigned utxo transaction in hex
    tx_hex: String,
    /// optional data of previous transactions referred by unsigned transaction inputs
    prev_txns: Option<Vec<PrevTxns>>,
    // TODO: add if needed for utxo:
    // pub sighash_type: Option<String>, optional signature hash type, one of values: NONE, SINGLE, ALL, NONE|ANYONECANPAY, SINGLE|ANYONECANPAY, ALL|ANYONECANPAY (if not set 'ALL' is used)
    // pub branch_id: Option<u32>, zcash or komodo optional consensus branch id, used for signing transactions ahead of current height
}

/// sign_raw_transaction RPC request's params for signing raw eth transactions
#[derive(Clone, Debug, Deserialize)]
pub struct SignEthTransactionParams {
    /// Eth transfer value
    value: Option<BigDecimal>,
    /// Eth to address
    to: Option<String>,
    /// Eth contract data
    data: Option<String>,
    /// Eth gas use limit
    gas_limit: U256,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", content = "tx")]
pub enum SignRawTransactionEnum {
    UTXO(SignUtxoTransactionParams),
    ETH(SignEthTransactionParams),
}

/// sign_raw_transaction RPC request
#[derive(Clone, Debug, Deserialize)]
pub struct SignRawTransactionRequest {
    coin: String,
    #[serde(flatten)]
    tx: SignRawTransactionEnum,
}

#[derive(Debug, Deserialize)]
pub struct MyAddressReq {
    coin: String,
    #[serde(default)]
    path_to_address: StandardHDCoinAddress,
}

#[derive(Debug, Serialize)]
pub struct MyWalletAddress {
    coin: String,
    wallet_address: String,
}

pub type SignatureResult<T> = Result<T, MmError<SignatureError>>;
pub type VerificationResult<T> = Result<T, MmError<VerificationError>>;

#[derive(Debug, Display)]
pub enum TxHistoryError {
    ErrorSerializing(String),
    ErrorDeserializing(String),
    ErrorSaving(String),
    ErrorLoading(String),
    ErrorClearing(String),
    #[display(fmt = "'internal_id' not found: {:?}", internal_id)]
    FromIdNotFound {
        internal_id: BytesJson,
    },
    NotSupported(String),
    InternalError(String),
}

#[derive(Clone, Debug, Deserialize, Display, PartialEq)]
pub enum PrivKeyPolicyNotAllowed {
    #[display(fmt = "Hardware Wallet is not supported")]
    HardwareWalletNotSupported,
    #[display(fmt = "Unsupported method: {}", _0)]
    UnsupportedMethod(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

impl Serialize for PrivKeyPolicyNotAllowed {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Clone, Debug, Display, PartialEq, Serialize)]
pub enum UnexpectedDerivationMethod {
    #[display(fmt = "Expected 'SingleAddress' derivation method")]
    ExpectedSingleAddress,
    #[display(fmt = "Expected 'HDWallet' derivationMethod")]
    ExpectedHDWallet,
    #[display(fmt = "Trezor derivation method is not supported yet!")]
    Trezor,
    #[display(fmt = "Unsupported error: {}", _0)]
    UnsupportedError(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

impl From<PrivKeyPolicyNotAllowed> for UnexpectedDerivationMethod {
    fn from(e: PrivKeyPolicyNotAllowed) -> Self {
        match e {
            PrivKeyPolicyNotAllowed::HardwareWalletNotSupported => UnexpectedDerivationMethod::Trezor,
            PrivKeyPolicyNotAllowed::UnsupportedMethod(method) => UnexpectedDerivationMethod::UnsupportedError(method),
            PrivKeyPolicyNotAllowed::InternalError(e) => UnexpectedDerivationMethod::InternalError(e),
        }
    }
}

pub trait Transaction: fmt::Debug + 'static {
    /// Raw transaction bytes of the transaction
    fn tx_hex(&self) -> Vec<u8>;
    /// Serializable representation of tx hash for displaying purpose
    fn tx_hash(&self) -> BytesJson;
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransactionEnum {
    UtxoTx(UtxoTx),
    SignedEthTx(SignedEthTx),
    ZTransaction(ZTransaction),
    CosmosTransaction(CosmosTransaction),
    #[cfg(not(target_arch = "wasm32"))]
    LightningPayment(LightningPayment),
}

ifrom!(TransactionEnum, UtxoTx);
ifrom!(TransactionEnum, SignedEthTx);
ifrom!(TransactionEnum, ZTransaction);
#[cfg(not(target_arch = "wasm32"))]
ifrom!(TransactionEnum, LightningPayment);

impl TransactionEnum {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn supports_tx_helper(&self) -> bool { !matches!(self, TransactionEnum::LightningPayment(_)) }

    #[cfg(target_arch = "wasm32")]
    pub fn supports_tx_helper(&self) -> bool { true }
}

// NB: When stable and groked by IDEs, `enum_dispatch` can be used instead of `Deref` to speed things up.
impl Deref for TransactionEnum {
    type Target = dyn Transaction;
    fn deref(&self) -> &dyn Transaction {
        match self {
            TransactionEnum::UtxoTx(ref t) => t,
            TransactionEnum::SignedEthTx(ref t) => t,
            TransactionEnum::ZTransaction(ref t) => t,
            TransactionEnum::CosmosTransaction(ref t) => t,
            #[cfg(not(target_arch = "wasm32"))]
            TransactionEnum::LightningPayment(ref p) => p,
        }
    }
}

/// Error type for handling tx serialization/deserialization operations.
#[derive(Debug, Clone)]
pub enum TxMarshalingErr {
    InvalidInput(String),
    /// For cases where serialized and deserialized values doesn't verify each other.
    CrossCheckFailed(String),
    NotSupported(String),
    Internal(String),
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum TransactionErr {
    /// Keeps transactions while throwing errors.
    TxRecoverable(TransactionEnum, String),
    /// Simply for plain error messages.
    Plain(String),
}

impl TransactionErr {
    /// Returns transaction if the error includes it.
    #[inline]
    pub fn get_tx(&self) -> Option<TransactionEnum> {
        match self {
            TransactionErr::TxRecoverable(tx, _) => Some(tx.clone()),
            _ => None,
        }
    }

    #[inline]
    /// Returns plain text part of error.
    pub fn get_plain_text_format(&self) -> String {
        match self {
            TransactionErr::TxRecoverable(_, err) => err.to_string(),
            TransactionErr::Plain(err) => err.to_string(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum FoundSwapTxSpend {
    Spent(TransactionEnum),
    Refunded(TransactionEnum),
}

pub enum CanRefundHtlc {
    CanRefundNow,
    // returns the number of seconds to sleep before HTLC becomes refundable
    HaveToWait(u64),
}

#[derive(Debug, Display, Eq, PartialEq)]
pub enum NegotiateSwapContractAddrErr {
    #[display(fmt = "InvalidOtherAddrLen, addr supplied {:?}", _0)]
    InvalidOtherAddrLen(BytesJson),
    #[display(fmt = "UnexpectedOtherAddr, addr supplied {:?}", _0)]
    UnexpectedOtherAddr(BytesJson),
    NoOtherAddrAndNoFallback,
}

#[derive(Debug, Display, Eq, PartialEq)]
pub enum ValidateOtherPubKeyErr {
    #[display(fmt = "InvalidPubKey: {:?}", _0)]
    InvalidPubKey(String),
}

#[derive(Clone, Debug)]
pub struct ConfirmPaymentInput {
    pub payment_tx: Vec<u8>,
    pub confirmations: u64,
    pub requires_nota: bool,
    pub wait_until: u64,
    pub check_every: u64,
}

#[derive(Clone, Debug)]
pub struct WatcherValidateTakerFeeInput {
    pub taker_fee_hash: Vec<u8>,
    pub sender_pubkey: Vec<u8>,
    pub min_block_number: u64,
    pub fee_addr: Vec<u8>,
    pub lock_duration: u64,
}

/// Helper struct wrapping arguments for [WatcherOps::watcher_validate_taker_payment].
#[derive(Clone)]
pub struct WatcherValidatePaymentInput {
    /// Taker payment serialized to raw bytes.
    pub payment_tx: Vec<u8>,
    /// Payment refund preimage generated by taker.
    pub taker_payment_refund_preimage: Vec<u8>,
    /// Taker payment can be refunded after this timestamp.
    pub time_lock: u64,
    /// Taker's pubkey.
    pub taker_pub: Vec<u8>,
    /// Maker's pubkey.
    pub maker_pub: Vec<u8>,
    /// Hash of the secret generated by maker.
    pub secret_hash: Vec<u8>,
    /// Validation timeout.
    pub wait_until: u64,
    /// Required number of taker payment's on-chain confirmations.
    pub confirmations: u64,
    /// Maker coin.
    pub maker_coin: MmCoinEnum,
}

#[derive(Clone)]
pub enum WatcherSpendType {
    TakerPaymentRefund,
    MakerPaymentSpend,
}

#[derive(Clone)]
pub struct ValidateWatcherSpendInput {
    pub payment_tx: Vec<u8>,
    pub maker_pub: Vec<u8>,
    pub swap_contract_address: Option<BytesJson>,
    pub time_lock: u64,
    pub secret_hash: Vec<u8>,
    pub amount: BigDecimal,
    pub watcher_reward: Option<WatcherReward>,
    pub spend_type: WatcherSpendType,
}

/// Helper struct wrapping arguments for [SwapOps::validate_taker_payment] and [SwapOps::validate_maker_payment].
#[derive(Clone, Debug)]
pub struct ValidatePaymentInput {
    /// Payment transaction serialized to raw bytes.
    pub payment_tx: Vec<u8>,
    /// Time lock duration in seconds.
    pub time_lock_duration: u64,
    /// Payment can be refunded after this timestamp.
    pub time_lock: u64,
    /// Pubkey of other side of the swap.
    pub other_pub: Vec<u8>,
    /// Hash of the secret generated by maker.
    pub secret_hash: Vec<u8>,
    /// Expected payment amount.
    pub amount: BigDecimal,
    /// Swap contract address if applicable.
    pub swap_contract_address: Option<BytesJson>,
    /// SPV proof check timeout.
    pub try_spv_proof_until: u64,
    /// Required number of payment's on-chain confirmations.
    pub confirmations: u64,
    /// Unique data of specific swap.
    pub unique_swap_data: Vec<u8>,
    /// The reward assigned to watcher for providing help to complete the swap.
    pub watcher_reward: Option<WatcherReward>,
}

#[derive(Clone, Debug)]
pub struct WatcherSearchForSwapTxSpendInput<'a> {
    pub time_lock: u32,
    pub taker_pub: &'a [u8],
    pub maker_pub: &'a [u8],
    pub secret_hash: &'a [u8],
    pub tx: &'a [u8],
    pub search_from_block: u64,
    pub watcher_reward: bool,
}

#[derive(Clone, Debug)]
pub struct SendMakerPaymentSpendPreimageInput<'a> {
    pub preimage: &'a [u8],
    pub secret_hash: &'a [u8],
    pub secret: &'a [u8],
    pub taker_pub: &'a [u8],
    pub watcher_reward: bool,
}

pub struct SearchForSwapTxSpendInput<'a> {
    pub time_lock: u64,
    pub other_pub: &'a [u8],
    pub secret_hash: &'a [u8],
    pub tx: &'a [u8],
    pub search_from_block: u64,
    pub swap_contract_address: &'a Option<BytesJson>,
    pub swap_unique_data: &'a [u8],
    pub watcher_reward: bool,
}

#[derive(Copy, Clone, Debug)]
pub enum RewardTarget {
    None,
    Contract,
    PaymentSender,
    PaymentSpender,
    PaymentReceiver,
}

#[derive(Clone, Debug)]
pub struct WatcherReward {
    pub amount: BigDecimal,
    pub is_exact_amount: bool,
    pub reward_target: RewardTarget,
    pub send_contract_reward_on_spend: bool,
}

/// Helper struct wrapping arguments for [SwapOps::send_taker_payment] and [SwapOps::send_maker_payment].
#[derive(Clone, Debug)]
pub struct SendPaymentArgs<'a> {
    /// Time lock duration in seconds.
    pub time_lock_duration: u64,
    /// Payment can be refunded after this timestamp.
    pub time_lock: u64,
    /// This is either:
    /// * Taker's pubkey if this structure is used in [`SwapOps::send_maker_payment`].
    /// * Maker's pubkey if this structure is used in [`SwapOps::send_taker_payment`].
    pub other_pubkey: &'a [u8],
    /// Hash of the secret generated by maker.
    pub secret_hash: &'a [u8],
    /// Payment amount
    pub amount: BigDecimal,
    /// Swap contract address if applicable.
    pub swap_contract_address: &'a Option<BytesJson>,
    /// Unique data of specific swap.
    pub swap_unique_data: &'a [u8],
    /// Instructions for the next step of the swap (e.g., Lightning invoice).
    pub payment_instructions: &'a Option<PaymentInstructions>,
    /// The reward assigned to watcher for providing help to complete the swap.
    pub watcher_reward: Option<WatcherReward>,
    /// As of now, this field is specifically used to wait for confirmations of ERC20 approval transaction.
    pub wait_for_confirmation_until: u64,
}

#[derive(Clone, Debug)]
pub struct SpendPaymentArgs<'a> {
    /// This is either:
    /// * Taker's payment tx if this structure is used in [`SwapOps::send_maker_spends_taker_payment`].
    /// * Maker's payment tx if this structure is used in [`SwapOps::send_taker_spends_maker_payment`].
    pub other_payment_tx: &'a [u8],
    pub time_lock: u64,
    /// This is either:
    /// * Taker's pubkey if this structure is used in [`SwapOps::send_maker_spends_taker_payment`].
    /// * Maker's pubkey if this structure is used in [`SwapOps::send_taker_spends_maker_payment`].
    pub other_pubkey: &'a [u8],
    pub secret: &'a [u8],
    pub secret_hash: &'a [u8],
    pub swap_contract_address: &'a Option<BytesJson>,
    pub swap_unique_data: &'a [u8],
    pub watcher_reward: bool,
}

#[derive(Clone, Debug)]
pub struct RefundPaymentArgs<'a> {
    pub payment_tx: &'a [u8],
    pub time_lock: u64,
    /// This is either:
    /// * Taker's pubkey if this structure is used in [`SwapOps::send_maker_refunds_payment`].
    /// * Maker's pubkey if this structure is used in [`SwapOps::send_taker_refunds_payment`].
    pub other_pubkey: &'a [u8],
    pub secret_hash: &'a [u8],
    pub swap_contract_address: &'a Option<BytesJson>,
    pub swap_unique_data: &'a [u8],
    pub watcher_reward: bool,
}

/// Helper struct wrapping arguments for [SwapOps::check_if_my_payment_sent].
#[derive(Clone, Debug)]
pub struct CheckIfMyPaymentSentArgs<'a> {
    /// Payment can be refunded after this timestamp.
    pub time_lock: u64,
    /// Pubkey of other side of the swap.
    pub other_pub: &'a [u8],
    /// Hash of the secret generated by maker.
    pub secret_hash: &'a [u8],
    /// Search after specific block to avoid scanning entire blockchain.
    pub search_from_block: u64,
    /// Swap contract address if applicable.
    pub swap_contract_address: &'a Option<BytesJson>,
    /// Unique data of specific swap.
    pub swap_unique_data: &'a [u8],
    /// Payment amount.
    pub amount: &'a BigDecimal,
    /// Instructions for the next step of the swap (e.g., Lightning invoice).
    pub payment_instructions: &'a Option<PaymentInstructions>,
}

#[derive(Clone, Debug)]
pub struct ValidateFeeArgs<'a> {
    pub fee_tx: &'a TransactionEnum,
    pub expected_sender: &'a [u8],
    pub fee_addr: &'a [u8],
    pub dex_fee: &'a DexFee,
    pub min_block_number: u64,
    pub uuid: &'a [u8],
}

pub struct EthValidateFeeArgs<'a> {
    pub fee_tx_hash: &'a H256,
    pub expected_sender: &'a [u8],
    pub fee_addr: &'a [u8],
    pub amount: &'a BigDecimal,
    pub min_block_number: u64,
    pub uuid: &'a [u8],
}

#[derive(Clone, Debug)]
pub struct WaitForHTLCTxSpendArgs<'a> {
    pub tx_bytes: &'a [u8],
    pub secret_hash: &'a [u8],
    pub wait_until: u64,
    pub from_block: u64,
    pub swap_contract_address: &'a Option<BytesJson>,
    pub check_every: f64,
    pub watcher_reward: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum PaymentInstructions {
    #[cfg(not(target_arch = "wasm32"))]
    Lightning(Invoice),
    WatcherReward(BigDecimal),
}

#[derive(Clone, Debug, Default)]
pub struct PaymentInstructionArgs<'a> {
    pub secret_hash: &'a [u8],
    pub amount: BigDecimal,
    pub maker_lock_duration: u64,
    pub expires_in: u64,
    pub watcher_reward: bool,
    pub wait_until: u64,
}

#[derive(Display)]
pub enum PaymentInstructionsErr {
    LightningInvoiceErr(String),
    WatcherRewardErr(String),
    InternalError(String),
}

impl From<NumConversError> for PaymentInstructionsErr {
    fn from(e: NumConversError) -> Self { PaymentInstructionsErr::InternalError(e.to_string()) }
}

#[derive(Display)]
pub enum ValidateInstructionsErr {
    ValidateLightningInvoiceErr(String),
    UnsupportedCoin(String),
    DeserializationErr(String),
}

#[cfg(not(target_arch = "wasm32"))]
impl From<ParseOrSemanticError> for ValidateInstructionsErr {
    fn from(e: ParseOrSemanticError) -> Self { ValidateInstructionsErr::ValidateLightningInvoiceErr(e.to_string()) }
}

#[derive(Display)]
pub enum RefundError {
    DecodeErr(String),
    DbError(String),
    Timeout(String),
    Internal(String),
}

#[derive(Debug, Display)]
pub enum WatcherRewardError {
    RPCError(String),
    InvalidCoinType(String),
    InternalError(String),
}

/// Swap operations (mostly based on the Hash/Time locked transactions implemented by coin wallets).
#[async_trait]
pub trait SwapOps {
    fn send_taker_fee(&self, fee_addr: &[u8], dex_fee: DexFee, uuid: &[u8]) -> TransactionFut;

    fn send_maker_payment(&self, maker_payment_args: SendPaymentArgs<'_>) -> TransactionFut;

    fn send_taker_payment(&self, taker_payment_args: SendPaymentArgs<'_>) -> TransactionFut;

    fn send_maker_spends_taker_payment(&self, maker_spends_payment_args: SpendPaymentArgs<'_>) -> TransactionFut;

    fn send_taker_spends_maker_payment(&self, taker_spends_payment_args: SpendPaymentArgs<'_>) -> TransactionFut;

    async fn send_taker_refunds_payment(&self, taker_refunds_payment_args: RefundPaymentArgs<'_>) -> TransactionResult;

    async fn send_maker_refunds_payment(&self, maker_refunds_payment_args: RefundPaymentArgs<'_>) -> TransactionResult;

    fn validate_fee(&self, validate_fee_args: ValidateFeeArgs<'_>) -> ValidatePaymentFut<()>;

    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()>;

    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()>;

    fn check_if_my_payment_sent(
        &self,
        if_my_payment_sent_args: CheckIfMyPaymentSentArgs<'_>,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send>;

    async fn search_for_swap_tx_spend_my(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String>;

    async fn search_for_swap_tx_spend_other(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String>;

    async fn extract_secret(
        &self,
        secret_hash: &[u8],
        spend_tx: &[u8],
        watcher_reward: bool,
    ) -> Result<Vec<u8>, String>;

    fn check_tx_signed_by_pub(&self, tx: &[u8], expected_pub: &[u8]) -> Result<bool, MmError<ValidatePaymentError>>;

    /// Whether the refund transaction can be sent now
    /// For example: there are no additional conditions for ETH, but for some UTXO coins we should wait for
    /// locktime < MTP
    fn can_refund_htlc(&self, locktime: u64) -> Box<dyn Future<Item = CanRefundHtlc, Error = String> + Send + '_> {
        let now = now_sec();
        let result = if now > locktime {
            CanRefundHtlc::CanRefundNow
        } else {
            CanRefundHtlc::HaveToWait(locktime - now + 1)
        };
        Box::new(futures01::future::ok(result))
    }

    /// Whether the swap payment is refunded automatically or not when the locktime expires, or the other side fails the HTLC.
    fn is_auto_refundable(&self) -> bool;

    /// Waits for an htlc to be refunded automatically.
    async fn wait_for_htlc_refund(&self, _tx: &[u8], _locktime: u64) -> RefundResult<()>;

    fn negotiate_swap_contract_addr(
        &self,
        other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>>;

    /// Consider using [`SwapOps::derive_htlc_pubkey`] if you need the public key only.
    /// Some coins may not have a private key.
    fn derive_htlc_key_pair(&self, swap_unique_data: &[u8]) -> KeyPair;

    /// Derives an HTLC key-pair and returns a public key corresponding to that key.
    fn derive_htlc_pubkey(&self, swap_unique_data: &[u8]) -> Vec<u8>;

    fn validate_other_pubkey(&self, raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr>;

    /// Instructions from the taker on how the maker should send his payment.
    async fn maker_payment_instructions(
        &self,
        args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>>;

    /// Instructions from the maker on how the taker should send his payment.
    async fn taker_payment_instructions(
        &self,
        args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>>;

    fn validate_maker_payment_instructions(
        &self,
        instructions: &[u8],
        args: PaymentInstructionArgs<'_>,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>>;

    fn validate_taker_payment_instructions(
        &self,
        instructions: &[u8],
        args: PaymentInstructionArgs<'_>,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>>;

    fn is_supported_by_watchers(&self) -> bool { false }

    // Do we also need a method for the fallback contract?
    fn contract_supports_watchers(&self) -> bool { true }

    fn maker_locktime_multiplier(&self) -> f64 { 2.0 }
}

/// Operations on maker coin from taker swap side
#[async_trait]
pub trait TakerSwapMakerCoin {
    /// Performs an action on Maker coin payment just before the Taker Swap payment refund begins
    async fn on_taker_payment_refund_start(&self, maker_payment: &[u8]) -> RefundResult<()>;
    /// Performs an action on Maker coin payment after the Taker Swap payment is refunded successfully
    async fn on_taker_payment_refund_success(&self, maker_payment: &[u8]) -> RefundResult<()>;
}

/// Operations on taker coin from maker swap side
#[async_trait]
pub trait MakerSwapTakerCoin {
    /// Performs an action on Taker coin payment just before the Maker Swap payment refund begins
    async fn on_maker_payment_refund_start(&self, taker_payment: &[u8]) -> RefundResult<()>;
    /// Performs an action on Taker coin payment after the Maker Swap payment is refunded successfully
    async fn on_maker_payment_refund_success(&self, taker_payment: &[u8]) -> RefundResult<()>;
}

#[async_trait]
pub trait WatcherOps {
    fn send_maker_payment_spend_preimage(&self, input: SendMakerPaymentSpendPreimageInput) -> TransactionFut;

    fn send_taker_payment_refund_preimage(&self, watcher_refunds_payment_args: RefundPaymentArgs) -> TransactionFut;

    fn create_taker_payment_refund_preimage(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u64,
        maker_pub: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut;

    fn create_maker_payment_spend_preimage(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u64,
        maker_pub: &[u8],
        secret_hash: &[u8],
        swap_unique_data: &[u8],
    ) -> TransactionFut;

    fn watcher_validate_taker_fee(&self, input: WatcherValidateTakerFeeInput) -> ValidatePaymentFut<()>;

    fn watcher_validate_taker_payment(&self, input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()>;

    fn taker_validates_payment_spend_or_refund(&self, _input: ValidateWatcherSpendInput) -> ValidatePaymentFut<()>;

    async fn watcher_search_for_swap_tx_spend(
        &self,
        input: WatcherSearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String>;

    async fn get_taker_watcher_reward(
        &self,
        other_coin: &MmCoinEnum,
        coin_amount: Option<BigDecimal>,
        other_coin_amount: Option<BigDecimal>,
        reward_amount: Option<BigDecimal>,
        wait_until: u64,
    ) -> Result<WatcherReward, MmError<WatcherRewardError>>;

    async fn get_maker_watcher_reward(
        &self,
        other_coin: &MmCoinEnum,
        reward_amount: Option<BigDecimal>,
        wait_until: u64,
    ) -> Result<Option<WatcherReward>, MmError<WatcherRewardError>>;
}

/// Helper struct wrapping arguments for [SwapOpsV2::send_taker_funding]
pub struct SendTakerFundingArgs<'a> {
    /// Taker will be able to refund the payment after this timestamp
    pub time_lock: u64,
    /// The hash of the secret generated by taker, this needs to be revealed for immediate refund
    pub taker_secret_hash: &'a [u8],
    /// Maker's pubkey
    pub maker_pub: &'a [u8],
    /// DEX fee amount
    pub dex_fee_amount: BigDecimal,
    /// Additional reward for maker (premium)
    pub premium_amount: BigDecimal,
    /// Actual volume of taker's payment
    pub trading_amount: BigDecimal,
    /// Unique data of specific swap
    pub swap_unique_data: &'a [u8],
}

/// Helper struct wrapping arguments for [SwapOpsV2::refund_taker_funding_secret]
pub struct RefundFundingSecretArgs<'a, Coin: CoinAssocTypes + ?Sized> {
    pub funding_tx: &'a Coin::Tx,
    pub time_lock: u64,
    pub maker_pubkey: &'a Coin::Pubkey,
    pub taker_secret: &'a [u8],
    pub taker_secret_hash: &'a [u8],
    pub swap_contract_address: &'a Option<BytesJson>,
    pub swap_unique_data: &'a [u8],
    pub watcher_reward: bool,
}

/// Helper struct wrapping arguments for [SwapOpsV2::gen_taker_funding_spend_preimage]
pub struct GenTakerFundingSpendArgs<'a, Coin: CoinAssocTypes + ?Sized> {
    /// Taker payment transaction serialized to raw bytes
    pub funding_tx: &'a Coin::Tx,
    /// Maker's pubkey
    pub maker_pub: &'a Coin::Pubkey,
    /// Taker's pubkey
    pub taker_pub: &'a Coin::Pubkey,
    /// Timelock of the funding tx
    pub funding_time_lock: u64,
    /// The hash of the secret generated by taker
    pub taker_secret_hash: &'a [u8],
    /// Timelock of the taker payment
    pub taker_payment_time_lock: u64,
    /// The hash of the secret generated by maker
    pub maker_secret_hash: &'a [u8],
}

/// Helper struct wrapping arguments for [SwapOpsV2::validate_taker_funding]
pub struct ValidateTakerFundingArgs<'a, Coin: CoinAssocTypes + ?Sized> {
    /// Taker funding transaction
    pub funding_tx: &'a Coin::Tx,
    /// Taker will be able to refund the payment after this timestamp
    pub time_lock: u64,
    /// The hash of the secret generated by taker
    pub taker_secret_hash: &'a [u8],
    /// Taker's pubkey
    pub other_pub: &'a Coin::Pubkey,
    /// DEX fee amount
    pub dex_fee_amount: BigDecimal,
    /// Additional reward for maker (premium)
    pub premium_amount: BigDecimal,
    /// Actual volume of taker's payment
    pub trading_amount: BigDecimal,
    /// Unique data of specific swap
    pub swap_unique_data: &'a [u8],
}

/// Helper struct wrapping arguments for taker payment's spend generation, used in
/// [SwapOpsV2::gen_taker_payment_spend_preimage], [SwapOpsV2::validate_taker_payment_spend_preimage] and
/// [SwapOpsV2::sign_and_broadcast_taker_payment_spend]
pub struct GenTakerPaymentSpendArgs<'a, Coin: CoinAssocTypes + ?Sized> {
    /// Taker payment transaction serialized to raw bytes
    pub taker_tx: &'a Coin::Tx,
    /// Taker will be able to refund the payment after this timestamp
    pub time_lock: u64,
    /// The hash of the secret generated by maker
    pub secret_hash: &'a [u8],
    /// Maker's pubkey
    pub maker_pub: &'a Coin::Pubkey,
    /// Taker's pubkey
    pub taker_pub: &'a Coin::Pubkey,
    /// Pubkey of address, receiving DEX fees
    pub dex_fee_pub: &'a [u8],
    /// DEX fee amount
    pub dex_fee_amount: BigDecimal,
    /// Additional reward for maker (premium)
    pub premium_amount: BigDecimal,
    /// Actual volume of taker's payment
    pub trading_amount: BigDecimal,
}

/// Taker payment spend preimage with taker's signature
pub struct TxPreimageWithSig<Coin: CoinAssocTypes + ?Sized> {
    /// The preimage, might be () for certain coin types (only signature might be used)
    pub preimage: Coin::Preimage,
    /// Taker's signature
    pub signature: Coin::Sig,
}

/// Enum covering error cases that can happen during transaction preimage generation.
#[derive(Debug, Display)]
pub enum TxGenError {
    /// RPC error
    Rpc(String),
    /// Error during conversion of BigDecimal amount to coin's specific monetary units (satoshis, wei, etc.).
    NumConversion(String),
    /// Address derivation error.
    AddressDerivation(String),
    /// Problem with tx preimage signing.
    Signing(String),
    /// Legacy error produced by usage of try_s/try_fus and other similar macros.
    Legacy(String),
    /// Input payment timelock overflows the type used by specific coin.
    LocktimeOverflow(String),
    /// Transaction fee is too high
    TxFeeTooHigh(String),
    /// Previous tx is not valid
    PrevTxIsNotValid(String),
}

impl From<UtxoRpcError> for TxGenError {
    fn from(err: UtxoRpcError) -> Self { TxGenError::Rpc(err.to_string()) }
}

impl From<NumConversError> for TxGenError {
    fn from(err: NumConversError) -> Self { TxGenError::NumConversion(err.to_string()) }
}

impl From<UtxoSignWithKeyPairError> for TxGenError {
    fn from(err: UtxoSignWithKeyPairError) -> Self { TxGenError::Signing(err.to_string()) }
}

/// Enum covering error cases that can happen during taker funding validation.
#[derive(Debug, Display)]
pub enum ValidateTakerFundingError {
    /// Payment sent to wrong address or has invalid amount.
    InvalidDestinationOrAmount(String),
    /// Error during conversion of BigDecimal amount to coin's specific monetary units (satoshis, wei, etc.).
    NumConversion(String),
    /// RPC error.
    Rpc(String),
    /// Serialized tx bytes don't match ones received from coin's RPC.
    #[display(fmt = "Tx bytes {:02x} don't match ones received from rpc {:02x}", actual, from_rpc)]
    TxBytesMismatch { from_rpc: BytesJson, actual: BytesJson },
    /// Provided transaction doesn't have output with specific index
    TxLacksOfOutputs,
    /// Input payment timelock overflows the type used by specific coin.
    LocktimeOverflow(String),
}

impl From<NumConversError> for ValidateTakerFundingError {
    fn from(err: NumConversError) -> Self { ValidateTakerFundingError::NumConversion(err.to_string()) }
}

impl From<UtxoRpcError> for ValidateTakerFundingError {
    fn from(err: UtxoRpcError) -> Self { ValidateTakerFundingError::Rpc(err.to_string()) }
}

/// Enum covering error cases that can happen during taker funding spend preimage validation.
#[derive(Debug, Display)]
pub enum ValidateTakerFundingSpendPreimageError {
    /// Funding tx has no outputs
    FundingTxNoOutputs,
    /// Actual preimage fee is either too high or too small
    UnexpectedPreimageFee(String),
    /// Error during signature deserialization.
    InvalidMakerSignature,
    /// Error during preimage comparison to an expected one.
    InvalidPreimage(String),
    /// Error during taker's signature check.
    SignatureVerificationFailure(String),
    /// Error during generation of an expected preimage.
    TxGenError(String),
    /// Input payment timelock overflows the type used by specific coin.
    LocktimeOverflow(String),
    /// Coin's RPC error
    Rpc(String),
}

impl From<UtxoSignWithKeyPairError> for ValidateTakerFundingSpendPreimageError {
    fn from(err: UtxoSignWithKeyPairError) -> Self {
        ValidateTakerFundingSpendPreimageError::SignatureVerificationFailure(err.to_string())
    }
}

impl From<TxGenError> for ValidateTakerFundingSpendPreimageError {
    fn from(err: TxGenError) -> Self { ValidateTakerFundingSpendPreimageError::TxGenError(format!("{:?}", err)) }
}

impl From<UtxoRpcError> for ValidateTakerFundingSpendPreimageError {
    fn from(err: UtxoRpcError) -> Self { ValidateTakerFundingSpendPreimageError::Rpc(err.to_string()) }
}

/// Enum covering error cases that can happen during taker payment spend preimage validation.
#[derive(Debug, Display)]
pub enum ValidateTakerPaymentSpendPreimageError {
    /// Error during signature deserialization.
    InvalidTakerSignature,
    /// Error during preimage comparison to an expected one.
    InvalidPreimage(String),
    /// Error during taker's signature check.
    SignatureVerificationFailure(String),
    /// Error during generation of an expected preimage.
    TxGenError(String),
    /// Input payment timelock overflows the type used by specific coin.
    LocktimeOverflow(String),
}

impl From<UtxoSignWithKeyPairError> for ValidateTakerPaymentSpendPreimageError {
    fn from(err: UtxoSignWithKeyPairError) -> Self {
        ValidateTakerPaymentSpendPreimageError::SignatureVerificationFailure(err.to_string())
    }
}

impl From<TxGenError> for ValidateTakerPaymentSpendPreimageError {
    fn from(err: TxGenError) -> Self { ValidateTakerPaymentSpendPreimageError::TxGenError(format!("{:?}", err)) }
}

/// Helper trait used for various types serialization to bytes
pub trait ToBytes {
    fn to_bytes(&self) -> Vec<u8>;
}

/// Defines associated types specific to each coin (Pubkey, Address, etc.)
pub trait CoinAssocTypes {
    type Pubkey: ToBytes + Send + Sync;
    type PubkeyParseError: Send + std::fmt::Display;
    type Tx: Transaction + Send + Sync;
    type TxParseError: Send + std::fmt::Display;
    type Preimage: ToBytes + Send + Sync;
    type PreimageParseError: Send + std::fmt::Display;
    type Sig: ToBytes + Send + Sync;
    type SigParseError: Send + std::fmt::Display;

    fn parse_pubkey(&self, pubkey: &[u8]) -> Result<Self::Pubkey, Self::PubkeyParseError>;

    fn parse_tx(&self, tx: &[u8]) -> Result<Self::Tx, Self::TxParseError>;

    fn parse_preimage(&self, tx: &[u8]) -> Result<Self::Preimage, Self::PreimageParseError>;

    fn parse_signature(&self, sig: &[u8]) -> Result<Self::Sig, Self::SigParseError>;
}

/// Operations specific to the [Trading Protocol Upgrade implementation](https://github.com/KomodoPlatform/komodo-defi-framework/issues/1895)
#[async_trait]
pub trait SwapOpsV2: CoinAssocTypes + Send + Sync + 'static {
    /// Generate and broadcast taker funding transaction that includes dex fee, maker premium and actual trading volume.
    /// Funding tx can be reclaimed immediately if maker back-outs (doesn't send maker payment)
    async fn send_taker_funding(&self, args: SendTakerFundingArgs<'_>) -> Result<Self::Tx, TransactionErr>;

    /// Validates taker funding transaction.
    async fn validate_taker_funding(&self, args: ValidateTakerFundingArgs<'_, Self>) -> ValidateTakerFundingResult;

    /// Refunds taker funding transaction using time-locked path without secret reveal.
    async fn refund_taker_funding_timelock(&self, args: RefundPaymentArgs<'_>) -> TransactionResult;

    /// Reclaims taker funding transaction using immediate refund path with secret reveal.
    async fn refund_taker_funding_secret(
        &self,
        args: RefundFundingSecretArgs<'_, Self>,
    ) -> Result<Self::Tx, TransactionErr>;

    /// Generates and signs a preimage spending funding tx to the combined taker payment
    async fn gen_taker_funding_spend_preimage(
        &self,
        args: &GenTakerFundingSpendArgs<'_, Self>,
        swap_unique_data: &[u8],
    ) -> GenPreimageResult<Self>;

    /// Validates taker funding spend preimage generated and signed by maker
    async fn validate_taker_funding_spend_preimage(
        &self,
        gen_args: &GenTakerFundingSpendArgs<'_, Self>,
        preimage: &TxPreimageWithSig<Self>,
    ) -> ValidateTakerFundingSpendPreimageResult;

    /// Generates and signs a preimage spending funding tx to the combined taker payment
    async fn sign_and_send_taker_funding_spend(
        &self,
        preimage: &TxPreimageWithSig<Self>,
        args: &GenTakerFundingSpendArgs<'_, Self>,
        swap_unique_data: &[u8],
    ) -> Result<Self::Tx, TransactionErr>;

    /// Refunds taker payment transaction.
    async fn refund_combined_taker_payment(&self, args: RefundPaymentArgs<'_>) -> TransactionResult;

    /// Generates and signs taker payment spend preimage. The preimage and signature should be
    /// shared with maker to proceed with protocol execution.
    async fn gen_taker_payment_spend_preimage(
        &self,
        args: &GenTakerPaymentSpendArgs<'_, Self>,
        swap_unique_data: &[u8],
    ) -> GenPreimageResult<Self>;

    /// Validate taker payment spend preimage on maker's side.
    async fn validate_taker_payment_spend_preimage(
        &self,
        gen_args: &GenTakerPaymentSpendArgs<'_, Self>,
        preimage: &TxPreimageWithSig<Self>,
    ) -> ValidateTakerPaymentSpendPreimageResult;

    /// Sign and broadcast taker payment spend on maker's side.
    async fn sign_and_broadcast_taker_payment_spend(
        &self,
        preimage: &TxPreimageWithSig<Self>,
        gen_args: &GenTakerPaymentSpendArgs<'_, Self>,
        secret: &[u8],
        swap_unique_data: &[u8],
    ) -> TransactionResult;

    /// Derives an HTLC key-pair and returns a public key corresponding to that key.
    fn derive_htlc_pubkey_v2(&self, swap_unique_data: &[u8]) -> Self::Pubkey;
}

/// Operations that coins have independently from the MarketMaker.
/// That is, things implemented by the coin wallets or public coin services.
#[async_trait]
pub trait MarketCoinOps {
    fn ticker(&self) -> &str;

    fn my_address(&self) -> MmResult<String, MyAddressError>;

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>>;

    fn sign_message_hash(&self, _message: &str) -> Option<[u8; 32]>;

    fn sign_message(&self, _message: &str) -> SignatureResult<String>;

    fn verify_message(&self, _signature: &str, _message: &str, _address: &str) -> VerificationResult<bool>;

    fn get_non_zero_balance(&self) -> NonZeroBalanceFut<MmNumber> {
        let closure = |spendable: BigDecimal| {
            if spendable.is_zero() {
                return MmError::err(GetNonZeroBalance::BalanceIsZero);
            }
            Ok(MmNumber::from(spendable))
        };
        Box::new(self.my_spendable_balance().map_err(From::from).and_then(closure))
    }

    fn my_balance(&self) -> BalanceFut<CoinBalance>;

    fn my_spendable_balance(&self) -> BalanceFut<BigDecimal> {
        Box::new(self.my_balance().map(|CoinBalance { spendable, .. }| spendable))
    }

    /// Base coin balance for tokens, e.g. ETH balance in ERC20 case
    fn base_coin_balance(&self) -> BalanceFut<BigDecimal>;

    fn platform_ticker(&self) -> &str;

    /// Receives raw transaction bytes in hexadecimal format as input and returns tx hash in hexadecimal format
    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send>;

    /// Receives raw transaction bytes as input and returns tx hash in hexadecimal format
    fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send>;

    /// Signs raw utxo transaction in hexadecimal format as input and returns signed transaction in hexadecimal format
    async fn sign_raw_tx(&self, args: &SignRawTransactionRequest) -> RawTransactionResult;

    fn wait_for_confirmations(&self, input: ConfirmPaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send>;

    fn wait_for_htlc_tx_spend(&self, args: WaitForHTLCTxSpendArgs<'_>) -> TransactionFut;

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>>;

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send>;

    fn display_priv_key(&self) -> Result<String, String>;

    /// Get the minimum amount to send.
    fn min_tx_amount(&self) -> BigDecimal;

    /// Get the minimum amount to trade.
    fn min_trading_vol(&self) -> MmNumber;

    fn is_privacy(&self) -> bool { false }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum WithdrawFee {
    UtxoFixed {
        amount: BigDecimal,
    },
    UtxoPerKbyte {
        amount: BigDecimal,
    },
    EthGas {
        /// in gwei
        gas_price: BigDecimal,
        gas: u64,
    },
    Qrc20Gas {
        /// in satoshi
        gas_limit: u64,
        gas_price: u64,
    },
    CosmosGas {
        gas_limit: u64,
        gas_price: f64,
    },
}

pub struct WithdrawSenderAddress<Address, Pubkey> {
    address: Address,
    pubkey: Pubkey,
    derivation_path: Option<DerivationPath>,
}

impl<Address, Pubkey> From<HDAddress<Address, Pubkey>> for WithdrawSenderAddress<Address, Pubkey> {
    fn from(addr: HDAddress<Address, Pubkey>) -> Self {
        WithdrawSenderAddress {
            address: addr.address,
            pubkey: addr.pubkey,
            derivation_path: Some(addr.derivation_path),
        }
    }
}

/// Rename to `GetWithdrawSenderAddresses` when withdraw supports multiple `from` addresses.
#[async_trait]
pub trait GetWithdrawSenderAddress {
    type Address;
    type Pubkey;

    async fn get_withdraw_sender_address(
        &self,
        req: &WithdrawRequest,
    ) -> MmResult<WithdrawSenderAddress<Self::Address, Self::Pubkey>, WithdrawError>;
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum WithdrawFrom {
    AddressId(HDAccountAddressId),
    /// Don't use `Bip44DerivationPath` or `RpcDerivationPath` because if there is an error in the path,
    /// `serde::Deserialize` returns "data did not match any variant of untagged enum WithdrawFrom".
    /// It's better to show the user an informative error.
    DerivationPath {
        derivation_path: String,
    },
    HDWalletAddress(StandardHDCoinAddress),
}

#[derive(Clone, Deserialize)]
pub struct WithdrawRequest {
    coin: String,
    from: Option<WithdrawFrom>,
    to: String,
    #[serde(default)]
    amount: BigDecimal,
    #[serde(default)]
    max: bool,
    fee: Option<WithdrawFee>,
    memo: Option<String>,
    /// Currently, this flag is used by ETH/ERC20 coins activated with MetaMask **only**.
    #[cfg(target_arch = "wasm32")]
    #[serde(default)]
    broadcast: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum StakingDetails {
    Qtum(QtumDelegationRequest),
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct AddDelegateRequest {
    pub coin: String,
    pub staking_details: StakingDetails,
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct RemoveDelegateRequest {
    pub coin: String,
}

#[derive(Deserialize)]
pub struct GetStakingInfosRequest {
    pub coin: String,
}

#[derive(Serialize, Deserialize)]
pub struct SignatureRequest {
    coin: String,
    message: String,
}

#[derive(Serialize, Deserialize)]
pub struct VerificationRequest {
    coin: String,
    message: String,
    signature: String,
    address: String,
}

impl WithdrawRequest {
    pub fn new_max(coin: String, to: String) -> WithdrawRequest {
        WithdrawRequest {
            coin,
            from: None,
            to,
            amount: 0.into(),
            max: true,
            fee: None,
            memo: None,
            #[cfg(target_arch = "wasm32")]
            broadcast: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StakingInfosDetails {
    Qtum(QtumStakingInfosDetails),
}

impl From<QtumStakingInfosDetails> for StakingInfosDetails {
    fn from(qtum_staking_infos: QtumStakingInfosDetails) -> Self { StakingInfosDetails::Qtum(qtum_staking_infos) }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StakingInfos {
    pub staking_infos_details: StakingInfosDetails,
}

#[derive(Serialize)]
pub struct SignatureResponse {
    signature: String,
}

#[derive(Serialize)]
pub struct VerificationResponse {
    is_valid: bool,
}

/// Please note that no type should have the same structure as another type,
/// because this enum has the `untagged` deserialization.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type")]
pub enum TxFeeDetails {
    Utxo(UtxoFeeDetails),
    Eth(EthTxFeeDetails),
    Qrc20(Qrc20FeeDetails),
    Slp(SlpFeeDetails),
    Tendermint(TendermintFeeDetails),
    #[cfg(all(
        feature = "enable-solana",
        not(target_os = "ios"),
        not(target_os = "android"),
        not(target_arch = "wasm32")
    ))]
    Solana(SolanaFeeDetails),
}

/// Deserialize the TxFeeDetails as an untagged enum.
impl<'de> Deserialize<'de> for TxFeeDetails {
    fn deserialize<D>(deserializer: D) -> Result<Self, <D as Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum TxFeeDetailsUnTagged {
            Utxo(UtxoFeeDetails),
            Eth(EthTxFeeDetails),
            Qrc20(Qrc20FeeDetails),
            #[cfg(all(
                feature = "enable-solana",
                not(target_os = "ios"),
                not(target_os = "android"),
                not(target_arch = "wasm32")
            ))]
            Solana(SolanaFeeDetails),
            Tendermint(TendermintFeeDetails),
        }

        match Deserialize::deserialize(deserializer)? {
            TxFeeDetailsUnTagged::Utxo(f) => Ok(TxFeeDetails::Utxo(f)),
            TxFeeDetailsUnTagged::Eth(f) => Ok(TxFeeDetails::Eth(f)),
            TxFeeDetailsUnTagged::Qrc20(f) => Ok(TxFeeDetails::Qrc20(f)),
            #[cfg(all(
                feature = "enable-solana",
                not(target_os = "ios"),
                not(target_os = "android"),
                not(target_arch = "wasm32")
            ))]
            TxFeeDetailsUnTagged::Solana(f) => Ok(TxFeeDetails::Solana(f)),
            TxFeeDetailsUnTagged::Tendermint(f) => Ok(TxFeeDetails::Tendermint(f)),
        }
    }
}

impl From<EthTxFeeDetails> for TxFeeDetails {
    fn from(eth_details: EthTxFeeDetails) -> Self { TxFeeDetails::Eth(eth_details) }
}

impl From<UtxoFeeDetails> for TxFeeDetails {
    fn from(utxo_details: UtxoFeeDetails) -> Self { TxFeeDetails::Utxo(utxo_details) }
}

impl From<Qrc20FeeDetails> for TxFeeDetails {
    fn from(qrc20_details: Qrc20FeeDetails) -> Self { TxFeeDetails::Qrc20(qrc20_details) }
}

#[cfg(all(
    feature = "enable-solana",
    not(target_os = "ios"),
    not(target_os = "android"),
    not(target_arch = "wasm32")
))]
impl From<SolanaFeeDetails> for TxFeeDetails {
    fn from(solana_details: SolanaFeeDetails) -> Self { TxFeeDetails::Solana(solana_details) }
}

impl From<TendermintFeeDetails> for TxFeeDetails {
    fn from(tendermint_details: TendermintFeeDetails) -> Self { TxFeeDetails::Tendermint(tendermint_details) }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct KmdRewardsDetails {
    amount: BigDecimal,
    claimed_by_me: bool,
}

impl KmdRewardsDetails {
    pub fn claimed_by_me(amount: BigDecimal) -> KmdRewardsDetails {
        KmdRewardsDetails {
            amount,
            claimed_by_me: true,
        }
    }
}

#[derive(Default, Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum TransactionType {
    StakingDelegation,
    RemoveDelegation,
    #[default]
    StandardTransfer,
    TokenTransfer(BytesJson),
    FeeForTokenTx,
    CustomTendermintMsg {
        msg_type: CustomTendermintMsgType,
        token_id: Option<BytesJson>,
    },
    NftTransfer,
}

/// Transaction details
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TransactionDetails {
    /// Raw bytes of signed transaction, this should be sent as is to `send_raw_transaction_bytes` RPC to broadcast the transaction
    pub tx_hex: BytesJson,
    /// Transaction hash in hexadecimal format
    tx_hash: String,
    /// Coins are sent from these addresses
    from: Vec<String>,
    /// Coins are sent to these addresses
    to: Vec<String>,
    /// Total tx amount
    total_amount: BigDecimal,
    /// The amount spent from "my" address
    spent_by_me: BigDecimal,
    /// The amount received by "my" address
    received_by_me: BigDecimal,
    /// Resulting "my" balance change
    my_balance_change: BigDecimal,
    /// Block height
    block_height: u64,
    /// Transaction timestamp
    timestamp: u64,
    /// Every coin can has specific fee details:
    /// In UTXO tx fee is paid with the coin itself (e.g. 1 BTC and 0.0001 BTC fee).
    /// But for ERC20 token transfer fee is paid with another coin: ETH, because it's ETH smart contract function call that requires gas to be burnt.
    fee_details: Option<TxFeeDetails>,
    /// The coin transaction belongs to
    coin: String,
    /// Internal MM2 id used for internal transaction identification, for some coins it might be equal to transaction hash
    internal_id: BytesJson,
    /// Amount of accrued rewards.
    #[serde(skip_serializing_if = "Option::is_none")]
    kmd_rewards: Option<KmdRewardsDetails>,
    /// Type of transactions, default is StandardTransfer
    #[serde(default)]
    transaction_type: TransactionType,
    memo: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct BlockHeightAndTime {
    height: u64,
    timestamp: u64,
}

impl TransactionDetails {
    /// Whether the transaction details block height should be updated (when tx is confirmed)
    pub fn should_update_block_height(&self) -> bool {
        // checking for std::u64::MAX because there was integer overflow
        // in case of electrum returned -1 so there could be records with MAX confirmations
        self.block_height == 0 || self.block_height == std::u64::MAX
    }

    /// Whether the transaction timestamp should be updated (when tx is confirmed)
    pub fn should_update_timestamp(&self) -> bool {
        // checking for std::u64::MAX because there was integer overflow
        // in case of electrum returned -1 so there could be records with MAX confirmations
        self.timestamp == 0
    }

    pub fn should_update_kmd_rewards(&self) -> bool { self.coin == "KMD" && self.kmd_rewards.is_none() }

    pub fn firo_negative_fee(&self) -> bool {
        match &self.fee_details {
            Some(TxFeeDetails::Utxo(utxo)) => utxo.amount < 0.into() && self.coin == "FIRO",
            _ => false,
        }
    }

    pub fn should_update(&self) -> bool {
        self.should_update_block_height()
            || self.should_update_timestamp()
            || self.should_update_kmd_rewards()
            || self.firo_negative_fee()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TradeFee {
    pub coin: String,
    pub amount: MmNumber,
    pub paid_from_trading_vol: bool,
}

#[derive(Clone, Debug, Default, PartialEq, PartialOrd, Serialize)]
pub struct CoinBalance {
    pub spendable: BigDecimal,
    pub unspendable: BigDecimal,
}

impl CoinBalance {
    pub fn new(spendable: BigDecimal) -> CoinBalance {
        CoinBalance {
            spendable,
            unspendable: BigDecimal::from(0),
        }
    }

    pub fn into_total(self) -> BigDecimal { self.spendable + self.unspendable }

    pub fn get_total(&self) -> BigDecimal { &self.spendable + &self.unspendable }
}

impl Add for CoinBalance {
    type Output = CoinBalance;

    fn add(self, rhs: Self) -> Self::Output {
        CoinBalance {
            spendable: self.spendable + rhs.spendable,
            unspendable: self.unspendable + rhs.unspendable,
        }
    }
}

/// The approximation is needed to cover the dynamic miner fee changing during a swap.
#[derive(Clone, Debug)]
pub enum FeeApproxStage {
    /// Do not increase the trade fee.
    WithoutApprox,
    /// Increase the trade fee slightly.
    StartSwap,
    /// Increase the trade fee slightly
    WatcherPreimage,
    /// Increase the trade fee significantly.
    OrderIssue,
    /// Increase the trade fee largely.
    TradePreimage,
}

#[derive(Debug)]
pub enum TradePreimageValue {
    Exact(BigDecimal),
    UpperBound(BigDecimal),
}

#[derive(Debug, Display, PartialEq)]
pub enum TradePreimageError {
    #[display(
        fmt = "Not enough {} to preimage the trade: available {}, required at least {}",
        coin,
        available,
        required
    )]
    NotSufficientBalance {
        coin: String,
        available: BigDecimal,
        required: BigDecimal,
    },
    #[display(fmt = "The amount {} less than minimum transaction amount {}", amount, threshold)]
    AmountIsTooSmall { amount: BigDecimal, threshold: BigDecimal },
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

impl From<NumConversError> for TradePreimageError {
    fn from(e: NumConversError) -> Self { TradePreimageError::InternalError(e.to_string()) }
}

impl From<UnexpectedDerivationMethod> for TradePreimageError {
    fn from(e: UnexpectedDerivationMethod) -> Self { TradePreimageError::InternalError(e.to_string()) }
}

impl TradePreimageError {
    /// Construct [`TradePreimageError`] from [`GenerateTxError`] using additional `coin` and `decimals`.
    pub fn from_generate_tx_error(
        gen_tx_err: GenerateTxError,
        coin: String,
        decimals: u8,
        is_upper_bound: bool,
    ) -> TradePreimageError {
        match gen_tx_err {
            GenerateTxError::EmptyUtxoSet { required } => {
                let required = big_decimal_from_sat_unsigned(required, decimals);
                TradePreimageError::NotSufficientBalance {
                    coin,
                    available: BigDecimal::from(0),
                    required,
                }
            },
            GenerateTxError::EmptyOutputs => TradePreimageError::InternalError(gen_tx_err.to_string()),
            GenerateTxError::OutputValueLessThanDust { value, dust } => {
                if is_upper_bound {
                    // If the preimage value is [`TradePreimageValue::UpperBound`], then we had to pass the account balance as the output value.
                    if value == 0 {
                        let required = big_decimal_from_sat_unsigned(dust, decimals);
                        TradePreimageError::NotSufficientBalance {
                            coin,
                            available: big_decimal_from_sat_unsigned(value, decimals),
                            required,
                        }
                    } else {
                        let error = format!(
                            "Output value {} (equal to the account balance) less than dust {}. Probably, dust is not set or outdated",
                            value, dust
                        );
                        TradePreimageError::InternalError(error)
                    }
                } else {
                    let amount = big_decimal_from_sat_unsigned(value, decimals);
                    let threshold = big_decimal_from_sat_unsigned(dust, decimals);
                    TradePreimageError::AmountIsTooSmall { amount, threshold }
                }
            },
            GenerateTxError::DeductFeeFromOutputFailed {
                output_value, required, ..
            } => {
                let available = big_decimal_from_sat_unsigned(output_value, decimals);
                let required = big_decimal_from_sat_unsigned(required, decimals);
                TradePreimageError::NotSufficientBalance {
                    coin,
                    available,
                    required,
                }
            },
            GenerateTxError::NotEnoughUtxos { sum_utxos, required } => {
                let available = big_decimal_from_sat_unsigned(sum_utxos, decimals);
                let required = big_decimal_from_sat_unsigned(required, decimals);
                TradePreimageError::NotSufficientBalance {
                    coin,
                    available,
                    required,
                }
            },
            GenerateTxError::Transport(e) => TradePreimageError::Transport(e),
            GenerateTxError::Internal(e) => TradePreimageError::InternalError(e),
        }
    }
}

/// The reason of unsuccessful conversion of two internal numbers, e.g. `u64` from `BigNumber`.
#[derive(Debug, Display)]
pub struct NumConversError(String);

impl From<ParseBigDecimalError> for NumConversError {
    fn from(e: ParseBigDecimalError) -> Self { NumConversError::new(e.to_string()) }
}

impl NumConversError {
    pub fn new(description: String) -> NumConversError { NumConversError(description) }

    pub fn description(&self) -> &str { &self.0 }
}

#[derive(Clone, Debug, Display, PartialEq, Serialize)]
pub enum BalanceError {
    #[display(fmt = "Transport: {}", _0)]
    Transport(String),
    #[display(fmt = "Invalid response: {}", _0)]
    InvalidResponse(String),
    UnexpectedDerivationMethod(UnexpectedDerivationMethod),
    #[display(fmt = "Wallet storage error: {}", _0)]
    WalletStorageError(String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
}

#[derive(Debug, PartialEq, Display)]
pub enum GetNonZeroBalance {
    #[display(fmt = "Internal error when retrieving balance")]
    MyBalanceError(BalanceError),
    #[display(fmt = "Balance is zero")]
    BalanceIsZero,
}

impl From<BalanceError> for GetNonZeroBalance {
    fn from(e: BalanceError) -> Self { GetNonZeroBalance::MyBalanceError(e) }
}

impl From<NumConversError> for BalanceError {
    fn from(e: NumConversError) -> Self { BalanceError::Internal(e.to_string()) }
}

impl From<UnexpectedDerivationMethod> for BalanceError {
    fn from(e: UnexpectedDerivationMethod) -> Self { BalanceError::UnexpectedDerivationMethod(e) }
}

impl From<Bip32Error> for BalanceError {
    fn from(e: Bip32Error) -> Self { BalanceError::Internal(e.to_string()) }
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum StakingInfosError {
    #[display(fmt = "Staking infos not available for: {}", coin)]
    CoinDoesntSupportStakingInfos { coin: String },
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(fmt = "Derivation method is not supported: {}", _0)]
    UnexpectedDerivationMethod(String),
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl From<UtxoRpcError> for StakingInfosError {
    fn from(e: UtxoRpcError) -> Self {
        match e {
            UtxoRpcError::Transport(rpc) | UtxoRpcError::ResponseParseError(rpc) => {
                StakingInfosError::Transport(rpc.to_string())
            },
            UtxoRpcError::InvalidResponse(error) => StakingInfosError::Transport(error),
            UtxoRpcError::Internal(error) => StakingInfosError::Internal(error),
        }
    }
}

impl From<UnexpectedDerivationMethod> for StakingInfosError {
    fn from(e: UnexpectedDerivationMethod) -> Self { StakingInfosError::UnexpectedDerivationMethod(e.to_string()) }
}

impl From<Qrc20AddressError> for StakingInfosError {
    fn from(e: Qrc20AddressError) -> Self {
        match e {
            Qrc20AddressError::UnexpectedDerivationMethod(e) => StakingInfosError::UnexpectedDerivationMethod(e),
            Qrc20AddressError::ScriptHashTypeNotSupported { script_hash_type } => {
                StakingInfosError::Internal(format!("Script hash type '{}' is not supported", script_hash_type))
            },
        }
    }
}

impl HttpStatusCode for StakingInfosError {
    fn status_code(&self) -> StatusCode {
        match self {
            StakingInfosError::NoSuchCoin { .. }
            | StakingInfosError::CoinDoesntSupportStakingInfos { .. }
            | StakingInfosError::UnexpectedDerivationMethod(_) => StatusCode::BAD_REQUEST,
            StakingInfosError::Transport(_) | StakingInfosError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CoinFindError> for StakingInfosError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => StakingInfosError::NoSuchCoin { coin },
        }
    }
}

#[derive(Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum DelegationError {
    #[display(
        fmt = "Not enough {} to delegate: available {}, required at least {}",
        coin,
        available,
        required
    )]
    NotSufficientBalance {
        coin: String,
        available: BigDecimal,
        required: BigDecimal,
    },
    #[display(fmt = "The amount {} is too small, required at least {}", amount, threshold)]
    AmountTooLow { amount: BigDecimal, threshold: BigDecimal },
    #[display(fmt = "Delegation not available for: {}", coin)]
    CoinDoesntSupportDelegation { coin: String },
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(fmt = "{}", _0)]
    CannotInteractWithSmartContract(String),
    #[display(fmt = "{}", _0)]
    AddressError(String),
    #[display(fmt = "Already delegating to: {}", _0)]
    AlreadyDelegating(String),
    #[display(fmt = "Delegation is not supported, reason: {}", reason)]
    DelegationOpsNotSupported { reason: String },
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

impl From<UtxoRpcError> for DelegationError {
    fn from(e: UtxoRpcError) -> Self {
        match e {
            UtxoRpcError::Transport(transport) | UtxoRpcError::ResponseParseError(transport) => {
                DelegationError::Transport(transport.to_string())
            },
            UtxoRpcError::InvalidResponse(resp) => DelegationError::Transport(resp),
            UtxoRpcError::Internal(internal) => DelegationError::InternalError(internal),
        }
    }
}

impl From<StakingInfosError> for DelegationError {
    fn from(e: StakingInfosError) -> Self {
        match e {
            StakingInfosError::CoinDoesntSupportStakingInfos { coin } => {
                DelegationError::CoinDoesntSupportDelegation { coin }
            },
            StakingInfosError::NoSuchCoin { coin } => DelegationError::NoSuchCoin { coin },
            StakingInfosError::Transport(e) => DelegationError::Transport(e),
            StakingInfosError::UnexpectedDerivationMethod(reason) => {
                DelegationError::DelegationOpsNotSupported { reason }
            },
            StakingInfosError::Internal(e) => DelegationError::InternalError(e),
        }
    }
}

impl From<CoinFindError> for DelegationError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => DelegationError::NoSuchCoin { coin },
        }
    }
}

impl From<BalanceError> for DelegationError {
    fn from(e: BalanceError) -> Self {
        match e {
            BalanceError::Transport(error) | BalanceError::InvalidResponse(error) => DelegationError::Transport(error),
            BalanceError::UnexpectedDerivationMethod(e) => {
                DelegationError::DelegationOpsNotSupported { reason: e.to_string() }
            },
            e @ BalanceError::WalletStorageError(_) => DelegationError::InternalError(e.to_string()),
            BalanceError::Internal(internal) => DelegationError::InternalError(internal),
        }
    }
}

impl From<UtxoSignWithKeyPairError> for DelegationError {
    fn from(e: UtxoSignWithKeyPairError) -> Self {
        let error = format!("Error signing: {}", e);
        DelegationError::InternalError(error)
    }
}

impl From<PrivKeyPolicyNotAllowed> for DelegationError {
    fn from(e: PrivKeyPolicyNotAllowed) -> Self { DelegationError::DelegationOpsNotSupported { reason: e.to_string() } }
}

impl From<UnexpectedDerivationMethod> for DelegationError {
    fn from(e: UnexpectedDerivationMethod) -> Self {
        DelegationError::DelegationOpsNotSupported { reason: e.to_string() }
    }
}

impl From<ScriptHashTypeNotSupported> for DelegationError {
    fn from(e: ScriptHashTypeNotSupported) -> Self { DelegationError::AddressError(e.to_string()) }
}

impl HttpStatusCode for DelegationError {
    fn status_code(&self) -> StatusCode {
        match self {
            DelegationError::Transport(_) | DelegationError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

impl DelegationError {
    pub fn from_generate_tx_error(gen_tx_err: GenerateTxError, coin: String, decimals: u8) -> DelegationError {
        match gen_tx_err {
            GenerateTxError::EmptyUtxoSet { required } => {
                let required = big_decimal_from_sat_unsigned(required, decimals);
                DelegationError::NotSufficientBalance {
                    coin,
                    available: BigDecimal::from(0),
                    required,
                }
            },
            GenerateTxError::EmptyOutputs => DelegationError::InternalError(gen_tx_err.to_string()),
            GenerateTxError::OutputValueLessThanDust { value, dust } => {
                let amount = big_decimal_from_sat_unsigned(value, decimals);
                let threshold = big_decimal_from_sat_unsigned(dust, decimals);
                DelegationError::AmountTooLow { amount, threshold }
            },
            GenerateTxError::DeductFeeFromOutputFailed {
                output_value, required, ..
            } => {
                let available = big_decimal_from_sat_unsigned(output_value, decimals);
                let required = big_decimal_from_sat_unsigned(required, decimals);
                DelegationError::NotSufficientBalance {
                    coin,
                    available,
                    required,
                }
            },
            GenerateTxError::NotEnoughUtxos { sum_utxos, required } => {
                let available = big_decimal_from_sat_unsigned(sum_utxos, decimals);
                let required = big_decimal_from_sat_unsigned(required, decimals);
                DelegationError::NotSufficientBalance {
                    coin,
                    available,
                    required,
                }
            },
            GenerateTxError::Transport(e) => DelegationError::Transport(e),
            GenerateTxError::Internal(e) => DelegationError::InternalError(e),
        }
    }
}

#[derive(Clone, Debug, Display, EnumFromStringify, EnumFromTrait, PartialEq, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum WithdrawError {
    #[display(
        fmt = "'{}' coin doesn't support 'init_withdraw' yet. Consider using 'withdraw' request instead",
        coin
    )]
    CoinDoesntSupportInitWithdraw {
        coin: String,
    },
    #[display(
        fmt = "Not enough {} to withdraw: available {}, required at least {}",
        coin,
        available,
        required
    )]
    NotSufficientBalance {
        coin: String,
        available: BigDecimal,
        required: BigDecimal,
    },
    #[display(
        fmt = "Not enough {} to afford fee. Available {}, required at least {}",
        coin,
        available,
        required
    )]
    NotSufficientPlatformBalanceForFee {
        coin: String,
        available: BigDecimal,
        required: BigDecimal,
    },
    #[display(fmt = "Balance is zero")]
    ZeroBalanceToWithdrawMax,
    #[display(fmt = "The amount {} is too small, required at least {}", amount, threshold)]
    AmountTooLow {
        amount: BigDecimal,
        threshold: BigDecimal,
    },
    #[display(fmt = "Invalid address: {}", _0)]
    InvalidAddress(String),
    #[display(fmt = "Invalid fee policy: {}", _0)]
    InvalidFeePolicy(String),
    #[display(fmt = "Invalid memo field: {}", _0)]
    InvalidMemo(String),
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin {
        coin: String,
    },
    #[from_trait(WithTimeout::timeout)]
    #[display(fmt = "Withdraw timed out {:?}", _0)]
    Timeout(Duration),
    #[display(fmt = "Request should contain a 'from' address/account")]
    FromAddressNotFound,
    #[display(fmt = "Unexpected 'from' address: {}", _0)]
    UnexpectedFromAddress(String),
    #[display(fmt = "Unknown '{}' account", account_id)]
    UnknownAccount {
        account_id: u32,
    },
    #[display(fmt = "RPC 'task' is awaiting '{}' user action", expected)]
    UnexpectedUserAction {
        expected: String,
    },
    #[from_trait(WithHwRpcError::hw_rpc_error)]
    HwError(HwRpcError),
    #[cfg(target_arch = "wasm32")]
    BroadcastExpected(String),
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[from_trait(WithInternal::internal)]
    #[from_stringify("NumConversError", "UnexpectedDerivationMethod", "PrivKeyPolicyNotAllowed")]
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
    #[display(fmt = "Unsupported error: {}", _0)]
    UnsupportedError(String),
    #[display(fmt = "{} coin doesn't support NFT withdrawing", coin)]
    CoinDoesntSupportNftWithdraw {
        coin: String,
    },
    #[display(fmt = "My address {} and from address {} mismatch", my_address, from)]
    AddressMismatchError {
        my_address: String,
        from: String,
    },
    #[display(fmt = "Contract type {} doesnt support 'withdraw_nft' yet", _0)]
    ContractTypeDoesntSupportNftWithdrawing(String),
    #[display(fmt = "Action not allowed for coin: {}", _0)]
    ActionNotAllowed(String),
    GetNftInfoError(GetNftInfoError),
    #[display(
        fmt = "Not enough NFTs amount with token_address: {} and token_id {}. Available {}, required {}",
        token_address,
        token_id,
        available,
        required
    )]
    NotEnoughNftsAmount {
        token_address: String,
        token_id: String,
        available: BigDecimal,
        required: BigDecimal,
    },
    #[display(fmt = "DB error {}", _0)]
    DbError(String),
}

impl HttpStatusCode for WithdrawError {
    fn status_code(&self) -> StatusCode {
        match self {
            WithdrawError::NoSuchCoin { .. } => StatusCode::NOT_FOUND,
            WithdrawError::Timeout(_) => StatusCode::REQUEST_TIMEOUT,
            WithdrawError::CoinDoesntSupportInitWithdraw { .. }
            | WithdrawError::NotSufficientBalance { .. }
            | WithdrawError::NotSufficientPlatformBalanceForFee { .. }
            | WithdrawError::ZeroBalanceToWithdrawMax
            | WithdrawError::AmountTooLow { .. }
            | WithdrawError::InvalidAddress(_)
            | WithdrawError::InvalidFeePolicy(_)
            | WithdrawError::InvalidMemo(_)
            | WithdrawError::FromAddressNotFound
            | WithdrawError::UnexpectedFromAddress(_)
            | WithdrawError::UnknownAccount { .. }
            | WithdrawError::UnexpectedUserAction { .. }
            | WithdrawError::UnsupportedError(_)
            | WithdrawError::ActionNotAllowed(_)
            | WithdrawError::GetNftInfoError(_)
            | WithdrawError::AddressMismatchError { .. }
            | WithdrawError::ContractTypeDoesntSupportNftWithdrawing(_)
            | WithdrawError::CoinDoesntSupportNftWithdraw { .. }
            | WithdrawError::NotEnoughNftsAmount { .. } => StatusCode::BAD_REQUEST,
            WithdrawError::HwError(_) => StatusCode::GONE,
            #[cfg(target_arch = "wasm32")]
            WithdrawError::BroadcastExpected(_) => StatusCode::BAD_REQUEST,
            WithdrawError::Transport(_) | WithdrawError::InternalError(_) | WithdrawError::DbError(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
        }
    }
}

impl From<BalanceError> for WithdrawError {
    fn from(e: BalanceError) -> Self {
        match e {
            BalanceError::Transport(error) | BalanceError::InvalidResponse(error) => WithdrawError::Transport(error),
            BalanceError::UnexpectedDerivationMethod(e) => WithdrawError::from(e),
            e @ BalanceError::WalletStorageError(_) => WithdrawError::InternalError(e.to_string()),
            BalanceError::Internal(internal) => WithdrawError::InternalError(internal),
        }
    }
}

impl From<CoinFindError> for WithdrawError {
    fn from(e: CoinFindError) -> Self {
        match e {
            CoinFindError::NoSuchCoin { coin } => WithdrawError::NoSuchCoin { coin },
        }
    }
}

impl From<UtxoSignWithKeyPairError> for WithdrawError {
    fn from(e: UtxoSignWithKeyPairError) -> Self {
        let error = format!("Error signing: {}", e);
        WithdrawError::InternalError(error)
    }
}

impl From<TimeoutError> for WithdrawError {
    fn from(e: TimeoutError) -> Self { WithdrawError::Timeout(e.duration) }
}

impl From<GetValidEthWithdrawAddError> for WithdrawError {
    fn from(e: GetValidEthWithdrawAddError) -> Self {
        match e {
            GetValidEthWithdrawAddError::AddressMismatchError { my_address, from } => {
                WithdrawError::AddressMismatchError { my_address, from }
            },
            GetValidEthWithdrawAddError::CoinDoesntSupportNftWithdraw { coin } => {
                WithdrawError::CoinDoesntSupportNftWithdraw { coin }
            },
            GetValidEthWithdrawAddError::InvalidAddress(e) => WithdrawError::InvalidAddress(e),
        }
    }
}

impl From<EthGasDetailsErr> for WithdrawError {
    fn from(e: EthGasDetailsErr) -> Self {
        match e {
            EthGasDetailsErr::InvalidFeePolicy(e) => WithdrawError::InvalidFeePolicy(e),
            EthGasDetailsErr::Internal(e) => WithdrawError::InternalError(e),
            EthGasDetailsErr::Transport(e) => WithdrawError::Transport(e),
        }
    }
}

impl From<Bip32Error> for WithdrawError {
    fn from(e: Bip32Error) -> Self {
        let error = format!("Error deriving key: {}", e);
        WithdrawError::InternalError(error)
    }
}

impl WithdrawError {
    /// Construct [`WithdrawError`] from [`GenerateTxError`] using additional `coin` and `decimals`.
    pub fn from_generate_tx_error(gen_tx_err: GenerateTxError, coin: String, decimals: u8) -> WithdrawError {
        match gen_tx_err {
            GenerateTxError::EmptyUtxoSet { required } => {
                let required = big_decimal_from_sat_unsigned(required, decimals);
                WithdrawError::NotSufficientBalance {
                    coin,
                    available: BigDecimal::from(0),
                    required,
                }
            },
            GenerateTxError::EmptyOutputs => WithdrawError::InternalError(gen_tx_err.to_string()),
            GenerateTxError::OutputValueLessThanDust { value, dust } => {
                let amount = big_decimal_from_sat_unsigned(value, decimals);
                let threshold = big_decimal_from_sat_unsigned(dust, decimals);
                WithdrawError::AmountTooLow { amount, threshold }
            },
            GenerateTxError::DeductFeeFromOutputFailed {
                output_value, required, ..
            } => {
                let available = big_decimal_from_sat_unsigned(output_value, decimals);
                let required = big_decimal_from_sat_unsigned(required, decimals);
                WithdrawError::NotSufficientBalance {
                    coin,
                    available,
                    required,
                }
            },
            GenerateTxError::NotEnoughUtxos { sum_utxos, required } => {
                let available = big_decimal_from_sat_unsigned(sum_utxos, decimals);
                let required = big_decimal_from_sat_unsigned(required, decimals);
                WithdrawError::NotSufficientBalance {
                    coin,
                    available,
                    required,
                }
            },
            GenerateTxError::Transport(e) => WithdrawError::Transport(e),
            GenerateTxError::Internal(e) => WithdrawError::InternalError(e),
        }
    }
}

#[derive(Debug, Display, EnumFromStringify, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum SignatureError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[from_stringify("CoinFindError", "ethkey::Error", "keys::Error", "PrivKeyPolicyNotAllowed")]
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
    #[display(fmt = "Coin is not found: {}", _0)]
    CoinIsNotFound(String),
    #[display(fmt = "sign_message_prefix is not set in coin config")]
    PrefixNotFound,
}

impl HttpStatusCode for SignatureError {
    fn status_code(&self) -> StatusCode {
        match self {
            SignatureError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            SignatureError::CoinIsNotFound(_) => StatusCode::BAD_REQUEST,
            SignatureError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            SignatureError::PrefixNotFound => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Debug, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum VerificationError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
    #[display(fmt = "Signature decoding error: {}", _0)]
    SignatureDecodingError(String),
    #[display(fmt = "Address decoding error: {}", _0)]
    AddressDecodingError(String),
    #[display(fmt = "Coin is not found: {}", _0)]
    CoinIsNotFound(String),
    #[display(fmt = "sign_message_prefix is not set in coin config")]
    PrefixNotFound,
}

impl HttpStatusCode for VerificationError {
    fn status_code(&self) -> StatusCode {
        match self {
            VerificationError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            VerificationError::SignatureDecodingError(_) => StatusCode::BAD_REQUEST,
            VerificationError::AddressDecodingError(_) => StatusCode::BAD_REQUEST,
            VerificationError::CoinIsNotFound(_) => StatusCode::BAD_REQUEST,
            VerificationError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            VerificationError::PrefixNotFound => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<base64::DecodeError> for VerificationError {
    fn from(e: base64::DecodeError) -> Self { VerificationError::SignatureDecodingError(e.to_string()) }
}

impl From<hex::FromHexError> for VerificationError {
    fn from(e: hex::FromHexError) -> Self { VerificationError::AddressDecodingError(e.to_string()) }
}

impl From<FromBase58Error> for VerificationError {
    fn from(e: FromBase58Error) -> Self {
        match e {
            FromBase58Error::InvalidBase58Character(c, _) => {
                VerificationError::AddressDecodingError(format!("Invalid Base58 Character: {}", c))
            },
            FromBase58Error::InvalidBase58Length => {
                VerificationError::AddressDecodingError(String::from("Invalid Base58 Length"))
            },
        }
    }
}

impl From<keys::Error> for VerificationError {
    fn from(e: keys::Error) -> Self { VerificationError::InternalError(e.to_string()) }
}

impl From<ethkey::Error> for VerificationError {
    fn from(e: ethkey::Error) -> Self { VerificationError::InternalError(e.to_string()) }
}

impl From<CoinFindError> for VerificationError {
    fn from(e: CoinFindError) -> Self { VerificationError::CoinIsNotFound(e.to_string()) }
}

/// NB: Implementations are expected to follow the pImpl idiom, providing cheap reference-counted cloning and garbage collection.
#[async_trait]
pub trait MmCoin:
    SwapOps + TakerSwapMakerCoin + MakerSwapTakerCoin + WatcherOps + MarketCoinOps + Send + Sync + 'static
{
    // `MmCoin` is an extension fulcrum for something that doesn't fit the `MarketCoinOps`. Practical examples:
    // name (might be required for some APIs, CoinMarketCap for instance);
    // coin statistics that we might want to share with UI;
    // state serialization, to get full rewind and debugging information about the coins participating in a SWAP operation.
    // status/availability check: https://github.com/artemii235/SuperNET/issues/156#issuecomment-446501816

    fn is_asset_chain(&self) -> bool;

    /// The coin can be initialized, but it cannot participate in the swaps.
    fn wallet_only(&self, ctx: &MmArc) -> bool {
        let coin_conf = coin_conf(ctx, self.ticker());
        coin_conf["wallet_only"].as_bool().unwrap_or(false)
    }

    /// Returns a spawner pinned to the coin.
    ///
    /// # Note
    ///
    /// `CoinFutSpawner` doesn't prevent the spawned futures from being aborted.
    fn spawner(&self) -> CoinFutSpawner;

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut;

    fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut;

    fn get_tx_hex_by_hash(&self, tx_hash: Vec<u8>) -> RawTransactionFut;

    /// Maximum number of digits after decimal point used to denominate integer coin units (satoshis, wei, etc.)
    fn decimals(&self) -> u8;

    /// Convert input address to the specified address format.
    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String>;

    fn validate_address(&self, address: &str) -> ValidateAddressResult;

    /// Loop collecting coin transaction history and saving it to local DB
    fn process_history_loop(&self, ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send>;

    /// Path to tx history file
    #[cfg(not(target_arch = "wasm32"))]
    fn tx_history_path(&self, ctx: &MmArc) -> PathBuf {
        let my_address = self.my_address().unwrap_or_default();
        // BCH cash address format has colon after prefix, e.g. bitcoincash:
        // Colon can't be used in file names on Windows so it should be escaped
        let my_address = my_address.replace(':', "_");
        ctx.dbdir()
            .join("TRANSACTIONS")
            .join(format!("{}_{}.json", self.ticker(), my_address))
    }

    /// Path to tx history migration file
    #[cfg(not(target_arch = "wasm32"))]
    fn tx_migration_path(&self, ctx: &MmArc) -> PathBuf {
        let my_address = self.my_address().unwrap_or_default();
        // BCH cash address format has colon after prefix, e.g. bitcoincash:
        // Colon can't be used in file names on Windows so it should be escaped
        let my_address = my_address.replace(':', "_");
        ctx.dbdir()
            .join("TRANSACTIONS")
            .join(format!("{}_{}_migration", self.ticker(), my_address))
    }

    /// Loads existing tx history from file, returns empty vector if file is not found
    /// Cleans the existing file if deserialization fails
    fn load_history_from_file(&self, ctx: &MmArc) -> TxHistoryFut<Vec<TransactionDetails>> {
        load_history_from_file_impl(self, ctx)
    }

    fn save_history_to_file(&self, ctx: &MmArc, history: Vec<TransactionDetails>) -> TxHistoryFut<()> {
        save_history_to_file_impl(self, ctx, history)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn get_tx_history_migration(&self, ctx: &MmArc) -> TxHistoryFut<u64> { get_tx_history_migration_impl(self, ctx) }

    #[cfg(not(target_arch = "wasm32"))]
    fn update_migration_file(&self, ctx: &MmArc, migration_number: u64) -> TxHistoryFut<()> {
        update_migration_file_impl(self, ctx, migration_number)
    }

    /// Transaction history background sync status
    fn history_sync_status(&self) -> HistorySyncState;

    /// Get fee to be paid per 1 swap transaction
    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send>;

    /// Get fee to be paid by sender per whole swap using the sending value and check if the wallet has sufficient balance to pay the fee.
    async fn get_sender_trade_fee(
        &self,
        value: TradePreimageValue,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee>;

    /// Get fee to be paid by receiver per whole swap and check if the wallet has sufficient balance to pay the fee.
    fn get_receiver_trade_fee(&self, stage: FeeApproxStage) -> TradePreimageFut<TradeFee>;

    /// Get transaction fee the Taker has to pay to send a `TakerFee` transaction and check if the wallet has sufficient balance to pay the fee.
    async fn get_fee_to_send_taker_fee(
        &self,
        dex_fee_amount: DexFee,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee>;

    /// required transaction confirmations number to ensure double-spend safety
    fn required_confirmations(&self) -> u64;

    /// whether coin requires notarization to ensure double-spend safety
    fn requires_notarization(&self) -> bool;

    /// set required transaction confirmations number
    fn set_required_confirmations(&self, confirmations: u64);

    /// set requires notarization
    fn set_requires_notarization(&self, requires_nota: bool);

    /// Get swap contract address if the coin uses it in Atomic Swaps.
    fn swap_contract_address(&self) -> Option<BytesJson>;

    /// Get fallback swap contract address if the coin uses it in Atomic Swaps.
    fn fallback_swap_contract(&self) -> Option<BytesJson>;

    /// The minimum number of confirmations at which a transaction is considered mature.
    fn mature_confirmations(&self) -> Option<u32>;

    /// Get some of the coin protocol related info in serialized format for p2p messaging.
    fn coin_protocol_info(&self, amount_to_receive: Option<MmNumber>) -> Vec<u8>;

    /// Check if serialized coin protocol info is supported by current version.
    /// Can also be used to check if orders can be matched or not.
    fn is_coin_protocol_supported(
        &self,
        info: &Option<Vec<u8>>,
        amount_to_send: Option<MmNumber>,
        locktime: u64,
        is_maker: bool,
    ) -> bool;

    /// Abort all coin related futures on coin deactivation.
    fn on_disabled(&self) -> Result<(), AbortedError>;

    /// For Handling the removal/deactivation of token on platform coin deactivation.
    fn on_token_deactivated(&self, ticker: &str);
}

/// The coin futures spawner. It's used to spawn futures that can be aborted immediately or after a timeout
/// on the the coin deactivation.
///
/// # Note
///
/// `CoinFutSpawner` doesn't prevent the spawned futures from being aborted.
#[derive(Clone)]
pub struct CoinFutSpawner {
    inner: WeakSpawner,
}

impl CoinFutSpawner {
    pub fn new(system: &AbortableQueue) -> CoinFutSpawner {
        CoinFutSpawner {
            inner: system.weak_spawner(),
        }
    }
}

impl SpawnFuture for CoinFutSpawner {
    fn spawn<F>(&self, f: F)
    where
        F: Future03<Output = ()> + Send + 'static,
    {
        self.inner.spawn(f)
    }
}

impl SpawnAbortable for CoinFutSpawner {
    fn spawn_with_settings<F>(&self, fut: F, settings: AbortSettings)
    where
        F: Future03<Output = ()> + Send + 'static,
    {
        self.inner.spawn_with_settings(fut, settings)
    }
}

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub enum MmCoinEnum {
    UtxoCoin(UtxoStandardCoin),
    QtumCoin(QtumCoin),
    Qrc20Coin(Qrc20Coin),
    EthCoin(EthCoin),
    ZCoin(ZCoin),
    Bch(BchCoin),
    SlpToken(SlpToken),
    Tendermint(TendermintCoin),
    TendermintToken(TendermintToken),
    #[cfg(all(
        feature = "enable-solana",
        not(target_os = "ios"),
        not(target_os = "android"),
        not(target_arch = "wasm32")
    ))]
    SolanaCoin(SolanaCoin),
    #[cfg(all(
        feature = "enable-solana",
        not(target_os = "ios"),
        not(target_os = "android"),
        not(target_arch = "wasm32")
    ))]
    SplToken(SplToken),
    #[cfg(not(target_arch = "wasm32"))]
    LightningCoin(LightningCoin),
    Test(TestCoin),
}

impl From<UtxoStandardCoin> for MmCoinEnum {
    fn from(c: UtxoStandardCoin) -> MmCoinEnum { MmCoinEnum::UtxoCoin(c) }
}

impl From<EthCoin> for MmCoinEnum {
    fn from(c: EthCoin) -> MmCoinEnum { MmCoinEnum::EthCoin(c) }
}

impl From<TestCoin> for MmCoinEnum {
    fn from(c: TestCoin) -> MmCoinEnum { MmCoinEnum::Test(c) }
}

#[cfg(all(
    feature = "enable-solana",
    not(target_os = "ios"),
    not(target_os = "android"),
    not(target_arch = "wasm32")
))]
impl From<SolanaCoin> for MmCoinEnum {
    fn from(c: SolanaCoin) -> MmCoinEnum { MmCoinEnum::SolanaCoin(c) }
}

#[cfg(all(
    feature = "enable-solana",
    not(target_os = "ios"),
    not(target_os = "android"),
    not(target_arch = "wasm32")
))]
impl From<SplToken> for MmCoinEnum {
    fn from(c: SplToken) -> MmCoinEnum { MmCoinEnum::SplToken(c) }
}

impl From<QtumCoin> for MmCoinEnum {
    fn from(coin: QtumCoin) -> Self { MmCoinEnum::QtumCoin(coin) }
}

impl From<Qrc20Coin> for MmCoinEnum {
    fn from(c: Qrc20Coin) -> MmCoinEnum { MmCoinEnum::Qrc20Coin(c) }
}

impl From<BchCoin> for MmCoinEnum {
    fn from(c: BchCoin) -> MmCoinEnum { MmCoinEnum::Bch(c) }
}

impl From<SlpToken> for MmCoinEnum {
    fn from(c: SlpToken) -> MmCoinEnum { MmCoinEnum::SlpToken(c) }
}

impl From<TendermintCoin> for MmCoinEnum {
    fn from(c: TendermintCoin) -> Self { MmCoinEnum::Tendermint(c) }
}

impl From<TendermintToken> for MmCoinEnum {
    fn from(c: TendermintToken) -> Self { MmCoinEnum::TendermintToken(c) }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<LightningCoin> for MmCoinEnum {
    fn from(c: LightningCoin) -> MmCoinEnum { MmCoinEnum::LightningCoin(c) }
}

impl From<ZCoin> for MmCoinEnum {
    fn from(c: ZCoin) -> MmCoinEnum { MmCoinEnum::ZCoin(c) }
}

// NB: When stable and groked by IDEs, `enum_dispatch` can be used instead of `Deref` to speed things up.
impl Deref for MmCoinEnum {
    type Target = dyn MmCoin;
    fn deref(&self) -> &dyn MmCoin {
        match self {
            MmCoinEnum::UtxoCoin(ref c) => c,
            MmCoinEnum::QtumCoin(ref c) => c,
            MmCoinEnum::Qrc20Coin(ref c) => c,
            MmCoinEnum::EthCoin(ref c) => c,
            MmCoinEnum::Bch(ref c) => c,
            MmCoinEnum::SlpToken(ref c) => c,
            MmCoinEnum::Tendermint(ref c) => c,
            MmCoinEnum::TendermintToken(ref c) => c,
            #[cfg(not(target_arch = "wasm32"))]
            MmCoinEnum::LightningCoin(ref c) => c,
            MmCoinEnum::ZCoin(ref c) => c,
            MmCoinEnum::Test(ref c) => c,
            #[cfg(all(
                feature = "enable-solana",
                not(target_os = "ios"),
                not(target_os = "android"),
                not(target_arch = "wasm32")
            ))]
            MmCoinEnum::SolanaCoin(ref c) => c,
            #[cfg(all(
                feature = "enable-solana",
                not(target_os = "ios"),
                not(target_os = "android"),
                not(target_arch = "wasm32")
            ))]
            MmCoinEnum::SplToken(ref c) => c,
        }
    }
}

impl MmCoinEnum {
    pub fn is_utxo_in_native_mode(&self) -> bool {
        match self {
            MmCoinEnum::UtxoCoin(ref c) => c.as_ref().rpc_client.is_native(),
            MmCoinEnum::QtumCoin(ref c) => c.as_ref().rpc_client.is_native(),
            MmCoinEnum::Qrc20Coin(ref c) => c.as_ref().rpc_client.is_native(),
            MmCoinEnum::Bch(ref c) => c.as_ref().rpc_client.is_native(),
            MmCoinEnum::SlpToken(ref c) => c.as_ref().rpc_client.is_native(),
            #[cfg(all(not(target_arch = "wasm32"), feature = "zhtlc"))]
            MmCoinEnum::ZCoin(ref c) => c.as_ref().rpc_client.is_native(),
            _ => false,
        }
    }

    pub fn is_eth(&self) -> bool { matches!(self, MmCoinEnum::EthCoin(_)) }

    fn is_platform_coin(&self) -> bool { self.ticker() == self.platform_ticker() }
}

#[async_trait]
pub trait BalanceTradeFeeUpdatedHandler {
    async fn balance_updated(&self, coin: &MmCoinEnum, new_balance: &BigDecimal);
}

#[derive(Clone)]
pub struct MmCoinStruct {
    pub inner: MmCoinEnum,
    is_available: Arc<AtomicBool>,
}

impl MmCoinStruct {
    fn new(coin: MmCoinEnum) -> Self {
        Self {
            inner: coin,
            is_available: AtomicBool::new(true).into(),
        }
    }

    /// Gets the current state of the parent coin whether
    /// it's available for the external requests or not.
    ///
    /// Always `true` for child tokens.
    pub fn is_available(&self) -> bool {
        !self.inner.is_platform_coin() // Tokens are always active or disabled
            || self.is_available.load(AtomicOrdering::SeqCst)
    }

    /// Makes the coin disabled to the external requests.
    /// Useful for executing `disable_coin` on parent coins
    /// that have child tokens enabled.
    ///
    /// Ineffective for child tokens.
    pub fn update_is_available(&self, to: bool) {
        if !self.inner.is_platform_coin() {
            warn!(
                "`update_is_available` is ineffective for tokens. Current token: {}",
                self.inner.ticker()
            );
            return;
        }

        self.is_available.store(to, AtomicOrdering::SeqCst);
    }
}

/// Represents the different types of DEX fees.
#[derive(Clone, Debug, PartialEq)]
pub enum DexFee {
    /// Standard dex fee which will be sent to the dex fee address
    Standard(MmNumber),
    /// Dex fee with the burn amount.
    ///   - `fee_amount` goes to the dex fee address.
    ///   - `burn_amount` will be added as `OP_RETURN` output in the dex fee transaction.
    WithBurn {
        fee_amount: MmNumber,
        burn_amount: MmNumber,
    },
}

impl DexFee {
    /// Creates a new `DexFee` with burn amounts.
    pub fn with_burn(fee_amount: MmNumber, burn_amount: MmNumber) -> DexFee {
        DexFee::WithBurn {
            fee_amount,
            burn_amount,
        }
    }

    /// Gets the fee amount associated with the dex fee.
    pub fn fee_amount(&self) -> MmNumber {
        match self {
            DexFee::Standard(t) => t.clone(),
            DexFee::WithBurn { fee_amount, .. } => fee_amount.clone(),
        }
    }

    /// Gets the burn amount associated with the dex fee, if applicable.
    pub fn burn_amount(&self) -> Option<MmNumber> {
        match self {
            DexFee::Standard(_) => None,
            DexFee::WithBurn { burn_amount, .. } => Some(burn_amount.clone()),
        }
    }

    /// Calculates the total spend amount, considering both the fee and burn amounts.
    pub fn total_spend_amount(&self) -> MmNumber {
        match self {
            DexFee::Standard(t) => t.clone(),
            DexFee::WithBurn {
                fee_amount,
                burn_amount,
            } => fee_amount + burn_amount,
        }
    }

    /// Converts the fee amount to micro-units based on the specified decimal places.
    pub fn fee_uamount(&self, decimals: u8) -> NumConversResult<u64> {
        let fee_amount = self.fee_amount();
        utxo::sat_from_big_decimal(&fee_amount.into(), decimals)
    }

    /// Converts the burn amount to micro-units, if applicable, based on the specified decimal places.
    pub fn burn_uamount(&self, decimals: u8) -> NumConversResult<Option<u64>> {
        if let Some(burn_amount) = self.burn_amount() {
            Ok(Some(utxo::sat_from_big_decimal(&burn_amount.into(), decimals)?))
        } else {
            Ok(None)
        }
    }
}

pub struct CoinsContext {
    /// A map from a currency ticker symbol to the corresponding coin.
    /// Similar to `LP_coins`.
    coins: AsyncMutex<HashMap<String, MmCoinStruct>>,
    balance_update_handlers: AsyncMutex<Vec<Box<dyn BalanceTradeFeeUpdatedHandler + Send + Sync>>>,
    account_balance_task_manager: AccountBalanceTaskManagerShared,
    create_account_manager: CreateAccountTaskManagerShared,
    get_new_address_manager: GetNewAddressTaskManagerShared,
    platform_coin_tokens: PaMutex<HashMap<String, HashSet<String>>>,
    scan_addresses_manager: ScanAddressesTaskManagerShared,
    withdraw_task_manager: WithdrawTaskManagerShared,
    #[cfg(target_arch = "wasm32")]
    tx_history_db: SharedDb<TxHistoryDb>,
    #[cfg(target_arch = "wasm32")]
    hd_wallet_db: SharedDb<HDWalletDb>,
}

#[derive(Debug)]
pub struct PlatformIsAlreadyActivatedErr {
    pub ticker: String,
}

impl CoinsContext {
    /// Obtains a reference to this crate context, creating it if necessary.
    pub fn from_ctx(ctx: &MmArc) -> Result<Arc<CoinsContext>, String> {
        Ok(try_s!(from_ctx(&ctx.coins_ctx, move || {
            Ok(CoinsContext {
                platform_coin_tokens: PaMutex::new(HashMap::new()),
                coins: AsyncMutex::new(HashMap::new()),
                balance_update_handlers: AsyncMutex::new(vec![]),
                account_balance_task_manager: AccountBalanceTaskManager::new_shared(),
                create_account_manager: CreateAccountTaskManager::new_shared(),
                get_new_address_manager: GetNewAddressTaskManager::new_shared(),
                scan_addresses_manager: ScanAddressesTaskManager::new_shared(),
                withdraw_task_manager: WithdrawTaskManager::new_shared(),
                #[cfg(target_arch = "wasm32")]
                tx_history_db: ConstructibleDb::new(ctx).into_shared(),
                #[cfg(target_arch = "wasm32")]
                hd_wallet_db: ConstructibleDb::new_shared_db(ctx).into_shared(),
            })
        })))
    }

    pub async fn add_token(&self, coin: MmCoinEnum) -> Result<(), MmError<RegisterCoinError>> {
        let mut coins = self.coins.lock().await;
        if coins.contains_key(coin.ticker()) {
            return MmError::err(RegisterCoinError::CoinIsInitializedAlready {
                coin: coin.ticker().into(),
            });
        }

        let ticker = coin.ticker();

        let mut platform_coin_tokens = self.platform_coin_tokens.lock();
        // Here, we try to add a token to platform_coin_tokens if the token belongs to a platform coin.
        if let Some(platform) = platform_coin_tokens.get_mut(coin.platform_ticker()) {
            platform.insert(ticker.to_owned());
        }

        coins.insert(ticker.into(), MmCoinStruct::new(coin));

        Ok(())
    }

    /// Adds a Layer 2 coin that depends on a standalone platform.
    /// The process of adding l2 coins is identical to that of adding tokens.
    pub async fn add_l2(&self, coin: MmCoinEnum) -> Result<(), MmError<RegisterCoinError>> {
        self.add_token(coin).await
    }

    pub async fn add_platform_with_tokens(
        &self,
        platform: MmCoinEnum,
        tokens: Vec<MmCoinEnum>,
    ) -> Result<(), MmError<PlatformIsAlreadyActivatedErr>> {
        let mut coins = self.coins.lock().await;
        let mut platform_coin_tokens = self.platform_coin_tokens.lock();

        let platform_ticker = platform.ticker().to_owned();

        if let Some(coin) = coins.get(&platform_ticker) {
            if coin.is_available() {
                return MmError::err(PlatformIsAlreadyActivatedErr {
                    ticker: platform.ticker().into(),
                });
            }

            coin.update_is_available(true);
        } else {
            coins.insert(platform_ticker.clone(), MmCoinStruct::new(platform));
        }

        // Tokens can't be activated without platform coin so we can safely insert them without checking prior existence
        let mut token_tickers = HashSet::with_capacity(tokens.len());
        // TODO
        // Handling for these case:
        // USDT was activated via enable RPC
        // We try to activate ETH coin and USDT token via enable_eth_with_tokens
        for token in tokens {
            token_tickers.insert(token.ticker().to_string());
            coins
                .entry(token.ticker().into())
                .or_insert_with(|| MmCoinStruct::new(token));
        }

        platform_coin_tokens
            .entry(platform_ticker)
            .or_default()
            .extend(token_tickers);
        Ok(())
    }

    /// If `ticker` is a platform coin, returns tokens dependent on it.
    pub async fn get_dependent_tokens(&self, ticker: &str) -> HashSet<String> {
        let coins = self.platform_coin_tokens.lock();
        coins.get(ticker).cloned().unwrap_or_default()
    }

    pub async fn remove_coin(&self, coin: MmCoinEnum) {
        let ticker = coin.ticker();
        let platform_ticker = coin.platform_ticker();
        let mut coins_storage = self.coins.lock().await;
        let mut platform_tokens_storage = self.platform_coin_tokens.lock();

        // Check if ticker is a platform coin and remove from it platform's token list
        if ticker == platform_ticker {
            if let Some(tokens_to_remove) = platform_tokens_storage.remove(ticker) {
                tokens_to_remove.iter().for_each(|token| {
                    if let Some(token) = coins_storage.remove(token) {
                        // Abort all token related futures on token deactivation
                        token
                            .inner
                            .on_disabled()
                            .error_log_with_msg(&format!("Error aborting coin({ticker}) futures"));
                    }
                });
            };
        } else {
            if let Some(tokens) = platform_tokens_storage.get_mut(platform_ticker) {
                tokens.remove(ticker);
            }
            if let Some(platform_coin) = coins_storage.get(platform_ticker) {
                platform_coin.inner.on_token_deactivated(ticker);
            }
        };

        //  Remove coin from coin list
        coins_storage
            .remove(ticker)
            .ok_or(format!("{} is disabled already", ticker))
            .error_log();

        // Abort all coin related futures on coin deactivation
        coin.on_disabled()
            .error_log_with_msg(&format!("Error aborting coin({ticker}) futures"));
    }

    #[cfg(target_arch = "wasm32")]
    async fn tx_history_db(&self) -> TxHistoryResult<TxHistoryDbLocked<'_>> {
        Ok(self.tx_history_db.get_or_initialize().await?)
    }
}

/// This enum is used in coin activation requests.
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum PrivKeyActivationPolicy {
    ContextPrivKey,
    Trezor,
}

impl Default for PrivKeyActivationPolicy {
    fn default() -> Self { PrivKeyActivationPolicy::ContextPrivKey }
}

#[derive(Clone, Debug)]
pub enum PrivKeyPolicy<T> {
    Iguana(T),
    HDWallet {
        /// Derivation path of the coin.
        /// This derivation path consists of `purpose` and `coin_type` only
        /// where the full `BIP44` address has the following structure:
        /// `m/purpose'/coin_type'/account'/change/address_index`.
        derivation_path: StandardHDPathToCoin,
        activated_key: T,
        bip39_secp_priv_key: ExtendedPrivateKey<secp256k1::SecretKey>,
    },
    Trezor,
    #[cfg(target_arch = "wasm32")]
    Metamask(EthMetamaskPolicy),
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug)]
pub struct EthMetamaskPolicy {
    pub(crate) public_key: EthH264,
    pub(crate) public_key_uncompressed: EthH520,
}

impl<T> From<T> for PrivKeyPolicy<T> {
    fn from(key_pair: T) -> Self { PrivKeyPolicy::Iguana(key_pair) }
}

impl<T> PrivKeyPolicy<T> {
    fn activated_key(&self) -> Option<&T> {
        match self {
            PrivKeyPolicy::Iguana(key_pair) => Some(key_pair),
            PrivKeyPolicy::HDWallet {
                activated_key: activated_key_pair,
                ..
            } => Some(activated_key_pair),
            PrivKeyPolicy::Trezor => None,
            #[cfg(target_arch = "wasm32")]
            PrivKeyPolicy::Metamask(_) => None,
        }
    }

    fn activated_key_or_err(&self) -> Result<&T, MmError<PrivKeyPolicyNotAllowed>> {
        self.activated_key().or_mm_err(|| {
            PrivKeyPolicyNotAllowed::UnsupportedMethod(
                "`activated_key_or_err` is supported only for `PrivKeyPolicy::KeyPair` or `PrivKeyPolicy::HDWallet`"
                    .to_string(),
            )
        })
    }

    fn bip39_secp_priv_key(&self) -> Option<&ExtendedPrivateKey<secp256k1::SecretKey>> {
        match self {
            PrivKeyPolicy::HDWallet {
                bip39_secp_priv_key, ..
            } => Some(bip39_secp_priv_key),
            PrivKeyPolicy::Iguana(_) | PrivKeyPolicy::Trezor => None,
            #[cfg(target_arch = "wasm32")]
            PrivKeyPolicy::Metamask(_) => None,
        }
    }

    fn bip39_secp_priv_key_or_err(
        &self,
    ) -> Result<&ExtendedPrivateKey<secp256k1::SecretKey>, MmError<PrivKeyPolicyNotAllowed>> {
        self.bip39_secp_priv_key().or_mm_err(|| {
            PrivKeyPolicyNotAllowed::UnsupportedMethod(
                "`bip39_secp_priv_key_or_err` is supported only for `PrivKeyPolicy::HDWallet`".to_string(),
            )
        })
    }

    fn derivation_path(&self) -> Option<&StandardHDPathToCoin> {
        match self {
            PrivKeyPolicy::HDWallet { derivation_path, .. } => Some(derivation_path),
            PrivKeyPolicy::Iguana(_) | PrivKeyPolicy::Trezor => None,
            #[cfg(target_arch = "wasm32")]
            PrivKeyPolicy::Metamask(_) => None,
        }
    }

    fn derivation_path_or_err(&self) -> Result<&StandardHDPathToCoin, MmError<PrivKeyPolicyNotAllowed>> {
        self.derivation_path().or_mm_err(|| {
            PrivKeyPolicyNotAllowed::UnsupportedMethod(
                "`derivation_path_or_err` is supported only for `PrivKeyPolicy::HDWallet`".to_string(),
            )
        })
    }

    fn hd_wallet_derived_priv_key_or_err(
        &self,
        path_to_address: &StandardHDCoinAddress,
    ) -> Result<Secp256k1Secret, MmError<PrivKeyPolicyNotAllowed>> {
        let bip39_secp_priv_key = self.bip39_secp_priv_key_or_err()?;
        let derivation_path = self.derivation_path_or_err()?;
        derive_secp256k1_secret(bip39_secp_priv_key.clone(), derivation_path, path_to_address)
            .mm_err(|e| PrivKeyPolicyNotAllowed::InternalError(e.to_string()))
    }
}

#[derive(Clone)]
pub enum PrivKeyBuildPolicy {
    IguanaPrivKey(IguanaPrivKey),
    GlobalHDAccount(GlobalHDAccountArc),
    Trezor,
}

impl PrivKeyBuildPolicy {
    /// Detects the `PrivKeyBuildPolicy` with which the given `MmArc` is initialized.
    pub fn detect_priv_key_policy(ctx: &MmArc) -> MmResult<PrivKeyBuildPolicy, CryptoCtxError> {
        let crypto_ctx = CryptoCtx::from_ctx(ctx)?;

        match crypto_ctx.key_pair_policy() {
            // Use an internal private key as the coin secret.
            KeyPairPolicy::Iguana => Ok(PrivKeyBuildPolicy::IguanaPrivKey(
                crypto_ctx.mm2_internal_privkey_secret(),
            )),
            KeyPairPolicy::GlobalHDAccount(global_hd) => Ok(PrivKeyBuildPolicy::GlobalHDAccount(global_hd.clone())),
        }
    }
}

#[derive(Debug)]
pub enum DerivationMethod<Address, HDWallet> {
    SingleAddress(Address),
    HDWallet(HDWallet),
}

impl<Address, HDWallet> DerivationMethod<Address, HDWallet> {
    pub fn single_addr(&self) -> Option<&Address> {
        match self {
            DerivationMethod::SingleAddress(my_address) => Some(my_address),
            DerivationMethod::HDWallet(_) => None,
        }
    }

    pub fn single_addr_or_err(&self) -> MmResult<&Address, UnexpectedDerivationMethod> {
        self.single_addr()
            .or_mm_err(|| UnexpectedDerivationMethod::ExpectedSingleAddress)
    }

    pub fn hd_wallet(&self) -> Option<&HDWallet> {
        match self {
            DerivationMethod::SingleAddress(_) => None,
            DerivationMethod::HDWallet(hd_wallet) => Some(hd_wallet),
        }
    }

    pub fn hd_wallet_or_err(&self) -> MmResult<&HDWallet, UnexpectedDerivationMethod> {
        self.hd_wallet()
            .or_mm_err(|| UnexpectedDerivationMethod::ExpectedHDWallet)
    }

    /// # Panic
    ///
    /// Panic if the address mode is [`DerivationMethod::HDWallet`].
    pub fn unwrap_single_addr(&self) -> &Address { self.single_addr_or_err().unwrap() }
}

#[async_trait]
pub trait CoinWithDerivationMethod {
    type Address;
    type HDWallet;

    fn derivation_method(&self) -> &DerivationMethod<Self::Address, Self::HDWallet>;

    fn has_hd_wallet_derivation_method(&self) -> bool {
        matches!(self.derivation_method(), DerivationMethod::HDWallet(_))
    }
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", content = "protocol_data")]
pub enum CoinProtocol {
    UTXO,
    QTUM,
    QRC20 {
        platform: String,
        contract_address: String,
    },
    ETH,
    ERC20 {
        platform: String,
        contract_address: String,
    },
    SLPTOKEN {
        platform: String,
        token_id: H256Json,
        decimals: u8,
        required_confirmations: Option<u64>,
    },
    BCH {
        slp_prefix: String,
    },
    TENDERMINT(TendermintProtocolInfo),
    TENDERMINTTOKEN(TendermintTokenProtocolInfo),
    #[cfg(not(target_arch = "wasm32"))]
    LIGHTNING {
        platform: String,
        network: BlockchainNetwork,
        confirmation_targets: PlatformCoinConfirmationTargets,
    },
    #[cfg(all(feature = "enable-solana", not(target_arch = "wasm32")))]
    SOLANA,
    #[cfg(all(feature = "enable-solana", not(target_arch = "wasm32")))]
    SPLTOKEN {
        platform: String,
        token_contract_address: String,
        decimals: u8,
    },
    ZHTLC(ZcoinProtocolInfo),
}

pub type RpcTransportEventHandlerShared = Arc<dyn RpcTransportEventHandler + Send + Sync + 'static>;

/// Common methods to measure the outgoing requests and incoming responses statistics.
pub trait RpcTransportEventHandler {
    fn debug_info(&self) -> String;

    fn on_outgoing_request(&self, data: &[u8]);

    fn on_incoming_response(&self, data: &[u8]);

    fn on_connected(&self, address: String) -> Result<(), String>;

    fn on_disconnected(&self, address: String) -> Result<(), String>;
}

impl fmt::Debug for dyn RpcTransportEventHandler + Send + Sync {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.debug_info()) }
}

impl RpcTransportEventHandler for RpcTransportEventHandlerShared {
    fn debug_info(&self) -> String { self.deref().debug_info() }

    fn on_outgoing_request(&self, data: &[u8]) { self.as_ref().on_outgoing_request(data) }

    fn on_incoming_response(&self, data: &[u8]) { self.as_ref().on_incoming_response(data) }

    fn on_connected(&self, address: String) -> Result<(), String> { self.as_ref().on_connected(address) }

    fn on_disconnected(&self, address: String) -> Result<(), String> { self.as_ref().on_disconnected(address) }
}

impl<T: RpcTransportEventHandler> RpcTransportEventHandler for Vec<T> {
    fn debug_info(&self) -> String {
        let selfi: Vec<String> = self.iter().map(|x| x.debug_info()).collect();
        format!("{:?}", selfi)
    }

    fn on_outgoing_request(&self, data: &[u8]) {
        for handler in self {
            handler.on_outgoing_request(data)
        }
    }

    fn on_incoming_response(&self, data: &[u8]) {
        for handler in self {
            handler.on_incoming_response(data)
        }
    }

    fn on_connected(&self, address: String) -> Result<(), String> {
        for handler in self {
            try_s!(handler.on_connected(address.clone()))
        }
        Ok(())
    }

    fn on_disconnected(&self, address: String) -> Result<(), String> {
        for handler in self {
            try_s!(handler.on_disconnected(address.clone()))
        }
        Ok(())
    }
}

pub enum RpcClientType {
    Native,
    Electrum,
    Ethereum,
}

impl ToString for RpcClientType {
    fn to_string(&self) -> String {
        match self {
            RpcClientType::Native => "native".into(),
            RpcClientType::Electrum => "electrum".into(),
            RpcClientType::Ethereum => "ethereum".into(),
        }
    }
}

#[derive(Clone)]
pub struct CoinTransportMetrics {
    /// Using a weak reference by default in order to avoid circular references and leaks.
    metrics: MetricsWeak,
    /// Name of coin the rpc client is intended to work with.
    ticker: String,
    /// RPC client type.
    client: String,
}

impl CoinTransportMetrics {
    fn new(metrics: MetricsWeak, ticker: String, client: RpcClientType) -> CoinTransportMetrics {
        CoinTransportMetrics {
            metrics,
            ticker,
            client: client.to_string(),
        }
    }

    fn into_shared(self) -> RpcTransportEventHandlerShared { Arc::new(self) }
}

impl RpcTransportEventHandler for CoinTransportMetrics {
    fn debug_info(&self) -> String { "CoinTransportMetrics".into() }

    fn on_outgoing_request(&self, data: &[u8]) {
        mm_counter!(self.metrics, "rpc_client.traffic.out", data.len() as u64,
            "coin" => self.ticker.to_owned(), "client" => self.client.to_owned());
        mm_counter!(self.metrics, "rpc_client.request.count", 1,
            "coin" => self.ticker.to_owned(), "client" => self.client.to_owned());
    }

    fn on_incoming_response(&self, data: &[u8]) {
        mm_counter!(self.metrics, "rpc_client.traffic.in", data.len() as u64,
            "coin" => self.ticker.to_owned(), "client" => self.client.to_owned());
        mm_counter!(self.metrics, "rpc_client.response.count", 1,
            "coin" => self.ticker.to_owned(), "client" => self.client.to_owned());
    }

    fn on_connected(&self, _address: String) -> Result<(), String> {
        // Handle a new connected endpoint if necessary.
        // Now just return the Ok
        Ok(())
    }

    fn on_disconnected(&self, _address: String) -> Result<(), String> {
        // Handle disconnected endpoint if necessary.
        // Now just return the Ok
        Ok(())
    }
}

#[async_trait]
impl BalanceTradeFeeUpdatedHandler for CoinsContext {
    async fn balance_updated(&self, coin: &MmCoinEnum, new_balance: &BigDecimal) {
        for sub in self.balance_update_handlers.lock().await.iter() {
            sub.balance_updated(coin, new_balance).await
        }
    }
}

pub fn coin_conf(ctx: &MmArc, ticker: &str) -> Json {
    match ctx.conf["coins"].as_array() {
        Some(coins) => coins
            .iter()
            .find(|coin| coin["coin"].as_str() == Some(ticker))
            .cloned()
            .unwrap_or(Json::Null),
        None => Json::Null,
    }
}

pub fn is_wallet_only_conf(conf: &Json) -> bool { conf["wallet_only"].as_bool().unwrap_or(false) }

pub fn is_wallet_only_ticker(ctx: &MmArc, ticker: &str) -> bool {
    let coin_conf = coin_conf(ctx, ticker);
    coin_conf["wallet_only"].as_bool().unwrap_or(false)
}

/// Adds a new currency into the list of currencies configured.
///
/// Returns an error if the currency already exists. Initializing the same currency twice is a bad habit
/// (might lead to misleading and confusing information during debugging and maintenance, see DRY)
/// and should be fixed on the call site.
///
/// * `req` - Payload of the corresponding "enable" or "electrum" RPC request.
pub async fn lp_coininit(ctx: &MmArc, ticker: &str, req: &Json) -> Result<MmCoinEnum, String> {
    let cctx = try_s!(CoinsContext::from_ctx(ctx));
    {
        let coins = cctx.coins.lock().await;
        if coins.get(ticker).is_some() {
            return ERR!("Coin {} already initialized", ticker);
        }
    }

    let coins_en = coin_conf(ctx, ticker);

    coins_conf_check(ctx, &coins_en, ticker, Some(req))?;

    // The legacy electrum/enable RPCs don't support Hardware Wallet policy.
    let priv_key_policy = try_s!(PrivKeyBuildPolicy::detect_priv_key_policy(ctx));

    let protocol: CoinProtocol = try_s!(json::from_value(coins_en["protocol"].clone()));

    let coin: MmCoinEnum = match &protocol {
        CoinProtocol::UTXO => {
            let params = try_s!(UtxoActivationParams::from_legacy_req(req));
            try_s!(utxo_standard_coin_with_policy(ctx, ticker, &coins_en, &params, priv_key_policy).await).into()
        },
        CoinProtocol::QTUM => {
            let params = try_s!(UtxoActivationParams::from_legacy_req(req));
            try_s!(qtum_coin_with_policy(ctx, ticker, &coins_en, &params, priv_key_policy).await).into()
        },
        CoinProtocol::ETH | CoinProtocol::ERC20 { .. } => {
            try_s!(eth_coin_from_conf_and_request(ctx, ticker, &coins_en, req, protocol, priv_key_policy).await).into()
        },
        CoinProtocol::QRC20 {
            platform,
            contract_address,
        } => {
            let params = try_s!(Qrc20ActivationParams::from_legacy_req(req));
            let contract_address = try_s!(qtum::contract_addr_from_str(contract_address));

            try_s!(
                qrc20_coin_with_policy(
                    ctx,
                    ticker,
                    platform,
                    &coins_en,
                    &params,
                    priv_key_policy,
                    contract_address
                )
                .await
            )
            .into()
        },
        CoinProtocol::BCH { slp_prefix } => {
            let prefix = try_s!(CashAddrPrefix::from_str(slp_prefix));
            let params = try_s!(BchActivationRequest::from_legacy_req(req));

            let bch = try_s!(bch_coin_with_policy(ctx, ticker, &coins_en, params, prefix, priv_key_policy).await);
            bch.into()
        },
        CoinProtocol::SLPTOKEN {
            platform,
            token_id,
            decimals,
            required_confirmations,
        } => {
            let platform_coin = try_s!(lp_coinfind(ctx, platform).await);
            let platform_coin = match platform_coin {
                Some(MmCoinEnum::Bch(coin)) => coin,
                Some(_) => return ERR!("Platform coin {} is not BCH", platform),
                None => return ERR!("Platform coin {} is not activated", platform),
            };

            let confs = required_confirmations.unwrap_or(platform_coin.required_confirmations());
            let token = try_s!(SlpToken::new(
                *decimals,
                ticker.into(),
                (*token_id).into(),
                platform_coin,
                confs
            ));
            token.into()
        },
        CoinProtocol::TENDERMINT { .. } => return ERR!("TENDERMINT protocol is not supported by lp_coininit"),
        CoinProtocol::TENDERMINTTOKEN(_) => return ERR!("TENDERMINTTOKEN protocol is not supported by lp_coininit"),
        CoinProtocol::ZHTLC { .. } => return ERR!("ZHTLC protocol is not supported by lp_coininit"),
        #[cfg(not(target_arch = "wasm32"))]
        CoinProtocol::LIGHTNING { .. } => return ERR!("Lightning protocol is not supported by lp_coininit"),
        #[cfg(all(feature = "enable-solana", not(target_arch = "wasm32")))]
        CoinProtocol::SOLANA => {
            return ERR!("Solana protocol is not supported by lp_coininit - use enable_solana_with_tokens instead")
        },
        #[cfg(all(feature = "enable-solana", not(target_arch = "wasm32")))]
        CoinProtocol::SPLTOKEN { .. } => {
            return ERR!("SplToken protocol is not supported by lp_coininit - use enable_spl instead")
        },
    };

    let register_params = RegisterCoinParams {
        ticker: ticker.to_owned(),
    };
    try_s!(lp_register_coin(ctx, coin.clone(), register_params).await);

    let tx_history = req["tx_history"].as_bool().unwrap_or(false);
    if tx_history {
        try_s!(lp_spawn_tx_history(ctx.clone(), coin.clone()).map_to_mm(RegisterCoinError::Internal));
    }
    Ok(coin)
}

#[derive(Debug, Display)]
pub enum RegisterCoinError {
    #[display(fmt = "Coin '{}' is initialized already", coin)]
    CoinIsInitializedAlready {
        coin: String,
    },
    Internal(String),
}

pub struct RegisterCoinParams {
    pub ticker: String,
}

pub async fn lp_register_coin(
    ctx: &MmArc,
    coin: MmCoinEnum,
    params: RegisterCoinParams,
) -> Result<(), MmError<RegisterCoinError>> {
    let RegisterCoinParams { ticker } = params;
    let cctx = CoinsContext::from_ctx(ctx).map_to_mm(RegisterCoinError::Internal)?;

    // TODO AP: locking the coins list during the entire initialization prevents different coins from being
    // activated concurrently which results in long activation time: https://github.com/KomodoPlatform/atomicDEX/issues/24
    // So I'm leaving the possibility of race condition intentionally in favor of faster concurrent activation.
    // Should consider refactoring: maybe extract the RPC client initialization part from coin init functions.
    let mut coins = cctx.coins.lock().await;
    match coins.raw_entry_mut().from_key(&ticker) {
        RawEntryMut::Occupied(_oe) => {
            return MmError::err(RegisterCoinError::CoinIsInitializedAlready { coin: ticker.clone() })
        },
        RawEntryMut::Vacant(ve) => ve.insert(ticker.clone(), MmCoinStruct::new(coin.clone())),
    };

    if coin.is_platform_coin() {
        let mut platform_coin_tokens = cctx.platform_coin_tokens.lock();
        platform_coin_tokens
            .entry(coin.ticker().to_string())
            .or_insert_with(HashSet::new);
    }
    Ok(())
}

fn lp_spawn_tx_history(ctx: MmArc, coin: MmCoinEnum) -> Result<(), String> {
    let spawner = coin.spawner();
    let fut = async move {
        let _res = coin.process_history_loop(ctx).compat().await;
    };
    spawner.spawn(fut);
    Ok(())
}

/// NB: Returns only the enabled (aka active) coins.
pub async fn lp_coinfind(ctx: &MmArc, ticker: &str) -> Result<Option<MmCoinEnum>, String> {
    let cctx = try_s!(CoinsContext::from_ctx(ctx));
    let coins = cctx.coins.lock().await;

    if let Some(coin) = coins.get(ticker) {
        if coin.is_available() {
            return Ok(Some(coin.inner.clone()));
        }
    };

    Ok(None)
}

/// Returns coins even if they are on the passive mode
pub async fn lp_coinfind_any(ctx: &MmArc, ticker: &str) -> Result<Option<MmCoinStruct>, String> {
    let cctx = try_s!(CoinsContext::from_ctx(ctx));
    let coins = cctx.coins.lock().await;

    Ok(coins.get(ticker).cloned())
}

/// Attempts to find a pair of active coins returning None if one is not enabled
pub async fn find_pair(ctx: &MmArc, base: &str, rel: &str) -> Result<Option<(MmCoinEnum, MmCoinEnum)>, String> {
    let fut_base = lp_coinfind(ctx, base);
    let fut_rel = lp_coinfind(ctx, rel);

    futures::future::try_join(fut_base, fut_rel)
        .map_ok(|(base, rel)| base.zip(rel))
        .await
}

#[derive(Debug, Display)]
pub enum CoinFindError {
    #[display(fmt = "No such coin: {}", coin)]
    NoSuchCoin { coin: String },
}

pub async fn lp_coinfind_or_err(ctx: &MmArc, ticker: &str) -> CoinFindResult<MmCoinEnum> {
    match lp_coinfind(ctx, ticker).await {
        Ok(Some(coin)) => Ok(coin),
        Ok(None) => MmError::err(CoinFindError::NoSuchCoin {
            coin: ticker.to_owned(),
        }),
        Err(e) => panic!("Unexpected error: {}", e),
    }
}

#[derive(Deserialize)]
struct ConvertAddressReq {
    coin: String,
    from: String,
    /// format to that the input address should be converted
    to_address_format: Json,
}

pub async fn convert_address(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: ConvertAddressReq = try_s!(json::from_value(req));
    let coin = match lp_coinfind(&ctx, &req.coin).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin: {}", req.coin),
        Err(err) => return ERR!("!lp_coinfind({}): {}", req.coin, err),
    };
    let result = json!({
        "result": {
            "address": try_s!(coin.convert_to_address(&req.from, req.to_address_format)),
        },
    });
    let body = try_s!(json::to_vec(&result));
    Ok(try_s!(Response::builder().body(body)))
}

pub async fn kmd_rewards_info(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    let coin = match lp_coinfind(&ctx, "KMD").await {
        Ok(Some(MmCoinEnum::UtxoCoin(t))) => t,
        Ok(Some(_)) => return ERR!("KMD was expected to be UTXO"),
        Ok(None) => return ERR!("KMD is not activated"),
        Err(err) => return ERR!("!lp_coinfind({}): KMD", err),
    };

    let res = json!({
        "result": try_s!(utxo::kmd_rewards_info(&coin).await),
    });
    let res = try_s!(json::to_vec(&res));
    Ok(try_s!(Response::builder().body(res)))
}

#[derive(Deserialize)]
struct ValidateAddressReq {
    coin: String,
    address: String,
}

#[derive(Serialize)]
pub struct ValidateAddressResult {
    pub is_valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub async fn validate_address(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: ValidateAddressReq = try_s!(json::from_value(req));
    let coin = match lp_coinfind(&ctx, &req.coin).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin: {}", req.coin),
        Err(err) => return ERR!("!lp_coinfind({}): {}", req.coin, err),
    };

    let res = json!({ "result": coin.validate_address(&req.address) });
    let body = try_s!(json::to_vec(&res));
    Ok(try_s!(Response::builder().body(body)))
}

pub async fn withdraw(ctx: MmArc, req: WithdrawRequest) -> WithdrawResult {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    coin.withdraw(req).compat().await
}

pub async fn get_raw_transaction(ctx: MmArc, req: RawTransactionRequest) -> RawTransactionResult {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    coin.get_raw_transaction(req).compat().await
}

pub async fn sign_message(ctx: MmArc, req: SignatureRequest) -> SignatureResult<SignatureResponse> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    let signature = coin.sign_message(&req.message)?;
    Ok(SignatureResponse { signature })
}

pub async fn verify_message(ctx: MmArc, req: VerificationRequest) -> VerificationResult<VerificationResponse> {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;

    let validate_address_result = coin.validate_address(&req.address);
    if !validate_address_result.is_valid {
        return MmError::err(VerificationError::InvalidRequest(
            validate_address_result.reason.unwrap_or_else(|| "Unknown".to_string()),
        ));
    }

    let is_valid = coin.verify_message(&req.signature, &req.message, &req.address)?;

    Ok(VerificationResponse { is_valid })
}

pub async fn sign_raw_transaction(ctx: MmArc, req: SignRawTransactionRequest) -> RawTransactionResult {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    coin.sign_raw_tx(&req).await
}

pub async fn remove_delegation(ctx: MmArc, req: RemoveDelegateRequest) -> DelegationResult {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    match coin {
        MmCoinEnum::QtumCoin(qtum) => qtum.remove_delegation().compat().await,
        _ => {
            return MmError::err(DelegationError::CoinDoesntSupportDelegation {
                coin: coin.ticker().to_string(),
            })
        },
    }
}

pub async fn get_staking_infos(ctx: MmArc, req: GetStakingInfosRequest) -> StakingInfosResult {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    match coin {
        MmCoinEnum::QtumCoin(qtum) => qtum.get_delegation_infos().compat().await,
        _ => {
            return MmError::err(StakingInfosError::CoinDoesntSupportStakingInfos {
                coin: coin.ticker().to_string(),
            })
        },
    }
}

pub async fn add_delegation(ctx: MmArc, req: AddDelegateRequest) -> DelegationResult {
    let coin = lp_coinfind_or_err(&ctx, &req.coin).await?;
    // Need to find a way to do a proper dispatch
    let coin_concrete = match coin {
        MmCoinEnum::QtumCoin(qtum) => qtum,
        _ => {
            return MmError::err(DelegationError::CoinDoesntSupportDelegation {
                coin: coin.ticker().to_string(),
            })
        },
    };
    match req.staking_details {
        StakingDetails::Qtum(qtum_staking) => coin_concrete.add_delegation(qtum_staking).compat().await,
    }
}

pub async fn send_raw_transaction(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or("No 'coin' field")).to_owned();
    let coin = match lp_coinfind(&ctx, &ticker).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin: {}", ticker),
        Err(err) => return ERR!("!lp_coinfind({}): {}", ticker, err),
    };
    let bytes_string = try_s!(req["tx_hex"].as_str().ok_or("No 'tx_hex' field"));
    let res = try_s!(coin.send_raw_tx(bytes_string).compat().await);
    let body = try_s!(json::to_vec(&json!({ "tx_hash": res })));
    Ok(try_s!(Response::builder().body(body)))
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", content = "additional_info")]
pub enum HistorySyncState {
    NotEnabled,
    NotStarted,
    InProgress(Json),
    Error(Json),
    Finished,
}

#[derive(Deserialize)]
struct MyTxHistoryRequest {
    coin: String,
    from_id: Option<BytesJson>,
    #[serde(default)]
    max: bool,
    #[serde(default = "ten")]
    limit: usize,
    page_number: Option<NonZeroUsize>,
}

/// Returns the transaction history of selected coin. Returns no more than `limit` records (default: 10).
/// Skips the first records up to from_id (skipping the from_id too).
/// Transactions are sorted by number of confirmations in ascending order.
pub async fn my_tx_history(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let request: MyTxHistoryRequest = try_s!(json::from_value(req));
    let coin = match lp_coinfind(&ctx, &request.coin).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin: {}", request.coin),
        Err(err) => return ERR!("!lp_coinfind({}): {}", request.coin, err),
    };

    let history = try_s!(coin.load_history_from_file(&ctx).compat().await);
    let total_records = history.len();
    let limit = if request.max { total_records } else { request.limit };

    let block_number = try_s!(coin.current_block().compat().await);
    let skip = match &request.from_id {
        Some(id) => {
            try_s!(history
                .iter()
                .position(|item| item.internal_id == *id)
                .ok_or(format!("from_id {:02x} is not found", id)))
                + 1
        },
        None => match request.page_number {
            Some(page_n) => (page_n.get() - 1) * request.limit,
            None => 0,
        },
    };

    let history = history.into_iter().skip(skip).take(limit);
    let history: Vec<Json> = history
        .map(|item| {
            let tx_block = item.block_height;
            let mut json = json::to_value(item).unwrap();
            json["confirmations"] = if tx_block == 0 {
                Json::from(0)
            } else if block_number >= tx_block {
                Json::from((block_number - tx_block) + 1)
            } else {
                Json::from(0)
            };
            json
        })
        .collect();

    let response = json!({
        "result": {
            "transactions": history,
            "limit": limit,
            "skipped": skip,
            "from_id": request.from_id,
            "total": total_records,
            "current_block": block_number,
            "sync_status": coin.history_sync_status(),
            "page_number": request.page_number,
            "total_pages": calc_total_pages(total_records, request.limit),
        }
    });
    let body = try_s!(json::to_vec(&response));
    Ok(try_s!(Response::builder().body(body)))
}

pub async fn get_trade_fee(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or("No 'coin' field")).to_owned();
    let coin = match lp_coinfind(&ctx, &ticker).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin: {}", ticker),
        Err(err) => return ERR!("!lp_coinfind({}): {}", ticker, err),
    };
    let fee_info = try_s!(coin.get_trade_fee().compat().await);
    let res = try_s!(json::to_vec(&json!({
        "result": {
            "coin": fee_info.coin,
            "amount": fee_info.amount.to_decimal(),
            "amount_fraction": fee_info.amount.to_fraction(),
            "amount_rat": fee_info.amount.to_ratio(),
        }
    })));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn get_enabled_coins(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    let coins_ctx: Arc<CoinsContext> = try_s!(CoinsContext::from_ctx(&ctx));
    let coins = coins_ctx.coins.lock().await;
    let enabled_coins: GetEnabledResponse = try_s!(coins
        .iter()
        .map(|(ticker, coin)| {
            let address = try_s!(coin.inner.my_address());
            Ok(EnabledCoin {
                ticker: ticker.clone(),
                address,
            })
        })
        .collect());
    let res = try_s!(json::to_vec(&Mm2RpcResult::new(enabled_coins)));
    Ok(try_s!(Response::builder().body(res)))
}

#[derive(Deserialize)]
pub struct ConfirmationsReq {
    coin: String,
    confirmations: u64,
}

pub async fn set_required_confirmations(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: ConfirmationsReq = try_s!(json::from_value(req));
    let coin = match lp_coinfind(&ctx, &req.coin).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin {}", req.coin),
        Err(err) => return ERR!("!lp_coinfind ({}): {}", req.coin, err),
    };
    coin.set_required_confirmations(req.confirmations);
    let res = try_s!(json::to_vec(&json!({
        "result": {
            "coin": req.coin,
            "confirmations": coin.required_confirmations(),
        }
    })));
    Ok(try_s!(Response::builder().body(res)))
}

#[derive(Deserialize)]
pub struct RequiresNotaReq {
    coin: String,
    requires_notarization: bool,
}

pub async fn set_requires_notarization(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: RequiresNotaReq = try_s!(json::from_value(req));
    let coin = match lp_coinfind(&ctx, &req.coin).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin {}", req.coin),
        Err(err) => return ERR!("!lp_coinfind ({}): {}", req.coin, err),
    };
    coin.set_requires_notarization(req.requires_notarization);
    let res = try_s!(json::to_vec(&json!({
        "result": {
            "coin": req.coin,
            "requires_notarization": coin.requires_notarization(),
        }
    })));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn show_priv_key(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let ticker = try_s!(req["coin"].as_str().ok_or("No 'coin' field")).to_owned();
    let coin = match lp_coinfind(&ctx, &ticker).await {
        Ok(Some(t)) => t,
        Ok(None) => return ERR!("No such coin: {}", ticker),
        Err(err) => return ERR!("!lp_coinfind({}): {}", ticker, err),
    };
    let res = try_s!(json::to_vec(&json!({
        "result": {
            "coin": ticker,
            "priv_key": try_s!(coin.display_priv_key()),
        }
    })));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn register_balance_update_handler(
    ctx: MmArc,
    handler: Box<dyn BalanceTradeFeeUpdatedHandler + Send + Sync>,
) {
    let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
    coins_ctx.balance_update_handlers.lock().await.push(handler);
}

pub fn update_coins_config(mut config: Json) -> Result<Json, String> {
    let coins = match config.as_array_mut() {
        Some(c) => c,
        _ => return ERR!("Coins config must be an array"),
    };

    for coin in coins {
        // the coin_as_str is used only to be formatted
        let coin_as_str = format!("{}", coin);
        let coin = try_s!(coin
            .as_object_mut()
            .ok_or(ERRL!("Expected object, found {:?}", coin_as_str)));
        if coin.contains_key("protocol") {
            // the coin is up-to-date
            continue;
        }
        let protocol = match coin.remove("etomic") {
            Some(etomic) => {
                let etomic = etomic
                    .as_str()
                    .ok_or(ERRL!("Expected etomic as string, found {:?}", etomic))?;
                if etomic == "0x0000000000000000000000000000000000000000" {
                    CoinProtocol::ETH
                } else {
                    let contract_address = etomic.to_owned();
                    CoinProtocol::ERC20 {
                        platform: "ETH".into(),
                        contract_address,
                    }
                }
            },
            _ => CoinProtocol::UTXO,
        };

        let protocol = json::to_value(protocol).map_err(|e| ERRL!("Error {:?} on process {:?}", e, coin_as_str))?;
        coin.insert("protocol".into(), protocol);
    }

    Ok(config)
}

#[derive(Deserialize)]
struct ConvertUtxoAddressReq {
    address: String,
    to_coin: String,
}

pub async fn convert_utxo_address(ctx: MmArc, req: Json) -> Result<Response<Vec<u8>>, String> {
    let req: ConvertUtxoAddressReq = try_s!(json::from_value(req));
    let mut addr: utxo::Address = try_s!(req.address.parse());
    let coin = match lp_coinfind(&ctx, &req.to_coin).await {
        Ok(Some(c)) => c,
        _ => return ERR!("Coin {} is not activated", req.to_coin),
    };
    let coin = match coin {
        MmCoinEnum::UtxoCoin(utxo) => utxo,
        _ => return ERR!("Coin {} is not utxo", req.to_coin),
    };
    addr.prefix = coin.as_ref().conf.pub_addr_prefix;
    addr.t_addr_prefix = coin.as_ref().conf.pub_t_addr_prefix;
    addr.checksum_type = coin.as_ref().conf.checksum_type;

    let response = try_s!(json::to_vec(&json!({
        "result": addr.to_string(),
    })));
    Ok(try_s!(Response::builder().body(response)))
}

pub fn address_by_coin_conf_and_pubkey_str(
    ctx: &MmArc,
    coin: &str,
    conf: &Json,
    pubkey: &str,
    addr_format: UtxoAddressFormat,
) -> Result<String, String> {
    let protocol: CoinProtocol = try_s!(json::from_value(conf["protocol"].clone()));
    match protocol {
        CoinProtocol::ERC20 { .. } | CoinProtocol::ETH => eth::addr_from_pubkey_str(pubkey),
        CoinProtocol::UTXO | CoinProtocol::QTUM | CoinProtocol::QRC20 { .. } | CoinProtocol::BCH { .. } => {
            utxo::address_by_conf_and_pubkey_str(coin, conf, pubkey, addr_format)
        },
        CoinProtocol::SLPTOKEN { platform, .. } => {
            let platform_conf = coin_conf(ctx, &platform);
            if platform_conf.is_null() {
                return ERR!("platform {} conf is null", platform);
            }
            // TODO is there any way to make it better without duplicating the prefix in the SLP conf?
            let platform_protocol: CoinProtocol = try_s!(json::from_value(platform_conf["protocol"].clone()));
            match platform_protocol {
                CoinProtocol::BCH { slp_prefix } => {
                    slp_addr_from_pubkey_str(pubkey, &slp_prefix).map_err(|e| ERRL!("{}", e))
                },
                _ => ERR!("Platform protocol {:?} is not BCH", platform_protocol),
            }
        },
        CoinProtocol::TENDERMINT(protocol) => tendermint::account_id_from_pubkey_hex(&protocol.account_prefix, pubkey)
            .map(|id| id.to_string())
            .map_err(|e| e.to_string()),
        CoinProtocol::TENDERMINTTOKEN(proto) => {
            let platform_conf = coin_conf(ctx, &proto.platform);
            if platform_conf.is_null() {
                return ERR!("platform {} conf is null", proto.platform);
            }
            // TODO is there any way to make it better without duplicating the prefix in the IBC conf?
            let platform_protocol: CoinProtocol = try_s!(json::from_value(platform_conf["protocol"].clone()));
            match platform_protocol {
                CoinProtocol::TENDERMINT(platform) => {
                    tendermint::account_id_from_pubkey_hex(&platform.account_prefix, pubkey)
                        .map(|id| id.to_string())
                        .map_err(|e| e.to_string())
                },
                _ => ERR!("Platform protocol {:?} is not TENDERMINT", platform_protocol),
            }
        },
        #[cfg(not(target_arch = "wasm32"))]
        CoinProtocol::LIGHTNING { .. } => {
            ERR!("address_by_coin_conf_and_pubkey_str is not implemented for lightning protocol yet!")
        },
        #[cfg(all(feature = "enable-solana", not(target_arch = "wasm32")))]
        CoinProtocol::SOLANA | CoinProtocol::SPLTOKEN { .. } => {
            ERR!("Solana pubkey is the public address - you do not need to use this rpc call.")
        },
        CoinProtocol::ZHTLC { .. } => ERR!("address_by_coin_conf_and_pubkey_str is not supported for ZHTLC protocol!"),
    }
}

#[cfg(target_arch = "wasm32")]
fn load_history_from_file_impl<T>(coin: &T, ctx: &MmArc) -> TxHistoryFut<Vec<TransactionDetails>>
where
    T: MmCoin + ?Sized,
{
    let ctx = ctx.clone();
    let ticker = coin.ticker().to_owned();
    let my_address = try_f!(coin.my_address());

    let fut = async move {
        let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
        let db = coins_ctx.tx_history_db().await?;
        let err = match load_tx_history(&db, &ticker, &my_address).await {
            Ok(history) => return Ok(history),
            Err(e) => e,
        };

        if let TxHistoryError::ErrorDeserializing(e) = err.get_inner() {
            ctx.log.log(
                "🌋",
                &[&"tx_history", &ticker.to_owned()],
                &ERRL!("Error {} on history deserialization, resetting the cache.", e),
            );
            clear_tx_history(&db, &ticker, &my_address).await?;
            return Ok(Vec::new());
        }

        Err(err)
    };
    Box::new(fut.boxed().compat())
}

#[cfg(not(target_arch = "wasm32"))]
fn load_history_from_file_impl<T>(coin: &T, ctx: &MmArc) -> TxHistoryFut<Vec<TransactionDetails>>
where
    T: MmCoin + ?Sized,
{
    let ticker = coin.ticker().to_owned();
    let history_path = coin.tx_history_path(ctx);
    let ctx = ctx.clone();

    let fut = async move {
        let content = match fs::read(&history_path).await {
            Ok(content) => content,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            },
            Err(err) => {
                let error = format!(
                    "Error '{}' reading from the history file {}",
                    err,
                    history_path.display()
                );
                return MmError::err(TxHistoryError::ErrorLoading(error));
            },
        };
        let serde_err = match json::from_slice(&content) {
            Ok(txs) => return Ok(txs),
            Err(e) => e,
        };

        ctx.log.log(
            "🌋",
            &[&"tx_history", &ticker],
            &ERRL!("Error {} on history deserialization, resetting the cache.", serde_err),
        );
        fs::remove_file(&history_path)
            .await
            .map_to_mm(|e| TxHistoryError::ErrorClearing(e.to_string()))?;
        Ok(Vec::new())
    };
    Box::new(fut.boxed().compat())
}

#[cfg(target_arch = "wasm32")]
fn save_history_to_file_impl<T>(coin: &T, ctx: &MmArc, mut history: Vec<TransactionDetails>) -> TxHistoryFut<()>
where
    T: MmCoin + MarketCoinOps + ?Sized,
{
    let ctx = ctx.clone();
    let ticker = coin.ticker().to_owned();
    let my_address = try_f!(coin.my_address());

    history.sort_unstable_by(compare_transaction_details);

    let fut = async move {
        let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
        let db = coins_ctx.tx_history_db().await?;
        save_tx_history(&db, &ticker, &my_address, history).await?;
        Ok(())
    };
    Box::new(fut.boxed().compat())
}

#[cfg(not(target_arch = "wasm32"))]
fn get_tx_history_migration_impl<T>(coin: &T, ctx: &MmArc) -> TxHistoryFut<u64>
where
    T: MmCoin + MarketCoinOps + ?Sized,
{
    let migration_path = coin.tx_migration_path(ctx);

    let fut = async move {
        let current_migration = match fs::read(&migration_path).await {
            Ok(bytes) => {
                let mut num_bytes = [0; 8];
                if bytes.len() == 8 {
                    num_bytes.clone_from_slice(&bytes);
                    u64::from_le_bytes(num_bytes)
                } else {
                    0
                }
            },
            Err(_) => 0,
        };

        Ok(current_migration)
    };

    Box::new(fut.boxed().compat())
}

#[cfg(not(target_arch = "wasm32"))]
fn update_migration_file_impl<T>(coin: &T, ctx: &MmArc, migration_number: u64) -> TxHistoryFut<()>
where
    T: MmCoin + MarketCoinOps + ?Sized,
{
    let migration_path = coin.tx_migration_path(ctx);
    let tmp_file = format!("{}.tmp", migration_path.display());

    let fut = async move {
        let fs_fut = async {
            let mut file = fs::File::create(&tmp_file).await?;
            file.write_all(&migration_number.to_le_bytes()).await?;
            file.flush().await?;
            fs::rename(&tmp_file, migration_path).await?;
            Ok(())
        };

        let res: io::Result<_> = fs_fut.await;
        if let Err(e) = res {
            let error = format!("Error '{}' creating/writing/renaming the tmp file {}", e, tmp_file);
            return MmError::err(TxHistoryError::ErrorSaving(error));
        }
        Ok(())
    };

    Box::new(fut.boxed().compat())
}

#[cfg(not(target_arch = "wasm32"))]
fn save_history_to_file_impl<T>(coin: &T, ctx: &MmArc, mut history: Vec<TransactionDetails>) -> TxHistoryFut<()>
where
    T: MmCoin + MarketCoinOps + ?Sized,
{
    let history_path = coin.tx_history_path(ctx);
    let tmp_file = format!("{}.tmp", history_path.display());

    history.sort_unstable_by(compare_transaction_details);

    let fut = async move {
        let content = json::to_vec(&history).map_to_mm(|e| TxHistoryError::ErrorSerializing(e.to_string()))?;

        let fs_fut = async {
            let mut file = fs::File::create(&tmp_file).await?;
            file.write_all(&content).await?;
            file.flush().await?;
            fs::rename(&tmp_file, &history_path).await?;
            Ok(())
        };

        let res: io::Result<_> = fs_fut.await;
        if let Err(e) = res {
            let error = format!("Error '{}' creating/writing/renaming the tmp file {}", e, tmp_file);
            return MmError::err(TxHistoryError::ErrorSaving(error));
        }
        Ok(())
    };
    Box::new(fut.boxed().compat())
}

pub(crate) fn compare_transaction_details(a: &TransactionDetails, b: &TransactionDetails) -> Ordering {
    let a = TxIdHeight::new(a.block_height, a.internal_id.deref());
    let b = TxIdHeight::new(b.block_height, b.internal_id.deref());
    compare_transactions(a, b)
}

pub(crate) struct TxIdHeight<Id> {
    block_height: u64,
    tx_id: Id,
}

impl<Id> TxIdHeight<Id> {
    pub(crate) fn new(block_height: u64, tx_id: Id) -> TxIdHeight<Id> { TxIdHeight { block_height, tx_id } }
}

pub(crate) fn compare_transactions<Id>(a: TxIdHeight<Id>, b: TxIdHeight<Id>) -> Ordering
where
    Id: Ord,
{
    // the transactions with block_height == 0 are the most recent so we need to separately handle them while sorting
    if a.block_height == b.block_height {
        a.tx_id.cmp(&b.tx_id)
    } else if a.block_height == 0 {
        Ordering::Less
    } else if b.block_height == 0 {
        Ordering::Greater
    } else {
        b.block_height.cmp(&a.block_height)
    }
}

/// Use trait in the case, when we have to send requests to rpc client.
#[async_trait]
pub trait RpcCommonOps {
    type RpcClient;
    type Error;

    /// Returns an alive RPC client or returns an error if no RPC endpoint is currently available.
    async fn get_live_client(&self) -> Result<Self::RpcClient, Self::Error>;
}

/// `get_my_address` function returns wallet address for necessary coin without its activation.
/// Currently supports only coins with `ETH` protocol type.
pub async fn get_my_address(ctx: MmArc, req: MyAddressReq) -> MmResult<MyWalletAddress, GetMyAddressError> {
    let ticker = req.coin.as_str();
    let conf = coin_conf(&ctx, ticker);
    coins_conf_check(&ctx, &conf, ticker, None).map_to_mm(GetMyAddressError::CoinsConfCheckError)?;

    let protocol: CoinProtocol = json::from_value(conf["protocol"].clone())?;

    let my_address = match protocol {
        CoinProtocol::ETH => get_eth_address(&ctx, &conf, ticker, &req.path_to_address).await?,
        _ => {
            return MmError::err(GetMyAddressError::CoinIsNotSupported(format!(
                "{} doesn't support get_my_address",
                req.coin
            )));
        },
    };

    Ok(my_address)
}

fn coins_conf_check(ctx: &MmArc, coins_en: &Json, ticker: &str, req: Option<&Json>) -> Result<(), String> {
    if coins_en.is_null() {
        let warning = format!(
            "Warning, coin {} is used without a corresponding configuration.",
            ticker
        );
        ctx.log.log(
            "😅",
            #[allow(clippy::unnecessary_cast)]
            &[&("coin" as &str), &ticker, &("no-conf" as &str)],
            &warning,
        );
    }

    if let Some(req) = req {
        if coins_en["mm2"].is_null() && req["mm2"].is_null() {
            return ERR!(concat!(
                "mm2 param is not set neither in coins config nor enable request, assuming that coin is not supported"
            ));
        }
    } else if coins_en["mm2"].is_null() {
        return ERR!(concat!(
            "mm2 param is not set in coins config, assuming that coin is not supported"
        ));
    }

    if coins_en["protocol"].is_null() {
        return ERR!(
            r#""protocol" field is missing in coins file. The file format is deprecated, please execute ./mm2 update_config command to convert it or download a new one"#
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use common::block_on;
    use mm2_test_helpers::for_tests::RICK;

    #[test]
    fn test_lp_coinfind() {
        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();
        let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
        let coin = MmCoinEnum::Test(TestCoin::new(RICK));

        // Add test coin to coins context
        common::block_on(coins_ctx.add_platform_with_tokens(coin.clone(), vec![])).unwrap();

        // Try to find RICK from coins context that was added above
        let _found = common::block_on(lp_coinfind(&ctx, RICK)).unwrap();

        assert!(matches!(Some(coin), _found));

        block_on(coins_ctx.coins.lock())
            .get(RICK)
            .unwrap()
            .update_is_available(false);

        // Try to find RICK from coins context after making it passive
        let found = common::block_on(lp_coinfind(&ctx, RICK)).unwrap();

        assert!(found.is_none());
    }

    #[test]
    fn test_lp_coinfind_any() {
        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();
        let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
        let coin = MmCoinEnum::Test(TestCoin::new(RICK));

        // Add test coin to coins context
        common::block_on(coins_ctx.add_platform_with_tokens(coin.clone(), vec![])).unwrap();

        // Try to find RICK from coins context that was added above
        let _found = common::block_on(lp_coinfind_any(&ctx, RICK)).unwrap();

        assert!(matches!(Some(coin.clone()), _found));

        block_on(coins_ctx.coins.lock())
            .get(RICK)
            .unwrap()
            .update_is_available(false);

        // Try to find RICK from coins context after making it passive
        let _found = common::block_on(lp_coinfind_any(&ctx, RICK)).unwrap();

        assert!(matches!(Some(coin), _found));
    }
}
