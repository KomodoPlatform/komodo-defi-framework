/******************************************************************************
 * Copyright © 2014-2019 The SuperNET Developers.                             *
 *                                                                            *
 * See the AUTHORS, DEVELOPER-AGREEMENT and LICENSE files at                  *
 * the top-level directory of this distribution for the individual copyright  *
 * holder information and the developer policies on copyright and licensing.  *
 *                                                                            *
 * Unless otherwise agreed in a custom licensing agreement, no part of the    *
 * SuperNET software, including this file may be copied, modified, propagated *
 * or distributed except according to the terms contained in the LICENSE file *
 *                                                                            *
 * Removal or modification of this copyright notice is prohibited.            *
 *                                                                            *
 ******************************************************************************/
//
//  utxo.rs
//  marketmaker
//
//  Copyright © 2017-2019 SuperNET. All rights reserved.
//

pub mod bch;
pub mod bch_and_slp_tx_history;
mod bchd_grpc;
#[allow(clippy::all)]
#[rustfmt::skip]
#[path = "utxo/pb.rs"]
mod bchd_pb;
pub mod qtum;
pub mod rpc_clients;
pub mod slp;
pub mod utxo_block_header_storage;
pub mod utxo_common;
pub mod utxo_standard;
pub mod utxo_withdraw;

#[cfg(not(target_arch = "wasm32"))] pub mod tx_cache;
#[cfg(not(target_arch = "wasm32"))]
pub mod utxo_sql_block_header_storage;

use async_trait::async_trait;
use bigdecimal::BigDecimal;
use bitcoin::network::constants::Network as BitcoinNetwork;
pub use bitcrypto::{dhash160, sha256, ChecksumType};
use chain::{OutPoint, TransactionOutput, TxHashAlgo};
use common::executor::{spawn, Timer};
#[cfg(not(target_arch = "wasm32"))]
use common::first_char_to_upper;
use common::jsonrpc_client::JsonRpcError;
use common::mm_ctx::MmArc;
use common::mm_error::prelude::*;
use common::mm_metrics::MetricsArc;
use common::{now_ms, small_rng};
use crypto::trezor::TrezorCoin;
use crypto::{Bip32Error, ChildNumber, DerivationPath};
use derive_more::Display;
#[cfg(not(target_arch = "wasm32"))] use dirs::home_dir;
use futures::channel::mpsc;
use futures::compat::Future01CompatExt;
use futures::lock::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};
use futures::stream::StreamExt;
use futures01::Future;
use keys::bytes::Bytes;
pub use keys::{Address, AddressFormat as UtxoAddressFormat, AddressHashEnum, KeyPair, Private, Public, Secret,
               Type as ScriptType};
#[cfg(test)] use mocktopus::macros::*;
use num_traits::ToPrimitive;
use primitives::hash::{H256, H264};
use rand::seq::SliceRandom;
use rpc::v1::types::{Bytes as BytesJson, Transaction as RpcTransaction, H256 as H256Json};
use script::{Builder, Script, SignatureVersion, TransactionInputSigner};
use serde_json::{self as json, Value as Json};
use serialization::{serialize, serialize_with_flags, SERIALIZE_TRANSACTION_WITNESS};
use std::collections::{HashMap, HashSet};
use std::convert::TryInto;
use std::hash::Hash;
use std::num::NonZeroU64;
use std::ops::Deref;
#[cfg(not(target_arch = "wasm32"))] use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, Weak};
use utxo_common::{big_decimal_from_sat, UtxoMergeParams, UtxoTxBuilder};
use utxo_signer::with_key_pair::sign_tx;
use utxo_signer::{TxProvider, TxProviderError, UtxoSignTxError, UtxoSignTxResult};

pub use chain::Transaction as UtxoTx;

#[cfg(not(target_arch = "wasm32"))]
use self::rpc_clients::{ConcurrentRequestMap, NativeClient, NativeClientImpl};
use self::rpc_clients::{ElectrumClient, ElectrumClientImpl, ElectrumRpcRequest, EstimateFeeMethod, EstimateFeeMode,
                        UnspentInfo, UtxoRpcClientEnum, UtxoRpcError, UtxoRpcFut, UtxoRpcResult};
use super::{BalanceError, BalanceFut, BalanceResult, CoinTransportMetrics, CoinsContext, DerivationMethod,
            DerivationMethodNotSupported, FeeApproxStage, FoundSwapTxSpend, HistorySyncState, KmdRewardsDetails,
            MarketCoinOps, MmCoin, NumConversError, NumConversResult, PrivKeyNotAllowed, PrivKeyPolicy, RpcClientType,
            RpcTransportEventHandler, RpcTransportEventHandlerShared, TradeFee, TradePreimageError, TradePreimageFut,
            TradePreimageResult, Transaction, TransactionDetails, TransactionEnum, TransactionFut, WithdrawError,
            WithdrawRequest};
use std::array::TryFromSliceError;

#[cfg(test)] pub mod utxo_tests;
#[cfg(target_arch = "wasm32")] pub mod utxo_wasm_tests;

const KILO_BYTE: u64 = 1000;
/// https://bitcoin.stackexchange.com/a/77192
const MAX_DER_SIGNATURE_LEN: usize = 72;
const COMPRESSED_PUBKEY_LEN: usize = 33;
const P2PKH_OUTPUT_LEN: u64 = 34;
const MATURE_CONFIRMATIONS_DEFAULT: u32 = 100;
const UTXO_DUST_AMOUNT: u64 = 1000;
/// Block count for KMD median time past calculation
///
/// # Safety
/// 11 > 0
const KMD_MTP_BLOCK_COUNT: NonZeroU64 = unsafe { NonZeroU64::new_unchecked(11u64) };
const DEFAULT_DYNAMIC_FEE_VOLATILITY_PERCENT: f64 = 0.5;
const BIP44_PURPOSE: u32 = 44;

pub type GenerateTxResult = Result<(TransactionInputSigner, AdditionalTxData), MmError<GenerateTxError>>;
pub type HistoryUtxoTxMap = HashMap<H256Json, HistoryUtxoTx>;
pub type UtxoConfResult<T> = Result<T, MmError<UtxoConfError>>;
pub type UtxoCoinBuildResult<T> = Result<T, MmError<UtxoCoinBuildError>>;

#[derive(Debug, Display)]
pub enum UtxoConfError {
    #[display(fmt = "'name' field is not found in config")]
    CurrencyNameIsNotSet,
    #[display(fmt = "Invalid 'derivation_path' purpose '{}'. BIP44 is supported only", purpose)]
    InvalidDerivationPathPurpose {
        purpose: u32,
    },
    #[display(
        fmt = "Invalid length '{}' of 'derivation_path'. Expected \"m/purpose'/coin_type'/\" path, i.e 2 children",
        found_children
    )]
    InvalidDerivationPathLen {
        found_children: usize,
    },
    #[display(fmt = "Error deserializing 'derivation_path': {}", _0)]
    ErrorDeserializingDerivationPath(String),
    InvalidConsensusBranchId(String),
    InvalidVersionGroupId(String),
    InvalidAddressFormat(String),
    InvalidBlockchainNetwork(String),
    InvalidDecimals(String),
}

impl From<Bip32Error> for UtxoConfError {
    fn from(e: Bip32Error) -> Self { UtxoConfError::ErrorDeserializingDerivationPath(e.to_string()) }
}

#[derive(Debug, Display)]
pub enum UtxoCoinBuildError {
    ConfError(UtxoConfError),
    #[display(fmt = "Native RPC client is only supported in native mode")]
    NativeRpcNotSupportedInWasm,
    ErrorReadingNativeModeConf(String),
    #[display(fmt = "Rpc port is not set neither in `coins` file nor in native daemon config")]
    RpcPortIsNotSet,
    ErrorDetectingFeeMethod(String),
    ErrorDetectingDecimals(String),
    #[display(
        fmt = "Failed to connect to at least 1 of {:?} in {} seconds.",
        electrum_servers,
        seconds
    )]
    FailedToConnectToElectrums {
        electrum_servers: Vec<ElectrumRpcRequest>,
        seconds: u64,
    },
    ElectrumProtocolVersionCheckError(String),
    #[display(fmt = "Can not detect the user home directory")]
    CantDetectUserHome,
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl From<UtxoConfError> for UtxoCoinBuildError {
    fn from(e: UtxoConfError) -> Self { UtxoCoinBuildError::ConfError(e) }
}

#[cfg(windows)]
#[cfg(not(target_arch = "wasm32"))]
fn get_special_folder_path() -> PathBuf {
    use libc::c_char;
    use std::ffi::CStr;
    use std::mem::zeroed;
    use std::ptr::null_mut;
    use winapi::shared::minwindef::MAX_PATH;
    use winapi::um::shlobj::SHGetSpecialFolderPathA;
    use winapi::um::shlobj::CSIDL_APPDATA;

    let mut buf: [c_char; MAX_PATH + 1] = unsafe { zeroed() };
    // https://docs.microsoft.com/en-us/windows/desktop/api/shlobj_core/nf-shlobj_core-shgetspecialfolderpatha
    let rc = unsafe { SHGetSpecialFolderPathA(null_mut(), buf.as_mut_ptr(), CSIDL_APPDATA, 1) };
    if rc != 1 {
        panic!("!SHGetSpecialFolderPathA")
    }
    Path::new(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str().unwrap()).to_path_buf()
}

#[cfg(not(windows))]
#[cfg(not(target_arch = "wasm32"))]
fn get_special_folder_path() -> PathBuf { panic!("!windows") }

impl Transaction for UtxoTx {
    fn tx_hex(&self) -> Vec<u8> {
        if self.has_witness() {
            serialize_with_flags(self, SERIALIZE_TRANSACTION_WITNESS).into()
        } else {
            serialize(self).into()
        }
    }

    fn tx_hash(&self) -> BytesJson { self.hash().reversed().to_vec().into() }
}

impl From<JsonRpcError> for BalanceError {
    fn from(e: JsonRpcError) -> Self { BalanceError::Transport(e.to_string()) }
}

impl From<UtxoRpcError> for BalanceError {
    fn from(e: UtxoRpcError) -> Self {
        match e {
            UtxoRpcError::Internal(desc) => BalanceError::Internal(desc),
            _ => BalanceError::Transport(e.to_string()),
        }
    }
}

