/******************************************************************************
 * Copyright © 2022 Atomic Private Limited and its contributors               *
 *                                                                            *
 * See the CONTRIBUTOR-LICENSE-AGREEMENT, COPYING, LICENSE-COPYRIGHT-NOTICE   *
 * and DEVELOPER-CERTIFICATE-OF-ORIGIN files in the LEGAL directory in        *
 * the top-level directory of this distribution for the individual copyright  *
 * holder information and the developer policies on copyright and licensing.  *
 *                                                                            *
 * Unless otherwise agreed in a custom licensing agreement, no part of the    *
 * AtomicDEX software, including this file may be copied, modified, propagated*
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

#![allow(uncommon_codepoints)]
#![feature(integer_atomics)]
#![feature(async_closure)]
#![feature(hash_raw_entry)]
#![feature(stmt_expr_attributes)]

#[macro_use] extern crate common;
#[macro_use] extern crate fomat_macros;
#[macro_use] extern crate gstuff;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate ser_error_derive;

use async_trait::async_trait;
use base58::FromBase58Error;
use bigdecimal::{BigDecimal, ParseBigDecimalError, Zero};
use common::executor::{spawn, Timer};
use common::mm_metrics::MetricsWeak;
use common::mm_number::MmNumber;
use common::{calc_total_pages, now_ms, ten, HttpStatusCode};
use crypto::{Bip32Error, CryptoCtx, DerivationPath};
use derive_more::Display;
use futures::compat::Future01CompatExt;
use futures::lock::Mutex as AsyncMutex;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use http::{Response, StatusCode};
use keys::{AddressFormat as UtxoAddressFormat, KeyPair, NetworkPrefix as CashAddrPrefix};
use mm2_core::mm_ctx::{from_ctx, MmArc, MmWeak};
use mm2_err_handle::prelude::*;
use rpc::v1::types::{Bytes as BytesJson, H256 as H256Json};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{self as json, Value as Json};
use std::collections::hash_map::{HashMap, RawEntryMut};
use std::fmt;
use std::num::NonZeroUsize;
use std::ops::{Add, Deref};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use utxo_signer::with_key_pair::UtxoSignWithKeyPairError;

cfg_native! {
    use crate::lightning::LightningCoin;
    use crate::lightning::ln_conf::PlatformCoinConfirmations;
    use async_std::fs;
    use futures::AsyncWriteExt;
    use std::io;
    use zcash_primitives::transaction::Transaction as ZTransaction;
    use z_coin::ZcoinConsensusParams;
}

cfg_wasm32! {
    use mm2_db::indexed_db::{ConstructibleDb, DbLocked, SharedDb};
    use hd_wallet_storage::HDWalletDb;
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
            Err(e) => return Box::new(futures01::future::err(e)),
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

#[cfg(not(target_arch = "wasm32"))]
macro_rules! ok_or_retry_after_sleep {
    ($e:expr, $delay: ident) => {
        loop {
            match $e {
                Ok(res) => break res,
                Err(e) => {
                    error!("error {:?}", e);
                    Timer::sleep($delay).await;
                    continue;
                },
            }
        }
    };
}

#[cfg(not(target_arch = "wasm32"))]
macro_rules! ok_or_retry_after_sleep_sync {
    ($e:expr, $delay: ident) => {
        loop {
            match $e {
                Ok(res) => break res,
                Err(e) => {
                    error!("error {:?}", e);
                    std::thread::sleep(core::time::Duration::from_secs($delay));
                    continue;
                },
            }
        }
    };
}

pub mod coin_balance;
#[doc(hidden)]
#[cfg(test)]
pub mod coins_tests;
pub mod eth;
pub mod hd_pubkey;
pub mod hd_wallet;
pub mod hd_wallet_storage;
#[cfg(not(target_arch = "wasm32"))] pub mod lightning;
#[cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]
pub mod my_tx_history_v2;
pub mod qrc20;
pub mod rpc_command;
#[doc(hidden)]
#[allow(unused_variables)]
pub mod test_coin;
pub mod tx_history_storage;
pub use test_coin::TestCoin;

#[doc(hidden)]
#[allow(unused_variables)]
#[cfg(not(target_arch = "wasm32"))]
pub mod solana;
#[cfg(not(target_arch = "wasm32"))]
pub use solana::spl::SplToken;
#[cfg(not(target_arch = "wasm32"))]
pub use solana::{solana_coin_from_conf_and_params, SolanaActivationParams, SolanaCoin, SolanaFeeDetails};

pub mod utxo;
#[cfg(not(target_arch = "wasm32"))] pub mod z_coin;

use eth::{eth_coin_from_conf_and_request, EthCoin, EthTxFeeDetails, SignedEthTx};
use hd_wallet::{HDAddress, HDAddressId};
use qrc20::Qrc20ActivationParams;
use qrc20::{qrc20_coin_from_conf_and_params, Qrc20Coin, Qrc20FeeDetails};
use qtum::{Qrc20AddressError, ScriptHashTypeNotSupported};
use rpc_command::init_create_account::{CreateAccountTaskManager, CreateAccountTaskManagerShared};
use rpc_command::init_scan_for_new_addresses::{ScanAddressesTaskManager, ScanAddressesTaskManagerShared};
use rpc_command::init_withdraw::{WithdrawTaskManager, WithdrawTaskManagerShared};
use utxo::bch::{bch_coin_from_conf_and_params, BchActivationRequest, BchCoin};
use utxo::qtum::{self, qtum_coin_with_priv_key, QtumCoin};
use utxo::qtum::{QtumDelegationOps, QtumDelegationRequest, QtumStakingInfosDetails};
use utxo::rpc_clients::UtxoRpcError;
use utxo::slp::SlpToken;
use utxo::slp::{slp_addr_from_pubkey_str, SlpFeeDetails};
use utxo::utxo_common::big_decimal_from_sat_unsigned;
use utxo::utxo_standard::{utxo_standard_coin_with_priv_key, UtxoStandardCoin};
use utxo::UtxoActivationParams;
use utxo::{BlockchainNetwork, GenerateTxError, UtxoFeeDetails, UtxoTx};
#[cfg(not(target_arch = "wasm32"))] use z_coin::ZCoin;

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
}

impl HttpStatusCode for RawTransactionError {
    fn status_code(&self) -> StatusCode {
        match self {
            RawTransactionError::NoSuchCoin { .. }
            | RawTransactionError::InvalidHashError(_)
            | RawTransactionError::HashNotExist(_) => StatusCode::BAD_REQUEST,
            RawTransactionError::Transport(_) | RawTransactionError::InternalError(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
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

#[derive(Debug, Display)]
pub enum PrivKeyNotAllowed {
    #[display(fmt = "Hardware Wallet is not supported")]
    HardwareWalletNotSupported,
}

#[derive(Debug, Display, PartialEq, Serialize)]
pub enum UnexpectedDerivationMethod {
    #[display(fmt = "Iguana private key is unavailable")]
    IguanaPrivKeyUnavailable,
    #[display(fmt = "HD wallet is unavailable")]
    HDWalletUnavailable,
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
    #[cfg(not(target_arch = "wasm32"))]
    ZTransaction(ZTransaction),
}
ifrom!(TransactionEnum, UtxoTx);
ifrom!(TransactionEnum, SignedEthTx);
#[cfg(not(target_arch = "wasm32"))]
ifrom!(TransactionEnum, ZTransaction);

// NB: When stable and groked by IDEs, `enum_dispatch` can be used instead of `Deref` to speed things up.
impl Deref for TransactionEnum {
    type Target = dyn Transaction;
    fn deref(&self) -> &dyn Transaction {
        match self {
            TransactionEnum::UtxoTx(ref t) => t,
            TransactionEnum::SignedEthTx(ref t) => t,
            #[cfg(not(target_arch = "wasm32"))]
            TransactionEnum::ZTransaction(ref t) => t,
        }
    }
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

pub type TransactionFut = Box<dyn Future<Item = TransactionEnum, Error = TransactionErr> + Send>;

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

#[derive(Clone, Debug)]
pub struct ValidatePaymentInput {
    pub payment_tx: Vec<u8>,
    pub time_lock: u32,
    pub other_pub: Vec<u8>,
    pub secret_hash: Vec<u8>,
    pub amount: BigDecimal,
    pub swap_contract_address: Option<BytesJson>,
    pub try_spv_proof_until: u64,
    pub confirmations: u64,
    pub unique_swap_data: Vec<u8>,
}

pub struct SearchForSwapTxSpendInput<'a> {
    pub time_lock: u32,
    pub other_pub: &'a [u8],
    pub secret_hash: &'a [u8],
    pub tx: &'a [u8],
    pub search_from_block: u64,
    pub swap_contract_address: &'a Option<BytesJson>,
    pub swap_unique_data: &'a [u8],
}

/// Swap operations (mostly based on the Hash/Time locked transactions implemented by coin wallets).
#[async_trait]
pub trait SwapOps {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, uuid: &[u8]) -> TransactionFut;

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut;

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut;

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut;

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut;

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut;

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut;

    fn validate_fee(
        &self,
        fee_tx: &TransactionEnum,
        expected_sender: &[u8],
        fee_addr: &[u8],
        amount: &BigDecimal,
        min_block_number: u64,
        uuid: &[u8],
    ) -> Box<dyn Future<Item = (), Error = String> + Send>;

    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send>;

    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send>;

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        search_from_block: u64,
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send>;

    async fn search_for_swap_tx_spend_my(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String>;

    async fn search_for_swap_tx_spend_other(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String>;

    fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String>;

    /// Whether the refund transaction can be sent now
    /// For example: there are no additional conditions for ETH, but for some UTXO coins we should wait for
    /// locktime < MTP
    fn can_refund_htlc(&self, locktime: u64) -> Box<dyn Future<Item = CanRefundHtlc, Error = String> + Send + '_> {
        let now = now_ms() / 1000;
        let result = if now > locktime {
            CanRefundHtlc::CanRefundNow
        } else {
            CanRefundHtlc::HaveToWait(locktime - now + 1)
        };
        Box::new(futures01::future::ok(result))
    }

    fn negotiate_swap_contract_addr(
        &self,
        other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>>;

    fn derive_htlc_key_pair(&self, swap_unique_data: &[u8]) -> KeyPair;
}

/// Operations that coins have independently from the MarketMaker.
/// That is, things implemented by the coin wallets or public coin services.
pub trait MarketCoinOps {
    fn ticker(&self) -> &str;

    fn my_address(&self) -> Result<String, String>;

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

    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        confirmations: u64,
        requires_nota: bool,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send>;

    fn wait_for_tx_spend(
        &self,
        transaction: &[u8],
        wait_until: u64,
        from_block: u64,
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut;

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, String>;

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send>;

    fn display_priv_key(&self) -> Result<String, String>;

    /// Get the minimum amount to send.
    fn min_tx_amount(&self) -> BigDecimal;

    /// Get the minimum amount to trade.
    fn min_trading_vol(&self) -> MmNumber;

    fn is_privacy(&self) -> bool { false }
}

#[derive(Debug, Deserialize, PartialEq)]
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

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub enum WithdrawFrom {
    // AccountId { account_id: u32 },
    AddressId(HDAddressId),
    /// Don't use `Bip44DerivationPath` or `RpcDerivationPath` because if there is an error in the path,
    /// `serde::Deserialize` returns "data did not match any variant of untagged enum WithdrawFrom".
    /// It's better to show the user an informative error.
    DerivationPath {
        derivation_path: String,
    },
}

#[derive(Deserialize)]
pub struct WithdrawRequest {
    coin: String,
    from: Option<WithdrawFrom>,
    to: String,
    #[serde(default)]
    amount: BigDecimal,
    #[serde(default)]
    max: bool,
    fee: Option<WithdrawFee>,
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
    pub fn new(
        coin: String,
        from: Option<WithdrawFrom>,
        to: String,
        amount: BigDecimal,
        max: bool,
        fee: Option<WithdrawFee>,
    ) -> WithdrawRequest {
        WithdrawRequest {
            coin,
            from,
            to,
            amount,
            max,
            fee,
        }
    }

    pub fn new_max(coin: String, to: String) -> WithdrawRequest {
        WithdrawRequest {
            coin,
            from: None,
            to,
            amount: 0.into(),
            max: true,
            fee: None,
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
    #[cfg(not(target_arch = "wasm32"))]
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
            #[cfg(not(target_arch = "wasm32"))]
            Solana(SolanaFeeDetails),
        }

        match Deserialize::deserialize(deserializer)? {
            TxFeeDetailsUnTagged::Utxo(f) => Ok(TxFeeDetails::Utxo(f)),
            TxFeeDetailsUnTagged::Eth(f) => Ok(TxFeeDetails::Eth(f)),
            TxFeeDetailsUnTagged::Qrc20(f) => Ok(TxFeeDetails::Qrc20(f)),
            #[cfg(not(target_arch = "wasm32"))]
            TxFeeDetailsUnTagged::Solana(f) => Ok(TxFeeDetails::Solana(f)),
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

#[cfg(not(target_arch = "wasm32"))]
impl From<SolanaFeeDetails> for TxFeeDetails {
    fn from(solana_details: SolanaFeeDetails) -> Self { TxFeeDetails::Solana(solana_details) }
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum TransactionType {
    StakingDelegation,
    RemoveDelegation,
    StandardTransfer,
    TokenTransfer(BytesJson),
}

impl Default for TransactionType {
    fn default() -> Self { TransactionType::StandardTransfer }
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

#[derive(Debug, Display, PartialEq)]
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

impl From<PrivKeyNotAllowed> for DelegationError {
    fn from(e: PrivKeyNotAllowed) -> Self { DelegationError::DelegationOpsNotSupported { reason: e.to_string() } }
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

#[derive(Clone, Debug, Deserialize, Display, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum WithdrawError {
    /*                                              */
    /*------------ Trezor device errors ------------*/
    /*                                             */
    #[display(fmt = "Trezor device disconnected")]
    TrezorDisconnected,
    #[display(fmt = "Trezor internal error: {}", _0)]
    HardwareWalletInternal(String),
    #[display(fmt = "No Trezor device available")]
    NoTrezorDeviceAvailable,
    #[display(fmt = "Unexpected Hardware Wallet device: {}", _0)]
    FoundUnexpectedDevice(String),
    /*                                         */
    /*------------- WithdrawError -------------*/
    /*                                         */
    #[display(
        fmt = "'{}' coin doesn't support 'init_withdraw' yet. Consider using 'withdraw' request instead",
        coin
    )]
    CoinDoesntSupportInitWithdraw { coin: String },
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
    #[display(fmt = "Balance is zero")]
    ZeroBalanceToWithdrawMax,
    #[display(fmt = "The amount {} is too small, required at least {}", amount, threshold)]
    AmountTooLow { amount: BigDecimal, threshold: BigDecimal },
    #[display(fmt = "Invalid address: {}", _0)]
    InvalidAddress(String),
    #[display(fmt = "Invalid fee policy: {}", _0)]
    InvalidFeePolicy(String),
    #[display(fmt = "No such coin {}", coin)]
    NoSuchCoin { coin: String },
    #[display(fmt = "Withdraw timed out {:?}", _0)]
    Timeout(Duration),
    #[display(fmt = "Unexpected user action. Expected '{}'", expected)]
    UnexpectedUserAction { expected: String },
    #[display(fmt = "Request should contain a 'from' address/account")]
    FromAddressNotFound,
    #[display(fmt = "Unexpected 'from' address: {}", _0)]
    UnexpectedFromAddress(String),
    #[display(fmt = "Unknown '{}' account", account_id)]
    UnknownAccount { account_id: u32 },
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    InternalError(String),
}

