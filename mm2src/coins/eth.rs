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
//  eth.rs
//  marketmaker
//
//  Copyright © 2022 AtomicDEX. All rights reserved.
//
use super::eth::Action::{Call, Create};
use crate::lp_price::get_base_price_in_rel;
use crate::nft::nft_structs::{ContractType, ConvertChain, NftListReq, TransactionNftDetails, WithdrawErc1155,
                              WithdrawErc721};
use async_trait::async_trait;
use bitcrypto::{keccak256, ripemd160, sha256};
use common::custom_futures::repeatable::{Ready, Retry, RetryOnError};
use common::custom_futures::timeout::FutureTimerExt;
use common::executor::{abortable_queue::AbortableQueue, AbortableSystem, AbortedError, Timer};
use common::log::{debug, error, info, warn};
use common::number_type_casting::SafeTypeCastingNumbers;
use common::{get_utc_timestamp, now_sec, small_rng, DEX_FEE_ADDR_RAW_PUBKEY};
#[cfg(target_arch = "wasm32")]
use common::{now_ms, wait_until_ms};
use crypto::privkey::key_pair_from_secret;
use crypto::{CryptoCtx, CryptoCtxError, GlobalHDAccountArc, KeyPairPolicy};
use derive_more::Display;
use enum_from::EnumFromStringify;
use ethabi::{Contract, Function, Token};
pub use ethcore_transaction::SignedTransaction as SignedEthTx;
use ethcore_transaction::{Action, Transaction as UnSignedEthTx, UnverifiedTransaction};
use ethereum_types::{Address, H160, H256, U256};
use ethkey::{public_to_address, KeyPair, Public, Signature};
use ethkey::{sign, verify_address};
use futures::compat::Future01CompatExt;
use futures::future::{join_all, select_ok, try_join_all, Either, FutureExt, TryFutureExt};
use futures01::Future;
use http::StatusCode;
use mm2_core::mm_ctx::{MmArc, MmWeak};
use mm2_err_handle::prelude::*;
use mm2_net::transport::{slurp_url, GuiAuthValidation, GuiAuthValidationGenerator, SlurpError};
use mm2_number::bigdecimal_custom::CheckedDivision;
use mm2_number::{BigDecimal, MmNumber};
#[cfg(test)] use mocktopus::macros::*;
use rand::seq::SliceRandom;
use rpc::v1::types::Bytes as BytesJson;
use secp256k1::PublicKey;
use serde_json::{self as json, Value as Json};
use serialization::{CompactInteger, Serializable, Stream};
use sha3::{Digest, Keccak256};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::ops::Deref;
#[cfg(not(target_arch = "wasm32"))] use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use url::Url;
use web3::types::{Action as TraceAction, BlockId, BlockNumber, Bytes, CallRequest, FilterBuilder, Log, Trace,
                  TraceFilterBuilder, Transaction as Web3Transaction, TransactionId, U64};
use web3::{self, Web3};
use web3_transport::{http_transport::HttpTransportNode, EthFeeHistoryNamespace, Web3Transport};

cfg_wasm32! {
    use crypto::MetamaskArc;
    use ethereum_types::{H264, H520};
    use mm2_metamask::MetamaskError;
    use web3::types::TransactionRequest;
}

use super::watcher_common::{validate_watcher_reward, REWARD_GAS_AMOUNT};
use super::{coin_conf, lp_coinfind_or_err, AsyncMutex, BalanceError, BalanceFut, CheckIfMyPaymentSentArgs,
            CoinBalance, CoinFutSpawner, CoinProtocol, CoinTransportMetrics, CoinsContext, ConfirmPaymentInput,
            EthValidateFeeArgs, FeeApproxStage, FoundSwapTxSpend, HistorySyncState, IguanaPrivKey, MakerSwapTakerCoin,
            MarketCoinOps, MmCoin, MmCoinEnum, MyAddressError, MyWalletAddress, NegotiateSwapContractAddrErr,
            NumConversError, NumConversResult, PaymentInstructionArgs, PaymentInstructions, PaymentInstructionsErr,
            PrivKeyBuildPolicy, PrivKeyPolicyNotAllowed, RawTransactionError, RawTransactionFut,
            RawTransactionRequest, RawTransactionRes, RawTransactionResult, RefundError, RefundPaymentArgs,
            RefundResult, RewardTarget, RpcClientType, RpcTransportEventHandler, RpcTransportEventHandlerShared,
            SearchForSwapTxSpendInput, SendMakerPaymentSpendPreimageInput, SendPaymentArgs, SignatureError,
            SignatureResult, SpendPaymentArgs, SwapOps, TakerSwapMakerCoin, TradeFee, TradePreimageError,
            TradePreimageFut, TradePreimageResult, TradePreimageValue, Transaction, TransactionDetails,
            TransactionEnum, TransactionErr, TransactionFut, TransactionType, TxMarshalingErr,
            UnexpectedDerivationMethod, ValidateAddressResult, ValidateFeeArgs, ValidateInstructionsErr,
            ValidateOtherPubKeyErr, ValidatePaymentError, ValidatePaymentFut, ValidatePaymentInput, VerificationError,
            VerificationResult, WaitForHTLCTxSpendArgs, WatcherOps, WatcherReward, WatcherRewardError,
            WatcherSearchForSwapTxSpendInput, WatcherValidatePaymentInput, WatcherValidateTakerFeeInput,
            WithdrawError, WithdrawFee, WithdrawFut, WithdrawRequest, WithdrawResult, EARLY_CONFIRMATION_ERR_LOG,
            INVALID_CONTRACT_ADDRESS_ERR_LOG, INVALID_PAYMENT_STATE_ERR_LOG, INVALID_RECEIVER_ERR_LOG,
            INVALID_SENDER_ERR_LOG, INVALID_SWAP_ID_ERR_LOG};
pub use rlp;

#[cfg(test)] mod eth_tests;
#[cfg(target_arch = "wasm32")] mod eth_wasm_tests;
mod web3_transport;

#[path = "eth/v2_activation.rs"] pub mod v2_activation;
use crate::nft::{find_wallet_amount, WithdrawNftResult};
use v2_activation::{build_address_and_priv_key_policy, EthActivationV2Error};

mod nonce;
use nonce::ParityNonce;

/// https://github.com/artemii235/etomic-swap/blob/master/contracts/EtomicSwap.sol
/// Dev chain (195.201.137.5:8565) contract address: 0x83965C539899cC0F918552e5A26915de40ee8852
/// Ropsten: https://ropsten.etherscan.io/address/0x7bc1bbdd6a0a722fc9bffc49c921b685ecb84b94
/// ETH mainnet: https://etherscan.io/address/0x8500AFc0bc5214728082163326C2FF0C73f4a871
const SWAP_CONTRACT_ABI: &str = include_str!("eth/swap_contract_abi.json");
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-20.md
const ERC20_ABI: &str = include_str!("eth/erc20_abi.json");
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-721.md
const ERC721_ABI: &str = include_str!("eth/erc721_abi.json");
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1155.md
const ERC1155_ABI: &str = include_str!("eth/erc1155_abi.json");
/// Payment states from etomic swap smart contract: https://github.com/artemii235/etomic-swap/blob/master/contracts/EtomicSwap.sol#L5
pub enum PaymentState {
    Uninitialized,
    Sent,
    Spent,
    Refunded,
}
// Ethgasstation API returns response in 10^8 wei units. So 10 from their API mean 1 gwei
const ETH_GAS_STATION_DECIMALS: u8 = 8;
const GAS_PRICE_PERCENT: u64 = 10;
/// It can change 12.5% max each block according to https://www.blocknative.com/blog/eip-1559-fees
const BASE_BLOCK_FEE_DIFF_PCT: u64 = 13;
const DEFAULT_LOGS_BLOCK_RANGE: u64 = 1000;

const DEFAULT_REQUIRED_CONFIRMATIONS: u8 = 1;

const ETH_DECIMALS: u8 = 18;

/// Take into account that the dynamic fee may increase by 3% during the swap.
const GAS_PRICE_APPROXIMATION_PERCENT_ON_START_SWAP: u64 = 3;
/// Take into account that the dynamic fee may increase until the locktime is expired
const GAS_PRICE_APPROXIMATION_PERCENT_ON_WATCHER_PREIMAGE: u64 = 3;
/// Take into account that the dynamic fee may increase at each of the following stages:
/// - it may increase by 2% until a swap is started;
/// - it may increase by 3% during the swap.
const GAS_PRICE_APPROXIMATION_PERCENT_ON_ORDER_ISSUE: u64 = 5;
/// Take into account that the dynamic fee may increase at each of the following stages:
/// - it may increase by 2% until an order is issued;
/// - it may increase by 2% until a swap is started;
/// - it may increase by 3% during the swap.
const GAS_PRICE_APPROXIMATION_PERCENT_ON_TRADE_PREIMAGE: u64 = 7;

const ETH_GAS: u64 = 150_000;

/// Lifetime of generated signed message for gui-auth requests
const GUI_AUTH_SIGNED_MESSAGE_LIFETIME_SEC: i64 = 90;

lazy_static! {
    pub static ref SWAP_CONTRACT: Contract = Contract::load(SWAP_CONTRACT_ABI.as_bytes()).unwrap();
    pub static ref ERC20_CONTRACT: Contract = Contract::load(ERC20_ABI.as_bytes()).unwrap();
    pub static ref ERC721_CONTRACT: Contract = Contract::load(ERC721_ABI.as_bytes()).unwrap();
    pub static ref ERC1155_CONTRACT: Contract = Contract::load(ERC1155_ABI.as_bytes()).unwrap();
}

pub type Web3RpcFut<T> = Box<dyn Future<Item = T, Error = MmError<Web3RpcError>> + Send>;
pub type Web3RpcResult<T> = Result<T, MmError<Web3RpcError>>;
pub type GasStationResult = Result<GasStationData, MmError<GasStationReqErr>>;
type GasDetails = (U256, U256);

#[derive(Debug, Display)]
pub enum GasStationReqErr {
    #[display(fmt = "Transport '{}' error: {}", uri, error)]
    Transport {
        uri: String,
        error: String,
    },
    #[display(fmt = "Invalid response: {}", _0)]
    InvalidResponse(String),
    Internal(String),
}

impl From<serde_json::Error> for GasStationReqErr {
    fn from(e: serde_json::Error) -> Self { GasStationReqErr::InvalidResponse(e.to_string()) }
}

impl From<SlurpError> for GasStationReqErr {
    fn from(e: SlurpError) -> Self {
        let error = e.to_string();
        match e {
            SlurpError::ErrorDeserializing { .. } => GasStationReqErr::InvalidResponse(error),
            SlurpError::Transport { uri, .. } | SlurpError::Timeout { uri, .. } => {
                GasStationReqErr::Transport { uri, error }
            },
            SlurpError::Internal(_) | SlurpError::InvalidRequest(_) => GasStationReqErr::Internal(error),
        }
    }
}

#[derive(Debug, Display)]
pub enum Web3RpcError {
    #[display(fmt = "Transport: {}", _0)]
    Transport(String),
    #[display(fmt = "Invalid response: {}", _0)]
    InvalidResponse(String),
    #[display(fmt = "Timeout: {}", _0)]
    Timeout(String),
    #[display(fmt = "Internal: {}", _0)]
    Internal(String),
}

impl From<GasStationReqErr> for Web3RpcError {
    fn from(err: GasStationReqErr) -> Self {
        match err {
            GasStationReqErr::Transport { .. } => Web3RpcError::Transport(err.to_string()),
            GasStationReqErr::InvalidResponse(err) => Web3RpcError::InvalidResponse(err),
            GasStationReqErr::Internal(err) => Web3RpcError::Internal(err),
        }
    }
}

impl From<serde_json::Error> for Web3RpcError {
    fn from(e: serde_json::Error) -> Self { Web3RpcError::InvalidResponse(e.to_string()) }
}

impl From<web3::Error> for Web3RpcError {
    fn from(e: web3::Error) -> Self {
        let error_str = e.to_string();
        match e {
            web3::Error::InvalidResponse(_) | web3::Error::Decoder(_) | web3::Error::Rpc(_) => {
                Web3RpcError::InvalidResponse(error_str)
            },
            web3::Error::Unreachable | web3::Error::Transport(_) | web3::Error::Io(_) => {
                Web3RpcError::Transport(error_str)
            },
            _ => Web3RpcError::Internal(error_str),
        }
    }
}

impl From<web3::Error> for RawTransactionError {
    fn from(e: web3::Error) -> Self { RawTransactionError::Transport(e.to_string()) }
}

impl From<Web3RpcError> for RawTransactionError {
    fn from(e: Web3RpcError) -> Self {
        match e {
            Web3RpcError::Transport(tr) | Web3RpcError::InvalidResponse(tr) => RawTransactionError::Transport(tr),
            Web3RpcError::Internal(internal) | Web3RpcError::Timeout(internal) => {
                RawTransactionError::InternalError(internal)
            },
        }
    }
}

impl From<ethabi::Error> for Web3RpcError {
    fn from(e: ethabi::Error) -> Web3RpcError {
        // Currently, we use the `ethabi` crate to work with a smart contract ABI known at compile time.
        // It's an internal error if there are any issues during working with a smart contract ABI.
        Web3RpcError::Internal(e.to_string())
    }
}

#[cfg(target_arch = "wasm32")]
impl From<MetamaskError> for Web3RpcError {
    fn from(e: MetamaskError) -> Self {
        match e {
            MetamaskError::Internal(internal) => Web3RpcError::Internal(internal),
            other => Web3RpcError::Transport(other.to_string()),
        }
    }
}

impl From<ethabi::Error> for WithdrawError {
    fn from(e: ethabi::Error) -> Self {
        // Currently, we use the `ethabi` crate to work with a smart contract ABI known at compile time.
        // It's an internal error if there are any issues during working with a smart contract ABI.
        WithdrawError::InternalError(e.to_string())
    }
}

impl From<web3::Error> for WithdrawError {
    fn from(e: web3::Error) -> Self { WithdrawError::Transport(e.to_string()) }
}

impl From<Web3RpcError> for WithdrawError {
    fn from(e: Web3RpcError) -> Self {
        match e {
            Web3RpcError::Transport(err) | Web3RpcError::InvalidResponse(err) => WithdrawError::Transport(err),
            Web3RpcError::Internal(internal) | Web3RpcError::Timeout(internal) => {
                WithdrawError::InternalError(internal)
            },
        }
    }
}

impl From<web3::Error> for TradePreimageError {
    fn from(e: web3::Error) -> Self { TradePreimageError::Transport(e.to_string()) }
}

impl From<Web3RpcError> for TradePreimageError {
    fn from(e: Web3RpcError) -> Self {
        match e {
            Web3RpcError::Transport(err) | Web3RpcError::InvalidResponse(err) => TradePreimageError::Transport(err),
            Web3RpcError::Internal(internal) | Web3RpcError::Timeout(internal) => {
                TradePreimageError::InternalError(internal)
            },
        }
    }
}

impl From<ethabi::Error> for TradePreimageError {
    fn from(e: ethabi::Error) -> Self {
        // Currently, we use the `ethabi` crate to work with a smart contract ABI known at compile time.
        // It's an internal error if there are any issues during working with a smart contract ABI.
        TradePreimageError::InternalError(e.to_string())
    }
}

impl From<ethabi::Error> for BalanceError {
    fn from(e: ethabi::Error) -> Self {
        // Currently, we use the `ethabi` crate to work with a smart contract ABI known at compile time.
        // It's an internal error if there are any issues during working with a smart contract ABI.
        BalanceError::Internal(e.to_string())
    }
}

impl From<web3::Error> for BalanceError {
    fn from(e: web3::Error) -> Self { BalanceError::from(Web3RpcError::from(e)) }
}