impl From<UtxoRpcError> for WithdrawError {
    fn from(e: UtxoRpcError) -> Self {
        match e {
            UtxoRpcError::Transport(transport) | UtxoRpcError::ResponseParseError(transport) => {
                WithdrawError::Transport(transport.to_string())
            },
            UtxoRpcError::InvalidResponse(resp) => WithdrawError::Transport(resp),
            UtxoRpcError::Internal(internal) => WithdrawError::InternalError(internal),
        }
    }
}

impl From<JsonRpcError> for TradePreimageError {
    fn from(e: JsonRpcError) -> Self { TradePreimageError::Transport(e.to_string()) }
}

impl From<UtxoRpcError> for TradePreimageError {
    fn from(e: UtxoRpcError) -> Self {
        match e {
            UtxoRpcError::Transport(transport) | UtxoRpcError::ResponseParseError(transport) => {
                TradePreimageError::Transport(transport.to_string())
            },
            UtxoRpcError::InvalidResponse(resp) => TradePreimageError::Transport(resp),
            UtxoRpcError::Internal(internal) => TradePreimageError::InternalError(internal),
        }
    }
}

impl From<UtxoRpcError> for TxProviderError {
    fn from(rpc: UtxoRpcError) -> Self {
        match rpc {
            resp @ UtxoRpcError::ResponseParseError(_) | resp @ UtxoRpcError::InvalidResponse(_) => {
                TxProviderError::InvalidResponse(resp.to_string())
            },
            UtxoRpcError::Transport(transport) => TxProviderError::Transport(transport.to_string()),
            UtxoRpcError::Internal(internal) => TxProviderError::Internal(internal),
        }
    }
}

#[async_trait]
impl TxProvider for UtxoRpcClientEnum {
    async fn get_rpc_transaction(&self, tx_hash: &H256Json) -> Result<RpcTransaction, MmError<TxProviderError>> {
        Ok(self.get_verbose_transaction(tx_hash).compat().await?)
    }
}

/// The `UtxoTx` with the block height transaction mined in.
pub struct HistoryUtxoTx {
    pub height: Option<u64>,
    pub tx: UtxoTx,
}

/// Additional transaction data that can't be easily got from raw transaction without calling
/// additional RPC methods, e.g. to get input amount we need to request all previous transactions
/// and check output values
#[derive(Debug)]
pub struct AdditionalTxData {
    pub received_by_me: u64,
    pub spent_by_me: u64,
    pub fee_amount: u64,
    pub unused_change: Option<u64>,
    pub kmd_rewards: Option<KmdRewardsDetails>,
}

/// The fee set from coins config
#[derive(Debug)]
pub enum TxFee {
    /// Tell the coin that it should request the fee from daemon RPC and calculate it relying on tx size
    Dynamic(EstimateFeeMethod),
    /// Tell the coin that it has fixed tx fee per kb.
    FixedPerKb(u64),
}

/// The actual "runtime" fee that is received from RPC in case of dynamic calculation
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ActualTxFee {
    /// fee amount per Kbyte received from coin RPC
    Dynamic(u64),
    /// Use specified amount per each 1 kb of transaction and also per each output less than amount.
    /// Used by DOGE, but more coins might support it too.
    FixedPerKb(u64),
}

/// Fee policy applied on transaction creation
pub enum FeePolicy {
    /// Send the exact amount specified in output(s), fee is added to spent input amount
    SendExact,
    /// Contains the index of output from which fee should be deducted
    DeductFromOutput(usize),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CachedUnspentInfo {
    pub outpoint: OutPoint,
    pub value: u64,
}

impl From<UnspentInfo> for CachedUnspentInfo {
    fn from(unspent: UnspentInfo) -> CachedUnspentInfo {
        CachedUnspentInfo {
            outpoint: unspent.outpoint,
            value: unspent.value,
        }
    }
}

impl From<CachedUnspentInfo> for UnspentInfo {
    fn from(cached: CachedUnspentInfo) -> UnspentInfo {
        UnspentInfo {
            outpoint: cached.outpoint,
            value: cached.value,
            height: None,
        }
    }
}

/// The cache of recently send transactions used to track the spent UTXOs and replace them with new outputs
/// The daemon needs some time to update the listunspent list for address which makes it return already spent UTXOs
/// This cache helps to prevent UTXO reuse in such cases
pub struct RecentlySpentOutPoints {
    /// Maps CachedUnspentInfo A to a set of CachedUnspentInfo which `spent` A
    input_to_output_map: HashMap<CachedUnspentInfo, HashSet<CachedUnspentInfo>>,
    /// Maps CachedUnspentInfo A to a set of CachedUnspentInfo that `were spent by` A
    output_to_input_map: HashMap<CachedUnspentInfo, HashSet<CachedUnspentInfo>>,
    /// Cache includes only outputs having script_pubkey == for_script_pubkey
    for_script_pubkey: Bytes,
}

impl RecentlySpentOutPoints {
    fn new(for_script_pubkey: Bytes) -> Self {
        RecentlySpentOutPoints {
            input_to_output_map: HashMap::new(),
            output_to_input_map: HashMap::new(),
            for_script_pubkey,
        }
    }

    pub fn add_spent(&mut self, inputs: Vec<UnspentInfo>, spend_tx_hash: H256, outputs: Vec<TransactionOutput>) {
        let inputs: HashSet<_> = inputs.into_iter().map(From::from).collect();
        let to_replace: HashSet<_> = outputs
            .iter()
            .enumerate()
            .filter_map(|(index, output)| {
                if output.script_pubkey == self.for_script_pubkey {
                    Some(CachedUnspentInfo {
                        outpoint: OutPoint {
                            hash: spend_tx_hash,
                            index: index as u32,
                        },
                        value: output.value,
                    })
                } else {
                    None
                }
            })
            .collect();

        let mut prev_inputs_spent = HashSet::new();

        // check if inputs are already in spending cached chain
        for input in &inputs {
            if let Some(prev_inputs) = self.output_to_input_map.get(input) {
                for prev_input in prev_inputs {
                    if let Some(outputs) = self.input_to_output_map.get_mut(prev_input) {
                        prev_inputs_spent.insert(prev_input.clone());
                        outputs.remove(input);
                        for replace in &to_replace {
                            outputs.insert(replace.clone());
                        }
                    }
                }
            }
        }

        prev_inputs_spent.extend(inputs.clone());
        for output in &to_replace {
            self.output_to_input_map
                .insert(output.clone(), prev_inputs_spent.clone());
        }

        for input in inputs {
            self.input_to_output_map.insert(input, to_replace.clone());
        }
    }

