/******************************************************************************
 * Copyright © 2023 Pampex LTD and TillyHK LTD                                *
 *                                                                            *
 * See the CONTRIBUTOR-LICENSE-AGREEMENT, COPYING, LICENSE-COPYRIGHT-NOTICE   *
 * and DEVELOPER-CERTIFICATE-OF-ORIGIN files in the LEGAL directory in        *
 * the top-level directory of this distribution for the individual copyright  *
 * holder information and the developer policies on copyright and licensing.  *
 *                                                                            *
 * Unless otherwise agreed in a custom licensing agreement, no part of the    *
 * Komodo DeFi Framework software, including this file may be copied, modified, propagated *
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
//  Copyright © 2023 Pampex LTD and TillyHK LTD. All rights reserved.
//
use self::wallet_connect::{send_transaction_with_walletconnect, WcEthTxParams};
use super::eth::Action::{Call, Create};
use super::watcher_common::{validate_watcher_reward, REWARD_GAS_AMOUNT};
use super::*;
use crate::coin_balance::{
    EnableCoinBalanceError, EnabledCoinBalanceParams, HDAccountBalance, HDAddressBalance, HDWalletBalance,
    HDWalletBalanceOps,
};
use crate::eth::eth_rpc::ETH_RPC_REQUEST_TIMEOUT;
use crate::eth::web3_transport::websocket_transport::{WebsocketTransport, WebsocketTransportNode};
use crate::hd_wallet::{
    DisplayAddress, HDAccountOps, HDCoinAddress, HDCoinWithdrawOps, HDConfirmAddress, HDPathAccountToAddressId,
    HDWalletCoinOps, HDXPubExtractor,
};
use crate::lp_price::get_base_price_in_rel;
use crate::nft::nft_errors::ParseContractTypeError;
use crate::nft::nft_structs::{
    ContractType, ConvertChain, NftInfo, TransactionNftDetails, WithdrawErc1155, WithdrawErc721,
};
use crate::nft::WithdrawNftResult;
use crate::rpc_command::account_balance::{AccountBalanceParams, AccountBalanceRpcOps, HDAccountBalanceResponse};
use crate::rpc_command::get_new_address::{
    GetNewAddressParams, GetNewAddressResponse, GetNewAddressRpcError, GetNewAddressRpcOps,
};
use crate::rpc_command::hd_account_balance_rpc_error::HDAccountBalanceRpcError;
use crate::rpc_command::init_account_balance::{InitAccountBalanceParams, InitAccountBalanceRpcOps};
use crate::rpc_command::init_create_account::{
    CreateAccountRpcError, CreateAccountState, CreateNewAccountParams, InitCreateAccountRpcOps,
};
use crate::rpc_command::init_scan_for_new_addresses::{
    InitScanAddressesRpcOps, ScanAddressesParams, ScanAddressesResponse,
};
use crate::rpc_command::init_withdraw::InitWithdrawCoin;
use crate::rpc_command::{
    account_balance, get_new_address, init_account_balance, init_create_account, init_scan_for_new_addresses,
};
use crate::{
    coin_balance, scan_for_new_addresses_impl, BalanceResult, CoinWithDerivationMethod, DerivationMethod, DexFee,
    Eip1559Ops, GasPriceRpcParam, MakerNftSwapOpsV2, ParseCoinAssocTypes, ParseNftAssocTypes, PrivKeyPolicy,
    RpcCommonOps, SendNftMakerPaymentArgs, SpendNftMakerPaymentArgs, ToBytes, ValidateNftMakerPaymentArgs,
    ValidateWatcherSpendInput, WatcherSpendType,
};
use async_trait::async_trait;
use bitcrypto::{dhash160, keccak256, ripemd160, sha256};
use common::custom_futures::repeatable::{Ready, Retry, RetryOnError};
use common::custom_futures::timeout::FutureTimerExt;
use common::executor::{
    abortable_queue::AbortableQueue, AbortSettings, AbortableSystem, AbortedError, SpawnAbortable, Timer,
};
use common::log::{debug, error, info, warn};
use common::number_type_casting::SafeTypeCastingNumbers;
use common::wait_until_sec;
use common::{now_sec, small_rng, DEX_FEE_ADDR_RAW_PUBKEY};
use crypto::privkey::key_pair_from_secret;
use crypto::{Bip44Chain, CryptoCtx, CryptoCtxError, GlobalHDAccountArc, KeyPairPolicy};
use derive_more::Display;
use enum_derives::EnumFromStringify;

use compatible_time::Instant;
use ethabi::{Contract, Token};
use ethcore_transaction::tx_builders::TxBuilderError;
use ethcore_transaction::{
    Action, TransactionWrapper, TransactionWrapperBuilder as UnSignedEthTxBuilder, UnverifiedEip1559Transaction,
    UnverifiedEip2930Transaction, UnverifiedLegacyTransaction, UnverifiedTransactionWrapper,
};
pub use ethcore_transaction::{SignedTransaction as SignedEthTx, TxType};
use ethereum_types::{Address, H160, H256, U256};
use ethkey::{public_to_address, sign, verify_address, KeyPair, Public, Signature};
use futures::compat::Future01CompatExt;
use futures::future::{join, join_all, select_ok, try_join_all, FutureExt, TryFutureExt};
use futures01::Future;
use http::Uri;
use kdf_walletconnect::{WalletConnectCtx, WalletConnectOps};
use mm2_core::mm_ctx::{MmArc, MmWeak};
use mm2_number::bigdecimal_custom::CheckedDivision;
use mm2_number::{BigDecimal, BigUint, MmNumber};
use num_traits::FromPrimitive;
use rand::seq::SliceRandom;
use regex::Regex;
use rlp::{DecoderError, Encodable, RlpStream};
use rpc::v1::types::Bytes as BytesJson;
use secp256k1::PublicKey;
use serde_json::{self as json, Value as Json};
use serialization::{CompactInteger, Serializable, Stream};
use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::ops::Deref;
use std::str::from_utf8;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use web3::types::{
    Action as TraceAction, BlockId, BlockNumber, Bytes, CallRequest, FilterBuilder, Log, Trace, TraceFilterBuilder,
    Transaction as Web3Transaction, TransactionId, U64,
};
use web3::{self, Web3};

cfg_wasm32! {
    use crypto::MetamaskArc;
    use mm2_metamask::MetamaskError;
    use web3::types::TransactionRequest;
}

use super::{
    coin_conf, lp_coinfind_or_err, AsyncMutex, BalanceError, BalanceFut, CheckIfMyPaymentSentArgs, CoinBalance,
    CoinProtocol, CoinTransportMetrics, CoinsContext, ConfirmPaymentInput, EthValidateFeeArgs, FeeApproxStage,
    FoundSwapTxSpend, HistorySyncState, IguanaPrivKey, MarketCoinOps, MmCoin, MmCoinEnum, MyAddressError,
    MyWalletAddress, NegotiateSwapContractAddrErr, NumConversError, NumConversResult, PaymentInstructionArgs,
    PaymentInstructions, PaymentInstructionsErr, PrivKeyBuildPolicy, PrivKeyPolicyNotAllowed, RawTransactionError,
    RawTransactionFut, RawTransactionRequest, RawTransactionRes, RawTransactionResult, RefundPaymentArgs, RewardTarget,
    RpcTransportEventHandler, RpcTransportEventHandlerShared, SearchForSwapTxSpendInput,
    SendMakerPaymentSpendPreimageInput, SendPaymentArgs, SignEthTransactionParams, SignRawTransactionEnum,
    SignRawTransactionRequest, SignatureError, SignatureResult, SpendPaymentArgs, SwapGasFeePolicy, SwapOps, TradeFee,
    TradePreimageError, TradePreimageFut, TradePreimageResult, TradePreimageValue, Transaction, TransactionDetails,
    TransactionEnum, TransactionErr, TransactionFut, TransactionType, TxMarshalingErr, UnexpectedDerivationMethod,
    ValidateAddressResult, ValidateFeeArgs, ValidateInstructionsErr, ValidateOtherPubKeyErr, ValidatePaymentError,
    ValidatePaymentFut, ValidatePaymentInput, VerificationError, VerificationResult, WaitForHTLCTxSpendArgs,
    WatcherOps, WatcherReward, WatcherRewardError, WatcherSearchForSwapTxSpendInput, WatcherValidatePaymentInput,
    WatcherValidateTakerFeeInput, WeakSpawner, WithdrawError, WithdrawFee, WithdrawFut, WithdrawRequest,
    WithdrawResult, EARLY_CONFIRMATION_ERR_LOG, INVALID_CONTRACT_ADDRESS_ERR_LOG, INVALID_PAYMENT_STATE_ERR_LOG,
    INVALID_RECEIVER_ERR_LOG, INVALID_SENDER_ERR_LOG, INVALID_SWAP_ID_ERR_LOG,
};

pub use eth_errors::{
    EthAssocTypesError, EthGasDetailsErr, EthNftAssocTypesError, GetEthAddressError, GetValidEthWithdrawAddError,
    Web3RpcError,
};

pub use eth_gas::EthTxFeeDetails;
pub(crate) use eth_gas::{EthGasLimit, EthGasLimitV2, ExtractGasLimit, PayForGasOption};
use eth_gas::{
    GasDetails, GasPriceAdjust, GAS_PRICE_APPROXIMATION_PERCENT_ON_ORDER_ISSUE,
    GAS_PRICE_APPROXIMATION_PERCENT_ON_START_SWAP, GAS_PRICE_APPROXIMATION_PERCENT_ON_TRADE_PREIMAGE,
    GAS_PRICE_APPROXIMATION_PERCENT_ON_WATCHER_PREIMAGE,
};

#[cfg(test)]
pub(crate) use eth_utils::display_u256_with_decimal_point;
use eth_utils::nonce_sequencer::PerNetNonceLocks;
pub use eth_utils::{
    addr_from_pubkey_str, addr_from_raw_pubkey, addr_from_str, checksum_address, get_eth_address, mm_number_from_u256,
    mm_number_to_u256, pubkey_from_extended, signed_eth_tx_from_bytes, u256_from_big_decimal, u256_to_big_decimal,
    valid_addr_from_str, wei_from_coins_mm_number, wei_from_gwei_decimal, wei_to_eth_decimal, wei_to_gwei_decimal,
};
use eth_utils::{
    calc_total_fee, call_request_with_pay_for_gas_option, eth_addr_to_hex, get_conf_param_or_from_plaform_coin,
    get_eth_gas_details_from_withdraw_fee, get_function_input_data, get_function_name, get_raw_transaction_impl,
    get_swap_fee_policy_for_estimate, get_tx_hex_by_hash_impl, get_valid_nft_addr_to_withdraw, increase_by_percent,
    increase_gas_price_by_stage, rpc_event_handlers_for_eth_transport, sign_and_send_transaction_with_keypair,
    sign_raw_eth_tx, tx_builder_with_pay_for_gas_option, validate_fee_impl, ESTIMATE_GAS_MULT, GAS_PRICE_ADJUST,
    MAX_ETH_TX_TYPE_SUPPORTED, SWAP_GAS_FEE_POLICY,
};
pub(crate) use eth_utils::{decode_contract_call, signed_tx_from_web3_tx};

#[cfg(target_arch = "wasm32")]
use eth_utils::sign_and_send_transaction_with_metamask;

pub use rlp;
cfg_native! {
    use std::path::PathBuf;
}

pub mod eth_errors;

pub mod eth_balance_events;
mod eth_rpc;
#[cfg(test)]
mod eth_tests;
#[cfg(target_arch = "wasm32")]
mod eth_wasm_tests;
#[cfg(any(test, feature = "for-tests"))]
mod for_tests;
pub(crate) mod nft_swap_v2;
pub mod wallet_connect;
mod web3_transport;
use web3_transport::{http_transport::HttpTransportNode, Web3Transport};

pub mod eth_gas;
pub mod eth_hd_wallet;
use eth_hd_wallet::EthHDWallet;

#[path = "eth/activation.rs"]
pub mod activation;
pub use activation::eth_coin_from_conf_and_request;

#[path = "eth/v2_activation.rs"]
pub mod v2_activation;
use v2_activation::{build_address_and_priv_key_policy, EthActivationV2Error};

mod eth_withdraw;
use eth_withdraw::withdraw_impl;
pub(crate) use eth_withdraw::{withdraw_erc1155, withdraw_erc721};

pub mod fee_estimation;
use fee_estimation::eip1559::{
    block_native::BlocknativeGasApiCaller, infura::InfuraGasApiCaller, simple::FeePerGasSimpleEstimator,
    FeePerGasEstimated, GasApiConfig, GasApiProvider, FEE_PRIORITY_LEVEL_N,
};

pub mod erc20;
use erc20::get_token_decimals;
pub(crate) mod eth_swap_v2;
use eth_swap_v2::{extract_id_from_tx_data, EthPaymentType, PaymentMethod, SpendTxSearchParams};

pub mod eth_watcher;

pub mod eth_utils;
pub mod tron;

pub const ETH_PROTOCOL_TYPE: &str = "ETH";
pub const ERC20_PROTOCOL_TYPE: &str = "ERC20";

/// https://github.com/artemii235/etomic-swap/blob/master/contracts/EtomicSwap.sol
/// Dev chain (195.201.137.5:8565) contract address: 0x83965C539899cC0F918552e5A26915de40ee8852
/// Ropsten: https://ropsten.etherscan.io/address/0x7bc1bbdd6a0a722fc9bffc49c921b685ecb84b94
/// ETH mainnet: https://etherscan.io/address/0x8500AFc0bc5214728082163326C2FF0C73f4a871
pub const SWAP_CONTRACT_ABI: &str = include_str!("eth/swap_contract_abi.json");
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-20.md
pub const ERC20_ABI: &str = include_str!("eth/erc20_abi.json");
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-721.md
const ERC721_ABI: &str = include_str!("eth/erc721_abi.json");
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-1155.md
const ERC1155_ABI: &str = include_str!("eth/erc1155_abi.json");
const NFT_SWAP_CONTRACT_ABI: &str = include_str!("eth/nft_swap_contract_abi.json");
const NFT_MAKER_SWAP_V2_ABI: &str = include_str!("eth/nft_maker_swap_v2_abi.json");
const MAKER_SWAP_V2_ABI: &str = include_str!("eth/maker_swap_v2_abi.json");
const TAKER_SWAP_V2_ABI: &str = include_str!("eth/taker_swap_v2_abi.json");

/// Payment states from etomic swap smart contract: https://github.com/artemii235/etomic-swap/blob/master/contracts/EtomicSwap.sol#L5
pub enum PaymentState {
    Uninitialized,
    Sent,
    Spent,
    Refunded,
}

#[allow(dead_code)]
pub(crate) enum MakerPaymentStateV2 {
    Uninitialized,
    PaymentSent,
    TakerSpent,
    MakerRefunded,
}

#[allow(dead_code)]
pub(crate) enum TakerPaymentStateV2 {
    Uninitialized,
    PaymentSent,
    TakerApproved,
    MakerSpent,
    TakerRefunded,
}

/// It can change 12.5% max each block according to https://www.blocknative.com/blog/eip-1559-fees
const BASE_BLOCK_FEE_DIFF_PCT: u64 = 13;
const DEFAULT_LOGS_BLOCK_RANGE: u64 = 1000;

const DEFAULT_REQUIRED_CONFIRMATIONS: u8 = 1;

pub(crate) const ETH_DECIMALS: u8 = 18;

pub(crate) const ETH_GWEI_DECIMALS: u8 = 9;

lazy_static! {
    pub static ref SWAP_CONTRACT: Contract = Contract::load(SWAP_CONTRACT_ABI.as_bytes()).unwrap();
    pub static ref MAKER_SWAP_V2: Contract = Contract::load(MAKER_SWAP_V2_ABI.as_bytes()).unwrap();
    pub static ref TAKER_SWAP_V2: Contract = Contract::load(TAKER_SWAP_V2_ABI.as_bytes()).unwrap();
    pub static ref ERC20_CONTRACT: Contract = Contract::load(ERC20_ABI.as_bytes()).unwrap();
    pub static ref ERC721_CONTRACT: Contract = Contract::load(ERC721_ABI.as_bytes()).unwrap();
    pub static ref ERC1155_CONTRACT: Contract = Contract::load(ERC1155_ABI.as_bytes()).unwrap();
    pub static ref NFT_SWAP_CONTRACT: Contract = Contract::load(NFT_SWAP_CONTRACT_ABI.as_bytes()).unwrap();
    pub static ref NFT_MAKER_SWAP_V2: Contract = Contract::load(NFT_MAKER_SWAP_V2_ABI.as_bytes()).unwrap();
}

pub type EthDerivationMethod = DerivationMethod<Address, EthHDWallet>;
pub type Web3RpcFut<T> = Box<dyn Future<Item = T, Error = MmError<Web3RpcError>> + Send>;
pub type Web3RpcResult<T> = Result<T, MmError<Web3RpcError>>;
type EthPrivKeyPolicy = PrivKeyPolicy<KeyPair>;

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

/// Specifies which blockchain the EthCoin operates on: EVM-compatible or TRON.
/// This distinction allows unified logic for EVM & TRON coins.
#[derive(Clone, Debug)]
pub enum ChainSpec {
    Evm { chain_id: u64 },
    Tron { network: tron::Network },
}

impl ChainSpec {
    pub fn chain_id(&self) -> Option<u64> {
        match self {
            ChainSpec::Evm { chain_id } => Some(*chain_id),
            ChainSpec::Tron { .. } => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            ChainSpec::Evm { .. } => "EVM",
            ChainSpec::Tron { .. } => "TRON",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EthCoinType {
    /// Ethereum itself or it's forks: ETC/others
    Eth,
    /// ERC20 token with smart contract address
    /// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-20.md
    Erc20 {
        platform: String,
        token_addr: Address,
    },
    Nft {
        platform: String,
    },
}

/// An alternative to `crate::PrivKeyBuildPolicy`, typical only for ETH coin.
pub enum EthPrivKeyBuildPolicy {
    IguanaPrivKey(IguanaPrivKey),
    GlobalHDAccount(GlobalHDAccountArc),
    #[cfg(target_arch = "wasm32")]
    Metamask(MetamaskArc),
    Trezor,
    WalletConnect {
        session_topic: kdf_walletconnect::WcTopic,
    },
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

impl From<PrivKeyBuildPolicy> for EthPrivKeyBuildPolicy {
    fn from(policy: PrivKeyBuildPolicy) -> EthPrivKeyBuildPolicy {
        match policy {
            PrivKeyBuildPolicy::IguanaPrivKey(iguana) => EthPrivKeyBuildPolicy::IguanaPrivKey(iguana),
            PrivKeyBuildPolicy::GlobalHDAccount(global_hd) => EthPrivKeyBuildPolicy::GlobalHDAccount(global_hd),
            PrivKeyBuildPolicy::Trezor => EthPrivKeyBuildPolicy::Trezor,
            PrivKeyBuildPolicy::WalletConnect { session_topic } => {
                EthPrivKeyBuildPolicy::WalletConnect { session_topic }
            },
        }
    }
}

/// pImpl idiom.
pub struct EthCoinImpl {
    ticker: String,
    pub coin_type: EthCoinType,
    /// Specifies the underlying blockchain (EVM or TRON).
    pub chain_spec: ChainSpec,
    pub(crate) priv_key_policy: EthPrivKeyPolicy,
    /// Either an Iguana address or a 'EthHDWallet' instance.
    /// Arc is used to use the same hd wallet from platform coin if we need to.
    /// This allows the reuse of the same derived accounts/addresses of the
    /// platform coin for tokens and vice versa.
    derivation_method: Arc<EthDerivationMethod>,
    sign_message_prefix: Option<String>,
    swap_contract_address: Address,
    swap_v2_contracts: Option<SwapV2Contracts>,
    fallback_swap_contract: Option<Address>,
    contract_supports_watchers: bool,
    web3_instances: AsyncMutex<Vec<Web3Instance>>,
    decimals: u8,
    history_sync_state: Mutex<HistorySyncState>,
    required_confirmations: AtomicU64,
    #[cfg_attr(any(test, feature = "run-docker-tests"), allow(dead_code))]
    swap_gas_fee_policy: Mutex<SwapGasFeePolicy>,
    max_eth_tx_type: Option<u64>,
    gas_price_adjust: Option<GasPriceAdjust>,
    /// Coin needs access to the context in order to reuse the logging and shutdown facilities.
    /// Using a weak reference by default in order to avoid circular references and leaks.
    pub ctx: MmWeak,
    /// The name of the coin with which Trezor wallet associates this asset.
    trezor_coin: Option<String>,
    /// the block range used for eth_getLogs
    logs_block_range: u64,
    /// A mapping of Ethereum addresses to their respective nonce locks.
    /// This is used to ensure that only one transaction is sent at a time per address.
    /// Each address is associated with an `AsyncMutex` which is locked when a transaction is being created and sent,
    /// and unlocked once the transaction is confirmed. This prevents nonce conflicts when multiple transactions
    /// are initiated concurrently from the same address.
    address_nonce_locks: PerNetNonceLocks,
    erc20_tokens_infos: Arc<Mutex<HashMap<String, Erc20TokenDetails>>>,
    /// Stores information about NFTs owned by the user. Each entry in the HashMap is uniquely identified by a composite key
    /// consisting of the token address and token ID, separated by a comma. This field is essential for tracking the NFT assets
    /// information (chain & contract type, amount etc.), where ownership and amount, in ERC1155 case, might change over time.
    pub nfts_infos: Arc<AsyncMutex<HashMap<String, NftInfo>>>,
    /// Config provided gas limits for swap and send transactions
    pub(crate) gas_limit: EthGasLimit,
    /// Config provided gas limits v2 for swap v2 transactions
    pub(crate) gas_limit_v2: EthGasLimitV2,
    /// If not None, gas limit is obtained from eth_estimateGas and multiplied by this value, for swap transactions
    #[allow(dead_code)] // TODO: remove allow
    pub(crate) estimate_gas_mult: Option<f64>,
    /// This spawner is used to spawn coin's related futures that should be aborted on coin deactivation
    /// and on [`MmArc::stop`].
    pub abortable_system: AbortableQueue,
}

#[derive(Clone, Debug)]
pub struct Web3Instance(Web3<Web3Transport>);

impl AsRef<Web3<Web3Transport>> for Web3Instance {
    fn as_ref(&self) -> &Web3<Web3Transport> {
        &self.0
    }
}

/// Information about a token that follows the ERC20 protocol on an EVM-based network.
#[derive(Clone, Debug)]
pub struct Erc20TokenDetails {
    /// The contract address of the token on the EVM-based network.
    pub token_address: Address,
    /// The number of decimal places the token uses.
    /// This represents the smallest unit that the token can be divided into.
    pub decimals: u8,
}

#[derive(Copy, Clone, Deserialize)]
pub struct SwapV2Contracts {
    pub maker_swap_v2_contract: Address,
    pub taker_swap_v2_contract: Address,
    pub nft_maker_swap_v2_contract: Address,
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

impl EthCoinImpl {
    #[cfg(not(target_arch = "wasm32"))]
    fn eth_traces_path(&self, ctx: &MmArc, my_address: Address) -> PathBuf {
        ctx.address_dir(&my_address.display_address())
            .join("TRANSACTIONS")
            .join(format!("{}_{:#02x}_trace.json", self.ticker, my_address))
    }

    /// Load saved ETH traces from local DB
    #[cfg(not(target_arch = "wasm32"))]
    fn load_saved_traces(&self, ctx: &MmArc, my_address: Address) -> Option<SavedTraces> {
        let path = self.eth_traces_path(ctx, my_address);
        let content = gstuff::slurp(&path);
        if content.is_empty() {
            None
        } else {
            json::from_slice(&content).ok()
        }
    }

    /// Load saved ETH traces from local DB
    #[cfg(target_arch = "wasm32")]
    fn load_saved_traces(&self, _ctx: &MmArc, _my_address: Address) -> Option<SavedTraces> {
        common::panic_w("'load_saved_traces' is not implemented in WASM");
        unreachable!()
    }

    /// Store ETH traces to local DB
    #[cfg(not(target_arch = "wasm32"))]
    fn store_eth_traces(&self, ctx: &MmArc, my_address: Address, traces: &SavedTraces) {
        let content = json::to_vec(traces).unwrap();
        let path = self.eth_traces_path(ctx, my_address);
        mm2_io::fs::write(&path, &content, true).unwrap();
    }

    /// Store ETH traces to local DB
    #[cfg(target_arch = "wasm32")]
    fn store_eth_traces(&self, _ctx: &MmArc, _my_address: Address, _traces: &SavedTraces) {
        common::panic_w("'store_eth_traces' is not implemented in WASM");
        unreachable!()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn erc20_events_path(&self, ctx: &MmArc, my_address: Address) -> PathBuf {
        ctx.address_dir(&my_address.display_address())
            .join("TRANSACTIONS")
            .join(format!("{}_{:#02x}_events.json", self.ticker, my_address))
    }

    /// Store ERC20 events to local DB
    #[cfg(not(target_arch = "wasm32"))]
    fn store_erc20_events(&self, ctx: &MmArc, my_address: Address, events: &SavedErc20Events) {
        let content = json::to_vec(events).unwrap();
        let path = self.erc20_events_path(ctx, my_address);
        mm2_io::fs::write(&path, &content, true).unwrap();
    }

    /// Store ERC20 events to local DB
    #[cfg(target_arch = "wasm32")]
    fn store_erc20_events(&self, _ctx: &MmArc, _my_address: Address, _events: &SavedErc20Events) {
        common::panic_w("'store_erc20_events' is not implemented in WASM");
        unreachable!()
    }

    /// Load saved ERC20 events from local DB
    #[cfg(not(target_arch = "wasm32"))]
    fn load_saved_erc20_events(&self, ctx: &MmArc, my_address: Address) -> Option<SavedErc20Events> {
        let path = self.erc20_events_path(ctx, my_address);
        let content = gstuff::slurp(&path);
        if content.is_empty() {
            None
        } else {
            json::from_slice(&content).ok()
        }
    }

    /// Load saved ERC20 events from local DB
    #[cfg(target_arch = "wasm32")]
    fn load_saved_erc20_events(&self, _ctx: &MmArc, _my_address: Address) -> Option<SavedErc20Events> {
        common::panic_w("'load_saved_erc20_events' is not implemented in WASM");
        unreachable!()
    }

    /// The id used to differentiate payments on Etomic swap smart contract
    pub(crate) fn etomic_swap_id(&self, time_lock: u32, secret_hash: &[u8]) -> Vec<u8> {
        let timelock_bytes = time_lock.to_le_bytes();
        self.generate_etomic_swap_id(&timelock_bytes, secret_hash)
    }

    /// The id used to differentiate payments on Etomic swap v2 smart contracts
    pub(crate) fn etomic_swap_id_v2(&self, time_lock: u64, secret_hash: &[u8]) -> Vec<u8> {
        let timelock_bytes = time_lock.to_le_bytes();
        self.generate_etomic_swap_id(&timelock_bytes, secret_hash)
    }

    fn generate_etomic_swap_id(&self, time_lock_bytes: &[u8], secret_hash: &[u8]) -> Vec<u8> {
        let mut input = Vec::with_capacity(time_lock_bytes.len() + secret_hash.len());
        input.extend_from_slice(time_lock_bytes);
        input.extend_from_slice(secret_hash);
        sha256(&input).to_vec()
    }

    /// Try to parse address from string.
    pub fn address_from_str(&self, address: &str) -> Result<Address, String> {
        Ok(try_s!(valid_addr_from_str(address)))
    }

    pub fn erc20_token_address(&self) -> Option<Address> {
        match self.coin_type {
            EthCoinType::Erc20 { token_addr, .. } => Some(token_addr),
            EthCoinType::Eth | EthCoinType::Nft { .. } => None,
        }
    }

    pub fn add_erc_token_info(&self, ticker: String, info: Erc20TokenDetails) {
        self.erc20_tokens_infos.lock().unwrap().insert(ticker, info);
    }

    /// # Warning
    /// Be very careful using this function since it returns dereferenced clone
    /// of value behind the MutexGuard and makes it non-thread-safe.
    pub fn get_erc_tokens_infos(&self) -> HashMap<String, Erc20TokenDetails> {
        let guard = self.erc20_tokens_infos.lock().unwrap();
        (*guard).clone()
    }

    #[inline(always)]
    pub fn chain_id(&self) -> Option<u64> {
        self.chain_spec.chain_id()
    }
}

#[derive(Clone)]
pub struct EthCoin(Arc<EthCoinImpl>);
impl Deref for EthCoin {
    type Target = EthCoinImpl;
    fn deref(&self) -> &EthCoinImpl {
        &self.0
    }
}

#[async_trait]
impl SwapOps for EthCoin {
    async fn send_taker_fee(&self, dex_fee: DexFee, _uuid: &[u8], _expire_at: u64) -> TransactionResult {
        let address = try_tx_s!(addr_from_raw_pubkey(self.dex_pubkey()));
        self.send_to_address(
            address,
            try_tx_s!(u256_from_big_decimal(&dex_fee.fee_amount().into(), self.decimals)),
        )
        .map(TransactionEnum::from)
        .compat()
        .await
    }

    async fn send_maker_payment(&self, maker_payment_args: SendPaymentArgs<'_>) -> TransactionResult {
        self.send_hash_time_locked_payment(maker_payment_args)
            .compat()
            .await
            .map(TransactionEnum::from)
    }

    async fn send_taker_payment(&self, taker_payment_args: SendPaymentArgs<'_>) -> TransactionResult {
        self.send_hash_time_locked_payment(taker_payment_args)
            .map(TransactionEnum::from)
            .compat()
            .await
    }

    async fn send_maker_spends_taker_payment(
        &self,
        maker_spends_payment_args: SpendPaymentArgs<'_>,
    ) -> TransactionResult {
        self.spend_hash_time_locked_payment(maker_spends_payment_args)
            .await
            .map(TransactionEnum::from)
    }

    async fn send_taker_spends_maker_payment(
        &self,
        taker_spends_payment_args: SpendPaymentArgs<'_>,
    ) -> TransactionResult {
        self.spend_hash_time_locked_payment(taker_spends_payment_args)
            .await
            .map(TransactionEnum::from)
    }

    async fn send_taker_refunds_payment(&self, taker_refunds_payment_args: RefundPaymentArgs<'_>) -> TransactionResult {
        self.refund_hash_time_locked_payment(taker_refunds_payment_args)
            .await
            .map(TransactionEnum::from)
    }

    async fn send_maker_refunds_payment(&self, maker_refunds_payment_args: RefundPaymentArgs<'_>) -> TransactionResult {
        self.refund_hash_time_locked_payment(maker_refunds_payment_args)
            .await
            .map(TransactionEnum::from)
    }

    async fn validate_fee(&self, validate_fee_args: ValidateFeeArgs<'_>) -> ValidatePaymentResult<()> {
        let tx = match validate_fee_args.fee_tx {
            TransactionEnum::SignedEthTx(t) => t.clone(),
            fee_tx => {
                return MmError::err(ValidatePaymentError::InternalError(format!(
                    "Invalid fee tx type. fee tx: {fee_tx:?}"
                )))
            },
        };
        validate_fee_impl(
            self.clone(),
            EthValidateFeeArgs {
                fee_tx_hash: &tx.tx_hash(),
                expected_sender: validate_fee_args.expected_sender,
                amount: &validate_fee_args.dex_fee.fee_amount().into(),
                min_block_number: validate_fee_args.min_block_number,
                uuid: validate_fee_args.uuid,
            },
        )
        .compat()
        .await
    }

    #[inline]
    async fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentResult<()> {
        self.validate_payment(input).compat().await
    }

    #[inline]
    async fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentResult<()> {
        self.validate_payment(input).compat().await
    }

    async fn check_if_my_payment_sent(
        &self,
        if_my_payment_sent_args: CheckIfMyPaymentSentArgs<'_>,
    ) -> Result<Option<TransactionEnum>, String> {
        let time_lock = if_my_payment_sent_args
            .time_lock
            .try_into()
            .map_err(|e: TryFromIntError| e.to_string())?;
        let id = self.etomic_swap_id(time_lock, if_my_payment_sent_args.secret_hash);
        let swap_contract_address = if_my_payment_sent_args.swap_contract_address.try_to_address()?;
        let from_block = if_my_payment_sent_args.search_from_block;
        let status = self
            .payment_status(swap_contract_address, Token::FixedBytes(id.clone()))
            .compat()
            .await?;

        if status == U256::from(PaymentState::Uninitialized as u8) {
            return Ok(None);
        };

        let mut current_block = self.current_block().compat().await?;
        if current_block < from_block {
            current_block = from_block;
        }

        let mut from_block = from_block;

        loop {
            let to_block = current_block.min(from_block + self.logs_block_range);

            let events = self
                .payment_sent_events(swap_contract_address, from_block, to_block)
                .compat()
                .await?;

            let found = events.iter().find(|event| &event.data.0[..32] == id.as_slice());

            match found {
                Some(event) => {
                    let transaction = try_s!(
                        self.transaction(TransactionId::Hash(event.transaction_hash.unwrap()))
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

    async fn extract_secret(
        &self,
        _secret_hash: &[u8],
        spend_tx: &[u8],
        watcher_reward: bool,
    ) -> Result<[u8; 32], String> {
        let unverified: UnverifiedTransactionWrapper = try_s!(rlp::decode(spend_tx));
        let function_name = get_function_name("receiverSpend", watcher_reward);
        let function = try_s!(SWAP_CONTRACT.function(&function_name));

        // Validate contract call; expected to be receiverSpend.
        // https://www.4byte.directory/signatures/?bytes4_signature=02ed292b.
        let expected_signature = function.short_signature();
        let actual_signature = &unverified.unsigned().data()[0..4];
        if actual_signature != expected_signature {
            return ERR!(
                "Expected 'receiverSpend' contract call signature: {:?}, found {:?}",
                expected_signature,
                actual_signature
            );
        };

        let tokens = try_s!(decode_contract_call(function, unverified.unsigned().data()));
        if tokens.len() < 3 {
            return ERR!("Invalid arguments in 'receiverSpend' call: {:?}", tokens);
        }
        match &tokens[2] {
            Token::FixedBytes(secret) => Ok(try_s!(secret.as_slice().try_into())),
            _ => ERR!(
                "Expected secret to be fixed bytes, decoded function data is {:?}",
                tokens
            ),
        }
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
            EthPrivKeyPolicy::Iguana(ref key_pair)
            | EthPrivKeyPolicy::HDWallet {
                activated_key: ref key_pair,
                ..
            } => key_pair_from_secret(key_pair.secret().as_fixed_bytes()).expect("valid key"),
            EthPrivKeyPolicy::Trezor | EthPrivKeyPolicy::WalletConnect { .. } => todo!(),
            #[cfg(target_arch = "wasm32")]
            EthPrivKeyPolicy::Metamask(_) => todo!(),
        }
    }

    #[inline]
    fn derive_htlc_pubkey(&self, _swap_unique_data: &[u8]) -> [u8; 33] {
        match self.priv_key_policy {
            EthPrivKeyPolicy::Iguana(ref key_pair)
            | EthPrivKeyPolicy::HDWallet {
                activated_key: ref key_pair,
                ..
            } => key_pair_from_secret(&key_pair.secret().to_fixed_bytes())
                .expect("valid key")
                .public_slice()
                .try_into()
                .expect("valid key length!"),
            EthPrivKeyPolicy::WalletConnect { public_key, .. } => public_key.into(),
            EthPrivKeyPolicy::Trezor => todo!(),
            #[cfg(target_arch = "wasm32")]
            EthPrivKeyPolicy::Metamask(ref metamask_policy) => metamask_policy.public_key.0,
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
        std::env::var("USE_WATCHER_REWARD").is_ok()
        //self.contract_supports_watchers
    }
}

#[async_trait]
#[cfg_attr(test, mockable)]
impl MarketCoinOps for EthCoin {
    fn ticker(&self) -> &str {
        &self.ticker[..]
    }

    fn my_address(&self) -> MmResult<String, MyAddressError> {
        match self.derivation_method() {
            DerivationMethod::SingleAddress(my_address) => Ok(my_address.display_address()),
            DerivationMethod::HDWallet(_) => MmError::err(MyAddressError::UnexpectedDerivationMethod(
                "'my_address' is deprecated for HD wallets".to_string(),
            )),
        }
    }

    fn address_from_pubkey(&self, pubkey: &H264Json) -> MmResult<String, AddressFromPubkeyError> {
        let addr = addr_from_raw_pubkey(&pubkey.0).map_err(AddressFromPubkeyError::InternalError)?;
        Ok(addr.display_address())
    }

    async fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> {
        match self.priv_key_policy {
            EthPrivKeyPolicy::Iguana(ref key_pair)
            | EthPrivKeyPolicy::HDWallet {
                activated_key: ref key_pair,
                ..
            } => {
                let uncompressed_without_prefix = hex::encode(key_pair.public());
                Ok(format!("04{uncompressed_without_prefix}"))
            },
            EthPrivKeyPolicy::Trezor => {
                let public_key = self
                    .deref()
                    .derivation_method
                    .hd_wallet()
                    .ok_or(UnexpectedDerivationMethod::ExpectedHDWallet)?
                    .get_enabled_address()
                    .await
                    .ok_or_else(|| UnexpectedDerivationMethod::InternalError("no enabled address".to_owned()))?
                    .pubkey();
                let uncompressed_without_prefix = hex::encode(public_key);
                Ok(format!("04{uncompressed_without_prefix}"))
            },
            #[cfg(target_arch = "wasm32")]
            EthPrivKeyPolicy::Metamask(ref metamask_policy) => {
                Ok(format!("{:02x}", metamask_policy.public_key_uncompressed))
            },
            EthPrivKeyPolicy::WalletConnect {
                public_key_uncompressed,
                ..
            } => Ok(format!("{public_key_uncompressed:02x}")),
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

    fn sign_message(&self, message: &str, address: Option<HDAddressSelector>) -> SignatureResult<String> {
        let message_hash = self.sign_message_hash(message).ok_or(SignatureError::PrefixNotFound)?;

        let secret = if let Some(address) = address {
            let path_to_coin = self.priv_key_policy.path_to_coin_or_err().map_mm_err()?;
            let derivation_path = address
                .valid_derivation_path(path_to_coin)
                .mm_err(|err| SignatureError::InvalidRequest(err.to_string()))
                .map_mm_err()?;
            let privkey = self
                .priv_key_policy
                .hd_wallet_derived_priv_key_or_err(&derivation_path)
                .map_mm_err()?;
            ethkey::Secret::from_slice(privkey.as_slice()).ok_or(MmError::new(SignatureError::InternalError(
                "failed to derive ethkey::Secret".to_string(),
            )))?
        } else {
            self.priv_key_policy
                .activated_key_or_err()
                .map_mm_err()?
                .secret()
                .clone()
        };
        let signature = sign(&secret, &H256::from(message_hash))?;

        Ok(format!("0x{signature}"))
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
            .get_balance()
            .and_then(move |result| u256_to_big_decimal(result, decimals).map_mm_err())
            .map(|spendable| CoinBalance {
                spendable,
                unspendable: BigDecimal::from(0),
            });
        Box::new(fut)
    }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> {
        Box::new(
            self.eth_balance()
                .and_then(move |result| u256_to_big_decimal(result, ETH_DECIMALS).map_mm_err()),
        )
    }

    fn platform_ticker(&self) -> &str {
        match &self.coin_type {
            EthCoinType::Eth => self.ticker(),
            EthCoinType::Erc20 { platform, .. } | EthCoinType::Nft { platform } => platform,
        }
    }

    fn send_raw_tx(&self, mut tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
        if tx.starts_with("0x") {
            tx = &tx[2..];
        }
        let bytes = try_fus!(hex::decode(tx));

        let coin = self.clone();

        let fut = async move {
            coin.send_raw_transaction(bytes.into())
                .await
                .map(|res| format!("{res:02x}")) // TODO: add 0x hash (use unified hash format for eth wherever it is returned)
                .map_err(|e| ERRL!("{}", e))
        };

        Box::new(fut.boxed().compat())
    }

    fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> {
        let coin = self.clone();

        let tx = tx.to_owned();
        let fut = async move {
            coin.send_raw_transaction(tx.into())
                .await
                .map(|res| format!("{res:02x}"))
                .map_err(|e| ERRL!("{}", e))
        };

        Box::new(fut.boxed().compat())
    }

    async fn sign_raw_tx(&self, args: &SignRawTransactionRequest) -> RawTransactionResult {
        if let SignRawTransactionEnum::ETH(eth_args) = &args.tx {
            sign_raw_eth_tx(self, eth_args).await
        } else {
            MmError::err(RawTransactionError::InvalidParam("eth type expected".to_string()))
        }
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

        let unsigned: UnverifiedTransactionWrapper = try_fus!(rlp::decode(&input.payment_tx));
        let tx = try_fus!(SignedEthTx::new(unsigned));
        let tx_hash = tx.tx_hash();

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

    async fn wait_for_htlc_tx_spend(&self, args: WaitForHTLCTxSpendArgs<'_>) -> TransactionResult {
        let unverified: UnverifiedTransactionWrapper = try_tx_s!(rlp::decode(args.tx_bytes));
        let tx = try_tx_s!(SignedEthTx::new(unverified));

        let swap_contract_address = match args.swap_contract_address {
            Some(addr) => try_tx_s!(addr.try_to_address()),
            None => match tx.unsigned().action() {
                Call(address) => *address,
                Create => {
                    return Err(TransactionErr::Plain(ERRL!(
                        "Invalid payment action: the payment action cannot be create"
                    )))
                },
            },
        };

        let func_name = match self.coin_type {
            EthCoinType::Eth => get_function_name("ethPayment", args.watcher_reward),
            EthCoinType::Erc20 { .. } => get_function_name("erc20Payment", args.watcher_reward),
            EthCoinType::Nft { .. } => {
                return Err(TransactionErr::ProtocolNotSupported(ERRL!(
                    "Nft Protocol is not supported yet!"
                )))
            },
        };

        let id = try_tx_s!(extract_id_from_tx_data(tx.unsigned().data(), &SWAP_CONTRACT, &func_name).await);

        let find_params = SpendTxSearchParams {
            swap_contract_address,
            event_name: "ReceiverSpent",
            abi_contract: &SWAP_CONTRACT,
            swap_id: &try_tx_s!(id.as_slice().try_into()),
            from_block: args.from_block,
            wait_until: args.wait_until,
            check_every: args.check_every,
        };
        let tx_hash = self
            .find_transaction_hash_by_event(find_params)
            .await
            .map_err(|e| TransactionErr::Plain(e.get_inner().to_string()))?;

        let spend_tx = self
            .wait_for_transaction(tx_hash, args.wait_until, args.check_every)
            .await
            .map_err(|e| TransactionErr::Plain(e.get_inner().to_string()))?;
        Ok(TransactionEnum::from(spend_tx))
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        signed_eth_tx_from_bytes(bytes)
            .map(TransactionEnum::from)
            .map_to_mm(TxMarshalingErr::InvalidInput)
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        let coin = self.clone();

        let fut = async move {
            coin.block_number()
                .await
                .map(|res| res.as_u64())
                .map_err(|e| ERRL!("{}", e))
        };

        Box::new(fut.boxed().compat())
    }

    fn display_priv_key(&self) -> Result<String, String> {
        match self.priv_key_policy {
            EthPrivKeyPolicy::Iguana(ref key_pair)
            | EthPrivKeyPolicy::HDWallet {
                activated_key: ref key_pair,
                ..
            } => Ok(format!("{:#02x}", key_pair.secret())),
            EthPrivKeyPolicy::Trezor => ERR!("'display_priv_key' is not supported for Hardware Wallets"),
            #[cfg(target_arch = "wasm32")]
            EthPrivKeyPolicy::Metamask(_) => ERR!("'display_priv_key' is not supported for MetaMask"),
            EthPrivKeyPolicy::WalletConnect { .. } => ERR!("'display_priv_key' is not supported for WalletConnect"),
        }
    }

    #[inline]
    fn min_tx_amount(&self) -> BigDecimal {
        BigDecimal::from(0)
    }

    #[inline]
    fn min_trading_vol(&self) -> MmNumber {
        let pow = self.decimals as u32;
        MmNumber::from(1) / MmNumber::from(10u64.pow(pow))
    }

    #[inline]
    fn should_burn_dex_fee(&self) -> bool {
        false
    }

    fn is_trezor(&self) -> bool {
        self.priv_key_policy.is_trezor()
    }
}

type EthTxFut = Box<dyn Future<Item = SignedEthTx, Error = TransactionErr> + Send + 'static>;

#[async_trait]
impl RpcCommonOps for EthCoin {
    type RpcClient = Web3Instance;
    type Error = Web3RpcError;

    async fn get_live_client(&self) -> Result<Self::RpcClient, Self::Error> {
        let mut clients = self.web3_instances.lock().await;

        // try to find first live client
        for (i, client) in clients.clone().into_iter().enumerate() {
            if let Web3Transport::Websocket(socket_transport) = client.as_ref().transport() {
                socket_transport.maybe_spawn_connection_loop(self.clone());
            };

            if !client.as_ref().transport().is_last_request_failed() {
                // Bring the live client to the front of rpc_clients
                clients.rotate_left(i);
                return Ok(client);
            }

            match client
                .as_ref()
                .web3()
                .client_version()
                .timeout(ETH_RPC_REQUEST_TIMEOUT)
                .await
            {
                Ok(Ok(_)) => {
                    // Bring the live client to the front of rpc_clients
                    clients.rotate_left(i);
                    return Ok(client);
                },
                Ok(Err(rpc_error)) => {
                    debug!("Could not get client version on: {:?}. Error: {}", &client, rpc_error);

                    if let Web3Transport::Websocket(socket_transport) = client.as_ref().transport() {
                        socket_transport.stop_connection_loop().await;
                    };
                },
                Err(timeout_error) => {
                    debug!(
                        "Client version timeout exceed on: {:?}. Error: {}",
                        &client, timeout_error
                    );

                    if let Web3Transport::Websocket(socket_transport) = client.as_ref().transport() {
                        socket_transport.stop_connection_loop().await;
                    };
                },
            };
        }

        return Err(Web3RpcError::Transport(
            "All the current rpc nodes are unavailable.".to_string(),
        ));
    }
}

impl EthCoin {
    pub(crate) async fn web3(&self) -> Result<Web3<Web3Transport>, Web3RpcError> {
        self.get_live_client().await.map(|t| t.0)
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

        let coin = self.clone();

        let fut = async move { coin.logs(filter).await.map_err(|e| ERRL!("{}", e)) };

        Box::new(fut.boxed().compat())
    }

    /// Gets ETH traces from ETH node between addresses in `from_block` and `to_block`
    async fn eth_traces(
        &self,
        from_addr: Vec<Address>,
        to_addr: Vec<Address>,
        from_block: BlockNumber,
        to_block: BlockNumber,
        limit: Option<usize>,
    ) -> Web3RpcResult<Vec<Trace>> {
        let mut filter = TraceFilterBuilder::default()
            .from_address(from_addr)
            .to_address(to_addr)
            .from_block(from_block)
            .to_block(to_block);
        if let Some(l) = limit {
            filter = filter.count(l);
        }
        drop_mutability!(filter);

        self.trace_filter(filter.build()).await.map_to_mm(Web3RpcError::from)
    }

    /// Gets Transfer events from ERC20 smart contract `addr` between `from_block` and `to_block`
    async fn erc20_transfer_events(
        &self,
        contract: Address,
        from_addr: Option<Address>,
        to_addr: Option<Address>,
        from_block: BlockNumber,
        to_block: BlockNumber,
        limit: Option<usize>,
    ) -> Web3RpcResult<Vec<Log>> {
        let contract_event = ERC20_CONTRACT.event("Transfer")?;
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
        drop_mutability!(filter);

        self.logs(filter.build()).await.map_to_mm(Web3RpcError::from)
    }

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

        let my_address = match self.derivation_method.single_addr_or_err().await {
            Ok(addr) => addr,
            Err(e) => {
                ctx.log.log(
                    "",
                    &[&"tx_history", &self.ticker],
                    &ERRL!("Error on getting my address: {}", e),
                );
                return;
            },
        };
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

            let current_block = match self.block_number().await {
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

            let mut saved_traces = match self.load_saved_traces(ctx, my_address) {
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
                        vec![my_address],
                        vec![],
                        BlockNumber::Number(before_earliest),
                        BlockNumber::Number(saved_traces.earliest_block),
                        None,
                    )
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
                        vec![my_address],
                        BlockNumber::Number(before_earliest),
                        BlockNumber::Number(saved_traces.earliest_block),
                        None,
                    )
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
                self.store_eth_traces(ctx, my_address, &saved_traces);
            }

            if current_block > saved_traces.latest_block {
                let from_traces_after_latest = match self
                    .eth_traces(
                        vec![my_address],
                        vec![],
                        BlockNumber::Number(saved_traces.latest_block + 1),
                        BlockNumber::Number(current_block),
                        None,
                    )
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
                        vec![my_address],
                        BlockNumber::Number(saved_traces.latest_block + 1),
                        BlockNumber::Number(current_block),
                        None,
                    )
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

                self.store_eth_traces(ctx, my_address, &saved_traces);
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

                let receipt = match self.transaction_receipt(trace.transaction_hash.unwrap()).await {
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
                    EthCoinType::Nft { .. } => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Error on getting fee coin: Nft Protocol is not supported yet!"),
                        );
                        continue;
                    },
                };
                let fee_details: Option<EthTxFeeDetails> = match receipt {
                    Some(r) => {
                        let gas_used = r.gas_used.unwrap_or_default();
                        let gas_price = web3_tx.gas_price.unwrap_or_default();
                        // TODO: create and use EthTxFeeDetails::from(web3_tx)
                        // It's relatively safe to unwrap `EthTxFeeDetails::new` as it may fail due to `u256_to_big_decimal` only.
                        // Also TX history is not used by any GUI and has significant disadvantages.
                        Some(EthTxFeeDetails::new(gas_used, PayForGasOption::Legacy { gas_price }, fee_coin).unwrap())
                    },
                    None => None,
                };

                let total_amount: BigDecimal = u256_to_big_decimal(call_data.value, ETH_DECIMALS).unwrap();
                let mut received_by_me = 0.into();
                let mut spent_by_me = 0.into();

                if call_data.from == my_address {
                    // ETH transfer is actually happening only if no error occurred
                    if trace.error.is_none() {
                        spent_by_me = total_amount.clone();
                    }
                    if let Some(ref fee) = fee_details {
                        spent_by_me += &fee.total_fee;
                    }
                }

                if call_data.to == my_address {
                    // ETH transfer is actually happening only if no error occurred
                    if trace.error.is_none() {
                        received_by_me = total_amount.clone();
                    }
                }

                let raw = signed_tx_from_web3_tx(web3_tx).unwrap();
                let block = match self
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
                    to: vec![call_data.to.display_address()],
                    from: vec![call_data.from.display_address()],
                    coin: self.ticker.clone(),
                    fee_details: fee_details.map(|d| d.into()),
                    block_height: trace.block_number,
                    tx: TransactionData::new_signed(
                        BytesJson(rlp::encode(&raw).to_vec()),
                        format!("{:02x}", BytesJson(raw.tx_hash_as_bytes().to_vec())),
                    ),
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

        let my_address = match self.derivation_method.single_addr_or_err().await {
            Ok(addr) => addr,
            Err(e) => {
                ctx.log.log(
                    "",
                    &[&"tx_history", &self.ticker],
                    &ERRL!("Error on getting my address: {}", e),
                );
                return;
            },
        };
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

            let current_block = match self.block_number().await {
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

            let mut saved_events = match self.load_saved_erc20_events(ctx, my_address) {
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
                        Some(my_address),
                        None,
                        BlockNumber::Number(before_earliest),
                        BlockNumber::Number(saved_events.earliest_block - 1),
                        None,
                    )
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
                        Some(my_address),
                        BlockNumber::Number(before_earliest),
                        BlockNumber::Number(saved_events.earliest_block - 1),
                        None,
                    )
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
                self.store_erc20_events(ctx, my_address, &saved_events);
            }

            if current_block > saved_events.latest_block {
                let from_events_after_latest = match self
                    .erc20_transfer_events(
                        token_addr,
                        Some(my_address),
                        None,
                        BlockNumber::Number(saved_events.latest_block + 1),
                        BlockNumber::Number(current_block),
                        None,
                    )
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
                        Some(my_address),
                        BlockNumber::Number(saved_events.latest_block + 1),
                        BlockNumber::Number(current_block),
                        None,
                    )
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
                self.store_erc20_events(ctx, my_address, &saved_events);
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

                if from_addr == my_address {
                    spent_by_me = total_amount.clone();
                }

                if to_addr == my_address {
                    received_by_me = total_amount.clone();
                }

                mm_counter!(ctx.metrics, "tx.history.request.count", 1,
                    "coin" => self.ticker.clone(), "client" => "ethereum", "method" => "tx_detail_by_hash");

                let web3_tx = match self
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

                let receipt = match self.transaction_receipt(event.transaction_hash.unwrap()).await {
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
                    EthCoinType::Nft { .. } => {
                        ctx.log.log(
                            "",
                            &[&"tx_history", &self.ticker],
                            &ERRL!("Error on getting fee coin: Nft Protocol is not supported yet!"),
                        );
                        continue;
                    },
                };
                let fee_details = match receipt {
                    Some(r) => {
                        let gas_used = r.gas_used.unwrap_or_default();
                        let gas_price = web3_tx.gas_price.unwrap_or_default();
                        // It's relatively safe to unwrap `EthTxFeeDetails::new` as it may fail
                        // due to `u256_to_big_decimal` only.
                        // Also TX history is not used by any GUI and has significant disadvantages.
                        Some(EthTxFeeDetails::new(gas_used, PayForGasOption::Legacy { gas_price }, fee_coin).unwrap())
                    },
                    None => None,
                };
                let block_number = event.block_number.unwrap();
                let block = match self.block(BlockId::Number(BlockNumber::Number(block_number))).await {
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
                    to: vec![to_addr.display_address()],
                    from: vec![from_addr.display_address()],
                    coin: self.ticker.clone(),
                    fee_details: fee_details.map(|d| d.into()),
                    block_height: block_number.as_u64(),
                    tx: TransactionData::new_signed(
                        BytesJson(rlp::encode(&raw).to_vec()),
                        format!("{:02x}", BytesJson(raw.tx_hash_as_bytes().to_vec())),
                    ),
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

    /// Returns tx type as number if this type supported by this coin
    fn is_tx_type_supported(&self, tx_type: &TxType) -> bool {
        let tx_type_as_num = match tx_type {
            TxType::Legacy => 0_u64,
            TxType::Type1 => 1_u64,
            TxType::Type2 => 2_u64,
            TxType::Invalid => return false,
        };
        let max_tx_type = self.max_eth_tx_type.unwrap_or(0_u64);
        tx_type_as_num <= max_tx_type
    }

    /// Retrieves the lock associated with a given address.
    ///
    /// This function is used to ensure that only one transaction is sent at a time per address.
    /// If the address does not have an associated lock, a new one is created and stored.
    async fn get_address_lock(&self, address: Address) -> Arc<AsyncMutex<()>> {
        self.address_nonce_locks.get_adddress_lock(address).await
    }
}

#[cfg_attr(test, mockable)]
impl EthCoin {
    /// Sign and send eth transaction.
    /// This function is primarily for swap transactions so internally it relies on the swap tx fee policy
    pub fn sign_and_send_transaction(&self, value: U256, action: Action, data: Vec<u8>, gas: U256) -> EthTxFut {
        let coin = self.clone();
        let fut = async move {
            match coin.priv_key_policy {
                EthPrivKeyPolicy::Iguana(ref key_pair)
                | EthPrivKeyPolicy::HDWallet {
                    activated_key: ref key_pair,
                    ..
                } => {
                    let address = coin
                        .derivation_method
                        .single_addr_or_err()
                        .await
                        .map_err(|e| TransactionErr::Plain(ERRL!("{}", e)))?;

                    sign_and_send_transaction_with_keypair(&coin, key_pair, address, value, action, data, gas).await
                },
                EthPrivKeyPolicy::WalletConnect { .. } => {
                    let wc = {
                        let ctx = MmArc::from_weak(&coin.ctx).expect("No context");
                        WalletConnectCtx::from_ctx(&ctx)
                            .expect("TODO: handle error when enable kdf initialization without key.")
                    };
                    let address = coin
                        .derivation_method
                        .single_addr_or_err()
                        .await
                        .map_err(|e| TransactionErr::Plain(ERRL!("{}", e)))?;

                    send_transaction_with_walletconnect(coin, &wc, address, value, action, &data, gas).await
                },
                EthPrivKeyPolicy::Trezor => Err(TransactionErr::Plain(ERRL!("Trezor is not supported for swaps yet!"))),
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
            EthCoinType::Eth => self.sign_and_send_transaction(
                value,
                Action::Call(address),
                vec![],
                U256::from(self.gas_limit.eth_send_coins),
            ),
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let abi = try_tx_fus!(Contract::load(ERC20_ABI.as_bytes()));
                let function = try_tx_fus!(abi.function("transfer"));
                let data = try_tx_fus!(function.encode_input(&[Token::Address(address), Token::Uint(value)]));
                self.sign_and_send_transaction(
                    0.into(),
                    Action::Call(*token_addr),
                    data,
                    U256::from(self.gas_limit.eth_send_erc20),
                )
            },
            EthCoinType::Nft { .. } => Box::new(futures01::future::err(TransactionErr::ProtocolNotSupported(ERRL!(
                "Nft Protocol is not supported yet!"
            )))),
        }
    }

    fn send_hash_time_locked_payment(&self, args: SendPaymentArgs<'_>) -> EthTxFut {
        let receiver_addr = try_tx_fus!(addr_from_raw_pubkey(args.other_pubkey));
        let swap_contract_address = try_tx_fus!(args.swap_contract_address.try_to_address());
        let id = self.etomic_swap_id(try_tx_fus!(args.time_lock.try_into()), args.secret_hash);
        let trade_amount = try_tx_fus!(u256_from_big_decimal(&args.amount, self.decimals));

        let time_lock = U256::from(args.time_lock);

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
                        let reward_amount = try_tx_fus!(u256_from_big_decimal(&reward.amount, self.decimals));
                        if !matches!(reward.reward_target, RewardTarget::None) || reward.send_contract_reward_on_spend {
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
                let gas = U256::from(self.gas_limit.eth_payment);
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

                debug!("Using watcher reward {:?} for swap payment", args.watcher_reward);

                let data = match args.watcher_reward {
                    Some(reward) => {
                        let reward_amount = match reward.reward_target {
                            RewardTarget::Contract | RewardTarget::PaymentSender => {
                                let eth_reward_amount =
                                    try_tx_fus!(u256_from_big_decimal(&reward.amount, ETH_DECIMALS));
                                value += eth_reward_amount;
                                eth_reward_amount
                            },
                            RewardTarget::PaymentSpender => {
                                let token_reward_amount =
                                    try_tx_fus!(u256_from_big_decimal(&reward.amount, self.decimals));
                                amount += token_reward_amount;
                                token_reward_amount
                            },
                            _ => {
                                // TODO tests passed without this change, need to research on how it worked
                                if reward.send_contract_reward_on_spend {
                                    let eth_reward_amount =
                                        try_tx_fus!(u256_from_big_decimal(&reward.amount, ETH_DECIMALS));
                                    value += eth_reward_amount;
                                    eth_reward_amount
                                } else {
                                    0.into()
                                }
                            },
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
                let gas = U256::from(self.gas_limit.erc20_payment);

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
                                            approved.tx_hash_as_bytes(),
                                            e
                                        ))
                                    })
                                    .and_then(move |_| {
                                        arc.sign_and_send_transaction(
                                            value,
                                            Call(swap_contract_address),
                                            data,
                                            gas,
                                        )
                                    })
                                }),
                        )
                    } else {
                        Box::new(arc.sign_and_send_transaction(
                            value,
                            Call(swap_contract_address),
                            data,
                            gas,
                        ))
                    }
                }))
            },
            EthCoinType::Nft { .. } => Box::new(futures01::future::err(TransactionErr::ProtocolNotSupported(ERRL!(
                "Nft Protocol is not supported yet!"
            )))),
        }
    }

    fn watcher_spends_hash_time_locked_payment(&self, input: SendMakerPaymentSpendPreimageInput) -> EthTxFut {
        let tx: UnverifiedTransactionWrapper = try_tx_fus!(rlp::decode(input.preimage));
        let payment = try_tx_fus!(SignedEthTx::new(tx));

        let function_name = get_function_name("receiverSpend", input.watcher_reward);
        let spend_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));
        let clone = self.clone();
        let secret_vec = input.secret.to_vec();
        let taker_addr = addr_from_raw_pubkey(input.taker_pub).unwrap();
        let swap_contract_address = match payment.unsigned().action() {
            Call(address) => *address,
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
                let decoded = try_tx_fus!(decode_contract_call(payment_func, payment.unsigned().data()));
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

                            let value = payment.unsigned().value();
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
                                Call(swap_contract_address),
                                data,
                                U256::from(clone.gas_limit.eth_receiver_spend),
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

                let decoded = try_tx_fus!(decode_contract_call(payment_func, payment.unsigned().data()));
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
                                Call(swap_contract_address),
                                data,
                                U256::from(clone.gas_limit.erc20_receiver_spend),
                            )
                        }),
                )
            },
            EthCoinType::Nft { .. } => Box::new(futures01::future::err(TransactionErr::ProtocolNotSupported(ERRL!(
                "Nft Protocol is not supported yet!"
            )))),
        }
    }

    fn watcher_refunds_hash_time_locked_payment(&self, args: RefundPaymentArgs) -> EthTxFut {
        let tx: UnverifiedTransactionWrapper = try_tx_fus!(rlp::decode(args.payment_tx));
        let payment = try_tx_fus!(SignedEthTx::new(tx));

        let function_name = get_function_name("senderRefund", true);
        let refund_func = try_tx_fus!(SWAP_CONTRACT.function(&function_name));

        let clone = self.clone();
        let taker_addr = addr_from_raw_pubkey(args.other_pubkey).unwrap();
        let swap_contract_address = match payment.unsigned().action() {
            Call(address) => *address,
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
                let decoded = try_tx_fus!(decode_contract_call(payment_func, payment.unsigned().data()));
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

                            let value = payment.unsigned().value();
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
                                Call(swap_contract_address),
                                data,
                                U256::from(clone.gas_limit.eth_sender_refund),
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

                let decoded = try_tx_fus!(decode_contract_call(payment_func, payment.unsigned().data()));
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
                                Call(swap_contract_address),
                                data,
                                U256::from(clone.gas_limit.erc20_sender_refund),
                            )
                        }),
                )
            },
            EthCoinType::Nft { .. } => Box::new(futures01::future::err(TransactionErr::ProtocolNotSupported(ERRL!(
                "Nft Protocol is not supported yet!"
            )))),
        }
    }

    async fn spend_hash_time_locked_payment<'a>(
        &self,
        args: SpendPaymentArgs<'a>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let tx: UnverifiedTransactionWrapper = try_tx_s!(rlp::decode(args.other_payment_tx));
        let payment = try_tx_s!(SignedEthTx::new(tx));
        let my_address = try_tx_s!(self.derivation_method.single_addr_or_err().await);
        let swap_contract_address = try_tx_s!(args.swap_contract_address.try_to_address());

        let function_name = get_function_name("receiverSpend", args.watcher_reward);
        let spend_func = try_tx_s!(SWAP_CONTRACT.function(&function_name));

        let secret_vec = args.secret.to_vec();
        let watcher_reward = args.watcher_reward;

        match self.coin_type {
            EthCoinType::Eth => {
                let function_name = get_function_name("ethPayment", watcher_reward);
                let payment_func = try_tx_s!(SWAP_CONTRACT.function(&function_name));
                let decoded = try_tx_s!(decode_contract_call(payment_func, payment.unsigned().data()));

                let state = try_tx_s!(
                    self.payment_status(swap_contract_address, decoded[0].clone())
                        .compat()
                        .await
                );
                if state != U256::from(PaymentState::Sent as u8) {
                    return Err(TransactionErr::Plain(ERRL!(
                        "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                        payment,
                        state
                    )));
                }

                let data = if watcher_reward {
                    try_tx_s!(spend_func.encode_input(&[
                        decoded[0].clone(),
                        Token::Uint(payment.unsigned().value()),
                        Token::FixedBytes(secret_vec),
                        Token::Address(Address::default()),
                        Token::Address(payment.sender()),
                        Token::Address(my_address),
                        decoded[4].clone(),
                        decoded[5].clone(),
                        decoded[6].clone(),
                    ]))
                } else {
                    try_tx_s!(spend_func.encode_input(&[
                        decoded[0].clone(),
                        Token::Uint(payment.unsigned().value()),
                        Token::FixedBytes(secret_vec),
                        Token::Address(Address::default()),
                        Token::Address(payment.sender()),
                    ]))
                };

                self.sign_and_send_transaction(
                    0.into(),
                    Call(swap_contract_address),
                    data,
                    U256::from(self.gas_limit.eth_receiver_spend),
                )
                .compat()
                .await
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let function_name = get_function_name("erc20Payment", watcher_reward);
                let payment_func = try_tx_s!(SWAP_CONTRACT.function(&function_name));

                let decoded = try_tx_s!(decode_contract_call(payment_func, payment.unsigned().data()));
                let state = try_tx_s!(
                    self.payment_status(swap_contract_address, decoded[0].clone())
                        .compat()
                        .await
                );
                if state != U256::from(PaymentState::Sent as u8) {
                    return Err(TransactionErr::Plain(ERRL!(
                        "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                        payment,
                        state
                    )));
                }

                let data = if watcher_reward {
                    try_tx_s!(spend_func.encode_input(&[
                        decoded[0].clone(),
                        decoded[1].clone(),
                        Token::FixedBytes(secret_vec),
                        Token::Address(token_addr),
                        Token::Address(payment.sender()),
                        Token::Address(my_address),
                        decoded[6].clone(),
                        decoded[7].clone(),
                        decoded[8].clone(),
                    ]))
                } else {
                    try_tx_s!(spend_func.encode_input(&[
                        decoded[0].clone(),
                        decoded[1].clone(),
                        Token::FixedBytes(secret_vec),
                        Token::Address(token_addr),
                        Token::Address(payment.sender()),
                    ]))
                };

                self.sign_and_send_transaction(
                    0.into(),
                    Call(swap_contract_address),
                    data,
                    U256::from(self.gas_limit.erc20_receiver_spend),
                )
                .compat()
                .await
            },
            EthCoinType::Nft { .. } => Err(TransactionErr::ProtocolNotSupported(ERRL!(
                "Nft Protocol is not supported!"
            ))),
        }
    }

    async fn refund_hash_time_locked_payment<'a>(
        &self,
        args: RefundPaymentArgs<'a>,
    ) -> Result<SignedEthTx, TransactionErr> {
        let tx: UnverifiedTransactionWrapper = try_tx_s!(rlp::decode(args.payment_tx));
        let payment = try_tx_s!(SignedEthTx::new(tx));
        let my_address = try_tx_s!(self.derivation_method.single_addr_or_err().await);
        let swap_contract_address = try_tx_s!(args.swap_contract_address.try_to_address());

        let function_name = get_function_name("senderRefund", args.watcher_reward);
        let refund_func = try_tx_s!(SWAP_CONTRACT.function(&function_name));
        let watcher_reward = args.watcher_reward;

        match self.coin_type {
            EthCoinType::Eth => {
                let function_name = get_function_name("ethPayment", watcher_reward);
                let payment_func = try_tx_s!(SWAP_CONTRACT.function(&function_name));

                let decoded = try_tx_s!(decode_contract_call(payment_func, payment.unsigned().data()));

                let state = try_tx_s!(
                    self.payment_status(swap_contract_address, decoded[0].clone())
                        .compat()
                        .await
                );
                if state != U256::from(PaymentState::Sent as u8) {
                    return Err(TransactionErr::Plain(ERRL!(
                        "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                        payment,
                        state
                    )));
                }

                let value = payment.unsigned().value();
                let data = if watcher_reward {
                    try_tx_s!(refund_func.encode_input(&[
                        decoded[0].clone(),
                        Token::Uint(value),
                        decoded[2].clone(),
                        Token::Address(Address::default()),
                        Token::Address(my_address),
                        decoded[1].clone(),
                        decoded[4].clone(),
                        decoded[5].clone(),
                        decoded[6].clone(),
                    ]))
                } else {
                    try_tx_s!(refund_func.encode_input(&[
                        decoded[0].clone(),
                        Token::Uint(value),
                        decoded[2].clone(),
                        Token::Address(Address::default()),
                        decoded[1].clone(),
                    ]))
                };

                self.sign_and_send_transaction(
                    0.into(),
                    Call(swap_contract_address),
                    data,
                    U256::from(self.gas_limit.eth_sender_refund),
                )
                .compat()
                .await
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let function_name = get_function_name("erc20Payment", watcher_reward);
                let payment_func = try_tx_s!(SWAP_CONTRACT.function(&function_name));

                let decoded = try_tx_s!(decode_contract_call(payment_func, payment.unsigned().data()));
                let state = try_tx_s!(
                    self.payment_status(swap_contract_address, decoded[0].clone())
                        .compat()
                        .await
                );
                if state != U256::from(PaymentState::Sent as u8) {
                    return Err(TransactionErr::Plain(ERRL!(
                        "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                        payment,
                        state
                    )));
                }

                let data = if watcher_reward {
                    try_tx_s!(refund_func.encode_input(&[
                        decoded[0].clone(),
                        decoded[1].clone(),
                        decoded[4].clone(),
                        Token::Address(token_addr),
                        Token::Address(my_address),
                        decoded[3].clone(),
                        decoded[6].clone(),
                        decoded[7].clone(),
                        decoded[8].clone(),
                    ]))
                } else {
                    try_tx_s!(refund_func.encode_input(&[
                        decoded[0].clone(),
                        decoded[1].clone(),
                        decoded[4].clone(),
                        Token::Address(token_addr),
                        decoded[3].clone(),
                    ]))
                };

                self.sign_and_send_transaction(
                    0.into(),
                    Call(swap_contract_address),
                    data,
                    U256::from(self.gas_limit.erc20_sender_refund),
                )
                .compat()
                .await
            },
            EthCoinType::Nft { .. } => Err(TransactionErr::ProtocolNotSupported(ERRL!(
                "Nft Protocol is not supported yet!"
            ))),
        }
    }

    fn address_balance(&self, address: Address) -> BalanceFut<U256> {
        let coin = self.clone();
        let fut = async move {
            match coin.coin_type {
                EthCoinType::Eth => Ok(coin.balance(address, Some(BlockNumber::Latest)).await?),
                EthCoinType::Erc20 { ref token_addr, .. } => {
                    let function = ERC20_CONTRACT.function("balanceOf")?;
                    let data = function.encode_input(&[Token::Address(address)])?;

                    let res = coin
                        .call_request(address, *token_addr, None, Some(data.into()), BlockNumber::Latest)
                        .await?;
                    let decoded = function.decode_output(&res.0)?;
                    match decoded[0] {
                        Token::Uint(number) => Ok(number),
                        _ => {
                            let error = format!("Expected U256 as balanceOf result but got {decoded:?}");
                            MmError::err(BalanceError::InvalidResponse(error))
                        },
                    }
                },
                EthCoinType::Nft { .. } => {
                    MmError::err(BalanceError::Internal("Nft Protocol is not supported yet!".to_string()))
                },
            }
        };
        Box::new(fut.boxed().compat())
    }

    fn get_balance(&self) -> BalanceFut<U256> {
        let coin = self.clone();
        let fut = async move {
            let my_address = coin.derivation_method.single_addr_or_err().await.map_mm_err()?;
            coin.address_balance(my_address).compat().await
        };
        Box::new(fut.boxed().compat())
    }

    pub async fn get_tokens_balance_list_for_address(
        &self,
        address: Address,
    ) -> Result<CoinBalanceMap, MmError<BalanceError>> {
        let coin = || self;

        let tokens = self.get_erc_tokens_infos();
        let mut requests = Vec::with_capacity(tokens.len());

        for (token_ticker, info) in tokens {
            let fut = async move {
                let balance_as_u256 = coin()
                    .get_token_balance_for_address(address, info.token_address)
                    .await?;
                let balance_as_big_decimal = u256_to_big_decimal(balance_as_u256, info.decimals).map_mm_err()?;
                let balance = CoinBalance::new(balance_as_big_decimal);
                Ok((token_ticker, balance))
            };
            requests.push(fut);
        }

        try_join_all(requests).await.map(|res| res.into_iter().collect())
    }

    pub async fn get_tokens_balance_list(&self) -> Result<CoinBalanceMap, MmError<BalanceError>> {
        let my_address = self.derivation_method.single_addr_or_err().await.map_mm_err()?;
        self.get_tokens_balance_list_for_address(my_address).await
    }

    async fn get_token_balance_for_address(
        &self,
        address: Address,
        token_address: Address,
    ) -> Result<U256, MmError<BalanceError>> {
        let function = ERC20_CONTRACT.function("balanceOf")?;
        let data = function.encode_input(&[Token::Address(address)])?;
        let res = self
            .call_request(address, token_address, None, Some(data.into()), BlockNumber::Latest)
            .await?;
        let decoded = function.decode_output(&res.0)?;

        match decoded[0] {
            Token::Uint(number) => Ok(number),
            _ => {
                let error = format!("Expected U256 as balanceOf result but got {decoded:?}");
                MmError::err(BalanceError::InvalidResponse(error))
            },
        }
    }

    async fn get_token_balance(&self, token_address: Address) -> Result<U256, MmError<BalanceError>> {
        let my_address = self.derivation_method.single_addr_or_err().await.map_mm_err()?;
        self.get_token_balance_for_address(my_address, token_address).await
    }

    async fn erc1155_balance(&self, token_addr: Address, token_id: &str) -> MmResult<BigUint, BalanceError> {
        let wallet_amount_uint = match self.coin_type {
            EthCoinType::Eth | EthCoinType::Nft { .. } => {
                let function = ERC1155_CONTRACT.function("balanceOf")?;
                let token_id_u256 = U256::from_dec_str(token_id)
                    .map_to_mm(|e| NumConversError::new(format!("{e:?}")))
                    .map_mm_err()?;
                let my_address = self.derivation_method.single_addr_or_err().await.map_mm_err()?;
                let data = function.encode_input(&[Token::Address(my_address), Token::Uint(token_id_u256)])?;
                let result = self
                    .call_request(my_address, token_addr, None, Some(data.into()), BlockNumber::Latest)
                    .await?;
                let decoded = function.decode_output(&result.0)?;
                match decoded[0] {
                    Token::Uint(number) => number,
                    _ => {
                        let error = format!("Expected U256 as balanceOf result but got {decoded:?}");
                        return MmError::err(BalanceError::InvalidResponse(error));
                    },
                }
            },
            EthCoinType::Erc20 { .. } => {
                return MmError::err(BalanceError::Internal(
                    "Erc20 coin type doesnt support Erc1155 standard".to_owned(),
                ))
            },
        };
        // The "balanceOf" function in ERC1155 standard returns the exact count of tokens held by address without any decimals or scaling factors
        let wallet_amount = wallet_amount_uint.to_string().parse::<BigUint>()?;
        Ok(wallet_amount)
    }

    async fn erc721_owner(&self, token_addr: Address, token_id: &str) -> MmResult<Address, GetNftInfoError> {
        let owner_address = match self.coin_type {
            EthCoinType::Eth | EthCoinType::Nft { .. } => {
                let function = ERC721_CONTRACT.function("ownerOf")?;
                let token_id_u256 = U256::from_dec_str(token_id)
                    .map_to_mm(|e| NumConversError::new(format!("{e:?}")))
                    .map_mm_err()?;
                let data = function.encode_input(&[Token::Uint(token_id_u256)])?;
                let my_address = self.derivation_method.single_addr_or_err().await.map_mm_err()?;
                let result = self
                    .call_request(my_address, token_addr, None, Some(data.into()), BlockNumber::Latest)
                    .await?;
                let decoded = function.decode_output(&result.0)?;
                match decoded[0] {
                    Token::Address(owner) => owner,
                    _ => {
                        let error = format!("Expected Address as ownerOf result but got {decoded:?}");
                        return MmError::err(GetNftInfoError::InvalidResponse(error));
                    },
                }
            },
            EthCoinType::Erc20 { .. } => {
                return MmError::err(GetNftInfoError::Internal(
                    "Erc20 coin type doesnt support Erc721 standard".to_owned(),
                ))
            },
        };
        Ok(owner_address)
    }

    fn estimate_gas_wrapper(&self, req: CallRequest) -> Box<dyn Future<Item = U256, Error = web3::Error> + Send> {
        let coin = self.clone();

        // always using None block number as old Geth version accept only single argument in this RPC
        let fut = async move { coin.estimate_gas(req, None).await };

        Box::new(fut.boxed().compat())
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
    async fn estimate_gas_for_contract_call(&self, contract_addr: Address, call_data: Bytes) -> Web3RpcResult<U256> {
        let coin = self.clone();
        let my_address = coin.derivation_method.single_addr_or_err().await.map_mm_err()?;
        let fee_policy_for_estimate =
            get_swap_fee_policy_for_estimate(self.get_swap_gas_fee_policy().await.map_mm_err()?);
        let pay_for_gas_option = coin
            .get_swap_pay_for_gas_option(fee_policy_for_estimate)
            .await
            .map_mm_err()?;
        let eth_value = U256::zero();
        let estimate_gas_req = CallRequest {
            value: Some(eth_value),
            data: Some(call_data),
            from: Some(my_address),
            to: Some(contract_addr),
            ..CallRequest::default()
        };
        // gas price must be supplied because some smart contracts base their
        // logic on gas price, e.g. TUSD: https://github.com/KomodoPlatform/atomicDEX-API/issues/643
        let estimate_gas_req = call_request_with_pay_for_gas_option(estimate_gas_req, pay_for_gas_option);
        coin.estimate_gas_wrapper(estimate_gas_req)
            .compat()
            .await
            .map_to_mm(Web3RpcError::from)
    }

    #[allow(unused)] // TODO: remove
    async fn estimate_gas_for_contract_call_if_conf(
        &self,
        contract_addr: Address,
        call_data: Bytes,
    ) -> Web3RpcResult<Option<U256>> {
        if let Some(estimate_gas_mult) = self.estimate_gas_mult {
            let gas_estimated = self.estimate_gas_for_contract_call(contract_addr, call_data).await?;
            let gas_estimated = u256_to_big_decimal(gas_estimated, 0).map_mm_err()?
                * BigDecimal::from_f64(estimate_gas_mult).unwrap_or(BigDecimal::from(1));
            return Ok(Some(u256_from_big_decimal(&gas_estimated, 0).map_mm_err()?));
        }
        Ok(None)
    }

    fn eth_balance(&self) -> BalanceFut<U256> {
        let coin = self.clone();
        let fut = async move {
            let my_address = coin.derivation_method.single_addr_or_err().await.map_mm_err()?;
            coin.balance(my_address, Some(BlockNumber::Latest))
                .await
                .map_to_mm(BalanceError::from)
        };
        Box::new(fut.boxed().compat())
    }

    pub(crate) async fn call_request(
        &self,
        from: Address,
        to: Address,
        value: Option<U256>,
        data: Option<Bytes>,
        block_number: BlockNumber,
    ) -> Result<Bytes, web3::Error> {
        let request = CallRequest {
            from: Some(from),
            to: Some(to),
            gas: None,
            gas_price: None,
            value,
            data,
            ..CallRequest::default()
        };

        self.call(request, Some(BlockId::Number(block_number))).await
    }

    pub fn allowance(&self, spender: Address) -> Web3RpcFut<U256> {
        let coin = self.clone();
        let fut = async move {
            match coin.coin_type {
                EthCoinType::Eth => MmError::err(Web3RpcError::Internal(
                    "'allowance' must not be called for ETH coin".to_owned(),
                )),
                EthCoinType::Erc20 { ref token_addr, .. } => {
                    let function = ERC20_CONTRACT.function("allowance")?;
                    let my_address = coin.derivation_method.single_addr_or_err().await.map_mm_err()?;
                    let data = function.encode_input(&[Token::Address(my_address), Token::Address(spender)])?;

                    let res = coin
                        .call_request(my_address, *token_addr, None, Some(data.into()), BlockNumber::Latest)
                        .await?;
                    let decoded = function.decode_output(&res.0)?;

                    match decoded[0] {
                        Token::Uint(number) => Ok(number),
                        _ => {
                            let error = format!("Expected U256 as allowance result but got {decoded:?}");
                            MmError::err(Web3RpcError::InvalidResponse(error))
                        },
                    }
                },
                EthCoinType::Nft { .. } => MmError::err(Web3RpcError::NftProtocolNotSupported),
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

    pub fn approve(&self, spender: Address, amount: U256) -> EthTxFut {
        let coin = self.clone();
        let fut = async move {
            let token_addr = match coin.coin_type {
                EthCoinType::Eth => return TX_PLAIN_ERR!("'approve' is expected to be call for ERC20 coins only"),
                EthCoinType::Erc20 { token_addr, .. } => token_addr,
                EthCoinType::Nft { .. } => {
                    return Err(TransactionErr::ProtocolNotSupported(ERRL!(
                        "Nft Protocol is not supported by 'approve'!"
                    )))
                },
            };
            let function = try_tx_s!(ERC20_CONTRACT.function("approve"));
            let data = try_tx_s!(function.encode_input(&[Token::Address(spender), Token::Uint(amount)]));

            let gas_limit = try_tx_s!(
                coin.estimate_gas_for_contract_call(token_addr, Bytes::from(data.clone()))
                    .await
            );

            coin.sign_and_send_transaction(0.into(), Call(token_addr), data, gas_limit)
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

        let coin = self.clone();

        let fut = async move { coin.logs(filter).await.map_err(|e| ERRL!("{}", e)) };
        Box::new(fut.boxed().compat())
    }

    /// Returns events from `from_block` to `to_block` or current `latest` block.
    /// According to ["eth_getLogs" doc](https://docs.infura.io/api/networks/ethereum/json-rpc-methods/eth_getlogs) `toBlock` is optional, default is "latest".
    async fn events_from_block(
        &self,
        swap_contract_address: Address,
        event_name: &str,
        from_block: u64,
        to_block: Option<u64>,
        swap_contract: &Contract,
    ) -> MmResult<Vec<Log>, FindPaymentSpendError> {
        let contract_event = swap_contract.event(event_name)?;
        let mut filter_builder = FilterBuilder::default()
            .topics(Some(vec![contract_event.signature()]), None, None, None)
            .from_block(BlockNumber::Number(from_block.into()))
            .address(vec![swap_contract_address]);
        if let Some(block) = to_block {
            filter_builder = filter_builder.to_block(BlockNumber::Number(block.into()));
        }
        let filter = filter_builder.build();
        let events_logs = self
            .logs(filter)
            .await
            .map_err(|e| FindPaymentSpendError::Transport(e.to_string()))?;
        Ok(events_logs)
    }

    fn validate_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        let expected_swap_contract_address = try_f!(input
            .swap_contract_address
            .try_to_address()
            .map_to_mm(ValidatePaymentError::InvalidParameter));

        let unsigned: UnverifiedTransactionWrapper = try_f!(rlp::decode(&input.payment_tx));
        let tx =
            try_f!(SignedEthTx::new(unsigned)
                .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string())));
        let sender = try_f!(addr_from_raw_pubkey(&input.other_pub).map_to_mm(ValidatePaymentError::InvalidParameter));
        let time_lock = try_f!(input
            .time_lock
            .try_into()
            .map_to_mm(ValidatePaymentError::TimelockOverflow));

        let selfi = self.clone();
        let swap_id = selfi.etomic_swap_id(time_lock, &input.secret_hash);
        let decimals = self.decimals;
        let secret_hash = if input.secret_hash.len() == 32 {
            ripemd160(&input.secret_hash).to_vec()
        } else {
            input.secret_hash.to_vec()
        };
        let trade_amount = try_f!(u256_from_big_decimal(&(input.amount), decimals).map_mm_err());
        let fut = async move {
            let status = selfi
                .payment_status(expected_swap_contract_address, Token::FixedBytes(swap_id.clone()))
                .compat()
                .await
                .map_to_mm(ValidatePaymentError::Transport)?;
            if status != U256::from(PaymentState::Sent as u8) {
                return MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
                    "Payment state is not PAYMENT_STATE_SENT, got {status}"
                )));
            }

            let tx_from_rpc = selfi.transaction(TransactionId::Hash(tx.tx_hash())).await?;
            let tx_from_rpc = tx_from_rpc.as_ref().ok_or_else(|| {
                ValidatePaymentError::TxDoesNotExist(format!("Didn't find provided tx {:?} on ETH node", tx.tx_hash()))
            })?;

            if tx_from_rpc.from != Some(sender) {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Payment tx {tx_from_rpc:?} was sent from wrong address, expected {sender:?}"
                )));
            }

            let my_address = selfi.derivation_method.single_addr_or_err().await.map_mm_err()?;
            match &selfi.coin_type {
                EthCoinType::Eth => {
                    let mut expected_value = trade_amount;

                    if tx_from_rpc.to != Some(expected_swap_contract_address) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx {tx_from_rpc:?} was sent to wrong address, expected {expected_swap_contract_address:?}",
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
                            "Invalid 'swap_id' {decoded:?}, expected {swap_id:?}"
                        )));
                    }

                    if decoded[1] != Token::Address(my_address) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx receiver arg {:?} is invalid, expected {:?}",
                            decoded[1],
                            Token::Address(my_address)
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

                        let expected_reward_amount =
                            u256_from_big_decimal(&watcher_reward.amount, decimals).map_mm_err()?;
                        let actual_reward_amount = decoded[6].clone().into_uint().ok_or_else(|| {
                            ValidatePaymentError::WrongPaymentTx("Invalid type for watcher reward argument".to_string())
                        })?;

                        validate_watcher_reward(
                            expected_reward_amount.as_u64(),
                            actual_reward_amount.as_u64(),
                            watcher_reward.is_exact_amount,
                        )?;

                        match watcher_reward.reward_target {
                            RewardTarget::None | RewardTarget::PaymentReceiver => {
                                if watcher_reward.send_contract_reward_on_spend {
                                    expected_value += actual_reward_amount
                                }
                            },
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
                            "Payment tx {tx_from_rpc:?} was sent to wrong address, expected {expected_swap_contract_address:?}",
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
                            "Invalid 'swap_id' {decoded:?}, expected {swap_id:?}"
                        )));
                    }

                    if decoded[2] != Token::Address(*token_addr) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx token_addr arg {:?} is invalid, expected {:?}",
                            decoded[2],
                            Token::Address(*token_addr)
                        )));
                    }

                    if decoded[3] != Token::Address(my_address) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx receiver arg {:?} is invalid, expected {:?}",
                            decoded[3],
                            Token::Address(my_address),
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

                        let expected_reward_amount = match watcher_reward.reward_target {
                            RewardTarget::Contract | RewardTarget::PaymentSender => {
                                u256_from_big_decimal(&watcher_reward.amount, ETH_DECIMALS).map_mm_err()?
                            },
                            RewardTarget::PaymentSpender => {
                                u256_from_big_decimal(&watcher_reward.amount, selfi.decimals).map_mm_err()?
                            },
                            _ => {
                                // TODO tests passed without this change, need to research on how it worked
                                if watcher_reward.send_contract_reward_on_spend {
                                    u256_from_big_decimal(&watcher_reward.amount, ETH_DECIMALS).map_mm_err()?
                                } else {
                                    0.into()
                                }
                            },
                        };

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
                            _ => {
                                if watcher_reward.send_contract_reward_on_spend {
                                    expected_value += actual_reward_amount
                                }
                            },
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
                            tx_from_rpc.value, expected_value
                        )));
                    }
                },
                EthCoinType::Nft { .. } => {
                    return MmError::err(ValidatePaymentError::ProtocolNotSupported(
                        "Nft protocol is not supported by legacy swap".to_string(),
                    ))
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
        let fut = async move {
            let my_address = coin
                .derivation_method
                .single_addr_or_err()
                .await
                .map_err(|e| ERRL!("{}", e))?;
            coin.call_request(
                my_address,
                swap_contract_address,
                None,
                Some(data.into()),
                // TODO worth reviewing places where we could use BlockNumber::Pending
                BlockNumber::Latest,
            )
            .await
            .map_err(|e| ERRL!("{}", e))
        };

        Box::new(fut.boxed().compat().and_then(move |bytes| {
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
        let unverified: UnverifiedTransactionWrapper = try_s!(rlp::decode(tx));
        let tx = try_s!(SignedEthTx::new(unverified));

        let func_name = match self.coin_type {
            EthCoinType::Eth => get_function_name("ethPayment", watcher_reward),
            EthCoinType::Erc20 { .. } => get_function_name("erc20Payment", watcher_reward),
            EthCoinType::Nft { .. } => return ERR!("Nft Protocol is not supported yet!"),
        };

        let payment_func = try_s!(SWAP_CONTRACT.function(&func_name));
        let decoded = try_s!(decode_contract_call(payment_func, tx.unsigned().data()));
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
                self.events_from_block(
                    swap_contract_address,
                    "ReceiverSpent",
                    from_block,
                    Some(to_block),
                    &SWAP_CONTRACT
                )
                .await
            );

            let found = spend_events.iter().find(|event| &event.data.0[..32] == id.as_slice());

            if let Some(event) = found {
                match event.transaction_hash {
                    Some(tx_hash) => {
                        let transaction = match try_s!(self.transaction(TransactionId::Hash(tx_hash)).await) {
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
                        let transaction = match try_s!(self.transaction(TransactionId::Hash(tx_hash)).await) {
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
        let pay_for_gas_policy = self.get_swap_gas_fee_policy().await.map_mm_err()?;
        let pay_for_gas_option = repeatable!(async {
            self.get_swap_pay_for_gas_option(pay_for_gas_policy.clone())
                .await
                .retry_on_err()
        })
        .until_s(wait_until)
        .repeat_every_secs(10.)
        .await
        .map_err(|_| WatcherRewardError::RPCError("Error getting the gas price".to_string()))?;

        let gas_cost_wei = calc_total_fee(U256::from(REWARD_GAS_AMOUNT), &pay_for_gas_option)
            .map_err(|e| WatcherRewardError::InternalError(e.to_string()))?;
        let gas_cost_eth = u256_to_big_decimal(gas_cost_wei, ETH_DECIMALS)
            .map_err(|e| WatcherRewardError::InternalError(e.to_string()))?;
        Ok(gas_cost_eth)
    }

    /// Get gas price
    pub async fn get_gas_price(&self) -> Web3RpcResult<U256> {
        let coin = self.clone();
        let eth_gas_price_fut = async {
            match coin.gas_price().await {
                Ok(eth_gas) => Some(eth_gas),
                Err(e) => {
                    error!("Error {} on eth_gasPrice request", e);
                    None
                },
            }
        }
        .boxed();

        let eth_fee_history_price_fut = async {
            match coin.eth_fee_history(U256::from(1u64), BlockNumber::Latest, &[]).await {
                Ok(res) => res
                    .base_fee_per_gas
                    .first()
                    .map(|val| increase_by_percent(*val, BASE_BLOCK_FEE_DIFF_PCT)),
                Err(e) => {
                    debug!("Error {} on eth_feeHistory request", e);
                    None
                },
            }
        }
        .boxed();

        let (eth_gas_price, eth_fee_history_price) = join(eth_gas_price_fut, eth_fee_history_price_fut).await;
        // on editions < 2021 the compiler will resolve array.into_iter() as (&array).into_iter()
        // https://doc.rust-lang.org/edition-guide/rust-2021/IntoIterator-for-arrays.html#details
        let gas_price = IntoIterator::into_iter([eth_gas_price, eth_fee_history_price])
            .flatten()
            .max()
            .or_mm_err(|| Web3RpcError::Internal("All requests failed".into()))?;
        if let Some(gas_price_adjust) = &self.gas_price_adjust {
            let gas_price = u256_to_big_decimal(gas_price, 0).map_mm_err()?;
            let mult = BigDecimal::try_from(gas_price_adjust.legacy_price_mult).map_err(|_| {
                MmError::new(Web3RpcError::NumConversError(
                    "gas_price_mult conversion error".to_string(),
                ))
            })?;
            let gas_price_adjusted = gas_price * mult;
            let gas_price_adjusted = u256_from_big_decimal(&gas_price_adjusted, 0).map_mm_err()?;
            Ok(gas_price_adjusted)
        } else {
            Ok(gas_price)
        }
    }

    /// Get gas base fee and suggest priority tip fees for the next block (see EIP-1559)
    pub async fn get_eip1559_gas_fee(&self, use_simple: bool) -> Web3RpcResult<FeePerGasEstimated> {
        let coin = self.clone();
        let history_estimator_fut = FeePerGasSimpleEstimator::estimate_fee_by_history(&coin);
        let ctx =
            MmArc::from_weak(&coin.ctx).ok_or_else(|| MmError::new(Web3RpcError::Internal("ctx is null".into())))?;

        let gas_api_conf = ctx.conf["gas_api"].clone();
        if gas_api_conf.is_null() || use_simple {
            return history_estimator_fut
                .await
                .map_err(|e| MmError::new(Web3RpcError::Internal(e.to_string())));
        }
        let gas_api_conf: GasApiConfig = json::from_value(gas_api_conf)
            .map_err(|e| MmError::new(Web3RpcError::InvalidGasApiConfig(e.to_string())))?;
        let provider_estimator_fut = match gas_api_conf.provider {
            GasApiProvider::Infura => InfuraGasApiCaller::fetch_infura_fee_estimation(&gas_api_conf.url).boxed(),
            GasApiProvider::Blocknative => {
                BlocknativeGasApiCaller::fetch_blocknative_fee_estimation(&gas_api_conf.url).boxed()
            },
        };
        provider_estimator_fut
            .or_else(|provider_estimator_err| {
                debug!(
                    "Call to eth gas api provider failed {}, using internal fee estimator",
                    provider_estimator_err
                );
                history_estimator_fut.map_err(move |history_estimator_err| {
                    MmError::new(Web3RpcError::Internal(format!(
                        "All gas api requests failed, provider estimator error: {provider_estimator_err}, history estimator error: {history_estimator_err}"
                    )))
                })
            })
            .await
    }

    async fn get_swap_pay_for_gas_option(&self, swap_fee_policy: SwapGasFeePolicy) -> Web3RpcResult<PayForGasOption> {
        let coin = self.clone();
        match swap_fee_policy {
            SwapGasFeePolicy::Legacy => {
                let gas_price = coin.get_gas_price().await?;
                Ok(PayForGasOption::Legacy { gas_price })
            },
            SwapGasFeePolicy::Low | SwapGasFeePolicy::Medium | SwapGasFeePolicy::High => {
                let fee_per_gas = coin.get_eip1559_gas_fee(false).await?;
                let pay_result = match swap_fee_policy {
                    SwapGasFeePolicy::Low => PayForGasOption::Eip1559 {
                        max_fee_per_gas: fee_per_gas.low.max_fee_per_gas,
                        max_priority_fee_per_gas: fee_per_gas.low.max_priority_fee_per_gas,
                    },
                    SwapGasFeePolicy::Medium => PayForGasOption::Eip1559 {
                        max_fee_per_gas: fee_per_gas.medium.max_fee_per_gas,
                        max_priority_fee_per_gas: fee_per_gas.medium.max_priority_fee_per_gas,
                    },
                    _ => PayForGasOption::Eip1559 {
                        max_fee_per_gas: fee_per_gas.high.max_fee_per_gas,
                        max_priority_fee_per_gas: fee_per_gas.high.max_priority_fee_per_gas,
                    },
                };
                Ok(pay_result)
            },
        }
    }

    /// Get pay for gas option from the sign_raw_tx rpc params GasPriceRpcParam.
    /// The rpc params allow to set legacy or priority gas fee explicitly or make use GasPricePolicy value, set by a dedicated rpc.
    async fn get_swap_pay_for_gas_option_from_rpc(
        &self,
        gas_price_param: &Option<GasPriceRpcParam>,
    ) -> Web3RpcResult<PayForGasOption> {
        let pay_for_gas_option = match gas_price_param {
            Some(GasPriceRpcParam::GasPricePolicy(policy)) => self.get_swap_pay_for_gas_option(policy.clone()).await?,
            Some(GasPriceRpcParam::Legacy { gas_price }) => PayForGasOption::Legacy {
                gas_price: wei_from_gwei_decimal(gas_price).map_mm_err()?,
            },
            Some(GasPriceRpcParam::Eip1559 {
                max_fee_per_gas,
                max_priority_fee_per_gas,
            }) => PayForGasOption::Eip1559 {
                max_fee_per_gas: wei_from_gwei_decimal(max_fee_per_gas).map_mm_err()?,
                max_priority_fee_per_gas: wei_from_gwei_decimal(max_priority_fee_per_gas).map_mm_err()?,
            },
            None => {
                // use legacy gas_price() if not set
                let gas_price = self.get_gas_price().await?;
                PayForGasOption::Legacy { gas_price }
            },
        };
        Ok(pay_for_gas_option)
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
            match self.clone().get_addr_nonce(addr).compat().await {
                Ok((new_nonce, _)) if new_nonce > prev_nonce => Ready(()),
                Ok((_nonce, _)) => Retry(()),
                Err(e) => {
                    error!("Error getting {} {} nonce: {}", self.ticker(), addr, e);
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
    async fn wait_for_tx_appears_on_rpc(
        &self,
        tx_hash: H256,
        wait_rpc_timeout_s: u64,
        check_every: f64,
    ) -> Web3RpcResult<Option<SignedEthTx>> {
        let wait_until = wait_until_sec(wait_rpc_timeout_s);
        while now_sec() < wait_until {
            let maybe_tx = self.transaction(TransactionId::Hash(tx_hash)).await?;
            if let Some(tx) = maybe_tx {
                let signed_tx = signed_tx_from_web3_tx(tx).map_to_mm(Web3RpcError::InvalidResponse)?;
                return Ok(Some(signed_tx));
            }

            Timer::sleep(check_every).await;
        }

        warn!(
            "Couldn't fetch the '{tx_hash:02x}' transaction hex as it hasn't appeared on the RPC node in {wait_rpc_timeout_s}s"
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

                let web3_receipt = match selfi.transaction_receipt(payment_hash).await {
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

                match selfi.block_number().await {
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

    /// Requests the nonce from all available nodes and returns the highest nonce available with the list of nodes that returned the highest nonce.
    /// Transactions will be sent using the nodes that returned the highest nonce.
    pub fn get_addr_nonce(
        self,
        addr: Address,
    ) -> Box<dyn Future<Item = (U256, Vec<Web3Instance>), Error = String> + Send> {
        const TMP_SOCKET_DURATION: Duration = Duration::from_secs(300);

        let fut = async move {
            let mut errors: u32 = 0;
            let web3_instances = self.web3_instances.lock().await.to_vec();
            loop {
                let (futures, web3_instances): (Vec<_>, Vec<_>) = web3_instances
                    .iter()
                    .map(|instance| {
                        if let Web3Transport::Websocket(socket_transport) = instance.as_ref().transport() {
                            socket_transport.maybe_spawn_temporary_connection_loop(
                                self.clone(),
                                Instant::now() + TMP_SOCKET_DURATION,
                            );
                        };

                        let nonce = instance
                            .as_ref()
                            .eth()
                            .transaction_count(addr, Some(BlockNumber::Pending));

                        (nonce, instance.clone())
                    })
                    .unzip();

                let nonces: Vec<_> = join_all(futures)
                    .await
                    .into_iter()
                    .zip(web3_instances)
                    .filter_map(|(nonce_res, instance)| match nonce_res {
                        Ok(n) => Some((n, instance)),
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
                            .filter_map(|(n, instance)| if n == max { Some(instance) } else { None })
                            .collect(),
                    ));
                }
                Timer::sleep(1.).await
            }
        };
        Box::new(Box::pin(fut).compat())
    }

    /// Estimated trade fee for the provided gas limit
    pub async fn estimate_trade_fee(&self, gas_limit: U256, stage: FeeApproxStage) -> TradePreimageResult<TradeFee> {
        let pay_for_gas_option = self
            .get_swap_pay_for_gas_option(self.get_swap_gas_fee_policy().await.map_mm_err()?)
            .await
            .map_mm_err()?;
        let pay_for_gas_option = increase_gas_price_by_stage(pay_for_gas_option, &stage);
        let total_fee = calc_total_fee(gas_limit, &pay_for_gas_option).map_mm_err()?;
        let amount = u256_to_big_decimal(total_fee, ETH_DECIMALS).map_mm_err()?;
        let fee_coin = match &self.coin_type {
            EthCoinType::Eth => &self.ticker,
            EthCoinType::Erc20 { platform, .. } => platform,
            EthCoinType::Nft { .. } => return MmError::err(TradePreimageError::NftProtocolNotSupported),
        };
        Ok(TradeFee {
            coin: fee_coin.into(),
            amount: amount.into(),
            paid_from_trading_vol: false,
        })
    }

    pub async fn platform_coin(&self) -> CoinFindResult<EthCoin> {
        match &self.coin_type {
            EthCoinType::Eth => Ok(self.clone()),
            EthCoinType::Erc20 { platform, .. } | EthCoinType::Nft { platform } => {
                let ctx = MmArc::from_weak(&self.ctx).expect("No context");
                let platform_coin = lp_coinfind_or_err(&ctx, platform).await?;
                match platform_coin {
                    MmCoinEnum::EthCoin(eth_coin) => Ok(eth_coin),
                    _ => MmError::err(CoinFindError::NoSuchCoin {
                        coin: platform.to_string(),
                    }),
                }
            },
        }
    }
}

#[async_trait]
impl MmCoin for EthCoin {
    fn is_asset_chain(&self) -> bool {
        false
    }

    fn spawner(&self) -> WeakSpawner {
        self.abortable_system.weak_spawner()
    }

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

    fn decimals(&self) -> u8 {
        self.decimals
    }

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
                    EthCoinType::Nft {..} => return Err(())
                }
                Ok(())
            };
            Box::new(fut.boxed().compat())
        }
    }

    fn history_sync_status(&self) -> HistorySyncState {
        self.history_sync_state.lock().unwrap().clone()
    }

    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> {
        let coin = self.clone();
        Box::new(
            async move {
                let pay_for_gas_option = coin
                    .get_swap_pay_for_gas_option(coin.get_swap_gas_fee_policy().await.map_err(|e| e.to_string())?)
                    .await
                    .map_err(|e| e.to_string())?;

                let fee = calc_total_fee(U256::from(coin.gas_limit.eth_max_trade_gas), &pay_for_gas_option)
                    .map_err(|e| e.to_string())?;
                let fee_coin = match &coin.coin_type {
                    EthCoinType::Eth => &coin.ticker,
                    EthCoinType::Erc20 { platform, .. } => platform,
                    EthCoinType::Nft { .. } => return ERR!("Nft Protocol is not supported yet!"),
                };
                Ok(TradeFee {
                    coin: fee_coin.into(),
                    amount: try_s!(u256_to_big_decimal(fee, ETH_DECIMALS)).into(),
                    paid_from_trading_vol: false,
                })
            }
            .boxed()
            .compat(),
        )
    }

    async fn get_sender_trade_fee(
        &self,
        value: TradePreimageValue,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        let pay_for_gas_option = self
            .get_swap_pay_for_gas_option(self.get_swap_gas_fee_policy().await.map_mm_err()?)
            .await
            .map_mm_err()?;
        let pay_for_gas_option = increase_gas_price_by_stage(pay_for_gas_option, &stage);
        let gas_limit = match self.coin_type {
            EthCoinType::Eth => {
                //let eth_payment_gas = self.
                // this gas_limit includes gas for `ethPayment` and optionally `senderRefund` contract calls
                if matches!(stage, FeeApproxStage::OrderIssueMax | FeeApproxStage::TradePreimageMax) {
                    U256::from(self.gas_limit.eth_payment) + U256::from(self.gas_limit.eth_sender_refund)
                } else {
                    U256::from(self.gas_limit.eth_payment)
                }
            },
            EthCoinType::Erc20 { token_addr, .. } => {
                let mut gas = U256::from(self.gas_limit.erc20_payment);
                let value = match value {
                    TradePreimageValue::Exact(value) | TradePreimageValue::UpperBound(value) => {
                        u256_from_big_decimal(&value, self.decimals).map_mm_err()?
                    },
                };
                let allowed = self.allowance(self.swap_contract_address).compat().await.map_mm_err()?;
                if allowed < value {
                    // estimate gas for the `approve` contract call

                    // Pass a dummy spender. Let's use `my_address`.
                    let spender = self.derivation_method.single_addr_or_err().await.map_mm_err()?;
                    let approve_function = ERC20_CONTRACT.function("approve")?;
                    let approve_data = approve_function.encode_input(&[Token::Address(spender), Token::Uint(value)])?;
                    let approve_gas_limit = self
                        .estimate_gas_for_contract_call(token_addr, Bytes::from(approve_data))
                        .await
                        .map_mm_err()?;

                    // this gas_limit includes gas for `approve`, `erc20Payment` contract calls
                    gas += approve_gas_limit;
                }
                // add 'senderRefund' gas if requested
                if matches!(stage, FeeApproxStage::TradePreimage | FeeApproxStage::TradePreimageMax) {
                    gas += U256::from(self.gas_limit.erc20_sender_refund);
                }
                gas
            },
            EthCoinType::Nft { .. } => return MmError::err(TradePreimageError::NftProtocolNotSupported),
        };

        let total_fee = calc_total_fee(gas_limit, &pay_for_gas_option).map_mm_err()?;
        let amount = u256_to_big_decimal(total_fee, ETH_DECIMALS).map_mm_err()?;
        let fee_coin = match &self.coin_type {
            EthCoinType::Eth => &self.ticker,
            EthCoinType::Erc20 { platform, .. } => platform,
            EthCoinType::Nft { .. } => return MmError::err(TradePreimageError::NftProtocolNotSupported),
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
            let pay_for_gas_option = coin
                .get_swap_pay_for_gas_option(coin.get_swap_gas_fee_policy().await.map_mm_err()?)
                .await
                .map_mm_err()?;
            let pay_for_gas_option = increase_gas_price_by_stage(pay_for_gas_option, &stage);
            let (fee_coin, total_fee) = match &coin.coin_type {
                EthCoinType::Eth => (
                    &coin.ticker,
                    calc_total_fee(U256::from(coin.gas_limit.eth_receiver_spend), &pay_for_gas_option).map_mm_err()?,
                ),
                EthCoinType::Erc20 { platform, .. } => (
                    platform,
                    calc_total_fee(U256::from(coin.gas_limit.erc20_receiver_spend), &pay_for_gas_option)
                        .map_mm_err()?,
                ),
                EthCoinType::Nft { .. } => return MmError::err(TradePreimageError::NftProtocolNotSupported),
            };
            let amount = u256_to_big_decimal(total_fee, ETH_DECIMALS).map_mm_err()?;
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
        dex_fee_amount: DexFee,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        let dex_fee_amount = u256_from_big_decimal(&dex_fee_amount.fee_amount().into(), self.decimals).map_mm_err()?;
        // pass the dummy params
        let to_addr = addr_from_raw_pubkey(&DEX_FEE_ADDR_RAW_PUBKEY)
            .expect("addr_from_raw_pubkey should never fail with DEX_FEE_ADDR_RAW_PUBKEY");
        let my_address = self.derivation_method.single_addr_or_err().await.map_mm_err()?;
        let (eth_value, data, call_addr, fee_coin) = match &self.coin_type {
            EthCoinType::Eth => (dex_fee_amount, Vec::new(), &to_addr, &self.ticker),
            EthCoinType::Erc20 { platform, token_addr } => {
                let function = ERC20_CONTRACT.function("transfer")?;
                let data = function.encode_input(&[Token::Address(to_addr), Token::Uint(dex_fee_amount)])?;
                (0.into(), data, token_addr, platform)
            },
            EthCoinType::Nft { .. } => return MmError::err(TradePreimageError::NftProtocolNotSupported),
        };
        let fee_policy_for_estimate =
            get_swap_fee_policy_for_estimate(self.get_swap_gas_fee_policy().await.map_mm_err()?);
        let pay_for_gas_option = self
            .get_swap_pay_for_gas_option(fee_policy_for_estimate)
            .await
            .map_mm_err()?;
        let pay_for_gas_option = increase_gas_price_by_stage(pay_for_gas_option, &stage);
        let estimate_gas_req = CallRequest {
            value: Some(eth_value),
            data: Some(data.clone().into()),
            from: Some(my_address),
            to: Some(*call_addr),
            gas: None,
            ..CallRequest::default()
        };
        // gas price must be supplied because some smart contracts base their
        // logic on gas price, e.g. TUSD: https://github.com/KomodoPlatform/atomicDEX-API/issues/643
        let estimate_gas_req = call_request_with_pay_for_gas_option(estimate_gas_req, pay_for_gas_option.clone());
        // Please note if the wallet's balance is insufficient to withdraw, then `estimate_gas` may fail with the `Exception` error.
        // Ideally we should determine the case when we have the insufficient balance and return `TradePreimageError::NotSufficientBalance` error.
        let gas_limit = self.estimate_gas_wrapper(estimate_gas_req).compat().await?;
        let total_fee = calc_total_fee(gas_limit, &pay_for_gas_option).map_mm_err()?;
        let amount = u256_to_big_decimal(total_fee, ETH_DECIMALS).map_mm_err()?;
        Ok(TradeFee {
            coin: fee_coin.into(),
            amount: amount.into(),
            paid_from_trading_vol: false,
        })
    }

    fn required_confirmations(&self) -> u64 {
        self.required_confirmations.load(AtomicOrdering::Relaxed)
    }

    fn requires_notarization(&self) -> bool {
        false
    }

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

    fn mature_confirmations(&self) -> Option<u32> {
        None
    }

    fn coin_protocol_info(&self, _amount_to_receive: Option<MmNumber>) -> Vec<u8> {
        Vec::new()
    }

    fn is_coin_protocol_supported(
        &self,
        _info: &Option<Vec<u8>>,
        _amount_to_send: Option<MmNumber>,
        _locktime: u64,
        _is_maker: bool,
    ) -> bool {
        true
    }

    fn on_disabled(&self) -> Result<(), AbortedError> {
        AbortableSystem::abort_all(&self.abortable_system)
    }

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
    fn try_to_address(&self) -> Result<Address, String> {
        self.0.try_to_address()
    }
}

impl TryToAddress for [u8] {
    fn try_to_address(&self) -> Result<Address, String> {
        (&self).try_to_address()
    }
}

impl TryToAddress for &[u8] {
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

impl Transaction for SignedEthTx {
    fn tx_hex(&self) -> Vec<u8> {
        rlp::encode(self).to_vec()
    }

    fn tx_hash_as_bytes(&self) -> BytesJson {
        self.tx_hash().as_bytes().into()
    }
}

impl ToBytes for Signature {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_vec()
    }
}

impl ToBytes for SignedEthTx {
    fn to_bytes(&self) -> Vec<u8> {
        let mut stream = RlpStream::new();
        self.rlp_append(&mut stream);
        // Handle potential panicking.
        if stream.is_finished() {
            Vec::from(stream.out())
        } else {
            // TODO: Consider returning Result<Vec<u8>, Error> in future refactoring for better error handling.
            warn!("RlpStream was not finished; returning an empty Vec as a fail-safe.");
            vec![]
        }
    }
}

#[async_trait]
impl ParseCoinAssocTypes for EthCoin {
    type Address = Address;
    type AddressParseError = MmError<EthAssocTypesError>;
    type Pubkey = Public;
    type PubkeyParseError = MmError<EthAssocTypesError>;
    type Tx = SignedEthTx;
    type TxParseError = MmError<EthAssocTypesError>;
    type Preimage = SignedEthTx;
    type PreimageParseError = MmError<EthAssocTypesError>;
    type Sig = Signature;
    type SigParseError = MmError<EthAssocTypesError>;

    async fn my_addr(&self) -> Self::Address {
        match self.derivation_method() {
            DerivationMethod::SingleAddress(addr) => *addr,
            // Todo: Expect should not fail but we need to handle it properly
            DerivationMethod::HDWallet(hd_wallet) => hd_wallet
                .get_enabled_address()
                .await
                .expect("Getting enabled address should not fail!")
                .address(),
        }
    }

    fn parse_address(&self, address: &str) -> Result<Self::Address, Self::AddressParseError> {
        // crate `Address::from_str` supports both address variants with and without `0x` prefix
        Address::from_str(address).map_to_mm(|e| EthAssocTypesError::InvalidHexString(e.to_string()))
    }

    /// As derive_htlc_pubkey_v2 returns coin specific pubkey we can use [Public::from_slice] directly
    fn parse_pubkey(&self, pubkey: &[u8]) -> Result<Self::Pubkey, Self::PubkeyParseError> {
        Ok(Public::from_slice(pubkey))
    }

    fn parse_tx(&self, tx: &[u8]) -> Result<Self::Tx, Self::TxParseError> {
        let unverified: UnverifiedTransactionWrapper = rlp::decode(tx).map_err(EthAssocTypesError::from)?;
        SignedEthTx::new(unverified).map_to_mm(|e| EthAssocTypesError::TxParseError(e.to_string()))
    }

    fn parse_preimage(&self, tx: &[u8]) -> Result<Self::Preimage, Self::PreimageParseError> {
        self.parse_tx(tx)
    }

    fn parse_signature(&self, sig: &[u8]) -> Result<Self::Sig, Self::SigParseError> {
        if sig.len() != 65 {
            return MmError::err(EthAssocTypesError::ParseSignatureError(
                "Signature slice is not 65 bytes long".to_string(),
            ));
        };

        let mut arr = [0; 65];
        arr.copy_from_slice(sig);
        Ok(Signature::from(arr)) // Assuming `Signature::from([u8; 65])` exists
    }
}

impl ToBytes for Address {
    fn to_bytes(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl AddrToString for Address {
    fn addr_to_string(&self) -> String {
        eth_addr_to_hex(self)
    }
}

impl ToBytes for BigUint {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_bytes_be()
    }
}

impl ToBytes for ContractType {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_string().into_bytes()
    }
}

impl ToBytes for Public {
    fn to_bytes(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl ParseNftAssocTypes for EthCoin {
    type ContractAddress = Address;
    type TokenId = BigUint;
    type ContractType = ContractType;
    type NftAssocTypesError = MmError<EthNftAssocTypesError>;

    fn parse_contract_address(
        &self,
        contract_address: &[u8],
    ) -> Result<Self::ContractAddress, Self::NftAssocTypesError> {
        contract_address
            .try_to_address()
            .map_to_mm(EthNftAssocTypesError::ParseTokenContractError)
    }

    fn parse_token_id(&self, token_id: &[u8]) -> Result<Self::TokenId, Self::NftAssocTypesError> {
        Ok(BigUint::from_bytes_be(token_id))
    }

    fn parse_contract_type(&self, contract_type: &[u8]) -> Result<Self::ContractType, Self::NftAssocTypesError> {
        let contract_str = from_utf8(contract_type).map_err(|e| EthNftAssocTypesError::Utf8Error(e.to_string()))?;
        ContractType::from_str(contract_str).map_to_mm(EthNftAssocTypesError::from)
    }
}

#[async_trait]
impl MakerNftSwapOpsV2 for EthCoin {
    async fn send_nft_maker_payment_v2(
        &self,
        args: SendNftMakerPaymentArgs<'_, Self>,
    ) -> Result<Self::Tx, TransactionErr> {
        self.send_nft_maker_payment_v2_impl(args).await
    }

    async fn validate_nft_maker_payment_v2(
        &self,
        args: ValidateNftMakerPaymentArgs<'_, Self>,
    ) -> ValidatePaymentResult<()> {
        self.validate_nft_maker_payment_v2_impl(args).await
    }

    async fn spend_nft_maker_payment_v2(
        &self,
        args: SpendNftMakerPaymentArgs<'_, Self>,
    ) -> Result<Self::Tx, TransactionErr> {
        self.spend_nft_maker_payment_v2_impl(args).await
    }

    async fn refund_nft_maker_payment_v2_timelock(
        &self,
        args: RefundNftMakerPaymentArgs<'_, Self>,
    ) -> Result<Self::Tx, TransactionErr> {
        self.refund_nft_maker_payment_v2_timelock_impl(args).await
    }

    async fn refund_nft_maker_payment_v2_secret(
        &self,
        args: RefundNftMakerPaymentArgs<'_, Self>,
    ) -> Result<Self::Tx, TransactionErr> {
        self.refund_nft_maker_payment_v2_secret_impl(args).await
    }
}

impl CoinWithPrivKeyPolicy for EthCoin {
    type KeyPair = KeyPair;

    fn priv_key_policy(&self) -> &PrivKeyPolicy<Self::KeyPair> {
        &self.priv_key_policy
    }
}

impl CoinWithDerivationMethod for EthCoin {
    fn derivation_method(&self) -> &DerivationMethod<HDCoinAddress<Self>, Self::HDWallet> {
        &self.derivation_method
    }
}

#[async_trait]
impl IguanaBalanceOps for EthCoin {
    type BalanceObject = CoinBalanceMap;

    async fn iguana_balances(&self) -> BalanceResult<Self::BalanceObject> {
        let platform_balance = self.my_balance().compat().await?;
        let token_balances = self.get_tokens_balance_list().await?;
        let mut balances = CoinBalanceMap::new();
        balances.insert(self.ticker().to_string(), platform_balance);
        balances.extend(token_balances);
        Ok(balances)
    }
}

#[async_trait]
impl GetNewAddressRpcOps for EthCoin {
    type BalanceObject = CoinBalanceMap;
    async fn get_new_address_rpc_without_conf(
        &self,
        params: GetNewAddressParams,
    ) -> MmResult<GetNewAddressResponse<Self::BalanceObject>, GetNewAddressRpcError> {
        get_new_address::common_impl::get_new_address_rpc_without_conf(self, params).await
    }

    async fn get_new_address_rpc<ConfirmAddress>(
        &self,
        params: GetNewAddressParams,
        confirm_address: &ConfirmAddress,
    ) -> MmResult<GetNewAddressResponse<Self::BalanceObject>, GetNewAddressRpcError>
    where
        ConfirmAddress: HDConfirmAddress,
    {
        get_new_address::common_impl::get_new_address_rpc(self, params, confirm_address).await
    }
}

#[async_trait]
impl AccountBalanceRpcOps for EthCoin {
    type BalanceObject = CoinBalanceMap;

    async fn account_balance_rpc(
        &self,
        params: AccountBalanceParams,
    ) -> MmResult<HDAccountBalanceResponse<Self::BalanceObject>, HDAccountBalanceRpcError> {
        account_balance::common_impl::account_balance_rpc(self, params).await
    }
}

#[async_trait]
impl InitAccountBalanceRpcOps for EthCoin {
    type BalanceObject = CoinBalanceMap;

    async fn init_account_balance_rpc(
        &self,
        params: InitAccountBalanceParams,
    ) -> MmResult<HDAccountBalance<Self::BalanceObject>, HDAccountBalanceRpcError> {
        init_account_balance::common_impl::init_account_balance_rpc(self, params).await
    }
}

#[async_trait]
impl InitScanAddressesRpcOps for EthCoin {
    type BalanceObject = CoinBalanceMap;

    async fn init_scan_for_new_addresses_rpc(
        &self,
        params: ScanAddressesParams,
    ) -> MmResult<ScanAddressesResponse<Self::BalanceObject>, HDAccountBalanceRpcError> {
        init_scan_for_new_addresses::common_impl::scan_for_new_addresses_rpc(self, params).await
    }
}

#[async_trait]
impl InitCreateAccountRpcOps for EthCoin {
    type BalanceObject = CoinBalanceMap;

    async fn init_create_account_rpc<XPubExtractor>(
        &self,
        params: CreateNewAccountParams,
        state: CreateAccountState,
        xpub_extractor: Option<XPubExtractor>,
    ) -> MmResult<HDAccountBalance<Self::BalanceObject>, CreateAccountRpcError>
    where
        XPubExtractor: HDXPubExtractor + Send,
    {
        init_create_account::common_impl::init_create_new_account_rpc(self, params, state, xpub_extractor).await
    }

    async fn revert_creating_account(&self, account_id: u32) {
        init_create_account::common_impl::revert_creating_account(self, account_id).await
    }
}

#[async_trait]
impl Eip1559Ops for EthCoin {
    /// Gets gas fee policy for swaps from the platform_coin, for any token
    #[cfg(not(any(test, feature = "run-docker-tests")))]
    async fn get_swap_gas_fee_policy(&self) -> CoinFindResult<SwapGasFeePolicy> {
        let platform_coin = self.platform_coin().await?;
        let swap_txfee_policy = platform_coin.swap_gas_fee_policy.lock().unwrap().clone();
        Ok(swap_txfee_policy)
    }

    #[cfg(any(test, feature = "run-docker-tests"))]
    async fn get_swap_gas_fee_policy(&self) -> CoinFindResult<SwapGasFeePolicy> {
        Ok(SwapGasFeePolicy::default())
    }

    /// Store gas fee policy for swaps in the platform_coin, for any token
    #[cfg(not(any(test, feature = "run-docker-tests")))]
    async fn set_swap_gas_fee_policy(&self, swap_txfee_policy: SwapGasFeePolicy) -> CoinFindResult<()> {
        let platform_coin = self.platform_coin().await?;
        *platform_coin.swap_gas_fee_policy.lock().unwrap() = swap_txfee_policy;
        Ok(())
    }

    #[cfg(any(test, feature = "run-docker-tests"))]
    async fn set_swap_gas_fee_policy(&self, _swap_txfee_policy: SwapGasFeePolicy) -> CoinFindResult<()> {
        Ok(())
    }
}

#[async_trait]
impl TakerCoinSwapOpsV2 for EthCoin {
    /// Wrapper for [EthCoin::send_taker_funding_impl]
    async fn send_taker_funding(&self, args: SendTakerFundingArgs<'_>) -> Result<Self::Tx, TransactionErr> {
        self.send_taker_funding_impl(args).await
    }

    /// Wrapper for [EthCoin::validate_taker_funding_impl]
    async fn validate_taker_funding(&self, args: ValidateTakerFundingArgs<'_, Self>) -> ValidateSwapV2TxResult {
        self.validate_taker_funding_impl(args).await
    }

    async fn refund_taker_funding_timelock(
        &self,
        args: RefundTakerPaymentArgs<'_>,
    ) -> Result<Self::Tx, TransactionErr> {
        self.refund_taker_payment_with_timelock_impl(args).await
    }

    async fn refund_taker_funding_secret(
        &self,
        args: RefundFundingSecretArgs<'_, Self>,
    ) -> Result<Self::Tx, TransactionErr> {
        self.refund_taker_funding_secret_impl(args).await
    }

    /// Wrapper for [EthCoin::search_for_taker_funding_spend_impl]
    async fn search_for_taker_funding_spend(
        &self,
        tx: &Self::Tx,
        _from_block: u64,
        _secret_hash: &[u8],
    ) -> Result<Option<FundingTxSpend<Self>>, SearchForFundingSpendErr> {
        self.search_for_taker_funding_spend_impl(tx).await
    }

    /// Eth doesnt have preimages
    async fn gen_taker_funding_spend_preimage(
        &self,
        args: &GenTakerFundingSpendArgs<'_, Self>,
        _swap_unique_data: &[u8],
    ) -> GenPreimageResult<Self> {
        Ok(TxPreimageWithSig {
            preimage: args.funding_tx.clone(),
            signature: args.funding_tx.signature(),
        })
    }

    /// Eth doesnt have preimages
    async fn validate_taker_funding_spend_preimage(
        &self,
        _gen_args: &GenTakerFundingSpendArgs<'_, Self>,
        _preimage: &TxPreimageWithSig<Self>,
    ) -> ValidateTakerFundingSpendPreimageResult {
        Ok(())
    }

    /// Wrapper for [EthCoin::taker_payment_approve]
    async fn sign_and_send_taker_funding_spend(
        &self,
        _preimage: &TxPreimageWithSig<Self>,
        args: &GenTakerFundingSpendArgs<'_, Self>,
        _swap_unique_data: &[u8],
    ) -> Result<Self::Tx, TransactionErr> {
        self.taker_payment_approve(args).await
    }

    async fn refund_combined_taker_payment(
        &self,
        args: RefundTakerPaymentArgs<'_>,
    ) -> Result<Self::Tx, TransactionErr> {
        self.refund_taker_payment_with_timelock_impl(args).await
    }

    fn skip_taker_payment_spend_preimage(&self) -> bool {
        true
    }

    /// Eth skips taker_payment_spend_preimage, as it doesnt need it
    async fn gen_taker_payment_spend_preimage(
        &self,
        _args: &GenTakerPaymentSpendArgs<'_, Self>,
        _swap_unique_data: &[u8],
    ) -> GenPreimageResult<Self> {
        MmError::err(TxGenError::Other(
            "EVM-based coin doesn't have taker_payment_spend_preimage. Report the Bug!".to_string(),
        ))
    }

    /// Eth skips taker_payment_spend_preimage, as it doesnt need it
    async fn validate_taker_payment_spend_preimage(
        &self,
        _gen_args: &GenTakerPaymentSpendArgs<'_, Self>,
        _preimage: &TxPreimageWithSig<Self>,
    ) -> ValidateTakerPaymentSpendPreimageResult {
        MmError::err(ValidateTakerPaymentSpendPreimageError::InvalidPreimage(
            "EVM-based coin skips taker_payment_spend_preimage validation. Report the Bug!".to_string(),
        ))
    }

    /// Eth doesnt have preimages
    async fn sign_and_broadcast_taker_payment_spend(
        &self,
        _preimage: Option<&TxPreimageWithSig<Self>>,
        gen_args: &GenTakerPaymentSpendArgs<'_, Self>,
        secret: &[u8],
        _swap_unique_data: &[u8],
    ) -> Result<Self::Tx, TransactionErr> {
        self.sign_and_broadcast_taker_payment_spend_impl(gen_args, secret).await
    }

    /// Wrapper for [EthCoin::find_taker_payment_spend_tx_impl]
    async fn find_taker_payment_spend_tx(
        &self,
        taker_payment: &Self::Tx,
        from_block: u64,
        wait_until: u64,
    ) -> MmResult<Self::Tx, FindPaymentSpendError> {
        const CHECK_EVERY: f64 = 10.;
        self.find_taker_payment_spend_tx_impl(taker_payment, from_block, wait_until, CHECK_EVERY)
            .await
    }

    async fn extract_secret_v2(&self, _secret_hash: &[u8], spend_tx: &Self::Tx) -> Result<[u8; 32], String> {
        self.extract_secret_v2_impl(spend_tx).await
    }
}

impl CommonSwapOpsV2 for EthCoin {
    #[inline(always)]
    fn derive_htlc_pubkey_v2(&self, _swap_unique_data: &[u8]) -> Self::Pubkey {
        match self.priv_key_policy {
            EthPrivKeyPolicy::Iguana(ref key_pair)
            | EthPrivKeyPolicy::HDWallet {
                activated_key: ref key_pair,
                ..
            } => *key_pair.public(),
            EthPrivKeyPolicy::Trezor => todo!(),
            #[cfg(target_arch = "wasm32")]
            EthPrivKeyPolicy::Metamask(ref metamask_policy) => {
                // The metamask public key should be uncompressed
                // Remove the first byte (0x04) from the uncompressed public key
                let pubkey_bytes: [u8; 64] = metamask_policy.public_key_uncompressed[1..65]
                    .try_into()
                    .expect("slice with incorrect length");
                Public::from_slice(&pubkey_bytes)
            },
            EthPrivKeyPolicy::WalletConnect {
                public_key_uncompressed,
                ..
            } => {
                let pubkey_bytes: [u8; 64] = public_key_uncompressed[1..65]
                    .try_into()
                    .expect("slice with incorrect length");
                Public::from_slice(&pubkey_bytes)
            },
        }
    }

    #[inline(always)]
    fn derive_htlc_pubkey_v2_bytes(&self, swap_unique_data: &[u8]) -> Vec<u8> {
        self.derive_htlc_pubkey_v2(swap_unique_data).to_bytes()
    }

    #[inline(always)]
    fn taker_pubkey_bytes(&self) -> Option<Vec<u8>> {
        Some(self.derive_htlc_pubkey_v2(&[]).to_bytes()) // unique_data not used for non-private coins
    }
}

#[async_trait]
impl MakerCoinSwapOpsV2 for EthCoin {
    async fn send_maker_payment_v2(&self, args: SendMakerPaymentArgs<'_, Self>) -> Result<Self::Tx, TransactionErr> {
        self.send_maker_payment_v2_impl(args).await
    }

    async fn validate_maker_payment_v2(&self, args: ValidateMakerPaymentArgs<'_, Self>) -> ValidatePaymentResult<()> {
        self.validate_maker_payment_v2_impl(args).await
    }

    async fn refund_maker_payment_v2_timelock(
        &self,
        args: RefundMakerPaymentTimelockArgs<'_>,
    ) -> Result<Self::Tx, TransactionErr> {
        self.refund_maker_payment_v2_timelock_impl(args).await
    }

    async fn refund_maker_payment_v2_secret(
        &self,
        args: RefundMakerPaymentSecretArgs<'_, Self>,
    ) -> Result<Self::Tx, TransactionErr> {
        self.refund_maker_payment_v2_secret_impl(args).await
    }

    async fn spend_maker_payment_v2(&self, args: SpendMakerPaymentArgs<'_, Self>) -> Result<Self::Tx, TransactionErr> {
        self.spend_maker_payment_v2_impl(args).await
    }
}