impl HttpStatusCode for WithdrawError {
    fn status_code(&self) -> StatusCode {
        match self {
            WithdrawError::NoSuchCoin { .. } => StatusCode::NOT_FOUND,
            WithdrawError::Timeout(_) => StatusCode::REQUEST_TIMEOUT,
            WithdrawError::CoinDoesntSupportInitWithdraw { .. }
            | WithdrawError::UnexpectedUserAction { .. }
            | WithdrawError::NotSufficientBalance { .. }
            | WithdrawError::ZeroBalanceToWithdrawMax
            | WithdrawError::AmountTooLow { .. }
            | WithdrawError::InvalidAddress(_)
            | WithdrawError::InvalidFeePolicy(_)
            | WithdrawError::FromAddressNotFound
            | WithdrawError::UnexpectedFromAddress(_)
            | WithdrawError::UnknownAccount { .. } => StatusCode::BAD_REQUEST,
            WithdrawError::NoTrezorDeviceAvailable
            | WithdrawError::TrezorDisconnected
            | WithdrawError::FoundUnexpectedDevice(_) => StatusCode::GONE,
            WithdrawError::HardwareWalletInternal(_)
            | WithdrawError::Transport(_)
            | WithdrawError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<NumConversError> for WithdrawError {
    fn from(e: NumConversError) -> Self { WithdrawError::InternalError(e.to_string()) }
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

impl From<UnexpectedDerivationMethod> for WithdrawError {
    fn from(e: UnexpectedDerivationMethod) -> Self { WithdrawError::InternalError(e.to_string()) }
}

impl From<PrivKeyNotAllowed> for WithdrawError {
    fn from(e: PrivKeyNotAllowed) -> Self { WithdrawError::InternalError(e.to_string()) }
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

#[derive(Serialize, Display, Debug, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum SignatureError {
    #[display(fmt = "Invalid request: {}", _0)]
    InvalidRequest(String),
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

impl From<keys::Error> for SignatureError {
    fn from(e: keys::Error) -> Self { SignatureError::InternalError(e.to_string()) }
}

impl From<ethkey::Error> for SignatureError {
    fn from(e: ethkey::Error) -> Self { SignatureError::InternalError(e.to_string()) }
}

impl From<PrivKeyNotAllowed> for SignatureError {
    fn from(e: PrivKeyNotAllowed) -> Self { SignatureError::InternalError(e.to_string()) }
}

impl From<CoinFindError> for SignatureError {
    fn from(e: CoinFindError) -> Self { SignatureError::CoinIsNotFound(e.to_string()) }
}

#[derive(Serialize, Display, Debug, SerializeErrorType)]
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
pub trait MmCoin: SwapOps + MarketCoinOps + fmt::Debug + Send + Sync + 'static {
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

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut;

    fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut;

    /// Maximum number of digits after decimal point used to denominate integer coin units (satoshis, wei, etc.)
    fn decimals(&self) -> u8;

    /// Convert input address to the specified address format.
    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String>;

    fn validate_address(&self, address: &str) -> ValidateAddressResult;

    /// Loop collecting coin transaction history and saving it to local DB
    fn process_history_loop(&self, ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send>;

    /// Path to tx history file
    fn tx_history_path(&self, ctx: &MmArc) -> PathBuf {
        let my_address = self.my_address().unwrap_or_default();
        // BCH cash address format has colon after prefix, e.g. bitcoincash:
        // Colon can't be used in file names on Windows so it should be escaped
        let my_address = my_address.replace(':', "_");
        ctx.dbdir()
            .join("TRANSACTIONS")
            .join(format!("{}_{}.json", self.ticker(), my_address))
    }

    /// Loads existing tx history from file, returns empty vector if file is not found
    /// Cleans the existing file if deserialization fails
    fn load_history_from_file(&self, ctx: &MmArc) -> TxHistoryFut<Vec<TransactionDetails>> {
        load_history_from_file_impl(self, ctx)
    }

    fn save_history_to_file(&self, ctx: &MmArc, history: Vec<TransactionDetails>) -> TxHistoryFut<()> {
        save_history_to_file_impl(self, ctx, history)
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
        dex_fee_amount: BigDecimal,
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

    /// The minimum number of confirmations at which a transaction is considered mature.
    fn mature_confirmations(&self) -> Option<u32>;

    /// Get some of the coin config info in serialized format for p2p messaging.
    fn coin_protocol_info(&self) -> Vec<u8>;

    /// Check if serialized coin protocol info is supported by current version.
    fn is_coin_protocol_supported(&self, info: &Option<Vec<u8>>) -> bool;
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum MmCoinEnum {
    UtxoCoin(UtxoStandardCoin),
    QtumCoin(QtumCoin),
    Qrc20Coin(Qrc20Coin),
    EthCoin(EthCoin),
    #[cfg(not(target_arch = "wasm32"))]
    ZCoin(ZCoin),
    Bch(BchCoin),
    SlpToken(SlpToken),
    #[cfg(not(target_arch = "wasm32"))]
    SolanaCoin(SolanaCoin),
    #[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
impl From<SolanaCoin> for MmCoinEnum {
    fn from(c: SolanaCoin) -> MmCoinEnum { MmCoinEnum::SolanaCoin(c) }
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
impl From<LightningCoin> for MmCoinEnum {
    fn from(c: LightningCoin) -> MmCoinEnum { MmCoinEnum::LightningCoin(c) }
}

#[cfg(not(target_arch = "wasm32"))]
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
            #[cfg(not(target_arch = "wasm32"))]
            MmCoinEnum::LightningCoin(ref c) => c,
            #[cfg(not(target_arch = "wasm32"))]
            MmCoinEnum::ZCoin(ref c) => c,
            MmCoinEnum::Test(ref c) => c,
            #[cfg(not(target_arch = "wasm32"))]
            MmCoinEnum::SolanaCoin(ref c) => c,
            #[cfg(not(target_arch = "wasm32"))]
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
}

#[async_trait]
pub trait BalanceTradeFeeUpdatedHandler {
    async fn balance_updated(&self, coin: &MmCoinEnum, new_balance: &BigDecimal);
}

pub struct CoinsContext {
    /// A map from a currency ticker symbol to the corresponding coin.
    /// Similar to `LP_coins`.
    coins: AsyncMutex<HashMap<String, MmCoinEnum>>,
    balance_update_handlers: AsyncMutex<Vec<Box<dyn BalanceTradeFeeUpdatedHandler + Send + Sync>>>,
    withdraw_task_manager: WithdrawTaskManagerShared,
    create_account_manager: CreateAccountTaskManagerShared,
    scan_addresses_manager: ScanAddressesTaskManagerShared,
    #[cfg(target_arch = "wasm32")]
    tx_history_db: SharedDb<TxHistoryDb>,
    #[cfg(target_arch = "wasm32")]
    hd_wallet_db: SharedDb<HDWalletDb>,
}

#[derive(Debug)]
pub struct CoinIsAlreadyActivatedErr {
    pub ticker: String,
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
                coins: AsyncMutex::new(HashMap::new()),
                balance_update_handlers: AsyncMutex::new(vec![]),
                withdraw_task_manager: WithdrawTaskManager::new_shared(),
                create_account_manager: CreateAccountTaskManager::new_shared(),
                scan_addresses_manager: ScanAddressesTaskManager::new_shared(),
                #[cfg(target_arch = "wasm32")]
                tx_history_db: ConstructibleDb::new_shared(ctx),
                #[cfg(target_arch = "wasm32")]
                hd_wallet_db: ConstructibleDb::new_shared(ctx),
            })
        })))
    }

    pub async fn add_coin(&self, coin: MmCoinEnum) -> Result<(), MmError<CoinIsAlreadyActivatedErr>> {
        let mut coins = self.coins.lock().await;
        if coins.contains_key(coin.ticker()) {
            return MmError::err(CoinIsAlreadyActivatedErr {
                ticker: coin.ticker().into(),
            });
        }

        coins.insert(coin.ticker().into(), coin);
        Ok(())
    }

    pub async fn add_platform_with_tokens(
        &self,
        platform: MmCoinEnum,
        tokens: Vec<MmCoinEnum>,
    ) -> Result<(), MmError<PlatformIsAlreadyActivatedErr>> {
        let mut coins = self.coins.lock().await;
        if coins.contains_key(platform.ticker()) {
            return MmError::err(PlatformIsAlreadyActivatedErr {
                ticker: platform.ticker().into(),
            });
        }

        coins.insert(platform.ticker().into(), platform);

        // Tokens can't be activated without platform coin so we can safely insert them without checking prior existence
        for token in tokens {
            coins.insert(token.ticker().into(), token);
        }
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    async fn tx_history_db(&self) -> TxHistoryResult<TxHistoryDbLocked<'_>> {
        Ok(self.tx_history_db.get_or_initialize().await?)
    }
}

/// This enum is used in coin activation requests.
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum PrivKeyActivationPolicy {
    IguanaPrivKey,
    Trezor,
}

impl PrivKeyActivationPolicy {
    /// The function can be used as a default deserialization constructor:
    /// `#[serde(default = "PrivKeyActivationPolicy::iguana_priv_key")]`
    pub fn iguana_priv_key() -> PrivKeyActivationPolicy { PrivKeyActivationPolicy::IguanaPrivKey }