impl From<Web3RpcError> for BalanceError {
    fn from(e: Web3RpcError) -> Self {
        match e {
            Web3RpcError::Transport(tr) | Web3RpcError::InvalidResponse(tr) => BalanceError::Transport(tr),
            Web3RpcError::Internal(internal) | Web3RpcError::Timeout(internal) => BalanceError::Internal(internal),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct SavedTraces {
    /// ETH traces for my_address
    traces: Vec<Trace>,
    /// Earliest processed block
    earliest_block: U64,
    /// Latest processed block
    latest_block: U64,
}

#[derive(Debug, Deserialize, Serialize)]
struct SavedErc20Events {
    /// ERC20 events for my_address
    events: Vec<Log>,
    /// Earliest processed block
    earliest_block: U64,
    /// Latest processed block
    latest_block: U64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EthCoinType {
    /// Ethereum itself or it's forks: ETC/others
    Eth,
    /// ERC20 token with smart contract address
    /// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-20.md
    Erc20 { platform: String, token_addr: Address },
}

/// An alternative to `crate::PrivKeyBuildPolicy`, typical only for ETH coin.
pub enum EthPrivKeyBuildPolicy {
    IguanaPrivKey(IguanaPrivKey),
    GlobalHDAccount(GlobalHDAccountArc),
    #[cfg(target_arch = "wasm32")]
    Metamask(MetamaskArc),
}

impl EthPrivKeyBuildPolicy {
    /// Detects the `EthPrivKeyBuildPolicy` with which the given `MmArc` is initialized.
    pub fn detect_priv_key_policy(ctx: &MmArc) -> MmResult<EthPrivKeyBuildPolicy, CryptoCtxError> {
        let crypto_ctx = CryptoCtx::from_ctx(ctx)?;

        match crypto_ctx.key_pair_policy() {
            KeyPairPolicy::Iguana => {
                // Use an internal private key as the coin secret.
                let priv_key = crypto_ctx.mm2_internal_privkey_secret();
                Ok(EthPrivKeyBuildPolicy::IguanaPrivKey(priv_key))
            },
            KeyPairPolicy::GlobalHDAccount(global_hd) => Ok(EthPrivKeyBuildPolicy::GlobalHDAccount(global_hd.clone())),
        }
    }
}

impl TryFrom<PrivKeyBuildPolicy> for EthPrivKeyBuildPolicy {
    type Error = PrivKeyPolicyNotAllowed;

    /// Converts `PrivKeyBuildPolicy` to `EthPrivKeyBuildPolicy`
    /// taking into account that  ETH doesn't support `Trezor` yet.
    fn try_from(policy: PrivKeyBuildPolicy) -> Result<Self, Self::Error> {
        match policy {
            PrivKeyBuildPolicy::IguanaPrivKey(iguana) => Ok(EthPrivKeyBuildPolicy::IguanaPrivKey(iguana)),
            PrivKeyBuildPolicy::GlobalHDAccount(global_hd) => Ok(EthPrivKeyBuildPolicy::GlobalHDAccount(global_hd)),
            PrivKeyBuildPolicy::Trezor => Err(PrivKeyPolicyNotAllowed::HardwareWalletNotSupported),
        }
    }
}

/// An alternative to `crate::PrivKeyPolicy`, typical only for ETH coin.
#[derive(Clone)]
pub enum EthPrivKeyPolicy {
    KeyPair(KeyPair),
    #[cfg(target_arch = "wasm32")]
    Metamask(EthMetamaskPolicy),
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
pub struct EthMetamaskPolicy {
    pub(crate) public_key: H264,
    pub(crate) public_key_uncompressed: H520,
}

impl From<KeyPair> for EthPrivKeyPolicy {
    fn from(key_pair: KeyPair) -> Self { EthPrivKeyPolicy::KeyPair(key_pair) }
}

impl EthPrivKeyPolicy {
    pub fn key_pair_or_err(&self) -> MmResult<&KeyPair, PrivKeyPolicyNotAllowed> {
        match self {
            EthPrivKeyPolicy::KeyPair(key_pair) => Ok(key_pair),
            #[cfg(target_arch = "wasm32")]
            EthPrivKeyPolicy::Metamask(_) => MmError::err(PrivKeyPolicyNotAllowed::HardwareWalletNotSupported),
        }
    }
}

/// pImpl idiom.
pub struct EthCoinImpl {
    ticker: String,
    pub coin_type: EthCoinType,
    priv_key_policy: EthPrivKeyPolicy,
    my_address: Address,
    sign_message_prefix: Option<String>,
    swap_contract_address: Address,
    fallback_swap_contract: Option<Address>,
    contract_supports_watchers: bool,
    web3: Web3<Web3Transport>,
    /// The separate web3 instances kept to get nonce, will replace the web3 completely soon
    web3_instances: Vec<Web3Instance>,
    decimals: u8,
    gas_station_url: Option<String>,
    gas_station_decimals: u8,
    gas_station_policy: GasStationPricePolicy,
    history_sync_state: Mutex<HistorySyncState>,
    required_confirmations: AtomicU64,
    /// Coin needs access to the context in order to reuse the logging and shutdown facilities.
    /// Using a weak reference by default in order to avoid circular references and leaks.
    pub ctx: MmWeak,
    chain_id: Option<u64>,
    /// the block range used for eth_getLogs
    logs_block_range: u64,
    nonce_lock: Arc<AsyncMutex<()>>,
    erc20_tokens_infos: Arc<Mutex<HashMap<String, Erc20TokenInfo>>>,
    /// This spawner is used to spawn coin's related futures that should be aborted on coin deactivation
    /// and on [`MmArc::stop`].
    pub abortable_system: AbortableQueue,
}

#[derive(Clone, Debug)]
pub struct Web3Instance {
    web3: Web3<Web3Transport>,
    is_parity: bool,
}

#[derive(Clone, Debug)]
pub struct Erc20TokenInfo {
    pub token_address: Address,
    pub decimals: u8,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "format")]
pub enum EthAddressFormat {
    /// Single-case address (lowercase)
    #[serde(rename = "singlecase")]
    SingleCase,
    /// Mixed-case address.
    /// https://eips.ethereum.org/EIPS/eip-55
    #[serde(rename = "mixedcase")]
    MixedCase,
}

#[cfg_attr(test, mockable)]
async fn make_gas_station_request(url: &str) -> GasStationResult {
    let resp = slurp_url(url).await?;
    if resp.0 != StatusCode::OK {
        let error = format!("Gas price request failed with status code {}", resp.0);
        return MmError::err(GasStationReqErr::Transport {
            uri: url.to_owned(),
            error,
        });
    }
    let result: GasStationData = json::from_slice(&resp.2)?;
    Ok(result)
}

impl EthCoinImpl {
    /// Gets Transfer events from ERC20 smart contract `addr` between `from_block` and `to_block`
    fn erc20_transfer_events(
        &self,
        contract: Address,
        from_addr: Option<Address>,
        to_addr: Option<Address>,
        from_block: BlockNumber,
        to_block: BlockNumber,
        limit: Option<usize>,
    ) -> Box<dyn Future<Item = Vec<Log>, Error = String> + Send> {
        let contract_event = try_fus!(ERC20_CONTRACT.event("Transfer"));
        let topic0 = Some(vec![contract_event.signature()]);
        let topic1 = from_addr.map(|addr| vec![addr.into()]);
        let topic2 = to_addr.map(|addr| vec![addr.into()]);
        let mut filter = FilterBuilder::default()
            .topics(topic0, topic1, topic2, None)
            .from_block(from_block)
            .to_block(to_block)
            .address(vec![contract]);

        if let Some(l) = limit {
            filter = filter.limit(l);
        }

        Box::new(
            self.web3
                .eth()
                .logs(filter.build())
                .compat()
                .map_err(|e| ERRL!("{}", e)),
        )
    }

    /// Gets ETH traces from ETH node between addresses in `from_block` and `to_block`
    fn eth_traces(
        &self,
        from_addr: Vec<Address>,
        to_addr: Vec<Address>,
        from_block: BlockNumber,
        to_block: BlockNumber,
        limit: Option<usize>,
    ) -> Box<dyn Future<Item = Vec<Trace>, Error = String> + Send> {
        let mut filter = TraceFilterBuilder::default()
            .from_address(from_addr)
            .to_address(to_addr)
            .from_block(from_block)
            .to_block(to_block);

        if let Some(l) = limit {
            filter = filter.count(l);
        }

        Box::new(
            self.web3
                .trace()
                .filter(filter.build())
                .compat()
                .map_err(|e| ERRL!("{}", e)),
        )
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn eth_traces_path(&self, ctx: &MmArc) -> PathBuf {
        ctx.dbdir()
            .join("TRANSACTIONS")
            .join(format!("{}_{:#02x}_trace.json", self.ticker, self.my_address))
    }

    /// Load saved ETH traces from local DB
    #[cfg(not(target_arch = "wasm32"))]
    fn load_saved_traces(&self, ctx: &MmArc) -> Option<SavedTraces> {
        let content = gstuff::slurp(&self.eth_traces_path(ctx));
        if content.is_empty() {
            None
        } else {
            match json::from_slice(&content) {
                Ok(t) => Some(t),
                Err(_) => None,
            }
        }
    }

    /// Load saved ETH traces from local DB
    #[cfg(target_arch = "wasm32")]
    fn load_saved_traces(&self, _ctx: &MmArc) -> Option<SavedTraces> {
        common::panic_w("'load_saved_traces' is not implemented in WASM");
        unreachable!()
    }

    /// Store ETH traces to local DB
    #[cfg(not(target_arch = "wasm32"))]
    fn store_eth_traces(&self, ctx: &MmArc, traces: &SavedTraces) {
        let content = json::to_vec(traces).unwrap();
        let tmp_file = format!("{}.tmp", self.eth_traces_path(ctx).display());
        std::fs::write(&tmp_file, content).unwrap();
        std::fs::rename(tmp_file, self.eth_traces_path(ctx)).unwrap();
    }

    /// Store ETH traces to local DB
    #[cfg(target_arch = "wasm32")]
    fn store_eth_traces(&self, _ctx: &MmArc, _traces: &SavedTraces) {
        common::panic_w("'store_eth_traces' is not implemented in WASM");
        unreachable!()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn erc20_events_path(&self, ctx: &MmArc) -> PathBuf {
        ctx.dbdir()
            .join("TRANSACTIONS")
            .join(format!("{}_{:#02x}_events.json", self.ticker, self.my_address))
    }

    /// Store ERC20 events to local DB
    #[cfg(not(target_arch = "wasm32"))]
    fn store_erc20_events(&self, ctx: &MmArc, events: &SavedErc20Events) {
        let content = json::to_vec(events).unwrap();
        let tmp_file = format!("{}.tmp", self.erc20_events_path(ctx).display());
        std::fs::write(&tmp_file, content).unwrap();
        std::fs::rename(tmp_file, self.erc20_events_path(ctx)).unwrap();
    }

    /// Store ERC20 events to local DB
    #[cfg(target_arch = "wasm32")]
    fn store_erc20_events(&self, _ctx: &MmArc, _events: &SavedErc20Events) {
        common::panic_w("'store_erc20_events' is not implemented in WASM");
        unreachable!()
    }

    /// Load saved ERC20 events from local DB
    #[cfg(not(target_arch = "wasm32"))]
    fn load_saved_erc20_events(&self, ctx: &MmArc) -> Option<SavedErc20Events> {
        let content = gstuff::slurp(&self.erc20_events_path(ctx));
        if content.is_empty() {
            None
        } else {
            match json::from_slice(&content) {
                Ok(t) => Some(t),
                Err(_) => None,
            }
        }
    }

    /// Load saved ERC20 events from local DB
    #[cfg(target_arch = "wasm32")]
    fn load_saved_erc20_events(&self, _ctx: &MmArc) -> Option<SavedErc20Events> {
        common::panic_w("'load_saved_erc20_events' is not implemented in WASM");
        unreachable!()
    }

    /// The id used to differentiate payments on Etomic swap smart contract
    fn etomic_swap_id(&self, time_lock: u32, secret_hash: &[u8]) -> Vec<u8> {
        let mut input = vec![];
        input.extend_from_slice(&time_lock.to_le_bytes());
        input.extend_from_slice(secret_hash);
        sha256(&input).to_vec()
    }

    /// Gets `SenderRefunded` events from etomic swap smart contract since `from_block`
    fn refund_events(
        &self,
        swap_contract_address: Address,
        from_block: u64,
        to_block: u64,
    ) -> Box<dyn Future<Item = Vec<Log>, Error = String> + Send> {
        let contract_event = try_fus!(SWAP_CONTRACT.event("SenderRefunded"));
        let filter = FilterBuilder::default()
            .topics(Some(vec![contract_event.signature()]), None, None, None)
            .from_block(BlockNumber::Number(from_block.into()))
            .to_block(BlockNumber::Number(to_block.into()))
            .address(vec![swap_contract_address])
            .build();

        Box::new(self.web3.eth().logs(filter).compat().map_err(|e| ERRL!("{}", e)))
    }

    /// Try to parse address from string.
    pub fn address_from_str(&self, address: &str) -> Result<Address, String> {
        Ok(try_s!(valid_addr_from_str(address)))
    }

    pub fn erc20_token_address(&self) -> Option<Address> {
        match self.coin_type {
            EthCoinType::Erc20 { token_addr, .. } => Some(token_addr),
            EthCoinType::Eth => None,
        }
    }

    pub fn add_erc_token_info(&self, ticker: String, info: Erc20TokenInfo) {
        self.erc20_tokens_infos.lock().unwrap().insert(ticker, info);
    }

    /// # Warning
    /// Be very careful using this function since it returns dereferenced clone
    /// of value behind the MutexGuard and makes it non-thread-safe.
    pub fn get_erc_tokens_infos(&self) -> HashMap<String, Erc20TokenInfo> {
        let guard = self.erc20_tokens_infos.lock().unwrap();
        (*guard).clone()
    }
}

async fn get_raw_transaction_impl(coin: EthCoin, req: RawTransactionRequest) -> RawTransactionResult {
    let tx = match req.tx_hash.strip_prefix("0x") {
        Some(tx) => tx,
        None => &req.tx_hash,
    };
    let hash = H256::from_str(tx).map_to_mm(|e| RawTransactionError::InvalidHashError(e.to_string()))?;
    get_tx_hex_by_hash_impl(coin, hash).await
}

async fn get_tx_hex_by_hash_impl(coin: EthCoin, tx_hash: H256) -> RawTransactionResult {
    let web3_tx = coin
        .web3
        .eth()
        .transaction(TransactionId::Hash(tx_hash))
        .await?
        .or_mm_err(|| RawTransactionError::HashNotExist(tx_hash.to_string()))?;
    let raw = signed_tx_from_web3_tx(web3_tx).map_to_mm(RawTransactionError::InternalError)?;
    Ok(RawTransactionRes {
        tx_hex: BytesJson(rlp::encode(&raw).to_vec()),
    })
}

async fn withdraw_impl(coin: EthCoin, req: WithdrawRequest) -> WithdrawResult {
    let to_addr = coin
        .address_from_str(&req.to)
        .map_to_mm(WithdrawError::InvalidAddress)?;
    let my_balance = coin.my_balance().compat().await?;
    let my_balance_dec = u256_to_big_decimal(my_balance, coin.decimals)?;

    let (mut wei_amount, dec_amount) = if req.max {
        (my_balance, my_balance_dec.clone())
    } else {
        let wei_amount = wei_from_big_decimal(&req.amount, coin.decimals)?;
        (wei_amount, req.amount.clone())
    };
    if wei_amount > my_balance {
        return MmError::err(WithdrawError::NotSufficientBalance {
            coin: coin.ticker.clone(),
            available: my_balance_dec.clone(),
            required: dec_amount,
        });
    };
    let (mut eth_value, data, call_addr, fee_coin) = match &coin.coin_type {
        EthCoinType::Eth => (wei_amount, vec![], to_addr, coin.ticker()),
        EthCoinType::Erc20 { platform, token_addr } => {
            let function = ERC20_CONTRACT.function("transfer")?;
            let data = function.encode_input(&[Token::Address(to_addr), Token::Uint(wei_amount)])?;
            (0.into(), data, *token_addr, platform.as_str())
        },
    };
    let eth_value_dec = u256_to_big_decimal(eth_value, coin.decimals)?;

    let (gas, gas_price) =
        get_eth_gas_details(&coin, req.fee, eth_value, data.clone().into(), call_addr, req.max).await?;
    let total_fee = gas * gas_price;
    let total_fee_dec = u256_to_big_decimal(total_fee, coin.decimals)?;

    if req.max && coin.coin_type == EthCoinType::Eth {
        if eth_value < total_fee || wei_amount < total_fee {
            return MmError::err(WithdrawError::AmountTooLow {
                amount: eth_value_dec,
                threshold: total_fee_dec,
            });
        }
        eth_value -= total_fee;
        wei_amount -= total_fee;
    };

    let (tx_hash, tx_hex) = match coin.priv_key_policy {
        EthPrivKeyPolicy::KeyPair(ref key_pair) => {
            let _nonce_lock = coin.nonce_lock.lock().await;
            let (nonce, _) = get_addr_nonce(coin.my_address, coin.web3_instances.clone())
                .compat()
                .timeout_secs(30.)
                .await?
                .map_to_mm(WithdrawError::Transport)?;

            let tx = UnSignedEthTx {
                nonce,
                value: eth_value,
                action: Action::Call(call_addr),
                data,
                gas,
                gas_price,
            };

            let signed = tx.sign(key_pair.secret(), coin.chain_id);
            let bytes = rlp::encode(&signed);

            (signed.hash, BytesJson::from(bytes.to_vec()))
        },
        #[cfg(target_arch = "wasm32")]
        EthPrivKeyPolicy::Metamask(_) => {
            if !req.broadcast {
                let error = "Set 'broadcast' to generate, sign and broadcast a transaction with MetaMask".to_string();
                return MmError::err(WithdrawError::BroadcastExpected(error));
            }

            let tx_to_send = TransactionRequest {
                from: coin.my_address,
                to: Some(to_addr),
                gas: Some(gas),
                gas_price: Some(gas_price),
                value: Some(eth_value),
                data: Some(data.clone().into()),
                nonce: None,
                ..TransactionRequest::default()
            };

            // Wait for 10 seconds for the transaction to appear on the RPC node.
            let wait_rpc_timeout = 10_000;
            let check_every = 1.;

            // Please note that this method may take a long time
            // due to `wallet_switchEthereumChain` and `eth_sendTransaction` requests.
            let tx_hash = coin.web3.eth().send_transaction(tx_to_send).await?;

            let signed_tx = coin
                .wait_for_tx_appears_on_rpc(tx_hash, wait_rpc_timeout, check_every)
                .await?;
            let tx_hex = signed_tx
                .map(|tx| BytesJson::from(rlp::encode(&tx).to_vec()))
                // Return an empty `tx_hex` if the transaction is still not appeared on the RPC node.
                .unwrap_or_default();
            (tx_hash, tx_hex)
        },
    };

    let tx_hash_bytes = BytesJson::from(tx_hash.0.to_vec());
    let tx_hash_str = format!("{:02x}", tx_hash_bytes);

    let amount_decimal = u256_to_big_decimal(wei_amount, coin.decimals)?;
    let mut spent_by_me = amount_decimal.clone();
    let received_by_me = if to_addr == coin.my_address {
        amount_decimal.clone()
    } else {
        0.into()
    };
    let fee_details = EthTxFeeDetails::new(gas, gas_price, fee_coin)?;
    if coin.coin_type == EthCoinType::Eth {
        spent_by_me += &fee_details.total_fee;
    }
    let my_address = coin.my_address()?;
    Ok(TransactionDetails {
        to: vec![checksum_address(&format!("{:#02x}", to_addr))],
        from: vec![my_address],
        total_amount: amount_decimal,
        my_balance_change: &received_by_me - &spent_by_me,
        spent_by_me,
        received_by_me,
        tx_hex,
        tx_hash: tx_hash_str,
        block_height: 0,
        fee_details: Some(fee_details.into()),
        coin: coin.ticker.clone(),
        internal_id: vec![].into(),
        timestamp: now_sec(),
        kmd_rewards: None,
        transaction_type: Default::default(),
        memo: None,
    })
}

/// `withdraw_erc1155` function returns details of `ERC-1155` transaction including tx hex,
/// which should be sent to`send_raw_transaction` RPC to broadcast the transaction.
pub async fn withdraw_erc1155(ctx: MmArc, withdraw_type: WithdrawErc1155, url: Url) -> WithdrawNftResult {
    let coin = lp_coinfind_or_err(&ctx, &withdraw_type.chain.to_ticker()).await?;
    let (to_addr, token_addr, eth_coin) =
        get_valid_nft_add_to_withdraw(coin, &withdraw_type.to, &withdraw_type.token_address)?;
    let my_address = eth_coin.my_address()?;

    // todo check amount in nft cache, instead of sending new moralis req
    // dont use `get_nft_metadata` for erc1155, it can return info related to other owner.
    let nft_req = NftListReq {
        chains: vec![withdraw_type.chain],
        url,
    };
    let wallet_amount = find_wallet_amount(
        ctx,
        nft_req,
        withdraw_type.token_address.clone(),
        withdraw_type.token_id.clone(),
    )
    .await?;

    let amount_dec = if withdraw_type.max {
        wallet_amount.clone()
    } else {
        withdraw_type.amount.unwrap_or_else(|| 1.into())
    };

    if amount_dec > wallet_amount {
        return MmError::err(WithdrawError::NotEnoughNftsAmount {
            token_address: withdraw_type.token_address,
            token_id: withdraw_type.token_id.to_string(),
            available: wallet_amount,
            required: amount_dec,
        });
    }

    let (eth_value, data, call_addr, fee_coin) = match eth_coin.coin_type {
        EthCoinType::Eth => {
            let function = ERC1155_CONTRACT.function("safeTransferFrom")?;
            let token_id_u256 = U256::from_dec_str(&withdraw_type.token_id.to_string())
                .map_err(|e| format!("{:?}", e))
                .map_to_mm(NumConversError::new)?;
            let amount_u256 = U256::from_dec_str(&amount_dec.to_string())
                .map_err(|e| format!("{:?}", e))
                .map_to_mm(NumConversError::new)?;
            let data = function.encode_input(&[
                Token::Address(eth_coin.my_address),
                Token::Address(to_addr),
                Token::Uint(token_id_u256),
                Token::Uint(amount_u256),
                Token::Bytes("0x".into()),
            ])?;
            (0.into(), data, token_addr, eth_coin.ticker())
        },
        EthCoinType::Erc20 { .. } => {
            return MmError::err(WithdrawError::InternalError(
                "Erc20 coin type doesnt support withdraw nft".to_owned(),
            ))
        },
    };
    let (gas, gas_price) = get_eth_gas_details(
        &eth_coin,
        withdraw_type.fee,
        eth_value,
        data.clone().into(),
        call_addr,
        false,
    )
    .await?;
    let _nonce_lock = eth_coin.nonce_lock.lock().await;
    let (nonce, _) = get_addr_nonce(eth_coin.my_address, eth_coin.web3_instances.clone())
        .compat()
        .timeout_secs(30.)
        .await?
        .map_to_mm(WithdrawError::Transport)?;

    let tx = UnSignedEthTx {
        nonce,
        value: eth_value,
        action: Action::Call(call_addr),
        data,
        gas,
        gas_price,
    };

    let secret = eth_coin.priv_key_policy.key_pair_or_err()?.secret();
    let signed = tx.sign(secret, eth_coin.chain_id);
    let signed_bytes = rlp::encode(&signed);
    let fee_details = EthTxFeeDetails::new(gas, gas_price, fee_coin)?;

    Ok(TransactionNftDetails {
        tx_hex: BytesJson::from(signed_bytes.to_vec()),
        tx_hash: format!("{:02x}", signed.tx_hash()),
        from: vec![my_address],
        to: vec![withdraw_type.to],
        contract_type: ContractType::Erc1155,
        token_address: withdraw_type.token_address,
        token_id: withdraw_type.token_id,
        amount: amount_dec,
        fee_details: Some(fee_details.into()),
        coin: eth_coin.ticker.clone(),
        block_height: 0,
        timestamp: now_sec(),
        internal_id: 0,
        transaction_type: TransactionType::NftTransfer,
    })
}

/// `withdraw_erc721` function returns details of `ERC-721` transaction including tx hex,
/// which should be sent to`send_raw_transaction` RPC to broadcast the transaction.
pub async fn withdraw_erc721(ctx: MmArc, withdraw_type: WithdrawErc721) -> WithdrawNftResult {
    let coin = lp_coinfind_or_err(&ctx, &withdraw_type.chain.to_ticker()).await?;
    let (to_addr, token_addr, eth_coin) =
        get_valid_nft_add_to_withdraw(coin, &withdraw_type.to, &withdraw_type.token_address)?;
    let my_address = eth_coin.my_address()?;

    let (eth_value, data, call_addr, fee_coin) = match eth_coin.coin_type {
        EthCoinType::Eth => {
            let function = ERC721_CONTRACT.function("safeTransferFrom")?;
            let token_id_u256 = U256::from_dec_str(&withdraw_type.token_id.to_string())
                .map_err(|e| format!("{:?}", e))
                .map_to_mm(NumConversError::new)?;
            let data = function.encode_input(&[
                Token::Address(eth_coin.my_address),
                Token::Address(to_addr),
                Token::Uint(token_id_u256),
            ])?;
            (0.into(), data, token_addr, eth_coin.ticker())
        },
        EthCoinType::Erc20 { .. } => {
            return MmError::err(WithdrawError::InternalError(
                "Erc20 coin type doesnt support withdraw nft".to_owned(),
            ))
        },
    };
    let (gas, gas_price) = get_eth_gas_details(
        &eth_coin,
        withdraw_type.fee,
        eth_value,
        data.clone().into(),
        call_addr,
        false,
    )
    .await?;
    let _nonce_lock = eth_coin.nonce_lock.lock().await;
    let (nonce, _) = get_addr_nonce(eth_coin.my_address, eth_coin.web3_instances.clone())
        .compat()
        .timeout_secs(30.)
        .await?
        .map_to_mm(WithdrawError::Transport)?;

    let tx = UnSignedEthTx {
        nonce,
        value: eth_value,
        action: Action::Call(call_addr),
        data,
        gas,
        gas_price,
    };

    let secret = eth_coin.priv_key_policy.key_pair_or_err()?.secret();
    let signed = tx.sign(secret, eth_coin.chain_id);
    let signed_bytes = rlp::encode(&signed);
    let fee_details = EthTxFeeDetails::new(gas, gas_price, fee_coin)?;

    Ok(TransactionNftDetails {
        tx_hex: BytesJson::from(signed_bytes.to_vec()),
        tx_hash: format!("{:02x}", signed.tx_hash()),
        from: vec![my_address],
        to: vec![withdraw_type.to],
        contract_type: ContractType::Erc721,
        token_address: withdraw_type.token_address,
        token_id: withdraw_type.token_id,
        amount: 1.into(),
        fee_details: Some(fee_details.into()),
        coin: eth_coin.ticker.clone(),
        block_height: 0,
        timestamp: now_sec(),
        internal_id: 0,
        transaction_type: TransactionType::NftTransfer,
    })
}

#[derive(Clone)]
pub struct EthCoin(Arc<EthCoinImpl>);
impl Deref for EthCoin {
    type Target = EthCoinImpl;
    fn deref(&self) -> &EthCoinImpl { &self.0 }
}

#[async_trait]
impl SwapOps for EthCoin {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, _uuid: &[u8]) -> TransactionFut {
        let address = try_tx_fus!(addr_from_raw_pubkey(fee_addr));

        Box::new(
            self.send_to_address(address, try_tx_fus!(wei_from_big_decimal(&amount, self.decimals)))
                .map(TransactionEnum::from),
        )
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

    fn send_maker_spends_taker_payment(&self, maker_spends_payment_args: SpendPaymentArgs) -> TransactionFut {
        Box::new(
            self.spend_hash_time_locked_payment(maker_spends_payment_args)
                .map(TransactionEnum::from),
        )
    }

    fn send_taker_spends_maker_payment(&self, taker_spends_payment_args: SpendPaymentArgs) -> TransactionFut {
        Box::new(
            self.spend_hash_time_locked_payment(taker_spends_payment_args)
                .map(TransactionEnum::from),
        )
    }

    fn send_taker_refunds_payment(&self, taker_refunds_payment_args: RefundPaymentArgs) -> TransactionFut {
        Box::new(
            self.refund_hash_time_locked_payment(taker_refunds_payment_args)
                .map(TransactionEnum::from),
        )
    }

    fn send_maker_refunds_payment(&self, maker_refunds_payment_args: RefundPaymentArgs) -> TransactionFut {
        Box::new(
            self.refund_hash_time_locked_payment(maker_refunds_payment_args)
                .map(TransactionEnum::from),
        )
    }

    fn validate_fee(&self, validate_fee_args: ValidateFeeArgs<'_>) -> ValidatePaymentFut<()> {
        let tx = match validate_fee_args.fee_tx {
            TransactionEnum::SignedEthTx(t) => t.clone(),
            _ => panic!(),
        };
        validate_fee_impl(self.clone(), EthValidateFeeArgs {
            fee_tx_hash: &tx.hash,
            expected_sender: validate_fee_args.expected_sender,
            fee_addr: validate_fee_args.fee_addr,
            amount: validate_fee_args.amount,
            min_block_number: validate_fee_args.min_block_number,
            uuid: validate_fee_args.uuid,
        })
    }

    #[inline]
    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        self.validate_payment(input)
    }

    #[inline]
    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        self.validate_payment(input)
    }

    fn check_if_my_payment_sent(
        &self,
        if_my_payment_sent_args: CheckIfMyPaymentSentArgs,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        let id = self.etomic_swap_id(if_my_payment_sent_args.time_lock, if_my_payment_sent_args.secret_hash);
        let swap_contract_address = try_fus!(if_my_payment_sent_args.swap_contract_address.try_to_address());
        let selfi = self.clone();
        let from_block = if_my_payment_sent_args.search_from_block;
        let fut = async move {
            let status = try_s!(
                selfi
                    .payment_status(swap_contract_address, Token::FixedBytes(id.clone()))
                    .compat()
                    .await
            );

            if status == U256::from(PaymentState::Uninitialized as u8) {
                return Ok(None);
            };

            let mut current_block = try_s!(selfi.current_block().compat().await);
            if current_block < from_block {
                current_block = from_block;
            }

            let mut from_block = from_block;

            loop {
                let to_block = current_block.min(from_block + selfi.logs_block_range);

                let events = try_s!(
                    selfi
                        .payment_sent_events(swap_contract_address, from_block, to_block)
                        .compat()
                        .await
                );

                let found = events.iter().find(|event| &event.data.0[..32] == id.as_slice());

                match found {
                    Some(event) => {
                        let transaction = try_s!(
                            selfi
                                .web3
                                .eth()
                                .transaction(TransactionId::Hash(event.transaction_hash.unwrap()))
                                .await
                        );
                        match transaction {
                            Some(t) => break Ok(Some(try_s!(signed_tx_from_web3_tx(t)).into())),
                            None => break Ok(None),
                        }
                    },
                    None => {
                        if to_block >= current_block {
                            break Ok(None);
                        }
                        from_block = to_block;
                    },
                }
            }
        };
        Box::new(fut.boxed().compat())
    }

    async fn search_for_swap_tx_spend_my(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        let swap_contract_address = try_s!(input.swap_contract_address.try_to_address());
        self.search_for_swap_tx_spend(
            input.tx,
            swap_contract_address,
            input.secret_hash,
            input.search_from_block,
            input.watcher_reward,
        )
        .await
    }

    async fn search_for_swap_tx_spend_other(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        let swap_contract_address = try_s!(input.swap_contract_address.try_to_address());
        self.search_for_swap_tx_spend(
            input.tx,
            swap_contract_address,
            input.secret_hash,
            input.search_from_block,
            input.watcher_reward,
        )
        .await
    }

    fn check_tx_signed_by_pub(&self, _tx: &[u8], _expected_pub: &[u8]) -> Result<bool, MmError<ValidatePaymentError>> {
        unimplemented!();
    }

    async fn extract_secret(
        &self,
        _secret_hash: &[u8],
        spend_tx: &[u8],
        watcher_reward: bool,
    ) -> Result<Vec<u8>, String> {
        let unverified: UnverifiedTransaction = try_s!(rlp::decode(spend_tx));
        let function_name = get_function_name("receiverSpend", watcher_reward);
        let function = try_s!(SWAP_CONTRACT.function(&function_name));

        // Validate contract call; expected to be receiverSpend.
        // https://www.4byte.directory/signatures/?bytes4_signature=02ed292b.
        let expected_signature = function.short_signature();
        let actual_signature = &unverified.data[0..4];
        if actual_signature != expected_signature {
            return ERR!(
                "Expected 'receiverSpend' contract call signature: {:?}, found {:?}",
                expected_signature,
                actual_signature
            );
        };

        let tokens = try_s!(decode_contract_call(function, &unverified.data));
        if tokens.len() < 3 {
            return ERR!("Invalid arguments in 'receiverSpend' call: {:?}", tokens);
        }
        match &tokens[2] {
            Token::FixedBytes(secret) => Ok(secret.to_vec()),
            _ => ERR!(
                "Expected secret to be fixed bytes, decoded function data is {:?}",
                tokens
            ),
        }
    }

    fn is_auto_refundable(&self) -> bool { false }

    async fn wait_for_htlc_refund(&self, _tx: &[u8], _locktime: u64) -> RefundResult<()> {
        MmError::err(RefundError::Internal(
            "wait_for_htlc_refund is not supported for this coin!".into(),
        ))
    }

    fn negotiate_swap_contract_addr(
        &self,
        other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        match other_side_address {
            Some(bytes) => {
                if bytes.len() != 20 {
                    return MmError::err(NegotiateSwapContractAddrErr::InvalidOtherAddrLen(bytes.into()));
                }
                let other_addr = Address::from_slice(bytes);

                if other_addr == self.swap_contract_address {
                    return Ok(Some(self.swap_contract_address.0.to_vec().into()));
                }

                if Some(other_addr) == self.fallback_swap_contract {
                    return Ok(self.fallback_swap_contract.map(|addr| addr.0.to_vec().into()));
                }
                MmError::err(NegotiateSwapContractAddrErr::UnexpectedOtherAddr(bytes.into()))
            },
            None => self
                .fallback_swap_contract
                .map(|addr| Some(addr.0.to_vec().into()))
                .ok_or_else(|| MmError::new(NegotiateSwapContractAddrErr::NoOtherAddrAndNoFallback)),
        }
    }

    #[inline]
    fn derive_htlc_key_pair(&self, _swap_unique_data: &[u8]) -> keys::KeyPair {
        match self.priv_key_policy {
            EthPrivKeyPolicy::KeyPair(ref key_pair) => {
                key_pair_from_secret(key_pair.secret().as_bytes()).expect("valid key")
            },
            #[cfg(target_arch = "wasm32")]
            EthPrivKeyPolicy::Metamask(_) => todo!(),
        }
    }

    #[inline]
    fn derive_htlc_pubkey(&self, _swap_unique_data: &[u8]) -> Vec<u8> {
        match self.priv_key_policy {
            EthPrivKeyPolicy::KeyPair(ref key_pair) => key_pair_from_secret(key_pair.secret().as_bytes())
                .expect("valid key")
                .public_slice()
                .to_vec(),
            #[cfg(target_arch = "wasm32")]
            EthPrivKeyPolicy::Metamask(ref metamask_policy) => metamask_policy.public_key.as_bytes().to_vec(),
        }
    }

    fn validate_other_pubkey(&self, raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> {
        if let Err(e) = PublicKey::from_slice(raw_pubkey) {
            return MmError::err(ValidateOtherPubKeyErr::InvalidPubKey(e.to_string()));
        };
        Ok(())
    }

    async fn maker_payment_instructions(
        &self,
        args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        let watcher_reward = if args.watcher_reward {
            Some(
                self.get_watcher_reward_amount(args.wait_until)
                    .await
                    .map_err(|err| PaymentInstructionsErr::WatcherRewardErr(err.get_inner().to_string()))?
                    .to_string()
                    .into_bytes(),
            )
        } else {
            None
        };
        Ok(watcher_reward)
    }

    async fn taker_payment_instructions(
        &self,
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        Ok(None)
    }

    fn validate_maker_payment_instructions(
        &self,
        instructions: &[u8],
        _args: PaymentInstructionArgs,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        let watcher_reward = BigDecimal::from_str(
            &String::from_utf8(instructions.to_vec())
                .map_err(|err| ValidateInstructionsErr::DeserializationErr(err.to_string()))?,
        )
        .map_err(|err| ValidateInstructionsErr::DeserializationErr(err.to_string()))?;

        // TODO: Reward can be validated here
        Ok(PaymentInstructions::WatcherReward(watcher_reward))
    }

    fn validate_taker_payment_instructions(
        &self,
        _instructions: &[u8],
        _args: PaymentInstructionArgs,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        MmError::err(ValidateInstructionsErr::UnsupportedCoin(self.ticker().to_string()))
    }

    fn is_supported_by_watchers(&self) -> bool {
        false
        //self.contract_supports_watchers
    }
}

#[async_trait]
impl TakerSwapMakerCoin for EthCoin {
    async fn on_taker_payment_refund_start(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_taker_payment_refund_success(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
impl MakerSwapTakerCoin for EthCoin {
    async fn on_maker_payment_refund_start(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_maker_payment_refund_success(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
impl WatcherOps for EthCoin {
    fn send_maker_payment_spend_preimage(&self, input: SendMakerPaymentSpendPreimageInput) -> TransactionFut {
        Box::new(
            self.watcher_spends_hash_time_locked_payment(input)
                .map(TransactionEnum::from),
        )
    }

    fn create_maker_payment_spend_preimage(
        &self,
        maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx: UnverifiedTransaction = try_tx_fus!(rlp::decode(maker_payment_tx));
        let signed = try_tx_fus!(SignedEthTx::new(tx));
        let fut = async move { Ok(TransactionEnum::from(signed)) };

        Box::new(fut.boxed().compat())
    }

    fn create_taker_payment_refund_preimage(
        &self,
        taker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx: UnverifiedTransaction = try_tx_fus!(rlp::decode(taker_payment_tx));
        let signed = try_tx_fus!(SignedEthTx::new(tx));
        let fut = async move { Ok(TransactionEnum::from(signed)) };

        Box::new(fut.boxed().compat())
    }

    fn send_taker_payment_refund_preimage(&self, args: RefundPaymentArgs) -> TransactionFut {
        Box::new(
            self.watcher_refunds_hash_time_locked_payment(args)
                .map(TransactionEnum::from),
        )
    }

    fn watcher_validate_taker_fee(&self, validate_fee_args: WatcherValidateTakerFeeInput) -> ValidatePaymentFut<()> {
        validate_fee_impl(self.clone(), EthValidateFeeArgs {
            fee_tx_hash: &H256::from_slice(validate_fee_args.taker_fee_hash.as_slice()),
            expected_sender: &validate_fee_args.sender_pubkey,
            fee_addr: &validate_fee_args.fee_addr,
            amount: &BigDecimal::from(0),
            min_block_number: validate_fee_args.min_block_number,
            uuid: &[],
        })

        // TODO: Add validations specific for watchers
        // 1.Validate if taker fee is old
    }

    fn watcher_validate_taker_payment(&self, input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
        let unsigned: UnverifiedTransaction = try_f!(rlp::decode(&input.payment_tx));
        let tx =
            try_f!(SignedEthTx::new(unsigned)
                .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string())));
        let sender = try_f!(addr_from_raw_pubkey(&input.taker_pub).map_to_mm(ValidatePaymentError::InvalidParameter));
        let receiver = try_f!(addr_from_raw_pubkey(&input.maker_pub).map_to_mm(ValidatePaymentError::InvalidParameter));

        let selfi = self.clone();
        let swap_id = selfi.etomic_swap_id(input.time_lock, &input.secret_hash);
        let secret_hash = if input.secret_hash.len() == 32 {
            ripemd160(&input.secret_hash).to_vec()
        } else {
            input.secret_hash.to_vec()
        };
        let expected_swap_contract_address = self.swap_contract_address;
        let fallback_swap_contract = self.fallback_swap_contract;
        let decimals = self.decimals;

        let fut = async move {
            let tx_from_rpc = selfi.web3.eth().transaction(TransactionId::Hash(tx.hash)).await?;

            let tx_from_rpc = tx_from_rpc.as_ref().ok_or_else(|| {
                ValidatePaymentError::TxDoesNotExist(format!("Didn't find provided tx {:?} on ETH node", tx))
            })?;

            if tx_from_rpc.from != Some(sender) {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "{INVALID_SENDER_ERR_LOG}: Payment tx {tx_from_rpc:?} was sent from wrong address, expected {sender:?}"
                )));
            }

            let swap_contract_address = tx_from_rpc.to.ok_or_else(|| {
                ValidatePaymentError::TxDeserializationError(format!(
                    "Swap contract address not found in payment Tx {tx_from_rpc:?}"
                ))
            })?;

            if swap_contract_address != expected_swap_contract_address
                && Some(swap_contract_address) != fallback_swap_contract
            {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "{INVALID_CONTRACT_ADDRESS_ERR_LOG}: Payment tx {tx_from_rpc:?} was sent to wrong address, expected either {expected_swap_contract_address:?} or the fallback {fallback_swap_contract:?}"
                )));
            }

            let status = selfi
                .payment_status(swap_contract_address, Token::FixedBytes(swap_id.clone()))
                .compat()
                .await
                .map_to_mm(ValidatePaymentError::Transport)?;
            if status != U256::from(PaymentState::Sent as u8) && status != U256::from(PaymentState::Spent as u8) {
                return MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
                    "{INVALID_PAYMENT_STATE_ERR_LOG}: Payment state is not PAYMENT_STATE_SENT or PAYMENT_STATE_SPENT, got {status}"
                )));
            }

            let watcher_reward = selfi
                .get_taker_watcher_reward(&input.maker_coin, None, None, None, input.wait_until)
                .await
                .map_err(|err| ValidatePaymentError::WatcherRewardError(err.into_inner().to_string()))?;
            let expected_reward_amount = wei_from_big_decimal(&watcher_reward.amount, decimals)?;

            match &selfi.coin_type {
                EthCoinType::Eth => {
                    let function_name = get_function_name("ethPayment", true);
                    let function = SWAP_CONTRACT
                        .function(&function_name)
                        .map_to_mm(|err| ValidatePaymentError::InternalError(err.to_string()))?;
                    let decoded = decode_contract_call(function, &tx_from_rpc.input.0)
                        .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string()))?;

                    let swap_id_input = get_function_input_data(&decoded, function, 0)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
                    if swap_id_input != Token::FixedBytes(swap_id.clone()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "{INVALID_SWAP_ID_ERR_LOG}: Invalid 'swap_id' {decoded:?}, expected {swap_id:?}"
                        )));
                    }

                    let receiver_input = get_function_input_data(&decoded, function, 1)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
                    if receiver_input != Token::Address(receiver) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "{INVALID_RECEIVER_ERR_LOG}: Payment tx receiver arg {receiver_input:?} is invalid, expected {:?}", Token::Address(receiver)
                        )));
                    }

                    let secret_hash_input = get_function_input_data(&decoded, function, 2)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
                    if secret_hash_input != Token::FixedBytes(secret_hash.to_vec()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx secret_hash arg {:?} is invalid, expected {:?}",
                            secret_hash_input,
                            Token::FixedBytes(secret_hash.to_vec()),
                        )));
                    }

                    let time_lock_input = get_function_input_data(&decoded, function, 3)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
                    if time_lock_input != Token::Uint(U256::from(input.time_lock)) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx time_lock arg {:?} is invalid, expected {:?}",
                            time_lock_input,
                            Token::Uint(U256::from(input.time_lock)),
                        )));
                    }

                    let reward_target_input = get_function_input_data(&decoded, function, 4)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
                    let expected_reward_target = watcher_reward.reward_target as u8;
                    if reward_target_input != Token::Uint(U256::from(expected_reward_target)) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx reward target arg {:?} is invalid, expected {:?}",
                            reward_target_input, expected_reward_target
                        )));
                    }

                    let sends_contract_reward_input = get_function_input_data(&decoded, function, 5)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
                    if sends_contract_reward_input != Token::Bool(watcher_reward.send_contract_reward_on_spend) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx sends_contract_reward_on_spend arg {:?} is invalid, expected {:?}",
                            sends_contract_reward_input, watcher_reward.send_contract_reward_on_spend
                        )));
                    }

                    let reward_amount_input = get_function_input_data(&decoded, function, 6)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?
                        .into_uint()
                        .ok_or_else(|| {
                            ValidatePaymentError::WrongPaymentTx("Invalid type for reward amount argument".to_string())
                        })?;

                    validate_watcher_reward(expected_reward_amount.as_u64(), reward_amount_input.as_u64(), false)?;

                    // TODO: Validate the value
                },
                EthCoinType::Erc20 {
                    platform: _,
                    token_addr,
                } => {
                    let function_name = get_function_name("erc20Payment", true);
                    let function = SWAP_CONTRACT
                        .function(&function_name)
                        .map_to_mm(|err| ValidatePaymentError::InternalError(err.to_string()))?;
                    let decoded = decode_contract_call(function, &tx_from_rpc.input.0)
                        .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string()))?;

                    let swap_id_input = get_function_input_data(&decoded, function, 0)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
                    if swap_id_input != Token::FixedBytes(swap_id.clone()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "{INVALID_SWAP_ID_ERR_LOG}: Invalid 'swap_id' {decoded:?}, expected {swap_id:?}"
                        )));
                    }

                    let token_addr_input = get_function_input_data(&decoded, function, 2)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
                    if token_addr_input != Token::Address(*token_addr) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx token_addr arg {:?} is invalid, expected {:?}",
                            token_addr_input,
                            Token::Address(*token_addr)
                        )));
                    }

                    let receiver_addr_input = get_function_input_data(&decoded, function, 3)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
                    if receiver_addr_input != Token::Address(receiver) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "{INVALID_RECEIVER_ERR_LOG}: Payment tx receiver arg {receiver_addr_input:?} is invalid, expected {:?}", Token::Address(receiver),
                        )));
                    }

                    let secret_hash_input = get_function_input_data(&decoded, function, 4)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
                    if secret_hash_input != Token::FixedBytes(secret_hash.to_vec()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx secret_hash arg {:?} is invalid, expected {:?}",
                            secret_hash_input,
                            Token::FixedBytes(secret_hash.to_vec()),
                        )));
                    }

                    let time_lock_input = get_function_input_data(&decoded, function, 5)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
                    if time_lock_input != Token::Uint(U256::from(input.time_lock)) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx time_lock arg {:?} is invalid, expected {:?}",
                            time_lock_input,
                            Token::Uint(U256::from(input.time_lock)),
                        )));
                    }

                    let reward_target_input = get_function_input_data(&decoded, function, 6)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
                    let expected_reward_target = watcher_reward.reward_target as u8;
                    if reward_target_input != Token::Uint(U256::from(expected_reward_target)) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx reward target arg {:?} is invalid, expected {:?}",
                            reward_target_input, expected_reward_target
                        )));
                    }

                    let sends_contract_reward_input = get_function_input_data(&decoded, function, 7)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?;
                    if sends_contract_reward_input != Token::Bool(watcher_reward.send_contract_reward_on_spend) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx sends_contract_reward_on_spend arg {:?} is invalid, expected {:?}",
                            sends_contract_reward_input, watcher_reward.send_contract_reward_on_spend
                        )));
                    }

                    let reward_amount_input = get_function_input_data(&decoded, function, 8)
                        .map_to_mm(ValidatePaymentError::TxDeserializationError)?
                        .into_uint()
                        .ok_or_else(|| {
                            ValidatePaymentError::WrongPaymentTx("Invalid type for reward amount argument".to_string())
                        })?;

                    validate_watcher_reward(expected_reward_amount.as_u64(), reward_amount_input.as_u64(), false)?;

                    if tx_from_rpc.value != reward_amount_input {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx value arg {:?} is invalid, expected {:?}",
                            tx_from_rpc.value, reward_amount_input
                        )));
                    }
                },
            }

            Ok(())
        };
        Box::new(fut.boxed().compat())
    }

    async fn watcher_search_for_swap_tx_spend(
        &self,
        input: WatcherSearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        let unverified: UnverifiedTransaction = try_s!(rlp::decode(input.tx));
        let tx = try_s!(SignedEthTx::new(unverified));
        let swap_contract_address = match tx.action {
            Call(address) => address,
            Create => return Err(ERRL!("Invalid payment action: the payment action cannot be create")),
        };

        self.search_for_swap_tx_spend(
            input.tx,
            swap_contract_address,
            input.secret_hash,
            input.search_from_block,
            true,
        )
        .await
    }

    async fn get_taker_watcher_reward(
        &self,
        other_coin: &MmCoinEnum,
        _coin_amount: Option<BigDecimal>,
        _other_coin_amount: Option<BigDecimal>,
        reward_amount: Option<BigDecimal>,
        wait_until: u64,
    ) -> Result<WatcherReward, MmError<WatcherRewardError>> {
        let reward_target = if other_coin.is_eth() {
            RewardTarget::Contract
        } else {
            RewardTarget::PaymentSender
        };

        let is_exact_amount = reward_amount.is_some();
        let amount = match reward_amount {
            Some(amount) => amount,
            None => self.get_watcher_reward_amount(wait_until).await?,
        };

        let send_contract_reward_on_spend = false;

        Ok(WatcherReward {
            amount,
            is_exact_amount,
            reward_target,
            send_contract_reward_on_spend,
        })
    }

    async fn get_maker_watcher_reward(
        &self,
        other_coin: &MmCoinEnum,
        reward_amount: Option<BigDecimal>,
        wait_until: u64,
    ) -> Result<Option<WatcherReward>, MmError<WatcherRewardError>> {
        let reward_target = if other_coin.is_eth() {
            RewardTarget::None
        } else {
            RewardTarget::PaymentSpender
        };

        let is_exact_amount = reward_amount.is_some();
        let amount = match reward_amount {
            Some(amount) => amount,
            None => {
                let gas_cost_eth = self.get_watcher_reward_amount(wait_until).await?;

                match &self.coin_type {
                    EthCoinType::Eth => gas_cost_eth,
                    EthCoinType::Erc20 { .. } => {
                        if other_coin.is_eth() {
                            gas_cost_eth
                        } else {
                            get_base_price_in_rel(Some(self.ticker().to_string()), Some("ETH".to_string()))
                                .await
                                .and_then(|price_in_eth| gas_cost_eth.checked_div(price_in_eth))
                                .ok_or_else(|| {
                                    WatcherRewardError::RPCError(format!(
                                        "Price of coin {} in ETH could not be found",
                                        self.ticker()
                                    ))
                                })?
                        }
                    },
                }
            },
        };

        let send_contract_reward_on_spend = true;

        Ok(Some(WatcherReward {
            amount,
            is_exact_amount,
            reward_target,
            send_contract_reward_on_spend,
        }))
    }
}

