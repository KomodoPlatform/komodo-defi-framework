use super::ethermint_account::EthermintAccount;
use super::htlc::{ClaimHtlcMsg, ClaimHtlcProto, CreateHtlcMsg, CreateHtlcProto, HtlcType, QueryHtlcRequestProto,
                  QueryHtlcResponse, TendermintHtlc, HTLC_STATE_COMPLETED, HTLC_STATE_OPEN, HTLC_STATE_REFUNDED};
use super::ibc::transfer_v1::MsgTransfer;
use super::ibc::IBC_GAS_LIMIT_DEFAULT;
use super::{rpc::*, TENDERMINT_COIN_PROTOCOL_TYPE};
use crate::coin_errors::{MyAddressError, ValidatePaymentError, ValidatePaymentResult};
use crate::hd_wallet::{HDPathAccountToAddressId, WithdrawFrom};
use crate::rpc_command::tendermint::staking::ValidatorStatus;
use crate::rpc_command::tendermint::{IBCChainRegistriesResponse, IBCChainRegistriesResult, IBCChainsRequestError,
                                     IBCTransferChannel, IBCTransferChannelTag, IBCTransferChannelsRequestError,
                                     IBCTransferChannelsResponse, IBCTransferChannelsResult, CHAIN_REGISTRY_BRANCH,
                                     CHAIN_REGISTRY_IBC_DIR_NAME, CHAIN_REGISTRY_REPO_NAME, CHAIN_REGISTRY_REPO_OWNER};
use crate::tendermint::ibc::IBC_OUT_SOURCE_PORT;
use crate::utxo::sat_from_big_decimal;
use crate::utxo::utxo_common::big_decimal_from_sat;
use crate::{big_decimal_from_sat_unsigned, BalanceError, BalanceFut, BigDecimal, CheckIfMyPaymentSentArgs,
            CoinBalance, CoinFutSpawner, ConfirmPaymentInput, DexFee, FeeApproxStage, FoundSwapTxSpend,
            HistorySyncState, MakerSwapTakerCoin, MarketCoinOps, MmCoin, MmCoinEnum, NegotiateSwapContractAddrErr,
            PaymentInstructionArgs, PaymentInstructions, PaymentInstructionsErr, PrivKeyBuildPolicy, PrivKeyPolicy,
            PrivKeyPolicyNotAllowed, RawTransactionError, RawTransactionFut, RawTransactionRequest, RawTransactionRes,
            RawTransactionResult, RefundError, RefundPaymentArgs, RefundResult, RpcCommonOps,
            SearchForSwapTxSpendInput, SendMakerPaymentSpendPreimageInput, SendPaymentArgs, SignRawTransactionRequest,
            SignatureError, SignatureResult, SpendPaymentArgs, SwapOps, TakerSwapMakerCoin, ToBytes, TradeFee,
            TradePreimageError, TradePreimageFut, TradePreimageResult, TradePreimageValue, TransactionData,
            TransactionDetails, TransactionEnum, TransactionErr, TransactionFut, TransactionResult, TransactionType,
            TxFeeDetails, TxMarshalingErr, UnexpectedDerivationMethod, ValidateAddressResult, ValidateFeeArgs,
            ValidateInstructionsErr, ValidateOtherPubKeyErr, ValidatePaymentFut, ValidatePaymentInput,
            ValidateWatcherSpendInput, VerificationError, VerificationResult, WaitForHTLCTxSpendArgs, WatcherOps,
            WatcherReward, WatcherRewardError, WatcherSearchForSwapTxSpendInput, WatcherValidatePaymentInput,
            WatcherValidateTakerFeeInput, WithdrawError, WithdrawFee, WithdrawFut, WithdrawRequest};
use async_std::prelude::FutureExt as AsyncStdFutureExt;
use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine;
use bip32::DerivationPath;
use bitcrypto::{dhash160, sha256};
use common::executor::{abortable_queue::AbortableQueue, AbortableSystem};
use common::executor::{AbortedError, Timer};
use common::log::{debug, warn};
use common::{get_utc_timestamp, now_sec, Future01CompatExt, PagingOptions, DEX_FEE_ADDR_PUBKEY};
use cosmrs::bank::MsgSend;
use cosmrs::crypto::secp256k1::SigningKey;
use cosmrs::proto::cosmos::auth::v1beta1::{BaseAccount, QueryAccountRequest, QueryAccountResponse};
use cosmrs::proto::cosmos::bank::v1beta1::{MsgSend as MsgSendProto, QueryBalanceRequest, QueryBalanceResponse};
use cosmrs::proto::cosmos::base::query::v1beta1::PageRequest;
use cosmrs::proto::cosmos::base::tendermint::v1beta1::{GetBlockByHeightRequest, GetBlockByHeightResponse,
                                                       GetLatestBlockRequest, GetLatestBlockResponse};
use cosmrs::proto::cosmos::base::v1beta1::Coin as CoinProto;
use cosmrs::proto::cosmos::staking::v1beta1::{QueryValidatorsRequest,
                                              QueryValidatorsResponse as QueryValidatorsResponseProto};
use cosmrs::proto::cosmos::tx::v1beta1::{GetTxRequest, GetTxResponse, GetTxsEventRequest, GetTxsEventResponse,
                                         SimulateRequest, SimulateResponse, Tx, TxBody, TxRaw};
use cosmrs::proto::prost::{DecodeError, Message};
use cosmrs::staking::{QueryValidatorsResponse, Validator};
use cosmrs::tendermint::block::Height;
use cosmrs::tendermint::chain::Id as ChainId;
use cosmrs::tendermint::PublicKey;
use cosmrs::tx::{self, Fee, Msg, Raw, SignDoc, SignerInfo};
use cosmrs::{AccountId, Any, Coin, Denom, ErrorReport};
use crypto::privkey::key_pair_from_secret;
use crypto::{HDPathToCoin, Secp256k1Secret};
use derive_more::Display;
use futures::future::try_join_all;
use futures::lock::Mutex as AsyncMutex;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use hex::FromHexError;
use instant::Duration;
use itertools::Itertools;
use kdf_walletconnect::{WalletConnectCtx, WalletConnectOps};
use keys::{KeyPair, Public};
use mm2_core::mm_ctx::{MmArc, MmWeak};
use mm2_err_handle::prelude::*;
use mm2_git::{FileMetadata, GitController, GithubClient, RepositoryOperations, GITHUB_API_URI};
use mm2_number::MmNumber;
use mm2_p2p::p2p_ctx::P2PContext;
use parking_lot::Mutex as PaMutex;
use primitives::hash::H256;
use regex::Regex;
use rpc::v1::types::Bytes as BytesJson;
use serde_json::{self as json, Value as Json};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::io;
use std::num::NonZeroU32;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

// ABCI Request Paths
const ABCI_GET_LATEST_BLOCK_PATH: &str = "/cosmos.base.tendermint.v1beta1.Service/GetLatestBlock";
const ABCI_GET_BLOCK_BY_HEIGHT_PATH: &str = "/cosmos.base.tendermint.v1beta1.Service/GetBlockByHeight";
const ABCI_SIMULATE_TX_PATH: &str = "/cosmos.tx.v1beta1.Service/Simulate";
const ABCI_QUERY_ACCOUNT_PATH: &str = "/cosmos.auth.v1beta1.Query/Account";
const ABCI_QUERY_BALANCE_PATH: &str = "/cosmos.bank.v1beta1.Query/Balance";
const ABCI_GET_TX_PATH: &str = "/cosmos.tx.v1beta1.Service/GetTx";
const ABCI_GET_TXS_EVENT_PATH: &str = "/cosmos.tx.v1beta1.Service/GetTxsEvent";
const ABCI_VALIDATORS_PATH: &str = "/cosmos.staking.v1beta1.Query/Validators";

pub(crate) const MIN_TX_SATOSHIS: i64 = 1;

// ABCI Request Defaults
const ABCI_REQUEST_HEIGHT: Option<Height> = None;
const ABCI_REQUEST_PROVE: bool = false;

/// 0.25 is good average gas price on atom and iris
const DEFAULT_GAS_PRICE: f64 = 0.25;
pub(super) const TIMEOUT_HEIGHT_DELTA: u64 = 100;
pub const GAS_LIMIT_DEFAULT: u64 = 125_000;
pub const GAS_WANTED_BASE_VALUE: f64 = 50_000.;
pub(crate) const TX_DEFAULT_MEMO: &str = "";

// https://github.com/irisnet/irismod/blob/5016c1be6fdbcffc319943f33713f4a057622f0a/modules/htlc/types/validation.go#L19-L22
const MAX_TIME_LOCK: i64 = 34560;
const MIN_TIME_LOCK: i64 = 50;

const ACCOUNT_SEQUENCE_ERR: &str = "account sequence mismatch";

lazy_static! {
    static ref SEQUENCE_PARSER_REGEX: Regex = Regex::new(r"expected (\d+)").unwrap();
}

pub struct SerializedUnsignedTx {
    tx_json: Json,
    body_bytes: Vec<u8>,
}

type TendermintPrivKeyPolicy = PrivKeyPolicy<TendermintKeyPair>;

pub struct TendermintKeyPair {
    private_key_secret: Secp256k1Secret,
    public_key: Public,
}

impl TendermintKeyPair {
    fn new(private_key_secret: Secp256k1Secret, public_key: Public) -> Self {
        Self {
            private_key_secret,
            public_key,
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct RpcNode {
    url: String,
    #[serde(default)]
    komodo_proxy: bool,
}

impl RpcNode {
    #[cfg(test)]
    fn for_test(url: &str) -> Self {
        Self {
            url: url.to_string(),
            komodo_proxy: false,
        }
    }
}

#[async_trait]
pub trait TendermintCommons {
    fn platform_denom(&self) -> &Denom;

    fn set_history_sync_state(&self, new_state: HistorySyncState);

    async fn get_block_timestamp(&self, block: i64) -> MmResult<Option<u64>, TendermintCoinRpcError>;

    async fn get_all_balances(&self) -> MmResult<AllBalancesResult, TendermintCoinRpcError>;

    async fn rpc_client(&self) -> MmResult<HttpClient, TendermintCoinRpcError>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TendermintFeeDetails {
    pub coin: String,
    pub amount: BigDecimal,
    #[serde(skip)]
    pub uamount: u64,
    pub gas_limit: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TendermintProtocolInfo {
    decimals: u8,
    denom: String,
    pub account_prefix: String,
    pub chain_id: String,
    gas_price: Option<f64>,
    chain_registry_name: Option<String>,
}

#[derive(Clone)]
pub struct ActivatedTokenInfo {
    pub(crate) decimals: u8,
    pub ticker: String,
}

pub struct TendermintConf {
    avg_blocktime: u8,
    /// Derivation path of the coin.
    /// This derivation path consists of `purpose` and `coin_type` only
    /// where the full `BIP44` address has the following structure:
    /// `m/purpose'/coin_type'/account'/change/address_index`.
    derivation_path: Option<HDPathToCoin>,
}

impl TendermintConf {
    pub fn try_from_json(ticker: &str, conf: &Json) -> MmResult<Self, TendermintInitError> {
        let avg_blocktime = conf.get("avg_blocktime").or_mm_err(|| TendermintInitError {
            ticker: ticker.to_string(),
            kind: TendermintInitErrorKind::AvgBlockTimeMissing,
        })?;

        let avg_blocktime = avg_blocktime.as_i64().or_mm_err(|| TendermintInitError {
            ticker: ticker.to_string(),
            kind: TendermintInitErrorKind::AvgBlockTimeInvalid,
        })?;

        let avg_blocktime = u8::try_from(avg_blocktime).map_to_mm(|_| TendermintInitError {
            ticker: ticker.to_string(),
            kind: TendermintInitErrorKind::AvgBlockTimeInvalid,
        })?;

        let derivation_path = json::from_value(conf["derivation_path"].clone()).map_to_mm(|e| TendermintInitError {
            ticker: ticker.to_string(),
            kind: TendermintInitErrorKind::ErrorDeserializingDerivationPath(e.to_string()),
        })?;

        Ok(TendermintConf {
            avg_blocktime,
            derivation_path,
        })
    }
}

pub enum TendermintActivationPolicy {
    PrivateKey(PrivKeyPolicy<TendermintKeyPair>),
    PublicKey(PublicKey),
}

impl TendermintActivationPolicy {
    pub fn with_private_key_policy(private_key_policy: PrivKeyPolicy<TendermintKeyPair>) -> Self {
        Self::PrivateKey(private_key_policy)
    }

    pub fn with_public_key(account_public_key: PublicKey) -> Self { Self::PublicKey(account_public_key) }

    fn generate_account_id(&self, account_prefix: &str) -> Result<AccountId, ErrorReport> {
        match self {
            Self::PrivateKey(priv_key_policy) => {
                let pk = priv_key_policy.activated_key().ok_or_else(|| {
                    ErrorReport::new(io::Error::new(io::ErrorKind::NotFound, "Activated key not found"))
                })?;

                Ok(
                    account_id_from_privkey(pk.private_key_secret.as_slice(), account_prefix)
                        .map_err(|e| ErrorReport::new(io::Error::new(io::ErrorKind::InvalidData, e.to_string())))?,
                )
            },

            Self::PublicKey(account_public_key) => {
                account_id_from_raw_pubkey(account_prefix, &account_public_key.to_bytes())
            },
        }
    }

    fn public_key(&self) -> Result<PublicKey, io::Error> {
        match self {
            Self::PrivateKey(private_key_policy) => match private_key_policy {
                PrivKeyPolicy::Iguana(pair) => PublicKey::from_raw_secp256k1(&pair.public_key.to_bytes())
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Couldn't generate public key")),

                PrivKeyPolicy::HDWallet { activated_key, .. } => {
                    PublicKey::from_raw_secp256k1(&activated_key.public_key.to_bytes())
                        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Couldn't generate public key"))
                },

                _ => Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "UnsupportedModeForPrivKeyPolicy",
                )),
            },
            Self::PublicKey(account_public_key) => Ok(*account_public_key),
        }
    }

    pub(crate) fn activated_key_or_err(&self) -> Result<&Secp256k1Secret, MmError<PrivKeyPolicyNotAllowed>> {
        match self {
            Self::PrivateKey(private_key) => Ok(private_key.activated_key_or_err()?.private_key_secret.as_ref()),
            Self::PublicKey(_) => MmError::err(PrivKeyPolicyNotAllowed::UnsupportedMethod(
                "`activated_key_or_err` is not supported for pubkey-only activations".to_string(),
            )),
        }
    }

    pub(crate) fn activated_key(&self) -> Option<Secp256k1Secret> {
        match self {
            Self::PrivateKey(private_key) => Some(*private_key.activated_key()?.private_key_secret.as_ref()),
            Self::PublicKey(_) => None,
        }
    }

    pub(crate) fn path_to_coin_or_err(&self) -> Result<&HDPathToCoin, MmError<PrivKeyPolicyNotAllowed>> {
        match self {
            Self::PrivateKey(private_key) => Ok(private_key.path_to_coin_or_err()?),
            Self::PublicKey(_) => MmError::err(PrivKeyPolicyNotAllowed::UnsupportedMethod(
                "`path_to_coin_or_err` is not supported for pubkey-only activations".to_string(),
            )),
        }
    }

    pub(crate) fn hd_wallet_derived_priv_key_or_err(
        &self,
        path_to_address: &DerivationPath,
    ) -> Result<Secp256k1Secret, MmError<PrivKeyPolicyNotAllowed>> {
        match self {
            Self::PrivateKey(pair) => pair.hd_wallet_derived_priv_key_or_err(path_to_address),
            Self::PublicKey(_) => MmError::err(PrivKeyPolicyNotAllowed::UnsupportedMethod(
                "`hd_wallet_derived_priv_key_or_err` is not supported for pubkey-only activations".to_string(),
            )),
        }
    }
}

struct TendermintRpcClient(AsyncMutex<TendermintRpcClientImpl>);

struct TendermintRpcClientImpl {
    rpc_clients: Vec<HttpClient>,
}

#[async_trait]
impl RpcCommonOps for TendermintCoin {
    type RpcClient = HttpClient;
    type Error = TendermintCoinRpcError;

    async fn get_live_client(&self) -> Result<Self::RpcClient, Self::Error> {
        let mut client_impl = self.client.0.lock().await;
        // try to find first live client
        for (i, client) in client_impl.rpc_clients.clone().into_iter().enumerate() {
            match client.perform(HealthRequest).timeout(Duration::from_secs(15)).await {
                Ok(Ok(_)) => {
                    // Bring the live client to the front of rpc_clients
                    client_impl.rpc_clients.rotate_left(i);
                    return Ok(client);
                },
                Ok(Err(rpc_error)) => {
                    debug!("Could not perform healthcheck on: {:?}. Error: {}", &client, rpc_error);
                },
                Err(timeout_error) => {
                    debug!("Healthcheck timeout exceed on: {:?}. Error: {}", &client, timeout_error);
                },
            };
        }
        return Err(TendermintCoinRpcError::RpcClientError(
            "All the current rpc nodes are unavailable.".to_string(),
        ));
    }
}

#[derive(PartialEq)]
pub enum TendermintWalletConnectionType {
    Wc(String),
    WcLedger(String),
    KeplrLedger,
    Keplr,
    Native,
}

impl Default for TendermintWalletConnectionType {
    fn default() -> Self { Self::Native }
}

pub struct TendermintCoinImpl {
    ticker: String,
    /// As seconds
    avg_blocktime: u8,
    /// My address
    pub account_id: AccountId,
    pub(super) account_prefix: String,
    pub(super) activation_policy: TendermintActivationPolicy,
    pub(crate) decimals: u8,
    pub(super) denom: Denom,
    pub(crate) chain_id: ChainId,
    gas_price: Option<f64>,
    pub tokens_info: PaMutex<HashMap<String, ActivatedTokenInfo>>,
    /// This spawner is used to spawn coin's related futures that should be aborted on coin deactivation
    /// or on [`MmArc::stop`].
    pub(super) abortable_system: AbortableQueue,
    pub(crate) history_sync_state: Mutex<HistorySyncState>,
    client: TendermintRpcClient,
    pub(crate) chain_registry_name: Option<String>,
    pub(crate) ctx: MmWeak,
    pub(crate) wallet_type: TendermintWalletConnectionType,
}

#[derive(Clone)]
pub struct TendermintCoin(Arc<TendermintCoinImpl>);

impl Deref for TendermintCoin {
    type Target = TendermintCoinImpl;

    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone)]
pub struct TendermintInitError {
    pub ticker: String,
    pub kind: TendermintInitErrorKind,
}

#[derive(Display, Debug, Clone)]
pub enum TendermintInitErrorKind {
    Internal(String),
    InvalidPrivKey(String),
    CouldNotGenerateAccountId(String),
    EmptyRpcUrls,
    RpcClientInitError(String),
    InvalidChainId(String),
    InvalidDenom(String),
    InvalidPathToAddress(String),
    #[display(fmt = "'derivation_path' field is not found in config")]
    DerivationPathIsNotSet,
    #[display(fmt = "'account' field is not found in config")]
    AccountIsNotSet,
    #[display(fmt = "'address_index' field is not found in config")]
    AddressIndexIsNotSet,
    #[display(fmt = "Error deserializing 'derivation_path': {}", _0)]
    ErrorDeserializingDerivationPath(String),
    #[display(fmt = "Error deserializing 'path_to_address': {}", _0)]
    ErrorDeserializingPathToAddress(String),
    PrivKeyPolicyNotAllowed(PrivKeyPolicyNotAllowed),
    RpcError(String),
    #[display(fmt = "avg_blocktime is missing in coin configuration")]
    AvgBlockTimeMissing,
    #[display(fmt = "avg_blocktime must be in-between '0' and '255'.")]
    AvgBlockTimeInvalid,
    BalanceStreamInitError(String),
    #[display(fmt = "Watcher features can not be used with pubkey-only activation policy.")]
    CantUseWatchersWithPubkeyPolicy,
    #[display(
        fmt = "Unable to fetch chain account from WalletConnect. Please try again or reconnect your session - {}",
        _0
    )]
    UnableToFetchChainAccount(String),
}