    /// The function can be used as a default deserialization constructor:
    /// `#[serde(default = "PrivKeyActivationPolicy::trezor")]`
    pub fn trezor() -> PrivKeyActivationPolicy { PrivKeyActivationPolicy::Trezor }
}

#[derive(Debug)]
pub enum PrivKeyPolicy<T> {
    KeyPair(T),
    Trezor,
}

impl<T> PrivKeyPolicy<T> {
    pub fn key_pair(&self) -> Option<&T> {
        match self {
            PrivKeyPolicy::KeyPair(key_pair) => Some(key_pair),
            PrivKeyPolicy::Trezor => None,
        }
    }

    pub fn key_pair_or_err(&self) -> Result<&T, MmError<PrivKeyNotAllowed>> {
        self.key_pair()
            .or_mm_err(|| PrivKeyNotAllowed::HardwareWalletNotSupported)
    }
}

#[derive(Clone)]
pub enum PrivKeyBuildPolicy<'a> {
    IguanaPrivKey(&'a [u8]),
    Trezor,
}

impl<'a> PrivKeyBuildPolicy<'a> {
    pub fn iguana_priv_key(crypto_ctx: &'a CryptoCtx) -> Self {
        PrivKeyBuildPolicy::IguanaPrivKey(crypto_ctx.iguana_ctx().secp256k1_privkey_bytes())
    }
}

#[derive(Debug)]
pub enum DerivationMethod<Address, HDWallet> {
    Iguana(Address),
    HDWallet(HDWallet),
}

impl<Address, HDWallet> DerivationMethod<Address, HDWallet> {
    pub fn iguana(&self) -> Option<&Address> {
        match self {
            DerivationMethod::Iguana(my_address) => Some(my_address),
            DerivationMethod::HDWallet(_) => None,
        }
    }