    pub fn replace_spent_outputs_with_cache(&self, mut outputs: HashSet<UnspentInfo>) -> HashSet<UnspentInfo> {
        let mut replacement_unspents = HashSet::new();
        outputs = outputs
            .into_iter()
            .filter(|unspent| {
                let outs = self.input_to_output_map.get(&unspent.clone().into());
                match outs {
                    Some(outs) => {
                        for out in outs.iter() {
                            if !replacement_unspents.contains(out) {
                                replacement_unspents.insert(out.clone());
                            }
                        }
                        false
                    },
                    None => true,
                }
            })
            .collect();
        if replacement_unspents.is_empty() {
            return outputs;
        }
        outputs.extend(replacement_unspents.into_iter().map(From::from));
        self.replace_spent_outputs_with_cache(outputs)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub enum BlockchainNetwork {
    #[serde(rename = "mainnet")]
    Mainnet,
    #[serde(rename = "testnet")]
    Testnet,
    #[serde(rename = "regtest")]
    Regtest,
}

impl From<BlockchainNetwork> for BitcoinNetwork {
    fn from(network: BlockchainNetwork) -> Self {
        match network {
            BlockchainNetwork::Mainnet => BitcoinNetwork::Bitcoin,
            BlockchainNetwork::Testnet => BitcoinNetwork::Testnet,
            BlockchainNetwork::Regtest => BitcoinNetwork::Regtest,
        }
    }
}

#[derive(Debug)]
pub struct UtxoCoinConf {
    pub ticker: String,
    /// https://en.bitcoin.it/wiki/List_of_address_prefixes
    /// https://github.com/jl777/coins/blob/master/coins
    pub pub_addr_prefix: u8,
    pub p2sh_addr_prefix: u8,
    pub wif_prefix: u8,
    pub pub_t_addr_prefix: u8,
    pub p2sh_t_addr_prefix: u8,
    // https://github.com/bitcoin/bips/blob/master/bip-0173.mediawiki#Segwit_address_format
    pub bech32_hrp: Option<String>,
    /// True if coins uses Proof of Stake consensus algo
    /// Proof of Work is expected by default
    /// https://en.bitcoin.it/wiki/Proof_of_Stake
    /// https://en.bitcoin.it/wiki/Proof_of_work
    /// The actual meaning of this is nTime field is used in transaction
    pub is_pos: bool,
    /// Special field for Zcash and it's forks
    /// Defines if Overwinter network upgrade was activated
    /// https://z.cash/upgrade/overwinter/
    pub overwintered: bool,
    /// The tx version used to detect the transaction ser/de/signing algo
    /// For now it's mostly used for Zcash and forks because they changed the algo in
    /// Overwinter and then Sapling upgrades
    /// https://github.com/zcash/zips/blob/master/zip-0243.rst
    pub tx_version: i32,
    /// If true - allow coins withdraw to P2SH addresses (Segwit).
    /// the flag will also affect the address that MM2 generates by default in the future
    /// will be the Segwit (starting from 3 for BTC case) instead of legacy
    /// https://en.bitcoin.it/wiki/Segregated_Witness
    pub segwit: bool,
    /// Does coin require transactions to be notarized to be considered as confirmed?
    /// https://komodoplatform.com/security-delayed-proof-of-work-dpow/
    pub requires_notarization: AtomicBool,
    /// The address format indicates the default address format from coin config file
    pub default_address_format: UtxoAddressFormat,
    /// Is current coin KMD asset chain?
    /// https://komodoplatform.atlassian.net/wiki/spaces/KPSD/pages/71729160/What+is+a+Parallel+Chain+Asset+Chain
    pub asset_chain: bool,
    /// Dynamic transaction fee volatility in percent. The value is used to predict a possible increase in dynamic fee.
    pub tx_fee_volatility_percent: f64,
    /// Transaction version group id for Zcash transactions since Overwinter: https://github.com/zcash/zips/blob/master/zip-0202.rst
    pub version_group_id: u32,
    /// Consensus branch id for Zcash transactions since Overwinter: https://github.com/zcash/zcash/blob/master/src/consensus/upgrades.cpp#L11
    /// used in transaction sig hash calculation
    pub consensus_branch_id: u32,
    /// Defines if coin uses Zcash transaction format
    pub zcash: bool,
    /// Address and privkey checksum type
    pub checksum_type: ChecksumType,
    /// Fork id used in sighash
    pub fork_id: u32,
    /// Signature version
    pub signature_version: SignatureVersion,
    pub required_confirmations: AtomicU64,
    /// if set to true MM2 will check whether calculated fee is lower than relay fee and use
    /// relay fee amount instead of calculated
    /// https://github.com/KomodoPlatform/atomicDEX-API/issues/617
    pub force_min_relay_fee: bool,
    /// Block count for median time past calculation
    pub mtp_block_count: NonZeroU64,
    pub estimate_fee_mode: Option<EstimateFeeMode>,
    /// The minimum number of confirmations at which a transaction is considered mature
    pub mature_confirmations: u32,
    /// The number of blocks used for estimate_fee/estimate_smart_fee RPC calls
    pub estimate_fee_blocks: u32,
    /// Defines if the coin can be used in the lightning network
    /// For now BTC is only supported by LDK but in the future any segwit coins can be supported in lightning network
    pub lightning: bool,
    /// bitcoin/testnet/signet/regtest Needed for lightning node to know which network to connect to
    pub network: Option<String>,
    /// The name of the coin with which Trezor wallet associates this asset.
    pub trezor_coin: Option<TrezorCoin>,
    /// Derivation path used to derive addresses from a master key.
    pub derivation_path: Option<DerivationPath>,
}

#[derive(Debug)]
pub struct UtxoCoinFields {
    /// UTXO coin config
    pub conf: UtxoCoinConf,
    /// Default decimals amount is 8 (BTC and almost all other UTXO coins)
    /// But there are forks which have different decimals:
    /// Peercoin has 6
    /// Emercoin has 6
    /// Bitcoin Diamond has 7
    pub decimals: u8,
    pub tx_fee: TxFee,
    /// Minimum transaction value at which the value is not less than fee
    pub dust_amount: u64,
    /// RPC client
    pub rpc_client: UtxoRpcClientEnum,
    /// Either ECDSA key pair or a Hardware Wallet info.
    pub priv_key_policy: PrivKeyPolicy<KeyPair>,
    /// Either an Iguana address or an info about last derived account/address.
    pub derivation_method: DerivationMethod<Address, HDWalletInfo>,
    pub history_sync_state: Mutex<HistorySyncState>,
    /// Path to the TX cache directory
    pub tx_cache_directory: Option<PathBuf>,
    /// The cache of recently send transactions used to track the spent UTXOs and replace them with new outputs
    /// The daemon needs some time to update the listunspent list for address which makes it return already spent UTXOs
    /// This cache helps to prevent UTXO reuse in such cases
    pub recently_spent_outpoints: AsyncMutex<RecentlySpentOutPoints>,
    pub tx_hash_algo: TxHashAlgo,
}

#[derive(Debug, Display)]
pub enum UnsupportedAddr {
    #[display(
        fmt = "{} address format activated for {}, but {} format used instead",
        activated_format,
        ticker,
        used_format
    )]
    FormatMismatch {
        ticker: String,
        activated_format: String,
        used_format: String,
    },
    #[display(fmt = "Expected a valid P2PKH or P2SH prefix for {}", _0)]
    PrefixError(String),
    #[display(fmt = "Address hrp {} is not a valid hrp for {}", hrp, ticker)]
    HrpError { ticker: String, hrp: String },
    #[display(fmt = "Segwit not activated in the config for {}", _0)]
    SegwitNotActivated(String),
}

impl From<UnsupportedAddr> for WithdrawError {
    fn from(e: UnsupportedAddr) -> Self { WithdrawError::InvalidAddress(e.to_string()) }
}

impl UtxoCoinFields {
    pub fn transaction_preimage(&self) -> TransactionInputSigner {
        let lock_time = if self.conf.ticker == "KMD" {
            (now_ms() / 1000) as u32 - 3600 + 777 * 2
        } else {
            (now_ms() / 1000) as u32
        };

        let str_d_zeel = if self.conf.ticker == "NAV" {
            Some("".into())
        } else {
            None
        };

        let n_time = if self.conf.is_pos {
            Some((now_ms() / 1000) as u32)
        } else {
            None
        };

        TransactionInputSigner {
            version: self.conf.tx_version,
            n_time,
            overwintered: self.conf.overwintered,
            version_group_id: self.conf.version_group_id,
            consensus_branch_id: self.conf.consensus_branch_id,
            expiry_height: 0,
            value_balance: 0,
            inputs: vec![],
            outputs: vec![],
            lock_time,
            join_splits: vec![],
            shielded_spends: vec![],
            shielded_outputs: vec![],
            zcash: self.conf.zcash,
            str_d_zeel,
            hash_algo: self.tx_hash_algo.into(),
        }
    }
}

#[derive(Debug, Display)]
#[allow(clippy::large_enum_variant)]
pub enum BroadcastTxErr {
    /// RPC client error
    Rpc(UtxoRpcError),
    /// Other specific error
    Other(String),
}

impl From<UtxoRpcError> for BroadcastTxErr {
    fn from(err: UtxoRpcError) -> Self { BroadcastTxErr::Rpc(err) }
}

#[async_trait]
#[cfg_attr(test, mockable)]
pub trait UtxoTxBroadcastOps {
    async fn broadcast_tx(&self, tx: &UtxoTx) -> Result<H256Json, MmError<BroadcastTxErr>>;
}

#[async_trait]
#[cfg_attr(test, mockable)]
pub trait UtxoTxGenerationOps {
    async fn get_tx_fee(&self) -> Result<ActualTxFee, JsonRpcError>;

    /// Calculates interest if the coin is KMD
    /// Adds the value to existing output to my_script_pub or creates additional interest output
    /// returns transaction and data as is if the coin is not KMD
    async fn calc_interest_if_required(
        &self,
        mut unsigned: TransactionInputSigner,
        mut data: AdditionalTxData,
        my_script_pub: Bytes,
    ) -> UtxoRpcResult<(TransactionInputSigner, AdditionalTxData)>;
}

#[async_trait]
#[cfg_attr(test, mockable)]
pub trait UtxoCommonOps: UtxoTxGenerationOps + UtxoTxBroadcastOps {
    async fn get_htlc_spend_fee(&self, tx_size: u64) -> UtxoRpcResult<u64>;

    fn addresses_from_script(&self, script: &Script) -> Result<Vec<Address>, String>;

    fn denominate_satoshis(&self, satoshi: i64) -> f64;

    /// Get a public key that matches [`PrivKeyPolicy::KeyPair`].
    ///
    /// # Fail
    ///
    /// The method is expected to fail if [`UtxoCoinFields::priv_key_policy`] is [`PrivKeyPolicy::HardwareWallet`].
    /// It's worth adding a method like `my_public_key_der_path`
    /// that takes a derivation path from which we derive the corresponding public key.
    fn my_public_key(&self) -> Result<&Public, MmError<DerivationMethodNotSupported>>;

    /// Try to parse address from string using specified on asset enable format,
    /// and if it failed inform user that he used a wrong format.
    fn address_from_str(&self, address: &str) -> Result<Address, String>;

    async fn get_current_mtp(&self) -> UtxoRpcResult<u32>;

    /// Check if the output is spendable (is not coinbase or it has enough confirmations).
    fn is_unspent_mature(&self, output: &RpcTransaction) -> bool;

    /// Calculates interest of the specified transaction.
    /// Please note, this method has to be used for KMD transactions only.
    async fn calc_interest_of_tx(&self, tx: &UtxoTx, input_transactions: &mut HistoryUtxoTxMap) -> UtxoRpcResult<u64>;

    /// Try to get a `HistoryUtxoTx` transaction from `utxo_tx_map` or try to request it from Rpc client.
    async fn get_mut_verbose_transaction_from_map_or_rpc<'a, 'b>(
        &'a self,
        tx_hash: H256Json,
        utxo_tx_map: &'b mut HistoryUtxoTxMap,
    ) -> UtxoRpcResult<&'b mut HistoryUtxoTx>;

    async fn p2sh_spending_tx(
        &self,
        prev_transaction: UtxoTx,
        redeem_script: Bytes,
        outputs: Vec<TransactionOutput>,
        script_data: Script,
        sequence: u32,
        lock_time: u32,
    ) -> Result<UtxoTx, String>;