/// TODO: Rename this into `ClientRpcError` because this is very
/// confusing atm.
#[derive(Display, Debug, Serialize, SerializeErrorType)]
#[serde(tag = "error_type", content = "error_data")]
pub enum TendermintCoinRpcError {
    Prost(String),
    InvalidResponse(String),
    PerformError(String),
    RpcClientError(String),
    InternalError(String),
    #[display(fmt = "Account type '{}' is not supported for HTLCs", prefix)]
    UnexpectedAccountType {
        prefix: String,
    },
}

impl From<DecodeError> for TendermintCoinRpcError {
    fn from(err: DecodeError) -> Self { TendermintCoinRpcError::Prost(err.to_string()) }
}

impl From<PrivKeyPolicyNotAllowed> for TendermintCoinRpcError {
    fn from(err: PrivKeyPolicyNotAllowed) -> Self { TendermintCoinRpcError::InternalError(err.to_string()) }
}

impl From<TendermintCoinRpcError> for WithdrawError {
    fn from(err: TendermintCoinRpcError) -> Self { WithdrawError::Transport(err.to_string()) }
}

impl From<TendermintCoinRpcError> for BalanceError {
    fn from(err: TendermintCoinRpcError) -> Self {
        match err {
            TendermintCoinRpcError::InvalidResponse(e) => BalanceError::InvalidResponse(e),
            TendermintCoinRpcError::Prost(e) => BalanceError::InvalidResponse(e),
            TendermintCoinRpcError::PerformError(e) | TendermintCoinRpcError::RpcClientError(e) => {
                BalanceError::Transport(e)
            },
            TendermintCoinRpcError::InternalError(e) => BalanceError::Internal(e),
            TendermintCoinRpcError::UnexpectedAccountType { prefix } => {
                BalanceError::Internal(format!("Account type '{prefix}' is not supported for HTLCs"))
            },
        }
    }
}

impl From<TendermintCoinRpcError> for ValidatePaymentError {
    fn from(err: TendermintCoinRpcError) -> Self {
        match err {
            TendermintCoinRpcError::InvalidResponse(e) => ValidatePaymentError::InvalidRpcResponse(e),
            TendermintCoinRpcError::Prost(e) => ValidatePaymentError::InvalidRpcResponse(e),
            TendermintCoinRpcError::PerformError(e) | TendermintCoinRpcError::RpcClientError(e) => {
                ValidatePaymentError::Transport(e)
            },
            TendermintCoinRpcError::InternalError(e) => ValidatePaymentError::InternalError(e),
            TendermintCoinRpcError::UnexpectedAccountType { prefix } => {
                ValidatePaymentError::InvalidParameter(format!("Account type '{prefix}' is not supported for HTLCs"))
            },
        }
    }
}

impl From<TendermintCoinRpcError> for TradePreimageError {
    fn from(err: TendermintCoinRpcError) -> Self { TradePreimageError::Transport(err.to_string()) }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<tendermint_rpc::Error> for TendermintCoinRpcError {
    fn from(err: tendermint_rpc::Error) -> Self { TendermintCoinRpcError::PerformError(err.to_string()) }
}

#[cfg(target_arch = "wasm32")]
impl From<PerformError> for TendermintCoinRpcError {
    fn from(err: PerformError) -> Self { TendermintCoinRpcError::PerformError(err.to_string()) }
}

impl From<TendermintCoinRpcError> for RawTransactionError {
    fn from(err: TendermintCoinRpcError) -> Self { RawTransactionError::Transport(err.to_string()) }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CosmosTransaction {
    pub data: cosmrs::proto::cosmos::tx::v1beta1::TxRaw,
}

impl crate::Transaction for CosmosTransaction {
    fn tx_hex(&self) -> Vec<u8> { self.data.encode_to_vec() }

    fn tx_hash_as_bytes(&self) -> BytesJson {
        let bytes = self.data.encode_to_vec();
        let hash = sha256(&bytes);
        hash.to_vec().into()
    }
}

pub(crate) fn account_id_from_privkey(priv_key: &[u8], prefix: &str) -> MmResult<AccountId, TendermintInitErrorKind> {
    let signing_key =
        SigningKey::from_slice(priv_key).map_to_mm(|e| TendermintInitErrorKind::InvalidPrivKey(e.to_string()))?;

    signing_key
        .public_key()
        .account_id(prefix)
        .map_to_mm(|e| TendermintInitErrorKind::CouldNotGenerateAccountId(e.to_string()))
}

#[derive(Display, Debug)]
pub enum AccountIdFromPubkeyHexErr {
    InvalidHexString(FromHexError),
    CouldNotCreateAccountId(ErrorReport),
}

impl From<FromHexError> for AccountIdFromPubkeyHexErr {
    fn from(err: FromHexError) -> Self { AccountIdFromPubkeyHexErr::InvalidHexString(err) }
}

impl From<ErrorReport> for AccountIdFromPubkeyHexErr {
    fn from(err: ErrorReport) -> Self { AccountIdFromPubkeyHexErr::CouldNotCreateAccountId(err) }
}

pub fn account_id_from_pubkey_hex(prefix: &str, pubkey: &str) -> Result<AccountId, AccountIdFromPubkeyHexErr> {
    let pubkey_bytes = hex::decode(pubkey)?;
    Ok(account_id_from_raw_pubkey(prefix, &pubkey_bytes)?)
}

pub fn account_id_from_raw_pubkey(prefix: &str, pubkey: &[u8]) -> Result<AccountId, ErrorReport> {
    let pubkey_hash = dhash160(pubkey);
    AccountId::new(prefix, pubkey_hash.as_slice())
}

#[derive(Debug, Clone, PartialEq)]
pub struct AllBalancesResult {
    pub platform_balance: BigDecimal,
    pub tokens_balances: HashMap<String, BigDecimal>,
}

#[derive(Debug, Display)]
enum SearchForSwapTxSpendErr {
    Cosmrs(ErrorReport),
    Rpc(TendermintCoinRpcError),
    TxMessagesEmpty,
    ClaimHtlcTxNotFound,
    UnexpectedHtlcState(i32),
    #[display(fmt = "Account type '{}' is not supported for HTLCs", prefix)]
    UnexpectedAccountType {
        prefix: String,
    },
    Proto(DecodeError),
}

impl From<ErrorReport> for SearchForSwapTxSpendErr {
    fn from(e: ErrorReport) -> Self { SearchForSwapTxSpendErr::Cosmrs(e) }
}

impl From<TendermintCoinRpcError> for SearchForSwapTxSpendErr {
    fn from(e: TendermintCoinRpcError) -> Self { SearchForSwapTxSpendErr::Rpc(e) }
}

impl From<DecodeError> for SearchForSwapTxSpendErr {
    fn from(e: DecodeError) -> Self { SearchForSwapTxSpendErr::Proto(e) }
}

#[async_trait]
impl TendermintCommons for TendermintCoin {
    fn platform_denom(&self) -> &Denom { &self.denom }

    fn set_history_sync_state(&self, new_state: HistorySyncState) {
        *self.history_sync_state.lock().unwrap() = new_state;
    }

    async fn get_block_timestamp(&self, block: i64) -> MmResult<Option<u64>, TendermintCoinRpcError> {
        let block_response = self.get_block_by_height(block).await?;
        let block_header = some_or_return_ok_none!(some_or_return_ok_none!(block_response.block).header);
        let timestamp = some_or_return_ok_none!(block_header.time);

        Ok(u64::try_from(timestamp.seconds).ok())
    }

    async fn get_all_balances(&self) -> MmResult<AllBalancesResult, TendermintCoinRpcError> {
        let platform_balance_denom = self
            .account_balance_for_denom(&self.account_id, self.denom.to_string())
            .await?;
        let platform_balance = big_decimal_from_sat_unsigned(platform_balance_denom, self.decimals);
        let ibc_assets_info = self.tokens_info.lock().clone();

        let mut requests = Vec::with_capacity(ibc_assets_info.len());
        for (denom, info) in ibc_assets_info {
            let fut = async move {
                let balance_denom = self
                    .account_balance_for_denom(&self.account_id, denom)
                    .await
                    .map_err(|e| e.into_inner())?;
                let balance_decimal = big_decimal_from_sat_unsigned(balance_denom, info.decimals);
                Ok::<_, TendermintCoinRpcError>((info.ticker, balance_decimal))
            };
            requests.push(fut);
        }
        let tokens_balances = try_join_all(requests).await?.into_iter().collect();

        Ok(AllBalancesResult {
            platform_balance,
            tokens_balances,
        })
    }

    #[inline(always)]
    async fn rpc_client(&self) -> MmResult<HttpClient, TendermintCoinRpcError> {
        self.get_live_client().await.map_to_mm(|e| e)
    }
}

impl TendermintCoin {
    #[allow(clippy::too_many_arguments)]
    pub async fn init(
        ctx: &MmArc,
        ticker: String,
        conf: TendermintConf,
        protocol_info: TendermintProtocolInfo,
        nodes: Vec<RpcNode>,
        tx_history: bool,
        activation_policy: TendermintActivationPolicy,
        wallet_type: Option<TendermintWalletConnectionType>,
    ) -> MmResult<Self, TendermintInitError> {
        if nodes.is_empty() {
            return MmError::err(TendermintInitError {
                ticker,
                kind: TendermintInitErrorKind::EmptyRpcUrls,
            });
        }

        let account_id = activation_policy
            .generate_account_id(&protocol_info.account_prefix)
            .map_to_mm(|e| TendermintInitError {
                ticker: ticker.clone(),
                kind: TendermintInitErrorKind::CouldNotGenerateAccountId(e.to_string()),
            })?;

        let rpc_clients = clients_from_urls(ctx, nodes).mm_err(|kind| TendermintInitError {
            ticker: ticker.clone(),
            kind,
        })?;

        let client_impl = TendermintRpcClientImpl { rpc_clients };

        let chain_id = ChainId::try_from(protocol_info.chain_id).map_to_mm(|e| TendermintInitError {
            ticker: ticker.clone(),
            kind: TendermintInitErrorKind::InvalidChainId(e.to_string()),
        })?;

        let denom = Denom::from_str(&protocol_info.denom).map_to_mm(|e| TendermintInitError {
            ticker: ticker.clone(),
            kind: TendermintInitErrorKind::InvalidDenom(e.to_string()),
        })?;

        let history_sync_state = if tx_history {
            HistorySyncState::NotStarted
        } else {
            HistorySyncState::NotEnabled
        };

        // Create an abortable system linked to the `MmCtx` so if the context is stopped via `MmArc::stop`,
        // all spawned futures related to `TendermintCoin` will be aborted as well.
        let abortable_system = ctx
            .abortable_system
            .create_subsystem()
            .map_to_mm(|e| TendermintInitError {
                ticker: ticker.clone(),
                kind: TendermintInitErrorKind::Internal(e.to_string()),
            })?;

        Ok(TendermintCoin(Arc::new(TendermintCoinImpl {
            ticker,
            account_id,
            account_prefix: protocol_info.account_prefix,
            activation_policy,
            decimals: protocol_info.decimals,
            denom,
            chain_id,
            gas_price: protocol_info.gas_price,
            avg_blocktime: conf.avg_blocktime,
            tokens_info: PaMutex::new(HashMap::new()),
            abortable_system,
            history_sync_state: Mutex::new(history_sync_state),
            client: TendermintRpcClient(AsyncMutex::new(client_impl)),
            chain_registry_name: protocol_info.chain_registry_name,
            ctx: ctx.weak(),
            wallet_type: wallet_type.unwrap_or_default(),
        })))
    }

    /// Extracts corresponding IBC channel ID for `AccountId` from https://github.com/KomodoPlatform/chain-registry/tree/nucl.
    pub(crate) async fn detect_channel_id_for_ibc_transfer(
        &self,
        to_address: &AccountId,
    ) -> Result<String, MmError<WithdrawError>> {
        let ctx = MmArc::from_weak(&self.ctx).ok_or_else(|| WithdrawError::InternalError("No context".to_owned()))?;

        let source_registry_name = self
            .chain_registry_name
            .clone()
            .ok_or_else(|| WithdrawError::RegistryNameIsMissing(to_address.prefix().to_owned()))?;

        let destination_registry_name = chain_registry_name_from_account_prefix(&ctx, to_address.prefix())
            .ok_or_else(|| WithdrawError::RegistryNameIsMissing(to_address.prefix().to_owned()))?;

        let channels = get_ibc_transfer_channels(source_registry_name, destination_registry_name)
            .await
            .map_err(|_| WithdrawError::IBCChannelCouldNotFound(to_address.to_string()))?;

        Ok(channels
            .ibc_transfer_channels
            .last()
            .ok_or_else(|| WithdrawError::InternalError("channel list can not be empty".to_owned()))?
            .channel_id
            .clone())
    }

    #[inline(always)]
    fn gas_price(&self) -> f64 { self.gas_price.unwrap_or(DEFAULT_GAS_PRICE) }

    #[allow(unused)]
    async fn get_latest_block(&self) -> MmResult<GetLatestBlockResponse, TendermintCoinRpcError> {
        let request = GetLatestBlockRequest {};
        let request = AbciRequest::new(
            Some(ABCI_GET_LATEST_BLOCK_PATH.to_string()),
            request.encode_to_vec(),
            ABCI_REQUEST_HEIGHT,
            ABCI_REQUEST_PROVE,
        );

        let response = self.rpc_client().await?.perform(request).await?;

        Ok(GetLatestBlockResponse::decode(response.response.value.as_slice())?)
    }

    #[allow(unused)]
    async fn get_block_by_height(&self, height: i64) -> MmResult<GetBlockByHeightResponse, TendermintCoinRpcError> {
        let request = GetBlockByHeightRequest { height };
        let request = AbciRequest::new(
            Some(ABCI_GET_BLOCK_BY_HEIGHT_PATH.to_string()),
            request.encode_to_vec(),
            ABCI_REQUEST_HEIGHT,
            ABCI_REQUEST_PROVE,
        );

        let response = self.rpc_client().await?.perform(request).await?;

        Ok(GetBlockByHeightResponse::decode(response.response.value.as_slice())?)
    }

    // We must simulate the tx on rpc nodes in order to calculate network fee.
    // Right now cosmos doesn't expose any of gas price and fee informations directly.
    // Therefore, we can call SimulateRequest or CheckTx(doesn't work with using Abci interface) to get used gas or fee itself.
    pub(super) fn gen_simulated_tx(
        &self,
        account_info: &BaseAccount,
        priv_key: &Secp256k1Secret,
        tx_payload: Any,
        timeout_height: u64,
        memo: String,
    ) -> cosmrs::Result<Vec<u8>> {
        let fee_amount = Coin {
            denom: self.denom.clone(),
            amount: 0_u64.into(),
        };

        let fee = Fee::from_amount_and_gas(fee_amount, GAS_LIMIT_DEFAULT);

        let signkey = SigningKey::from_slice(priv_key.as_slice())?;
        let tx_body = tx::Body::new(vec![tx_payload], memo, timeout_height as u32);
        let auth_info = SignerInfo::single_direct(Some(signkey.public_key()), account_info.sequence).auth_info(fee);
        let sign_doc = SignDoc::new(&tx_body, &auth_info, &self.chain_id, account_info.account_number)?;
        sign_doc.sign(&signkey)?.to_bytes()
    }

    /// This is converted from irismod and cosmos-sdk source codes written in golang.
    /// Refs:
    ///  - Main algorithm: https://github.com/irisnet/irismod/blob/main/modules/htlc/types/htlc.go#L157
    ///  - Coins string building https://github.com/cosmos/cosmos-sdk/blob/main/types/coin.go#L210-L225
    fn calculate_htlc_id(
        &self,
        from_address: &AccountId,
        to_address: &AccountId,
        amount: &[Coin],
        secret_hash: &[u8],
    ) -> String {
        // Needs to be sorted if contains multiple coins
        // let mut amount = amount;
        // amount.sort();

        let coins_string = amount
            .iter()
            .map(|t| format!("{}{}", t.amount, t.denom))
            .collect::<Vec<String>>()
            .join(",");

        let mut htlc_id = vec![];
        htlc_id.extend_from_slice(secret_hash);
        htlc_id.extend_from_slice(&from_address.to_bytes());
        htlc_id.extend_from_slice(&to_address.to_bytes());
        htlc_id.extend_from_slice(coins_string.as_bytes());
        sha256(&htlc_id).to_string().to_uppercase()
    }

    async fn common_send_raw_tx_bytes(
        &self,
        tx_payload: Any,
        fee: Fee,
        timeout_height: u64,
        memo: String,
        timeout: Duration,
    ) -> Result<(String, Raw), TransactionErr> {
        // As there wouldn't be enough time to process the data, to mitigate potential edge problems (such as attempting to send transaction
        // bytes half a second before expiration, which may take longer to send and result in the transaction amount being wasted due to a timeout),
        // reduce the expiration time by 5 seconds.
        const SAFETY_MARGIN: Duration = Duration::from_secs(5);
        let expiration = try_tx_s!(timeout
            .checked_sub(SAFETY_MARGIN)
            .ok_or("Timeout duration is too short"));

        match self.activation_policy {
            TendermintActivationPolicy::PrivateKey(_) => {
                try_tx_s!(
                    self.seq_safe_send_raw_tx_bytes(tx_payload, fee, timeout_height, memo)
                        .timeout(expiration)
                        .await
                )
            },
            TendermintActivationPolicy::PublicKey(_) => {
                if self.is_wallet_connect() {
                    return try_tx_s!(
                        self.seq_safe_send_raw_tx_bytes(tx_payload, fee, timeout_height, memo)
                            .timeout(expiration)
                            .await
                    );
                };

                try_tx_s!(
                    self.send_unsigned_tx_externally(tx_payload, fee, timeout_height, memo, expiration)
                        .timeout(expiration)
                        .await
                )
            },
        }
    }

    async fn get_tx_raw(
        &self,
        account_info: &BaseAccount,
        tx_payload: Any,
        fee: Fee,
        timeout_height: u64,
        memo: String,
    ) -> Result<Raw, TransactionErr> {
        if self.is_wallet_connect() {
            let ctx = try_tx_s!(MmArc::from_weak(&self.ctx).ok_or(ERRL!("ctx must be initialized already")));
            let wc = try_tx_s!(WalletConnectCtx::from_ctx(&ctx).map_err(|e| e.to_string()));
            let SerializedUnsignedTx { tx_json, .. } = if self.is_ledger_connection() {
                try_tx_s!(self.any_to_legacy_amino_json(account_info, tx_payload, fee, timeout_height, memo))
            } else {
                try_tx_s!(self.any_to_serialized_sign_doc(account_info, tx_payload, fee, timeout_height, memo))
            };

            return Ok(try_tx_s!(self.wc_sign_tx(&wc, tx_json).await.map_err(|err| err.to_string())).into());
        }

        let tx_raw = try_tx_s!(self.any_to_signed_raw_tx(
            try_tx_s!(self.activation_policy.activated_key_or_err()),
            account_info,
            tx_payload,
            fee,
            timeout_height,
            memo,
        ));

        Ok(tx_raw)
    }

    async fn seq_safe_send_raw_tx_bytes(
        &self,
        tx_payload: Any,
        fee: Fee,
        timeout_height: u64,
        memo: String,
    ) -> Result<(String, Raw), TransactionErr> {
        let mut account_info = try_tx_s!(self.account_info(&self.account_id).await);

        loop {
            let tx_raw = try_tx_s!(
                self.get_tx_raw(
                    &account_info,
                    tx_payload.clone(),
                    fee.clone(),
                    timeout_height,
                    memo.clone(),
                )
                .await
            );

            // Attempt to send the transaction bytes
            match self.send_raw_tx_bytes(try_tx_s!(&tx_raw.to_bytes())).compat().await {
                Ok(tx_id) => {
                    return Ok((tx_id, tx_raw));
                },
                Err(e) => {
                    // Handle sequence number mismatch and retry
                    if e.contains(ACCOUNT_SEQUENCE_ERR) {
                        account_info.sequence = try_tx_s!(parse_expected_sequence_number(&e));
                        debug!("Account sequence mismatch, retrying...");
                        continue;
                    }

                    return Err(TransactionErr::Plain(ERRL!("Transaction failed: {}", e)));
                },
            }
        }
    }