#[cfg_attr(test, mockable)]
impl MarketCoinOps for EthCoin {
    fn ticker(&self) -> &str { &self.ticker[..] }

    fn my_address(&self) -> MmResult<String, MyAddressError> {
        Ok(checksum_address(&format!("{:#02x}", self.my_address)))
    }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> {
        match self.priv_key_policy {
            EthPrivKeyPolicy::KeyPair(ref key_pair) => {
                let uncompressed_without_prefix = hex::encode(key_pair.public());
                Ok(format!("04{}", uncompressed_without_prefix))
            },
            #[cfg(target_arch = "wasm32")]
            EthPrivKeyPolicy::Metamask(ref metamask_policy) => {
                Ok(format!("{:02x}", metamask_policy.public_key_uncompressed))
            },
        }
    }

    /// Hash message for signature using Ethereum's message signing format.
    /// keccak256(PREFIX_LENGTH + PREFIX + MESSAGE_LENGTH + MESSAGE)
    fn sign_message_hash(&self, message: &str) -> Option<[u8; 32]> {
        let message_prefix = self.sign_message_prefix.as_ref()?;

        let mut stream = Stream::new();
        let prefix_len = CompactInteger::from(message_prefix.len());
        prefix_len.serialize(&mut stream);
        stream.append_slice(message_prefix.as_bytes());
        stream.append_slice(message.len().to_string().as_bytes());
        stream.append_slice(message.as_bytes());
        Some(keccak256(&stream.out()).take())
    }

    fn sign_message(&self, message: &str) -> SignatureResult<String> {
        let message_hash = self.sign_message_hash(message).ok_or(SignatureError::PrefixNotFound)?;
        let privkey = &self.priv_key_policy.key_pair_or_err()?.secret();
        let signature = sign(privkey, &H256::from(message_hash))?;
        Ok(format!("0x{}", signature))
    }

    fn verify_message(&self, signature: &str, message: &str, address: &str) -> VerificationResult<bool> {
        let message_hash = self
            .sign_message_hash(message)
            .ok_or(VerificationError::PrefixNotFound)?;
        let address = self
            .address_from_str(address)
            .map_err(VerificationError::AddressDecodingError)?;
        let signature = Signature::from_str(signature.strip_prefix("0x").unwrap_or(signature))?;
        let is_verified = verify_address(&address, &signature, &H256::from(message_hash))?;
        Ok(is_verified)
    }

    fn my_balance(&self) -> BalanceFut<CoinBalance> {
        let decimals = self.decimals;
        let fut = self
            .my_balance()
            .and_then(move |result| Ok(u256_to_big_decimal(result, decimals)?))
            .map(|spendable| CoinBalance {
                spendable,
                unspendable: BigDecimal::from(0),
            });
        Box::new(fut)
    }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> {
        Box::new(
            self.eth_balance()
                .and_then(move |result| Ok(u256_to_big_decimal(result, ETH_DECIMALS)?)),
        )
    }

    fn platform_ticker(&self) -> &str {
        match &self.coin_type {
            EthCoinType::Eth => self.ticker(),
            EthCoinType::Erc20 { platform, .. } => platform,
        }
    }

    fn send_raw_tx(&self, mut tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
        if tx.starts_with("0x") {
            tx = &tx[2..];
        }
        let bytes = try_fus!(hex::decode(tx));
        Box::new(
            self.web3
                .eth()
                .send_raw_transaction(bytes.into())
                .compat()
                .map(|res| format!("{:02x}", res))
                .map_err(|e| ERRL!("{}", e)),
        )
    }

    fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> {
        Box::new(
            self.web3
                .eth()
                .send_raw_transaction(tx.into())
                .compat()
                .map(|res| format!("{:02x}", res))
                .map_err(|e| ERRL!("{}", e)),
        )
    }