    /// Get transaction outputs available to spend.
    async fn ordered_mature_unspents<'a>(
        &'a self,
        address: &Address,
    ) -> UtxoRpcResult<(Vec<UnspentInfo>, AsyncMutexGuard<'a, RecentlySpentOutPoints>)>;

    /// Try to load verbose transaction from cache or try to request it from Rpc client.
    fn get_verbose_transaction_from_cache_or_rpc(&self, txid: H256Json) -> UtxoRpcFut<VerboseTransactionFrom>;

    /// Cache transaction if the coin supports `TX_CACHE` and tx height is set and not zero.
    async fn cache_transaction_if_possible(&self, tx: &RpcTransaction) -> Result<(), String>;

    /// Returns available unspents in ascending order + RecentlySpentOutPoints MutexGuard for further interaction
    /// (e.g. to add new transaction to it).
    async fn list_unspent_ordered(
        &self,
        address: &Address,
    ) -> UtxoRpcResult<(Vec<UnspentInfo>, AsyncMutexGuard<'_, RecentlySpentOutPoints>)>;

    async fn preimage_trade_fee_required_to_send_outputs(
        &self,
        outputs: Vec<TransactionOutput>,
        fee_policy: FeePolicy,
        gas_fee: Option<u64>,
        stage: &FeeApproxStage,
    ) -> TradePreimageResult<BigDecimal>;

    /// Increase the given `dynamic_fee` according to the fee approximation `stage`.
    /// The method is used to predict a possible increase in dynamic fee.
    fn increase_dynamic_fee_by_stage(&self, dynamic_fee: u64, stage: &FeeApproxStage) -> u64;

    async fn p2sh_tx_locktime(&self, htlc_locktime: u32) -> Result<u32, MmError<UtxoRpcError>>;

    fn addr_format(&self) -> &UtxoAddressFormat;

    fn addr_format_for_standard_scripts(&self) -> UtxoAddressFormat;

    fn address_from_pubkey(&self, pubkey: &Public) -> Address;
}

#[async_trait]
pub trait UtxoStandardOps {
    /// Gets tx details by hash requesting the coin RPC if required.
    /// * `input_transactions` - the cache of the already requested transactions.
    async fn tx_details_by_hash(
        &self,
        hash: &[u8],
        input_transactions: &mut HistoryUtxoTxMap,
    ) -> Result<TransactionDetails, String>;

    async fn request_tx_history(&self, metrics: MetricsArc) -> RequestTxHistoryResult;

    /// Calculate the KMD rewards and re-calculate the transaction fee
    /// if the specified `tx_details` was generated without considering the KMD rewards.
    /// Please note, this method has to be used for KMD transactions only.
    async fn update_kmd_rewards(
        &self,
        tx_details: &mut TransactionDetails,
        input_transactions: &mut HistoryUtxoTxMap,
    ) -> UtxoRpcResult<()>;
}

#[derive(Clone, Debug)]
pub struct UtxoArc(Arc<UtxoCoinFields>);
impl Deref for UtxoArc {
    type Target = UtxoCoinFields;
    fn deref(&self) -> &UtxoCoinFields { &*self.0 }
}

impl From<UtxoCoinFields> for UtxoArc {
    fn from(coin: UtxoCoinFields) -> UtxoArc { UtxoArc(Arc::new(coin)) }
}

impl From<Arc<UtxoCoinFields>> for UtxoArc {
    fn from(arc: Arc<UtxoCoinFields>) -> UtxoArc { UtxoArc(arc) }
}

impl UtxoArc {
    /// Returns weak reference to the inner UtxoCoinFields
    pub fn downgrade(&self) -> UtxoWeak {
        let weak = Arc::downgrade(&self.0);
        UtxoWeak(weak)
    }
}

#[derive(Clone, Debug)]
pub struct UtxoWeak(Weak<UtxoCoinFields>);

impl From<Weak<UtxoCoinFields>> for UtxoWeak {
    fn from(weak: Weak<UtxoCoinFields>) -> Self { UtxoWeak(weak) }
}

impl UtxoWeak {
    pub fn upgrade(&self) -> Option<UtxoArc> { self.0.upgrade().map(UtxoArc::from) }
}

// We can use a shared UTXO lock for all UTXO coins at 1 time.
// It's highly likely that we won't experience any issues with it as we won't need to send "a lot" of transactions concurrently.
lazy_static! {
    pub static ref UTXO_LOCK: AsyncMutex<()> = AsyncMutex::new(());
}

#[derive(Debug, Display)]
pub enum GenerateTxError {
    #[display(
        fmt = "Couldn't generate tx from empty UTXOs set, required no less than {} satoshis",
        required
    )]
    EmptyUtxoSet { required: u64 },
    #[display(fmt = "Couldn't generate tx with empty output set")]
    EmptyOutputs,
    #[display(fmt = "Output value {} less than dust {}", value, dust)]
    OutputValueLessThanDust { value: u64, dust: u64 },
    #[display(
        fmt = "Output {} value {} is too small, required no less than {}",
        output_idx,
        output_value,
        required
    )]
    DeductFeeFromOutputFailed {
        output_idx: usize,
        output_value: u64,
        required: u64,
    },
    #[display(
        fmt = "Sum of input values {} is too small, required no less than {}",
        sum_utxos,
        required
    )]
    NotEnoughUtxos { sum_utxos: u64, required: u64 },
    #[display(fmt = "Transport error: {}", _0)]
    Transport(String),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

impl From<JsonRpcError> for GenerateTxError {
    fn from(rpc_err: JsonRpcError) -> Self { GenerateTxError::Transport(rpc_err.to_string()) }
}

impl From<UtxoRpcError> for GenerateTxError {
    fn from(e: UtxoRpcError) -> Self {
        match e {
            UtxoRpcError::Transport(rpc) | UtxoRpcError::ResponseParseError(rpc) => {
                GenerateTxError::Transport(rpc.to_string())
            },
            UtxoRpcError::InvalidResponse(error) => GenerateTxError::Transport(error),
            UtxoRpcError::Internal(error) => GenerateTxError::Internal(error),
        }
    }
}

impl From<NumConversError> for GenerateTxError {
    fn from(e: NumConversError) -> Self { GenerateTxError::Internal(e.to_string()) }
}

pub enum RequestTxHistoryResult {
    Ok(Vec<(H256Json, u64)>),
    Retry { error: String },
    HistoryTooLarge,
    CriticalError(String),
}

pub enum VerboseTransactionFrom {
    Cache(RpcTransaction),
    Rpc(RpcTransaction),
}

impl VerboseTransactionFrom {
    fn into_inner(self) -> RpcTransaction {
        match self {
            VerboseTransactionFrom::Rpc(tx) | VerboseTransactionFrom::Cache(tx) => tx,
        }
    }
}

pub fn compressed_key_pair_from_bytes(raw: &[u8], prefix: u8, checksum_type: ChecksumType) -> Result<KeyPair, String> {
    if raw.len() != 32 {
        return ERR!("Invalid raw priv key len {}", raw.len());
    }

    let private = Private {
        prefix,
        compressed: true,
        secret: Secret::from(raw),
        checksum_type,
    };
    Ok(try_s!(KeyPair::from_private(private)))
}