    async fn send_unsigned_tx_externally(
        &self,
        tx_payload: Any,
        fee: Fee,
        timeout_height: u64,
        memo: String,
        timeout: Duration,
    ) -> Result<(String, Raw), TransactionErr> {
        #[derive(Deserialize)]
        struct TxHashData {
            hash: String,
        }

        let ctx = try_tx_s!(MmArc::from_weak(&self.ctx).ok_or(ERRL!("ctx must be initialized already")));

        let account_info = try_tx_s!(self.account_info(&self.account_id).await);
        let SerializedUnsignedTx { tx_json, body_bytes } = if self.is_ledger_connection() {
            try_tx_s!(self.any_to_legacy_amino_json(&account_info, tx_payload, fee, timeout_height, memo))
        } else {
            try_tx_s!(self.any_to_serialized_sign_doc(&account_info, tx_payload, fee, timeout_height, memo))
        };

        let data: TxHashData = try_tx_s!(ctx
            .ask_for_data(&format!("TX_HASH:{}", self.ticker()), tx_json.clone(), timeout)
            .await
            .map_err(|e| ERRL!("{}", e)));

        let tx = try_tx_s!(self.request_tx(data.hash.clone()).await.map_err(|e| ERRL!("{}", e)));

        let tx_raw = TxRaw {
            body_bytes: tx.body.as_ref().map(Message::encode_to_vec).unwrap_or_default(),
            auth_info_bytes: tx.auth_info.as_ref().map(Message::encode_to_vec).unwrap_or_default(),
            signatures: tx.signatures,
        };

        if body_bytes != tx_raw.body_bytes {
            return Err(crate::TransactionErr::Plain(ERRL!(
                "Unsigned transaction don't match with the externally provided transaction."
            )));
        }

        Ok((data.hash, Raw::from(tx_raw)))
    }

    #[allow(deprecated)]
    pub(super) async fn calculate_fee(
        &self,
        msg: Any,
        timeout_height: u64,
        memo: String,
        withdraw_fee: Option<WithdrawFee>,
    ) -> MmResult<Fee, TendermintCoinRpcError> {
        let Ok(activated_priv_key) = self.activation_policy.activated_key_or_err() else {
            let (gas_price, gas_limit) = self.gas_info_for_withdraw(&withdraw_fee, GAS_LIMIT_DEFAULT);
            let amount = ((GAS_WANTED_BASE_VALUE * 1.5) * gas_price).ceil();

            let fee_amount = Coin {
                denom: self.platform_denom().clone(),
                amount: (amount as u64).into(),
            };

            return Ok(Fee::from_amount_and_gas(fee_amount, gas_limit));
        };

        let mut account_info = self.account_info(&self.account_id).await?;
        let (response, raw_response) = loop {
            let tx_bytes = self
                .gen_simulated_tx(
                    &account_info,
                    activated_priv_key,
                    msg.clone(),
                    timeout_height,
                    memo.clone(),
                )
                .map_to_mm(|e| TendermintCoinRpcError::InternalError(format!("{}", e)))?;

            let request = AbciRequest::new(
                Some(ABCI_SIMULATE_TX_PATH.to_string()),
                SimulateRequest { tx_bytes, tx: None }.encode_to_vec(),
                ABCI_REQUEST_HEIGHT,
                ABCI_REQUEST_PROVE,
            );

            let raw_response = self.rpc_client().await?.perform(request).await?;

            let log = raw_response.response.log.to_string();
            if log.contains(ACCOUNT_SEQUENCE_ERR) {
                account_info.sequence = parse_expected_sequence_number(&log)?;
                debug!("Got wrong account sequence, trying again.");
                continue;
            }

            match raw_response.response.code {
                cosmrs::tendermint::abci::Code::Ok => {},
                cosmrs::tendermint::abci::Code::Err(ecode) => {
                    return MmError::err(TendermintCoinRpcError::InvalidResponse(format!(
                        "Could not read gas_info. Error code: {} Message: {}",
                        ecode, raw_response.response.log
                    )));
                },
            };

            break (
                SimulateResponse::decode(raw_response.response.value.as_slice())?,
                raw_response,
            );
        };

        let gas = response.gas_info.as_ref().ok_or_else(|| {
            TendermintCoinRpcError::InvalidResponse(format!(
                "Could not read gas_info. Invalid Response: {:?}",
                raw_response
            ))
        })?;

        let (gas_price, gas_limit) = self.gas_info_for_withdraw(&withdraw_fee, GAS_LIMIT_DEFAULT);

        let amount = ((gas.gas_used as f64 * 1.5) * gas_price).ceil();

        let fee_amount = Coin {
            denom: self.platform_denom().clone(),
            amount: (amount as u64).into(),
        };

        Ok(Fee::from_amount_and_gas(fee_amount, gas_limit))
    }

    #[allow(deprecated)]
    pub(super) async fn calculate_account_fee_amount_as_u64(
        &self,
        account_id: &AccountId,
        priv_key: Option<Secp256k1Secret>,
        msg: Any,
        timeout_height: u64,
        memo: String,
        withdraw_fee: Option<WithdrawFee>,
    ) -> MmResult<u64, TendermintCoinRpcError> {
        let Some(priv_key) = priv_key else {
            let (gas_price, _) = self.gas_info_for_withdraw(&withdraw_fee, 0);
            return Ok(((GAS_WANTED_BASE_VALUE * 1.5) * gas_price).ceil() as u64);
        };

        let mut account_info = self.account_info(account_id).await?;
        let (response, raw_response) = loop {
            let tx_bytes = self
                .gen_simulated_tx(&account_info, &priv_key, msg.clone(), timeout_height, memo.clone())
                .map_to_mm(|e| TendermintCoinRpcError::InternalError(format!("{}", e)))?;

            let request = AbciRequest::new(
                Some(ABCI_SIMULATE_TX_PATH.to_string()),
                SimulateRequest { tx_bytes, tx: None }.encode_to_vec(),
                ABCI_REQUEST_HEIGHT,
                ABCI_REQUEST_PROVE,
            );

            let raw_response = self.rpc_client().await?.perform(request).await?;

            let log = raw_response.response.log.to_string();
            if log.contains(ACCOUNT_SEQUENCE_ERR) {
                account_info.sequence = parse_expected_sequence_number(&log)?;
                debug!("Got wrong account sequence, trying again.");
                continue;
            }

            match raw_response.response.code {
                cosmrs::tendermint::abci::Code::Ok => {},
                cosmrs::tendermint::abci::Code::Err(ecode) => {
                    return MmError::err(TendermintCoinRpcError::InvalidResponse(format!(
                        "Could not read gas_info. Error code: {} Message: {}",
                        ecode, raw_response.response.log
                    )));
                },
            };

            break (
                SimulateResponse::decode(raw_response.response.value.as_slice())?,
                raw_response,
            );
        };

        let gas = response.gas_info.as_ref().ok_or_else(|| {
            TendermintCoinRpcError::InvalidResponse(format!(
                "Could not read gas_info. Invalid Response: {:?}",
                raw_response
            ))
        })?;

        let (gas_price, _) = self.gas_info_for_withdraw(&withdraw_fee, 0);

        Ok(((gas.gas_used as f64 * 1.5) * gas_price).ceil() as u64)
    }

    pub(super) async fn account_info(&self, account_id: &AccountId) -> MmResult<BaseAccount, TendermintCoinRpcError> {
        let request = QueryAccountRequest {
            address: account_id.to_string(),
        };
        let request = AbciRequest::new(
            Some(ABCI_QUERY_ACCOUNT_PATH.to_string()),
            request.encode_to_vec(),
            ABCI_REQUEST_HEIGHT,
            ABCI_REQUEST_PROVE,
        );

        let response = self.rpc_client().await?.perform(request).await?;
        let account_response = QueryAccountResponse::decode(response.response.value.as_slice())?;
        let account = account_response
            .account
            .or_mm_err(|| TendermintCoinRpcError::InvalidResponse("Account is None".into()))?;

        let base_account = match BaseAccount::decode(account.value.as_slice()) {
            Ok(account) => account,
            Err(err) if &self.account_prefix == "iaa" => {
                let ethermint_account = EthermintAccount::decode(account.value.as_slice())?;

                ethermint_account
                    .base_account
                    .or_mm_err(|| TendermintCoinRpcError::Prost(err.to_string()))?
            },
            Err(err) => {
                return MmError::err(TendermintCoinRpcError::Prost(err.to_string()));
            },
        };

        Ok(base_account)
    }

    pub(super) async fn account_balance_for_denom(
        &self,
        account_id: &AccountId,
        denom: String,
    ) -> MmResult<u64, TendermintCoinRpcError> {
        let request = QueryBalanceRequest {
            address: account_id.to_string(),
            denom,
        };
        let request = AbciRequest::new(
            Some(ABCI_QUERY_BALANCE_PATH.to_string()),
            request.encode_to_vec(),
            ABCI_REQUEST_HEIGHT,
            ABCI_REQUEST_PROVE,
        );

        let response = self.rpc_client().await?.perform(request).await?;
        let response = QueryBalanceResponse::decode(response.response.value.as_slice())?;
        response
            .balance
            .or_mm_err(|| TendermintCoinRpcError::InvalidResponse("balance is None".into()))?
            .amount
            .parse()
            .map_to_mm(|e| TendermintCoinRpcError::InvalidResponse(format!("balance is not u64, err {}", e)))
    }

    #[allow(clippy::result_large_err)]
    pub(super) fn account_id_and_pk_for_withdraw(
        &self,
        withdraw_from: Option<WithdrawFrom>,
    ) -> Result<(AccountId, Option<H256>), WithdrawError> {
        if let TendermintActivationPolicy::PublicKey(_) = self.activation_policy {
            return Ok((self.account_id.clone(), None));
        }

        match withdraw_from {
            Some(from) => {
                let path_to_coin = self
                    .activation_policy
                    .path_to_coin_or_err()
                    .map_err(|e| WithdrawError::InternalError(e.to_string()))?;

                let path_to_address = from
                    .to_address_path(path_to_coin.coin_type())
                    .map_err(|e| WithdrawError::InternalError(e.to_string()))?
                    .to_derivation_path(path_to_coin)
                    .map_err(|e| WithdrawError::InternalError(e.to_string()))?;

                let priv_key = self
                    .activation_policy
                    .hd_wallet_derived_priv_key_or_err(&path_to_address)
                    .map_err(|e| WithdrawError::InternalError(e.to_string()))?;

                let account_id = account_id_from_privkey(priv_key.as_slice(), &self.account_prefix)
                    .map_err(|e| WithdrawError::InternalError(e.to_string()))?;
                Ok((account_id, Some(priv_key)))
            },
            None => {
                let activated_key = self
                    .activation_policy
                    .activated_key_or_err()
                    .map_err(|e| WithdrawError::InternalError(e.to_string()))?;

                Ok((self.account_id.clone(), Some(*activated_key)))
            },
        }
    }

    pub(super) async fn any_to_transaction_data(
        &self,
        maybe_pk: Option<H256>,
        message: Any,
        account_info: &BaseAccount,
        fee: Fee,
        timeout_height: u64,
        memo: String,
    ) -> Result<TransactionData, ErrorReport> {
        if let Some(priv_key) = maybe_pk {
            let tx_raw = self.any_to_signed_raw_tx(&priv_key, account_info, message, fee, timeout_height, memo)?;
            let tx_bytes = tx_raw.to_bytes()?;
            let hash = sha256(&tx_bytes);

            return Ok(TransactionData::new_signed(
                tx_bytes.into(),
                hex::encode_upper(hash.as_slice()),
            ));
        };

        let SerializedUnsignedTx { tx_json, .. } = if self.is_ledger_connection() {
            self.any_to_legacy_amino_json(account_info, message, fee, timeout_height, memo)
        } else {
            self.any_to_serialized_sign_doc(account_info, message, fee, timeout_height, memo)
        }?;

        if self.is_wallet_connect() {
            let ctx = MmArc::from_weak(&self.ctx)
                .ok_or(MyAddressError::InternalError(ERRL!("ctx must be initialized already")))?;
            let wallet_connect = WalletConnectCtx::from_ctx(&ctx)?;

            let tx_raw: Raw = self.wc_sign_tx(&wallet_connect, tx_json).await?.into();
            let tx_bytes = tx_raw.to_bytes()?;
            let hash = sha256(&tx_bytes);

            return Ok(TransactionData::new_signed(
                tx_bytes.into(),
                hex::encode_upper(hash.as_slice()),
            ));
        };

        Ok(TransactionData::Unsigned(tx_json))
    }

    fn gen_create_htlc_tx(
        &self,
        denom: Denom,
        to: &AccountId,
        amount: cosmrs::Amount,
        secret_hash: &[u8],
        time_lock: u64,
    ) -> MmResult<TendermintHtlc, TxMarshalingErr> {
        let amount = vec![Coin { denom, amount }];
        let timestamp = 0_u64;

        let htlc_type = HtlcType::from_str(&self.account_prefix).map_err(|_| {
            TxMarshalingErr::NotSupported(format!(
                "Account type '{}' is not supported for HTLCs",
                self.account_prefix
            ))
        })?;

        let msg_payload = CreateHtlcMsg::new(
            htlc_type,
            self.account_id.clone(),
            to.clone(),
            amount.clone(),
            hex::encode(secret_hash),
            timestamp,
            time_lock,
        );

        let htlc_id = self.calculate_htlc_id(&self.account_id, to, &amount, secret_hash);

        Ok(TendermintHtlc {
            id: htlc_id,
            msg_payload: msg_payload
                .to_any()
                .map_err(|e| MmError::new(TxMarshalingErr::InvalidInput(e.to_string())))?,
        })
    }

    fn gen_claim_htlc_tx(&self, htlc_id: String, secret: &[u8]) -> MmResult<TendermintHtlc, TxMarshalingErr> {
        let htlc_type = HtlcType::from_str(&self.account_prefix).map_err(|_| {
            TxMarshalingErr::NotSupported(format!(
                "Account type '{}' is not supported for HTLCs",
                self.account_prefix
            ))
        })?;

        let msg_payload = ClaimHtlcMsg::new(htlc_type, htlc_id.clone(), self.account_id.clone(), hex::encode(secret));

        Ok(TendermintHtlc {
            id: htlc_id,
            msg_payload: msg_payload
                .to_any()
                .map_err(|e| MmError::new(TxMarshalingErr::InvalidInput(e.to_string())))?,
        })
    }

    pub(super) fn any_to_signed_raw_tx(
        &self,
        priv_key: &Secp256k1Secret,
        account_info: &BaseAccount,
        tx_payload: Any,
        fee: Fee,
        timeout_height: u64,
        memo: String,
    ) -> cosmrs::Result<Raw> {
        let signkey = SigningKey::from_slice(priv_key.as_slice())?;
        let tx_body = tx::Body::new(vec![tx_payload], memo, timeout_height as u32);
        let auth_info = SignerInfo::single_direct(Some(signkey.public_key()), account_info.sequence).auth_info(fee);
        let sign_doc = SignDoc::new(&tx_body, &auth_info, &self.chain_id, account_info.account_number)?;
        sign_doc.sign(&signkey)
    }

    pub(super) fn any_to_serialized_sign_doc(
        &self,
        account_info: &BaseAccount,
        tx_payload: Any,
        fee: Fee,
        timeout_height: u64,
        memo: String,
    ) -> cosmrs::Result<SerializedUnsignedTx> {
        let tx_body = tx::Body::new(vec![tx_payload], memo, timeout_height as u32);
        let pubkey = self.activation_policy.public_key()?.into();
        let auth_info = SignerInfo::single_direct(Some(pubkey), account_info.sequence).auth_info(fee);
        let sign_doc = SignDoc::new(&tx_body, &auth_info, &self.chain_id, account_info.account_number)?;

        let tx_json = if self.is_wallet_connect() {
            // if wallet_type is WalletConnect, update tx_json to use WalletConnect type.
            json!({
                "signerAddress":  self.my_address()?,
                "signDoc": {
                    "accountNumber": sign_doc.account_number.to_string(),
                    "chainId": sign_doc.chain_id,
                    "bodyBytes": general_purpose::STANDARD.encode(&sign_doc.body_bytes),
                    "authInfoBytes": general_purpose::STANDARD.encode(&sign_doc.auth_info_bytes)
                }
            })
        } else {
            json!({
                "sign_doc": {
                    "body_bytes": &sign_doc.body_bytes,
                    "auth_info_bytes": sign_doc.auth_info_bytes,
                    "chain_id": sign_doc.chain_id,
                    "account_number": sign_doc.account_number,
                }
            })
        };

        Ok(SerializedUnsignedTx {
            tx_json,
            body_bytes: sign_doc.body_bytes,
        })
    }

    /// This should only be used for Keplr/WalletConnect from Ledger!
    /// When using Keplr from Ledger, they don't accept `SING_MODE_DIRECT` transactions.
    ///
    /// Visit https://docs.cosmos.network/main/build/architecture/adr-050-sign-mode-textual#context for more context.
    pub(super) fn any_to_legacy_amino_json(
        &self,
        account_info: &BaseAccount,
        tx_payload: Any,
        fee: Fee,
        timeout_height: u64,
        memo: String,
    ) -> cosmrs::Result<SerializedUnsignedTx> {
        const MSG_SEND_TYPE_URL: &str = "/cosmos.bank.v1beta1.MsgSend";
        const LEDGER_MSG_SEND_TYPE_URL: &str = "cosmos-sdk/MsgSend";

        // Ledger's keplr works as wallet-only, so `MsgSend` support is enough for now.
        if tx_payload.type_url != MSG_SEND_TYPE_URL {
            return Err(ErrorReport::new(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "Signing mode `SIGN_MODE_LEGACY_AMINO_JSON` is not supported for '{}' transaction type.",
                    tx_payload.type_url
                ),
            )));
        }

        let msg_send = MsgSend::from_any(&tx_payload)?;
        let timeout_height = u32::try_from(timeout_height)?;

        let amount: Vec<Json> = msg_send
            .amount
            .into_iter()
            .map(|t| {
                json!( {
                    "denom": t.denom,
                    // Numbers needs to be converted into string type.
                    // Ref: https://github.com/cosmos/ledger-cosmos/blob/c707129e59f6e0f07ad67161a6b75e8951af063c/docs/TXSPEC.md#json-format
                    "amount": t.amount.to_string(),
                })
            })
            .collect();

        let msg = json!({
            "type": LEDGER_MSG_SEND_TYPE_URL,
            "value": json!({
                "from_address": msg_send.from_address.to_string(),
                "to_address": msg_send.to_address.to_string(),
                "amount": amount,
            })
        });

        let fee_amount: Vec<Json> = fee
            .amount
            .into_iter()
            .map(|t| {
                json!( {
                    "denom": t.denom,
                    // Numbers needs to be converted into string type.
                    // Ref: https://github.com/cosmos/ledger-cosmos/blob/c707129e59f6e0f07ad67161a6b75e8951af063c/docs/TXSPEC.md#json-format
                    "amount": t.amount.to_string(),
                })
            })
            .collect();

