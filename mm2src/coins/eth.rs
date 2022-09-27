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
use async_trait::async_trait;
use bitcrypto::{keccak256, sha256};
use common::executor::Timer;
use common::log::{error, info, warn};
use common::{get_utc_timestamp, now_ms, small_rng, DEX_FEE_ADDR_RAW_PUBKEY};
use crypto::privkey::key_pair_from_secret;
use derive_more::Display;
use ethabi::{Contract, Token};
pub use ethcore_transaction::SignedTransaction as SignedEthTx;
use ethcore_transaction::{Action, Transaction as UnSignedEthTx, UnverifiedTransaction};
use ethereum_types::{Address, H160, H256, U256};
use ethkey::{public_to_address, KeyPair, Public, Signature};
use ethkey::{sign, verify_address};
use futures::compat::Future01CompatExt;
use futures::future::{join_all, select, Either, FutureExt, TryFutureExt};
use futures01::Future;
use http::StatusCode;
use mm2_core::mm_ctx::{MmArc, MmWeak};
use mm2_err_handle::prelude::*;
use mm2_net::transport::{slurp_url, GuiAuthValidation, GuiAuthValidationGenerator, SlurpError};
use mm2_number::{BigDecimal, MmNumber};
#[cfg(test)] use mocktopus::macros::*;
use rand::seq::SliceRandom;
use rpc::v1::types::Bytes as BytesJson;
use secp256k1::PublicKey;
use serde_json::{self as json, Value as Json};
use serialization::{CompactInteger, Serializable, Stream};
use sha3::{Digest, Keccak256};
use std::collections::HashMap;
use std::ops::Deref;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use web3::types::{Action as TraceAction, BlockId, BlockNumber, Bytes, CallRequest, FilterBuilder, Log, Trace,
                  TraceFilterBuilder, Transaction as Web3Transaction, TransactionId};
use web3::{self, Web3};
use web3_transport::{EthFeeHistoryNamespace, Web3Transport, Web3TransportNode};

use super::{coin_conf, AsyncMutex, BalanceError, BalanceFut, CoinBalance, CoinProtocol, CoinTransportMetrics,
            CoinsContext, FeeApproxStage, FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin, MyAddressError,
            NegotiateSwapContractAddrErr, NumConversError, NumConversResult, RawTransactionError, RawTransactionFut,
            RawTransactionRequest, RawTransactionRes, RawTransactionResult, RpcClientType, RpcTransportEventHandler,
            RpcTransportEventHandlerShared, SearchForSwapTxSpendInput, SignatureError, SignatureResult, SwapOps,
            TradeFee, TradePreimageError, TradePreimageFut, TradePreimageResult, TradePreimageValue, Transaction,
            TransactionDetails, TransactionEnum, TransactionErr, TransactionFut, TxMarshalingErr,
            UnexpectedDerivationMethod, ValidateAddressResult, ValidatePaymentError, ValidatePaymentFut,
            ValidatePaymentInput, VerificationError, VerificationResult, WatcherValidatePaymentInput, WithdrawError,
            WithdrawFee, WithdrawFut, WithdrawRequest, WithdrawResult};

pub use rlp;

#[cfg(test)] mod eth_tests;
#[cfg(target_arch = "wasm32")] mod eth_wasm_tests;
mod web3_transport;

#[path = "eth/v2_activation.rs"] pub mod v2_activation;

/// https://github.com/artemii235/etomic-swap/blob/master/contracts/EtomicSwap.sol
/// Dev chain (195.201.0.6:8565) contract address: 0xa09ad3cd7e96586ebd05a2607ee56b56fb2db8fd
/// Ropsten: https://ropsten.etherscan.io/address/0x7bc1bbdd6a0a722fc9bffc49c921b685ecb84b94
/// ETH mainnet: https://etherscan.io/address/0x8500AFc0bc5214728082163326C2FF0C73f4a871
const SWAP_CONTRACT_ABI: &str = r#"[{"constant":false,"inputs":[{"name":"_id","type":"bytes32"},{"name":"_amount","type":"uint256"},{"name":"_secret","type":"bytes32"},{"name":"_tokenAddress","type":"address"},{"name":"_sender","type":"address"}],"name":"receiverSpend","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"","type":"bytes32"}],"name":"payments","outputs":[{"name":"paymentHash","type":"bytes20"},{"name":"lockTime","type":"uint64"},{"name":"state","type":"uint8"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"_id","type":"bytes32"},{"name":"_receiver","type":"address"},{"name":"_secretHash","type":"bytes20"},{"name":"_lockTime","type":"uint64"}],"name":"ethPayment","outputs":[],"payable":true,"stateMutability":"payable","type":"function"},{"constant":false,"inputs":[{"name":"_id","type":"bytes32"},{"name":"_amount","type":"uint256"},{"name":"_paymentHash","type":"bytes20"},{"name":"_tokenAddress","type":"address"},{"name":"_receiver","type":"address"}],"name":"senderRefund","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":false,"inputs":[{"name":"_id","type":"bytes32"},{"name":"_amount","type":"uint256"},{"name":"_tokenAddress","type":"address"},{"name":"_receiver","type":"address"},{"name":"_secretHash","type":"bytes20"},{"name":"_lockTime","type":"uint64"}],"name":"erc20Payment","outputs":[],"payable":true,"stateMutability":"payable","type":"function"},{"inputs":[],"payable":false,"stateMutability":"nonpayable","type":"constructor"},{"anonymous":false,"inputs":[{"indexed":false,"name":"id","type":"bytes32"}],"name":"PaymentSent","type":"event"},{"anonymous":false,"inputs":[{"indexed":false,"name":"id","type":"bytes32"},{"indexed":false,"name":"secret","type":"bytes32"}],"name":"ReceiverSpent","type":"event"},{"anonymous":false,"inputs":[{"indexed":false,"name":"id","type":"bytes32"}],"name":"SenderRefunded","type":"event"}]"#;
/// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-20.md
const ERC20_ABI: &str = r#"[{"constant":true,"inputs":[],"name":"name","outputs":[{"name":"","type":"string"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"_spender","type":"address"},{"name":"_value","type":"uint256"}],"name":"approve","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[],"name":"totalSupply","outputs":[{"name":"","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"_from","type":"address"},{"name":"_to","type":"address"},{"name":"_value","type":"uint256"}],"name":"transferFrom","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[],"name":"decimals","outputs":[{"name":"","type":"uint8"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"_spender","type":"address"},{"name":"_subtractedValue","type":"uint256"}],"name":"decreaseApproval","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"_owner","type":"address"}],"name":"balanceOf","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[],"name":"symbol","outputs":[{"name":"","type":"string"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"_to","type":"address"},{"name":"_value","type":"uint256"}],"name":"transfer","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":false,"inputs":[{"name":"_spender","type":"address"},{"name":"_addedValue","type":"uint256"}],"name":"increaseApproval","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"_owner","type":"address"},{"name":"_spender","type":"address"}],"name":"allowance","outputs":[{"name":"","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"inputs":[],"payable":false,"stateMutability":"nonpayable","type":"constructor"},{"anonymous":false,"inputs":[{"indexed":true,"name":"owner","type":"address"},{"indexed":true,"name":"spender","type":"address"},{"indexed":false,"name":"value","type":"uint256"}],"name":"Approval","type":"event"},{"anonymous":false,"inputs":[{"indexed":true,"name":"from","type":"address"},{"indexed":true,"name":"to","type":"address"},{"indexed":false,"name":"value","type":"uint256"}],"name":"Transfer","type":"event"}]"#;