    pub fn iguana_or_err(&self) -> MmResult<&Address, UnexpectedDerivationMethod> {
        self.iguana()
            .or_mm_err(|| UnexpectedDerivationMethod::IguanaPrivKeyUnavailable)
    }

    pub fn hd_wallet(&self) -> Option<&HDWallet> {
        match self {
            DerivationMethod::Iguana(_) => None,
            DerivationMethod::HDWallet(hd_wallet) => Some(hd_wallet),
        }
    }

    pub fn hd_wallet_or_err(&self) -> MmResult<&HDWallet, UnexpectedDerivationMethod> {
        self.hd_wallet()
            .or_mm_err(|| UnexpectedDerivationMethod::HDWalletUnavailable)
    }

    /// # Panic
    ///
    /// Panic if the address mode is [`DerivationMethod::HDWallet`].
    pub fn unwrap_iguana(&self) -> &Address { self.iguana_or_err().unwrap() }
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
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    #[cfg(not(target_arch = "wasm32"))]
    LIGHTNING {
        platform: String,
        network: BlockchainNetwork,
        confirmations: PlatformCoinConfirmations,
    },
    #[cfg(not(target_arch = "wasm32"))]
    SOLANA,
    #[cfg(not(target_arch = "wasm32"))]
    SPLTOKEN {
        platform: String,
        token_contract_address: String,
        decimals: u8,
    },
    #[cfg(not(target_arch = "wasm32"))]
    ZHTLC {
        consensus_params: ZcoinConsensusParams,
    },
}

pub type RpcTransportEventHandlerShared = Arc<dyn RpcTransportEventHandler + Send + Sync + 'static>;

/// Common methods to measure the outgoing requests and incoming responses statistics.
pub trait RpcTransportEventHandler {
    fn debug_info(&self) -> String;