        let (tx_json, body_bytes) = match self.wallet_type {
            TendermintWalletConnectionType::WcLedger(_) => {
                let signer_address = self.my_address().unwrap();
                let body_bytes = tx::Body::new(vec![tx_payload], &memo, timeout_height).into_bytes()?;
                let json = serde_json::json!({
                    "signerAddress": signer_address,
                    "signDoc": {
                        "account_number": account_info.account_number.to_string(),
                        "chain_id": self.chain_id.to_string(),
                        "fee": {
                            "amount": fee_amount,
                            "gas": fee.gas_limit.to_string()
                        },
                        "memo": memo,
                        "msgs": [msg],
                        "sequence": account_info.sequence.to_string(),
                    },
                });
                (json, body_bytes)
            },
            TendermintWalletConnectionType::KeplrLedger => {
                let original_tx_type_url = tx_payload.type_url.clone();
                let body_bytes = tx::Body::new(vec![tx_payload], &memo, timeout_height).into_bytes()?;
                let json = serde_json::json!({
                    "legacy_amino_json": {
                        "account_number": account_info.account_number.to_string(),
                        "chain_id": self.chain_id.to_string(),
                        "fee": {
                            "amount": fee_amount,
                            "gas": fee.gas_limit.to_string()
                        },
                        "memo": memo,
                        "msgs": [msg],
                        "sequence": account_info.sequence.to_string(),
                    },
                    "original_tx_type_url": original_tx_type_url,
                });
                (json, body_bytes)
            },
            _ => {
                return Err(ErrorReport::new(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Only WalletConnect activated with Ledger can call this funciton",
                )))
            },
        };

        Ok(SerializedUnsignedTx { tx_json, body_bytes })
    }

    pub fn add_activated_token_info(&self, ticker: String, decimals: u8, denom: Denom) {
        self.tokens_info
            .lock()
            .insert(denom.to_string(), ActivatedTokenInfo { decimals, ticker });
    }

    fn estimate_blocks_from_duration(&self, duration: u64) -> i64 {
        let estimated_time_lock = (duration / self.avg_blocktime as u64) as i64;

        estimated_time_lock.clamp(MIN_TIME_LOCK, MAX_TIME_LOCK)
    }

    pub(crate) fn check_if_my_payment_sent_for_denom(
        &self,
        decimals: u8,
        denom: Denom,
        other_pub: &[u8],
        secret_hash: &[u8],
        amount: &BigDecimal,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        let amount = try_fus!(sat_from_big_decimal(amount, decimals));
        let amount = vec![Coin {
            denom,
            amount: amount.into(),
        }];

        let pubkey_hash = dhash160(other_pub);
        let to_address = try_fus!(AccountId::new(&self.account_prefix, pubkey_hash.as_slice()));

        let htlc_id = self.calculate_htlc_id(&self.account_id, &to_address, &amount, secret_hash);

        let coin = self.clone();
        let fut = async move {
            let htlc_response = try_s!(coin.query_htlc(htlc_id.clone()).await);

            let Some(htlc_state) = htlc_response.htlc_state() else {
                return Ok(None);
            };

            match htlc_state {
                HTLC_STATE_OPEN | HTLC_STATE_COMPLETED | HTLC_STATE_REFUNDED => {},
                unexpected_state => return Err(format!("Unexpected state for HTLC {}", unexpected_state)),
            };

            let rpc_client = try_s!(coin.rpc_client().await);
            let q = format!("create_htlc.id = '{}'", htlc_id);

            let response = try_s!(
                // Search single tx
                rpc_client
                    .perform(TxSearchRequest::new(
                        q,
                        false,
                        1,
                        1,
                        TendermintResultOrder::Descending.into()
                    ))
                    .await
            );

            if let Some(tx) = response.txs.first() {
                if let cosmrs::tendermint::abci::Code::Err(err_code) = tx.tx_result.code {
                    return Err(format!(
                        "Got {} error code. Broadcasted HTLC likely isn't valid.",
                        err_code
                    ));
                }

                let deserialized_tx = try_s!(cosmrs::Tx::from_bytes(&tx.tx));
                let msg = try_s!(deserialized_tx.body.messages.first().ok_or("Tx body couldn't be read."));
                let htlc = try_s!(CreateHtlcProto::decode(
                    try_s!(HtlcType::from_str(&coin.account_prefix)),
                    msg.value.as_slice()
                ));

                let Some(hash_lock) = htlc_response.hash_lock() else {
                    return Ok(None);
                };

                if htlc.hash_lock().to_uppercase() == hash_lock.to_uppercase() {
                    let htlc = TransactionEnum::CosmosTransaction(CosmosTransaction {
                        data: try_s!(TxRaw::decode(tx.tx.as_slice())),
                    });
                    return Ok(Some(htlc));
                }
            }

            Ok(None)
        };

        Box::new(fut.boxed().compat())
    }

    pub(super) fn send_htlc_for_denom(
        &self,
        time_lock_duration: u64,
        other_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        denom: Denom,
        decimals: u8,
    ) -> TransactionFut {
        let pubkey_hash = dhash160(other_pub);
        let to = try_tx_fus!(AccountId::new(&self.account_prefix, pubkey_hash.as_slice()));

        let amount_as_u64 = try_tx_fus!(sat_from_big_decimal(&amount, decimals));
        let amount = cosmrs::Amount::from(amount_as_u64);

        let secret_hash = secret_hash.to_vec();
        let coin = self.clone();
        let fut = async move {
            let time_lock = coin.estimate_blocks_from_duration(time_lock_duration);

            let create_htlc_tx = try_tx_s!(coin.gen_create_htlc_tx(denom, &to, amount, &secret_hash, time_lock as u64));

            let current_block = try_tx_s!(coin.current_block().compat().await);
            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

            let fee = try_tx_s!(
                coin.calculate_fee(
                    create_htlc_tx.msg_payload.clone(),
                    timeout_height,
                    TX_DEFAULT_MEMO.to_owned(),
                    None
                )
                .await
            );

            let (_tx_id, tx_raw) = try_tx_s!(
                coin.common_send_raw_tx_bytes(
                    create_htlc_tx.msg_payload.clone(),
                    fee.clone(),
                    timeout_height,
                    TX_DEFAULT_MEMO.into(),
                    Duration::from_secs(time_lock_duration),
                )
                .await
            );

            Ok(TransactionEnum::CosmosTransaction(CosmosTransaction {
                data: tx_raw.into(),
            }))
        };

        Box::new(fut.boxed().compat())
    }

    pub(super) fn send_taker_fee_for_denom(
        &self,
        fee_addr: &[u8],
        amount: BigDecimal,
        denom: Denom,
        decimals: u8,
        uuid: &[u8],
        expires_at: u64,
    ) -> TransactionFut {
        let memo = try_tx_fus!(Uuid::from_slice(uuid)).to_string();
        let from_address = self.account_id.clone();
        let pubkey_hash = dhash160(fee_addr);
        let to_address = try_tx_fus!(AccountId::new(&self.account_prefix, pubkey_hash.as_slice()));

        let amount_as_u64 = try_tx_fus!(sat_from_big_decimal(&amount, decimals));
        let amount = cosmrs::Amount::from(amount_as_u64);

        let amount = vec![Coin { denom, amount }];

        let tx_payload = try_tx_fus!(MsgSend {
            from_address,
            to_address,
            amount,
        }
        .to_any());

        let coin = self.clone();
        let fut = async move {
            let current_block = try_tx_s!(coin.current_block().compat().await.map_to_mm(WithdrawError::Transport));
            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

            let fee = try_tx_s!(
                coin.calculate_fee(tx_payload.clone(), timeout_height, TX_DEFAULT_MEMO.to_owned(), None)
                    .await
            );

            let timeout = expires_at.checked_sub(now_sec()).unwrap_or_default();
            let (_tx_id, tx_raw) = try_tx_s!(
                coin.common_send_raw_tx_bytes(
                    tx_payload.clone(),
                    fee.clone(),
                    timeout_height,
                    memo.clone(),
                    Duration::from_secs(timeout)
                )
                .await
            );

            Ok(TransactionEnum::CosmosTransaction(CosmosTransaction {
                data: tx_raw.into(),
            }))
        };

        Box::new(fut.boxed().compat())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate_fee_for_denom(
        &self,
        fee_tx: &TransactionEnum,
        expected_sender: &[u8],
        fee_addr: &[u8],
        amount: &BigDecimal,
        decimals: u8,
        uuid: &[u8],
        denom: String,
    ) -> ValidatePaymentFut<()> {
        let tx = match fee_tx {
            TransactionEnum::CosmosTransaction(tx) => tx.clone(),
            invalid_variant => {
                return Box::new(futures01::future::err(
                    ValidatePaymentError::WrongPaymentTx(format!("Unexpected tx variant {:?}", invalid_variant)).into(),
                ))
            },
        };

        let uuid = try_f!(Uuid::from_slice(uuid).map_to_mm(|r| ValidatePaymentError::InvalidParameter(r.to_string())))
            .to_string();

        let sender_pubkey_hash = dhash160(expected_sender);
        let expected_sender_address = try_f!(AccountId::new(&self.account_prefix, sender_pubkey_hash.as_slice())
            .map_to_mm(|r| ValidatePaymentError::InvalidParameter(r.to_string())))
        .to_string();

        let dex_fee_addr_pubkey_hash = dhash160(fee_addr);
        let expected_dex_fee_address = try_f!(AccountId::new(
            &self.account_prefix,
            dex_fee_addr_pubkey_hash.as_slice()
        )
        .map_to_mm(|r| ValidatePaymentError::InvalidParameter(r.to_string())))
        .to_string();

        let expected_amount = try_f!(sat_from_big_decimal(amount, decimals));
        let expected_amount = CoinProto {
            denom,
            amount: expected_amount.to_string(),
        };

        let coin = self.clone();
        let fut = async move {
            let tx_body = TxBody::decode(tx.data.body_bytes.as_slice())
                .map_to_mm(|e| ValidatePaymentError::TxDeserializationError(e.to_string()))?;
            if tx_body.messages.len() != 1 {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(
                    "Tx body must have exactly one message".to_string(),
                ));
            }

            let msg = MsgSendProto::decode(tx_body.messages[0].value.as_slice())
                .map_to_mm(|e| ValidatePaymentError::TxDeserializationError(e.to_string()))?;
            if msg.to_address != expected_dex_fee_address {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Dex fee is sent to wrong address: {}, expected {}",
                    msg.to_address, expected_dex_fee_address
                )));
            }

            if msg.amount.len() != 1 {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(
                    "Msg must have exactly one Coin".to_string(),
                ));
            }

            if msg.amount[0] != expected_amount {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Invalid amount {:?}, expected {:?}",
                    msg.amount[0], expected_amount
                )));
            }

            if msg.from_address != expected_sender_address {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Invalid sender: {}, expected {}",
                    msg.from_address, expected_sender_address
                )));
            }

            if tx_body.memo != uuid {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Invalid memo: {}, expected {}",
                    msg.from_address, uuid
                )));
            }

            let encoded_tx = tx.data.encode_to_vec();
            let hash = hex::encode_upper(sha256(&encoded_tx).as_slice());
            let encoded_from_rpc = coin
                .request_tx(hash)
                .await
                .map_err(|e| MmError::new(ValidatePaymentError::TxDeserializationError(e.into_inner().to_string())))?
                .encode_to_vec();
            if encoded_tx != encoded_from_rpc {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(
                    "Transaction from RPC doesn't match the input".to_string(),
                ));
            }
            Ok(())
        };
        Box::new(fut.boxed().compat())
    }

    pub(super) async fn validate_payment_for_denom(
        &self,
        input: ValidatePaymentInput,
        denom: Denom,
        decimals: u8,
    ) -> ValidatePaymentResult<()> {
        let tx = cosmrs::Tx::from_bytes(&input.payment_tx)
            .map_to_mm(|e| ValidatePaymentError::TxDeserializationError(e.to_string()))?;

        if tx.body.messages.len() != 1 {
            return MmError::err(ValidatePaymentError::WrongPaymentTx(
                "Payment tx must have exactly one message".into(),
            ));
        }
        let htlc_type = HtlcType::from_str(&self.account_prefix).map_err(|_| {
            ValidatePaymentError::InvalidParameter(format!(
                "Account type '{}' is not supported for HTLCs",
                self.account_prefix
            ))
        })?;

        let create_htlc_msg_proto = CreateHtlcProto::decode(htlc_type, tx.body.messages[0].value.as_slice())
            .map_to_mm(|e| ValidatePaymentError::WrongPaymentTx(e.to_string()))?;
        let create_htlc_msg = CreateHtlcMsg::try_from(create_htlc_msg_proto)
            .map_to_mm(|e| ValidatePaymentError::WrongPaymentTx(e.to_string()))?;

        let sender_pubkey_hash = dhash160(&input.other_pub);
        let sender = AccountId::new(&self.account_prefix, sender_pubkey_hash.as_slice())
            .map_to_mm(|e| ValidatePaymentError::InvalidParameter(e.to_string()))?;

        let amount = sat_from_big_decimal(&input.amount, decimals)?;
        let amount = vec![Coin {
            denom,
            amount: amount.into(),
        }];

        let time_lock = self.estimate_blocks_from_duration(input.time_lock_duration);

        let expected_msg = CreateHtlcMsg::new(
            htlc_type,
            sender.clone(),
            self.account_id.clone(),
            amount.clone(),
            hex::encode(&input.secret_hash),
            0,
            time_lock as u64,
        );

        if create_htlc_msg != expected_msg {
            return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                "Incorrect CreateHtlc message {:?}, expected {:?}",
                create_htlc_msg, expected_msg
            )));
        }

        let hash = hex::encode_upper(sha256(&input.payment_tx).as_slice());
        let tx_from_rpc = self.request_tx(hash).await?;
        if input.payment_tx != tx_from_rpc.encode_to_vec() {
            return MmError::err(ValidatePaymentError::InvalidRpcResponse(
                "Tx from RPC doesn't match the input".into(),
            ));
        }

        let htlc_id = self.calculate_htlc_id(&sender, &self.account_id, &amount, &input.secret_hash);

        let htlc_response = self.query_htlc(htlc_id.clone()).await?;
        let htlc_state = htlc_response
            .htlc_state()
            .or_mm_err(|| ValidatePaymentError::InvalidRpcResponse(format!("No HTLC data for {}", htlc_id)))?;

        match htlc_state {
            HTLC_STATE_OPEN => Ok(()),
            unexpected_state => MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
                "{}",
                unexpected_state
            ))),
        }
    }

    pub(super) async fn get_sender_trade_fee_for_denom(
        &self,
        ticker: String,
        denom: Denom,
        decimals: u8,
        amount: BigDecimal,
    ) -> TradePreimageResult<TradeFee> {
        const TIME_LOCK: u64 = 1750;

        let mut sec = [0u8; 32];
        common::os_rng(&mut sec).map_err(|e| MmError::new(TradePreimageError::InternalError(e.to_string())))?;
        drop_mutability!(sec);

        let to_address = account_id_from_pubkey_hex(&self.account_prefix, DEX_FEE_ADDR_PUBKEY)
            .map_err(|e| MmError::new(TradePreimageError::InternalError(e.to_string())))?;

        let amount = sat_from_big_decimal(&amount, decimals)?;

        let create_htlc_tx = self
            .gen_create_htlc_tx(denom, &to_address, amount.into(), sha256(&sec).as_slice(), TIME_LOCK)
            .map_err(|e| {
                MmError::new(TradePreimageError::InternalError(format!(
                    "Could not create HTLC. {:?}",
                    e.into_inner()
                )))
            })?;

        let current_block = self.current_block().compat().await.map_err(|e| {
            MmError::new(TradePreimageError::InternalError(format!(
                "Could not get current_block. {}",
                e
            )))
        })?;

        let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

        let fee_uamount = self
            .calculate_account_fee_amount_as_u64(
                &self.account_id,
                self.activation_policy.activated_key(),
                create_htlc_tx.msg_payload.clone(),
                timeout_height,
                TX_DEFAULT_MEMO.to_owned(),
                None,
            )
            .await?;

        let fee_amount = big_decimal_from_sat_unsigned(fee_uamount, self.decimals);

        Ok(TradeFee {
            coin: ticker,
            amount: fee_amount.into(),
            paid_from_trading_vol: false,
        })
    }

    pub(super) async fn get_fee_to_send_taker_fee_for_denom(
        &self,
        ticker: String,
        denom: Denom,
        decimals: u8,
        dex_fee_amount: DexFee,
    ) -> TradePreimageResult<TradeFee> {
        let to_address = account_id_from_pubkey_hex(&self.account_prefix, DEX_FEE_ADDR_PUBKEY)
            .map_err(|e| MmError::new(TradePreimageError::InternalError(e.to_string())))?;
        let amount = sat_from_big_decimal(&dex_fee_amount.fee_amount().into(), decimals)?;

        let current_block = self.current_block().compat().await.map_err(|e| {
            MmError::new(TradePreimageError::InternalError(format!(
                "Could not get current_block. {}",
                e
            )))
        })?;

        let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

        let msg_send = MsgSend {
            from_address: self.account_id.clone(),
            to_address: to_address.clone(),
            amount: vec![Coin {
                denom,
                amount: amount.into(),
            }],
        }
        .to_any()
        .map_err(|e| MmError::new(TradePreimageError::InternalError(e.to_string())))?;

        let fee_uamount = self
            .calculate_account_fee_amount_as_u64(
                &self.account_id,
                self.activation_policy.activated_key(),
                msg_send,
                timeout_height,
                TX_DEFAULT_MEMO.to_owned(),
                None,
            )
            .await?;
        let fee_amount = big_decimal_from_sat_unsigned(fee_uamount, decimals);

        Ok(TradeFee {
            coin: ticker,
            amount: fee_amount.into(),
            paid_from_trading_vol: false,
        })
    }

    pub(super) async fn get_balance_as_unsigned_and_decimal(
        &self,
        account_id: &AccountId,
        denom: &Denom,
        decimals: u8,
    ) -> MmResult<(u64, BigDecimal), TendermintCoinRpcError> {
        let denom_ubalance = self.account_balance_for_denom(account_id, denom.to_string()).await?;
        let denom_balance_dec = big_decimal_from_sat_unsigned(denom_ubalance, decimals);

        Ok((denom_ubalance, denom_balance_dec))
    }

    async fn request_tx(&self, hash: String) -> MmResult<Tx, TendermintCoinRpcError> {
        let request = GetTxRequest { hash };
        let response = self
            .rpc_client()
            .await?
            .abci_query(
                Some(ABCI_GET_TX_PATH.to_string()),
                request.encode_to_vec(),
                ABCI_REQUEST_HEIGHT,
                ABCI_REQUEST_PROVE,
            )
            .await?;

        let response = GetTxResponse::decode(response.value.as_slice())?;
        response
            .tx
            .or_mm_err(|| TendermintCoinRpcError::InvalidResponse(format!("Tx {} does not exist", request.hash)))
    }

    /// Returns status code of transaction.
    /// If tx doesn't exists on chain, then returns `None`.
    async fn get_tx_status_code_or_none(
        &self,
        hash: String,
    ) -> MmResult<Option<cosmrs::tendermint::abci::Code>, TendermintCoinRpcError> {
        let request = GetTxRequest { hash };
        let response = self
            .rpc_client()
            .await?
            .abci_query(
                Some(ABCI_GET_TX_PATH.to_string()),
                request.encode_to_vec(),
                ABCI_REQUEST_HEIGHT,
                ABCI_REQUEST_PROVE,
            )
            .await?;

        let tx = GetTxResponse::decode(response.value.as_slice())?;

        if let Some(tx_response) = tx.tx_response {
            // non-zero values are error.
            match tx_response.code {
                TX_SUCCESS_CODE => Ok(Some(cosmrs::tendermint::abci::Code::Ok)),
                err_code => Ok(Some(cosmrs::tendermint::abci::Code::Err(
                    // This will never panic, as `0` code goes the the success variant above.
                    NonZeroU32::new(err_code).unwrap(),
                ))),
            }
        } else {
            Ok(None)
        }
    }

    pub(crate) async fn query_htlc(&self, id: String) -> MmResult<QueryHtlcResponse, TendermintCoinRpcError> {
        let htlc_type =
            HtlcType::from_str(&self.account_prefix).map_err(|_| TendermintCoinRpcError::UnexpectedAccountType {
                prefix: self.account_prefix.clone(),
            })?;

        let request = QueryHtlcRequestProto { id };
        let response = self
            .rpc_client()
            .await?
            .abci_query(
                Some(htlc_type.get_htlc_abci_query_path()),
                request.encode_to_vec(),
                ABCI_REQUEST_HEIGHT,
                ABCI_REQUEST_PROVE,
            )
            .await?;

        Ok(QueryHtlcResponse::decode(htlc_type, response.value.as_slice())?)
    }

    #[inline]
    pub(crate) fn is_tx_amount_enough(&self, decimals: u8, amount: &BigDecimal) -> bool {
        let min_tx_amount = big_decimal_from_sat(MIN_TX_SATOSHIS, decimals);
        amount >= &min_tx_amount
    }

    async fn search_for_swap_tx_spend(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> MmResult<Option<FoundSwapTxSpend>, SearchForSwapTxSpendErr> {
        let tx = cosmrs::Tx::from_bytes(input.tx)?;
        let first_message = tx
            .body
            .messages
            .first()
            .or_mm_err(|| SearchForSwapTxSpendErr::TxMessagesEmpty)?;

        let htlc_type =
            HtlcType::from_str(&self.account_prefix).map_err(|_| SearchForSwapTxSpendErr::UnexpectedAccountType {
                prefix: self.account_prefix.clone(),
            })?;

        let htlc_proto = CreateHtlcProto::decode(htlc_type, first_message.value.as_slice())?;
        let htlc = CreateHtlcMsg::try_from(htlc_proto)?;
        let htlc_id = self.calculate_htlc_id(htlc.sender(), htlc.to(), htlc.amount(), input.secret_hash);

        let htlc_response = self.query_htlc(htlc_id.clone()).await?;

        let htlc_state = match htlc_response.htlc_state() {
            Some(htlc_state) => htlc_state,
            None => return Ok(None),
        };

        match htlc_state {
            HTLC_STATE_OPEN => Ok(None),
            HTLC_STATE_COMPLETED => {
                let events_string = format!("claim_htlc.id='{}'", htlc_id);
                // TODO: Remove deprecated attribute when new version of tendermint-rs is released
                #[allow(deprecated)]
                let request = GetTxsEventRequest {
                    events: vec![events_string],
                    order_by: TendermintResultOrder::Ascending as i32,
                    page: 1,
                    limit: 1,
                    pagination: None,
                };
                let encoded_request = request.encode_to_vec();

                let response = self
                    .rpc_client()
                    .await?
                    .abci_query(
                        Some(ABCI_GET_TXS_EVENT_PATH.to_string()),
                        encoded_request.as_slice(),
                        ABCI_REQUEST_HEIGHT,
                        ABCI_REQUEST_PROVE,
                    )
                    .await
                    .map_to_mm(TendermintCoinRpcError::from)?;
                let response = GetTxsEventResponse::decode(response.value.as_slice())?;
                match response.txs.first() {
                    Some(tx) => {
                        let tx = TransactionEnum::CosmosTransaction(CosmosTransaction {
                            data: TxRaw {
                                body_bytes: tx.body.as_ref().map(Message::encode_to_vec).unwrap_or_default(),
                                auth_info_bytes: tx.auth_info.as_ref().map(Message::encode_to_vec).unwrap_or_default(),
                                signatures: tx.signatures.clone(),
                            },
                        });
                        Ok(Some(FoundSwapTxSpend::Spent(tx)))
                    },
                    None => MmError::err(SearchForSwapTxSpendErr::ClaimHtlcTxNotFound),
                }
            },
            HTLC_STATE_REFUNDED => {
                // HTLC is refunded automatically without transaction. We have to return dummy tx data
                Ok(Some(FoundSwapTxSpend::Refunded(TransactionEnum::CosmosTransaction(
                    CosmosTransaction { data: TxRaw::default() },
                ))))
            },
            unexpected_state => MmError::err(SearchForSwapTxSpendErr::UnexpectedHtlcState(unexpected_state)),
        }
    }

    pub(crate) fn gas_info_for_withdraw(
        &self,
        withdraw_fee: &Option<WithdrawFee>,
        fallback_gas_limit: u64,
    ) -> (f64, u64) {
        match withdraw_fee {
            Some(WithdrawFee::CosmosGas { gas_price, gas_limit }) => (*gas_price, *gas_limit),
            _ => (self.gas_price(), fallback_gas_limit),
        }
    }

    pub(crate) fn active_ticker_and_decimals_from_denom(&self, denom: &str) -> Option<(String, u8)> {
        if self.denom.as_ref() == denom {
            return Some((self.ticker.clone(), self.decimals));
        }

        let tokens = self.tokens_info.lock();

        if let Some(token_info) = tokens.get(denom) {
            return Some((token_info.ticker.to_owned(), token_info.decimals));
        }

        None
    }

    pub fn is_ledger_connection(&self) -> bool {
        matches!(
            self.wallet_type,
            TendermintWalletConnectionType::WcLedger(_) | TendermintWalletConnectionType::KeplrLedger
        )
    }

    pub fn is_wallet_connect(&self) -> bool {
        matches!(
            self.wallet_type,
            TendermintWalletConnectionType::WcLedger(_) | TendermintWalletConnectionType::Wc(_)
        )
    }

    pub fn try_wallet_connect_session(&self) -> Option<&str> {
        match self.wallet_type {
            TendermintWalletConnectionType::WcLedger(ref session_topic)
            | TendermintWalletConnectionType::Wc(ref session_topic) => Some(session_topic),
            _ => None,
        }
    }

    pub(crate) async fn validators_list(
        &self,
        filter_status: ValidatorStatus,
        paging: PagingOptions,
    ) -> MmResult<Vec<Validator>, TendermintCoinRpcError> {
        let request = QueryValidatorsRequest {
            status: filter_status.to_string(),
            pagination: Some(PageRequest {
                key: vec![],
                offset: ((paging.page_number.get() - 1usize) * paging.limit) as u64,
                limit: paging.limit as u64,
                count_total: false,
                reverse: false,
            }),
        };

        let raw_response = self
            .rpc_client()
            .await?
            .abci_query(
                Some(ABCI_VALIDATORS_PATH.to_owned()),
                request.encode_to_vec(),
                ABCI_REQUEST_HEIGHT,
                ABCI_REQUEST_PROVE,
            )
            .await?;

        let decoded_proto = QueryValidatorsResponseProto::decode(raw_response.value.as_slice())?;
        let typed_response = QueryValidatorsResponse::try_from(decoded_proto)
            .map_err(|e| TendermintCoinRpcError::InternalError(e.to_string()))?;

        Ok(typed_response.validators)
    }
}