    fn wait_for_confirmations(&self, input: ConfirmPaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        macro_rules! update_status_with_error {
            ($status: ident, $error: ident) => {
                match $error.get_inner() {
                    Web3RpcError::Timeout(_) => $status.append(" Timed out."),
                    _ => $status.append(" Failed."),
                }
            };
        }

        let ctx = try_fus!(MmArc::from_weak(&self.ctx).ok_or("No context"));
        let mut status = ctx.log.status_handle();
        status.status(&[&self.ticker], "Waiting for confirmations…");
        status.deadline(input.wait_until * 1000);

        let unsigned: UnverifiedTransaction = try_fus!(rlp::decode(&input.payment_tx));
        let tx = try_fus!(SignedEthTx::new(unsigned));
        let tx_hash = tx.hash();

        let required_confirms = U64::from(input.confirmations);
        let check_every = input.check_every as f64;
        let selfi = self.clone();
        let fut = async move {
            loop {
                // Wait for one confirmation and return the transaction confirmation block number
                let confirmed_at = match selfi
                    .transaction_confirmed_at(tx_hash, input.wait_until, check_every)
                    .compat()
                    .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        update_status_with_error!(status, e);
                        return Err(e.to_string());
                    },
                };

                // checking that confirmed_at is greater than zero to prevent overflow.
                // untrusted RPC nodes might send a zero value to cause overflow if we didn't do this check.
                // required_confirms should always be more than 0 anyways but we should keep this check nonetheless.
                if confirmed_at <= U64::from(0) {
                    error!(
                        "confirmed_at: {}, for payment tx: {:02x}, for coin:{} should be greater than zero!",
                        confirmed_at,
                        tx_hash,
                        selfi.ticker()
                    );
                    Timer::sleep(check_every).await;
                    continue;
                }

                // Wait for a block that achieves the required confirmations
                let confirmation_block_number = confirmed_at + required_confirms - 1;
                if let Err(e) = selfi
                    .wait_for_block(confirmation_block_number, input.wait_until, check_every)
                    .compat()
                    .await
                {
                    update_status_with_error!(status, e);
                    return Err(e.to_string());
                }

                // Make sure that there was no chain reorganization that led to transaction confirmation block to be changed
                match selfi
                    .transaction_confirmed_at(tx_hash, input.wait_until, check_every)
                    .compat()
                    .await
                {
                    Ok(conf) => {
                        if conf == confirmed_at {
                            status.append(" Confirmed.");
                            break Ok(());
                        }
                    },
                    Err(e) => {
                        update_status_with_error!(status, e);
                        return Err(e.to_string());
                    },
                }

                Timer::sleep(check_every).await;
            }
        };

        Box::new(fut.boxed().compat())
    }

    fn wait_for_htlc_tx_spend(&self, args: WaitForHTLCTxSpendArgs<'_>) -> TransactionFut {
        let unverified: UnverifiedTransaction = try_tx_fus!(rlp::decode(args.tx_bytes));
        let tx = try_tx_fus!(SignedEthTx::new(unverified));

        let swap_contract_address = match args.swap_contract_address {
            Some(addr) => try_tx_fus!(addr.try_to_address()),
            None => match tx.action {
                Call(address) => address,
                Create => {
                    return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                        "Invalid payment action: the payment action cannot be create"
                    ))))
                },
            },
        };

        let func_name = match self.coin_type {
            EthCoinType::Eth => get_function_name("ethPayment", args.watcher_reward),
            EthCoinType::Erc20 { .. } => get_function_name("erc20Payment", args.watcher_reward),
        };

        let payment_func = try_tx_fus!(SWAP_CONTRACT.function(&func_name));
        let decoded = try_tx_fus!(decode_contract_call(payment_func, &tx.data));
        let id = match decoded.first() {
            Some(Token::FixedBytes(bytes)) => bytes.clone(),
            invalid_token => {
                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                    "Expected Token::FixedBytes, got {:?}",
                    invalid_token
                ))))
            },
        };
        let selfi = self.clone();
        let from_block = args.from_block;
        let wait_until = args.wait_until;
        let check_every = args.check_every;
        let fut = async move {
            loop {
                if now_sec() > wait_until {
                    return TX_PLAIN_ERR!(
                        "Waited too long until {} for transaction {:?} to be spent ",
                        wait_until,
                        tx,
                    );
                }

                let current_block = match selfi.current_block().compat().await {
                    Ok(b) => b,
                    Err(e) => {
                        error!("Error getting block number: {}", e);
                        Timer::sleep(5.).await;
                        continue;
                    },
                };

                let events = match selfi
                    .spend_events(swap_contract_address, from_block, current_block)
                    .compat()
                    .await
                {
                    Ok(ev) => ev,
                    Err(e) => {
                        error!("Error getting spend events: {}", e);
                        Timer::sleep(5.).await;
                        continue;
                    },
                };

                let found = events.iter().find(|event| &event.data.0[..32] == id.as_slice());

                if let Some(event) = found {
                    if let Some(tx_hash) = event.transaction_hash {
                        let transaction = match selfi.web3.eth().transaction(TransactionId::Hash(tx_hash)).await {
                            Ok(Some(t)) => t,
                            Ok(None) => {
                                info!("Tx {} not found yet", tx_hash);
                                Timer::sleep(check_every).await;
                                continue;
                            },
                            Err(e) => {
                                error!("Get tx {} error: {}", tx_hash, e);
                                Timer::sleep(check_every).await;
                                continue;
                            },
                        };

                        return Ok(TransactionEnum::from(try_tx_s!(signed_tx_from_web3_tx(transaction))));
                    }
                }

                Timer::sleep(5.).await;
            }
        };
        Box::new(fut.boxed().compat())
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        signed_eth_tx_from_bytes(bytes)
            .map(TransactionEnum::from)
            .map_to_mm(TxMarshalingErr::InvalidInput)
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        Box::new(
            self.web3
                .eth()
                .block_number()
                .compat()
                .map(|res| res.as_u64())
                .map_err(|e| ERRL!("{}", e)),
        )
    }

    fn display_priv_key(&self) -> Result<String, String> {
        match self.priv_key_policy {
            EthPrivKeyPolicy::KeyPair(ref key_pair) => Ok(format!("{:#02x}", key_pair.secret())),
            #[cfg(target_arch = "wasm32")]
            EthPrivKeyPolicy::Metamask(_) => ERR!("'display_priv_key' doesn't support MetaMask"),
        }
    }

    fn min_tx_amount(&self) -> BigDecimal { BigDecimal::from(0) }

    fn min_trading_vol(&self) -> MmNumber {
        let pow = self.decimals / 3;
        MmNumber::from(1) / MmNumber::from(10u64.pow(pow as u32))
    }
}

pub fn signed_eth_tx_from_bytes(bytes: &[u8]) -> Result<SignedEthTx, String> {
    let tx: UnverifiedTransaction = try_s!(rlp::decode(bytes));
    let signed = try_s!(SignedEthTx::new(tx));
    Ok(signed)
}

// We can use a nonce lock shared between tokens using the same platform coin and the platform itself.
// For example, ETH/USDT-ERC20 should use the same lock, but it will be different for BNB/USDT-BEP20.
lazy_static! {
    static ref NONCE_LOCK: Mutex<HashMap<String, Arc<AsyncMutex<()>>>> = Mutex::new(HashMap::new());
}

type EthTxFut = Box<dyn Future<Item = SignedEthTx, Error = TransactionErr> + Send + 'static>;

async fn sign_and_send_transaction_with_keypair(
    ctx: MmArc,
    coin: &EthCoin,
    key_pair: &KeyPair,
    value: U256,
    action: Action,
    data: Vec<u8>,
    gas: U256,
) -> Result<SignedEthTx, TransactionErr> {
    let mut status = ctx.log.status_handle();
    macro_rules! tags {
        () => {
            &[&"sign-and-send"]
        };
    }
    let _nonce_lock = coin.nonce_lock.lock().await;
    status.status(tags!(), "get_addr_nonce…");
    let (nonce, web3_instances_with_latest_nonce) = try_tx_s!(
        get_addr_nonce(coin.my_address, coin.web3_instances.clone())
            .compat()
            .await
    );
    status.status(tags!(), "get_gas_price…");
    let gas_price = try_tx_s!(coin.get_gas_price().compat().await);

    let tx = UnSignedEthTx {
        nonce,
        gas_price,
        gas,
        action,
        value,
        data,
    };

    let signed = tx.sign(key_pair.secret(), coin.chain_id);
    let bytes = Bytes(rlp::encode(&signed).to_vec());
    status.status(tags!(), "send_raw_transaction…");

    let futures = web3_instances_with_latest_nonce
        .into_iter()
        .map(|web3_instance| web3_instance.web3.eth().send_raw_transaction(bytes.clone()));
    try_tx_s!(select_ok(futures).await.map_err(|e| ERRL!("{}", e)), signed);

    status.status(tags!(), "get_addr_nonce…");
    coin.wait_for_addr_nonce_increase(coin.my_address, nonce).await;
    Ok(signed)
}

#[cfg(target_arch = "wasm32")]
async fn sign_and_send_transaction_with_metamask(
    coin: EthCoin,
    value: U256,
    action: Action,
    data: Vec<u8>,
    gas: U256,
) -> Result<SignedEthTx, TransactionErr> {
    let to = match action {
        Action::Create => None,
        Action::Call(to) => Some(to),
    };

    let gas_price = try_tx_s!(coin.get_gas_price().compat().await);

    let tx_to_send = TransactionRequest {
        from: coin.my_address,
        to,
        gas: Some(gas),
        gas_price: Some(gas_price),
        value: Some(value),
        data: Some(data.clone().into()),
        nonce: None,
        ..TransactionRequest::default()
    };

    // It's important to return the transaction hex for the swap,
    // so wait up to 60 seconds for the transaction to appear on the RPC node.
    let wait_rpc_timeout = 60_000;
    let check_every = 1.;

    // Please note that this method may take a long time
    // due to `wallet_switchEthereumChain` and `eth_sendTransaction` requests.
    let tx_hash = try_tx_s!(coin.web3.eth().send_transaction(tx_to_send).await);

    let maybe_signed_tx = try_tx_s!(
        coin.wait_for_tx_appears_on_rpc(tx_hash, wait_rpc_timeout, check_every)
            .await
    );
    match maybe_signed_tx {
        Some(signed_tx) => Ok(signed_tx),
        None => TX_PLAIN_ERR!(
            "Waited too long until the transaction {:?} appear on the RPC node",
            tx_hash
        ),
    }
}

impl EthCoin {
    /// Downloads and saves ETH transaction history of my_address, relies on Parity trace_filter API
    /// https://wiki.parity.io/JSONRPC-trace-module#trace_filter, this requires tracing to be enabled
    /// in node config. Other ETH clients (Geth, etc.) are `not` supported (yet).
    #[allow(clippy::cognitive_complexity)]
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    async fn process_eth_history(&self, ctx: &MmArc) {
        // Artem Pikulin: by playing a bit with Parity mainnet node I've discovered that trace_filter API responds after reasonable time for 1000 blocks.
        // I've tried to increase the amount to 10000, but request times out somewhere near 2500000 block.
        // Also the Parity RPC server seem to get stuck while request in running (other requests performance is also lowered).
        let delta = U64::from(1000);

        let mut success_iteration = 0i32;
        loop {
            if ctx.is_stopping() {
                break;
            };
            {
                let coins_ctx = CoinsContext::from_ctx(ctx).unwrap();
                let coins = coins_ctx.coins.lock().await;
                if !coins.contains_key(&self.ticker) {
                    ctx.log.log("", &[&"tx_history", &self.ticker], "Loop stopped");
                    break;
                };
            }

            let current_block = match self.web3.eth().block_number().await {
                Ok(block) => block,
                Err(e) => {
                    ctx.log.log(
                        "",
                        &[&"tx_history", &self.ticker],
                        &ERRL!("Error {} on eth_block_number, retrying", e),
                    );
                    Timer::sleep(10.).await;
                    continue;
                },
            };

            let mut saved_traces = match self.load_saved_traces(ctx) {
                Some(traces) => traces,
                None => SavedTraces {
                    traces: vec![],
                    earliest_block: current_block,
                    latest_block: current_block,
                },
            };
            *self.history_sync_state.lock().unwrap() = HistorySyncState::InProgress(json!({
                "blocks_left": saved_traces.earliest_block.as_u64(),
            }));

            let mut existing_history = match self.load_history_from_file(ctx).compat().await {
                Ok(history) => history,
                Err(e) => {
                    ctx.log.log(
                        "",
                        &[&"tx_history", &self.ticker],
                        &ERRL!("Error {} on 'load_history_from_file', stop the history loop", e),
                    );
                    return;
                },
            };

            // AP: AFAIK ETH RPC doesn't support conditional filters like `get this OR this` so we have
            // to run several queries to get trace events including our address as sender `or` receiver
            // TODO refactor this to batch requests instead of single request per query
            if saved_traces.earliest_block > 0.into() {
                let before_earliest = if saved_traces.earliest_block >= delta {
                    saved_traces.earliest_block - delta
                } else {
                    0.into()
                };

                let from_traces_before_earliest = match self
                    .eth_traces(
                        vec![self.my_address],
                        vec![],
                        BlockNumber::Number(before_earliest),
                        BlockNumber::Number(saved_traces.earliest_block),
                        None,
                    )
                    .compat()
                    .await
                {
                    Ok(traces) => traces,
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Error {} on eth_traces, retrying", e),
                        );
                        Timer::sleep(10.).await;
                        continue;
                    },
                };

                let to_traces_before_earliest = match self
                    .eth_traces(
                        vec![],
                        vec![self.my_address],
                        BlockNumber::Number(before_earliest),
                        BlockNumber::Number(saved_traces.earliest_block),
                        None,
                    )
                    .compat()
                    .await
                {
                    Ok(traces) => traces,
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Error {} on eth_traces, retrying", e),
                        );
                        Timer::sleep(10.).await;
                        continue;
                    },
                };

                let total_length = from_traces_before_earliest.len() + to_traces_before_earliest.len();
                mm_counter!(ctx.metrics, "tx.history.response.total_length", total_length as u64,
                    "coin" => self.ticker.clone(), "client" => "ethereum", "method" => "eth_traces");

                saved_traces.traces.extend(from_traces_before_earliest);
                saved_traces.traces.extend(to_traces_before_earliest);
                saved_traces.earliest_block = if before_earliest > 0.into() {
                    // need to exclude the before earliest block from next iteration
                    before_earliest - 1
                } else {
                    0.into()
                };
                self.store_eth_traces(ctx, &saved_traces);
            }

            if current_block > saved_traces.latest_block {
                let from_traces_after_latest = match self
                    .eth_traces(
                        vec![self.my_address],
                        vec![],
                        BlockNumber::Number(saved_traces.latest_block + 1),
                        BlockNumber::Number(current_block),
                        None,
                    )
                    .compat()
                    .await
                {
                    Ok(traces) => traces,
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Error {} on eth_traces, retrying", e),
                        );
                        Timer::sleep(10.).await;
                        continue;
                    },
                };

                let to_traces_after_latest = match self
                    .eth_traces(
                        vec![],
                        vec![self.my_address],
                        BlockNumber::Number(saved_traces.latest_block + 1),
                        BlockNumber::Number(current_block),
                        None,
                    )
                    .compat()
                    .await
                {
                    Ok(traces) => traces,
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Error {} on eth_traces, retrying", e),
                        );
                        Timer::sleep(10.).await;
                        continue;
                    },
                };

                let total_length = from_traces_after_latest.len() + to_traces_after_latest.len();
                mm_counter!(ctx.metrics, "tx.history.response.total_length", total_length as u64,
                    "coin" => self.ticker.clone(), "client" => "ethereum", "method" => "eth_traces");

                saved_traces.traces.extend(from_traces_after_latest);
                saved_traces.traces.extend(to_traces_after_latest);
                saved_traces.latest_block = current_block;

                self.store_eth_traces(ctx, &saved_traces);
            }
            saved_traces.traces.sort_by(|a, b| b.block_number.cmp(&a.block_number));
            for trace in saved_traces.traces {
                let hash = sha256(&json::to_vec(&trace).unwrap());
                let internal_id = BytesJson::from(hash.to_vec());
                let processed = existing_history.iter().find(|tx| tx.internal_id == internal_id);
                if processed.is_some() {
                    continue;
                }

                // TODO Only standard Call traces are supported, contract creations, suicides and block rewards will be supported later
                let call_data = match trace.action {
                    TraceAction::Call(d) => d,
                    _ => continue,
                };

                mm_counter!(ctx.metrics, "tx.history.request.count", 1, "coin" => self.ticker.clone(), "method" => "tx_detail_by_hash");

                let web3_tx = match self
                    .web3
                    .eth()
                    .transaction(TransactionId::Hash(trace.transaction_hash.unwrap()))
                    .await
                {
                    Ok(tx) => tx,
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!(
                                "Error {} on getting transaction {:?}",
                                e,
                                trace.transaction_hash.unwrap()
                            ),
                        );
                        continue;
                    },
                };
                let web3_tx = match web3_tx {
                    Some(t) => t,
                    None => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("No such transaction {:?}", trace.transaction_hash.unwrap()),
                        );
                        continue;
                    },
                };

                mm_counter!(ctx.metrics, "tx.history.response.count", 1, "coin" => self.ticker.clone(), "method" => "tx_detail_by_hash");

                let receipt = match self
                    .web3
                    .eth()
                    .transaction_receipt(trace.transaction_hash.unwrap())
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!(
                                "Error {} on getting transaction {:?} receipt",
                                e,
                                trace.transaction_hash.unwrap()
                            ),
                        );
                        continue;
                    },
                };
                let fee_coin = match &self.coin_type {
                    EthCoinType::Eth => self.ticker(),
                    EthCoinType::Erc20 { platform, .. } => platform.as_str(),
                };
                let fee_details: Option<EthTxFeeDetails> = match receipt {
                    Some(r) => {
                        let gas_used = r.gas_used.unwrap_or_default();
                        let gas_price = web3_tx.gas_price.unwrap_or_default();
                        // It's relatively safe to unwrap `EthTxFeeDetails::new` as it may fail
                        // due to `u256_to_big_decimal` only.
                        // Also TX history is not used by any GUI and has significant disadvantages.
                        Some(EthTxFeeDetails::new(gas_used, gas_price, fee_coin).unwrap())
                    },
                    None => None,
                };

                let total_amount: BigDecimal = u256_to_big_decimal(call_data.value, ETH_DECIMALS).unwrap();
                let mut received_by_me = 0.into();
                let mut spent_by_me = 0.into();

                if call_data.from == self.my_address {
                    // ETH transfer is actually happening only if no error occurred
                    if trace.error.is_none() {
                        spent_by_me = total_amount.clone();
                    }
                    if let Some(ref fee) = fee_details {
                        spent_by_me += &fee.total_fee;
                    }
                }

                if call_data.to == self.my_address {
                    // ETH transfer is actually happening only if no error occurred
                    if trace.error.is_none() {
                        received_by_me = total_amount.clone();
                    }
                }

                let raw = signed_tx_from_web3_tx(web3_tx).unwrap();
                let block = match self
                    .web3
                    .eth()
                    .block(BlockId::Number(BlockNumber::Number(trace.block_number.into())))
                    .await
                {
                    Ok(b) => b.unwrap(),
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Error {} on getting block {} data", e, trace.block_number),
                        );
                        continue;
                    },
                };

                let details = TransactionDetails {
                    my_balance_change: &received_by_me - &spent_by_me,
                    spent_by_me,
                    received_by_me,
                    total_amount,
                    to: vec![checksum_address(&format!("{:#02x}", call_data.to))],
                    from: vec![checksum_address(&format!("{:#02x}", call_data.from))],
                    coin: self.ticker.clone(),
                    fee_details: fee_details.map(|d| d.into()),
                    block_height: trace.block_number,
                    tx_hash: format!("{:02x}", BytesJson(raw.hash.as_bytes().to_vec())),
                    tx_hex: BytesJson(rlp::encode(&raw).to_vec()),
                    internal_id,
                    timestamp: block.timestamp.into_or_max(),
                    kmd_rewards: None,
                    transaction_type: Default::default(),
                    memo: None,
                };

                existing_history.push(details);

                if let Err(e) = self.save_history_to_file(ctx, existing_history.clone()).compat().await {
                    ctx.log.log(
                        "",
                        &[&"tx_history", &self.ticker],
                        &ERRL!("Error {} on 'save_history_to_file', stop the history loop", e),
                    );
                    return;
                }
            }
            if saved_traces.earliest_block == 0.into() {
                if success_iteration == 0 {
                    ctx.log.log(
                        "😅",
                        &[&"tx_history", &("coin", self.ticker.clone().as_str())],
                        "history has been loaded successfully",
                    );
                }

                success_iteration += 1;
                *self.history_sync_state.lock().unwrap() = HistorySyncState::Finished;
                Timer::sleep(15.).await;
            } else {
                Timer::sleep(2.).await;
            }
        }
    }

    /// Downloads and saves ERC20 transaction history of my_address
    #[allow(clippy::cognitive_complexity)]
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    async fn process_erc20_history(&self, token_addr: H160, ctx: &MmArc) {
        let delta = U64::from(10000);

        let mut success_iteration = 0i32;
        loop {
            if ctx.is_stopping() {
                break;
            };
            {
                let coins_ctx = CoinsContext::from_ctx(ctx).unwrap();
                let coins = coins_ctx.coins.lock().await;
                if !coins.contains_key(&self.ticker) {
                    ctx.log.log("", &[&"tx_history", &self.ticker], "Loop stopped");
                    break;
                };
            }

            let current_block = match self.web3.eth().block_number().await {
                Ok(block) => block,
                Err(e) => {
                    ctx.log.log(
                        "",
                        &[&"tx_history", &self.ticker],
                        &ERRL!("Error {} on eth_block_number, retrying", e),
                    );
                    Timer::sleep(10.).await;
                    continue;
                },
            };

            let mut saved_events = match self.load_saved_erc20_events(ctx) {
                Some(events) => events,
                None => SavedErc20Events {
                    events: vec![],
                    earliest_block: current_block,
                    latest_block: current_block,
                },
            };
            *self.history_sync_state.lock().unwrap() = HistorySyncState::InProgress(json!({
                "blocks_left": saved_events.earliest_block,
            }));

            // AP: AFAIK ETH RPC doesn't support conditional filters like `get this OR this` so we have
            // to run several queries to get transfer events including our address as sender `or` receiver
            // TODO refactor this to batch requests instead of single request per query
            if saved_events.earliest_block > 0.into() {
                let before_earliest = if saved_events.earliest_block >= delta {
                    saved_events.earliest_block - delta
                } else {
                    0.into()
                };

                let from_events_before_earliest = match self
                    .erc20_transfer_events(
                        token_addr,
                        Some(self.my_address),
                        None,
                        BlockNumber::Number(before_earliest),
                        BlockNumber::Number(saved_events.earliest_block - 1),
                        None,
                    )
                    .compat()
                    .await
                {
                    Ok(events) => events,
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Error {} on erc20_transfer_events, retrying", e),
                        );
                        Timer::sleep(10.).await;
                        continue;
                    },
                };

                let to_events_before_earliest = match self
                    .erc20_transfer_events(
                        token_addr,
                        None,
                        Some(self.my_address),
                        BlockNumber::Number(before_earliest),
                        BlockNumber::Number(saved_events.earliest_block - 1),
                        None,
                    )
                    .compat()
                    .await
                {
                    Ok(events) => events,
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Error {} on erc20_transfer_events, retrying", e),
                        );
                        Timer::sleep(10.).await;
                        continue;
                    },
                };

                let total_length = from_events_before_earliest.len() + to_events_before_earliest.len();
                mm_counter!(ctx.metrics, "tx.history.response.total_length", total_length as u64,
                    "coin" => self.ticker.clone(), "client" => "ethereum", "method" => "erc20_transfer_events");

                saved_events.events.extend(from_events_before_earliest);
                saved_events.events.extend(to_events_before_earliest);
                saved_events.earliest_block = if before_earliest > 0.into() {
                    before_earliest - 1
                } else {
                    0.into()
                };
                self.store_erc20_events(ctx, &saved_events);
            }

            if current_block > saved_events.latest_block {
                let from_events_after_latest = match self
                    .erc20_transfer_events(
                        token_addr,
                        Some(self.my_address),
                        None,
                        BlockNumber::Number(saved_events.latest_block + 1),
                        BlockNumber::Number(current_block),
                        None,
                    )
                    .compat()
                    .await
                {
                    Ok(events) => events,
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Error {} on erc20_transfer_events, retrying", e),
                        );
                        Timer::sleep(10.).await;
                        continue;
                    },
                };

                let to_events_after_latest = match self
                    .erc20_transfer_events(
                        token_addr,
                        None,
                        Some(self.my_address),
                        BlockNumber::Number(saved_events.latest_block + 1),
                        BlockNumber::Number(current_block),
                        None,
                    )
                    .compat()
                    .await
                {
                    Ok(events) => events,
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Error {} on erc20_transfer_events, retrying", e),
                        );
                        Timer::sleep(10.).await;
                        continue;
                    },
                };

                let total_length = from_events_after_latest.len() + to_events_after_latest.len();
                mm_counter!(ctx.metrics, "tx.history.response.total_length", total_length as u64,
                    "coin" => self.ticker.clone(), "client" => "ethereum", "method" => "erc20_transfer_events");

                saved_events.events.extend(from_events_after_latest);
                saved_events.events.extend(to_events_after_latest);
                saved_events.latest_block = current_block;
                self.store_erc20_events(ctx, &saved_events);
            }

            let all_events: HashMap<_, _> = saved_events
                .events
                .iter()
                .filter(|e| e.block_number.is_some() && e.transaction_hash.is_some() && !e.is_removed())
                .map(|e| (e.transaction_hash.unwrap(), e))
                .collect();
            let mut all_events: Vec<_> = all_events.into_values().collect();
            all_events.sort_by(|a, b| b.block_number.unwrap().cmp(&a.block_number.unwrap()));

            for event in all_events {
                let mut existing_history = match self.load_history_from_file(ctx).compat().await {
                    Ok(history) => history,
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Error {} on 'load_history_from_file', stop the history loop", e),
                        );
                        return;
                    },
                };
                let internal_id = BytesJson::from(sha256(&json::to_vec(&event).unwrap()).to_vec());
                if existing_history.iter().any(|item| item.internal_id == internal_id) {
                    // the transaction already imported
                    continue;
                };

                let amount = U256::from(event.data.0.as_slice());
                let total_amount = u256_to_big_decimal(amount, self.decimals).unwrap();
                let mut received_by_me = 0.into();
                let mut spent_by_me = 0.into();

                let from_addr = H160::from(event.topics[1]);
                let to_addr = H160::from(event.topics[2]);

                if from_addr == self.my_address {
                    spent_by_me = total_amount.clone();
                }

                if to_addr == self.my_address {
                    received_by_me = total_amount.clone();
                }

                mm_counter!(ctx.metrics, "tx.history.request.count", 1,
                    "coin" => self.ticker.clone(), "client" => "ethereum", "method" => "tx_detail_by_hash");

                let web3_tx = match self
                    .web3
                    .eth()
                    .transaction(TransactionId::Hash(event.transaction_hash.unwrap()))
                    .await
                {
                    Ok(tx) => tx,
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!(
                                "Error {} on getting transaction {:?}",
                                e,
                                event.transaction_hash.unwrap()
                            ),
                        );
                        continue;
                    },
                };

                mm_counter!(ctx.metrics, "tx.history.response.count", 1,
                    "coin" => self.ticker.clone(), "client" => "ethereum", "method" => "tx_detail_by_hash");

                let web3_tx = match web3_tx {
                    Some(t) => t,
                    None => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("No such transaction {:?}", event.transaction_hash.unwrap()),
                        );
                        continue;
                    },
                };

                let receipt = match self
                    .web3
                    .eth()
                    .transaction_receipt(event.transaction_hash.unwrap())
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!(
                                "Error {} on getting transaction {:?} receipt",
                                e,
                                event.transaction_hash.unwrap()
                            ),
                        );
                        continue;
                    },
                };
                let fee_coin = match &self.coin_type {
                    EthCoinType::Eth => self.ticker(),
                    EthCoinType::Erc20 { platform, .. } => platform.as_str(),
                };
                let fee_details = match receipt {
                    Some(r) => {
                        let gas_used = r.gas_used.unwrap_or_default();
                        let gas_price = web3_tx.gas_price.unwrap_or_default();
                        // It's relatively safe to unwrap `EthTxFeeDetails::new` as it may fail
                        // due to `u256_to_big_decimal` only.
                        // Also TX history is not used by any GUI and has significant disadvantages.
                        Some(EthTxFeeDetails::new(gas_used, gas_price, fee_coin).unwrap())
                    },
                    None => None,
                };
                let block_number = event.block_number.unwrap();
                let block = match self
                    .web3
                    .eth()
                    .block(BlockId::Number(BlockNumber::Number(block_number)))
                    .await
                {
                    Ok(Some(b)) => b,
                    Ok(None) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Block {} is None", block_number),
                        );
                        continue;
                    },
                    Err(e) => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Error {} on getting block {} data", e, block_number),
                        );
                        continue;
                    },
                };

                let raw = signed_tx_from_web3_tx(web3_tx).unwrap();
                let details = TransactionDetails {
                    my_balance_change: &received_by_me - &spent_by_me,
                    spent_by_me,
                    received_by_me,
                    total_amount,
                    to: vec![checksum_address(&format!("{:#02x}", to_addr))],
                    from: vec![checksum_address(&format!("{:#02x}", from_addr))],
                    coin: self.ticker.clone(),
                    fee_details: fee_details.map(|d| d.into()),
                    block_height: block_number.as_u64(),
                    tx_hash: format!("{:02x}", BytesJson(raw.hash.as_bytes().to_vec())),
                    tx_hex: BytesJson(rlp::encode(&raw).to_vec()),
                    internal_id: BytesJson(internal_id.to_vec()),
                    timestamp: block.timestamp.into_or_max(),
                    kmd_rewards: None,
                    transaction_type: Default::default(),
                    memo: None,
                };

                existing_history.push(details);

                if let Err(e) = self.save_history_to_file(ctx, existing_history).compat().await {
                    ctx.log.log(
                        "",
                        &[&"tx_history", &self.ticker],
                        &ERRL!("Error {} on 'save_history_to_file', stop the history loop", e),
                    );
                    return;
                }
            }
            if saved_events.earliest_block == 0.into() {
                if success_iteration == 0 {
                    ctx.log.log(
                        "😅",
                        &[&"tx_history", &("coin", self.ticker.clone().as_str())],
                        "history has been loaded successfully",
                    );
                }

                success_iteration += 1;
                *self.history_sync_state.lock().unwrap() = HistorySyncState::Finished;
                Timer::sleep(15.).await;
            } else {
                Timer::sleep(2.).await;
            }
        }
    }
}