/// Payment states from etomic swap smart contract: https://github.com/artemii235/etomic-swap/blob/master/contracts/EtomicSwap.sol#L5
pub const PAYMENT_STATE_UNINITIALIZED: u8 = 0;
pub const PAYMENT_STATE_SENT: u8 = 1;
const _PAYMENT_STATE_SPENT: u8 = 2;
const _PAYMENT_STATE_REFUNDED: u8 = 3;
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
/// Take into account that the dynamic fee may increase at each of the following stages:
/// - it may increase by 2% until a swap is started;
/// - it may increase by 3% during the swap.
const GAS_PRICE_APPROXIMATION_PERCENT_ON_ORDER_ISSUE: u64 = 5;
/// Take into account that the dynamic fee may increase at each of the following stages:
/// - it may increase by 2% until an order is issued;
/// - it may increase by 2% until a swap is started;
/// - it may increase by 3% during the swap.
const GAS_PRICE_APPROXIMATION_PERCENT_ON_TRADE_PREIMAGE: u64 = 7;

/// Lifetime of generated signed message for gui-auth requests
const GUI_AUTH_SIGNED_MESSAGE_LIFETIME_SEC: i64 = 90;

lazy_static! {
    pub static ref SWAP_CONTRACT: Contract = Contract::load(SWAP_CONTRACT_ABI.as_bytes()).unwrap();
    pub static ref ERC20_CONTRACT: Contract = Contract::load(ERC20_ABI.as_bytes()).unwrap();
}

pub type Web3RpcFut<T> = Box<dyn Future<Item = T, Error = MmError<Web3RpcError>> + Send>;
pub type Web3RpcResult<T> = Result<T, MmError<Web3RpcError>>;
pub type GasStationResult = Result<GasStationData, MmError<GasStationReqErr>>;

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
        match e.kind() {
            web3::ErrorKind::InvalidResponse(_)
            | web3::ErrorKind::Decoder(_)
            | web3::ErrorKind::Msg(_)
            | web3::ErrorKind::Rpc(_) => Web3RpcError::InvalidResponse(error_str),
            web3::ErrorKind::Transport(_) | web3::ErrorKind::Io(_) => Web3RpcError::Transport(error_str),
            _ => Web3RpcError::Internal(error_str),
        }
    }
}

impl From<web3::Error> for RawTransactionError {
    fn from(e: web3::Error) -> Self { RawTransactionError::Transport(e.to_string()) }
}

impl From<ethabi::Error> for Web3RpcError {
    fn from(e: ethabi::Error) -> Web3RpcError {
        // Currently, we use the `ethabi` crate to work with a smart contract ABI known at compile time.
        // It's an internal error if there are any issues during working with a smart contract ABI.
        Web3RpcError::Internal(e.to_string())
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
            Web3RpcError::Internal(internal) => WithdrawError::InternalError(internal),
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
            Web3RpcError::Internal(internal) => TradePreimageError::InternalError(internal),
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
    fn from(e: web3::Error) -> Self { BalanceError::Transport(e.to_string()) }
}

#[derive(Debug, Deserialize, Serialize)]
struct SavedTraces {
    /// ETH traces for my_address
    traces: Vec<Trace>,
    /// Earliest processed block
    earliest_block: U256,
    /// Latest processed block
    latest_block: U256,
}

#[derive(Debug, Deserialize, Serialize)]
struct SavedErc20Events {
    /// ERC20 events for my_address
    events: Vec<Log>,
    /// Earliest processed block
    earliest_block: U256,
    /// Latest processed block
    latest_block: U256,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EthCoinType {
    /// Ethereum itself or it's forks: ETC/others
    Eth,
    /// ERC20 token with smart contract address
    /// https://github.com/ethereum/EIPs/blob/master/EIPS/eip-20.md
    Erc20 { platform: String, token_addr: Address },
}

/// pImpl idiom.
pub struct EthCoinImpl {
    ticker: String,
    coin_type: EthCoinType,
    key_pair: KeyPair,
    my_address: Address,
    sign_message_prefix: Option<String>,
    swap_contract_address: Address,
    fallback_swap_contract: Option<Address>,
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

#[cfg_attr(test, mockable)]
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

        Box::new(self.web3.eth().logs(filter.build()).map_err(|e| ERRL!("{}", e)))
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

        Box::new(self.web3.trace().filter(filter.build()).map_err(|e| ERRL!("{}", e)))
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
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

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
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

    fn estimate_gas(&self, req: CallRequest) -> Box<dyn Future<Item = U256, Error = web3::Error> + Send> {
        // always using None block number as old Geth version accept only single argument in this RPC
        Box::new(self.web3.eth().estimate_gas(req, None))
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
            .from_block(BlockNumber::Number(from_block))
            .to_block(BlockNumber::Number(to_block))
            .address(vec![swap_contract_address])
            .build();

        Box::new(self.web3.eth().logs(filter).map_err(|e| ERRL!("{}", e)))
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
            .from_block(BlockNumber::Number(from_block))
            .to_block(BlockNumber::Number(to_block))
            .address(vec![swap_contract_address])
            .build();

        Box::new(self.web3.eth().logs(filter).map_err(|e| ERRL!("{}", e)))
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

    /// WARNING
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
    let web3_tx = coin.web3.eth().transaction(TransactionId::Hash(hash)).compat().await?;
    let web3_tx = web3_tx.or_mm_err(|| RawTransactionError::HashNotExist(req.tx_hash))?;
    let raw = signed_tx_from_web3_tx(web3_tx).map_to_mm(RawTransactionError::InternalError)?;
    Ok(RawTransactionRes {
        tx_hex: BytesJson(rlp::encode(&raw)),
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

    let (gas, gas_price) = match req.fee {
        Some(WithdrawFee::EthGas { gas_price, gas }) => {
            let gas_price = wei_from_big_decimal(&gas_price, 9)?;
            (gas.into(), gas_price)
        },
        Some(fee_policy) => {
            let error = format!("Expected 'EthGas' fee type, found {:?}", fee_policy);
            return MmError::err(WithdrawError::InvalidFeePolicy(error));
        },
        None => {
            let gas_price = coin.get_gas_price().compat().await?;
            // covering edge case by deducting the standard transfer fee when we want to max withdraw ETH
            let eth_value_for_estimate = if req.max && coin.coin_type == EthCoinType::Eth {
                eth_value - gas_price * U256::from(21000)
            } else {
                eth_value
            };
            let estimate_gas_req = CallRequest {
                value: Some(eth_value_for_estimate),
                data: Some(data.clone().into()),
                from: Some(coin.my_address),
                to: call_addr,
                gas: None,
                // gas price must be supplied because some smart contracts base their
                // logic on gas price, e.g. TUSD: https://github.com/KomodoPlatform/atomicDEX-API/issues/643
                gas_price: Some(gas_price),
            };
            // TODO Note if the wallet's balance is insufficient to withdraw, then `estimate_gas` may fail with the `Exception` error.
            // TODO Ideally we should determine the case when we have the insufficient balance and return `WithdrawError::NotSufficientBalance`.
            let gas_limit = coin.estimate_gas(estimate_gas_req).compat().await?;
            (gas_limit, gas_price)
        },
    };
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
    let _nonce_lock = coin.nonce_lock.lock().await;
    let nonce_fut = get_addr_nonce(coin.my_address, coin.web3_instances.clone()).compat();
    let nonce = match select(nonce_fut, Timer::sleep(30.)).await {
        Either::Left((nonce_res, _)) => nonce_res.map_to_mm(WithdrawError::Transport)?,
        Either::Right(_) => return MmError::err(WithdrawError::Transport("Get address nonce timed out".to_owned())),
    };
    let tx = UnSignedEthTx {
        nonce,
        value: eth_value,
        action: Action::Call(call_addr),
        data,
        gas,
        gas_price,
    };

    let signed = tx.sign(coin.key_pair.secret(), coin.chain_id);
    let bytes = rlp::encode(&signed);
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
        tx_hex: bytes.into(),
        tx_hash: format!("{:02x}", signed.tx_hash()),
        block_height: 0,
        fee_details: Some(fee_details.into()),
        coin: coin.ticker.clone(),
        internal_id: vec![].into(),
        timestamp: now_ms() / 1000,
        kmd_rewards: None,
        transaction_type: Default::default(),
    })
}

#[derive(Clone)]
pub struct EthCoin(Arc<EthCoinImpl>);
impl Deref for EthCoin {
    type Target = EthCoinImpl;
    fn deref(&self) -> &EthCoinImpl { &*self.0 }
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