fn clients_from_urls(ctx: &MmArc, nodes: Vec<RpcNode>) -> MmResult<Vec<HttpClient>, TendermintInitErrorKind> {
    if nodes.is_empty() {
        return MmError::err(TendermintInitErrorKind::EmptyRpcUrls);
    }

    let p2p_keypair = if nodes.iter().any(|n| n.komodo_proxy) {
        let p2p_ctx = P2PContext::fetch_from_mm_arc(ctx);
        Some(p2p_ctx.keypair().clone())
    } else {
        None
    };

    let mut clients = Vec::new();
    let mut errors = Vec::new();

    // check that all urls are valid
    // keep all invalid urls in one vector to show all of them in error
    for node in nodes.iter() {
        let proxy_sign_keypair = if node.komodo_proxy { p2p_keypair.clone() } else { None };
        match HttpClient::new(node.url.as_str(), proxy_sign_keypair) {
            Ok(client) => clients.push(client),
            Err(e) => errors.push(format!("Url {} is invalid, got error {}", node.url, e)),
        }
    }
    drop_mutability!(clients);
    drop_mutability!(errors);
    if !errors.is_empty() {
        let errors: String = errors.into_iter().join(", ");
        return MmError::err(TendermintInitErrorKind::RpcClientInitError(errors));
    }
    Ok(clients)
}

pub async fn get_ibc_chain_list() -> IBCChainRegistriesResult {
    fn map_metadata_to_chain_registry_name(metadata: &FileMetadata) -> Result<String, MmError<IBCChainsRequestError>> {
        let split_filename_by_dash: Vec<&str> = metadata.name.split('-').collect();
        let chain_registry_name = split_filename_by_dash
            .first()
            .or_mm_err(|| {
                IBCChainsRequestError::InternalError(format!(
                    "Could not read chain registry name from '{}'",
                    metadata.name
                ))
            })?
            .to_string();

        Ok(chain_registry_name)
    }

    let git_controller: GitController<GithubClient> = GitController::new(GITHUB_API_URI);

    let metadata_list = git_controller
        .client
        .get_file_metadata_list(
            CHAIN_REGISTRY_REPO_OWNER,
            CHAIN_REGISTRY_REPO_NAME,
            CHAIN_REGISTRY_BRANCH,
            CHAIN_REGISTRY_IBC_DIR_NAME,
        )
        .await
        .map_err(|e| IBCChainsRequestError::Transport(format!("{:?}", e)))?;

    let chain_list: Result<Vec<String>, MmError<IBCChainsRequestError>> =
        metadata_list.iter().map(map_metadata_to_chain_registry_name).collect();

    let mut distinct_chain_list = chain_list?;
    distinct_chain_list.dedup();

    Ok(IBCChainRegistriesResponse {
        chain_registry_list: distinct_chain_list,
    })
}

#[async_trait]
#[allow(unused_variables)]
impl MmCoin for TendermintCoin {
    fn is_asset_chain(&self) -> bool { false }

    fn wallet_only(&self, ctx: &MmArc) -> bool {
        let coin_conf = crate::coin_conf(ctx, self.ticker());
        // If coin is not in config, it means that it was added manually (a custom token) and should be treated as wallet only
        if coin_conf.is_null() {
            return true;
        }
        let wallet_only_conf = coin_conf["wallet_only"].as_bool().unwrap_or(false);

        wallet_only_conf || self.is_ledger_connection()
    }

    fn spawner(&self) -> CoinFutSpawner { CoinFutSpawner::new(&self.abortable_system) }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        let coin = self.clone();
        let fut = async move {
            let to_address =
                AccountId::from_str(&req.to).map_to_mm(|e| WithdrawError::InvalidAddress(e.to_string()))?;

            let is_ibc_transfer = to_address.prefix() != coin.account_prefix || req.ibc_source_channel.is_some();

            let (account_id, maybe_pk) = coin.account_id_and_pk_for_withdraw(req.from)?;

            let (balance_denom, balance_dec) = coin
                .get_balance_as_unsigned_and_decimal(&account_id, &coin.denom, coin.decimals())
                .await?;

            let (amount_denom, amount_dec) = if req.max {
                let amount_denom = balance_denom;
                (amount_denom, big_decimal_from_sat_unsigned(amount_denom, coin.decimals))
            } else {
                (sat_from_big_decimal(&req.amount, coin.decimals)?, req.amount.clone())
            };

            if !coin.is_tx_amount_enough(coin.decimals, &amount_dec) {
                return MmError::err(WithdrawError::AmountTooLow {
                    amount: amount_dec,
                    threshold: coin.min_tx_amount(),
                });
            }

            let received_by_me = if to_address == account_id {
                amount_dec
            } else {
                BigDecimal::default()
            };

            let channel_id = if is_ibc_transfer {
                match &req.ibc_source_channel {
                    Some(_) => req.ibc_source_channel,
                    None => Some(coin.detect_channel_id_for_ibc_transfer(&to_address).await?),
                }
            } else {
                None
            };

            let msg_payload = create_withdraw_msg_as_any(
                account_id.clone(),
                to_address.clone(),
                &coin.denom,
                amount_denom,
                channel_id.clone(),
            )
            .await?;

            let memo = req.memo.unwrap_or_else(|| TX_DEFAULT_MEMO.into());

            let current_block = coin
                .current_block()
                .compat()
                .await
                .map_to_mm(WithdrawError::Transport)?;

            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

            let (_, gas_limit) = if is_ibc_transfer {
                coin.gas_info_for_withdraw(&req.fee, IBC_GAS_LIMIT_DEFAULT)
            } else {
                coin.gas_info_for_withdraw(&req.fee, GAS_LIMIT_DEFAULT)
            };

            let fee_amount_u64 = coin
                .calculate_account_fee_amount_as_u64(
                    &account_id,
                    maybe_pk,
                    msg_payload.clone(),
                    timeout_height,
                    memo.clone(),
                    req.fee,
                )
                .await?;

            let fee_amount_u64 = if coin.is_ledger_connection() {
                // When using `SIGN_MODE_LEGACY_AMINO_JSON`, Keplr ignores the fee we calculated
                // and calculates another one which is usually double what we calculate.
                // To make sure the transaction doesn't fail on the Keplr side (because if Keplr
                // calculates a higher fee than us, the withdrawal might fail), we use three times
                // the actual fee.
                fee_amount_u64 * 3
            } else {
                fee_amount_u64
            };

            let fee_amount_dec = big_decimal_from_sat_unsigned(fee_amount_u64, coin.decimals());

            let fee_amount = Coin {
                denom: coin.denom.clone(),
                amount: fee_amount_u64.into(),
            };

            let fee = Fee::from_amount_and_gas(fee_amount, gas_limit);

            let (amount_denom, total_amount) = if req.max {
                if balance_denom < fee_amount_u64 {
                    return MmError::err(WithdrawError::NotSufficientBalance {
                        coin: coin.ticker.clone(),
                        available: balance_dec,
                        required: fee_amount_dec,
                    });
                }
                let amount_denom = balance_denom - fee_amount_u64;
                (amount_denom, balance_dec)
            } else {
                let total = &req.amount + &fee_amount_dec;
                if balance_dec < total {
                    return MmError::err(WithdrawError::NotSufficientBalance {
                        coin: coin.ticker.clone(),
                        available: balance_dec,
                        required: total,
                    });
                }

                (sat_from_big_decimal(&req.amount, coin.decimals)?, total)
            };

            let msg_payload = create_withdraw_msg_as_any(
                account_id.clone(),
                to_address.clone(),
                &coin.denom,
                amount_denom,
                channel_id,
            )
            .await?;

            let account_info = coin.account_info(&account_id).await?;

            let tx = coin
                .any_to_transaction_data(maybe_pk, msg_payload, &account_info, fee, timeout_height, memo.clone())
                .await
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let internal_id = {
                let hex_vec = tx.tx_hex().cloned().unwrap_or_default().to_vec();
                sha256(&hex_vec).to_vec().into()
            };

            Ok(TransactionDetails {
                tx,
                from: vec![account_id.to_string()],
                to: vec![req.to],
                my_balance_change: &received_by_me - &total_amount,
                spent_by_me: total_amount.clone(),
                total_amount,
                received_by_me,
                block_height: 0,
                timestamp: 0,
                fee_details: Some(TxFeeDetails::Tendermint(TendermintFeeDetails {
                    coin: coin.ticker.clone(),
                    amount: fee_amount_dec,
                    uamount: fee_amount_u64,
                    gas_limit,
                })),
                coin: coin.ticker.to_string(),
                internal_id,
                kmd_rewards: None,
                transaction_type: if is_ibc_transfer {
                    TransactionType::TendermintIBCTransfer
                } else {
                    TransactionType::StandardTransfer
                },
                memo: Some(memo),
            })
        };
        Box::new(fut.boxed().compat())
    }

    fn get_raw_transaction(&self, mut req: RawTransactionRequest) -> RawTransactionFut {
        let coin = self.clone();
        let fut = async move {
            req.tx_hash.make_ascii_uppercase();
            let tx_from_rpc = coin.request_tx(req.tx_hash).await?;
            Ok(RawTransactionRes {
                tx_hex: tx_from_rpc.encode_to_vec().into(),
            })
        };
        Box::new(fut.boxed().compat())
    }

    fn get_tx_hex_by_hash(&self, tx_hash: Vec<u8>) -> RawTransactionFut {
        let coin = self.clone();
        let hash = hex::encode_upper(H256::from(tx_hash.as_slice()));
        let fut = async move {
            let tx_from_rpc = coin.request_tx(hash).await?;
            Ok(RawTransactionRes {
                tx_hex: tx_from_rpc.encode_to_vec().into(),
            })
        };
        Box::new(fut.boxed().compat())
    }

    fn decimals(&self) -> u8 { self.decimals }

    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> {
        // TODO
        Err("Not implemented".into())
    }

    fn validate_address(&self, address: &str) -> ValidateAddressResult {
        match AccountId::from_str(address) {
            Ok(_) => ValidateAddressResult {
                is_valid: true,
                reason: None,
            },
            Err(e) => ValidateAddressResult {
                is_valid: false,
                reason: Some(e.to_string()),
            },
        }
    }

    fn process_history_loop(&self, ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> {
        warn!("process_history_loop is deprecated, tendermint uses tx_history_v2");
        Box::new(futures01::future::err(()))
    }

    fn history_sync_status(&self) -> HistorySyncState { self.history_sync_state.lock().unwrap().clone() }

    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> {
        Box::new(futures01::future::err("Not implemented".into()))
    }

    async fn get_sender_trade_fee(
        &self,
        value: TradePreimageValue,
        _stage: FeeApproxStage,
        _include_refund_fee: bool,
    ) -> TradePreimageResult<TradeFee> {
        let amount = match value {
            TradePreimageValue::Exact(decimal) | TradePreimageValue::UpperBound(decimal) => decimal,
        };
        self.get_sender_trade_fee_for_denom(self.ticker.clone(), self.denom.clone(), self.decimals, amount)
            .await
    }

    fn get_receiver_trade_fee(&self, stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        let coin = self.clone();
        let fut = async move {
            // We can't simulate Claim Htlc without having information about broadcasted htlc tx.
            // Since create and claim htlc fees are almost same, we can simply simulate create htlc tx.
            coin.get_sender_trade_fee_for_denom(
                coin.ticker.clone(),
                coin.denom.clone(),
                coin.decimals,
                coin.min_tx_amount(),
            )
            .await
        };
        Box::new(fut.boxed().compat())
    }

    async fn get_fee_to_send_taker_fee(
        &self,
        dex_fee_amount: DexFee,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        self.get_fee_to_send_taker_fee_for_denom(self.ticker.clone(), self.denom.clone(), self.decimals, dex_fee_amount)
            .await
    }

    fn required_confirmations(&self) -> u64 { 0 }

    fn requires_notarization(&self) -> bool { false }

    fn set_required_confirmations(&self, confirmations: u64) {
        warn!("set_required_confirmations is not supported for tendermint")
    }

    fn set_requires_notarization(&self, requires_nota: bool) { warn!("TendermintCoin doesn't support notarization") }

    fn swap_contract_address(&self) -> Option<BytesJson> { None }

    fn fallback_swap_contract(&self) -> Option<BytesJson> { None }

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

    fn on_token_deactivated(&self, _ticker: &str) {}
}

#[async_trait]
impl MarketCoinOps for TendermintCoin {
    fn ticker(&self) -> &str { &self.ticker }

    fn my_address(&self) -> MmResult<String, MyAddressError> { Ok(self.account_id.to_string()) }

    async fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> {
        let key = SigningKey::from_slice(self.activation_policy.activated_key_or_err()?.as_slice())
            .expect("privkey validity is checked on coin creation");
        Ok(key.public_key().to_string())
    }

    fn sign_message_hash(&self, _message: &str) -> Option<[u8; 32]> {
        // TODO
        None
    }

    fn sign_message(&self, _message: &str) -> SignatureResult<String> {
        // TODO
        MmError::err(SignatureError::InternalError("Not implemented".into()))
    }

    fn verify_message(&self, _signature: &str, _message: &str, _address: &str) -> VerificationResult<bool> {
        // TODO
        MmError::err(VerificationError::InternalError("Not implemented".into()))
    }

    fn my_balance(&self) -> BalanceFut<CoinBalance> {
        let coin = self.clone();
        let fut = async move {
            let balance_denom = coin
                .account_balance_for_denom(&coin.account_id, coin.denom.to_string())
                .await?;
            Ok(CoinBalance {
                spendable: big_decimal_from_sat_unsigned(balance_denom, coin.decimals),
                unspendable: BigDecimal::default(),
            })
        };
        Box::new(fut.boxed().compat())
    }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> {
        Box::new(self.my_balance().map(|coin_balance| coin_balance.spendable))
    }