#[cfg_attr(test, mockable)]
impl EthCoin {
    fn sign_and_send_transaction(&self, value: U256, action: Action, data: Vec<u8>, gas: U256) -> EthTxFut {
        let ctx = try_tx_fus!(MmArc::from_weak(&self.ctx).ok_or("!ctx"));
        let coin = self.clone();
        let fut = async move {
            match coin.priv_key_policy {
                EthPrivKeyPolicy::KeyPair(ref key_pair) => {
                    sign_and_send_transaction_with_keypair(ctx, &coin, key_pair, value, action, data, gas).await
                },
                #[cfg(target_arch = "wasm32")]
                EthPrivKeyPolicy::Metamask(_) => {
                    sign_and_send_transaction_with_metamask(coin, value, action, data, gas).await
                },
            }
        };
        Box::new(fut.boxed().compat())
    }

    pub fn send_to_address(&self, address: Address, value: U256) -> EthTxFut {
        match &self.coin_type {
            EthCoinType::Eth => self.sign_and_send_transaction(value, Action::Call(address), vec![], U256::from(21000)),
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let abi = try_tx_fus!(Contract::load(ERC20_ABI.as_bytes()));
                let function = try_tx_fus!(abi.function("transfer"));
                let data = try_tx_fus!(function.encode_input(&[Token::Address(address), Token::Uint(value)]));
                self.sign_and_send_transaction(0.into(), Action::Call(*token_addr), data, U256::from(210_000))
            },
        }
    }

    fn send_hash_time_locked_payment(&self, args: SendPaymentArgs<'_>) -> EthTxFut {
        let receiver_addr = try_tx_fus!(addr_from_raw_pubkey(args.other_pubkey));
        let swap_contract_address = try_tx_fus!(args.swap_contract_address.try_to_address());
        let id = self.etomic_swap_id(args.time_lock, args.secret_hash);
        let trade_amount = try_tx_fus!(wei_from_big_decimal(&args.amount, self.decimals));

        let time_lock = U256::from(args.time_lock);
        let gas = U256::from(ETH_GAS);

        let secret_hash = if args.secret_hash.len() == 32 {
            ripemd160(args.secret_hash).to_vec()
        } else {
            args.secret_hash.to_vec()
        };

        match &self.coin_type {
            EthCoinType::Eth => {
                let function_name = get_function_name("ethPayment", args.watcher_reward.is_some());
                let function = try_tx_fus!(SWAP_CONTRACT.function(&function_name));

                let mut value = trade_amount;
                let data = match &args.watcher_reward {
                    Some(reward) => {
                        let reward_amount = try_tx_fus!(wei_from_big_decimal(&reward.amount, self.decimals));
                        if !matches!(reward.reward_target, RewardTarget::None) {
                            value += reward_amount;
                        }

                        try_tx_fus!(function.encode_input(&[
                            Token::FixedBytes(id),
                            Token::Address(receiver_addr),
                            Token::FixedBytes(secret_hash),
                            Token::Uint(time_lock),
                            Token::Uint(U256::from(reward.reward_target as u8)),
                            Token::Bool(reward.send_contract_reward_on_spend),
                            Token::Uint(reward_amount)
                        ]))
                    },
                    None => try_tx_fus!(function.encode_input(&[
                        Token::FixedBytes(id),
                        Token::Address(receiver_addr),
                        Token::FixedBytes(secret_hash),
                        Token::Uint(time_lock),
                    ])),
                };

                self.sign_and_send_transaction(value, Action::Call(swap_contract_address), data, gas)
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let allowance_fut = self
                    .allowance(swap_contract_address)
                    .map_err(|e| TransactionErr::Plain(ERRL!("{}", e)));

                let function_name = get_function_name("erc20Payment", args.watcher_reward.is_some());
                let function = try_tx_fus!(SWAP_CONTRACT.function(&function_name));

                let mut value = U256::from(0);
                let mut amount = trade_amount;

                let data = match args.watcher_reward {
                    Some(reward) => {
                        let reward_amount = try_tx_fus!(wei_from_big_decimal(&reward.amount, self.decimals));

                        match reward.reward_target {
                            RewardTarget::Contract | RewardTarget::PaymentSender => value += reward_amount,
                            RewardTarget::PaymentSpender => amount += reward_amount,
                            _ => (),
                        };

                        try_tx_fus!(function.encode_input(&[
                            Token::FixedBytes(id),
                            Token::Uint(amount),
                            Token::Address(*token_addr),
                            Token::Address(receiver_addr),
                            Token::FixedBytes(secret_hash),
                            Token::Uint(time_lock),
                            Token::Uint(U256::from(reward.reward_target as u8)),
                            Token::Bool(reward.send_contract_reward_on_spend),
                            Token::Uint(reward_amount),
                        ]))
                    },
                    None => {
                        try_tx_fus!(function.encode_input(&[
                            Token::FixedBytes(id),
                            Token::Uint(trade_amount),
                            Token::Address(*token_addr),
                            Token::Address(receiver_addr),
                            Token::FixedBytes(secret_hash),
                            Token::Uint(time_lock)
                        ]))
                    },
                };

                let wait_for_required_allowance_until = args.wait_for_confirmation_until;

                let arc = self.clone();
                Box::new(allowance_fut.and_then(move |allowed| -> EthTxFut {
                    if allowed < amount {
                        Box::new(
                            arc.approve(swap_contract_address, U256::max_value())
                                .and_then(move |approved| {
                                    // make sure the approve tx is confirmed by making sure that the allowed value has been updated
                                    // this call is cheaper than waiting for confirmation calls
                                    arc.wait_for_required_allowance(
                                        swap_contract_address,
                                        amount,
                                        wait_for_required_allowance_until,
                                    )
                                    .map_err(move |e| {
                                        TransactionErr::Plain(ERRL!(
                                            "Allowed value was not updated in time after sending approve transaction {:02x}: {}",
                                            approved.tx_hash(),
                                            e
                                        ))
                                    })
                                    .and_then(move |_| {
                                        arc.sign_and_send_transaction(
                                            value,
                                            Action::Call(swap_contract_address),
                                            data,
                                            gas,
                                        )
                                    })
                                }),
                        )
                    } else {
                        Box::new(arc.sign_and_send_transaction(
                            value,
                            Action::Call(swap_contract_address),
                            data,
                            gas,
                        ))
                    }
                }))
            },
        }
    }

    fn watcher_spends_hash_time_locked_payment(&self, input: SendMakerPaymentSpendPreimageInput) -> EthTxFut {
        let tx: UnverifiedTransaction = try_tx_fus!(rlp::decode(input.preimage));
        let payment = try_tx_fus!(SignedEthTx::new(tx));

        let function_name = get_function_name("receiverSpend", input.watcher_reward);
        let spend_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));
        let clone = self.clone();
        let secret_vec = input.secret.to_vec();
        let taker_addr = addr_from_raw_pubkey(input.taker_pub).unwrap();
        let swap_contract_address = match payment.action {
            Call(address) => address,
            Create => {
                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                    "Invalid payment action: the payment action cannot be create"
                ))))
            },
        };

        let watcher_reward = input.watcher_reward;
        match self.coin_type {
            EthCoinType::Eth => {
                let function_name = get_function_name("ethPayment", watcher_reward);
                let payment_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));
                let decoded = try_tx_fus!(decode_contract_call(payment_func, &payment.data));
                let swap_id_input = try_tx_fus!(get_function_input_data(&decoded, payment_func, 0));

                let state_f = self.payment_status(swap_contract_address, swap_id_input.clone());
                Box::new(
                    state_f
                        .map_err(TransactionErr::Plain)
                        .and_then(move |state| -> EthTxFut {
                            if state != U256::from(PaymentState::Sent as u8) {
                                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                                    "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                                    payment,
                                    state
                                ))));
                            }

                            let value = payment.value;
                            let reward_target = try_tx_fus!(get_function_input_data(&decoded, payment_func, 4));
                            let sends_contract_reward = try_tx_fus!(get_function_input_data(&decoded, payment_func, 5));
                            let watcher_reward_amount = try_tx_fus!(get_function_input_data(&decoded, payment_func, 6));

                            let data = try_tx_fus!(spend_func.encode_input(&[
                                swap_id_input,
                                Token::Uint(value),
                                Token::FixedBytes(secret_vec.clone()),
                                Token::Address(Address::default()),
                                Token::Address(payment.sender()),
                                Token::Address(taker_addr),
                                reward_target,
                                sends_contract_reward,
                                watcher_reward_amount,
                            ]));

                            clone.sign_and_send_transaction(
                                0.into(),
                                Action::Call(swap_contract_address),
                                data,
                                U256::from(ETH_GAS),
                            )
                        }),
                )
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let function_name = get_function_name("erc20Payment", watcher_reward);
                let payment_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));

                let decoded = try_tx_fus!(decode_contract_call(payment_func, &payment.data));
                let swap_id_input = try_tx_fus!(get_function_input_data(&decoded, payment_func, 0));
                let amount_input = try_tx_fus!(get_function_input_data(&decoded, payment_func, 1));

                let reward_target = try_tx_fus!(get_function_input_data(&decoded, payment_func, 6));
                let sends_contract_reward = try_tx_fus!(get_function_input_data(&decoded, payment_func, 7));
                let reward_amount = try_tx_fus!(get_function_input_data(&decoded, payment_func, 8));

                let state_f = self.payment_status(swap_contract_address, swap_id_input.clone());

                Box::new(
                    state_f
                        .map_err(TransactionErr::Plain)
                        .and_then(move |state| -> EthTxFut {
                            if state != U256::from(PaymentState::Sent as u8) {
                                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                                    "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                                    payment,
                                    state
                                ))));
                            }
                            let data = try_tx_fus!(spend_func.encode_input(&[
                                swap_id_input.clone(),
                                amount_input,
                                Token::FixedBytes(secret_vec.clone()),
                                Token::Address(token_addr),
                                Token::Address(payment.sender()),
                                Token::Address(taker_addr),
                                reward_target,
                                sends_contract_reward,
                                reward_amount
                            ]));
                            clone.sign_and_send_transaction(
                                0.into(),
                                Action::Call(swap_contract_address),
                                data,
                                U256::from(ETH_GAS),
                            )
                        }),
                )
            },
        }
    }

    fn watcher_refunds_hash_time_locked_payment(&self, args: RefundPaymentArgs) -> EthTxFut {
        let tx: UnverifiedTransaction = try_tx_fus!(rlp::decode(args.payment_tx));
        let payment = try_tx_fus!(SignedEthTx::new(tx));

        let function_name = get_function_name("senderRefund", true);
        let refund_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));

        let clone = self.clone();
        let taker_addr = addr_from_raw_pubkey(args.other_pubkey).unwrap();
        let swap_contract_address = match payment.action {
            Call(address) => address,
            Create => {
                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                    "Invalid payment action: the payment action cannot be create"
                ))))
            },
        };

        match self.coin_type {
            EthCoinType::Eth => {
                let function_name = get_function_name("ethPayment", true);
                let payment_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));
                let decoded = try_tx_fus!(decode_contract_call(payment_func, &payment.data));
                let swap_id_input = try_tx_fus!(get_function_input_data(&decoded, payment_func, 0));
                let receiver_input = try_tx_fus!(get_function_input_data(&decoded, payment_func, 1));
                let hash_input = try_tx_fus!(get_function_input_data(&decoded, payment_func, 2));

                let state_f = self.payment_status(swap_contract_address, swap_id_input.clone());
                Box::new(
                    state_f
                        .map_err(TransactionErr::Plain)
                        .and_then(move |state| -> EthTxFut {
                            if state != U256::from(PaymentState::Sent as u8) {
                                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                                    "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                                    payment,
                                    state
                                ))));
                            }

                            let value = payment.value;
                            let reward_target = try_tx_fus!(get_function_input_data(&decoded, payment_func, 4));
                            let sends_contract_reward = try_tx_fus!(get_function_input_data(&decoded, payment_func, 5));
                            let reward_amount = try_tx_fus!(get_function_input_data(&decoded, payment_func, 6));

                            let data = try_tx_fus!(refund_func.encode_input(&[
                                swap_id_input.clone(),
                                Token::Uint(value),
                                hash_input.clone(),
                                Token::Address(Address::default()),
                                Token::Address(taker_addr),
                                receiver_input.clone(),
                                reward_target,
                                sends_contract_reward,
                                reward_amount
                            ]));

                            clone.sign_and_send_transaction(
                                0.into(),
                                Action::Call(swap_contract_address),
                                data,
                                U256::from(ETH_GAS),
                            )
                        }),
                )
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let function_name = get_function_name("erc20Payment", true);
                let payment_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));

                let decoded = try_tx_fus!(decode_contract_call(payment_func, &payment.data));
                let swap_id_input = try_tx_fus!(get_function_input_data(&decoded, payment_func, 0));
                let amount_input = try_tx_fus!(get_function_input_data(&decoded, payment_func, 1));
                let receiver_input = try_tx_fus!(get_function_input_data(&decoded, payment_func, 3));
                let hash_input = try_tx_fus!(get_function_input_data(&decoded, payment_func, 4));

                let reward_target = try_tx_fus!(get_function_input_data(&decoded, payment_func, 6));
                let sends_contract_reward = try_tx_fus!(get_function_input_data(&decoded, payment_func, 7));
                let reward_amount = try_tx_fus!(get_function_input_data(&decoded, payment_func, 8));

                let state_f = self.payment_status(swap_contract_address, swap_id_input.clone());
                Box::new(
                    state_f
                        .map_err(TransactionErr::Plain)
                        .and_then(move |state| -> EthTxFut {
                            if state != U256::from(PaymentState::Sent as u8) {
                                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                                    "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                                    payment,
                                    state
                                ))));
                            }

                            let data = try_tx_fus!(refund_func.encode_input(&[
                                swap_id_input.clone(),
                                amount_input.clone(),
                                hash_input.clone(),
                                Token::Address(token_addr),
                                Token::Address(taker_addr),
                                receiver_input.clone(),
                                reward_target,
                                sends_contract_reward,
                                reward_amount
                            ]));

                            clone.sign_and_send_transaction(
                                0.into(),
                                Action::Call(swap_contract_address),
                                data,
                                U256::from(ETH_GAS),
                            )
                        }),
                )
            },
        }
    }

    fn spend_hash_time_locked_payment(&self, args: SpendPaymentArgs) -> EthTxFut {
        let tx: UnverifiedTransaction = try_tx_fus!(rlp::decode(args.other_payment_tx));
        let payment = try_tx_fus!(SignedEthTx::new(tx));
        let swap_contract_address = try_tx_fus!(args.swap_contract_address.try_to_address());

        let function_name = get_function_name("receiverSpend", args.watcher_reward);
        let spend_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));

        let clone = self.clone();
        let secret_vec = args.secret.to_vec();
        let watcher_reward = args.watcher_reward;

        match self.coin_type {
            EthCoinType::Eth => {
                let function_name = get_function_name("ethPayment", watcher_reward);
                let payment_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));
                let decoded = try_tx_fus!(decode_contract_call(payment_func, &payment.data));

                let state_f = self.payment_status(swap_contract_address, decoded[0].clone());
                Box::new(
                    state_f
                        .map_err(TransactionErr::Plain)
                        .and_then(move |state| -> EthTxFut {
                            if state != U256::from(PaymentState::Sent as u8) {
                                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                                    "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                                    payment,
                                    state
                                ))));
                            }

                            let data = if watcher_reward {
                                try_tx_fus!(spend_func.encode_input(&[
                                    decoded[0].clone(),
                                    Token::Uint(payment.value),
                                    Token::FixedBytes(secret_vec),
                                    Token::Address(Address::default()),
                                    Token::Address(payment.sender()),
                                    Token::Address(clone.my_address),
                                    decoded[4].clone(),
                                    decoded[5].clone(),
                                    decoded[6].clone(),
                                ]))
                            } else {
                                try_tx_fus!(spend_func.encode_input(&[
                                    decoded[0].clone(),
                                    Token::Uint(payment.value),
                                    Token::FixedBytes(secret_vec),
                                    Token::Address(Address::default()),
                                    Token::Address(payment.sender()),
                                ]))
                            };

                            clone.sign_and_send_transaction(
                                0.into(),
                                Action::Call(swap_contract_address),
                                data,
                                U256::from(ETH_GAS),
                            )
                        }),
                )
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let function_name = get_function_name("erc20Payment", watcher_reward);
                let payment_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));

                let decoded = try_tx_fus!(decode_contract_call(payment_func, &payment.data));
                let state_f = self.payment_status(swap_contract_address, decoded[0].clone());

                Box::new(
                    state_f
                        .map_err(TransactionErr::Plain)
                        .and_then(move |state| -> EthTxFut {
                            if state != U256::from(PaymentState::Sent as u8) {
                                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                                    "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                                    payment,
                                    state
                                ))));
                            }
                            let data = if watcher_reward {
                                try_tx_fus!(spend_func.encode_input(&[
                                    decoded[0].clone(),
                                    decoded[1].clone(),
                                    Token::FixedBytes(secret_vec),
                                    Token::Address(token_addr),
                                    Token::Address(payment.sender()),
                                    Token::Address(clone.my_address),
                                    decoded[6].clone(),
                                    decoded[7].clone(),
                                    decoded[8].clone(),
                                ]))
                            } else {
                                try_tx_fus!(spend_func.encode_input(&[
                                    decoded[0].clone(),
                                    decoded[1].clone(),
                                    Token::FixedBytes(secret_vec),
                                    Token::Address(token_addr),
                                    Token::Address(payment.sender()),
                                ]))
                            };

                            clone.sign_and_send_transaction(
                                0.into(),
                                Action::Call(swap_contract_address),
                                data,
                                U256::from(ETH_GAS),
                            )
                        }),
                )
            },
        }
    }

    fn refund_hash_time_locked_payment(&self, args: RefundPaymentArgs) -> EthTxFut {
        let tx: UnverifiedTransaction = try_tx_fus!(rlp::decode(args.payment_tx));
        let payment = try_tx_fus!(SignedEthTx::new(tx));
        let swap_contract_address = try_tx_fus!(args.swap_contract_address.try_to_address());

        let function_name = get_function_name("senderRefund", args.watcher_reward);
        let refund_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));
        let watcher_reward = args.watcher_reward;

        let clone = self.clone();

        match self.coin_type {
            EthCoinType::Eth => {
                let function_name = get_function_name("ethPayment", watcher_reward);
                let payment_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));

                let decoded = try_tx_fus!(decode_contract_call(payment_func, &payment.data));

                let state_f = self.payment_status(swap_contract_address, decoded[0].clone());
                Box::new(
                    state_f
                        .map_err(TransactionErr::Plain)
                        .and_then(move |state| -> EthTxFut {
                            if state != U256::from(PaymentState::Sent as u8) {
                                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                                    "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                                    payment,
                                    state
                                ))));
                            }

                            let value = payment.value;
                            let data = if watcher_reward {
                                try_tx_fus!(refund_func.encode_input(&[
                                    decoded[0].clone(),
                                    Token::Uint(value),
                                    decoded[2].clone(),
                                    Token::Address(Address::default()),
                                    Token::Address(clone.my_address),
                                    decoded[1].clone(),
                                    decoded[4].clone(),
                                    decoded[5].clone(),
                                    decoded[6].clone(),
                                ]))
                            } else {
                                try_tx_fus!(refund_func.encode_input(&[
                                    decoded[0].clone(),
                                    Token::Uint(value),
                                    decoded[2].clone(),
                                    Token::Address(Address::default()),
                                    decoded[1].clone(),
                                ]))
                            };

                            clone.sign_and_send_transaction(
                                0.into(),
                                Action::Call(swap_contract_address),
                                data,
                                U256::from(ETH_GAS),
                            )
                        }),
                )
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let function_name = get_function_name("erc20Payment", watcher_reward);
                let payment_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));

                let decoded = try_tx_fus!(decode_contract_call(payment_func, &payment.data));
                let state_f = self.payment_status(swap_contract_address, decoded[0].clone());
                Box::new(
                    state_f
                        .map_err(TransactionErr::Plain)
                        .and_then(move |state| -> EthTxFut {
                            if state != U256::from(PaymentState::Sent as u8) {
                                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                                    "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                                    payment,
                                    state
                                ))));
                            }

                            let data = if watcher_reward {
                                try_tx_fus!(refund_func.encode_input(&[
                                    decoded[0].clone(),
                                    decoded[1].clone(),
                                    decoded[4].clone(),
                                    Token::Address(token_addr),
                                    Token::Address(clone.my_address),
                                    decoded[3].clone(),
                                    decoded[6].clone(),
                                    decoded[7].clone(),
                                    decoded[8].clone(),
                                ]))
                            } else {
                                try_tx_fus!(refund_func.encode_input(&[
                                    decoded[0].clone(),
                                    decoded[1].clone(),
                                    decoded[4].clone(),
                                    Token::Address(token_addr),
                                    decoded[3].clone(),
                                ]))
                            };

                            clone.sign_and_send_transaction(
                                0.into(),
                                Action::Call(swap_contract_address),
                                data,
                                U256::from(ETH_GAS),
                            )
                        }),
                )
            },
        }
    }

    fn my_balance(&self) -> BalanceFut<U256> {
        let coin = self.clone();
        let fut = async move {
            match coin.coin_type {
                EthCoinType::Eth => Ok(coin
                    .web3
                    .eth()
                    .balance(coin.my_address, Some(BlockNumber::Latest))
                    .await?),
                EthCoinType::Erc20 { ref token_addr, .. } => {
                    let function = ERC20_CONTRACT.function("balanceOf")?;
                    let data = function.encode_input(&[Token::Address(coin.my_address)])?;

                    let res = coin.call_request(*token_addr, None, Some(data.into())).await?;
                    let decoded = function.decode_output(&res.0)?;
                    match decoded[0] {
                        Token::Uint(number) => Ok(number),
                        _ => {
                            let error = format!("Expected U256 as balanceOf result but got {:?}", decoded);
                            MmError::err(BalanceError::InvalidResponse(error))
                        },
                    }
                },
            }
        };
        Box::new(fut.boxed().compat())
    }

    pub async fn get_tokens_balance_list(&self) -> Result<HashMap<String, CoinBalance>, MmError<BalanceError>> {
        let coin = || self;
        let mut requests = Vec::new();
        for (token_ticker, info) in self.get_erc_tokens_infos() {
            let fut = async move {
                let balance_as_u256 = coin().get_token_balance_by_address(info.token_address).await?;
                let balance_as_big_decimal = u256_to_big_decimal(balance_as_u256, info.decimals)?;
                let balance = CoinBalance::new(balance_as_big_decimal);
                Ok((token_ticker, balance))
            };
            requests.push(fut);
        }

        try_join_all(requests).await.map(|res| res.into_iter().collect())
    }

    async fn get_token_balance_by_address(&self, token_address: Address) -> Result<U256, MmError<BalanceError>> {
        let coin = self.clone();
        let function = ERC20_CONTRACT.function("balanceOf")?;
        let data = function.encode_input(&[Token::Address(coin.my_address)])?;
        let res = coin.call_request(token_address, None, Some(data.into())).await?;
        let decoded = function.decode_output(&res.0)?;

        match decoded[0] {
            Token::Uint(number) => Ok(number),
            _ => {
                let error = format!("Expected U256 as balanceOf result but got {:?}", decoded);
                MmError::err(BalanceError::InvalidResponse(error))
            },
        }
    }

    fn estimate_gas(&self, req: CallRequest) -> Box<dyn Future<Item = U256, Error = web3::Error> + Send> {
        // always using None block number as old Geth version accept only single argument in this RPC
        Box::new(self.web3.eth().estimate_gas(req, None).compat())
    }

    /// Estimates how much gas is necessary to allow the contract call to complete.
    /// `contract_addr` can be a ERC20 token address or any other contract address.
    ///
    /// # Important
    ///
    /// Don't use this method to estimate gas for a withdrawal of `ETH` coin.
    /// For more details, see `withdraw_impl`.
    ///
    /// Also, note that the contract call has to be initiated by my wallet address,
    /// because [`CallRequest::from`] is set to [`EthCoinImpl::my_address`].
    fn estimate_gas_for_contract_call(&self, contract_addr: Address, call_data: Bytes) -> Web3RpcFut<U256> {
        let coin = self.clone();
        Box::new(coin.get_gas_price().and_then(move |gas_price| {
            let eth_value = U256::zero();
            let estimate_gas_req = CallRequest {
                value: Some(eth_value),
                data: Some(call_data),
                from: Some(coin.my_address),
                to: Some(contract_addr),
                gas: None,
                // gas price must be supplied because some smart contracts base their
                // logic on gas price, e.g. TUSD: https://github.com/KomodoPlatform/atomicDEX-API/issues/643
                gas_price: Some(gas_price),
                ..CallRequest::default()
            };
            coin.estimate_gas(estimate_gas_req).map_to_mm_fut(Web3RpcError::from)
        }))
    }

    fn eth_balance(&self) -> BalanceFut<U256> {
        Box::new(
            self.web3
                .eth()
                .balance(self.my_address, Some(BlockNumber::Latest))
                .compat()
                .map_to_mm_fut(BalanceError::from),
        )
    }

    async fn call_request(&self, to: Address, value: Option<U256>, data: Option<Bytes>) -> Result<Bytes, web3::Error> {
        let request = CallRequest {
            from: Some(self.my_address),
            to: Some(to),
            gas: None,
            gas_price: None,
            value,
            data,
            ..CallRequest::default()
        };

        self.web3
            .eth()
            .call(request, Some(BlockId::Number(BlockNumber::Latest)))
            .await
    }

    fn allowance(&self, spender: Address) -> Web3RpcFut<U256> {
        let coin = self.clone();
        let fut = async move {
            match coin.coin_type {
                EthCoinType::Eth => MmError::err(Web3RpcError::Internal(
                    "'allowance' must not be called for ETH coin".to_owned(),
                )),
                EthCoinType::Erc20 { ref token_addr, .. } => {
                    let function = ERC20_CONTRACT.function("allowance")?;
                    let data = function.encode_input(&[Token::Address(coin.my_address), Token::Address(spender)])?;

                    let res = coin.call_request(*token_addr, None, Some(data.into())).await?;
                    let decoded = function.decode_output(&res.0)?;

                    match decoded[0] {
                        Token::Uint(number) => Ok(number),
                        _ => {
                            let error = format!("Expected U256 as allowance result but got {:?}", decoded);
                            MmError::err(Web3RpcError::InvalidResponse(error))
                        },
                    }
                },
            }
        };
        Box::new(fut.boxed().compat())
    }

    fn wait_for_required_allowance(
        &self,
        spender: Address,
        required_allowance: U256,
        wait_until: u64,
    ) -> Web3RpcFut<()> {
        const CHECK_ALLOWANCE_EVERY: f64 = 5.;

        let selfi = self.clone();
        let fut = async move {
            loop {
                if now_sec() > wait_until {
                    return MmError::err(Web3RpcError::Timeout(ERRL!(
                        "Waited too long until {} for allowance to be updated to at least {}",
                        wait_until,
                        required_allowance
                    )));
                }

                match selfi.allowance(spender).compat().await {
                    Ok(allowed) if allowed >= required_allowance => return Ok(()),
                    Ok(_allowed) => (),
                    Err(e) => match e.get_inner() {
                        Web3RpcError::Transport(e) => error!("Error {} on trying to get the allowed amount!", e),
                        _ => return Err(e),
                    },
                }

                Timer::sleep(CHECK_ALLOWANCE_EVERY).await;
            }
        };
        Box::new(fut.boxed().compat())
    }

    fn approve(&self, spender: Address, amount: U256) -> EthTxFut {
        let coin = self.clone();
        let fut = async move {
            let token_addr = match coin.coin_type {
                EthCoinType::Eth => return TX_PLAIN_ERR!("'approve' is expected to be call for ERC20 coins only"),
                EthCoinType::Erc20 { token_addr, .. } => token_addr,
            };
            let function = try_tx_s!(ERC20_CONTRACT.function("approve"));
            let data = try_tx_s!(function.encode_input(&[Token::Address(spender), Token::Uint(amount)]));

            let gas_limit = try_tx_s!(
                coin.estimate_gas_for_contract_call(token_addr, Bytes::from(data.clone()))
                    .compat()
                    .await
            );

            coin.sign_and_send_transaction(0.into(), Action::Call(token_addr), data, gas_limit)
                .compat()
                .await
        };
        Box::new(fut.boxed().compat())
    }

    /// Gets `PaymentSent` events from etomic swap smart contract since `from_block`
    fn payment_sent_events(
        &self,
        swap_contract_address: Address,
        from_block: u64,
        to_block: u64,
    ) -> Box<dyn Future<Item = Vec<Log>, Error = String> + Send> {
        let contract_event = try_fus!(SWAP_CONTRACT.event("PaymentSent"));
        let filter = FilterBuilder::default()
            .topics(Some(vec![contract_event.signature()]), None, None, None)
            .from_block(BlockNumber::Number(from_block.into()))
            .to_block(BlockNumber::Number(to_block.into()))
            .address(vec![swap_contract_address])
            .build();

        Box::new(self.web3.eth().logs(filter).compat().map_err(|e| ERRL!("{}", e)))
    }

    /// Gets `ReceiverSpent` events from etomic swap smart contract since `from_block`
    fn spend_events(
        &self,
        swap_contract_address: Address,
        from_block: u64,
        to_block: u64,
    ) -> Box<dyn Future<Item = Vec<Log>, Error = String> + Send> {
        let contract_event = try_fus!(SWAP_CONTRACT.event("ReceiverSpent"));
        let filter = FilterBuilder::default()
            .topics(Some(vec![contract_event.signature()]), None, None, None)
            .from_block(BlockNumber::Number(from_block.into()))
            .to_block(BlockNumber::Number(to_block.into()))
            .address(vec![swap_contract_address])
            .build();

        Box::new(self.web3.eth().logs(filter).compat().map_err(|e| ERRL!("{}", e)))
    }

    fn validate_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        let expected_swap_contract_address = try_f!(input
            .swap_contract_address
            .try_to_address()
            .map_to_mm(ValidatePaymentError::InvalidParameter));

        let unsigned: UnverifiedTransaction = try_f!(rlp::decode(&input.payment_tx));
        let tx =
            try_f!(SignedEthTx::new(unsigned)
                .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string())));
        let sender = try_f!(addr_from_raw_pubkey(&input.other_pub).map_to_mm(ValidatePaymentError::InvalidParameter));

        let selfi = self.clone();
        let swap_id = selfi.etomic_swap_id(input.time_lock, &input.secret_hash);
        let decimals = self.decimals;
        let secret_hash = if input.secret_hash.len() == 32 {
            ripemd160(&input.secret_hash).to_vec()
        } else {
            input.secret_hash.to_vec()
        };
        let trade_amount = try_f!(wei_from_big_decimal(&(input.amount), decimals));
        let fut = async move {
            let status = selfi
                .payment_status(expected_swap_contract_address, Token::FixedBytes(swap_id.clone()))
                .compat()
                .await
                .map_to_mm(ValidatePaymentError::Transport)?;
            if status != U256::from(PaymentState::Sent as u8) {
                return MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
                    "Payment state is not PAYMENT_STATE_SENT, got {}",
                    status
                )));
            }

            let tx_from_rpc = selfi.web3.eth().transaction(TransactionId::Hash(tx.hash)).await?;
            let tx_from_rpc = tx_from_rpc.as_ref().ok_or_else(|| {
                ValidatePaymentError::TxDoesNotExist(format!("Didn't find provided tx {:?} on ETH node", tx.hash))
            })?;

            if tx_from_rpc.from != Some(sender) {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Payment tx {:?} was sent from wrong address, expected {:?}",
                    tx_from_rpc, sender
                )));
            }

            match &selfi.coin_type {
                EthCoinType::Eth => {
                    let mut expected_value = trade_amount;

                    if tx_from_rpc.to != Some(expected_swap_contract_address) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx {:?} was sent to wrong address, expected {:?}",
                            tx_from_rpc, expected_swap_contract_address,
                        )));
                    }

                    let function_name = get_function_name("ethPayment", input.watcher_reward.is_some());
                    let function = SWAP_CONTRACT
                        .function(&function_name)
                        .map_to_mm(|err| ValidatePaymentError::InternalError(err.to_string()))?;

                    let decoded = decode_contract_call(function, &tx_from_rpc.input.0)
                        .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string()))?;

                    if decoded[0] != Token::FixedBytes(swap_id.clone()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Invalid 'swap_id' {:?}, expected {:?}",
                            decoded, swap_id
                        )));
                    }

                    if decoded[1] != Token::Address(selfi.my_address) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx receiver arg {:?} is invalid, expected {:?}",
                            decoded[1],
                            Token::Address(selfi.my_address)
                        )));
                    }

                    if decoded[2] != Token::FixedBytes(secret_hash.to_vec()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx secret_hash arg {:?} is invalid, expected {:?}",
                            decoded[2],
                            Token::FixedBytes(secret_hash.to_vec()),
                        )));
                    }

                    if decoded[3] != Token::Uint(U256::from(input.time_lock)) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx time_lock arg {:?} is invalid, expected {:?}",
                            decoded[3],
                            Token::Uint(U256::from(input.time_lock)),
                        )));
                    }

                    if let Some(watcher_reward) = input.watcher_reward {
                        if decoded[4] != Token::Uint(U256::from(watcher_reward.reward_target as u8)) {
                            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                                "Payment tx reward target arg {:?} is invalid, expected {:?}",
                                decoded[4], watcher_reward.reward_target as u8
                            )));
                        }

                        if decoded[5] != Token::Bool(watcher_reward.send_contract_reward_on_spend) {
                            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                                "Payment tx sends_contract_reward_on_spend arg {:?} is invalid, expected {:?}",
                                decoded[5], watcher_reward.send_contract_reward_on_spend
                            )));
                        }

                        let expected_reward_amount = wei_from_big_decimal(&watcher_reward.amount, decimals)?;
                        let actual_reward_amount = decoded[6].clone().into_uint().ok_or_else(|| {
                            ValidatePaymentError::WrongPaymentTx("Invalid type for watcher reward argument".to_string())
                        })?;

                        validate_watcher_reward(
                            expected_reward_amount.as_u64(),
                            actual_reward_amount.as_u64(),
                            watcher_reward.is_exact_amount,
                        )?;

                        match watcher_reward.reward_target {
                            RewardTarget::None | RewardTarget::PaymentReceiver => (),
                            RewardTarget::PaymentSender | RewardTarget::PaymentSpender | RewardTarget::Contract => {
                                expected_value += actual_reward_amount
                            },
                        };
                    }

                    if tx_from_rpc.value != expected_value {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx value arg {:?} is invalid, expected {:?}",
                            tx_from_rpc.value, trade_amount
                        )));
                    }
                },
                EthCoinType::Erc20 {
                    platform: _,
                    token_addr,
                } => {
                    let mut expected_value = U256::from(0);
                    let mut expected_amount = trade_amount;

                    if tx_from_rpc.to != Some(expected_swap_contract_address) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx {:?} was sent to wrong address, expected {:?}",
                            tx_from_rpc, expected_swap_contract_address,
                        )));
                    }
                    let function_name = get_function_name("erc20Payment", input.watcher_reward.is_some());
                    let function = SWAP_CONTRACT
                        .function(&function_name)
                        .map_to_mm(|err| ValidatePaymentError::InternalError(err.to_string()))?;
                    let decoded = decode_contract_call(function, &tx_from_rpc.input.0)
                        .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string()))?;

                    if decoded[0] != Token::FixedBytes(swap_id.clone()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Invalid 'swap_id' {:?}, expected {:?}",
                            decoded, swap_id
                        )));
                    }

                    if decoded[2] != Token::Address(*token_addr) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx token_addr arg {:?} is invalid, expected {:?}",
                            decoded[2],
                            Token::Address(*token_addr)
                        )));
                    }

                    if decoded[3] != Token::Address(selfi.my_address) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx receiver arg {:?} is invalid, expected {:?}",
                            decoded[3],
                            Token::Address(selfi.my_address),
                        )));
                    }

                    if decoded[4] != Token::FixedBytes(secret_hash.to_vec()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx secret_hash arg {:?} is invalid, expected {:?}",
                            decoded[4],
                            Token::FixedBytes(secret_hash.to_vec()),
                        )));
                    }

                    if decoded[5] != Token::Uint(U256::from(input.time_lock)) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx time_lock arg {:?} is invalid, expected {:?}",
                            decoded[5],
                            Token::Uint(U256::from(input.time_lock)),
                        )));
                    }

                    if let Some(watcher_reward) = input.watcher_reward {
                        if decoded[6] != Token::Uint(U256::from(watcher_reward.reward_target as u8)) {
                            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                                "Payment tx reward target arg {:?} is invalid, expected {:?}",
                                decoded[4], watcher_reward.reward_target as u8
                            )));
                        }

                        if decoded[7] != Token::Bool(watcher_reward.send_contract_reward_on_spend) {
                            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                                "Payment tx sends_contract_reward_on_spend arg {:?} is invalid, expected {:?}",
                                decoded[5], watcher_reward.send_contract_reward_on_spend
                            )));
                        }

                        let expected_reward_amount = wei_from_big_decimal(&watcher_reward.amount, decimals)?;
                        let actual_reward_amount = get_function_input_data(&decoded, function, 8)
                            .map_to_mm(ValidatePaymentError::TxDeserializationError)?
                            .into_uint()
                            .ok_or_else(|| {
                                ValidatePaymentError::WrongPaymentTx(
                                    "Invalid type for watcher reward argument".to_string(),
                                )
                            })?;

                        validate_watcher_reward(
                            expected_reward_amount.as_u64(),
                            actual_reward_amount.as_u64(),
                            watcher_reward.is_exact_amount,
                        )?;

                        match watcher_reward.reward_target {
                            RewardTarget::PaymentSender | RewardTarget::Contract => {
                                expected_value += actual_reward_amount
                            },
                            RewardTarget::PaymentSpender => expected_amount += actual_reward_amount,
                            _ => (),
                        };

                        if decoded[1] != Token::Uint(expected_amount) {
                            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                                "Payment tx amount arg {:?} is invalid, expected {:?}",
                                decoded[1], expected_amount,
                            )));
                        }
                    }

                    if tx_from_rpc.value != expected_value {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx value arg {:?} is invalid, expected {:?}",
                            tx_from_rpc.value, trade_amount
                        )));
                    }
                },
            }

            Ok(())
        };
        Box::new(fut.boxed().compat())
    }

    fn payment_status(
        &self,
        swap_contract_address: H160,
        token: Token,
    ) -> Box<dyn Future<Item = U256, Error = String> + Send + 'static> {
        let function = try_fus!(SWAP_CONTRACT.function("payments"));

        let data = try_fus!(function.encode_input(&[token]));

        let coin = self.clone();
        let fut = async move { coin.call_request(swap_contract_address, None, Some(data.into())).await };

        Box::new(fut.boxed().compat().map_err(|e| ERRL!("{}", e)).and_then(move |bytes| {
            let decoded_tokens = try_s!(function.decode_output(&bytes.0));
            let state = decoded_tokens
                .get(2)
                .ok_or_else(|| ERRL!("Payment status must contain 'state' as the 2nd token"))?;
            match state {
                Token::Uint(state) => Ok(*state),
                _ => ERR!("Payment status must be uint, got {:?}", state),
            }
        }))
    }

    async fn search_for_swap_tx_spend(
        &self,
        tx: &[u8],
        swap_contract_address: Address,
        _secret_hash: &[u8],
        search_from_block: u64,
        watcher_reward: bool,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        let unverified: UnverifiedTransaction = try_s!(rlp::decode(tx));
        let tx = try_s!(SignedEthTx::new(unverified));

        let func_name = match self.coin_type {
            EthCoinType::Eth => get_function_name("ethPayment", watcher_reward),
            EthCoinType::Erc20 { .. } => get_function_name("erc20Payment", watcher_reward),
        };

        let payment_func = try_s!(SWAP_CONTRACT.function(&func_name));
        let decoded = try_s!(decode_contract_call(payment_func, &tx.data));
        let id = match decoded.first() {
            Some(Token::FixedBytes(bytes)) => bytes.clone(),
            invalid_token => return ERR!("Expected Token::FixedBytes, got {:?}", invalid_token),
        };

        let mut current_block = try_s!(self.current_block().compat().await);
        if current_block < search_from_block {
            current_block = search_from_block;
        }

        let mut from_block = search_from_block;

        loop {
            let to_block = current_block.min(from_block + self.logs_block_range);

            let spend_events = try_s!(
                self.spend_events(swap_contract_address, from_block, to_block)
                    .compat()
                    .await
            );
            let found = spend_events.iter().find(|event| &event.data.0[..32] == id.as_slice());

            if let Some(event) = found {
                match event.transaction_hash {
                    Some(tx_hash) => {
                        let transaction = match try_s!(self.web3.eth().transaction(TransactionId::Hash(tx_hash)).await)
                        {
                            Some(t) => t,
                            None => {
                                return ERR!("Found ReceiverSpent event, but transaction {:02x} is missing", tx_hash)
                            },
                        };

                        return Ok(Some(FoundSwapTxSpend::Spent(TransactionEnum::from(try_s!(
                            signed_tx_from_web3_tx(transaction)
                        )))));
                    },
                    None => return ERR!("Found ReceiverSpent event, but it doesn't have tx_hash"),
                }
            }

            let refund_events = try_s!(
                self.refund_events(swap_contract_address, from_block, to_block)
                    .compat()
                    .await
            );
            let found = refund_events.iter().find(|event| &event.data.0[..32] == id.as_slice());

            if let Some(event) = found {
                match event.transaction_hash {
                    Some(tx_hash) => {
                        let transaction = match try_s!(self.web3.eth().transaction(TransactionId::Hash(tx_hash)).await)
                        {
                            Some(t) => t,
                            None => {
                                return ERR!("Found SenderRefunded event, but transaction {:02x} is missing", tx_hash)
                            },
                        };

                        return Ok(Some(FoundSwapTxSpend::Refunded(TransactionEnum::from(try_s!(
                            signed_tx_from_web3_tx(transaction)
                        )))));
                    },
                    None => return ERR!("Found SenderRefunded event, but it doesn't have tx_hash"),
                }
            }

            if to_block >= current_block {
                break;
            }
            from_block = to_block;
        }

        Ok(None)
    }

    pub async fn get_watcher_reward_amount(&self, wait_until: u64) -> Result<BigDecimal, MmError<WatcherRewardError>> {
        let gas_price = repeatable!(async { self.get_gas_price().compat().await.retry_on_err() })
            .until_s(wait_until)
            .repeat_every_secs(10.)
            .await
            .map_err(|_| WatcherRewardError::RPCError("Error getting the gas price".to_string()))?;

        let gas_cost_wei = U256::from(REWARD_GAS_AMOUNT) * gas_price;
        let gas_cost_eth =
            u256_to_big_decimal(gas_cost_wei, 18).map_err(|e| WatcherRewardError::InternalError(e.to_string()))?;
        Ok(gas_cost_eth)
    }

    /// Get gas price
    pub fn get_gas_price(&self) -> Web3RpcFut<U256> {
        let coin = self.clone();
        let fut = async move {
            // TODO refactor to error_log_passthrough once simple maker bot is merged
            let gas_station_price = match &coin.gas_station_url {
                Some(url) => {
                    match GasStationData::get_gas_price(url, coin.gas_station_decimals, coin.gas_station_policy)
                        .compat()
                        .await
                    {
                        Ok(from_station) => Some(increase_by_percent_one_gwei(from_station, GAS_PRICE_PERCENT)),
                        Err(e) => {
                            error!("Error {} on request to gas station url {}", e, url);
                            None
                        },
                    }
                },
                None => None,
            };

            let eth_gas_price = match coin.web3.eth().gas_price().await {
                Ok(eth_gas) => Some(eth_gas),
                Err(e) => {
                    error!("Error {} on eth_gasPrice request", e);
                    None
                },
            };

            let fee_history_namespace: EthFeeHistoryNamespace<_> = coin.web3.api();
            let eth_fee_history_price = match fee_history_namespace
                .eth_fee_history(U256::from(1u64), BlockNumber::Latest, &[])
                .await
            {
                Ok(res) => res
                    .base_fee_per_gas
                    .first()
                    .map(|val| increase_by_percent_one_gwei(*val, BASE_BLOCK_FEE_DIFF_PCT)),
                Err(e) => {
                    debug!("Error {} on eth_feeHistory request", e);
                    None
                },
            };

            // on editions < 2021 the compiler will resolve array.into_iter() as (&array).into_iter()
            // https://doc.rust-lang.org/edition-guide/rust-2021/IntoIterator-for-arrays.html#details
            IntoIterator::into_iter([gas_station_price, eth_gas_price, eth_fee_history_price])
                .flatten()
                .max()
                .or_mm_err(|| Web3RpcError::Internal("All requests failed".into()))
        };
        Box::new(fut.boxed().compat())
    }

    /// Checks every second till at least one ETH node recognizes that nonce is increased.
    /// Parity has reliable "nextNonce" method that always returns correct nonce for address.
    /// But we can't expect that all nodes will always be Parity.
    /// Some of ETH forks use Geth only so they don't have Parity nodes at all.
    ///
    /// Please note that we just keep looping in case of a transport error hoping it will go away.
    ///
    /// # Warning
    ///
    /// The function is endless, we just keep looping in case of a transport error hoping it will go away.
    async fn wait_for_addr_nonce_increase(&self, addr: Address, prev_nonce: U256) {
        repeatable!(async {
            match get_addr_nonce(addr, self.web3_instances.clone()).compat().await {
                Ok((new_nonce, _)) if new_nonce > prev_nonce => Ready(()),
                Ok((_nonce, _)) => Retry(()),
                Err(e) => {
                    error!("Error getting {} {} nonce: {}", self.ticker(), self.my_address, e);
                    Retry(())
                },
            }
        })
        .until_ready()
        .repeat_every_secs(1.)
        .await
        .ok();
    }

    /// Returns `None` if the transaction hasn't appeared on the RPC nodes at the specified time.
    #[cfg(target_arch = "wasm32")]
    async fn wait_for_tx_appears_on_rpc(
        &self,
        tx_hash: H256,
        wait_rpc_timeout_ms: u64,
        check_every: f64,
    ) -> Web3RpcResult<Option<SignedEthTx>> {
        let wait_until = wait_until_ms(wait_rpc_timeout_ms);
        while now_ms() < wait_until {
            let maybe_tx = self.web3.eth().transaction(TransactionId::Hash(tx_hash)).await?;
            if let Some(tx) = maybe_tx {
                let signed_tx = signed_tx_from_web3_tx(tx).map_to_mm(Web3RpcError::InvalidResponse)?;
                return Ok(Some(signed_tx));
            }

            Timer::sleep(check_every).await;
        }

        let timeout_s = wait_rpc_timeout_ms / 1000;
        warn!(
            "Couldn't fetch the '{tx_hash:02x}' transaction hex as it hasn't appeared on the RPC node in {timeout_s}s"
        );
        Ok(None)
    }

    fn transaction_confirmed_at(&self, payment_hash: H256, wait_until: u64, check_every: f64) -> Web3RpcFut<U64> {
        let selfi = self.clone();
        let fut = async move {
            loop {
                if now_sec() > wait_until {
                    return MmError::err(Web3RpcError::Timeout(ERRL!(
                        "Waited too long until {} for payment tx: {:02x}, for coin:{}, to be confirmed!",
                        wait_until,
                        payment_hash,
                        selfi.ticker()
                    )));
                }

                let web3_receipt = match selfi.web3.eth().transaction_receipt(payment_hash).await {
                    Ok(r) => r,
                    Err(e) => {
                        error!(
                            "Error {:?} getting the {} transaction {:?}, retrying in 15 seconds",
                            e,
                            selfi.ticker(),
                            payment_hash
                        );
                        Timer::sleep(check_every).await;
                        continue;
                    },
                };

                if let Some(receipt) = web3_receipt {
                    if receipt.status != Some(1.into()) {
                        return MmError::err(Web3RpcError::Internal(ERRL!(
                            "Tx receipt {:?} status of {} tx {:?} is failed",
                            receipt,
                            selfi.ticker(),
                            payment_hash
                        )));
                    }

                    if let Some(confirmed_at) = receipt.block_number {
                        break Ok(confirmed_at);
                    }
                }

                Timer::sleep(check_every).await;
            }
        };
        Box::new(fut.boxed().compat())
    }

    fn wait_for_block(&self, block_number: U64, wait_until: u64, check_every: f64) -> Web3RpcFut<()> {
        let selfi = self.clone();
        let fut = async move {
            loop {
                if now_sec() > wait_until {
                    return MmError::err(Web3RpcError::Timeout(ERRL!(
                        "Waited too long until {} for block number: {:02x} to appear on-chain, for coin:{}",
                        wait_until,
                        block_number,
                        selfi.ticker()
                    )));
                }

                match selfi.web3.eth().block_number().await {
                    Ok(current_block) => {
                        if current_block >= block_number {
                            break Ok(());
                        }
                    },
                    Err(e) => {
                        error!(
                            "Error {:?} getting the {} block number retrying in 15 seconds",
                            e,
                            selfi.ticker()
                        );
                    },
                };

                Timer::sleep(check_every).await;
            }
        };
        Box::new(fut.boxed().compat())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EthTxFeeDetails {
    pub coin: String,
    pub gas: u64,
    /// WEI units per 1 gas
    pub gas_price: BigDecimal,
    pub total_fee: BigDecimal,
}

impl EthTxFeeDetails {
    fn new(gas: U256, gas_price: U256, coin: &str) -> NumConversResult<EthTxFeeDetails> {
        let total_fee = gas * gas_price;
        // Fees are always paid in ETH, can use 18 decimals by default
        let total_fee = u256_to_big_decimal(total_fee, ETH_DECIMALS)?;
        let gas_price = u256_to_big_decimal(gas_price, ETH_DECIMALS)?;

        let gas_u64 = u64::try_from(gas).map_to_mm(|e| NumConversError::new(e.to_string()))?;

        Ok(EthTxFeeDetails {
            coin: coin.to_owned(),
            gas: gas_u64,
            gas_price,
            total_fee,
        })
    }
}

#[async_trait]
impl MmCoin for EthCoin {
    fn is_asset_chain(&self) -> bool { false }

    fn spawner(&self) -> CoinFutSpawner { CoinFutSpawner::new(&self.abortable_system) }

    fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut {
        Box::new(get_raw_transaction_impl(self.clone(), req).boxed().compat())
    }

    fn get_tx_hex_by_hash(&self, tx_hash: Vec<u8>) -> RawTransactionFut {
        if tx_hash.len() != H256::len_bytes() {
            let error = format!(
                "TX hash should have exactly {} bytes, got {}",
                H256::len_bytes(),
                tx_hash.len(),
            );
            return Box::new(futures01::future::err(MmError::new(
                RawTransactionError::InvalidHashError(error),
            )));
        }

        let tx_hash = H256::from_slice(tx_hash.as_slice());
        Box::new(get_tx_hex_by_hash_impl(self.clone(), tx_hash).boxed().compat())
    }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        Box::new(Box::pin(withdraw_impl(self.clone(), req)).compat())
    }

    fn decimals(&self) -> u8 { self.decimals }

    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> {
        let to_address_format: EthAddressFormat =
            json::from_value(to_address_format).map_err(|e| ERRL!("Error on parse ETH address format {:?}", e))?;
        match to_address_format {
            EthAddressFormat::SingleCase => ERR!("conversion is available only to mixed-case"),
            EthAddressFormat::MixedCase => {
                let _addr = try_s!(addr_from_str(from));
                Ok(checksum_address(from))
            },
        }
    }

    fn validate_address(&self, address: &str) -> ValidateAddressResult {
        let result = self.address_from_str(address);
        ValidateAddressResult {
            is_valid: result.is_ok(),
            reason: result.err(),
        }
    }

    fn process_history_loop(&self, ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> {
        cfg_wasm32! {
            ctx.log.log(
                "🤔",
                &[&"tx_history", &self.ticker],
                &ERRL!("Transaction history is not supported for ETH/ERC20 coins"),
            );
            Box::new(futures01::future::ok(()))
        }
        cfg_native! {
            let coin = self.clone();
            let fut = async move {
                match coin.coin_type {
                    EthCoinType::Eth => coin.process_eth_history(&ctx).await,
                    EthCoinType::Erc20 { ref token_addr, .. } => coin.process_erc20_history(*token_addr, &ctx).await,
                }
                Ok(())
            };
            Box::new(fut.boxed().compat())
        }
    }

    fn history_sync_status(&self) -> HistorySyncState { self.history_sync_state.lock().unwrap().clone() }

    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> {
        let coin = self.clone();
        Box::new(
            self.get_gas_price()
                .map_err(|e| e.to_string())
                .and_then(move |gas_price| {
                    let fee = gas_price * U256::from(ETH_GAS);
                    let fee_coin = match &coin.coin_type {
                        EthCoinType::Eth => &coin.ticker,
                        EthCoinType::Erc20 { platform, .. } => platform,
                    };
                    Ok(TradeFee {
                        coin: fee_coin.into(),
                        amount: try_s!(u256_to_big_decimal(fee, ETH_DECIMALS)).into(),
                        paid_from_trading_vol: false,
                    })
                }),
        )
    }

    async fn get_sender_trade_fee(
        &self,
        value: TradePreimageValue,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        let gas_price = self.get_gas_price().compat().await?;
        let gas_price = increase_gas_price_by_stage(gas_price, &stage);
        let gas_limit = match self.coin_type {
            EthCoinType::Eth => {
                // this gas_limit includes gas for `ethPayment` and `senderRefund` contract calls
                U256::from(300_000)
            },
            EthCoinType::Erc20 { token_addr, .. } => {
                let value = match value {
                    TradePreimageValue::Exact(value) | TradePreimageValue::UpperBound(value) => {
                        wei_from_big_decimal(&value, self.decimals)?
                    },
                };
                let allowed = self.allowance(self.swap_contract_address).compat().await?;
                if allowed < value {
                    // estimate gas for the `approve` contract call

                    // Pass a dummy spender. Let's use `my_address`.
                    let spender = self.my_address;
                    let approve_function = ERC20_CONTRACT.function("approve")?;
                    let approve_data = approve_function.encode_input(&[Token::Address(spender), Token::Uint(value)])?;
                    let approve_gas_limit = self
                        .estimate_gas_for_contract_call(token_addr, Bytes::from(approve_data))
                        .compat()
                        .await?;

                    // this gas_limit includes gas for `approve`, `erc20Payment` and `senderRefund` contract calls
                    U256::from(300_000) + approve_gas_limit
                } else {
                    // this gas_limit includes gas for `erc20Payment` and `senderRefund` contract calls
                    U256::from(300_000)
                }
            },
        };

        let total_fee = gas_limit * gas_price;
        let amount = u256_to_big_decimal(total_fee, ETH_DECIMALS)?;
        let fee_coin = match &self.coin_type {
            EthCoinType::Eth => &self.ticker,
            EthCoinType::Erc20 { platform, .. } => platform,
        };
        Ok(TradeFee {
            coin: fee_coin.into(),
            amount: amount.into(),
            paid_from_trading_vol: false,
        })
    }

    fn get_receiver_trade_fee(&self, stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        let coin = self.clone();
        let fut = async move {
            let gas_price = coin.get_gas_price().compat().await?;
            let gas_price = increase_gas_price_by_stage(gas_price, &stage);
            let total_fee = gas_price * U256::from(ETH_GAS);
            let amount = u256_to_big_decimal(total_fee, ETH_DECIMALS)?;
            let fee_coin = match &coin.coin_type {
                EthCoinType::Eth => &coin.ticker,
                EthCoinType::Erc20 { platform, .. } => platform,
            };
            Ok(TradeFee {
                coin: fee_coin.into(),
                amount: amount.into(),
                paid_from_trading_vol: false,
            })
        };
        Box::new(fut.boxed().compat())
    }

    async fn get_fee_to_send_taker_fee(
        &self,
        dex_fee_amount: BigDecimal,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        let dex_fee_amount = wei_from_big_decimal(&dex_fee_amount, self.decimals)?;

        // pass the dummy params
        let to_addr = addr_from_raw_pubkey(&DEX_FEE_ADDR_RAW_PUBKEY)
            .expect("addr_from_raw_pubkey should never fail with DEX_FEE_ADDR_RAW_PUBKEY");
        let (eth_value, data, call_addr, fee_coin) = match &self.coin_type {
            EthCoinType::Eth => (dex_fee_amount, Vec::new(), &to_addr, &self.ticker),
            EthCoinType::Erc20 { platform, token_addr } => {
                let function = ERC20_CONTRACT.function("transfer")?;
                let data = function.encode_input(&[Token::Address(to_addr), Token::Uint(dex_fee_amount)])?;
                (0.into(), data, token_addr, platform)
            },
        };

        let gas_price = self.get_gas_price().compat().await?;
        let gas_price = increase_gas_price_by_stage(gas_price, &stage);
        let estimate_gas_req = CallRequest {
            value: Some(eth_value),
            data: Some(data.clone().into()),
            from: Some(self.my_address),
            to: Some(*call_addr),
            gas: None,
            // gas price must be supplied because some smart contracts base their
            // logic on gas price, e.g. TUSD: https://github.com/KomodoPlatform/atomicDEX-API/issues/643
            gas_price: Some(gas_price),
            ..CallRequest::default()
        };

        // Please note if the wallet's balance is insufficient to withdraw, then `estimate_gas` may fail with the `Exception` error.
        // Ideally we should determine the case when we have the insufficient balance and return `TradePreimageError::NotSufficientBalance` error.
        let gas_limit = self.estimate_gas(estimate_gas_req).compat().await?;
        let total_fee = gas_limit * gas_price;
        let amount = u256_to_big_decimal(total_fee, ETH_DECIMALS)?;
        Ok(TradeFee {
            coin: fee_coin.into(),
            amount: amount.into(),
            paid_from_trading_vol: false,
        })
    }

    fn required_confirmations(&self) -> u64 { self.required_confirmations.load(AtomicOrdering::Relaxed) }

    fn requires_notarization(&self) -> bool { false }

    fn set_required_confirmations(&self, confirmations: u64) {
        self.required_confirmations
            .store(confirmations, AtomicOrdering::Relaxed);
    }

    fn set_requires_notarization(&self, _requires_nota: bool) {
        warn!("set_requires_notarization doesn't take any effect on ETH/ERC20 coins");
    }

    fn swap_contract_address(&self) -> Option<BytesJson> {
        Some(BytesJson::from(self.swap_contract_address.0.as_ref()))
    }

    fn fallback_swap_contract(&self) -> Option<BytesJson> {
        self.fallback_swap_contract.map(|a| BytesJson::from(a.0.as_ref()))
    }

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

    fn on_token_deactivated(&self, ticker: &str) {
        if let Ok(tokens) = self.erc20_tokens_infos.lock().as_deref_mut() {
            tokens.remove(ticker);
        };
    }
}

pub trait TryToAddress {
    fn try_to_address(&self) -> Result<Address, String>;
}

impl TryToAddress for BytesJson {
    fn try_to_address(&self) -> Result<Address, String> { self.0.try_to_address() }
}

impl TryToAddress for [u8] {
    fn try_to_address(&self) -> Result<Address, String> { (&self).try_to_address() }
}

impl<'a> TryToAddress for &'a [u8] {
    fn try_to_address(&self) -> Result<Address, String> {
        if self.len() != Address::len_bytes() {
            return ERR!(
                "Cannot construct an Ethereum address from {} bytes slice",
                Address::len_bytes()
            );
        }

        Ok(Address::from_slice(self))
    }
}

impl<T: TryToAddress> TryToAddress for Option<T> {
    fn try_to_address(&self) -> Result<Address, String> {
        match self {
            Some(ref inner) => inner.try_to_address(),
            None => ERR!("Cannot convert None to address"),
        }
    }
}

pub trait GuiAuthMessages {
    fn gui_auth_sign_message_hash(message: String) -> Option<[u8; 32]>;
    fn generate_gui_auth_signed_validation(generator: GuiAuthValidationGenerator)
        -> SignatureResult<GuiAuthValidation>;
}

impl GuiAuthMessages for EthCoin {
    fn gui_auth_sign_message_hash(message: String) -> Option<[u8; 32]> {
        let message_prefix = "atomicDEX Auth Ethereum Signed Message:\n";
        let prefix_len = CompactInteger::from(message_prefix.len());

        let mut stream = Stream::new();
        prefix_len.serialize(&mut stream);
        stream.append_slice(message_prefix.as_bytes());
        stream.append_slice(message.len().to_string().as_bytes());
        stream.append_slice(message.as_bytes());

        Some(keccak256(&stream.out()).take())
    }

    fn generate_gui_auth_signed_validation(
        generator: GuiAuthValidationGenerator,
    ) -> SignatureResult<GuiAuthValidation> {
        let timestamp_message = get_utc_timestamp() + GUI_AUTH_SIGNED_MESSAGE_LIFETIME_SEC;

        let message_hash =
            EthCoin::gui_auth_sign_message_hash(timestamp_message.to_string()).ok_or(SignatureError::PrefixNotFound)?;
        let signature = sign(&generator.secret, &H256::from(message_hash))?;

        Ok(GuiAuthValidation {
            coin_ticker: generator.coin_ticker,
            address: generator.address,
            timestamp_message,
            signature: format!("0x{}", signature),
        })
    }
}

fn validate_fee_impl(coin: EthCoin, validate_fee_args: EthValidateFeeArgs<'_>) -> ValidatePaymentFut<()> {
    let fee_tx_hash = validate_fee_args.fee_tx_hash.to_owned();
    let sender_addr = try_f!(
        addr_from_raw_pubkey(validate_fee_args.expected_sender).map_to_mm(ValidatePaymentError::InvalidParameter)
    );
    let fee_addr =
        try_f!(addr_from_raw_pubkey(validate_fee_args.fee_addr).map_to_mm(ValidatePaymentError::InvalidParameter));
    let amount = validate_fee_args.amount.clone();
    let min_block_number = validate_fee_args.min_block_number;

    let fut = async move {
        let expected_value = wei_from_big_decimal(&amount, coin.decimals)?;
        let tx_from_rpc = coin.web3.eth().transaction(TransactionId::Hash(fee_tx_hash)).await?;

        let tx_from_rpc = tx_from_rpc.as_ref().ok_or_else(|| {
            ValidatePaymentError::TxDoesNotExist(format!("Didn't find provided tx {:?} on ETH node", fee_tx_hash))
        })?;

        if tx_from_rpc.from != Some(sender_addr) {
            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                "{}: Fee tx {:?} was sent from wrong address, expected {:?}",
                INVALID_SENDER_ERR_LOG, tx_from_rpc, sender_addr
            )));
        }

        if let Some(block_number) = tx_from_rpc.block_number {
            if block_number <= min_block_number.into() {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "{}: Fee tx {:?} confirmed before min_block {}",
                    EARLY_CONFIRMATION_ERR_LOG, tx_from_rpc, min_block_number
                )));
            }
        }
        match &coin.coin_type {
            EthCoinType::Eth => {
                if tx_from_rpc.to != Some(fee_addr) {
                    return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                        "{}: Fee tx {:?} was sent to wrong address, expected {:?}",
                        INVALID_RECEIVER_ERR_LOG, tx_from_rpc, fee_addr
                    )));
                }

                if tx_from_rpc.value < expected_value {
                    return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                        "Fee tx {:?} value is less than expected {:?}",
                        tx_from_rpc, expected_value
                    )));
                }
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                if tx_from_rpc.to != Some(*token_addr) {
                    return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                        "{}: ERC20 Fee tx {:?} called wrong smart contract, expected {:?}",
                        INVALID_CONTRACT_ADDRESS_ERR_LOG, tx_from_rpc, token_addr
                    )));
                }

                let function = ERC20_CONTRACT
                    .function("transfer")
                    .map_to_mm(|e| ValidatePaymentError::InternalError(e.to_string()))?;
                let decoded_input = decode_contract_call(function, &tx_from_rpc.input.0)
                    .map_to_mm(|e| ValidatePaymentError::TxDeserializationError(e.to_string()))?;
                let address_input = get_function_input_data(&decoded_input, function, 0)
                    .map_to_mm(ValidatePaymentError::TxDeserializationError)?;

                if address_input != Token::Address(fee_addr) {
                    return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                        "{}: ERC20 Fee tx was sent to wrong address {:?}, expected {:?}",
                        INVALID_RECEIVER_ERR_LOG, address_input, fee_addr
                    )));
                }

                let value_input = get_function_input_data(&decoded_input, function, 1)
                    .map_to_mm(ValidatePaymentError::TxDeserializationError)?;

                match value_input {
                    Token::Uint(value) => {
                        if value < expected_value {
                            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                                "ERC20 Fee tx value {} is less than expected {}",
                                value, expected_value
                            )));
                        }
                    },
                    _ => {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Should have got uint token but got {:?}",
                            value_input
                        )))
                    },
                }
            },
        }

        Ok(())
    };
    Box::new(fut.boxed().compat())
}