pub fn compressed_pub_key_from_priv_raw(raw_priv: &[u8], sum_type: ChecksumType) -> Result<H264, String> {
    let key_pair: KeyPair = try_s!(compressed_key_pair_from_bytes(raw_priv, 0, sum_type));
    Ok(H264::from(&**key_pair.public()))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct UtxoFeeDetails {
    pub coin: Option<String>,
    pub amount: BigDecimal,
}

#[cfg(not(target_arch = "wasm32"))]
// https://github.com/KomodoPlatform/komodo/blob/master/zcutil/fetch-params.sh#L5
// https://github.com/KomodoPlatform/komodo/blob/master/zcutil/fetch-params.bat#L4
pub fn zcash_params_path() -> PathBuf {
    if cfg!(windows) {
        // >= Vista: c:\Users\$username\AppData\Roaming
        get_special_folder_path().join("ZcashParams")
    } else if cfg!(target_os = "macos") {
        home_dir()
            .unwrap()
            .join("Library")
            .join("Application Support")
            .join("ZcashParams")
    } else {
        home_dir().unwrap().join(".zcash-params")
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn coin_daemon_data_dir(name: &str, is_asset_chain: bool) -> PathBuf {
    // komodo/util.cpp/GetDefaultDataDir
    let mut data_dir = match dirs::home_dir() {
        Some(hd) => hd,
        None => Path::new("/").to_path_buf(),
    };

    if cfg!(windows) {
        // >= Vista: c:\Users\$username\AppData\Roaming
        data_dir = get_special_folder_path();
        if is_asset_chain {
            data_dir.push("Komodo");
        } else {
            data_dir.push(first_char_to_upper(name));
        }
    } else if cfg!(target_os = "macos") {
        data_dir.push("Library");
        data_dir.push("Application Support");
        if is_asset_chain {
            data_dir.push("Komodo");
        } else {
            data_dir.push(first_char_to_upper(name));
        }
    } else if is_asset_chain {
        data_dir.push(".komodo");
    } else {
        data_dir.push(format!(".{}", name));
    }

    if is_asset_chain {
        data_dir.push(name)
    };
    data_dir
}

/// Attempts to parse native daemon conf file and return rpcport, rpcuser and rpcpassword
#[cfg(not(target_arch = "wasm32"))]
fn read_native_mode_conf(
    filename: &dyn AsRef<Path>,
    network: &BlockchainNetwork,
) -> Result<(Option<u16>, String, String), String> {
    use ini::Ini;

    fn read_property<'a>(conf: &'a ini::Ini, network: &BlockchainNetwork, property: &str) -> Option<&'a String> {
        let subsection = match network {
            BlockchainNetwork::Mainnet => None,
            BlockchainNetwork::Testnet => conf.section(Some("test")),
            BlockchainNetwork::Regtest => conf.section(Some("regtest")),
        };
        subsection
            .and_then(|props| props.get(property))
            .or_else(|| conf.general_section().get(property))
    }

    let conf: Ini = match Ini::load_from_file(&filename) {
        Ok(ini) => ini,
        Err(err) => {
            return ERR!(
                "Error parsing the native wallet configuration '{}': {}",
                filename.as_ref().display(),
                err
            )
        },
    };
    let rpc_port = match read_property(&conf, network, "rpcport") {
        Some(port) => port.parse::<u16>().ok(),
        None => None,
    };
    let rpc_user = try_s!(read_property(&conf, network, "rpcuser").ok_or(ERRL!(
        "Conf file {} doesn't have the rpcuser key",
        filename.as_ref().display()
    )));
    let rpc_password = try_s!(read_property(&conf, network, "rpcpassword").ok_or(ERRL!(
        "Conf file {} doesn't have the rpcpassword key",
        filename.as_ref().display()
    )));
    Ok((rpc_port, rpc_user.clone(), rpc_password.clone()))
}

/// Electrum protocol version verifier.
/// The structure is used to handle the `on_connected` event and notify `electrum_version_loop`.
struct ElectrumProtoVerifier {
    on_connect_tx: mpsc::UnboundedSender<String>,
}

impl ElectrumProtoVerifier {
    fn into_shared(self) -> RpcTransportEventHandlerShared { Arc::new(self) }
}

impl RpcTransportEventHandler for ElectrumProtoVerifier {
    fn debug_info(&self) -> String { "ElectrumProtoVerifier".into() }

    fn on_outgoing_request(&self, _data: &[u8]) {}

    fn on_incoming_response(&self, _data: &[u8]) {}

    fn on_connected(&self, address: String) -> Result<(), String> {
        try_s!(self.on_connect_tx.unbounded_send(address));
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UtxoActivationParams {
    pub mode: UtxoRpcMode,
    pub utxo_merge_params: Option<UtxoMergeParams>,
    #[serde(default)]
    pub tx_history: bool,
    pub required_confirmations: Option<u64>,
    pub requires_notarization: Option<bool>,
    pub address_format: Option<UtxoAddressFormat>,
}

#[derive(Debug, Display)]
pub enum UtxoFromLegacyReqErr {
    UnexpectedMethod,
    InvalidElectrumServers(json::Error),
    InvalidMergeParams(json::Error),
    InvalidRequiredConfs(json::Error),
    InvalidRequiresNota(json::Error),
    InvalidAddressFormat(json::Error),
}

impl UtxoActivationParams {
    pub fn from_legacy_req(req: &Json) -> Result<Self, MmError<UtxoFromLegacyReqErr>> {
        let mode = match req["method"].as_str() {
            Some("enable") => UtxoRpcMode::Native,
            Some("electrum") => {
                let servers =
                    json::from_value(req["servers"].clone()).map_to_mm(UtxoFromLegacyReqErr::InvalidElectrumServers)?;
                UtxoRpcMode::Electrum { servers }
            },
            _ => return MmError::err(UtxoFromLegacyReqErr::UnexpectedMethod),
        };
        let utxo_merge_params =
            json::from_value(req["utxo_merge_params"].clone()).map_to_mm(UtxoFromLegacyReqErr::InvalidMergeParams)?;

        let tx_history = req["tx_history"].as_bool().unwrap_or_default();
        let required_confirmations = json::from_value(req["required_confirmations"].clone())
            .map_to_mm(UtxoFromLegacyReqErr::InvalidRequiredConfs)?;
        let requires_notarization = json::from_value(req["requires_notarization"].clone())
            .map_to_mm(UtxoFromLegacyReqErr::InvalidRequiresNota)?;
        let address_format =
            json::from_value(req["address_format"].clone()).map_to_mm(UtxoFromLegacyReqErr::InvalidAddressFormat)?;

        Ok(UtxoActivationParams {
            mode,
            utxo_merge_params,
            tx_history,
            required_confirmations,
            requires_notarization,
            address_format,
        })
    }
}

pub struct UtxoConfBuilder<'a> {
    conf: &'a Json,
    ticker: &'a str,
    params: UtxoActivationParams,
}

impl<'a> UtxoConfBuilder<'a> {
    pub fn new(conf: &'a Json, params: UtxoActivationParams, ticker: &'a str) -> Self {
        UtxoConfBuilder { conf, ticker, params }
    }

    pub fn build(&self) -> UtxoConfResult<UtxoCoinConf> {
        let checksum_type = self.checksum_type();
        let pub_addr_prefix = self.pub_addr_prefix();
        let p2sh_addr_prefix = self.p2sh_address_prefix();
        let pub_t_addr_prefix = self.pub_t_address_prefix();
        let p2sh_t_addr_prefix = self.p2sh_t_address_prefix();

        let wif_prefix = self.wif_prefix();

        let bech32_hrp = self.bech32_hrp();

        let default_address_format = self.default_address_format();

        let asset_chain = self.asset_chain();
        let tx_version = self.tx_version();
        let overwintered = self.overwintered();

        let tx_fee_volatility_percent = self.tx_fee_volatility_percent();
        let version_group_id = self.version_group_id(tx_version, overwintered)?;
        let consensus_branch_id = self.consensus_branch_id(tx_version)?;
        let signature_version = self.signature_version();
        let fork_id = self.fork_id();

        // should be sufficient to detect zcash by overwintered flag
        let zcash = overwintered;

        let required_confirmations = self.required_confirmations();
        let requires_notarization = self.requires_notarization();

        let mature_confirmations = self.mature_confirmations();

        let is_pos = self.is_pos();
        let segwit = self.segwit();
        let force_min_relay_fee = self.conf["force_min_relay_fee"].as_bool().unwrap_or(false);
        let mtp_block_count = self.mtp_block_count();
        let estimate_fee_mode = self.estimate_fee_mode();
        let estimate_fee_blocks = self.estimate_fee_blocks();
        let lightning = self.lightning();
        let network = self.network();
        let trezor_coin = self.trezor_coin();
        let derivation_path = self.derivation_path()?;

        Ok(UtxoCoinConf {
            ticker: self.ticker.to_owned(),
            is_pos,
            requires_notarization,
            overwintered,
            pub_addr_prefix,
            p2sh_addr_prefix,
            pub_t_addr_prefix,
            p2sh_t_addr_prefix,
            bech32_hrp,
            segwit,
            wif_prefix,
            tx_version,
            default_address_format,
            asset_chain,
            tx_fee_volatility_percent,
            version_group_id,
            consensus_branch_id,
            zcash,
            checksum_type,
            signature_version,
            fork_id,
            required_confirmations: required_confirmations.into(),
            force_min_relay_fee,
            mtp_block_count,
            estimate_fee_mode,
            mature_confirmations,
            estimate_fee_blocks,
            lightning,
            network,
            trezor_coin,
            derivation_path,
        })
    }

    fn checksum_type(&self) -> ChecksumType {
        match self.ticker {
            "GRS" => ChecksumType::DGROESTL512,
            "SMART" => ChecksumType::KECCAK256,
            _ => ChecksumType::DSHA256,
        }
    }

    fn pub_addr_prefix(&self) -> u8 {
        let pubtype = self.conf["pubtype"]
            .as_u64()
            .unwrap_or(if self.ticker == "BTC" { 0 } else { 60 });
        pubtype as u8
    }

    fn p2sh_address_prefix(&self) -> u8 {
        self.conf["p2shtype"]
            .as_u64()
            .unwrap_or(if self.ticker == "BTC" { 5 } else { 85 }) as u8
    }

    fn pub_t_address_prefix(&self) -> u8 { self.conf["taddr"].as_u64().unwrap_or(0) as u8 }

    fn p2sh_t_address_prefix(&self) -> u8 { self.conf["taddr"].as_u64().unwrap_or(0) as u8 }

    fn wif_prefix(&self) -> u8 {
        let wiftype = self.conf["wiftype"]
            .as_u64()
            .unwrap_or(if self.ticker == "BTC" { 128 } else { 188 });
        wiftype as u8
    }

    fn bech32_hrp(&self) -> Option<String> { json::from_value(self.conf["bech32_hrp"].clone()).unwrap_or(None) }

    fn default_address_format(&self) -> UtxoAddressFormat {
        let mut address_format: UtxoAddressFormat =
            json::from_value(self.conf["address_format"].clone()).unwrap_or(UtxoAddressFormat::Standard);

        if let UtxoAddressFormat::CashAddress {
            network: _,
            ref mut pub_addr_prefix,
            ref mut p2sh_addr_prefix,
        } = address_format
        {
            *pub_addr_prefix = self.pub_addr_prefix();
            *p2sh_addr_prefix = self.p2sh_address_prefix();
        }

        address_format
    }

    fn asset_chain(&self) -> bool { self.conf["asset"].as_str().is_some() }

    fn tx_version(&self) -> i32 { self.conf["txversion"].as_i64().unwrap_or(1) as i32 }

    fn overwintered(&self) -> bool { self.conf["overwintered"].as_u64().unwrap_or(0) == 1 }

    fn tx_fee_volatility_percent(&self) -> f64 {
        match self.conf["txfee_volatility_percent"].as_f64() {
            Some(volatility) => volatility,
            None => DEFAULT_DYNAMIC_FEE_VOLATILITY_PERCENT,
        }
    }

    fn version_group_id(&self, tx_version: i32, overwintered: bool) -> UtxoConfResult<u32> {
        let version_group_id = match self.conf["version_group_id"].as_str() {
            Some(s) => parse_hex_encoded_u32(s).mm_err(UtxoConfError::InvalidVersionGroupId)?,
            None => {
                if tx_version == 3 && overwintered {
                    0x03c4_8270
                } else if tx_version == 4 && overwintered {
                    0x892f_2085
                } else {
                    0
                }
            },
        };
        Ok(version_group_id)
    }

    fn consensus_branch_id(&self, tx_version: i32) -> UtxoConfResult<u32> {
        let consensus_branch_id = match self.conf["consensus_branch_id"].as_str() {
            Some(s) => parse_hex_encoded_u32(s).mm_err(UtxoConfError::InvalidConsensusBranchId)?,
            None => match tx_version {
                3 => 0x5ba8_1b19,
                4 => 0x76b8_09bb,
                _ => 0,
            },
        };
        Ok(consensus_branch_id)
    }

    fn signature_version(&self) -> SignatureVersion {
        let default_signature_version = if self.ticker == "BCH" || self.fork_id() != 0 {
            SignatureVersion::ForkId
        } else {
            SignatureVersion::Base
        };
        json::from_value(self.conf["signature_version"].clone()).unwrap_or(default_signature_version)
    }

    fn fork_id(&self) -> u32 {
        let default_fork_id = match self.ticker {
            "BCH" => "0x40",
            _ => "0x0",
        };
        let hex_string = self.conf["fork_id"].as_str().unwrap_or(default_fork_id);
        let fork_id = u32::from_str_radix(hex_string.trim_start_matches("0x"), 16).unwrap();
        fork_id
    }

    fn required_confirmations(&self) -> u64 {
        // param from request should override the config
        self.params
            .required_confirmations
            .unwrap_or_else(|| self.conf["required_confirmations"].as_u64().unwrap_or(1))
    }

    fn requires_notarization(&self) -> AtomicBool {
        self.params
            .requires_notarization
            .unwrap_or_else(|| self.conf["requires_notarization"].as_bool().unwrap_or(false))
            .into()
    }

    fn mature_confirmations(&self) -> u32 {
        self.conf["mature_confirmations"]
            .as_u64()
            .map(|x| x as u32)
            .unwrap_or(MATURE_CONFIRMATIONS_DEFAULT)
    }

    fn is_pos(&self) -> bool { self.conf["isPoS"].as_u64() == Some(1) }

    fn segwit(&self) -> bool { self.conf["segwit"].as_bool().unwrap_or(false) }

    fn mtp_block_count(&self) -> NonZeroU64 {
        json::from_value(self.conf["mtp_block_count"].clone()).unwrap_or(KMD_MTP_BLOCK_COUNT)
    }

    fn estimate_fee_mode(&self) -> Option<EstimateFeeMode> {
        json::from_value(self.conf["estimate_fee_mode"].clone()).unwrap_or(None)
    }

    fn estimate_fee_blocks(&self) -> u32 { json::from_value(self.conf["estimate_fee_blocks"].clone()).unwrap_or(1) }

    fn lightning(&self) -> bool {
        if self.segwit() && self.bech32_hrp().is_some() {
            self.conf["lightning"].as_bool().unwrap_or(false)
        } else {
            false
        }
    }

    fn network(&self) -> Option<String> { json::from_value(self.conf["network"].clone()).unwrap_or(None) }

    fn trezor_coin(&self) -> Option<TrezorCoin> {
        json::from_value(self.conf["trezor_coin"].clone()).unwrap_or_default()
    }

    fn derivation_path(&self) -> UtxoConfResult<Option<DerivationPath>> {
        let derivation_path = match self.conf["derivation_path"].as_str() {
            Some(derivation_path) => derivation_path,
            None => return Ok(None),
        };
        let derivation_path = DerivationPath::from_str(derivation_path)?;
        if derivation_path.len() != 2 {
            return MmError::err(UtxoConfError::InvalidDerivationPathLen {
                found_children: derivation_path.len(),
            });
        }
        match derivation_path.iter().next() {
            Some(ChildNumber(BIP44_PURPOSE)) => (),
            Some(ChildNumber(purpose)) => return MmError::err(UtxoConfError::InvalidDerivationPathPurpose { purpose }),
            None => return MmError::err(UtxoConfError::InvalidDerivationPathLen { found_children: 0 }),
        }
        Ok(Some(derivation_path))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "rpc", content = "rpc_data")]
pub enum UtxoRpcMode {
    Native,
    Electrum { servers: Vec<ElectrumRpcRequest> },
}

#[derive(Debug)]
pub struct ElectrumBuilderArgs {
    pub spawn_ping: bool,
    pub negotiate_version: bool,
    pub collect_metrics: bool,
}

impl Default for ElectrumBuilderArgs {
    fn default() -> Self {
        ElectrumBuilderArgs {
            spawn_ping: true,
            negotiate_version: true,
            collect_metrics: true,
        }
    }
}

#[derive(Clone)]
pub enum PrivKeyBuildPolicy<'a> {
    PrivKey(&'a [u8]),
    HardwareWallet,
}

#[derive(Debug)]
pub struct HDWalletInfo {
    pub address_format: UtxoAddressFormat,
    /// [`accounts.len()`] equals to a number of the user's accounts.
    pub accounts: Mutex<Vec<DerivationAccountInfo>>,
}

/// Consider adding addresses/pubkeys and `ExtendedPublicKey` of the account.
#[derive(Debug)]
pub struct DerivationAccountInfo {
    /// The number of addresses used by the corresponding account.
    pub addresses_number: u32,
}

impl HDWalletInfo {
    pub fn new(address_format: UtxoAddressFormat) -> HDWalletInfo {
        HDWalletInfo {
            address_format,
            accounts: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
pub trait UtxoCoinBuilder {
    type ResultCoin;

    async fn build(self) -> UtxoCoinBuildResult<Self::ResultCoin>;

    fn ctx(&self) -> &MmArc;

    fn conf(&self) -> &Json;

    fn activation_params(&self) -> UtxoActivationParams;

    fn ticker(&self) -> &str;

    fn priv_key(&self) -> PrivKeyBuildPolicy<'_>;

    async fn build_utxo_fields(&self) -> UtxoCoinBuildResult<UtxoCoinFields> {
        match self.priv_key() {
            PrivKeyBuildPolicy::PrivKey(priv_key) => self.build_utxo_fields_with_priv_key(priv_key).await,
            PrivKeyBuildPolicy::HardwareWallet => self.build_utxo_fields_with_hw().await,
        }
    }

    async fn build_utxo_fields_with_priv_key(&self, priv_key: &[u8]) -> UtxoCoinBuildResult<UtxoCoinFields> {
        let conf = UtxoConfBuilder::new(self.conf(), self.activation_params(), self.ticker()).build()?;

        let private = Private {
            prefix: conf.wif_prefix,
            secret: H256::from(priv_key),
            compressed: true,
            checksum_type: conf.checksum_type,
        };
        let key_pair = KeyPair::from_private(private).map_to_mm(|e| UtxoCoinBuildError::Internal(e.to_string()))?;
        let addr_format = self.address_format()?;
        let my_address = Address {
            prefix: conf.pub_addr_prefix,
            t_addr_prefix: conf.pub_t_addr_prefix,
            hash: AddressHashEnum::AddressHash(key_pair.public().address_hash()),
            checksum_type: conf.checksum_type,
            hrp: conf.bech32_hrp.clone(),
            addr_format,
        };

        let my_script_pubkey = output_script(&my_address, ScriptType::P2PKH).to_bytes();
        let derivation_method = DerivationMethod::Iguana(my_address);
        let priv_key_policy = PrivKeyPolicy::KeyPair(key_pair);

        let rpc_client = self.rpc_client().await?;
        let tx_fee = self.tx_fee(&rpc_client).await?;
        let decimals = self.decimals(&rpc_client).await?;
        let dust_amount = self.dust_amount();

        let initial_history_state = self.initial_history_state();
        let tx_cache_directory = Some(self.ctx().dbdir().join("TX_CACHE"));
        let tx_hash_algo = self.tx_hash_algo();

        let coin = UtxoCoinFields {
            conf,
            decimals,
            dust_amount,
            rpc_client,
            priv_key_policy,
            derivation_method,
            history_sync_state: Mutex::new(initial_history_state),
            tx_cache_directory,
            recently_spent_outpoints: AsyncMutex::new(RecentlySpentOutPoints::new(my_script_pubkey)),
            tx_fee,
            tx_hash_algo,
        };
        Ok(coin)
    }

    async fn build_utxo_fields_with_hw(&self) -> UtxoCoinBuildResult<UtxoCoinFields> {
        let conf = UtxoConfBuilder::new(self.conf(), self.activation_params(), self.ticker()).build()?;

        // For now, use a default script pubkey.
        // TODO change the type of `recently_spent_outpoints` to `AsyncMutex<HashMap<Bytes, RecentlySpentOutPoints>>`
        let my_script_pubkey = Bytes::new();
        let addr_format = self.address_format()?;

        let rpc_client = self.rpc_client().await?;
        let tx_fee = self.tx_fee(&rpc_client).await?;
        let decimals = self.decimals(&rpc_client).await?;
        let dust_amount = self.dust_amount();

        let initial_history_state = self.initial_history_state();
        let tx_cache_directory = Some(self.ctx().dbdir().join("TX_CACHE"));
        let tx_hash_algo = self.tx_hash_algo();

        let coin = UtxoCoinFields {
            conf,
            decimals,
            dust_amount,
            rpc_client,
            priv_key_policy: PrivKeyPolicy::HardwareWallet,
            derivation_method: DerivationMethod::HDWallet(HDWalletInfo::new(addr_format)),
            history_sync_state: Mutex::new(initial_history_state),
            tx_cache_directory,
            recently_spent_outpoints: AsyncMutex::new(RecentlySpentOutPoints::new(my_script_pubkey)),
            tx_fee,
            tx_hash_algo,
        };
        Ok(coin)
    }

    fn address_format(&self) -> UtxoCoinBuildResult<UtxoAddressFormat> {
        let format_from_req: Option<UtxoAddressFormat> = self.activation_params().address_format;
        let format_from_conf = json::from_value::<Option<UtxoAddressFormat>>(self.conf()["address_format"].clone())
            .map_to_mm(|e| UtxoConfError::InvalidAddressFormat(e.to_string()))?
            .unwrap_or(UtxoAddressFormat::Standard);

        let mut address_format = match format_from_req {
            Some(from_req) => {
                if from_req.is_segwit() != format_from_conf.is_segwit() {
                    let error = format!(
                        "Both conf {:?} and request {:?} must be either Segwit or Standard/CashAddress",
                        format_from_conf, from_req
                    );
                    return MmError::err(UtxoCoinBuildError::from(UtxoConfError::InvalidAddressFormat(error)));
                } else {
                    from_req
                }
            },
            None => format_from_conf,
        };

        if let UtxoAddressFormat::CashAddress {
            network: _,
            ref mut pub_addr_prefix,
            ref mut p2sh_addr_prefix,
        } = address_format
        {
            *pub_addr_prefix = self.pub_addr_prefix();
            *p2sh_addr_prefix = self.p2sh_address_prefix();
        }

        let is_segwit_in_conf = self.conf()["segwit"].as_bool().unwrap_or(false);
        if address_format.is_segwit() && (!is_segwit_in_conf || self.conf()["bech32_hrp"].is_null()) {
            let error =
                "Cannot use Segwit address format for coin without segwit support or bech32_hrp in config".to_owned();
            return MmError::err(UtxoCoinBuildError::from(UtxoConfError::InvalidAddressFormat(error)));
        }
        Ok(address_format)
    }

    fn pub_addr_prefix(&self) -> u8 {
        let pubtype = self.conf()["pubtype"]
            .as_u64()
            .unwrap_or(if self.ticker() == "BTC" { 0 } else { 60 });
        pubtype as u8
    }

    fn p2sh_address_prefix(&self) -> u8 {
        self.conf()["p2shtype"]
            .as_u64()
            .unwrap_or(if self.ticker() == "BTC" { 5 } else { 85 }) as u8
    }

    fn dust_amount(&self) -> u64 { json::from_value(self.conf()["dust"].clone()).unwrap_or(UTXO_DUST_AMOUNT) }

    fn network(&self) -> UtxoConfResult<BlockchainNetwork> {
        let conf = self.conf();
        if !conf["network"].is_null() {
            return json::from_value(conf["network"].clone())
                .map_to_mm(|e| UtxoConfError::InvalidBlockchainNetwork(e.to_string()));
        }
        Ok(BlockchainNetwork::Mainnet)
    }

    async fn decimals(&self, _rpc_client: &UtxoRpcClientEnum) -> UtxoCoinBuildResult<u8> {
        Ok(self.conf()["decimals"].as_u64().unwrap_or(8) as u8)
    }

    async fn tx_fee(&self, rpc_client: &UtxoRpcClientEnum) -> Result<TxFee, MmError<UtxoCoinBuildError>> {
        let tx_fee = match self.conf()["txfee"].as_u64() {
            None => TxFee::FixedPerKb(1000),
            Some(0) => {
                let fee_method = match &rpc_client {
                    UtxoRpcClientEnum::Electrum(_) => EstimateFeeMethod::Standard,
                    UtxoRpcClientEnum::Native(client) => client
                        .detect_fee_method()
                        .compat()
                        .await
                        .map_to_mm(UtxoCoinBuildError::ErrorDetectingFeeMethod)?,
                };
                TxFee::Dynamic(fee_method)
            },
            Some(fee) => TxFee::FixedPerKb(fee),
        };
        Ok(tx_fee)
    }

    fn initial_history_state(&self) -> HistorySyncState {
        if self.activation_params().tx_history {
            HistorySyncState::NotStarted
        } else {
            HistorySyncState::NotEnabled
        }
    }

    async fn rpc_client(&self) -> UtxoCoinBuildResult<UtxoRpcClientEnum> {
        match self.activation_params().mode {
            UtxoRpcMode::Native => {
                #[cfg(target_arch = "wasm32")]
                {
                    MmError::err(UtxoCoinBuildError::NativeRpcNotSupportedInWasm)
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let native = self.native_client()?;
                    Ok(UtxoRpcClientEnum::Native(native))
                }
            },
            UtxoRpcMode::Electrum { servers } => {
                let electrum = self.electrum_client(ElectrumBuilderArgs::default(), servers).await?;
                Ok(UtxoRpcClientEnum::Electrum(electrum))
            },
        }
    }

    async fn electrum_client(
        &self,
        args: ElectrumBuilderArgs,
        mut servers: Vec<ElectrumRpcRequest>,
    ) -> UtxoCoinBuildResult<ElectrumClient> {
        let (on_connect_tx, on_connect_rx) = mpsc::unbounded();
        let ticker = self.ticker().to_owned();
        let ctx = self.ctx();
        let mut event_handlers = vec![];
        if args.collect_metrics {
            event_handlers.push(
                CoinTransportMetrics::new(ctx.metrics.weak(), ticker.clone(), RpcClientType::Electrum).into_shared(),
            );
        }

        if args.negotiate_version {
            event_handlers.push(ElectrumProtoVerifier { on_connect_tx }.into_shared());
        }

        let mut rng = small_rng();
        servers.as_mut_slice().shuffle(&mut rng);
        let client = ElectrumClientImpl::new(ticker, event_handlers);
        for server in servers.iter() {
            match client.add_server(server).await {
                Ok(_) => (),
                Err(e) => log!("Error " (e) " connecting to " [server] ". Address won't be used"),
            };
        }

        let mut attempts = 0i32;
        while !client.is_connected().await {
            if attempts >= 10 {
                return MmError::err(UtxoCoinBuildError::FailedToConnectToElectrums {
                    electrum_servers: servers.clone(),
                    seconds: 5,
                });
            }

            Timer::sleep(0.5).await;
            attempts += 1;
        }

        let client = Arc::new(client);

        if args.negotiate_version {
            let weak_client = Arc::downgrade(&client);
            let client_name = format!("{} GUI/MM2 {}", ctx.gui().unwrap_or("UNKNOWN"), ctx.mm_version());
            spawn_electrum_version_loop(weak_client, on_connect_rx, client_name);

            wait_for_protocol_version_checked(&client)
                .await
                .map_to_mm(UtxoCoinBuildError::ElectrumProtocolVersionCheckError)?;
        }

        if args.spawn_ping {
            let weak_client = Arc::downgrade(&client);
            spawn_electrum_ping_loop(weak_client, servers);
        }

        Ok(ElectrumClient(client))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn native_client(&self) -> UtxoCoinBuildResult<NativeClient> {
        use base64::{encode_config as base64_encode, URL_SAFE};

        let native_conf_path = self.confpath()?;
        let network = self.network()?;
        let (rpc_port, rpc_user, rpc_password) = read_native_mode_conf(&native_conf_path, &network)
            .map_to_mm(UtxoCoinBuildError::ErrorReadingNativeModeConf)?;
        let auth_str = fomat!((rpc_user)":"(rpc_password));
        let rpc_port = match rpc_port {
            Some(p) => p,
            None => self.conf()["rpcport"]
                .as_u64()
                .or_mm_err(|| UtxoCoinBuildError::RpcPortIsNotSet)? as u16,
        };

        let ctx = self.ctx();
        let coin_ticker = self.ticker().to_owned();
        let event_handlers =
            vec![
                CoinTransportMetrics::new(ctx.metrics.weak(), coin_ticker.clone(), RpcClientType::Native).into_shared(),
            ];
        let client = Arc::new(NativeClientImpl {
            coin_ticker,
            uri: fomat!("http://127.0.0.1:"(rpc_port)),
            auth: format!("Basic {}", base64_encode(&auth_str, URL_SAFE)),
            event_handlers,
            request_id: 0u64.into(),
            list_unspent_concurrent_map: ConcurrentRequestMap::new(),
        });

        Ok(NativeClient(client))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn confpath(&self) -> UtxoCoinBuildResult<PathBuf> {
        let conf = self.conf();
        // Documented at https://github.com/jl777/coins#bitcoin-protocol-specific-json
        // "USERHOME/" prefix should be replaced with the user's home folder.
        let declared_confpath = match self.conf()["confpath"].as_str() {
            Some(path) if !path.is_empty() => path.trim(),
            _ => {
                let (name, is_asset_chain) = {
                    match conf["asset"].as_str() {
                        Some(a) => (a, true),
                        None => {
                            let name = conf["name"]
                                .as_str()
                                .or_mm_err(|| UtxoConfError::CurrencyNameIsNotSet)?;
                            (name, false)
                        },
                    }
                };
                let data_dir = coin_daemon_data_dir(name, is_asset_chain);
                let confname = format!("{}.conf", name);

                return Ok(data_dir.join(&confname[..]));
            },
        };

        let (confpath, rel_to_home) = match declared_confpath.strip_prefix("~/") {
            Some(stripped) => (stripped, true),
            None => match declared_confpath.strip_prefix("USERHOME/") {
                Some(stripped) => (stripped, true),
                None => (declared_confpath, false),
            },
        };

        if rel_to_home {
            let home = home_dir().or_mm_err(|| UtxoCoinBuildError::CantDetectUserHome)?;
            Ok(home.join(confpath))
        } else {
            Ok(confpath.into())
        }
    }

    fn tx_hash_algo(&self) -> TxHashAlgo {
        if self.ticker() == "GRS" {
            TxHashAlgo::SHA256
        } else {
            TxHashAlgo::DSHA256
        }
    }
}

/// Ping the electrum servers every 30 seconds to prevent them from disconnecting us.
/// According to docs server can do it if there are no messages in ~10 minutes.
/// https://electrumx.readthedocs.io/en/latest/protocol-methods.html?highlight=keep#server-ping
/// Weak reference will allow to stop the thread if client is dropped.
fn spawn_electrum_ping_loop(weak_client: Weak<ElectrumClientImpl>, servers: Vec<ElectrumRpcRequest>) {
    spawn(async move {
        loop {
            if let Some(client) = weak_client.upgrade() {
                if let Err(e) = ElectrumClient(client).server_ping().compat().await {
                    log!("Electrum servers " [servers] " ping error " [e]);
                }
            } else {
                log!("Electrum servers " [servers] " ping loop stopped");
                break;
            }
            Timer::sleep(30.).await
        }
    });
}

async fn check_electrum_server_version(
    weak_client: Weak<ElectrumClientImpl>,
    client_name: String,
    electrum_addr: String,
) {
    // client.remove_server() is called too often
    async fn remove_server(client: ElectrumClient, electrum_addr: &str) {
        if let Err(e) = client.remove_server(electrum_addr).await {
            log!("Error on remove server "[e]);
        }
    }

    if let Some(c) = weak_client.upgrade() {
        let client = ElectrumClient(c);
        let available_protocols = client.protocol_version();
        let version = match client
            .server_version(&electrum_addr, &client_name, available_protocols)
            .compat()
            .await
        {
            Ok(version) => version,
            Err(e) => {
                log!("Electrum " (electrum_addr) " server.version error \"" [e] "\".");
                if !e.error.is_transport() {
                    remove_server(client, &electrum_addr).await;
                };
                return;
            },
        };

        // check if the version is allowed
        let actual_version = match version.protocol_version.parse::<f32>() {
            Ok(v) => v,
            Err(e) => {
                log!("Error on parse protocol_version "[e]);
                remove_server(client, &electrum_addr).await;
                return;
            },
        };

        if !available_protocols.contains(&actual_version) {
            log!("Received unsupported protocol version " [actual_version] " from " [electrum_addr] ". Remove the connection");
            remove_server(client, &electrum_addr).await;
            return;
        }

        match client.set_protocol_version(&electrum_addr, actual_version).await {
            Ok(()) => {
                log!("Use protocol version " [actual_version] " for Electrum " [electrum_addr]);
            },
            Err(e) => {
                log!("Error on set protocol_version "[e]);
            },
        };
    }
}

/// Follow the `on_connect_rx` stream and verify the protocol version of each connected electrum server.
/// https://electrumx.readthedocs.io/en/latest/protocol-methods.html?highlight=keep#server-version
/// Weak reference will allow to stop the thread if client is dropped.
fn spawn_electrum_version_loop(
    weak_client: Weak<ElectrumClientImpl>,
    mut on_connect_rx: mpsc::UnboundedReceiver<String>,
    client_name: String,
) {
    spawn(async move {
        while let Some(electrum_addr) = on_connect_rx.next().await {
            spawn(check_electrum_server_version(
                weak_client.clone(),
                client_name.clone(),
                electrum_addr,
            ));
        }

        log!("Electrum server.version loop stopped");
    });
}

/// Wait until the protocol version of at least one client's Electrum is checked.
async fn wait_for_protocol_version_checked(client: &ElectrumClientImpl) -> Result<(), String> {
    let mut attempts = 0;
    loop {
        if attempts >= 10 {
            return ERR!("Failed protocol version verifying of at least 1 of Electrums in 5 seconds.");
        }

        if client.count_connections().await == 0 {
            // All of the connections were removed because of server.version checking
            return ERR!(
                "There are no Electrums with the required protocol version {:?}",
                client.protocol_version()
            );
        }

        if client.is_protocol_version_checked().await {
            break;
        }

        Timer::sleep(0.5).await;
        attempts += 1;
    }

    Ok(())
}

/// Function calculating KMD interest
/// https://komodoplatform.atlassian.net/wiki/spaces/KPSD/pages/71729215/What+is+the+5+Komodo+Stake+Reward
/// https://github.com/KomodoPlatform/komodo/blob/master/src/komodo_interest.h
fn kmd_interest(
    height: Option<u64>,
    value: u64,
    lock_time: u64,
    current_time: u64,
) -> Result<u64, KmdRewardsNotAccruedReason> {
    const KOMODO_ENDOFERA: u64 = 7_777_777;
    const LOCKTIME_THRESHOLD: u64 = 500_000_000;

    // value must be at least 10 KMD
    if value < 1_000_000_000 {
        return Err(KmdRewardsNotAccruedReason::UtxoAmountLessThanTen);
    }
    // locktime must be set
    if lock_time == 0 {
        return Err(KmdRewardsNotAccruedReason::LocktimeNotSet);
    }
    // interest doesn't accrue for lock_time < 500_000_000
    if lock_time < LOCKTIME_THRESHOLD {
        return Err(KmdRewardsNotAccruedReason::LocktimeLessThanThreshold);
    }
    let height = match height {
        Some(h) => h,
        None => return Err(KmdRewardsNotAccruedReason::TransactionInMempool), // consider that the transaction is not mined yet
    };
    // interest will stop accrue after block 7_777_777
    if height >= KOMODO_ENDOFERA {
        return Err(KmdRewardsNotAccruedReason::UtxoHeightGreaterThanEndOfEra);
    };
    // current time must be greater than tx lock_time
    if current_time < lock_time {
        return Err(KmdRewardsNotAccruedReason::OneHourNotPassedYet);
    }

    let mut minutes = (current_time - lock_time) / 60;

    // at least 1 hour should pass
    if minutes < 60 {
        return Err(KmdRewardsNotAccruedReason::OneHourNotPassedYet);
    }

    // interest stop accruing after 1 year before block 1000000
    if minutes > 365 * 24 * 60 {
        minutes = 365 * 24 * 60
    };
    // interest stop accruing after 1 month past 1000000 block
    if height >= 1_000_000 && minutes > 31 * 24 * 60 {
        minutes = 31 * 24 * 60;
    }
    // next 2 lines ported as is from Komodo codebase
    minutes -= 59;
    let accrued = (value / 10_512_000) * minutes;

    Ok(accrued)
}

fn kmd_interest_accrue_stop_at(height: u64, lock_time: u64) -> u64 {
    let seconds = if height < 1_000_000 {
        // interest stop accruing after 1 year before block 1000000
        365 * 24 * 60 * 60
    } else {
        // interest stop accruing after 1 month past 1000000 block
        31 * 24 * 60 * 60
    };

    lock_time + seconds
}

fn kmd_interest_accrue_start_at(lock_time: u64) -> u64 {
    let one_hour = 60 * 60;
    lock_time + one_hour
}

#[derive(Debug, Serialize, Eq, PartialEq)]
enum KmdRewardsNotAccruedReason {
    LocktimeNotSet,
    LocktimeLessThanThreshold,
    UtxoHeightGreaterThanEndOfEra,
    UtxoAmountLessThanTen,
    OneHourNotPassedYet,
    TransactionInMempool,
}

#[derive(Serialize)]
enum KmdRewardsAccrueInfo {
    Accrued(BigDecimal),
    NotAccruedReason(KmdRewardsNotAccruedReason),
}

#[derive(Serialize)]
pub struct KmdRewardsInfoElement {
    tx_hash: H256Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u64>,
    /// The zero-based index of the output in the transaction’s list of outputs.
    output_index: u32,
    amount: BigDecimal,
    locktime: u64,
    /// Amount of accrued rewards.
    accrued_rewards: KmdRewardsAccrueInfo,
    /// Rewards start to accrue at this time for the given transaction.
    /// None if the rewards will not be accrued.
    #[serde(skip_serializing_if = "Option::is_none")]
    accrue_start_at: Option<u64>,
    /// Rewards stop to accrue at this time for the given transaction.
    /// None if the rewards will not be accrued.
    #[serde(skip_serializing_if = "Option::is_none")]
    accrue_stop_at: Option<u64>,
}

/// Get rewards info of unspent outputs.
/// The list is ordered by the output value.
pub async fn kmd_rewards_info<T>(coin: &T) -> Result<Vec<KmdRewardsInfoElement>, String>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps,
{
    if coin.as_ref().conf.ticker != "KMD" {
        return ERR!("rewards info can be obtained for KMD only");
    }

    let utxo = coin.as_ref();
    let my_address = try_s!(utxo.derivation_method.iguana_or_err());
    let rpc_client = &utxo.rpc_client;
    let mut unspents = try_s!(rpc_client.list_unspent(my_address, utxo.decimals).compat().await);
    // list_unspent_ordered() returns ordered from lowest to highest by value unspent outputs.
    // reverse it to reorder from highest to lowest outputs.
    unspents.reverse();

    let mut result = Vec::with_capacity(unspents.len());
    for unspent in unspents {
        let tx_hash: H256Json = unspent.outpoint.hash.reversed().into();
        let tx_info = try_s!(rpc_client.get_verbose_transaction(&tx_hash).compat().await);

        let value = unspent.value;
        let locktime = tx_info.locktime as u64;
        let current_time = try_s!(coin.get_current_mtp().await) as u64;
        let accrued_rewards = match kmd_interest(tx_info.height, value, locktime, current_time) {
            Ok(interest) => {
                KmdRewardsAccrueInfo::Accrued(big_decimal_from_sat(interest as i64, coin.as_ref().decimals))
            },
            Err(reason) => KmdRewardsAccrueInfo::NotAccruedReason(reason),
        };

        // `accrue_start_at` and `accrue_stop_at` should be None if the rewards will never be obtained for the given transaction
        let (accrue_start_at, accrue_stop_at) = match &accrued_rewards {
            KmdRewardsAccrueInfo::Accrued(_)
            | KmdRewardsAccrueInfo::NotAccruedReason(KmdRewardsNotAccruedReason::TransactionInMempool)
            | KmdRewardsAccrueInfo::NotAccruedReason(KmdRewardsNotAccruedReason::OneHourNotPassedYet) => {
                let start_at = Some(kmd_interest_accrue_start_at(locktime));
                let stop_at = tx_info
                    .height
                    .map(|height| kmd_interest_accrue_stop_at(height, locktime));
                (start_at, stop_at)
            },
            _ => (None, None),
        };

        result.push(KmdRewardsInfoElement {
            tx_hash,
            height: tx_info.height,
            output_index: unspent.outpoint.index,
            amount: big_decimal_from_sat(value as i64, coin.as_ref().decimals),
            locktime,
            accrued_rewards,
            accrue_start_at,
            accrue_stop_at,
        });
    }

    Ok(result)
}

/// Denominate BigDecimal amount of coin units to satoshis
pub fn sat_from_big_decimal(amount: &BigDecimal, decimals: u8) -> NumConversResult<u64> {
    (amount * BigDecimal::from(10u64.pow(decimals as u32)))
        .to_u64()
        .or_mm_err(|| {
            let err = format!("Could not get sat from amount {} with decimals {}", amount, decimals);
            NumConversError::new(err)
        })
}

async fn send_outputs_from_my_address_impl<T>(coin: T, outputs: Vec<TransactionOutput>) -> Result<UtxoTx, String>
where
    T: AsRef<UtxoCoinFields> + UtxoCommonOps,
{
    let my_address = try_s!(coin.as_ref().derivation_method.iguana_or_err());
    let (unspents, recently_sent_txs) = try_s!(coin.list_unspent_ordered(my_address).await);
    generate_and_send_tx(&coin, unspents, None, FeePolicy::SendExact, recently_sent_txs, outputs).await
}

/// Generates and sends tx using unspents and outputs adding new record to the recently_spent in case of success
async fn generate_and_send_tx<T>(
    coin: &T,
    unspents: Vec<UnspentInfo>,
    required_inputs: Option<Vec<UnspentInfo>>,
    fee_policy: FeePolicy,
    mut recently_spent: AsyncMutexGuard<'_, RecentlySpentOutPoints>,
    outputs: Vec<TransactionOutput>,
) -> Result<UtxoTx, String>
where
    T: AsRef<UtxoCoinFields> + UtxoTxGenerationOps + UtxoTxBroadcastOps,
{
    let my_address = try_s!(coin.as_ref().derivation_method.iguana_or_err());
    let key_pair = try_s!(coin.as_ref().priv_key_policy.key_pair_or_err());

    let mut builder = UtxoTxBuilder::new(coin)
        .add_available_inputs(unspents)
        .add_outputs(outputs)
        .with_fee_policy(fee_policy);
    if let Some(required) = required_inputs {
        builder = builder.add_required_inputs(required);
    }
    let (unsigned, _) = try_s!(builder.build().await);

    let spent_unspents = unsigned
        .inputs
        .iter()
        .map(|input| UnspentInfo {
            outpoint: input.previous_output.clone(),
            value: input.amount,
            height: None,
        })
        .collect();

    let signature_version = match &my_address.addr_format {
        UtxoAddressFormat::Segwit => SignatureVersion::WitnessV0,
        _ => coin.as_ref().conf.signature_version,
    };

    let prev_script = Builder::build_p2pkh(&my_address.hash);
    let signed = try_s!(sign_tx(
        unsigned,
        key_pair,
        prev_script,
        signature_version,
        coin.as_ref().conf.fork_id
    ));

    try_s!(coin.broadcast_tx(&signed).await);

    recently_spent.add_spent(spent_unspents, signed.hash(), signed.outputs.clone());

    Ok(signed)
}

pub fn output_script(address: &Address, script_type: ScriptType) -> Script {
    match address.addr_format {
        UtxoAddressFormat::Segwit => Builder::build_witness_script(&address.hash),
        _ => match script_type {
            ScriptType::P2PKH => Builder::build_p2pkh(&address.hash),
            ScriptType::P2SH => Builder::build_p2sh(&address.hash),
            ScriptType::P2WPKH => Builder::build_witness_script(&address.hash),
            ScriptType::P2WSH => Builder::build_witness_script(&address.hash),
        },
    }
}

pub fn address_by_conf_and_pubkey_str(
    coin: &str,
    conf: &Json,
    pubkey: &str,
    addr_format: UtxoAddressFormat,
) -> Result<String, String> {
    // using a reasonable default here
    let params = UtxoActivationParams {
        mode: UtxoRpcMode::Native,
        utxo_merge_params: None,
        tx_history: false,
        required_confirmations: None,
        requires_notarization: None,
        address_format: None,
    };
    let conf_builder = UtxoConfBuilder::new(conf, params, coin);
    let utxo_conf = try_s!(conf_builder.build());
    let pubkey_bytes = try_s!(hex::decode(pubkey));
    let hash = dhash160(&pubkey_bytes);

    let address = Address {
        prefix: utxo_conf.pub_addr_prefix,
        t_addr_prefix: utxo_conf.pub_t_addr_prefix,
        hash: hash.into(),
        checksum_type: utxo_conf.checksum_type,
        hrp: utxo_conf.bech32_hrp,
        addr_format,
    };
    address.display_address()
}

fn parse_hex_encoded_u32(hex_encoded: &str) -> Result<u32, MmError<String>> {
    let hex_encoded = hex_encoded.strip_prefix("0x").unwrap_or(hex_encoded);
    let bytes = hex::decode(hex_encoded).map_to_mm(|e| e.to_string())?;
    let be_bytes: [u8; 4] = bytes
        .as_slice()
        .try_into()
        .map_to_mm(|e: TryFromSliceError| e.to_string())?;
    Ok(u32::from_be_bytes(be_bytes))
}

#[test]
fn test_parse_hex_encoded_u32() {
    assert_eq!(parse_hex_encoded_u32("0x892f2085"), Ok(2301567109));
    assert_eq!(parse_hex_encoded_u32("892f2085"), Ok(2301567109));
    assert_eq!(parse_hex_encoded_u32("0x7361707a"), Ok(1935765626));
}