    fn send_maker_payment(
        &self,
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        let taker_addr = try_tx_fus!(addr_from_raw_pubkey(taker_pub));
        let swap_contract_address = try_tx_fus!(swap_contract_address.try_to_address());

        Box::new(
            self.send_hash_time_locked_payment(
                self.etomic_swap_id(time_lock, secret_hash),
                try_tx_fus!(wei_from_big_decimal(&amount, self.decimals)),
                time_lock,
                secret_hash,
                taker_addr,
                swap_contract_address,
            )
            .map(TransactionEnum::from),
        )
    }

    fn send_taker_payment(
        &self,
        time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        let maker_addr = try_tx_fus!(addr_from_raw_pubkey(maker_pub));
        let swap_contract_address = try_tx_fus!(swap_contract_address.try_to_address());

        Box::new(
            self.send_hash_time_locked_payment(
                self.etomic_swap_id(time_lock, secret_hash),
                try_tx_fus!(wei_from_big_decimal(&amount, self.decimals)),
                time_lock,
                secret_hash,
                maker_addr,
                swap_contract_address,
            )
            .map(TransactionEnum::from),
        )
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        _time_lock: u32,
        _taker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx: UnverifiedTransaction = try_tx_fus!(rlp::decode(taker_payment_tx));
        let signed = try_tx_fus!(SignedEthTx::new(tx));
        let swap_contract_address = try_tx_fus!(swap_contract_address.try_to_address(), signed);

        Box::new(
            self.spend_hash_time_locked_payment(signed, swap_contract_address, secret)
                .map(TransactionEnum::from),
        )
    }

    fn send_taker_spends_maker_payment_preimage(&self, _preimage: &[u8], _secret: &[u8]) -> TransactionFut {
        unimplemented!();
    }

    fn create_taker_spends_maker_payment_preimage(
        &self,
        _maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        secret: &[u8],
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx: UnverifiedTransaction = try_tx_fus!(rlp::decode(maker_payment_tx));
        let signed = try_tx_fus!(SignedEthTx::new(tx));
        let swap_contract_address = try_tx_fus!(swap_contract_address.try_to_address());
        Box::new(
            self.spend_hash_time_locked_payment(signed, swap_contract_address, secret)
                .map(TransactionEnum::from),
        )
    }

    fn send_taker_refunds_payment(
        &self,
        taker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx: UnverifiedTransaction = try_tx_fus!(rlp::decode(taker_payment_tx));
        let signed = try_tx_fus!(SignedEthTx::new(tx));
        let swap_contract_address = try_tx_fus!(swap_contract_address.try_to_address());

        Box::new(
            self.refund_hash_time_locked_payment(swap_contract_address, signed)
                .map(TransactionEnum::from),
        )
    }

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        _time_lock: u32,
        _taker_pub: &[u8],
        _secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx: UnverifiedTransaction = try_tx_fus!(rlp::decode(maker_payment_tx));
        let signed = try_tx_fus!(SignedEthTx::new(tx));
        let swap_contract_address = try_tx_fus!(swap_contract_address.try_to_address());

        Box::new(
            self.refund_hash_time_locked_payment(swap_contract_address, signed)
                .map(TransactionEnum::from),
        )
    }

    fn validate_fee(
        &self,
        fee_tx: &TransactionEnum,
        expected_sender: &[u8],
        fee_addr: &[u8],
        amount: &BigDecimal,
        min_block_number: u64,
        _uuid: &[u8],
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let selfi = self.clone();
        let tx = match fee_tx {
            TransactionEnum::SignedEthTx(t) => t.clone(),
            _ => panic!(),
        };
        let sender_addr = try_fus!(addr_from_raw_pubkey(expected_sender));
        let fee_addr = try_fus!(addr_from_raw_pubkey(fee_addr));
        let amount = amount.clone();

        let fut = async move {
            let expected_value = try_s!(wei_from_big_decimal(&amount, selfi.decimals));
            let tx_from_rpc = try_s!(
                selfi
                    .web3
                    .eth()
                    .transaction(TransactionId::Hash(tx.hash))
                    .compat()
                    .await
            );
            let tx_from_rpc = match tx_from_rpc {
                Some(t) => t,
                None => return ERR!("Didn't find provided tx {:?} on ETH node", tx),
            };

            if tx_from_rpc.from != sender_addr {
                return ERR!(
                    "Fee tx {:?} was sent from wrong address, expected {:?}",
                    tx_from_rpc,
                    sender_addr
                );
            }

            if let Some(block_number) = tx_from_rpc.block_number {
                if block_number <= min_block_number.into() {
                    return ERR!(
                        "Fee tx {:?} confirmed before min_block {}",
                        tx_from_rpc,
                        min_block_number,
                    );
                }
            }
            match &selfi.coin_type {
                EthCoinType::Eth => {
                    if tx_from_rpc.to != Some(fee_addr) {
                        return ERR!(
                            "Fee tx {:?} was sent to wrong address, expected {:?}",
                            tx_from_rpc,
                            fee_addr
                        );
                    }

                    if tx_from_rpc.value < expected_value {
                        return ERR!(
                            "Fee tx {:?} value is less than expected {:?}",
                            tx_from_rpc,
                            expected_value
                        );
                    }
                },
                EthCoinType::Erc20 {
                    platform: _,
                    token_addr,
                } => {
                    if tx_from_rpc.to != Some(*token_addr) {
                        return ERR!(
                            "ERC20 Fee tx {:?} called wrong smart contract, expected {:?}",
                            tx_from_rpc,
                            token_addr
                        );
                    }

                    let function = try_s!(ERC20_CONTRACT.function("transfer"));
                    let decoded_input = try_s!(function.decode_input(&tx_from_rpc.input.0));

                    if decoded_input[0] != Token::Address(fee_addr) {
                        return ERR!(
                            "ERC20 Fee tx was sent to wrong address {:?}, expected {:?}",
                            decoded_input[0],
                            fee_addr
                        );
                    }

                    match decoded_input[1] {
                        Token::Uint(value) => {
                            if value < expected_value {
                                return ERR!("ERC20 Fee tx value {} is less than expected {}", value, expected_value);
                            }
                        },
                        _ => return ERR!("Should have got uint token but got {:?}", decoded_input[1]),
                    }
                },
            }

            Ok(())
        };
        Box::new(fut.boxed().compat())
    }

    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        let swap_contract_address = try_f!(input
            .swap_contract_address
            .try_to_address()
            .map_to_mm(ValidatePaymentError::InvalidInput));
        self.validate_payment(
            &input.payment_tx,
            input.time_lock,
            &input.other_pub,
            &input.secret_hash,
            input.amount,
            swap_contract_address,
        )
    }

    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        let swap_contract_address = try_f!(input
            .swap_contract_address
            .try_to_address()
            .map_to_mm(ValidatePaymentError::InvalidInput));
        self.validate_payment(
            &input.payment_tx,
            input.time_lock,
            &input.other_pub,
            &input.secret_hash,
            input.amount,
            swap_contract_address,
        )
    }