fn get_function_input_data(decoded: &[Token], func: &Function, index: usize) -> Result<Token, String> {
    decoded.get(index).cloned().ok_or(format!(
        "Missing input in function {}: No input found at index {}",
        func.name.clone(),
        index
    ))
}

fn get_function_name(name: &str, watcher_reward: bool) -> String {
    if watcher_reward {
        format!("{}{}", name, "Reward")
    } else {
        name.to_owned()
    }
}

pub fn addr_from_raw_pubkey(pubkey: &[u8]) -> Result<Address, String> {
    let pubkey = try_s!(PublicKey::from_slice(pubkey).map_err(|e| ERRL!("{:?}", e)));
    let eth_public = Public::from_slice(&pubkey.serialize_uncompressed()[1..65]);
    Ok(public_to_address(&eth_public))
}

pub fn addr_from_pubkey_str(pubkey: &str) -> Result<String, String> {
    let pubkey_bytes = try_s!(hex::decode(pubkey));
    let addr = try_s!(addr_from_raw_pubkey(&pubkey_bytes));
    Ok(format!("{:#02x}", addr))
}

fn display_u256_with_decimal_point(number: U256, decimals: u8) -> String {
    let mut string = number.to_string();
    let decimals = decimals as usize;
    if string.len() <= decimals {
        string.insert_str(0, &"0".repeat(decimals - string.len() + 1));
    }

    string.insert(string.len() - decimals, '.');
    string.trim_end_matches('0').into()
}