    fn platform_ticker(&self) -> &str { &self.ticker }

    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> {
        let tx_bytes = try_fus!(hex::decode(tx));
        self.send_raw_tx_bytes(&tx_bytes)
    }

    /// Consider using `seq_safe_raw_tx_bytes` instead.
    /// This is considered as unsafe due to sequence mismatches.
    fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> {
        // as sanity check
        try_fus!(Raw::from_bytes(tx));

        let coin = self.clone();
        let tx_bytes = tx.to_owned();
        let fut = async move {
            let broadcast_res = try_s!(try_s!(coin.rpc_client().await).broadcast_tx_commit(tx_bytes).await);

            if broadcast_res.check_tx.log.contains(ACCOUNT_SEQUENCE_ERR)
                || broadcast_res.tx_result.log.contains(ACCOUNT_SEQUENCE_ERR)
            {
                return ERR!(
                    "{}. check_tx log: {}, deliver_tx log: {}",
                    ACCOUNT_SEQUENCE_ERR,
                    broadcast_res.check_tx.log,
                    broadcast_res.tx_result.log
                );
            }

            if !broadcast_res.check_tx.code.is_ok() {
                return ERR!("Tx check failed {:?}", broadcast_res.check_tx);
            }

            if !broadcast_res.tx_result.code.is_ok() {
                return ERR!("Tx deliver failed {:?}", broadcast_res.tx_result);
            }
            Ok(broadcast_res.hash.to_string())
        };
        Box::new(fut.boxed().compat())
    }

    #[inline(always)]
    async fn sign_raw_tx(&self, _args: &SignRawTransactionRequest) -> RawTransactionResult {
        MmError::err(RawTransactionError::NotImplemented {
            coin: self.ticker().to_string(),
        })
    }

    fn wait_for_confirmations(&self, input: ConfirmPaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        // Sanity check
        let _: TxRaw = try_fus!(Message::decode(input.payment_tx.as_slice()));

        let tx_hash = hex::encode_upper(sha256(&input.payment_tx));

        let coin = self.clone();
        let fut = async move {
            loop {
                if now_sec() > input.wait_until {
                    return ERR!(
                        "Waited too long until {} for payment {} to be received",
                        input.wait_until,
                        tx_hash.clone()
                    );
                }

                let tx_status_code = try_s!(coin.get_tx_status_code_or_none(tx_hash.clone()).await);

                if let Some(tx_status_code) = tx_status_code {
                    return match tx_status_code {
                        cosmrs::tendermint::abci::Code::Ok => Ok(()),
                        cosmrs::tendermint::abci::Code::Err(err_code) => Err(format!(
                            "Got error code: '{}' for tx: '{}'. Broadcasted tx isn't valid.",
                            err_code, tx_hash
                        )),
                    };
                };

                Timer::sleep(input.check_every as f64).await;
            }
        };

        Box::new(fut.boxed().compat())
    }