    fn watcher_validate_taker_payment(
        &self,
        _input: WatcherValidatePaymentInput,
    ) -> Box<dyn Future<Item = (), Error = MmError<ValidatePaymentError>> + Send> {
        unimplemented!();
    }

    fn check_if_my_payment_sent(
        &self,
        time_lock: u32,
        _other_pub: &[u8],
        secret_hash: &[u8],
        from_block: u64,
        swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        let id = self.etomic_swap_id(time_lock, secret_hash);
        let swap_contract_address = try_fus!(swap_contract_address.try_to_address());
        let selfi = self.clone();
        let fut = async move {
            let status = try_s!(
                selfi
                    .payment_status(swap_contract_address, Token::FixedBytes(id.clone()))
                    .compat()
                    .await
            );

            if status == PAYMENT_STATE_UNINITIALIZED.into() {
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
                                .compat()
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
        self.search_for_swap_tx_spend(input.tx, swap_contract_address, input.search_from_block)
            .await
    }

    async fn search_for_swap_tx_spend_other(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        let swap_contract_address = try_s!(input.swap_contract_address.try_to_address());
        self.search_for_swap_tx_spend(input.tx, swap_contract_address, input.search_from_block)
            .await
    }

    fn extract_secret(&self, _secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> {
        let unverified: UnverifiedTransaction = try_s!(rlp::decode(spend_tx));
        let function = try_s!(SWAP_CONTRACT.function("receiverSpend"));
        let tokens = try_s!(function.decode_input(&unverified.data));
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

    fn negotiate_swap_contract_addr(
        &self,
        other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        match other_side_address {
            Some(bytes) => {
                if bytes.len() != 20 {
                    return MmError::err(NegotiateSwapContractAddrErr::InvalidOtherAddrLen(bytes.into()));
                }
                let other_addr = Address::from(bytes);
                if other_addr == self.swap_contract_address {
                    return Ok(Some(self.swap_contract_address.to_vec().into()));
                }

                if Some(other_addr) == self.fallback_swap_contract {
                    return Ok(self.fallback_swap_contract.map(|addr| addr.to_vec().into()));
                }
                MmError::err(NegotiateSwapContractAddrErr::UnexpectedOtherAddr(bytes.into()))
            },
            None => self
                .fallback_swap_contract
                .map(|addr| Some(addr.to_vec().into()))
                .ok_or_else(|| MmError::new(NegotiateSwapContractAddrErr::NoOtherAddrAndNoFallback)),
        }
    }

    fn derive_htlc_key_pair(&self, _swap_unique_data: &[u8]) -> keys::KeyPair {
        key_pair_from_secret(self.key_pair.secret()).expect("valid key")
    }
}

#[cfg_attr(test, mockable)]
impl MarketCoinOps for EthCoin {
    fn ticker(&self) -> &str { &self.ticker[..] }

    fn my_address(&self) -> MmResult<String, MyAddressError> {
        Ok(checksum_address(&format!("{:#02x}", self.my_address)))
    }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> {
        let uncompressed_without_prefix = hex::encode(self.key_pair.public());
        Ok(format!("04{}", uncompressed_without_prefix))
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
        let privkey = &self.key_pair.secret();
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
                .map(|res| format!("{:02x}", res))
                .map_err(|e| ERRL!("{}", e)),
        )
    }

    fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> {
        Box::new(
            self.web3
                .eth()
                .send_raw_transaction(tx.into())
                .map(|res| format!("{:02x}", res))
                .map_err(|e| ERRL!("{}", e)),
        )
    }

    fn wait_for_confirmations(
        &self,
        tx: &[u8],
        confirmations: u64,
        _requires_nota: bool,
        wait_until: u64,
        check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let ctx = try_fus!(MmArc::from_weak(&self.ctx).ok_or("No context"));
        let mut status = ctx.log.status_handle();
        status.status(&[&self.ticker], "Waiting for confirmations…");
        status.deadline(wait_until * 1000);

        let unsigned: UnverifiedTransaction = try_fus!(rlp::decode(tx));
        let tx = try_fus!(SignedEthTx::new(unsigned));

        let required_confirms = U256::from(confirmations);
        let selfi = self.clone();
        let fut = async move {
            loop {
                if status.ms2deadline().unwrap() < 0 {
                    status.append(" Timed out.");
                    return ERR!(
                        "Waited too long until {} for transaction {:?} confirmation ",
                        wait_until,
                        tx
                    );
                }

                let web3_receipt = match selfi.web3.eth().transaction_receipt(tx.hash()).compat().await {
                    Ok(r) => r,
                    Err(e) => {
                        error!(
                            "Error {:?} getting the {} transaction {:?}, retrying in 15 seconds",
                            e,
                            selfi.ticker(),
                            tx.tx_hash()
                        );
                        Timer::sleep(check_every as f64).await;
                        continue;
                    },
                };
                if let Some(receipt) = web3_receipt {
                    if receipt.status != Some(1.into()) {
                        status.append(" Failed.");
                        return ERR!(
                            "Tx receipt {:?} status of {} tx {:?} is failed",
                            receipt,
                            selfi.ticker(),
                            tx.tx_hash()
                        );
                    }

                    if let Some(confirmed_at) = receipt.block_number {
                        let current_block = match selfi.web3.eth().block_number().compat().await {
                            Ok(b) => b,
                            Err(e) => {
                                error!(
                                    "Error {:?} getting the {} block number retrying in 15 seconds",
                                    e,
                                    selfi.ticker()
                                );
                                Timer::sleep(check_every as f64).await;
                                continue;
                            },
                        };
                        // checking if the current block is above the confirmed_at block prediction for pos chain to prevent overflow
                        if current_block >= confirmed_at && current_block - confirmed_at + 1 >= required_confirms {
                            status.append(" Confirmed.");
                            return Ok(());
                        }
                    }
                }
                Timer::sleep(check_every as f64).await;
            }
        };
        Box::new(fut.boxed().compat())
    }

    fn wait_for_tx_spend(
        &self,
        tx_bytes: &[u8],
        wait_until: u64,
        from_block: u64,
        swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        let unverified: UnverifiedTransaction = try_tx_fus!(rlp::decode(tx_bytes));
        let tx = try_tx_fus!(SignedEthTx::new(unverified));
        let swap_contract_address = try_tx_fus!(swap_contract_address.try_to_address());

        let func_name = match self.coin_type {
            EthCoinType::Eth => "ethPayment",
            EthCoinType::Erc20 { .. } => "erc20Payment",
        };

        let payment_func = try_tx_fus!(SWAP_CONTRACT.function(func_name));
        let decoded = try_tx_fus!(payment_func.decode_input(&tx.data));
        let id = match &decoded[0] {
            Token::FixedBytes(bytes) => bytes.clone(),
            _ => panic!(),
        };
        let selfi = self.clone();

        let fut = async move {
            loop {
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
                        let transaction = match selfi
                            .web3
                            .eth()
                            .transaction(TransactionId::Hash(tx_hash))
                            .compat()
                            .await
                        {
                            Ok(Some(t)) => t,
                            Ok(None) => {
                                info!("Tx {} not found yet", tx_hash);
                                Timer::sleep(5.).await;
                                continue;
                            },
                            Err(e) => {
                                error!("Get tx {} error: {}", tx_hash, e);
                                Timer::sleep(5.).await;
                                continue;
                            },
                        };

                        return Ok(TransactionEnum::from(try_tx_s!(signed_tx_from_web3_tx(transaction))));
                    }
                }

                if now_ms() / 1000 > wait_until {
                    return TX_PLAIN_ERR!(
                        "Waited too long until {} for transaction {:?} to be spent ",
                        wait_until,
                        tx,
                    );
                }
                Timer::sleep(5.).await;
                continue;
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
                .map(|res| res.into())
                .map_err(|e| ERRL!("{}", e)),
        )
    }

    fn display_priv_key(&self) -> Result<String, String> { Ok(format!("{:#02x}", self.key_pair.secret())) }

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

async fn sign_and_send_transaction_impl(
    ctx: MmArc,
    coin: EthCoin,
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
    let nonce = try_tx_s!(
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
    let signed = tx.sign(coin.key_pair.secret(), coin.chain_id);
    let bytes = web3::types::Bytes(rlp::encode(&signed).to_vec());
    status.status(tags!(), "send_raw_transaction…");

    try_tx_s!(
        coin.web3
            .eth()
            .send_raw_transaction(bytes)
            .map_err(|e| ERRL!("{}", e))
            .compat()
            .await,
        signed
    );

    status.status(tags!(), "get_addr_nonce…");
    loop {
        // Check every second till ETH nodes recognize that nonce is increased
        // Parity has reliable "nextNonce" method that always returns correct nonce for address
        // But we can't expect that all nodes will always be Parity.
        // Some of ETH forks use Geth only so they don't have Parity nodes at all.
        let new_nonce = match get_addr_nonce(coin.my_address, coin.web3_instances.clone())
            .compat()
            .await
        {
            Ok(n) => n,
            Err(e) => {
                error!("Error getting {} {} nonce: {}", coin.ticker(), coin.my_address, e);
                // we can just keep looping in case of error hoping it will go away
                continue;
            },
        };
        if new_nonce > nonce {
            break;
        };
        Timer::sleep(1.).await;
    }
    Ok(signed)
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
        let delta = U256::from(1000);

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

            let current_block = match self.web3.eth().block_number().compat().await {
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
                "blocks_left": u64::from(saved_traces.earliest_block),
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
                        BlockNumber::Number(before_earliest.into()),
                        BlockNumber::Number((saved_traces.earliest_block).into()),
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
                        BlockNumber::Number(before_earliest.into()),
                        BlockNumber::Number((saved_traces.earliest_block).into()),
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
                        BlockNumber::Number((saved_traces.latest_block + 1).into()),
                        BlockNumber::Number(current_block.into()),
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
                        BlockNumber::Number((saved_traces.latest_block + 1).into()),
                        BlockNumber::Number(current_block.into()),
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
                    .compat()
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
                    .compat()
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
                    Some(r) => Some(
                        EthTxFeeDetails::new(r.gas_used.unwrap_or_else(|| 0.into()), web3_tx.gas_price, fee_coin)
                            .unwrap(),
                    ),
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
                    .block(BlockId::Number(BlockNumber::Number(trace.block_number)))
                    .compat()
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
                    tx_hash: format!("{:02x}", BytesJson(raw.hash.to_vec())),
                    tx_hex: BytesJson(rlp::encode(&raw)),
                    internal_id,
                    timestamp: block.timestamp.into(),
                    kmd_rewards: None,
                    transaction_type: Default::default(),
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
        let delta = U256::from(10000);

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

            let current_block = match self.web3.eth().block_number().compat().await {
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
                "blocks_left": u64::from(saved_events.earliest_block),
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
                        BlockNumber::Number(before_earliest.into()),
                        BlockNumber::Number((saved_events.earliest_block - 1).into()),
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
                        BlockNumber::Number(before_earliest.into()),
                        BlockNumber::Number((saved_events.earliest_block - 1).into()),
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
                        BlockNumber::Number((saved_events.latest_block + 1).into()),
                        BlockNumber::Number(current_block.into()),
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
                        BlockNumber::Number((saved_events.latest_block + 1).into()),
                        BlockNumber::Number(current_block.into()),
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
            let mut all_events: Vec<_> = all_events.into_iter().map(|(_, log)| log).collect();
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
                    .compat()
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
                    .compat()
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
                    Some(r) => Some(
                        EthTxFeeDetails::new(r.gas_used.unwrap_or_else(|| 0.into()), web3_tx.gas_price, fee_coin)
                            .unwrap(),
                    ),
                    None => None,
                };
                let block_number = event.block_number.unwrap();
                let block = match self
                    .web3
                    .eth()
                    .block(BlockId::Number(BlockNumber::Number(block_number.into())))
                    .compat()
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
                    block_height: block_number.into(),
                    tx_hash: format!("{:02x}", BytesJson(raw.hash.to_vec())),
                    tx_hex: BytesJson(rlp::encode(&raw)),
                    internal_id: BytesJson(internal_id.to_vec()),
                    timestamp: block.timestamp.into(),
                    kmd_rewards: None,
                    transaction_type: Default::default(),
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
        let fut = Box::pin(sign_and_send_transaction_impl(
            ctx,
            self.clone(),
            value,
            action,
            data,
            gas,
        ));
        Box::new(fut.compat())
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

    fn send_hash_time_locked_payment(
        &self,
        id: Vec<u8>,
        value: U256,
        time_lock: u32,
        secret_hash: &[u8],
        receiver_addr: Address,
        swap_contract_address: Address,
    ) -> EthTxFut {
        match &self.coin_type {
            EthCoinType::Eth => {
                let function = try_tx_fus!(SWAP_CONTRACT.function("ethPayment"));
                let data = try_tx_fus!(function.encode_input(&[
                    Token::FixedBytes(id),
                    Token::Address(receiver_addr),
                    Token::FixedBytes(secret_hash.to_vec()),
                    Token::Uint(U256::from(time_lock))
                ]));
                self.sign_and_send_transaction(value, Action::Call(swap_contract_address), data, U256::from(150_000))
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let allowance_fut = self
                    .allowance(swap_contract_address)
                    .map_err(|e| TransactionErr::Plain(ERRL!("{}", e)));

                let function = try_tx_fus!(SWAP_CONTRACT.function("erc20Payment"));
                let data = try_tx_fus!(function.encode_input(&[
                    Token::FixedBytes(id),
                    Token::Uint(value),
                    Token::Address(*token_addr),
                    Token::Address(receiver_addr),
                    Token::FixedBytes(secret_hash.to_vec()),
                    Token::Uint(U256::from(time_lock))
                ]));

                let arc = self.clone();
                Box::new(allowance_fut.and_then(move |allowed| -> EthTxFut {
                    if allowed < value {
                        Box::new(
                            arc.approve(swap_contract_address, U256::max_value())
                                .and_then(move |_approved| {
                                    arc.sign_and_send_transaction(
                                        0.into(),
                                        Action::Call(swap_contract_address),
                                        data,
                                        U256::from(150_000),
                                    )
                                }),
                        )
                    } else {
                        Box::new(arc.sign_and_send_transaction(
                            0.into(),
                            Action::Call(swap_contract_address),
                            data,
                            U256::from(150_000),
                        ))
                    }
                }))
            },
        }
    }

    fn spend_hash_time_locked_payment(
        &self,
        payment: SignedEthTx,
        swap_contract_address: Address,
        secret: &[u8],
    ) -> EthTxFut {
        let spend_func = try_tx_fus!(SWAP_CONTRACT.function("receiverSpend"));
        let clone = self.clone();
        let secret_vec = secret.to_vec();

        match self.coin_type {
            EthCoinType::Eth => {
                let payment_func = try_tx_fus!(SWAP_CONTRACT.function("ethPayment"));
                let decoded = try_tx_fus!(payment_func.decode_input(&payment.data));

                let state_f = self.payment_status(swap_contract_address, decoded[0].clone());
                Box::new(
                    state_f
                        .map_err(TransactionErr::Plain)
                        .and_then(move |state| -> EthTxFut {
                            if state != PAYMENT_STATE_SENT.into() {
                                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                                    "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                                    payment,
                                    state
                                ))));
                            }

                            let value = payment.value;
                            let data = try_tx_fus!(spend_func.encode_input(&[
                                decoded[0].clone(),
                                Token::Uint(value),
                                Token::FixedBytes(secret_vec),
                                Token::Address(Address::default()),
                                Token::Address(payment.sender()),
                            ]));

                            clone.sign_and_send_transaction(
                                0.into(),
                                Action::Call(swap_contract_address),
                                data,
                                U256::from(150_000),
                            )
                        }),
                )
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let payment_func = try_tx_fus!(SWAP_CONTRACT.function("erc20Payment"));
                let decoded = try_tx_fus!(payment_func.decode_input(&payment.data));
                let state_f = self.payment_status(swap_contract_address, decoded[0].clone());

                Box::new(
                    state_f
                        .map_err(TransactionErr::Plain)
                        .and_then(move |state| -> EthTxFut {
                            if state != PAYMENT_STATE_SENT.into() {
                                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                                    "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                                    payment,
                                    state
                                ))));
                            }
                            let data = try_tx_fus!(spend_func.encode_input(&[
                                decoded[0].clone(),
                                decoded[1].clone(),
                                Token::FixedBytes(secret_vec),
                                Token::Address(token_addr),
                                Token::Address(payment.sender()),
                            ]));

                            clone.sign_and_send_transaction(
                                0.into(),
                                Action::Call(swap_contract_address),
                                data,
                                U256::from(150_000),
                            )
                        }),
                )
            },
        }
    }

    fn refund_hash_time_locked_payment(&self, swap_contract_address: Address, payment: SignedEthTx) -> EthTxFut {
        let refund_func = try_tx_fus!(SWAP_CONTRACT.function("senderRefund"));
        let clone = self.clone();

        match self.coin_type {
            EthCoinType::Eth => {
                let payment_func = try_tx_fus!(SWAP_CONTRACT.function("ethPayment"));
                let decoded = try_tx_fus!(payment_func.decode_input(&payment.data));

                let state_f = self.payment_status(swap_contract_address, decoded[0].clone());
                Box::new(
                    state_f
                        .map_err(TransactionErr::Plain)
                        .and_then(move |state| -> EthTxFut {
                            if state != PAYMENT_STATE_SENT.into() {
                                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                                    "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                                    payment,
                                    state
                                ))));
                            }

                            let value = payment.value;
                            let data = try_tx_fus!(refund_func.encode_input(&[
                                decoded[0].clone(),
                                Token::Uint(value),
                                decoded[2].clone(),
                                Token::Address(Address::default()),
                                decoded[1].clone(),
                            ]));

                            clone.sign_and_send_transaction(
                                0.into(),
                                Action::Call(swap_contract_address),
                                data,
                                U256::from(150_000),
                            )
                        }),
                )
            },
            EthCoinType::Erc20 {
                platform: _,
                token_addr,
            } => {
                let payment_func = try_tx_fus!(SWAP_CONTRACT.function("erc20Payment"));
                let decoded = try_tx_fus!(payment_func.decode_input(&payment.data));
                let state_f = self.payment_status(swap_contract_address, decoded[0].clone());
                Box::new(
                    state_f
                        .map_err(TransactionErr::Plain)
                        .and_then(move |state| -> EthTxFut {
                            if state != PAYMENT_STATE_SENT.into() {
                                return Box::new(futures01::future::err(TransactionErr::Plain(ERRL!(
                                    "Payment {:?} state is not PAYMENT_STATE_SENT, got {}",
                                    payment,
                                    state
                                ))));
                            }

                            let data = try_tx_fus!(refund_func.encode_input(&[
                                decoded[0].clone(),
                                decoded[1].clone(),
                                decoded[4].clone(),
                                Token::Address(token_addr),
                                decoded[3].clone(),
                            ]));

                            clone.sign_and_send_transaction(
                                0.into(),
                                Action::Call(swap_contract_address),
                                data,
                                U256::from(150_000),
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
                    .compat()
                    .await?),
                EthCoinType::Erc20 { ref token_addr, .. } => {
                    let function = ERC20_CONTRACT.function("balanceOf")?;
                    let data = function.encode_input(&[Token::Address(coin.my_address)])?;

                    let res = coin.call_request(*token_addr, None, Some(data.into())).compat().await?;
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
        let coin = self.clone();
        let mut token_balances = HashMap::new();
        for (token_ticker, info) in self.get_erc_tokens_infos().iter() {
            let balance_as_u256 = coin.get_token_balance_by_address(info.token_address).await?;
            let balance_as_big_decimal = u256_to_big_decimal(balance_as_u256, info.decimals)?;
            let balance = CoinBalance {
                spendable: balance_as_big_decimal,
                unspendable: BigDecimal::from(0),
            };
            token_balances.insert(token_ticker.clone(), balance);
        }

        Ok(token_balances)
    }

    async fn get_token_balance_by_address(&self, token_address: Address) -> Result<U256, MmError<BalanceError>> {
        let coin = self.clone();
        let function = ERC20_CONTRACT.function("balanceOf")?;
        let data = function.encode_input(&[Token::Address(coin.my_address)])?;
        let res = coin
            .call_request(token_address, None, Some(data.into()))
            .compat()
            .await?;
        let decoded = function.decode_output(&res.0)?;

        match decoded[0] {
            Token::Uint(number) => Ok(number),
            _ => {
                let error = format!("Expected U256 as balanceOf result but got {:?}", decoded);
                MmError::err(BalanceError::InvalidResponse(error))
            },
        }
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
                to: contract_addr,
                gas: None,
                // gas price must be supplied because some smart contracts base their
                // logic on gas price, e.g. TUSD: https://github.com/KomodoPlatform/atomicDEX-API/issues/643
                gas_price: Some(gas_price),
            };
            coin.estimate_gas(estimate_gas_req).map_to_mm_fut(Web3RpcError::from)
        }))
    }

    fn eth_balance(&self) -> BalanceFut<U256> {
        Box::new(
            self.web3
                .eth()
                .balance(self.my_address, Some(BlockNumber::Latest))
                .map_to_mm_fut(BalanceError::from),
        )
    }

    fn call_request(
        &self,
        to: Address,
        value: Option<U256>,
        data: Option<Bytes>,
    ) -> impl Future<Item = Bytes, Error = web3::Error> {
        let request = CallRequest {
            from: Some(self.my_address),
            to,
            gas: None,
            gas_price: None,
            value,
            data,
        };

        self.web3.eth().call(request, Some(BlockNumber::Latest))
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

                    let res = coin.call_request(*token_addr, None, Some(data.into())).compat().await?;
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
            .from_block(BlockNumber::Number(from_block))
            .to_block(BlockNumber::Number(to_block))
            .address(vec![swap_contract_address])
            .build();

        Box::new(self.web3.eth().logs(filter).map_err(|e| ERRL!("{}", e)))
    }

    fn validate_payment(
        &self,
        payment_tx: &[u8],
        time_lock: u32,
        sender_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        expected_swap_contract_address: Address,
    ) -> ValidatePaymentFut<()> {
        let unsigned: UnverifiedTransaction = try_f!(rlp::decode(payment_tx));
        let tx =
            try_f!(SignedEthTx::new(unsigned)
                .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string())));
        let sender = try_f!(addr_from_raw_pubkey(sender_pub).map_to_mm(ValidatePaymentError::InvalidInput));
        let expected_value = try_f!(wei_from_big_decimal(&amount, self.decimals));
        let selfi = self.clone();
        let secret_hash = secret_hash.to_vec();
        let fut = async move {
            let swap_id = selfi.etomic_swap_id(time_lock, &secret_hash);
            let status = selfi
                .payment_status(expected_swap_contract_address, Token::FixedBytes(swap_id.clone()))
                .compat()
                .await
                .map_to_mm(ValidatePaymentError::Transport)?;
            if status != PAYMENT_STATE_SENT.into() {
                return MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
                    "Payment state is not PAYMENT_STATE_SENT, got {}",
                    status
                )));
            }

            let tx_from_rpc = selfi
                .web3
                .eth()
                .transaction(TransactionId::Hash(tx.hash))
                .compat()
                .await?;
            let tx_from_rpc = match tx_from_rpc {
                Some(t) => t,
                None => {
                    return MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
                        "Didn't find provided tx {:?} on ETH node",
                        tx
                    )))
                },
            };

            if tx_from_rpc.from != sender {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Payment tx {:?} was sent from wrong address, expected {:?}",
                    tx_from_rpc, sender
                )));
            }

            match &selfi.coin_type {
                EthCoinType::Eth => {
                    if tx_from_rpc.to != Some(expected_swap_contract_address) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx {:?} was sent to wrong address, expected {:?}",
                            tx_from_rpc, expected_swap_contract_address,
                        )));
                    }

                    if tx_from_rpc.value != expected_value {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx value arg {:?} is invalid, expected {:?}",
                            tx_from_rpc, expected_value
                        )));
                    }

                    let function = SWAP_CONTRACT
                        .function("ethPayment")
                        .map_to_mm(|err| ValidatePaymentError::InternalError(err.to_string()))?;
                    let decoded = function
                        .decode_input(&tx_from_rpc.input.0)
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

                    if decoded[3] != Token::Uint(U256::from(time_lock)) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx time_lock arg {:?} is invalid, expected {:?}",
                            decoded[3],
                            Token::Uint(U256::from(time_lock)),
                        )));
                    }
                },
                EthCoinType::Erc20 {
                    platform: _,
                    token_addr,
                } => {
                    if tx_from_rpc.to != Some(expected_swap_contract_address) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx {:?} was sent to wrong address, expected {:?}",
                            tx_from_rpc, expected_swap_contract_address,
                        )));
                    }

                    let function = SWAP_CONTRACT
                        .function("erc20Payment")
                        .map_to_mm(|err| ValidatePaymentError::InternalError(err.to_string()))?;
                    let decoded = function
                        .decode_input(&tx_from_rpc.input.0)
                        .map_to_mm(|err| ValidatePaymentError::TxDeserializationError(err.to_string()))?;
                    if decoded[0] != Token::FixedBytes(swap_id.clone()) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Invalid 'swap_id' {:?}, expected {:?}",
                            decoded, swap_id
                        )));
                    }

                    if decoded[1] != Token::Uint(expected_value) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx value arg {:?} is invalid, expected {:?}",
                            decoded[1],
                            Token::Uint(expected_value),
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

                    if decoded[5] != Token::Uint(U256::from(time_lock)) {
                        return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                            "Payment tx time_lock arg {:?} is invalid, expected {:?}",
                            decoded[5],
                            Token::Uint(U256::from(time_lock)),
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

        Box::new(
            self.call_request(swap_contract_address, None, Some(data.into()))
                .map_err(|e| ERRL!("{}", e))
                .and_then(move |bytes| {
                    let decoded_tokens = try_s!(function.decode_output(&bytes.0));
                    match decoded_tokens[2] {
                        Token::Uint(state) => Ok(state),
                        _ => ERR!("Payment status must be uint, got {:?}", decoded_tokens[2]),
                    }
                }),
        )
    }

    async fn search_for_swap_tx_spend(
        &self,
        tx: &[u8],
        swap_contract_address: Address,
        search_from_block: u64,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        let unverified: UnverifiedTransaction = try_s!(rlp::decode(tx));
        let tx = try_s!(SignedEthTx::new(unverified));

        let func_name = match self.coin_type {
            EthCoinType::Eth => "ethPayment",
            EthCoinType::Erc20 { .. } => "erc20Payment",
        };

        let payment_func = try_s!(SWAP_CONTRACT.function(func_name));
        let decoded = try_s!(payment_func.decode_input(&tx.data));
        let id = match &decoded[0] {
            Token::FixedBytes(bytes) => bytes.clone(),
            _ => panic!(),
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
                        let transaction =
                            match try_s!(self.web3.eth().transaction(TransactionId::Hash(tx_hash)).compat().await) {
                                Some(t) => t,
                                None => {
                                    return ERR!(
                                        "Found ReceiverSpent event, but transaction {:02x} is missing",
                                        tx_hash
                                    )
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
                        let transaction =
                            match try_s!(self.web3.eth().transaction(TransactionId::Hash(tx_hash)).compat().await) {
                                Some(t) => t,
                                None => {
                                    return ERR!(
                                        "Found SenderRefunded event, but transaction {:02x} is missing",
                                        tx_hash
                                    )
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

    /// Get gas price
    fn get_gas_price(&self) -> Web3RpcFut<U256> {
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

            let eth_gas_price = match coin.web3.eth().gas_price().compat().await {
                Ok(eth_gas) => Some(eth_gas),
                Err(e) => {
                    error!("Error {} on eth_gasPrice request", e);
                    None
                },
            };

            let fee_history_namespace: EthFeeHistoryNamespace<_> = coin.web3.api();
            let eth_fee_history_price = match fee_history_namespace
                .eth_fee_history(U256::from(1u64), BlockNumber::Latest, &[])
                .compat()
                .await
            {
                Ok(res) => res
                    .base_fee_per_gas
                    .first()
                    .map(|val| increase_by_percent_one_gwei(*val, BASE_BLOCK_FEE_DIFF_PCT)),
                Err(e) => {
                    error!("Error {} on eth_feeHistory request", e);
                    None
                },
            };

            let all_prices = vec![gas_station_price, eth_gas_price, eth_fee_history_price];
            all_prices
                .into_iter()
                .flatten()
                .max()
                .or_mm_err(|| Web3RpcError::Internal("All requests failed".into()))
        };
        Box::new(fut.boxed().compat())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EthTxFeeDetails {
    coin: String,
    gas: u64,
    /// WEI units per 1 gas
    gas_price: BigDecimal,
    total_fee: BigDecimal,
}

impl EthTxFeeDetails {
    fn new(gas: U256, gas_price: U256, coin: &str) -> NumConversResult<EthTxFeeDetails> {
        let total_fee = gas * gas_price;
        // Fees are always paid in ETH, can use 18 decimals by default
        let total_fee = u256_to_big_decimal(total_fee, ETH_DECIMALS)?;
        let gas_price = u256_to_big_decimal(gas_price, ETH_DECIMALS)?;

        Ok(EthTxFeeDetails {
            coin: coin.to_owned(),
            gas: gas.into(),
            gas_price,
            total_fee,
        })
    }
}

#[async_trait]
impl MmCoin for EthCoin {
    fn is_asset_chain(&self) -> bool { false }

    fn get_raw_transaction(&self, req: RawTransactionRequest) -> RawTransactionFut {
        Box::new(get_raw_transaction_impl(self.clone(), req).boxed().compat())
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
            return Box::new(futures01::future::ok(()));
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
                    let fee = gas_price * U256::from(150_000);
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
            let total_fee = gas_price * U256::from(150_000);
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
            to: *call_addr,
            gas: None,
            // gas price must be supplied because some smart contracts base their
            // logic on gas price, e.g. TUSD: https://github.com/KomodoPlatform/atomicDEX-API/issues/643
            gas_price: Some(gas_price),
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

    fn mature_confirmations(&self) -> Option<u32> { None }

    fn coin_protocol_info(&self) -> Vec<u8> { Vec::new() }

    fn is_coin_protocol_supported(&self, _info: &Option<Vec<u8>>) -> bool { true }
}

pub trait TryToAddress {
    fn try_to_address(&self) -> Result<Address, String>;
}

impl TryToAddress for BytesJson {
    fn try_to_address(&self) -> Result<Address, String> { Ok(Address::from(self.0.as_slice())) }
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

pub fn addr_from_raw_pubkey(pubkey: &[u8]) -> Result<Address, String> {
    let pubkey = try_s!(PublicKey::from_slice(pubkey).map_err(|e| ERRL!("{:?}", e)));
    let eth_public = Public::from(&pubkey.serialize_uncompressed()[1..65]);
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

    fn tx_hash(&self) -> BytesJson { self.hash.to_vec().into() }
}

fn signed_tx_from_web3_tx(transaction: Web3Transaction) -> Result<SignedEthTx, String> {
    let unverified = UnverifiedTransaction {
        r: transaction.r,
        s: transaction.s,
        v: transaction.v.as_u64(),
        hash: transaction.hash,
        unsigned: UnSignedEthTx {
            data: transaction.input.0,
            gas_price: transaction.gas_price,
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
        to: token_addr,
        gas: None,
        gas_price: None,
        value: Some(0.into()),
        data: Some(data.into()),
    };

    let f = web3
        .eth()
        .call(request, Some(BlockNumber::Latest))
        .map_err(|e| ERRL!("{}", e));
    let res = try_s!(f.compat().await);
    let tokens = try_s!(function.decode_output(&res.0));
    let decimals: u64 = match tokens[0] {
        Token::Uint(dec) => dec.into(),
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
    priv_key: &[u8],
    protocol: CoinProtocol,
) -> Result<EthCoin, String> {
    let mut urls: Vec<String> = try_s!(json::from_value(req["urls"].clone()));
    if urls.is_empty() {
        return ERR!("Enable request for ETH coin must have at least 1 node URL");
    }
    let mut rng = small_rng();
    urls.as_mut_slice().shuffle(&mut rng);

    let mut nodes = vec![];
    for url in urls.iter() {
        nodes.push(Web3TransportNode {
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

    let key_pair: KeyPair = try_s!(KeyPair::from_secret_slice(priv_key));
    let my_address = key_pair.address();

    let mut web3_instances = vec![];
    let event_handlers = rpc_event_handlers_for_eth_transport(ctx, ticker.to_string());
    for node in nodes.iter() {
        let transport = Web3Transport::with_event_handlers(vec![node.clone()], event_handlers.clone());
        let web3 = Web3::new(transport);
        let version = match web3.web3().client_version().compat().await {
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

    let transport = Web3Transport::with_event_handlers(nodes, event_handlers);
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

    let coin = EthCoinImpl {
        key_pair,
        my_address,
        coin_type,
        sign_message_prefix,
        swap_contract_address,
        fallback_swap_contract,
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
        if c.is_digit(10) {
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

/// Requests the nonce from all available nodes and checks that returned results equal.
/// Nodes might need some time to sync and there can be other coins that use same nodes in different order.
/// We need to be sure that nonce is updated on all of them before and after transaction is sent.
#[cfg_attr(test, mockable)]
fn get_addr_nonce(addr: Address, web3s: Vec<Web3Instance>) -> Box<dyn Future<Item = U256, Error = String> + Send> {
    let fut = async move {
        let mut errors: u32 = 0;
        loop {
            let futures: Vec<_> = web3s
                .iter()
                .map(|web3| {
                    if web3.is_parity {
                        web3.web3.eth().parity_next_nonce(addr).compat()
                    } else {
                        web3.web3
                            .eth()
                            .transaction_count(addr, Some(BlockNumber::Pending))
                            .compat()
                    }
                })
                .collect();

            let nonces: Vec<_> = join_all(futures)
                .await
                .into_iter()
                .filter_map(|nonce_res| match nonce_res {
                    Ok(n) => Some(n),
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
                let max = nonces.iter().max().unwrap();
                let min = nonces.iter().min().unwrap();
                if max == min {
                    return Ok(*max);
                } else {
                    warn!("Max nonce {} != {} min nonce", max, min);
                }
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
    }
}