pub fn u256_to_big_decimal(number: U256, decimals: u8) -> NumConversResult<BigDecimal> {
    let string = display_u256_with_decimal_point(number, decimals);
    Ok(string.parse::<BigDecimal>()?)
}

pub fn wei_from_big_decimal(amount: &BigDecimal, decimals: u8) -> NumConversResult<U256> {
    let mut amount = amount.to_string();
    let dot = amount.find(|c| c == '.');
    let decimals = decimals as usize;
    if let Some(index) = dot {
        let mut fractional = amount.split_off(index);
        // remove the dot from fractional part
        fractional.remove(0);
        if fractional.len() < decimals {
            fractional.insert_str(fractional.len(), &"0".repeat(decimals - fractional.len()));
        }
        fractional.truncate(decimals);
        amount.push_str(&fractional);
    } else {
        amount.insert_str(amount.len(), &"0".repeat(decimals));
    }
    U256::from_dec_str(&amount)
        .map_err(|e| format!("{:?}", e))
        .map_to_mm(NumConversError::new)
}

impl Transaction for SignedEthTx {
    fn tx_hex(&self) -> Vec<u8> { rlp::encode(self).to_vec() }

    fn tx_hash(&self) -> BytesJson { self.hash.0.to_vec().into() }
}

fn signed_tx_from_web3_tx(transaction: Web3Transaction) -> Result<SignedEthTx, String> {
    let r = transaction.r.ok_or_else(|| ERRL!("'Transaction::r' is not set"))?;
    let s = transaction.s.ok_or_else(|| ERRL!("'Transaction::s' is not set"))?;
    let v = transaction
        .v
        .ok_or_else(|| ERRL!("'Transaction::v' is not set"))?
        .as_u64();
    let gas_price = transaction
        .gas_price
        .ok_or_else(|| ERRL!("'Transaction::gas_price' is not set"))?;

    let unverified = UnverifiedTransaction {
        r,
        s,
        v,
        hash: transaction.hash,
        unsigned: UnSignedEthTx {
            data: transaction.input.0,
            gas_price,
            gas: transaction.gas,
            value: transaction.value,
            nonce: transaction.nonce,
            action: match transaction.to {
                Some(addr) => Action::Call(addr),
                None => Action::Create,
            },
        },
    };

    Ok(try_s!(SignedEthTx::new(unverified)))
}