    async fn wait_for_htlc_tx_spend(&self, args: WaitForHTLCTxSpendArgs<'_>) -> TransactionResult {
        let tx = try_tx_s!(cosmrs::Tx::from_bytes(args.tx_bytes));
        let first_message = try_tx_s!(tx.body.messages.first().ok_or("Tx body couldn't be read."));
        let htlc_proto = try_tx_s!(CreateHtlcProto::decode(
            try_tx_s!(HtlcType::from_str(&self.account_prefix)),
            first_message.value.as_slice()
        ));
        let htlc = try_tx_s!(CreateHtlcMsg::try_from(htlc_proto));
        let htlc_id = self.calculate_htlc_id(htlc.sender(), htlc.to(), htlc.amount(), args.secret_hash);

        let events_string = format!("claim_htlc.id='{}'", htlc_id);
        // TODO: Remove deprecated attribute when new version of tendermint-rs is released
        #[allow(deprecated)]
        let request = GetTxsEventRequest {
            events: vec![events_string],
            order_by: TendermintResultOrder::Ascending as i32,
            page: 1,
            limit: 1,
            pagination: None,
        };
        let encoded_request = request.encode_to_vec();

        loop {
            let response = try_tx_s!(
                try_tx_s!(self.rpc_client().await)
                    .abci_query(
                        Some(ABCI_GET_TXS_EVENT_PATH.to_string()),
                        encoded_request.as_slice(),
                        ABCI_REQUEST_HEIGHT,
                        ABCI_REQUEST_PROVE
                    )
                    .await
            );
            let response = try_tx_s!(GetTxsEventResponse::decode(response.value.as_slice()));
            if let Some(tx) = response.txs.first() {
                return Ok(TransactionEnum::CosmosTransaction(CosmosTransaction {
                    data: TxRaw {
                        body_bytes: tx.body.as_ref().map(Message::encode_to_vec).unwrap_or_default(),
                        auth_info_bytes: tx.auth_info.as_ref().map(Message::encode_to_vec).unwrap_or_default(),
                        signatures: tx.signatures.clone(),
                    },
                }));
            }
            Timer::sleep(5.).await;
            if get_utc_timestamp() > args.wait_until as i64 {
                return Err(TransactionErr::Plain("Waited too long".into()));
            }
        }
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        let tx_raw: TxRaw = Message::decode(bytes).map_to_mm(|e| TxMarshalingErr::InvalidInput(e.to_string()))?;
        Ok(TransactionEnum::CosmosTransaction(CosmosTransaction { data: tx_raw }))
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        let coin = self.clone();
        let fut = async move {
            let info = try_s!(try_s!(coin.rpc_client().await).abci_info().await);
            Ok(info.response.last_block_height.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn display_priv_key(&self) -> Result<String, String> {
        Ok(self
            .activation_policy
            .activated_key_or_err()
            .map_err(|e| e.to_string())?
            .to_string())
    }

    #[inline]
    fn min_tx_amount(&self) -> BigDecimal { big_decimal_from_sat(MIN_TX_SATOSHIS, self.decimals) }

    #[inline]
    fn min_trading_vol(&self) -> MmNumber { self.min_tx_amount().into() }

    fn is_trezor(&self) -> bool {
        match &self.activation_policy {
            TendermintActivationPolicy::PrivateKey(pk) => pk.is_trezor(),
            TendermintActivationPolicy::PublicKey(_) => false,
        }
    }
}

#[async_trait]
#[allow(unused_variables)]
impl SwapOps for TendermintCoin {
    async fn send_taker_fee(&self, fee_addr: &[u8], dex_fee: DexFee, uuid: &[u8], expire_at: u64) -> TransactionResult {
        self.send_taker_fee_for_denom(
            fee_addr,
            dex_fee.fee_amount().into(),
            self.denom.clone(),
            self.decimals,
            uuid,
            expire_at,
        )
        .compat()
        .await
    }

    async fn send_maker_payment(&self, maker_payment_args: SendPaymentArgs<'_>) -> TransactionResult {
        self.send_htlc_for_denom(
            maker_payment_args.time_lock_duration,
            maker_payment_args.other_pubkey,
            maker_payment_args.secret_hash,
            maker_payment_args.amount,
            self.denom.clone(),
            self.decimals,
        )
        .compat()
        .await
    }

    async fn send_taker_payment(&self, taker_payment_args: SendPaymentArgs<'_>) -> TransactionResult {
        self.send_htlc_for_denom(
            taker_payment_args.time_lock_duration,
            taker_payment_args.other_pubkey,
            taker_payment_args.secret_hash,
            taker_payment_args.amount,
            self.denom.clone(),
            self.decimals,
        )
        .compat()
        .await
    }

    async fn send_maker_spends_taker_payment(
        &self,
        maker_spends_payment_args: SpendPaymentArgs<'_>,
    ) -> TransactionResult {
        let tx = try_tx_s!(cosmrs::Tx::from_bytes(maker_spends_payment_args.other_payment_tx));
        let msg = try_tx_s!(tx.body.messages.first().ok_or("Tx body couldn't be read."));

        let htlc_proto = try_tx_s!(CreateHtlcProto::decode(
            try_tx_s!(HtlcType::from_str(&self.account_prefix)),
            msg.value.as_slice()
        ));
        let htlc = try_tx_s!(CreateHtlcMsg::try_from(htlc_proto));

        let mut amount = htlc.amount().to_vec();
        amount.sort();
        drop_mutability!(amount);

        let coins_string = amount
            .iter()
            .map(|t| format!("{}{}", t.amount, t.denom))
            .collect::<Vec<String>>()
            .join(",");

        let htlc_id = self.calculate_htlc_id(htlc.sender(), htlc.to(), &amount, maker_spends_payment_args.secret_hash);

        let claim_htlc_tx = try_tx_s!(self.gen_claim_htlc_tx(htlc_id, maker_spends_payment_args.secret));
        let timeout = maker_spends_payment_args
            .time_lock
            .checked_sub(now_sec())
            .unwrap_or_default();
        let coin = self.clone();

        let current_block = try_tx_s!(self.current_block().compat().await);
        let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

        let fee = try_tx_s!(
            self.calculate_fee(
                claim_htlc_tx.msg_payload.clone(),
                timeout_height,
                TX_DEFAULT_MEMO.to_owned(),
                None
            )
            .await
        );

        let (_tx_id, tx_raw) = try_tx_s!(
            coin.common_send_raw_tx_bytes(
                claim_htlc_tx.msg_payload.clone(),
                fee.clone(),
                timeout_height,
                TX_DEFAULT_MEMO.into(),
                Duration::from_secs(timeout),
            )
            .await
        );

        Ok(TransactionEnum::CosmosTransaction(CosmosTransaction {
            data: tx_raw.into(),
        }))
    }

    async fn send_taker_spends_maker_payment(
        &self,
        taker_spends_payment_args: SpendPaymentArgs<'_>,
    ) -> TransactionResult {
        let tx = try_tx_s!(cosmrs::Tx::from_bytes(taker_spends_payment_args.other_payment_tx));
        let msg = try_tx_s!(tx.body.messages.first().ok_or("Tx body couldn't be read."));

        let htlc_proto = try_tx_s!(CreateHtlcProto::decode(
            try_tx_s!(HtlcType::from_str(&self.account_prefix)),
            msg.value.as_slice()
        ));
        let htlc = try_tx_s!(CreateHtlcMsg::try_from(htlc_proto));

        let mut amount = htlc.amount().to_vec();
        amount.sort();
        drop_mutability!(amount);

        let coins_string = amount
            .iter()
            .map(|t| format!("{}{}", t.amount, t.denom))
            .collect::<Vec<String>>()
            .join(",");

        let htlc_id = self.calculate_htlc_id(htlc.sender(), htlc.to(), &amount, taker_spends_payment_args.secret_hash);

        let timeout = taker_spends_payment_args
            .time_lock
            .checked_sub(now_sec())
            .unwrap_or_default();
        let claim_htlc_tx = try_tx_s!(self.gen_claim_htlc_tx(htlc_id, taker_spends_payment_args.secret));
        let coin = self.clone();

        let current_block = try_tx_s!(self.current_block().compat().await);
        let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

        let fee = try_tx_s!(
            self.calculate_fee(
                claim_htlc_tx.msg_payload.clone(),
                timeout_height,
                TX_DEFAULT_MEMO.into(),
                None
            )
            .await
        );

        let (tx_id, tx_raw) = try_tx_s!(
            coin.common_send_raw_tx_bytes(
                claim_htlc_tx.msg_payload.clone(),
                fee.clone(),
                timeout_height,
                TX_DEFAULT_MEMO.into(),
                Duration::from_secs(timeout),
            )
            .await
        );

        Ok(TransactionEnum::CosmosTransaction(CosmosTransaction {
            data: tx_raw.into(),
        }))
    }

    async fn send_taker_refunds_payment(&self, taker_refunds_payment_args: RefundPaymentArgs<'_>) -> TransactionResult {
        Err(TransactionErr::Plain(
            "Doesn't need transaction broadcast to refund IRIS HTLC".into(),
        ))
    }

    async fn send_maker_refunds_payment(&self, maker_refunds_payment_args: RefundPaymentArgs<'_>) -> TransactionResult {
        Err(TransactionErr::Plain(
            "Doesn't need transaction broadcast to refund IRIS HTLC".into(),
        ))
    }

    async fn validate_fee(&self, validate_fee_args: ValidateFeeArgs<'_>) -> ValidatePaymentResult<()> {
        self.validate_fee_for_denom(
            validate_fee_args.fee_tx,
            validate_fee_args.expected_sender,
            validate_fee_args.fee_addr,
            &validate_fee_args.dex_fee.fee_amount().into(),
            self.decimals,
            validate_fee_args.uuid,
            self.denom.to_string(),
        )
        .compat()
        .await
    }

    async fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentResult<()> {
        self.validate_payment_for_denom(input, self.denom.clone(), self.decimals)
            .await
    }

    async fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentResult<()> {
        self.validate_payment_for_denom(input, self.denom.clone(), self.decimals)
            .await
    }

    async fn check_if_my_payment_sent(
        &self,
        if_my_payment_sent_args: CheckIfMyPaymentSentArgs<'_>,
    ) -> Result<Option<TransactionEnum>, String> {
        self.check_if_my_payment_sent_for_denom(
            self.decimals,
            self.denom.clone(),
            if_my_payment_sent_args.other_pub,
            if_my_payment_sent_args.secret_hash,
            if_my_payment_sent_args.amount,
        )
        .compat()
        .await
    }

    async fn search_for_swap_tx_spend_my(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        self.search_for_swap_tx_spend(input).await.map_err(|e| e.to_string())
    }

    async fn search_for_swap_tx_spend_other(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        self.search_for_swap_tx_spend(input).await.map_err(|e| e.to_string())
    }

    async fn extract_secret(
        &self,
        secret_hash: &[u8],
        spend_tx: &[u8],
        watcher_reward: bool,
    ) -> Result<Vec<u8>, String> {
        let tx = try_s!(cosmrs::Tx::from_bytes(spend_tx));
        let msg = try_s!(tx.body.messages.first().ok_or("Tx body couldn't be read."));

        let htlc_proto = try_s!(ClaimHtlcProto::decode(
            try_s!(HtlcType::from_str(&self.account_prefix)),
            msg.value.as_slice()
        ));
        let htlc = try_s!(ClaimHtlcMsg::try_from(htlc_proto));

        Ok(try_s!(hex::decode(htlc.secret())))
    }

    fn check_tx_signed_by_pub(&self, tx: &[u8], expected_pub: &[u8]) -> Result<bool, MmError<ValidatePaymentError>> {
        unimplemented!();
    }

    // Todo
    fn is_auto_refundable(&self) -> bool { false }

    // Todo
    async fn wait_for_htlc_refund(&self, _tx: &[u8], _locktime: u64) -> RefundResult<()> {
        MmError::err(RefundError::Internal(
            "wait_for_htlc_refund is not supported for this coin!".into(),
        ))
    }

    fn negotiate_swap_contract_addr(
        &self,
        other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        Ok(None)
    }

    #[inline]
    fn derive_htlc_key_pair(&self, _swap_unique_data: &[u8]) -> KeyPair {
        key_pair_from_secret(
            self.activation_policy
                .activated_key_or_err()
                .expect("valid priv key")
                .as_ref(),
        )
        .expect("valid priv key")
    }

    #[inline]
    fn derive_htlc_pubkey(&self, _swap_unique_data: &[u8]) -> Vec<u8> {
        self.activation_policy.public_key().expect("valid pubkey").to_bytes()
    }

    fn validate_other_pubkey(&self, raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> {
        PublicKey::from_raw_secp256k1(raw_pubkey)
            .or_mm_err(|| ValidateOtherPubKeyErr::InvalidPubKey(hex::encode(raw_pubkey)))?;
        Ok(())
    }

    async fn maker_payment_instructions(
        &self,
        args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        Ok(None)
    }

    async fn taker_payment_instructions(
        &self,
        args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        Ok(None)
    }

    fn validate_maker_payment_instructions(
        &self,
        _instructions: &[u8],
        args: PaymentInstructionArgs,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        MmError::err(ValidateInstructionsErr::UnsupportedCoin(self.ticker().to_string()))
    }

    fn validate_taker_payment_instructions(
        &self,
        _instructions: &[u8],
        args: PaymentInstructionArgs,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        MmError::err(ValidateInstructionsErr::UnsupportedCoin(self.ticker().to_string()))
    }
}

#[async_trait]
impl TakerSwapMakerCoin for TendermintCoin {
    async fn on_taker_payment_refund_start(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_taker_payment_refund_success(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
impl MakerSwapTakerCoin for TendermintCoin {
    async fn on_maker_payment_refund_start(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_maker_payment_refund_success(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
impl WatcherOps for TendermintCoin {
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

    fn watcher_validate_taker_fee(&self, _input: WatcherValidateTakerFeeInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    fn watcher_validate_taker_payment(&self, _input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    fn taker_validates_payment_spend_or_refund(&self, _input: ValidateWatcherSpendInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    async fn watcher_search_for_swap_tx_spend(
        &self,
        _input: WatcherSearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!();
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

/// Processes the given `priv_key_build_policy` and returns corresponding `TendermintPrivKeyPolicy`.
/// This function expects either [`PrivKeyBuildPolicy::IguanaPrivKey`]
/// or [`PrivKeyBuildPolicy::GlobalHDAccount`], otherwise returns `PrivKeyPolicyNotAllowed` error.
pub fn tendermint_priv_key_policy(
    conf: &TendermintConf,
    ticker: &str,
    priv_key_build_policy: PrivKeyBuildPolicy,
    path_to_address: HDPathAccountToAddressId,
) -> MmResult<TendermintPrivKeyPolicy, TendermintInitError> {
    match priv_key_build_policy {
        PrivKeyBuildPolicy::IguanaPrivKey(iguana) => {
            let mm2_internal_key_pair = key_pair_from_secret(iguana.as_ref()).mm_err(|e| TendermintInitError {
                ticker: ticker.to_string(),
                kind: TendermintInitErrorKind::Internal(e.to_string()),
            })?;

            let tendermint_pair = TendermintKeyPair::new(iguana, *mm2_internal_key_pair.public());

            Ok(TendermintPrivKeyPolicy::Iguana(tendermint_pair))
        },
        PrivKeyBuildPolicy::GlobalHDAccount(global_hd) => {
            let path_to_coin = conf.derivation_path.as_ref().or_mm_err(|| TendermintInitError {
                ticker: ticker.to_string(),
                kind: TendermintInitErrorKind::DerivationPathIsNotSet,
            })?;
            let activated_priv_key = global_hd
                .derive_secp256k1_secret(&path_to_address.to_derivation_path(path_to_coin).mm_err(|e| {
                    TendermintInitError {
                        ticker: ticker.to_string(),
                        kind: TendermintInitErrorKind::InvalidPathToAddress(e.to_string()),
                    }
                })?)
                .mm_err(|e| TendermintInitError {
                    ticker: ticker.to_string(),
                    kind: TendermintInitErrorKind::InvalidPrivKey(e.to_string()),
                })?;
            let bip39_secp_priv_key = global_hd.root_priv_key().clone();
            let pubkey = Public::from_slice(&bip39_secp_priv_key.public_key().to_bytes()).map_to_mm(|e| {
                TendermintInitError {
                    ticker: ticker.to_string(),
                    kind: TendermintInitErrorKind::Internal(e.to_string()),
                }
            })?;

            let tendermint_pair = TendermintKeyPair::new(activated_priv_key, pubkey);

            Ok(TendermintPrivKeyPolicy::HDWallet {
                path_to_coin: path_to_coin.clone(),
                activated_key: tendermint_pair,
                bip39_secp_priv_key,
            })
        },
        PrivKeyBuildPolicy::Trezor => {
            let kind =
                TendermintInitErrorKind::PrivKeyPolicyNotAllowed(PrivKeyPolicyNotAllowed::HardwareWalletNotSupported);
            MmError::err(TendermintInitError {
                ticker: ticker.to_string(),
                kind,
            })
        },
    }
}

pub(crate) fn chain_registry_name_from_account_prefix(ctx: &MmArc, prefix: &str) -> Option<String> {
    let Some(coins) = ctx.conf["coins"].as_array() else {
        return None;
    };

    for coin in coins {
        let protocol = coin
            .get("protocol")
            .unwrap_or(&serde_json::Value::Null)
            .get("type")
            .unwrap_or(&serde_json::Value::Null)
            .as_str();

        if protocol != Some(TENDERMINT_COIN_PROTOCOL_TYPE) {
            continue;
        }

        let coin_account_prefix = coin
            .get("protocol")
            .unwrap_or(&serde_json::Value::Null)
            .get("protocol_data")
            .unwrap_or(&serde_json::Value::Null)
            .get("account_prefix")
            .map(|t| t.as_str().unwrap_or_default());

        if coin_account_prefix == Some(prefix) {
            return coin
                .get("protocol")
                .unwrap_or(&serde_json::Value::Null)
                .get("protocol_data")
                .unwrap_or(&serde_json::Value::Null)
                .get("chain_registry_name")
                .map(|t| t.as_str().unwrap_or_default().to_owned());
        }
    }

    None
}

pub(crate) async fn create_withdraw_msg_as_any(
    sender: AccountId,
    receiver: AccountId,
    denom: &Denom,
    amount: u64,
    ibc_source_channel: Option<String>,
) -> Result<Any, MmError<WithdrawError>> {
    if let Some(channel_id) = ibc_source_channel {
        MsgTransfer::new_with_default_timeout(channel_id, sender, receiver, Coin {
            denom: denom.clone(),
            amount: amount.into(),
        })
        .to_any()
    } else {
        MsgSend {
            from_address: sender,
            to_address: receiver,
            amount: vec![Coin {
                denom: denom.clone(),
                amount: amount.into(),
            }],
        }
        .to_any()
    }
    .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))
}

pub async fn get_ibc_transfer_channels(
    source_registry_name: String,
    destination_registry_name: String,
) -> IBCTransferChannelsResult {
    #[derive(Deserialize)]
    struct ChainRegistry {
        channels: Vec<IbcChannel>,
    }

    #[derive(Deserialize)]
    struct ChannelInfo {
        channel_id: String,
        port_id: String,
    }

    #[derive(Deserialize)]
    struct IbcChannel {
        #[allow(dead_code)]
        chain_1: ChannelInfo,
        chain_2: ChannelInfo,
        ordering: String,
        version: String,
        tags: Option<IBCTransferChannelTag>,
    }

    let source_filename = format!("{}-{}.json", source_registry_name, destination_registry_name);
    let git_controller: GitController<GithubClient> = GitController::new(GITHUB_API_URI);

    let metadata_list = git_controller
        .client
        .get_file_metadata_list(
            CHAIN_REGISTRY_REPO_OWNER,
            CHAIN_REGISTRY_REPO_NAME,
            CHAIN_REGISTRY_BRANCH,
            CHAIN_REGISTRY_IBC_DIR_NAME,
        )
        .await
        .map_err(|e| IBCTransferChannelsRequestError::Transport(format!("{:?}", e)))?;

    let source_channel_file = metadata_list
        .iter()
        .find(|metadata| metadata.name == source_filename)
        .or_mm_err(|| IBCTransferChannelsRequestError::RegistrySourceCouldNotFound(source_filename))?;

    let mut registry_object = git_controller
        .client
        .deserialize_json_source::<ChainRegistry>(source_channel_file.to_owned())
        .await
        .map_err(|e| IBCTransferChannelsRequestError::Transport(format!("{:?}", e)))?;

    registry_object
        .channels
        .retain(|ch| ch.chain_2.port_id == *IBC_OUT_SOURCE_PORT);

    let result: Vec<IBCTransferChannel> = registry_object
        .channels
        .iter()
        .map(|ch| IBCTransferChannel {
            channel_id: ch.chain_2.channel_id.clone(),
            ordering: ch.ordering.clone(),
            version: ch.version.clone(),
            tags: ch.tags.clone().map(|t| IBCTransferChannelTag {
                status: t.status,
                preferred: t.preferred,
                dex: t.dex,
            }),
        })
        .collect();

    if result.is_empty() {
        return MmError::err(IBCTransferChannelsRequestError::CouldNotFindChannel(
            destination_registry_name,
        ));
    }

    Ok(IBCTransferChannelsResponse {
        ibc_transfer_channels: result,
    })
}

fn parse_expected_sequence_number(e: &str) -> MmResult<u64, TendermintCoinRpcError> {
    if let Some(sequence) = SEQUENCE_PARSER_REGEX.captures(e).and_then(|c| c.get(1)) {
        let account_sequence =
            u64::from_str(sequence.as_str()).map_to_mm(|e| TendermintCoinRpcError::InternalError(e.to_string()))?;

        return Ok(account_sequence);
    }

    MmError::err(TendermintCoinRpcError::InternalError(format!(
        "Could not parse the expected sequence number from this error message: '{}'",
        e
    )))
}

#[cfg(test)]
pub mod tendermint_falsecoin_tests {
    use super::*;

    use common::{block_on, wait_until_ms, DEX_FEE_ADDR_RAW_PUBKEY};
    use cosmrs::proto::cosmos::tx::v1beta1::{GetTxRequest, GetTxResponse, GetTxsEventResponse};
    use crypto::privkey::key_pair_from_seed;
    use std::mem::discriminant;

    pub const IRIS_TESTNET_HTLC_PAIR1_SEED: &str = "iris test seed";
    // pub const IRIS_TESTNET_HTLC_PAIR1_PUB_KEY: &[u8] = &[
    //     2, 35, 133, 39, 114, 92, 150, 175, 252, 203, 124, 85, 243, 144, 11, 52, 91, 128, 236, 82, 104, 212, 131, 40,
    //     79, 22, 40, 7, 119, 93, 50, 179, 43,
    // ];
    // const IRIS_TESTNET_HTLC_PAIR1_ADDRESS: &str = "iaa1e0rx87mdj79zejewuc4jg7ql9ud2286g2us8f2";

    // const IRIS_TESTNET_HTLC_PAIR2_SEED: &str = "iris test2 seed";
    const IRIS_TESTNET_HTLC_PAIR2_PUB_KEY: &[u8] = &[
        2, 90, 55, 151, 92, 7, 154, 117, 67, 96, 63, 202, 178, 78, 37, 101, 164, 173, 238, 60, 249, 175, 137, 52, 105,
        14, 16, 50, 130, 250, 64, 37, 17,
    ];
    const IRIS_TESTNET_HTLC_PAIR2_ADDRESS: &str = "iaa1erfnkjsmalkwtvj44qnfr2drfzdt4n9ldh0kjv";

    pub const IRIS_TESTNET_RPC_URL: &str = "http://34.80.202.172:26657";

    const TAKER_PAYMENT_SPEND_SEARCH_INTERVAL: f64 = 1.;
    const AVG_BLOCKTIME: u8 = 5;

    const SUCCEED_TX_HASH_SAMPLES: &[&str] = &[
        // https://nyancat.iobscan.io/#/tx?txHash=A010FC0AA33FC6D597A8635F9D127C0A7B892FAAC72489F4DADD90048CFE9279
        "A010FC0AA33FC6D597A8635F9D127C0A7B892FAAC72489F4DADD90048CFE9279",
        // https://nyancat.iobscan.io/#/tx?txHash=54FD77054AE311C484CC2EADD4621428BB23D14A9BAAC128B0E7B47422F86EC8
        "54FD77054AE311C484CC2EADD4621428BB23D14A9BAAC128B0E7B47422F86EC8",
        // https://nyancat.iobscan.io/#/tx?txHash=7C00FAE7F70C36A316A4736025B08A6EAA2A0CC7919A2C4FC4CD14D9FFD166F9
        "7C00FAE7F70C36A316A4736025B08A6EAA2A0CC7919A2C4FC4CD14D9FFD166F9",
    ];

    const FAILED_TX_HASH_SAMPLES: &[&str] = &[
        // https://nyancat.iobscan.io/#/tx?txHash=57EE62B2DF7E311C98C24AE2A53EB0FF2C16D289CECE0826CA1FF1108C91B3F9
        "57EE62B2DF7E311C98C24AE2A53EB0FF2C16D289CECE0826CA1FF1108C91B3F9",
        // https://nyancat.iobscan.io/#/tx?txHash=F3181D69C580318DFD54282C656AC81113BC600BCFBAAA480E6D8A6469EE8786
        "F3181D69C580318DFD54282C656AC81113BC600BCFBAAA480E6D8A6469EE8786",
        // https://nyancat.iobscan.io/#/tx?txHash=FE6F9F395DA94A14FCFC04E0E8C496197077D5F4968DA5528D9064C464ADF522
        "FE6F9F395DA94A14FCFC04E0E8C496197077D5F4968DA5528D9064C464ADF522",
    ];

    fn get_iris_usdc_ibc_protocol() -> TendermintProtocolInfo {
        TendermintProtocolInfo {
            decimals: 6,
            denom: String::from("ibc/5C465997B4F582F602CD64E12031C6A6E18CAF1E6EDC9B5D808822DC0B5F850C"),
            account_prefix: String::from("iaa"),
            chain_id: String::from("nyancat-9"),
            gas_price: None,
            chain_registry_name: None,
        }
    }

    fn get_iris_protocol() -> TendermintProtocolInfo {
        TendermintProtocolInfo {
            decimals: 6,
            denom: String::from("unyan"),
            account_prefix: String::from("iaa"),
            chain_id: String::from("nyancat-9"),
            gas_price: None,
            chain_registry_name: None,
        }
    }

    #[test]
    fn test_tx_hash_str_from_bytes() {
        let tx_hex = "0a97010a8f010a1c2f636f736d6f732e62616e6b2e763162657461312e4d736753656e64126f0a2d636f736d6f7331737661773061716334353834783832356a753775613033673578747877643061686c3836687a122d636f736d6f7331737661773061716334353834783832356a753775613033673578747877643061686c3836687a1a0f0a057561746f6d120631303030303018d998bf0512670a500a460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a2102000eef4ab169e7b26a4a16c47420c4176ab702119ba57a8820fb3e53c8e7506212040a020801180312130a0d0a057561746f6d12043130303010a08d061a4093e5aec96f7d311d129f5ec8714b21ad06a75e483ba32afab86354400b2ac8350bfc98731bbb05934bf138282750d71aadbe08ceb6bb195f2b55e1bbfdddaaad";
        let expected_hash = "1C25ED7D17FCC5959409498D5423594666C4E84F15AF7B4AF17DF29B2AF9E7F5";

        let tx_bytes = hex::decode(tx_hex).unwrap();
        let hash = sha256(&tx_bytes);
        assert_eq!(hex::encode_upper(hash.as_slice()), expected_hash);
    }

    #[test]
    fn test_htlc_create_and_claim() {
        let nodes = vec![RpcNode::for_test(IRIS_TESTNET_RPC_URL)];

        let protocol_conf = get_iris_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let tendermint_pair = TendermintKeyPair::new(key_pair.private().secret, *key_pair.public());
        let activation_policy =
            TendermintActivationPolicy::with_private_key_policy(TendermintPrivKeyPolicy::Iguana(tendermint_pair));

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "IRIS".to_string(),
            conf,
            protocol_conf,
            nodes,
            false,
            activation_policy,
            None,
        ))
        .unwrap();

        // << BEGIN HTLC CREATION
        let to: AccountId = IRIS_TESTNET_HTLC_PAIR2_ADDRESS.parse().unwrap();
        let amount = 1;
        let amount_dec = big_decimal_from_sat_unsigned(amount, coin.decimals);

        let mut sec = [0u8; 32];
        common::os_rng(&mut sec).unwrap();
        drop_mutability!(sec);

        let time_lock = 1000;

        let create_htlc_tx = coin
            .gen_create_htlc_tx(
                coin.denom.clone(),
                &to,
                amount.into(),
                sha256(&sec).as_slice(),
                time_lock,
            )
            .unwrap();

        let current_block_fut = coin.current_block().compat();
        let current_block = block_on(async { current_block_fut.await.unwrap() });
        let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

        let fee = block_on(async {
            coin.calculate_fee(
                create_htlc_tx.msg_payload.clone(),
                timeout_height,
                TX_DEFAULT_MEMO.to_owned(),
                None,
            )
            .await
            .unwrap()
        });

        let send_tx_fut = coin.common_send_raw_tx_bytes(
            create_htlc_tx.msg_payload.clone(),
            fee,
            timeout_height,
            TX_DEFAULT_MEMO.into(),
            Duration::from_secs(20),
        );
        block_on(async {
            send_tx_fut.await.unwrap();
        });
        // >> END HTLC CREATION

        let htlc_spent = block_on(coin.check_if_my_payment_sent(CheckIfMyPaymentSentArgs {
            time_lock: 0,
            other_pub: IRIS_TESTNET_HTLC_PAIR2_PUB_KEY,
            secret_hash: sha256(&sec).as_slice(),
            search_from_block: current_block,
            swap_contract_address: &None,
            swap_unique_data: &[],
            amount: &amount_dec,
            payment_instructions: &None,
        }))
        .unwrap();
        assert!(htlc_spent.is_some());

        // << BEGIN HTLC CLAIMING
        let claim_htlc_tx = coin.gen_claim_htlc_tx(create_htlc_tx.id, &sec).unwrap();

        let current_block_fut = coin.current_block().compat();
        let current_block = common::block_on(async { current_block_fut.await.unwrap() });
        let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

        let fee = block_on(async {
            coin.calculate_fee(
                claim_htlc_tx.msg_payload.clone(),
                timeout_height,
                TX_DEFAULT_MEMO.to_owned(),
                None,
            )
            .await
            .unwrap()
        });

        let send_tx_fut = coin.common_send_raw_tx_bytes(
            claim_htlc_tx.msg_payload,
            fee,
            timeout_height,
            TX_DEFAULT_MEMO.into(),
            Duration::from_secs(30),
        );

        let (tx_id, _tx_raw) = block_on(async { send_tx_fut.await.unwrap() });

        println!("Claim HTLC tx hash {}", tx_id);
        // >> END HTLC CLAIMING
    }

    #[test]
    fn try_query_claim_htlc_txs_and_get_secret() {
        let nodes = vec![RpcNode::for_test(IRIS_TESTNET_RPC_URL)];

        let protocol_conf = get_iris_usdc_ibc_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let tendermint_pair = TendermintKeyPair::new(key_pair.private().secret, *key_pair.public());
        let activation_policy =
            TendermintActivationPolicy::with_private_key_policy(TendermintPrivKeyPolicy::Iguana(tendermint_pair));

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "USDC-IBC".to_string(),
            conf,
            protocol_conf,
            nodes,
            false,
            activation_policy,
            None,
        ))
        .unwrap();

        let events = "claim_htlc.id='2B925FC83A106CC81590B3DB108AC2AE496FFA912F368FE5E29BC1ED2B754F2C'";
        // TODO: Remove deprecated attribute when new version of tendermint-rs is released
        #[allow(deprecated)]
        let request = GetTxsEventRequest {
            events: vec![events.into()],
            order_by: TendermintResultOrder::Ascending as i32,
            page: 1,
            limit: 1,
            pagination: None,
        };
        let response = block_on(block_on(coin.rpc_client()).unwrap().abci_query(
            Some(ABCI_GET_TXS_EVENT_PATH.to_string()),
            request.encode_to_vec(),
            ABCI_REQUEST_HEIGHT,
            ABCI_REQUEST_PROVE,
        ))
        .unwrap();
        println!("{:?}", response);

        let response = GetTxsEventResponse::decode(response.value.as_slice()).unwrap();
        let tx = response.txs.first().unwrap();
        println!("{:?}", tx);

        let first_msg = tx.body.as_ref().unwrap().messages.first().unwrap();
        println!("{:?}", first_msg);

        let claim_htlc = ClaimHtlcProto::decode(HtlcType::Iris, first_msg.value.as_slice()).unwrap();
        let expected_secret = [1; 32];
        let actual_secret = hex::decode(claim_htlc.secret()).unwrap();

        assert_eq!(actual_secret, expected_secret);
    }

    #[test]
    fn wait_for_tx_spend_test() {
        let nodes = vec![RpcNode::for_test(IRIS_TESTNET_RPC_URL)];

        let protocol_conf = get_iris_usdc_ibc_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let tendermint_pair = TendermintKeyPair::new(key_pair.private().secret, *key_pair.public());
        let activation_policy =
            TendermintActivationPolicy::with_private_key_policy(TendermintPrivKeyPolicy::Iguana(tendermint_pair));

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "USDC-IBC".to_string(),
            conf,
            protocol_conf,
            nodes,
            false,
            activation_policy,
            None,
        ))
        .unwrap();

        // https://nyancat.iobscan.io/#/tx?txHash=2DB382CE3D9953E4A94957B475B0E8A98F5B6DDB32D6BF0F6A765D949CF4A727
        let create_tx_hash = "2DB382CE3D9953E4A94957B475B0E8A98F5B6DDB32D6BF0F6A765D949CF4A727";

        let request = GetTxRequest {
            hash: create_tx_hash.into(),
        };

        let response = block_on(block_on(coin.rpc_client()).unwrap().abci_query(
            Some(ABCI_GET_TX_PATH.to_string()),
            request.encode_to_vec(),
            ABCI_REQUEST_HEIGHT,
            ABCI_REQUEST_PROVE,
        ))
        .unwrap();
        println!("{:?}", response);

        let response = GetTxResponse::decode(response.value.as_slice()).unwrap();
        let tx = response.tx.unwrap();

        println!("{:?}", tx);

        let encoded_tx = tx.encode_to_vec();

        let secret_hash = hex::decode("0C34C71EBA2A51738699F9F3D6DAFFB15BE576E8ED543203485791B5DA39D10D").unwrap();
        let spend_tx = block_on(coin.wait_for_htlc_tx_spend(WaitForHTLCTxSpendArgs {
            tx_bytes: &encoded_tx,
            secret_hash: &secret_hash,
            wait_until: get_utc_timestamp() as u64,
            from_block: 0,
            swap_contract_address: &None,
            check_every: TAKER_PAYMENT_SPEND_SEARCH_INTERVAL,
            watcher_reward: false,
        }))
        .unwrap();

        // https://nyancat.iobscan.io/#/tx?txHash=565C820C1F95556ADC251F16244AAD4E4274772F41BC13F958C9C2F89A14D137
        let expected_spend_hash = "565C820C1F95556ADC251F16244AAD4E4274772F41BC13F958C9C2F89A14D137";
        let hash = spend_tx.tx_hash_as_bytes();
        assert_eq!(hex::encode_upper(hash.0), expected_spend_hash);
    }

    #[test]
    fn validate_taker_fee_test() {
        let nodes = vec![RpcNode::for_test(IRIS_TESTNET_RPC_URL)];

        let protocol_conf = get_iris_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let tendermint_pair = TendermintKeyPair::new(key_pair.private().secret, *key_pair.public());
        let activation_policy =
            TendermintActivationPolicy::with_private_key_policy(TendermintPrivKeyPolicy::Iguana(tendermint_pair));

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "IRIS-TEST".to_string(),
            conf,
            protocol_conf,
            nodes,
            false,
            activation_policy,
            None,
        ))
        .unwrap();

        // CreateHtlc tx, validation should fail because first message of dex fee tx must be MsgSend
        // https://nyancat.iobscan.io/#/tx?txHash=2DB382CE3D9953E4A94957B475B0E8A98F5B6DDB32D6BF0F6A765D949CF4A727
        let create_htlc_tx_hash = "2DB382CE3D9953E4A94957B475B0E8A98F5B6DDB32D6BF0F6A765D949CF4A727";
        let create_htlc_tx_bytes = block_on(coin.request_tx(create_htlc_tx_hash.into()))
            .unwrap()
            .encode_to_vec();
        let create_htlc_tx = TransactionEnum::CosmosTransaction(CosmosTransaction {
            data: TxRaw::decode(create_htlc_tx_bytes.as_slice()).unwrap(),
        });

        let invalid_amount: MmNumber = 1.into();
        let error = block_on(coin.validate_fee(ValidateFeeArgs {
            fee_tx: &create_htlc_tx,
            expected_sender: &[],
            fee_addr: &DEX_FEE_ADDR_RAW_PUBKEY,
            dex_fee: &DexFee::Standard(invalid_amount.clone()),
            min_block_number: 0,
            uuid: &[1; 16],
        }))
        .unwrap_err()
        .into_inner();
        println!("{}", error);
        match error {
            ValidatePaymentError::TxDeserializationError(err) => {
                assert!(err.contains("failed to decode Protobuf message: MsgSend.amount"))
            },
            _ => panic!(
                "Expected `WrongPaymentTx` MsgSend.amount decode failure, found {:?}",
                error
            ),
        }

        // just a random transfer tx not related to AtomicDEX, should fail on recipient address check
        // https://nyancat.iobscan.io/#/tx?txHash=65815814E7D74832D87956144C1E84801DC94FE9A509D207A0ABC3F17775E5DF
        let random_transfer_tx_hash = "65815814E7D74832D87956144C1E84801DC94FE9A509D207A0ABC3F17775E5DF";
        let random_transfer_tx_bytes = block_on(coin.request_tx(random_transfer_tx_hash.into()))
            .unwrap()
            .encode_to_vec();

        let random_transfer_tx = TransactionEnum::CosmosTransaction(CosmosTransaction {
            data: TxRaw::decode(random_transfer_tx_bytes.as_slice()).unwrap(),
        });

        let error = block_on(coin.validate_fee(ValidateFeeArgs {
            fee_tx: &random_transfer_tx,
            expected_sender: &[],
            fee_addr: &DEX_FEE_ADDR_RAW_PUBKEY,
            dex_fee: &DexFee::Standard(invalid_amount.clone()),
            min_block_number: 0,
            uuid: &[1; 16],
        }))
        .unwrap_err()
        .into_inner();
        println!("{}", error);
        match error {
            ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("sent to wrong address")),
            _ => panic!("Expected `WrongPaymentTx` wrong address, found {:?}", error),
        }

        // dex fee tx sent during real swap
        // https://nyancat.iobscan.io/#/tx?txHash=8AA6B9591FE1EE93C8B89DE4F2C59B2F5D3473BD9FB5F3CFF6A5442BEDC881D7
        let dex_fee_hash = "8AA6B9591FE1EE93C8B89DE4F2C59B2F5D3473BD9FB5F3CFF6A5442BEDC881D7";
        let dex_fee_tx = block_on(coin.request_tx(dex_fee_hash.into())).unwrap();

        let pubkey = dex_fee_tx.auth_info.as_ref().unwrap().signer_infos[0]
            .public_key
            .as_ref()
            .unwrap()
            .value[2..]
            .to_vec();
        let dex_fee_tx = TransactionEnum::CosmosTransaction(CosmosTransaction {
            data: TxRaw::decode(dex_fee_tx.encode_to_vec().as_slice()).unwrap(),
        });

        let error = block_on(coin.validate_fee(ValidateFeeArgs {
            fee_tx: &dex_fee_tx,
            expected_sender: &[],
            fee_addr: &DEX_FEE_ADDR_RAW_PUBKEY,
            dex_fee: &DexFee::Standard(invalid_amount),
            min_block_number: 0,
            uuid: &[1; 16],
        }))
        .unwrap_err()
        .into_inner();
        println!("{}", error);
        match error {
            ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("Invalid amount")),
            _ => panic!("Expected `WrongPaymentTx` invalid amount, found {:?}", error),
        }

        let valid_amount: BigDecimal = "0.0001".parse().unwrap();
        // valid amount but invalid sender
        let error = block_on(coin.validate_fee(ValidateFeeArgs {
            fee_tx: &dex_fee_tx,
            expected_sender: &DEX_FEE_ADDR_RAW_PUBKEY,
            fee_addr: &DEX_FEE_ADDR_RAW_PUBKEY,
            dex_fee: &DexFee::Standard(valid_amount.clone().into()),
            min_block_number: 0,
            uuid: &[1; 16],
        }))
        .unwrap_err()
        .into_inner();
        println!("{}", error);
        match error {
            ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("Invalid sender")),
            _ => panic!("Expected `WrongPaymentTx` invalid sender, found {:?}", error),
        }

        // invalid memo
        let error = block_on(coin.validate_fee(ValidateFeeArgs {
            fee_tx: &dex_fee_tx,
            expected_sender: &pubkey,
            fee_addr: &DEX_FEE_ADDR_RAW_PUBKEY,
            dex_fee: &DexFee::Standard(valid_amount.into()),
            min_block_number: 0,
            uuid: &[1; 16],
        }))
        .unwrap_err()
        .into_inner();
        println!("{}", error);
        match error {
            ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("Invalid memo")),
            _ => panic!("Expected `WrongPaymentTx` invalid memo, found {:?}", error),
        }

        // https://nyancat.iobscan.io/#/tx?txHash=5939A9D1AF57BB828714E0C4C4D7F2AEE349BB719B0A1F25F8FBCC3BB227C5F9
        let fee_with_memo_hash = "5939A9D1AF57BB828714E0C4C4D7F2AEE349BB719B0A1F25F8FBCC3BB227C5F9";
        let fee_with_memo_tx = block_on(coin.request_tx(fee_with_memo_hash.into())).unwrap();

        let pubkey = fee_with_memo_tx.auth_info.as_ref().unwrap().signer_infos[0]
            .public_key
            .as_ref()
            .unwrap()
            .value[2..]
            .to_vec();

        let fee_with_memo_tx = TransactionEnum::CosmosTransaction(CosmosTransaction {
            data: TxRaw::decode(fee_with_memo_tx.encode_to_vec().as_slice()).unwrap(),
        });

        let uuid: Uuid = "cae6011b-9810-4710-b784-1e5dd0b3a0d0".parse().unwrap();
        let amount: BigDecimal = "0.0001".parse().unwrap();
        block_on(
            coin.validate_fee_for_denom(
                &fee_with_memo_tx,
                &pubkey,
                &DEX_FEE_ADDR_RAW_PUBKEY,
                &amount,
                6,
                uuid.as_bytes(),
                "nim".into(),
            )
            .compat(),
        )
        .unwrap();
    }

    #[test]
    fn validate_payment_test() {
        let nodes = vec![RpcNode::for_test(IRIS_TESTNET_RPC_URL)];

        let protocol_conf = get_iris_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let tendermint_pair = TendermintKeyPair::new(key_pair.private().secret, *key_pair.public());
        let activation_policy =
            TendermintActivationPolicy::with_private_key_policy(TendermintPrivKeyPolicy::Iguana(tendermint_pair));

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "IRIS-TEST".to_string(),
            conf,
            protocol_conf,
            nodes,
            false,
            activation_policy,
            None,
        ))
        .unwrap();