    fn on_outgoing_request(&self, data: &[u8]);

    fn on_incoming_response(&self, data: &[u8]);

    fn on_connected(&self, address: String) -> Result<(), String>;
}

impl fmt::Debug for dyn RpcTransportEventHandler + Send + Sync {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.debug_info()) }
}

impl RpcTransportEventHandler for RpcTransportEventHandlerShared {
    fn debug_info(&self) -> String { self.deref().debug_info() }

    fn on_outgoing_request(&self, data: &[u8]) { self.as_ref().on_outgoing_request(data) }

    fn on_incoming_response(&self, data: &[u8]) { self.as_ref().on_incoming_response(data) }

    fn on_connected(&self, address: String) -> Result<(), String> { self.as_ref().on_connected(address) }
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
            "coin" => self.ticker.clone(), "client" => self.client.clone());
        mm_counter!(self.metrics, "rpc_client.request.count", 1,
            "coin" => self.ticker.clone(), "client" => self.client.clone());
    }

    fn on_incoming_response(&self, data: &[u8]) {
        mm_counter!(self.metrics, "rpc_client.traffic.in", data.len() as u64,
            "coin" => self.ticker.clone(), "client" => self.client.clone());
        mm_counter!(self.metrics, "rpc_client.response.count", 1,
            "coin" => self.ticker.clone(), "client" => self.client.clone());
    }

    fn on_connected(&self, _address: String) -> Result<(), String> {
        // Handle a new connected endpoint if necessary.
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

    if coins_en.is_null() {
        ctx.log.log(
            "😅",
            #[allow(clippy::unnecessary_cast)]
            &[&("coin" as &str), &ticker, &("no-conf" as &str)],
            &fomat! ("Warning, coin " (ticker) " is used without a corresponding configuration."),
        );
    }

    if coins_en["mm2"].is_null() && req["mm2"].is_null() {
        return ERR!(concat!(
            "mm2 param is not set neither in coins config nor enable request, ",
            "assuming that coin is not supported"
        ));
    }
    let secret = try_s!(CryptoCtx::from_ctx(ctx))
        .iguana_ctx()
        .secp256k1_privkey_bytes()
        .to_vec();

    if coins_en["protocol"].is_null() {
        return ERR!(
            r#""protocol" field is missing in coins file. The file format is deprecated, please execute ./mm2 update_config command to convert it or download a new one"#
        );
    }
    let protocol: CoinProtocol = try_s!(json::from_value(coins_en["protocol"].clone()));

    let coin: MmCoinEnum = match &protocol {
        CoinProtocol::UTXO => {
            let params = try_s!(UtxoActivationParams::from_legacy_req(req));
            try_s!(utxo_standard_coin_with_priv_key(ctx, ticker, &coins_en, &params, &secret).await).into()
        },
        CoinProtocol::QTUM => {
            let params = try_s!(UtxoActivationParams::from_legacy_req(req));
            try_s!(qtum_coin_with_priv_key(ctx, ticker, &coins_en, &params, &secret).await).into()
        },
        CoinProtocol::ETH | CoinProtocol::ERC20 { .. } => {
            try_s!(eth_coin_from_conf_and_request(ctx, ticker, &coins_en, req, &secret, protocol).await).into()
        },
        CoinProtocol::QRC20 {
            platform,
            contract_address,
        } => {
            let params = try_s!(Qrc20ActivationParams::from_legacy_req(req));
            let contract_address = try_s!(qtum::contract_addr_from_str(contract_address));

            try_s!(
                qrc20_coin_from_conf_and_params(ctx, ticker, platform, &coins_en, &params, &secret, contract_address)
                    .await
            )
            .into()
        },
        CoinProtocol::BCH { slp_prefix } => {
            let prefix = try_s!(CashAddrPrefix::from_str(slp_prefix));
            let params = try_s!(BchActivationRequest::from_legacy_req(req));

            let bch = try_s!(bch_coin_from_conf_and_params(ctx, ticker, &coins_en, params, prefix, &secret).await);
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
            let token = SlpToken::new(*decimals, ticker.into(), (*token_id).into(), platform_coin, confs);
            token.into()
        },
        #[cfg(not(target_arch = "wasm32"))]
        CoinProtocol::ZHTLC { .. } => return ERR!("ZHTLC protocol is not supported by lp_coininit"),
        #[cfg(not(target_arch = "wasm32"))]
        CoinProtocol::LIGHTNING { .. } => return ERR!("Lightning protocol is not supported by lp_coininit"),
        #[cfg(not(target_arch = "wasm32"))]
        CoinProtocol::SOLANA => {
            return ERR!("Solana protocol is not supported by lp_coininit - use enable_solana_with_tokens instead")
        },
        #[cfg(not(target_arch = "wasm32"))]
        CoinProtocol::SPLTOKEN { .. } => {
            return ERR!("SplToken protocol is not supported by lp_coininit - use enable_spl instead")
        },
    };

    let register_params = RegisterCoinParams {
        ticker: ticker.to_owned(),
        tx_history: req["tx_history"].as_bool().unwrap_or(false),
    };
    try_s!(lp_register_coin(ctx, coin.clone(), register_params).await);
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
    pub tx_history: bool,
}