#[derive(Deserialize, Debug, Serialize)]
pub struct GasStationData {
    // matic gas station average fees is named standard, using alias to support both format.
    #[serde(alias = "average", alias = "standard")]
    average: MmNumber,
    fast: MmNumber,
}

/// Using tagged representation to allow adding variants with coefficients, percentage, etc in the future.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "policy", content = "additional_data")]
pub enum GasStationPricePolicy {
    /// Use mean between average and fast values, default and recommended to use on ETH mainnet due to
    /// gas price big spikes.
    MeanAverageFast,
    /// Use average value only. Useful for non-heavily congested networks (Matic, etc.)
    Average,
}

impl Default for GasStationPricePolicy {
    fn default() -> Self { GasStationPricePolicy::MeanAverageFast }
}

impl GasStationData {
    fn average_gwei(&self, decimals: u8, gas_price_policy: GasStationPricePolicy) -> NumConversResult<U256> {
        let gas_price = match gas_price_policy {
            GasStationPricePolicy::MeanAverageFast => ((&self.average + &self.fast) / MmNumber::from(2)).into(),
            GasStationPricePolicy::Average => self.average.to_decimal(),
        };
        wei_from_big_decimal(&gas_price, decimals)
    }

    fn get_gas_price(uri: &str, decimals: u8, gas_price_policy: GasStationPricePolicy) -> Web3RpcFut<U256> {
        let uri = uri.to_owned();
        let fut = async move {
            make_gas_station_request(&uri)
                .await?
                .average_gwei(decimals, gas_price_policy)
                .mm_err(|e| Web3RpcError::Internal(e.0))
        };
        Box::new(fut.boxed().compat())
    }
}

async fn get_token_decimals(web3: &Web3<Web3Transport>, token_addr: Address) -> Result<u8, String> {
    let function = try_s!(ERC20_CONTRACT.function("decimals"));
    let data = try_s!(function.encode_input(&[]));
    let request = CallRequest {
        from: Some(Address::default()),
        to: Some(token_addr),
        gas: None,
        gas_price: None,
        value: Some(0.into()),
        data: Some(data.into()),
        ..CallRequest::default()
    };

    let res = web3
        .eth()
        .call(request, Some(BlockId::Number(BlockNumber::Latest)))
        .map_err(|e| ERRL!("{}", e))
        .await?;
    let tokens = try_s!(function.decode_output(&res.0));
    let decimals = match tokens[0] {
        Token::Uint(dec) => dec.as_u64(),
        _ => return ERR!("Invalid decimals type {:?}", tokens),
    };
    Ok(decimals as u8)
}

pub fn valid_addr_from_str(addr_str: &str) -> Result<Address, String> {
    let addr = try_s!(addr_from_str(addr_str));
    if !is_valid_checksum_addr(addr_str) {
        return ERR!("Invalid address checksum");
    }
    Ok(addr)
}

pub fn addr_from_str(addr_str: &str) -> Result<Address, String> {
    if !addr_str.starts_with("0x") {
        return ERR!("Address must be prefixed with 0x");
    };

    Ok(try_s!(Address::from_str(&addr_str[2..])))
}

/// This function fixes a bug appeared on `ethabi` update:
/// 1. `ethabi(6.1.0)::Function::decode_input` had
/// ```rust
/// decode(&self.input_param_types(), &data[4..])
/// ```
///
/// 2. `ethabi(17.2.0)::Function::decode_input` has
/// ```rust
/// decode(&self.input_param_types(), data)
/// ```
pub fn decode_contract_call(function: &Function, contract_call_bytes: &[u8]) -> Result<Vec<Token>, ethabi::Error> {
    if contract_call_bytes.len() < 4 {
        return Err(ethabi::Error::Other(
            "Contract call should contain at least 4 bytes known as a function signature".into(),
        ));
    }

    let actual_signature = &contract_call_bytes[..4];
    let expected_signature = &function.short_signature();
    if actual_signature != expected_signature {
        let error =
            format!("Unexpected contract call signature: expected {expected_signature:?}, found {actual_signature:?}");
        return Err(ethabi::Error::Other(error.into()));
    }

    function.decode_input(&contract_call_bytes[4..])
}

fn rpc_event_handlers_for_eth_transport(ctx: &MmArc, ticker: String) -> Vec<RpcTransportEventHandlerShared> {
    let metrics = ctx.metrics.weak();
    vec![CoinTransportMetrics::new(metrics, ticker, RpcClientType::Ethereum).into_shared()]
}

#[inline]
fn new_nonce_lock() -> Arc<AsyncMutex<()>> { Arc::new(AsyncMutex::new(())) }

pub async fn eth_coin_from_conf_and_request(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    req: &Json,
    protocol: CoinProtocol,
    priv_key_policy: PrivKeyBuildPolicy,
) -> Result<EthCoin, String> {
    // Convert `PrivKeyBuildPolicy` to `EthPrivKeyBuildPolicy` if it's possible.
    let priv_key_policy = try_s!(EthPrivKeyBuildPolicy::try_from(priv_key_policy));

    let mut urls: Vec<String> = try_s!(json::from_value(req["urls"].clone()));
    if urls.is_empty() {
        return ERR!("Enable request for ETH coin must have at least 1 node URL");
    }
    let mut rng = small_rng();
    urls.as_mut_slice().shuffle(&mut rng);

    let mut nodes = vec![];
    for url in urls.iter() {
        nodes.push(HttpTransportNode {
            uri: try_s!(url.parse()),
            gui_auth: false,
        });
    }
    drop_mutability!(nodes);

    let swap_contract_address: Address = try_s!(json::from_value(req["swap_contract_address"].clone()));
    if swap_contract_address == Address::default() {
        return ERR!("swap_contract_address can't be zero address");
    }
    let fallback_swap_contract: Option<Address> = try_s!(json::from_value(req["fallback_swap_contract"].clone()));
    if let Some(fallback) = fallback_swap_contract {
        if fallback == Address::default() {
            return ERR!("fallback_swap_contract can't be zero address");
        }
    }
    let contract_supports_watchers = req["contract_supports_watchers"].as_bool().unwrap_or_default();

    let (my_address, key_pair) = try_s!(build_address_and_priv_key_policy(conf, priv_key_policy).await);

    let mut web3_instances = vec![];
    let event_handlers = rpc_event_handlers_for_eth_transport(ctx, ticker.to_string());
    for node in nodes.iter() {
        let transport = Web3Transport::new_http(vec![node.clone()], event_handlers.clone());
        let web3 = Web3::new(transport);
        let version = match web3.web3().client_version().await {
            Ok(v) => v,
            Err(e) => {
                error!("Couldn't get client version for url {}: {}", node.uri, e);
                continue;
            },
        };
        web3_instances.push(Web3Instance {
            web3,
            is_parity: version.contains("Parity") || version.contains("parity"),
        })
    }

    if web3_instances.is_empty() {
        return ERR!("Failed to get client version for all urls");
    }

    let transport = Web3Transport::new_http(nodes, event_handlers);
    let web3 = Web3::new(transport);

    let (coin_type, decimals) = match protocol {
        CoinProtocol::ETH => (EthCoinType::Eth, ETH_DECIMALS),
        CoinProtocol::ERC20 {
            platform,
            contract_address,
        } => {
            let token_addr = try_s!(valid_addr_from_str(&contract_address));
            let decimals = match conf["decimals"].as_u64() {
                None | Some(0) => try_s!(get_token_decimals(&web3, token_addr).await),
                Some(d) => d as u8,
            };
            (EthCoinType::Erc20 { platform, token_addr }, decimals)
        },
        _ => return ERR!("Expect ETH or ERC20 protocol"),
    };

    // param from request should override the config
    let required_confirmations = req["required_confirmations"]
        .as_u64()
        .unwrap_or_else(|| {
            conf["required_confirmations"]
                .as_u64()
                .unwrap_or(DEFAULT_REQUIRED_CONFIRMATIONS as u64)
        })
        .into();

    if req["requires_notarization"].as_bool().is_some() {
        warn!("requires_notarization doesn't take any effect on ETH/ERC20 coins");
    }

    let sign_message_prefix: Option<String> = json::from_value(conf["sign_message_prefix"].clone()).unwrap_or(None);

    let initial_history_state = if req["tx_history"].as_bool().unwrap_or(false) {
        HistorySyncState::NotStarted
    } else {
        HistorySyncState::NotEnabled
    };

    let gas_station_decimals: Option<u8> = try_s!(json::from_value(req["gas_station_decimals"].clone()));
    let gas_station_policy: GasStationPricePolicy =
        json::from_value(req["gas_station_policy"].clone()).unwrap_or_default();

    let key_lock = match &coin_type {
        EthCoinType::Eth => String::from(ticker),
        EthCoinType::Erc20 { ref platform, .. } => String::from(platform),
    };

    let mut map = NONCE_LOCK.lock().unwrap();

    let nonce_lock = map.entry(key_lock).or_insert_with(new_nonce_lock).clone();

    // Create an abortable system linked to the `MmCtx` so if the context is stopped via `MmArc::stop`,
    // all spawned futures related to `ETH` coin will be aborted as well.
    let abortable_system = try_s!(ctx.abortable_system.create_subsystem());

    let coin = EthCoinImpl {
        priv_key_policy: key_pair,
        my_address,
        coin_type,
        sign_message_prefix,
        swap_contract_address,
        fallback_swap_contract,
        contract_supports_watchers,
        decimals,
        ticker: ticker.into(),
        gas_station_url: try_s!(json::from_value(req["gas_station_url"].clone())),
        gas_station_decimals: gas_station_decimals.unwrap_or(ETH_GAS_STATION_DECIMALS),
        gas_station_policy,
        web3,
        web3_instances,
        history_sync_state: Mutex::new(initial_history_state),
        ctx: ctx.weak(),
        required_confirmations,
        chain_id: conf["chain_id"].as_u64(),
        logs_block_range: conf["logs_block_range"].as_u64().unwrap_or(DEFAULT_LOGS_BLOCK_RANGE),
        nonce_lock,
        erc20_tokens_infos: Default::default(),
        abortable_system,
    };
    Ok(EthCoin(Arc::new(coin)))
}

/// Displays the address in mixed-case checksum form
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-55.md
fn checksum_address(addr: &str) -> String {
    let mut addr = addr.to_lowercase();
    if addr.starts_with("0x") {
        addr.replace_range(..2, "");
    }

    let mut hasher = Keccak256::default();
    hasher.update(&addr);
    let hash = hasher.finalize();
    let mut result: String = "0x".into();
    for (i, c) in addr.chars().enumerate() {
        if c.is_ascii_digit() {
            result.push(c);
        } else {
            // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-55.md#specification
            // Convert the address to hex, but if the ith digit is a letter (ie. it's one of abcdef)
            // print it in uppercase if the 4*ith bit of the hash of the lowercase hexadecimal
            // address is 1 otherwise print it in lowercase.
            if hash[i / 2] & (1 << (7 - 4 * (i % 2))) != 0 {
                result.push(c.to_ascii_uppercase());
            } else {
                result.push(c.to_ascii_lowercase());
            }
        }
    }

    result
}

/// Checks that input is valid mixed-case checksum form address
/// The input must be 0x prefixed hex string
fn is_valid_checksum_addr(addr: &str) -> bool { addr == checksum_address(addr) }

/// Requests the nonce from all available nodes and returns the highest nonce available with the list of nodes that returned the highest nonce.
/// Transactions will be sent using the nodes that returned the highest nonce.
#[cfg_attr(test, mockable)]
fn get_addr_nonce(
    addr: Address,
    web3s: Vec<Web3Instance>,
) -> Box<dyn Future<Item = (U256, Vec<Web3Instance>), Error = String> + Send> {
    let fut = async move {
        let mut errors: u32 = 0;
        loop {
            let (futures, web3s): (Vec<_>, Vec<_>) = web3s
                .iter()
                .map(|web3| {
                    if web3.is_parity {
                        let parity: ParityNonce<_> = web3.web3.api();
                        (Either::Left(parity.parity_next_nonce(addr)), web3.clone())
                    } else {
                        (
                            Either::Right(web3.web3.eth().transaction_count(addr, Some(BlockNumber::Pending))),
                            web3.clone(),
                        )
                    }
                })
                .unzip();

            let nonces: Vec<_> = join_all(futures)
                .await
                .into_iter()
                .zip(web3s.into_iter())
                .filter_map(|(nonce_res, web3)| match nonce_res {
                    Ok(n) => Some((n, web3)),
                    Err(e) => {
                        error!("Error getting nonce for addr {:?}: {}", addr, e);
                        None
                    },
                })
                .collect();
            if nonces.is_empty() {
                // all requests errored
                errors += 1;
                if errors > 5 {
                    return ERR!("Couldn't get nonce after 5 errored attempts, aborting");
                }
            } else {
                let max = nonces
                    .iter()
                    .map(|(n, _)| *n)
                    .max()
                    .expect("nonces should not be empty!");
                break Ok((
                    max,
                    nonces
                        .into_iter()
                        .filter_map(|(n, web3)| if n == max { Some(web3) } else { None })
                        .collect(),
                ));
            }
            Timer::sleep(1.).await
        }
    };
    Box::new(Box::pin(fut).compat())
}

fn increase_by_percent_one_gwei(num: U256, percent: u64) -> U256 {
    let one_gwei = U256::from(10u64.pow(9));
    let percent = (num / U256::from(100)) * U256::from(percent);
    if percent < one_gwei {
        num + one_gwei
    } else {
        num + percent
    }
}

fn increase_gas_price_by_stage(gas_price: U256, level: &FeeApproxStage) -> U256 {
    match level {
        FeeApproxStage::WithoutApprox => gas_price,
        FeeApproxStage::StartSwap => {
            increase_by_percent_one_gwei(gas_price, GAS_PRICE_APPROXIMATION_PERCENT_ON_START_SWAP)
        },
        FeeApproxStage::OrderIssue => {
            increase_by_percent_one_gwei(gas_price, GAS_PRICE_APPROXIMATION_PERCENT_ON_ORDER_ISSUE)
        },
        FeeApproxStage::TradePreimage => {
            increase_by_percent_one_gwei(gas_price, GAS_PRICE_APPROXIMATION_PERCENT_ON_TRADE_PREIMAGE)
        },
        FeeApproxStage::WatcherPreimage => {
            increase_by_percent_one_gwei(gas_price, GAS_PRICE_APPROXIMATION_PERCENT_ON_WATCHER_PREIMAGE)
        },
    }
}

#[derive(Clone, Debug, Deserialize, Display, PartialEq, Serialize)]
pub enum GetEthAddressError {
    PrivKeyPolicyNotAllowed(PrivKeyPolicyNotAllowed),
    EthActivationV2Error(EthActivationV2Error),
    Internal(String),
}

impl From<PrivKeyPolicyNotAllowed> for GetEthAddressError {
    fn from(e: PrivKeyPolicyNotAllowed) -> Self { GetEthAddressError::PrivKeyPolicyNotAllowed(e) }
}

impl From<EthActivationV2Error> for GetEthAddressError {
    fn from(e: EthActivationV2Error) -> Self { GetEthAddressError::EthActivationV2Error(e) }
}

impl From<CryptoCtxError> for GetEthAddressError {
    fn from(e: CryptoCtxError) -> Self { GetEthAddressError::Internal(e.to_string()) }
}

/// `get_eth_address` returns wallet address for coin with `ETH` protocol type.
pub async fn get_eth_address(ctx: &MmArc, ticker: &str) -> MmResult<MyWalletAddress, GetEthAddressError> {
    let priv_key_policy = PrivKeyBuildPolicy::detect_priv_key_policy(ctx)?;
    // Convert `PrivKeyBuildPolicy` to `EthPrivKeyBuildPolicy` if it's possible.
    let priv_key_policy = EthPrivKeyBuildPolicy::try_from(priv_key_policy)?;

    let (my_address, ..) = build_address_and_priv_key_policy(&ctx.conf, priv_key_policy).await?;
    let wallet_address = checksum_address(&format!("{:#02x}", my_address));

    Ok(MyWalletAddress {
        coin: ticker.to_owned(),
        wallet_address,
    })
}

#[derive(Display)]
pub enum GetValidEthWithdrawAddError {
    #[display(fmt = "My address {} and from address {} mismatch", my_address, from)]
    AddressMismatchError {
        my_address: String,
        from: String,
    },
    #[display(fmt = "{} coin doesn't support NFT withdrawing", coin)]
    CoinDoesntSupportNftWithdraw {
        coin: String,
    },
    InvalidAddress(String),
}

fn get_valid_nft_add_to_withdraw(
    coin_enum: MmCoinEnum,
    to: &str,
    token_add: &str,
) -> MmResult<(Address, Address, EthCoin), GetValidEthWithdrawAddError> {
    let eth_coin = match coin_enum {
        MmCoinEnum::EthCoin(eth_coin) => eth_coin,
        _ => {
            return MmError::err(GetValidEthWithdrawAddError::CoinDoesntSupportNftWithdraw {
                coin: coin_enum.ticker().to_owned(),
            })
        },
    };
    let to_addr = valid_addr_from_str(to).map_err(GetValidEthWithdrawAddError::InvalidAddress)?;
    let token_addr = addr_from_str(token_add).map_err(GetValidEthWithdrawAddError::InvalidAddress)?;
    Ok((to_addr, token_addr, eth_coin))
}

#[derive(Clone, Debug, Deserialize, Display, EnumFromStringify, PartialEq, Serialize)]
pub enum EthGasDetailsErr {
    #[display(fmt = "Invalid fee policy: {}", _0)]
    InvalidFeePolicy(String),
    #[from_stringify("NumConversError")]
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
    #[display(fmt = "Transport: {}", _0)]
    Transport(String),
}

impl From<web3::Error> for EthGasDetailsErr {
    fn from(e: web3::Error) -> Self { EthGasDetailsErr::from(Web3RpcError::from(e)) }
}

impl From<Web3RpcError> for EthGasDetailsErr {
    fn from(e: Web3RpcError) -> Self {
        match e {
            Web3RpcError::Transport(tr) | Web3RpcError::InvalidResponse(tr) => EthGasDetailsErr::Transport(tr),
            Web3RpcError::Internal(internal) | Web3RpcError::Timeout(internal) => EthGasDetailsErr::Internal(internal),
        }
    }
}

async fn get_eth_gas_details(
    eth_coin: &EthCoin,
    fee: Option<WithdrawFee>,
    eth_value: U256,
    data: Bytes,
    call_addr: Address,
    fungible_max: bool,
) -> MmResult<GasDetails, EthGasDetailsErr> {
    match fee {
        Some(WithdrawFee::EthGas { gas_price, gas }) => {
            let gas_price = wei_from_big_decimal(&gas_price, 9)?;
            Ok((gas.into(), gas_price))
        },
        Some(fee_policy) => {
            let error = format!("Expected 'EthGas' fee type, found {:?}", fee_policy);
            MmError::err(EthGasDetailsErr::InvalidFeePolicy(error))
        },
        None => {
            let gas_price = eth_coin.get_gas_price().compat().await?;
            // covering edge case by deducting the standard transfer fee when we want to max withdraw ETH
            let eth_value_for_estimate = if fungible_max && eth_coin.coin_type == EthCoinType::Eth {
                eth_value - gas_price * U256::from(21000)
            } else {
                eth_value
            };
            let estimate_gas_req = CallRequest {
                value: Some(eth_value_for_estimate),
                data: Some(data),
                from: Some(eth_coin.my_address),
                to: Some(call_addr),
                gas: None,
                // gas price must be supplied because some smart contracts base their
                // logic on gas price, e.g. TUSD: https://github.com/KomodoPlatform/atomicDEX-API/issues/643
                gas_price: Some(gas_price),
                ..CallRequest::default()
            };
            // TODO Note if the wallet's balance is insufficient to withdraw, then `estimate_gas` may fail with the `Exception` error.
            // TODO Ideally we should determine the case when we have the insufficient balance and return `WithdrawError::NotSufficientBalance`.
            let gas_limit = eth_coin.estimate_gas(estimate_gas_req).compat().await?;
            Ok((gas_limit, gas_price))
        },
    }
}