        // just a random transfer tx not related to AtomicDEX, should fail because the message is not CreateHtlc
        // https://nyancat.iobscan.io/#/tx?txHash=65815814E7D74832D87956144C1E84801DC94FE9A509D207A0ABC3F17775E5DF
        let random_transfer_tx_hash = "65815814E7D74832D87956144C1E84801DC94FE9A509D207A0ABC3F17775E5DF";
        let random_transfer_tx_bytes = block_on(coin.request_tx(random_transfer_tx_hash.into()))
            .unwrap()
            .encode_to_vec();

        let input = ValidatePaymentInput {
            payment_tx: random_transfer_tx_bytes,
            time_lock_duration: 0,
            time_lock: 0,
            other_pub: Vec::new(),
            secret_hash: Vec::new(),
            amount: Default::default(),
            swap_contract_address: None,
            try_spv_proof_until: 0,
            confirmations: 0,
            unique_swap_data: Vec::new(),
            watcher_reward: None,
        };
        let validate_err = block_on(coin.validate_taker_payment(input)).unwrap_err();
        match validate_err.into_inner() {
            ValidatePaymentError::WrongPaymentTx(e) => assert!(e.contains("Incorrect CreateHtlc message")),
            unexpected => panic!("Unexpected error variant {:?}", unexpected),
        };

        // The HTLC that was already claimed or refunded should not pass the validation
        // https://nyancat.iobscan.io/#/tx?txHash=93CF377D470EB27BD6E2C5B95BFEFE99359F95B88C70D785B34D1D2C670201B9
        let claimed_htlc_tx_hash = "93CF377D470EB27BD6E2C5B95BFEFE99359F95B88C70D785B34D1D2C670201B9";
        let claimed_htlc_tx_bytes = block_on(coin.request_tx(claimed_htlc_tx_hash.into()))
            .unwrap()
            .encode_to_vec();

        let input = ValidatePaymentInput {
            payment_tx: claimed_htlc_tx_bytes,
            time_lock_duration: 20000,
            time_lock: 1664984893,
            other_pub: hex::decode("025a37975c079a7543603fcab24e2565a4adee3cf9af8934690e103282fa402511").unwrap(),
            secret_hash: hex::decode("441d0237e93677d3458e1e5a2e69f61e3622813521bf048dd56290306acdd134").unwrap(),
            amount: "0.01".parse().unwrap(),
            swap_contract_address: None,
            try_spv_proof_until: 0,
            confirmations: 0,
            unique_swap_data: Vec::new(),
            watcher_reward: None,
        };
        let validate_err = block_on(coin.validate_payment_for_denom(input, "nim".parse().unwrap(), 6)).unwrap_err();
        match validate_err.into_inner() {
            ValidatePaymentError::UnexpectedPaymentState(_) => (),
            unexpected => panic!("Unexpected error variant {:?}", unexpected),
        };
    }

    #[test]
    fn test_search_for_swap_tx_spend_spent() {
        let nodes = vec![RpcNode::for_test(IRIS_TESTNET_RPC_URL)];

        let protocol_conf = get_iris_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let tendermint_pair = TendermintKeyPair::new(key_pair.private().secret, *key_pair.public());
        let activation_policy =
            TendermintActivationPolicy::with_private_key_policy(TendermintPrivKeyPolicy::Iguana(tendermint_pair));

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "IRIS-TEST".to_string(),
            conf,
            protocol_conf,
            nodes,
            false,
            activation_policy,
            None,
        ))
        .unwrap();

        // https://nyancat.iobscan.io/#/tx?txHash=2DB382CE3D9953E4A94957B475B0E8A98F5B6DDB32D6BF0F6A765D949CF4A727
        let create_tx_hash = "2DB382CE3D9953E4A94957B475B0E8A98F5B6DDB32D6BF0F6A765D949CF4A727";

        let request = GetTxRequest {
            hash: create_tx_hash.into(),
        };

        let response = block_on(block_on(coin.rpc_client()).unwrap().abci_query(
            Some(ABCI_GET_TX_PATH.to_string()),
            request.encode_to_vec(),
            ABCI_REQUEST_HEIGHT,
            ABCI_REQUEST_PROVE,
        ))
        .unwrap();
        println!("{:?}", response);

        let response = GetTxResponse::decode(response.value.as_slice()).unwrap();
        let tx = response.tx.unwrap();

        println!("{:?}", tx);

        let encoded_tx = tx.encode_to_vec();

        let secret_hash = hex::decode("0C34C71EBA2A51738699F9F3D6DAFFB15BE576E8ED543203485791B5DA39D10D").unwrap();
        let input = SearchForSwapTxSpendInput {
            time_lock: 0,
            other_pub: &[],
            secret_hash: &secret_hash,
            tx: &encoded_tx,
            search_from_block: 0,
            swap_contract_address: &None,
            swap_unique_data: &[],
            watcher_reward: false,
        };

        let spend_tx = match block_on(coin.search_for_swap_tx_spend_my(input)).unwrap().unwrap() {
            FoundSwapTxSpend::Spent(tx) => tx,
            unexpected => panic!("Unexpected search_for_swap_tx_spend_my result {:?}", unexpected),
        };

        // https://nyancat.iobscan.io/#/tx?txHash=565C820C1F95556ADC251F16244AAD4E4274772F41BC13F958C9C2F89A14D137
        let expected_spend_hash = "565C820C1F95556ADC251F16244AAD4E4274772F41BC13F958C9C2F89A14D137";
        let hash = spend_tx.tx_hash_as_bytes();
        assert_eq!(hex::encode_upper(hash.0), expected_spend_hash);
    }

    #[test]
    fn test_search_for_swap_tx_spend_refunded() {
        let nodes = vec![RpcNode::for_test(IRIS_TESTNET_RPC_URL)];

        let protocol_conf = get_iris_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let tendermint_pair = TendermintKeyPair::new(key_pair.private().secret, *key_pair.public());
        let activation_policy =
            TendermintActivationPolicy::with_private_key_policy(TendermintPrivKeyPolicy::Iguana(tendermint_pair));

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "IRIS-TEST".to_string(),
            conf,
            protocol_conf,
            nodes,
            false,
            activation_policy,
            None,
        ))
        .unwrap();

        // https://nyancat.iobscan.io/#/tx?txHash=BD1A76F43E8E2C7A1104EE363D63455CD50C76F2BFE93B703235F0A973061297
        let create_tx_hash = "BD1A76F43E8E2C7A1104EE363D63455CD50C76F2BFE93B703235F0A973061297";

        let request = GetTxRequest {
            hash: create_tx_hash.into(),
        };

        let response = block_on(block_on(coin.rpc_client()).unwrap().abci_query(
            Some(ABCI_GET_TX_PATH.to_string()),
            request.encode_to_vec(),
            ABCI_REQUEST_HEIGHT,
            ABCI_REQUEST_PROVE,
        ))
        .unwrap();
        println!("{:?}", response);

        let response = GetTxResponse::decode(response.value.as_slice()).unwrap();
        let tx = response.tx.unwrap();

        println!("{:?}", tx);

        let encoded_tx = tx.encode_to_vec();

        let secret_hash = hex::decode("cb11cacffdfc82060aa4a9a1bb9cc094c4141b170994f7642cd54d7e7af6743e").unwrap();
        let input = SearchForSwapTxSpendInput {
            time_lock: 0,
            other_pub: &[],
            secret_hash: &secret_hash,
            tx: &encoded_tx,
            search_from_block: 0,
            swap_contract_address: &None,
            swap_unique_data: &[],
            watcher_reward: false,
        };

        match block_on(coin.search_for_swap_tx_spend_my(input)).unwrap().unwrap() {
            FoundSwapTxSpend::Refunded(tx) => {
                let expected = TransactionEnum::CosmosTransaction(CosmosTransaction { data: TxRaw::default() });
                assert_eq!(expected, tx);
            },
            unexpected => panic!("Unexpected search_for_swap_tx_spend_my result {:?}", unexpected),
        };
    }

    #[test]
    fn test_get_tx_status_code_or_none() {
        let nodes = vec![RpcNode::for_test(IRIS_TESTNET_RPC_URL)];
        let protocol_conf = get_iris_usdc_ibc_protocol();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();
        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let tendermint_pair = TendermintKeyPair::new(key_pair.private().secret, *key_pair.public());
        let activation_policy =
            TendermintActivationPolicy::with_private_key_policy(TendermintPrivKeyPolicy::Iguana(tendermint_pair));

        let coin = common::block_on(TendermintCoin::init(
            &ctx,
            "USDC-IBC".to_string(),
            conf,
            protocol_conf,
            nodes,
            false,
            activation_policy,
            None,
        ))
        .unwrap();

        for succeed_tx_hash in SUCCEED_TX_HASH_SAMPLES {
            let status_code = common::block_on(coin.get_tx_status_code_or_none(succeed_tx_hash.to_string()))
                .unwrap()
                .expect("tx exists");

            assert_eq!(status_code, cosmrs::tendermint::abci::Code::Ok);
        }

        for failed_tx_hash in FAILED_TX_HASH_SAMPLES {
            let status_code = common::block_on(coin.get_tx_status_code_or_none(failed_tx_hash.to_string()))
                .unwrap()
                .expect("tx exists");

            assert_eq!(
                discriminant(&status_code),
                discriminant(&cosmrs::tendermint::abci::Code::Err(NonZeroU32::new(61).unwrap()))
            );
        }

        // Doesn't exists
        let tx_hash = "0000000000000000000000000000000000000000000000000000000000000000".to_string();
        let status_code = common::block_on(coin.get_tx_status_code_or_none(tx_hash)).unwrap();
        assert!(status_code.is_none());
    }

    #[test]
    fn test_wait_for_confirmations() {
        const CHECK_INTERVAL: u64 = 2;

        let nodes = vec![RpcNode::for_test(IRIS_TESTNET_RPC_URL)];
        let protocol_conf = get_iris_usdc_ibc_protocol();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();
        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let tendermint_pair = TendermintKeyPair::new(key_pair.private().secret, *key_pair.public());
        let activation_policy =
            TendermintActivationPolicy::with_private_key_policy(TendermintPrivKeyPolicy::Iguana(tendermint_pair));

        let coin = common::block_on(TendermintCoin::init(
            &ctx,
            "USDC-IBC".to_string(),
            conf,
            protocol_conf,
            nodes,
            false,
            activation_policy,
            None,
        ))
        .unwrap();

        let wait_until = || wait_until_ms(45);

        for succeed_tx_hash in SUCCEED_TX_HASH_SAMPLES {
            let tx_bytes = block_on(coin.request_tx(succeed_tx_hash.to_string()))
                .unwrap()
                .encode_to_vec();

            let confirm_payment_input = ConfirmPaymentInput {
                payment_tx: tx_bytes,
                confirmations: 0,
                requires_nota: false,
                wait_until: wait_until(),
                check_every: CHECK_INTERVAL,
            };
            block_on(coin.wait_for_confirmations(confirm_payment_input).compat()).unwrap();
        }

        for failed_tx_hash in FAILED_TX_HASH_SAMPLES {
            let tx_bytes = block_on(coin.request_tx(failed_tx_hash.to_string()))
                .unwrap()
                .encode_to_vec();

            let confirm_payment_input = ConfirmPaymentInput {
                payment_tx: tx_bytes,
                confirmations: 0,
                requires_nota: false,
                wait_until: wait_until(),
                check_every: CHECK_INTERVAL,
            };
            block_on(coin.wait_for_confirmations(confirm_payment_input).compat()).unwrap_err();
        }
    }

    #[test]
    fn test_generate_account_id() {
        let key_pair = key_pair_from_seed("best seed").unwrap();

        let tendermint_pair = TendermintKeyPair::new(key_pair.private().secret, *key_pair.public());
        let pb = PublicKey::from_raw_secp256k1(&key_pair.public().to_bytes()).unwrap();

        let pk_activation_policy =
            TendermintActivationPolicy::with_private_key_policy(TendermintPrivKeyPolicy::Iguana(tendermint_pair));
        // Derive account id from the private key.
        let pk_account_id = pk_activation_policy.generate_account_id("cosmos").unwrap();
        assert_eq!(
            pk_account_id.to_string(),
            "cosmos1aghdjgt5gzntzqgdxdzhjfry90upmtfsy2wuwp"
        );

        let pb_activation_policy = TendermintActivationPolicy::with_public_key(pb);
        // Derive account id from the public key.
        let pb_account_id = pb_activation_policy.generate_account_id("cosmos").unwrap();
        // Public and private keys are from the same keypair, account ids must be equal.
        assert_eq!(pk_account_id, pb_account_id);
    }

    #[test]
    fn test_parse_expected_sequence_number() {
        assert_eq!(
            13,
            parse_expected_sequence_number("check_tx log: account sequence mismatch, expected 13").unwrap()
        );
        assert_eq!(
            5,
            parse_expected_sequence_number("check_tx log: account sequence mismatch, expected 5, got...").unwrap()
        );
        assert_eq!(17, parse_expected_sequence_number("account sequence mismatch, expected. check_tx log: account sequence mismatch, expected 17, got 16: incorrect account sequence, deliver_tx log...").unwrap());
        assert!(parse_expected_sequence_number("").is_err());
        assert!(parse_expected_sequence_number("check_tx log: account sequence mismatch, expected").is_err());
    }
}