pub async fn lp_register_coin(
    ctx: &MmArc,
    coin: MmCoinEnum,
    params: RegisterCoinParams,
) -> Result<(), MmError<RegisterCoinError>> {
    let RegisterCoinParams { ticker, tx_history } = params;
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
        RawEntryMut::Vacant(ve) => ve.insert(ticker.clone(), coin.clone()),
    };
    if tx_history {
        lp_spawn_tx_history(ctx.clone(), coin).map_to_mm(RegisterCoinError::Internal)?;
    }
    let ctx_weak = ctx.weak();
    spawn(async move { check_balance_update_loop(ctx_weak, ticker).await });
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn lp_spawn_tx_history(ctx: MmArc, coin: MmCoinEnum) -> Result<(), String> {
    try_s!(std::thread::Builder::new()
        .name(format!("tx_history_{}", coin.ticker()))
        .spawn(move || coin.process_history_loop(ctx).wait()));
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn lp_spawn_tx_history(ctx: MmArc, coin: MmCoinEnum) -> Result<(), String> {
    let fut = async move {
        let _res = coin.process_history_loop(ctx).compat().await;
    };
    common::executor::spawn_local(fut);
    Ok(())
}

/// NB: Returns only the enabled (aka active) coins.
pub async fn lp_coinfind(ctx: &MmArc, ticker: &str) -> Result<Option<MmCoinEnum>, String> {
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

#[derive(Clone, Debug, Serialize)]
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

#[derive(Serialize)]
struct EnabledCoin {
    ticker: String,
    address: String,
}

pub async fn get_enabled_coins(ctx: MmArc) -> Result<Response<Vec<u8>>, String> {
    let coins_ctx: Arc<CoinsContext> = try_s!(CoinsContext::from_ctx(&ctx));
    let coins = coins_ctx.coins.lock().await;
    let enabled_coins: Vec<_> = try_s!(coins
        .iter()
        .map(|(ticker, coin)| {
            let address = try_s!(coin.my_address());
            Ok(EnabledCoin {
                ticker: ticker.clone(),
                address,
            })
        })
        .collect());

    let res = try_s!(json::to_vec(&json!({ "result": enabled_coins })));
    Ok(try_s!(Response::builder().body(res)))
}

pub async fn disable_coin(ctx: &MmArc, ticker: &str) -> Result<(), String> {
    let coins_ctx = try_s!(CoinsContext::from_ctx(ctx));
    let mut coins = coins_ctx.coins.lock().await;
    match coins.remove(ticker) {
        Some(_) => Ok(()),
        None => ERR!("{} is disabled already", ticker),
    }
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

// TODO: Refactor this, it's actually not required to check balance and trade fee when there no orders using the coin
pub async fn check_balance_update_loop(ctx: MmWeak, ticker: String) {
    let mut current_balance = None;
    loop {
        Timer::sleep(10.).await;
        let ctx = match MmArc::from_weak(&ctx) {
            Some(ctx) => ctx,
            None => return,
        };

        match lp_coinfind(&ctx, &ticker).await {
            Ok(Some(coin)) => {
                let balance = match coin.my_spendable_balance().compat().await {
                    Ok(balance) => balance,
                    Err(_) => continue,
                };
                if Some(&balance) != current_balance.as_ref() {
                    let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
                    coins_ctx.balance_updated(&coin, &balance).await;
                    current_balance = Some(balance);
                }
            },
            Ok(None) => break,
            Err(_) => continue,
        }
    }
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
        #[cfg(not(target_arch = "wasm32"))]
        CoinProtocol::LIGHTNING { .. } => {
            ERR!("address_by_coin_conf_and_pubkey_str is not implemented for lightning protocol yet!")
        },
        #[cfg(not(target_arch = "wasm32"))]
        CoinProtocol::SOLANA | CoinProtocol::SPLTOKEN { .. } => {
            ERR!("Solana pubkey is the public address - you do not need to use this rpc call.")
        },
        #[cfg(not(target_arch = "wasm32"))]
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
    let my_address = try_f!(coin.my_address().map_to_mm(TxHistoryError::InternalError));

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
fn save_history_to_file_impl<T>(coin: &T, ctx: &MmArc, history: Vec<TransactionDetails>) -> TxHistoryFut<()>
where
    T: MmCoin + MarketCoinOps + ?Sized,
{
    let ctx = ctx.clone();
    let ticker = coin.ticker().to_owned();
    let my_address = try_f!(coin.my_address().map_to_mm(TxHistoryError::InternalError));

    let fut = async move {
        let coins_ctx = CoinsContext::from_ctx(&ctx).unwrap();
        let db = coins_ctx.tx_history_db().await?;
        save_tx_history(&db, &ticker, &my_address, history).await?;
        Ok(())
    };
    Box::new(fut.boxed().compat())
}

#[cfg(not(target_arch = "wasm32"))]
fn save_history_to_file_impl<T>(coin: &T, ctx: &MmArc, history: Vec<TransactionDetails>) -> TxHistoryFut<()>
where
    T: MmCoin + MarketCoinOps + ?Sized,
{
    let history_path = coin.tx_history_path(ctx);
    let tmp_file = format!("{}.tmp", history_path.display());

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
