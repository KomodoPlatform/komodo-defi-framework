use super::ibc::transfer_v1::MsgTransfer;
use super::ibc::IBC_GAS_LIMIT_DEFAULT;
use super::iris::ethermint_account::EthermintAccount;
use super::iris::htlc::{IrisHtlc, MsgClaimHtlc, MsgCreateHtlc, HTLC_STATE_COMPLETED, HTLC_STATE_OPEN,
                        HTLC_STATE_REFUNDED};
use super::iris::htlc_proto::{CreateHtlcProtoRep, QueryHtlcRequestProto, QueryHtlcResponseProto};
use super::rpc::*;
use crate::coin_errors::{MyAddressError, ValidatePaymentError};
use crate::rpc_command::tendermint::{IBCChainRegistriesResponse, IBCChainRegistriesResult, IBCChainsRequestError,
                                     IBCTransferChannel, IBCTransferChannelTag, IBCTransferChannelsRequest,
                                     IBCTransferChannelsRequestError, IBCTransferChannelsResponse,
                                     IBCTransferChannelsResult, IBCWithdrawRequest, CHAIN_REGISTRY_BRANCH,
                                     CHAIN_REGISTRY_IBC_DIR_NAME, CHAIN_REGISTRY_REPO_NAME, CHAIN_REGISTRY_REPO_OWNER};
use crate::tendermint::ibc::IBC_OUT_SOURCE_PORT;
use crate::utxo::sat_from_big_decimal;
use crate::utxo::utxo_common::big_decimal_from_sat;
use crate::{big_decimal_from_sat_unsigned, BalanceError, BalanceFut, BigDecimal, CheckIfMyPaymentSentArgs,
            CoinBalance, CoinFutSpawner, ConfirmPaymentInput, FeeApproxStage, FoundSwapTxSpend, HistorySyncState,
            MakerSwapTakerCoin, MarketCoinOps, MmCoin, MmCoinEnum, NegotiateSwapContractAddrErr,
            PaymentInstructionArgs, PaymentInstructions, PaymentInstructionsErr, PrivKeyBuildPolicy, PrivKeyPolicy,
            PrivKeyPolicyNotAllowed, RawTransactionError, RawTransactionFut, RawTransactionRequest, RawTransactionRes,
            RefundError, RefundPaymentArgs, RefundResult, RpcCommonOps, SearchForSwapTxSpendInput,
            SendMakerPaymentSpendPreimageInput, SendPaymentArgs, SignatureError, SignatureResult, SpendPaymentArgs,
            SwapOps, TakerSwapMakerCoin, TradeFee, TradePreimageError, TradePreimageFut, TradePreimageResult,
            TradePreimageValue, TransactionDetails, TransactionEnum, TransactionErr, TransactionFut,
            TransactionResult, TransactionType, TxFeeDetails, TxMarshalingErr, UnexpectedDerivationMethod,
            ValidateAddressResult, ValidateFeeArgs, ValidateInstructionsErr, ValidateOtherPubKeyErr,
            ValidatePaymentFut, ValidatePaymentInput, ValidateWatcherSpendInput, VerificationError,
            VerificationResult, WaitForHTLCTxSpendArgs, WatcherOps, WatcherReward, WatcherRewardError,
            WatcherSearchForSwapTxSpendInput, WatcherValidatePaymentInput, WatcherValidateTakerFeeInput,
            WithdrawError, WithdrawFee, WithdrawFrom, WithdrawFut, WithdrawRequest};
use async_std::prelude::FutureExt as AsyncStdFutureExt;
use async_trait::async_trait;
use bitcrypto::{dhash160, sha256};
use common::executor::{abortable_queue::AbortableQueue, AbortableSystem};
use common::executor::{AbortedError, Timer};
use common::log::{debug, warn};
use common::{get_utc_timestamp, now_sec, Future01CompatExt, DEX_FEE_ADDR_PUBKEY};
use cosmrs::bank::MsgSend;
use cosmrs::crypto::secp256k1::SigningKey;
use cosmrs::proto::cosmos::auth::v1beta1::{BaseAccount, QueryAccountRequest, QueryAccountResponse};
use cosmrs::proto::cosmos::bank::v1beta1::{MsgSend as MsgSendProto, QueryBalanceRequest, QueryBalanceResponse};
use cosmrs::proto::cosmos::base::tendermint::v1beta1::{GetBlockByHeightRequest, GetBlockByHeightResponse,
                                                       GetLatestBlockRequest, GetLatestBlockResponse};
use cosmrs::proto::cosmos::base::v1beta1::Coin as CoinProto;
use cosmrs::proto::cosmos::tx::v1beta1::{GetTxRequest, GetTxResponse, GetTxsEventRequest, GetTxsEventResponse,
                                         SimulateRequest, SimulateResponse, Tx, TxBody, TxRaw};
use cosmrs::tendermint::block::Height;
use cosmrs::tendermint::chain::Id as ChainId;
use cosmrs::tendermint::PublicKey;
use cosmrs::tx::{self, Fee, Msg, Raw, SignDoc, SignerInfo};
use cosmrs::{AccountId, Any, Coin, Denom, ErrorReport};
use crypto::privkey::key_pair_from_secret;
use crypto::{Secp256k1Secret, StandardHDCoinAddress, StandardHDPathToCoin};
use derive_more::Display;
use futures::future::try_join_all;
use futures::lock::Mutex as AsyncMutex;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use hex::FromHexError;
use itertools::Itertools;
use keys::KeyPair;
use mm2_core::mm_ctx::{MmArc, MmWeak};
use mm2_err_handle::prelude::*;
use mm2_git::{FileMetadata, GitController, GithubClient, RepositoryOperations, GITHUB_API_URI};
use mm2_number::MmNumber;
use parking_lot::Mutex as PaMutex;
use primitives::hash::H256;
use prost::{DecodeError, Message};
use rpc::v1::types::Bytes as BytesJson;
use serde_json::{self as json, Value as Json};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

// ABCI Request Paths
const ABCI_GET_LATEST_BLOCK_PATH: &str = "/cosmos.base.tendermint.v1beta1.Service/GetLatestBlock";
const ABCI_GET_BLOCK_BY_HEIGHT_PATH: &str = "/cosmos.base.tendermint.v1beta1.Service/GetBlockByHeight";
const ABCI_SIMULATE_TX_PATH: &str = "/cosmos.tx.v1beta1.Service/Simulate";
const ABCI_QUERY_ACCOUNT_PATH: &str = "/cosmos.auth.v1beta1.Query/Account";
const ABCI_QUERY_BALANCE_PATH: &str = "/cosmos.bank.v1beta1.Query/Balance";
const ABCI_GET_TX_PATH: &str = "/cosmos.tx.v1beta1.Service/GetTx";
const ABCI_QUERY_HTLC_PATH: &str = "/irismod.htlc.Query/HTLC";
const ABCI_GET_TXS_EVENT_PATH: &str = "/cosmos.tx.v1beta1.Service/GetTxsEvent";

pub(crate) const MIN_TX_SATOSHIS: i64 = 1;

// ABCI Request Defaults
const ABCI_REQUEST_HEIGHT: Option<Height> = None;
const ABCI_REQUEST_PROVE: bool = false;

/// 0.25 is good average gas price on atom and iris
const DEFAULT_GAS_PRICE: f64 = 0.25;
pub(super) const TIMEOUT_HEIGHT_DELTA: u64 = 100;
pub const GAS_LIMIT_DEFAULT: u64 = 125_000;
pub(crate) const TX_DEFAULT_MEMO: &str = "";

// https://github.com/irisnet/irismod/blob/5016c1be6fdbcffc319943f33713f4a057622f0a/modules/htlc/types/validation.go#L19-L22
const MAX_TIME_LOCK: i64 = 34560;
const MIN_TIME_LOCK: i64 = 50;

const ACCOUNT_SEQUENCE_ERR: &str = "incorrect account sequence";

type TendermintPrivKeyPolicy = PrivKeyPolicy<Secp256k1Secret>;

#[async_trait]
pub trait TendermintCommons {
    fn platform_denom(&self) -> &Denom;

    fn set_history_sync_state(&self, new_state: HistorySyncState);

    async fn get_block_timestamp(&self, block: i64) -> MmResult<Option<u64>, TendermintCoinRpcError>;

    async fn all_balances(&self) -> MmResult<AllBalancesResult, TendermintCoinRpcError>;

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
    chain_id: String,
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
    derivation_path: Option<StandardHDPathToCoin>,
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

pub struct TendermintCoinImpl {
    ticker: String,
    /// As seconds
    avg_blocktime: u8,
    /// My address
    pub account_id: AccountId,
    pub(super) account_prefix: String,
    pub(super) priv_key_policy: TendermintPrivKeyPolicy,
    pub(crate) decimals: u8,
    pub(super) denom: Denom,
    chain_id: ChainId,
    gas_price: Option<f64>,
    pub tokens_info: PaMutex<HashMap<String, ActivatedTokenInfo>>,
    /// This spawner is used to spawn coin's related futures that should be aborted on coin deactivation
    /// or on [`MmArc::stop`].
    pub(super) abortable_system: AbortableQueue,
    pub(crate) history_sync_state: Mutex<HistorySyncState>,
    client: TendermintRpcClient,
    chain_registry_name: Option<String>,
    pub(crate) ctx: MmWeak,
}

#[derive(Clone)]
pub struct TendermintCoin(Arc<TendermintCoinImpl>);

impl Deref for TendermintCoin {
    type Target = TendermintCoinImpl;

    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug)]
pub struct TendermintInitError {
    pub ticker: String,
    pub kind: TendermintInitErrorKind,
}

#[derive(Display, Debug)]
pub enum TendermintInitErrorKind {
    Internal(String),
    InvalidPrivKey(String),
    CouldNotGenerateAccountId(String),
    EmptyRpcUrls,
    RpcClientInitError(String),
    InvalidChainId(String),
    InvalidDenom(String),
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
}

#[derive(Display, Debug)]
pub enum TendermintCoinRpcError {
    Prost(DecodeError),
    InvalidResponse(String),
    PerformError(String),
    RpcClientError(String),
    InternalError(String),
}

impl From<DecodeError> for TendermintCoinRpcError {
    fn from(err: DecodeError) -> Self { TendermintCoinRpcError::Prost(err) }
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
            TendermintCoinRpcError::Prost(e) => BalanceError::InvalidResponse(e.to_string()),
            TendermintCoinRpcError::PerformError(e) => BalanceError::Transport(e),
            TendermintCoinRpcError::RpcClientError(e) => BalanceError::Transport(e),
            TendermintCoinRpcError::InternalError(e) => BalanceError::Internal(e),
        }
    }
}

impl From<TendermintCoinRpcError> for ValidatePaymentError {
    fn from(err: TendermintCoinRpcError) -> Self {
        match err {
            TendermintCoinRpcError::InvalidResponse(e) => ValidatePaymentError::InvalidRpcResponse(e),
            TendermintCoinRpcError::Prost(e) => ValidatePaymentError::InvalidRpcResponse(e.to_string()),
            TendermintCoinRpcError::PerformError(e) => ValidatePaymentError::Transport(e),
            TendermintCoinRpcError::RpcClientError(e) => ValidatePaymentError::Transport(e),
            TendermintCoinRpcError::InternalError(e) => ValidatePaymentError::InternalError(e),
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

    fn tx_hash(&self) -> BytesJson {
        let bytes = self.data.encode_to_vec();
        let hash = sha256(&bytes);
        hash.to_vec().into()
    }
}

pub(crate) fn account_id_from_privkey(priv_key: &[u8], prefix: &str) -> MmResult<AccountId, TendermintInitErrorKind> {
    let signing_key =
        SigningKey::from_bytes(priv_key).map_to_mm(|e| TendermintInitErrorKind::InvalidPrivKey(e.to_string()))?;

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

pub fn account_id_from_pubkey_hex(prefix: &str, pubkey: &str) -> MmResult<AccountId, AccountIdFromPubkeyHexErr> {
    let pubkey_bytes = hex::decode(pubkey)?;
    let pubkey_hash = dhash160(&pubkey_bytes);
    Ok(AccountId::new(prefix, pubkey_hash.as_slice())?)
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

    async fn all_balances(&self) -> MmResult<AllBalancesResult, TendermintCoinRpcError> {
        let platform_balance_denom = self
            .account_balance_for_denom(&self.account_id, self.denom.to_string())
            .await?;
        let platform_balance = big_decimal_from_sat_unsigned(platform_balance_denom, self.decimals);
        let ibc_assets_info = self.tokens_info.lock().clone();

        let mut requests = Vec::new();
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
    pub async fn init(
        ctx: &MmArc,
        ticker: String,
        conf: TendermintConf,
        protocol_info: TendermintProtocolInfo,
        rpc_urls: Vec<String>,
        tx_history: bool,
        priv_key_policy: TendermintPrivKeyPolicy,
    ) -> MmResult<Self, TendermintInitError> {
        if rpc_urls.is_empty() {
            return MmError::err(TendermintInitError {
                ticker,
                kind: TendermintInitErrorKind::EmptyRpcUrls,
            });
        }

        let priv_key = priv_key_policy.activated_key_or_err().mm_err(|e| TendermintInitError {
            ticker: ticker.clone(),
            kind: TendermintInitErrorKind::Internal(e.to_string()),
        })?;

        let account_id =
            account_id_from_privkey(priv_key.as_slice(), &protocol_info.account_prefix).mm_err(|kind| {
                TendermintInitError {
                    ticker: ticker.clone(),
                    kind,
                }
            })?;

        let rpc_clients = clients_from_urls(rpc_urls.as_ref()).mm_err(|kind| TendermintInitError {
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
            priv_key_policy,
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
        })))
    }

    pub fn ibc_withdraw(&self, req: IBCWithdrawRequest) -> WithdrawFut {
        let coin = self.clone();
        let fut = async move {
            let to_address =
                AccountId::from_str(&req.to).map_to_mm(|e| WithdrawError::InvalidAddress(e.to_string()))?;

            let (account_id, priv_key) = match req.from {
                Some(WithdrawFrom::HDWalletAddress(ref path_to_address)) => {
                    let priv_key = coin
                        .priv_key_policy
                        .hd_wallet_derived_priv_key_or_err(path_to_address)?;
                    let account_id = account_id_from_privkey(priv_key.as_slice(), &coin.account_prefix)
                        .map_err(|e| WithdrawError::InternalError(e.to_string()))?;
                    (account_id, priv_key)
                },
                Some(WithdrawFrom::AddressId(_)) | Some(WithdrawFrom::DerivationPath { .. }) => {
                    return MmError::err(WithdrawError::UnexpectedFromAddress(
                        "Withdraw from 'AddressId' or 'DerivationPath' is not supported yet for Tendermint!"
                            .to_string(),
                    ))
                },
                None => (coin.account_id.clone(), *coin.priv_key_policy.activated_key_or_err()?),
            };

            let (balance_denom, balance_dec) = coin
                .get_balance_as_unsigned_and_decimal(&account_id, &coin.denom, coin.decimals())
                .await?;

            // << BEGIN TX SIMULATION FOR FEE CALCULATION
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

            let memo = req.memo.unwrap_or_else(|| TX_DEFAULT_MEMO.into());

            let msg_transfer = MsgTransfer::new_with_default_timeout(
                req.ibc_source_channel.clone(),
                account_id.clone(),
                to_address.clone(),
                Coin {
                    denom: coin.denom.clone(),
                    amount: amount_denom.into(),
                },
            )
            .to_any()
            .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let current_block = coin
                .current_block()
                .compat()
                .await
                .map_to_mm(WithdrawError::Transport)?;

            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;
            // >> END TX SIMULATION FOR FEE CALCULATION

            let (_, gas_limit) = coin.gas_info_for_withdraw(&req.fee, IBC_GAS_LIMIT_DEFAULT);

            let fee_amount_u64 = coin
                .calculate_account_fee_amount_as_u64(
                    &account_id,
                    &priv_key,
                    msg_transfer.clone(),
                    timeout_height,
                    memo.clone(),
                    req.fee,
                )
                .await?;
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

            let msg_transfer = MsgTransfer::new_with_default_timeout(
                req.ibc_source_channel.clone(),
                account_id.clone(),
                to_address.clone(),
                Coin {
                    denom: coin.denom.clone(),
                    amount: amount_denom.into(),
                },
            )
            .to_any()
            .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let account_info = coin.account_info(&account_id).await?;
            let tx_raw = coin
                .any_to_signed_raw_tx(&priv_key, account_info, msg_transfer, fee, timeout_height, memo.clone())
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let tx_bytes = tx_raw
                .to_bytes()
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let hash = sha256(&tx_bytes);
            Ok(TransactionDetails {
                tx_hash: hex::encode_upper(hash.as_slice()),
                tx_hex: tx_bytes.into(),
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
                internal_id: hash.to_vec().into(),
                kmd_rewards: None,
                transaction_type: TransactionType::default(),
                memo: Some(memo),
            })
        };
        Box::new(fut.boxed().compat())
    }

    pub async fn get_ibc_transfer_channels(&self, req: IBCTransferChannelsRequest) -> IBCTransferChannelsResult {
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
            chain_1: ChannelInfo,
            #[allow(dead_code)]
            chain_2: ChannelInfo,
            ordering: String,
            version: String,
            tags: Option<IBCTransferChannelTag>,
        }

        let src_chain_registry_name = self.chain_registry_name.as_ref().or_mm_err(|| {
            IBCTransferChannelsRequestError::InternalError(format!(
                "`chain_registry_name` is not set for '{}'",
                self.platform_ticker()
            ))
        })?;

        let source_filename = format!(
            "{}-{}.json",
            src_chain_registry_name, req.destination_chain_registry_name
        );

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
            .retain(|ch| ch.chain_1.port_id == *IBC_OUT_SOURCE_PORT);

        let result: Vec<IBCTransferChannel> = registry_object
            .channels
            .iter()
            .map(|ch| IBCTransferChannel {
                channel_id: ch.chain_1.channel_id.clone(),
                ordering: ch.ordering.clone(),
                version: ch.version.clone(),
                tags: ch.tags.clone().map(|t| IBCTransferChannelTag {
                    status: t.status,
                    preferred: t.preferred,
                    dex: t.dex,
                }),
            })
            .collect();

        Ok(IBCTransferChannelsResponse {
            ibc_transfer_channels: result,
        })
    }

    #[inline(always)]
    fn gas_price(&self) -> f64 { self.gas_price.unwrap_or(DEFAULT_GAS_PRICE) }

    #[allow(unused)]
    async fn get_latest_block(&self) -> MmResult<GetLatestBlockResponse, TendermintCoinRpcError> {
        let path = AbciPath::from_str(ABCI_GET_LATEST_BLOCK_PATH).expect("valid path");

        let request = GetLatestBlockRequest {};
        let request = AbciRequest::new(
            Some(path),
            request.encode_to_vec(),
            ABCI_REQUEST_HEIGHT,
            ABCI_REQUEST_PROVE,
        );

        let response = self.rpc_client().await?.perform(request).await?;

        Ok(GetLatestBlockResponse::decode(response.response.value.as_slice())?)
    }

    #[allow(unused)]
    async fn get_block_by_height(&self, height: i64) -> MmResult<GetBlockByHeightResponse, TendermintCoinRpcError> {
        let path = AbciPath::from_str(ABCI_GET_BLOCK_BY_HEIGHT_PATH).expect("valid path");

        let request = GetBlockByHeightRequest { height };
        let request = AbciRequest::new(
            Some(path),
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
        account_info: BaseAccount,
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

        let signkey = SigningKey::from_bytes(priv_key.as_slice())?;
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
        amount: Vec<Coin>,
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

    pub(super) async fn seq_safe_send_raw_tx_bytes(
        &self,
        tx_payload: Any,
        fee: Fee,
        timeout_height: u64,
        memo: String,
    ) -> Result<(String, Raw), TransactionErr> {
        let (tx_id, tx_raw) = loop {
            let tx_raw = try_tx_s!(self.any_to_signed_raw_tx(
                try_tx_s!(self.priv_key_policy.activated_key_or_err()),
                try_tx_s!(self.account_info(&self.account_id).await),
                tx_payload.clone(),
                fee.clone(),
                timeout_height,
                memo.clone(),
            ));

            match self.send_raw_tx_bytes(&try_tx_s!(tx_raw.to_bytes())).compat().await {
                Ok(tx_id) => break (tx_id, tx_raw),
                Err(e) => {
                    if e.contains(ACCOUNT_SEQUENCE_ERR) {
                        debug!("Got wrong account sequence, trying again.");
                        continue;
                    }

                    return Err(crate::TransactionErr::Plain(ERRL!("{}", e)));
                },
            };
        };

        Ok((tx_id, tx_raw))
    }

    #[allow(deprecated)]
    pub(super) async fn calculate_fee(
        &self,
        msg: Any,
        timeout_height: u64,
        memo: String,
        withdraw_fee: Option<WithdrawFee>,
    ) -> MmResult<Fee, TendermintCoinRpcError> {
        let path = AbciPath::from_str(ABCI_SIMULATE_TX_PATH).expect("valid path");

        let (response, raw_response) = loop {
            let account_info = self.account_info(&self.account_id).await?;
            let activated_priv_key = self.priv_key_policy.activated_key_or_err()?;
            let tx_bytes = self
                .gen_simulated_tx(
                    account_info,
                    activated_priv_key,
                    msg.clone(),
                    timeout_height,
                    memo.clone(),
                )
                .map_to_mm(|e| TendermintCoinRpcError::InternalError(format!("{}", e)))?;

            let request = AbciRequest::new(
                Some(path.clone()),
                SimulateRequest { tx_bytes, tx: None }.encode_to_vec(),
                ABCI_REQUEST_HEIGHT,
                ABCI_REQUEST_PROVE,
            );

            let raw_response = self.rpc_client().await?.perform(request).await?;

            if raw_response.response.log.to_string().contains(ACCOUNT_SEQUENCE_ERR) {
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
        priv_key: &Secp256k1Secret,
        msg: Any,
        timeout_height: u64,
        memo: String,
        withdraw_fee: Option<WithdrawFee>,
    ) -> MmResult<u64, TendermintCoinRpcError> {
        let path = AbciPath::from_str(ABCI_SIMULATE_TX_PATH).expect("valid path");

        let (response, raw_response) = loop {
            let account_info = self.account_info(account_id).await?;
            let tx_bytes = self
                .gen_simulated_tx(account_info, priv_key, msg.clone(), timeout_height, memo.clone())
                .map_to_mm(|e| TendermintCoinRpcError::InternalError(format!("{}", e)))?;

            let request = AbciRequest::new(
                Some(path.clone()),
                SimulateRequest { tx_bytes, tx: None }.encode_to_vec(),
                ABCI_REQUEST_HEIGHT,
                ABCI_REQUEST_PROVE,
            );

            let raw_response = self.rpc_client().await?.perform(request).await?;

            if raw_response.response.log.to_string().contains(ACCOUNT_SEQUENCE_ERR) {
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
        let path = AbciPath::from_str(ABCI_QUERY_ACCOUNT_PATH).expect("valid path");
        let request = QueryAccountRequest {
            address: account_id.to_string(),
        };
        let request = AbciRequest::new(
            Some(path),
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
                    .or_mm_err(|| TendermintCoinRpcError::Prost(err))?
            },
            Err(err) => {
                return MmError::err(TendermintCoinRpcError::Prost(err));
            },
        };

        Ok(base_account)
    }

    pub(super) async fn account_balance_for_denom(
        &self,
        account_id: &AccountId,
        denom: String,
    ) -> MmResult<u64, TendermintCoinRpcError> {
        let path = AbciPath::from_str(ABCI_QUERY_BALANCE_PATH).expect("valid path");
        let request = QueryBalanceRequest {
            address: account_id.to_string(),
            denom,
        };
        let request = AbciRequest::new(
            Some(path),
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

    fn gen_create_htlc_tx(
        &self,
        denom: Denom,
        to: &AccountId,
        amount: cosmrs::Decimal,
        secret_hash: &[u8],
        time_lock: u64,
    ) -> MmResult<IrisHtlc, TxMarshalingErr> {
        let amount = vec![Coin { denom, amount }];
        let timestamp = 0_u64;
        let msg_payload = MsgCreateHtlc {
            sender: self.account_id.clone(),
            to: to.clone(),
            receiver_on_other_chain: "".to_string(),
            sender_on_other_chain: "".to_string(),
            amount: amount.clone(),
            hash_lock: hex::encode(secret_hash),
            timestamp,
            time_lock,
            transfer: false,
        };

        let htlc_id = self.calculate_htlc_id(&self.account_id, to, amount, secret_hash);

        Ok(IrisHtlc {
            id: htlc_id,
            msg_payload: msg_payload
                .to_any()
                .map_err(|e| MmError::new(TxMarshalingErr::InvalidInput(e.to_string())))?,
        })
    }

    fn gen_claim_htlc_tx(&self, htlc_id: String, secret: &[u8]) -> MmResult<IrisHtlc, TxMarshalingErr> {
        let msg_payload = MsgClaimHtlc {
            id: htlc_id.clone(),
            sender: self.account_id.clone(),
            secret: hex::encode(secret),
        };

        Ok(IrisHtlc {
            id: htlc_id,
            msg_payload: msg_payload
                .to_any()
                .map_err(|e| MmError::new(TxMarshalingErr::InvalidInput(e.to_string())))?,
        })
    }

    pub(super) fn any_to_signed_raw_tx(
        &self,
        priv_key: &Secp256k1Secret,
        account_info: BaseAccount,
        tx_payload: Any,
        fee: Fee,
        timeout_height: u64,
        memo: String,
    ) -> cosmrs::Result<Raw> {
        let signkey = SigningKey::from_bytes(priv_key.as_slice())?;
        let tx_body = tx::Body::new(vec![tx_payload], memo, timeout_height as u32);
        let auth_info = SignerInfo::single_direct(Some(signkey.public_key()), account_info.sequence).auth_info(fee);
        let sign_doc = SignDoc::new(&tx_body, &auth_info, &self.chain_id, account_info.account_number)?;
        sign_doc.sign(&signkey)
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

        let htlc_id = self.calculate_htlc_id(&self.account_id, &to_address, amount, secret_hash);

        let coin = self.clone();
        let fut = async move {
            let htlc_response = try_s!(coin.query_htlc(htlc_id.clone()).await);
            let htlc_data = match htlc_response.htlc {
                Some(htlc) => htlc,
                None => return Ok(None),
            };

            match htlc_data.state {
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

                let deserialized_tx = try_s!(cosmrs::Tx::from_bytes(tx.tx.as_bytes()));
                let msg = try_s!(deserialized_tx.body.messages.first().ok_or("Tx body couldn't be read."));
                let htlc = try_s!(CreateHtlcProtoRep::decode(msg.value.as_slice()));

                if htlc.hash_lock.to_uppercase() == htlc_data.hash_lock.to_uppercase() {
                    let htlc = TransactionEnum::CosmosTransaction(CosmosTransaction {
                        data: try_s!(TxRaw::decode(tx.tx.as_bytes())),
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
        let amount = cosmrs::Decimal::from(amount_as_u64);

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
                coin.seq_safe_send_raw_tx_bytes(
                    create_htlc_tx.msg_payload.clone(),
                    fee.clone(),
                    timeout_height,
                    TX_DEFAULT_MEMO.into(),
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
    ) -> TransactionFut {
        let memo = try_tx_fus!(Uuid::from_slice(uuid)).to_string();
        let from_address = self.account_id.clone();
        let pubkey_hash = dhash160(fee_addr);
        let to_address = try_tx_fus!(AccountId::new(&self.account_prefix, pubkey_hash.as_slice()));

        let amount_as_u64 = try_tx_fus!(sat_from_big_decimal(&amount, decimals));
        let amount = cosmrs::Decimal::from(amount_as_u64);

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

            let (_tx_id, tx_raw) = try_tx_s!(
                coin.seq_safe_send_raw_tx_bytes(tx_payload.clone(), fee.clone(), timeout_height, memo.clone())
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

    pub(super) fn validate_payment_for_denom(
        &self,
        input: ValidatePaymentInput,
        denom: Denom,
        decimals: u8,
    ) -> ValidatePaymentFut<()> {
        let coin = self.clone();
        let fut = async move {
            let tx = cosmrs::Tx::from_bytes(&input.payment_tx)
                .map_to_mm(|e| ValidatePaymentError::TxDeserializationError(e.to_string()))?;

            if tx.body.messages.len() != 1 {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(
                    "Payment tx must have exactly one message".into(),
                ));
            }

            let create_htlc_msg_proto = CreateHtlcProtoRep::decode(tx.body.messages[0].value.as_slice())
                .map_to_mm(|e| ValidatePaymentError::WrongPaymentTx(e.to_string()))?;
            let create_htlc_msg = MsgCreateHtlc::try_from(create_htlc_msg_proto)
                .map_to_mm(|e| ValidatePaymentError::WrongPaymentTx(e.to_string()))?;

            let sender_pubkey_hash = dhash160(&input.other_pub);
            let sender = AccountId::new(&coin.account_prefix, sender_pubkey_hash.as_slice())
                .map_to_mm(|e| ValidatePaymentError::InvalidParameter(e.to_string()))?;

            let amount = sat_from_big_decimal(&input.amount, decimals)?;
            let amount = vec![Coin {
                denom,
                amount: amount.into(),
            }];

            let time_lock = coin.estimate_blocks_from_duration(input.time_lock_duration);

            let expected_msg = MsgCreateHtlc {
                sender: sender.clone(),
                to: coin.account_id.clone(),
                receiver_on_other_chain: "".into(),
                sender_on_other_chain: "".into(),
                amount: amount.clone(),
                hash_lock: hex::encode(&input.secret_hash),
                timestamp: 0,
                time_lock: time_lock as u64,
                transfer: false,
            };

            if create_htlc_msg != expected_msg {
                return MmError::err(ValidatePaymentError::WrongPaymentTx(format!(
                    "Incorrect CreateHtlc message {:?}, expected {:?}",
                    create_htlc_msg, expected_msg
                )));
            }

            let hash = hex::encode_upper(sha256(&input.payment_tx).as_slice());
            let tx_from_rpc = coin.request_tx(hash).await?;
            if input.payment_tx != tx_from_rpc.encode_to_vec() {
                return MmError::err(ValidatePaymentError::InvalidRpcResponse(
                    "Tx from RPC doesn't match the input".into(),
                ));
            }

            let htlc_id = coin.calculate_htlc_id(&sender, &coin.account_id, amount, &input.secret_hash);

            let htlc_response = coin.query_htlc(htlc_id.clone()).await?;
            let htlc_data = htlc_response
                .htlc
                .or_mm_err(|| ValidatePaymentError::InvalidRpcResponse(format!("No HTLC data for {}", htlc_id)))?;

            match htlc_data.state {
                HTLC_STATE_OPEN => Ok(()),
                unexpected_state => MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
                    "{}",
                    unexpected_state
                ))),
            }
        };
        Box::new(fut.boxed().compat())
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
            .map_err(|e| MmError::new(TradePreimageError::InternalError(e.into_inner().to_string())))?;

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
                self.priv_key_policy
                    .activated_key_or_err()
                    .mm_err(|e| TradePreimageError::InternalError(e.to_string()))?,
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
        dex_fee_amount: BigDecimal,
    ) -> TradePreimageResult<TradeFee> {
        let to_address = account_id_from_pubkey_hex(&self.account_prefix, DEX_FEE_ADDR_PUBKEY)
            .map_err(|e| MmError::new(TradePreimageError::InternalError(e.into_inner().to_string())))?;
        let amount = sat_from_big_decimal(&dex_fee_amount, decimals)?;

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
                self.priv_key_policy
                    .activated_key_or_err()
                    .mm_err(|e| TradePreimageError::InternalError(e.to_string()))?,
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
        let path = AbciPath::from_str(ABCI_GET_TX_PATH).expect("valid path");
        let request = GetTxRequest { hash };
        let response = self
            .rpc_client()
            .await?
            .abci_query(
                Some(path),
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
        let path = AbciPath::from_str(ABCI_GET_TX_PATH).expect("valid path");
        let request = GetTxRequest { hash };
        let response = self
            .rpc_client()
            .await?
            .abci_query(
                Some(path),
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
                err_code => Ok(Some(cosmrs::tendermint::abci::Code::Err(err_code))),
            }
        } else {
            Ok(None)
        }
    }

    pub(crate) async fn query_htlc(&self, id: String) -> MmResult<QueryHtlcResponseProto, TendermintCoinRpcError> {
        let path = AbciPath::from_str(ABCI_QUERY_HTLC_PATH).expect("valid path");
        let request = QueryHtlcRequestProto { id };
        let response = self
            .rpc_client()
            .await?
            .abci_query(
                Some(path),
                request.encode_to_vec(),
                ABCI_REQUEST_HEIGHT,
                ABCI_REQUEST_PROVE,
            )
            .await?;

        Ok(QueryHtlcResponseProto::decode(response.value.as_slice())?)
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
        let htlc_proto = CreateHtlcProtoRep::decode(first_message.value.as_slice())?;
        let htlc = MsgCreateHtlc::try_from(htlc_proto)?;
        let htlc_id = self.calculate_htlc_id(&htlc.sender, &htlc.to, htlc.amount, input.secret_hash);

        let htlc_response = self.query_htlc(htlc_id.clone()).await?;
        let htlc_data = match htlc_response.htlc {
            Some(htlc) => htlc,
            None => return Ok(None),
        };

        match htlc_data.state {
            HTLC_STATE_OPEN => Ok(None),
            HTLC_STATE_COMPLETED => {
                let events_string = format!("claim_htlc.id='{}'", htlc_id);
                let request = GetTxsEventRequest {
                    events: vec![events_string],
                    pagination: None,
                    order_by: TendermintResultOrder::Ascending as i32,
                };
                let encoded_request = request.encode_to_vec();

                let path = AbciPath::from_str(ABCI_GET_TXS_EVENT_PATH).expect("valid path");
                let response = self
                    .rpc_client()
                    .await?
                    .abci_query(
                        Some(path),
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
}

fn clients_from_urls(rpc_urls: &[String]) -> MmResult<Vec<HttpClient>, TendermintInitErrorKind> {
    if rpc_urls.is_empty() {
        return MmError::err(TendermintInitErrorKind::EmptyRpcUrls);
    }
    let mut clients = Vec::new();
    let mut errors = Vec::new();
    // check that all urls are valid
    // keep all invalid urls in one vector to show all of them in error
    for url in rpc_urls.iter() {
        match HttpClient::new(url.as_str()) {
            Ok(client) => clients.push(client),
            Err(e) => errors.push(format!("Url {} is invalid, got error {}", url, e)),
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

    fn spawner(&self) -> CoinFutSpawner { CoinFutSpawner::new(&self.abortable_system) }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut {
        let coin = self.clone();
        let fut = async move {
            let to_address =
                AccountId::from_str(&req.to).map_to_mm(|e| WithdrawError::InvalidAddress(e.to_string()))?;
            if to_address.prefix() != coin.account_prefix {
                return MmError::err(WithdrawError::InvalidAddress(format!(
                    "expected {} address prefix",
                    coin.account_prefix
                )));
            }

            let (account_id, priv_key) = match req.from {
                Some(WithdrawFrom::HDWalletAddress(ref path_to_address)) => {
                    let priv_key = coin
                        .priv_key_policy
                        .hd_wallet_derived_priv_key_or_err(path_to_address)?;
                    let account_id = account_id_from_privkey(priv_key.as_slice(), &coin.account_prefix)
                        .map_err(|e| WithdrawError::InternalError(e.to_string()))?;
                    (account_id, priv_key)
                },
                Some(WithdrawFrom::AddressId(_)) | Some(WithdrawFrom::DerivationPath { .. }) => {
                    return MmError::err(WithdrawError::UnexpectedFromAddress(
                        "Withdraw from 'AddressId' or 'DerivationPath' is not supported yet for Tendermint!"
                            .to_string(),
                    ))
                },
                None => (coin.account_id.clone(), *coin.priv_key_policy.activated_key_or_err()?),
            };

            let (balance_denom, balance_dec) = coin
                .get_balance_as_unsigned_and_decimal(&account_id, &coin.denom, coin.decimals())
                .await?;

            // << BEGIN TX SIMULATION FOR FEE CALCULATION
            let (amount_denom, amount_dec) = if req.max {
                let amount_denom = balance_denom;
                (amount_denom, big_decimal_from_sat_unsigned(amount_denom, coin.decimals))
            } else {
                let total = req.amount.clone();

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

            let msg_send = MsgSend {
                from_address: account_id.clone(),
                to_address: to_address.clone(),
                amount: vec![Coin {
                    denom: coin.denom.clone(),
                    amount: amount_denom.into(),
                }],
            }
            .to_any()
            .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let memo = req.memo.unwrap_or_else(|| TX_DEFAULT_MEMO.into());
            let current_block = coin
                .current_block()
                .compat()
                .await
                .map_to_mm(WithdrawError::Transport)?;

            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;
            // >> END TX SIMULATION FOR FEE CALCULATION

            let (_, gas_limit) = coin.gas_info_for_withdraw(&req.fee, GAS_LIMIT_DEFAULT);

            let fee_amount_u64 = coin
                .calculate_account_fee_amount_as_u64(
                    &account_id,
                    &priv_key,
                    msg_send,
                    timeout_height,
                    memo.clone(),
                    req.fee,
                )
                .await?;
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

            let msg_send = MsgSend {
                from_address: account_id.clone(),
                to_address,
                amount: vec![Coin {
                    denom: coin.denom.clone(),
                    amount: amount_denom.into(),
                }],
            }
            .to_any()
            .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let account_info = coin.account_info(&account_id).await?;
            let tx_raw = coin
                .any_to_signed_raw_tx(&priv_key, account_info, msg_send, fee, timeout_height, memo.clone())
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let tx_bytes = tx_raw
                .to_bytes()
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let hash = sha256(&tx_bytes);

            Ok(TransactionDetails {
                tx_hash: hex::encode_upper(hash.as_slice()),
                tx_hex: tx_bytes.into(),
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
                internal_id: hash.to_vec().into(),
                kmd_rewards: None,
                transaction_type: TransactionType::default(),
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
            Ok(account) if account.prefix() != self.account_prefix => ValidateAddressResult {
                is_valid: false,
                reason: Some(format!(
                    "Expected {} account prefix, got {}",
                    self.account_prefix,
                    account.prefix()
                )),
            },
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
        dex_fee_amount: BigDecimal,
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

impl MarketCoinOps for TendermintCoin {
    fn ticker(&self) -> &str { &self.ticker }

    fn my_address(&self) -> MmResult<String, MyAddressError> { Ok(self.account_id.to_string()) }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> {
        let key = SigningKey::from_bytes(self.priv_key_policy.activated_key_or_err()?.as_slice())
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
            let broadcast_res = try_s!(
                try_s!(coin.rpc_client().await)
                    .broadcast_tx_commit(tx_bytes.into())
                    .await
            );

            if broadcast_res.check_tx.log.to_string().contains(ACCOUNT_SEQUENCE_ERR)
                || broadcast_res.deliver_tx.log.to_string().contains(ACCOUNT_SEQUENCE_ERR)
            {
                return ERR!(
                    "{}. check_tx log: {}, deliver_tx log: {}",
                    ACCOUNT_SEQUENCE_ERR,
                    broadcast_res.check_tx.log,
                    broadcast_res.deliver_tx.log
                );
            }

            if !broadcast_res.check_tx.code.is_ok() {
                return ERR!("Tx check failed {:?}", broadcast_res.check_tx);
            }

            if !broadcast_res.deliver_tx.code.is_ok() {
                return ERR!("Tx deliver failed {:?}", broadcast_res.deliver_tx);
            }
            Ok(broadcast_res.hash.to_string())
        };
        Box::new(fut.boxed().compat())
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

    fn wait_for_htlc_tx_spend(&self, args: WaitForHTLCTxSpendArgs<'_>) -> TransactionFut {
        let tx = try_tx_fus!(cosmrs::Tx::from_bytes(args.tx_bytes));
        let first_message = try_tx_fus!(tx.body.messages.first().ok_or("Tx body couldn't be read."));
        let htlc_proto = try_tx_fus!(CreateHtlcProtoRep::decode(first_message.value.as_slice()));
        let htlc = try_tx_fus!(MsgCreateHtlc::try_from(htlc_proto));
        let htlc_id = self.calculate_htlc_id(&htlc.sender, &htlc.to, htlc.amount, args.secret_hash);

        let events_string = format!("claim_htlc.id='{}'", htlc_id);
        let request = GetTxsEventRequest {
            events: vec![events_string],
            pagination: None,
            order_by: TendermintResultOrder::Ascending as i32,
        };
        let encoded_request = request.encode_to_vec();

        let coin = self.clone();
        let path = try_tx_fus!(AbciPath::from_str(ABCI_GET_TXS_EVENT_PATH));
        let wait_until = args.wait_until;
        let fut = async move {
            loop {
                let response = try_tx_s!(
                    try_tx_s!(coin.rpc_client().await)
                        .abci_query(
                            Some(path.clone()),
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
                if get_utc_timestamp() > wait_until as i64 {
                    return Err(TransactionErr::Plain("Waited too long".into()));
                }
            }
        };

        Box::new(fut.boxed().compat())
    }

    fn tx_enum_from_bytes(&self, bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        let tx_raw: TxRaw = Message::decode(bytes).map_to_mm(|e| TxMarshalingErr::InvalidInput(e.to_string()))?;
        Ok(TransactionEnum::CosmosTransaction(CosmosTransaction { data: tx_raw }))
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        let coin = self.clone();
        let fut = async move {
            let info = try_s!(try_s!(coin.rpc_client().await).abci_info().await);
            Ok(info.last_block_height.into())
        };
        Box::new(fut.boxed().compat())
    }

    fn display_priv_key(&self) -> Result<String, String> {
        Ok(self
            .priv_key_policy
            .activated_key_or_err()
            .map_err(|e| e.to_string())?
            .to_string())
    }

    #[inline]
    fn min_tx_amount(&self) -> BigDecimal { big_decimal_from_sat(MIN_TX_SATOSHIS, self.decimals) }

    #[inline]
    fn min_trading_vol(&self) -> MmNumber { self.min_tx_amount().into() }
}

#[async_trait]
#[allow(unused_variables)]
impl SwapOps for TendermintCoin {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, uuid: &[u8]) -> TransactionFut {
        self.send_taker_fee_for_denom(fee_addr, amount, self.denom.clone(), self.decimals, uuid)
    }

    fn send_maker_payment(&self, maker_payment_args: SendPaymentArgs) -> TransactionFut {
        self.send_htlc_for_denom(
            maker_payment_args.time_lock_duration,
            maker_payment_args.other_pubkey,
            maker_payment_args.secret_hash,
            maker_payment_args.amount,
            self.denom.clone(),
            self.decimals,
        )
    }

    fn send_taker_payment(&self, taker_payment_args: SendPaymentArgs) -> TransactionFut {
        self.send_htlc_for_denom(
            taker_payment_args.time_lock_duration,
            taker_payment_args.other_pubkey,
            taker_payment_args.secret_hash,
            taker_payment_args.amount,
            self.denom.clone(),
            self.decimals,
        )
    }

    fn send_maker_spends_taker_payment(&self, maker_spends_payment_args: SpendPaymentArgs) -> TransactionFut {
        let tx = try_tx_fus!(cosmrs::Tx::from_bytes(maker_spends_payment_args.other_payment_tx));
        let msg = try_tx_fus!(tx.body.messages.first().ok_or("Tx body couldn't be read."));
        let htlc_proto: CreateHtlcProtoRep = try_tx_fus!(prost::Message::decode(msg.value.as_slice()));
        let htlc = try_tx_fus!(MsgCreateHtlc::try_from(htlc_proto));

        let mut amount = htlc.amount.clone();
        amount.sort();
        drop_mutability!(amount);

        let coins_string = amount
            .iter()
            .map(|t| format!("{}{}", t.amount, t.denom))
            .collect::<Vec<String>>()
            .join(",");

        let htlc_id = self.calculate_htlc_id(&htlc.sender, &htlc.to, amount, maker_spends_payment_args.secret_hash);

        let claim_htlc_tx = try_tx_fus!(self.gen_claim_htlc_tx(htlc_id, maker_spends_payment_args.secret));
        let coin = self.clone();

        let fut = async move {
            let current_block = try_tx_s!(coin.current_block().compat().await);
            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

            let fee = try_tx_s!(
                coin.calculate_fee(
                    claim_htlc_tx.msg_payload.clone(),
                    timeout_height,
                    TX_DEFAULT_MEMO.to_owned(),
                    None
                )
                .await
            );

            let (_tx_id, tx_raw) = try_tx_s!(
                coin.seq_safe_send_raw_tx_bytes(
                    claim_htlc_tx.msg_payload.clone(),
                    fee.clone(),
                    timeout_height,
                    TX_DEFAULT_MEMO.into(),
                )
                .await
            );

            Ok(TransactionEnum::CosmosTransaction(CosmosTransaction {
                data: tx_raw.into(),
            }))
        };

        Box::new(fut.boxed().compat())
    }

    fn send_taker_spends_maker_payment(&self, taker_spends_payment_args: SpendPaymentArgs) -> TransactionFut {
        let tx = try_tx_fus!(cosmrs::Tx::from_bytes(taker_spends_payment_args.other_payment_tx));
        let msg = try_tx_fus!(tx.body.messages.first().ok_or("Tx body couldn't be read."));
        let htlc_proto: CreateHtlcProtoRep = try_tx_fus!(prost::Message::decode(msg.value.as_slice()));
        let htlc = try_tx_fus!(MsgCreateHtlc::try_from(htlc_proto));

        let mut amount = htlc.amount.clone();
        amount.sort();
        drop_mutability!(amount);

        let coins_string = amount
            .iter()
            .map(|t| format!("{}{}", t.amount, t.denom))
            .collect::<Vec<String>>()
            .join(",");

        let htlc_id = self.calculate_htlc_id(&htlc.sender, &htlc.to, amount, taker_spends_payment_args.secret_hash);

        let claim_htlc_tx = try_tx_fus!(self.gen_claim_htlc_tx(htlc_id, taker_spends_payment_args.secret));
        let coin = self.clone();

        let fut = async move {
            let current_block = try_tx_s!(coin.current_block().compat().await);
            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

            let fee = try_tx_s!(
                coin.calculate_fee(
                    claim_htlc_tx.msg_payload.clone(),
                    timeout_height,
                    TX_DEFAULT_MEMO.into(),
                    None
                )
                .await
            );

            let (tx_id, tx_raw) = try_tx_s!(
                coin.seq_safe_send_raw_tx_bytes(
                    claim_htlc_tx.msg_payload.clone(),
                    fee.clone(),
                    timeout_height,
                    TX_DEFAULT_MEMO.into(),
                )
                .await
            );

            Ok(TransactionEnum::CosmosTransaction(CosmosTransaction {
                data: tx_raw.into(),
            }))
        };

        Box::new(fut.boxed().compat())
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

    fn validate_fee(&self, validate_fee_args: ValidateFeeArgs) -> ValidatePaymentFut<()> {
        self.validate_fee_for_denom(
            validate_fee_args.fee_tx,
            validate_fee_args.expected_sender,
            validate_fee_args.fee_addr,
            validate_fee_args.amount,
            self.decimals,
            validate_fee_args.uuid,
            self.denom.to_string(),
        )
    }

    fn validate_maker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        self.validate_payment_for_denom(input, self.denom.clone(), self.decimals)
    }

    fn validate_taker_payment(&self, input: ValidatePaymentInput) -> ValidatePaymentFut<()> {
        self.validate_payment_for_denom(input, self.denom.clone(), self.decimals)
    }

    fn check_if_my_payment_sent(
        &self,
        if_my_payment_sent_args: CheckIfMyPaymentSentArgs,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        self.check_if_my_payment_sent_for_denom(
            self.decimals,
            self.denom.clone(),
            if_my_payment_sent_args.other_pub,
            if_my_payment_sent_args.secret_hash,
            if_my_payment_sent_args.amount,
        )
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
        let htlc_proto: super::iris::htlc_proto::ClaimHtlcProtoRep =
            try_s!(prost::Message::decode(msg.value.as_slice()));
        let htlc = try_s!(MsgClaimHtlc::try_from(htlc_proto));

        Ok(try_s!(hex::decode(htlc.secret)))
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
    fn derive_htlc_key_pair(&self, swap_unique_data: &[u8]) -> KeyPair {
        key_pair_from_secret(
            self.priv_key_policy
                .activated_key_or_err()
                .expect("valid priv key")
                .as_ref(),
        )
        .expect("valid priv key")
    }

    #[inline]
    fn derive_htlc_pubkey(&self, swap_unique_data: &[u8]) -> Vec<u8> {
        self.derive_htlc_key_pair(swap_unique_data).public_slice().to_vec()
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
    path_to_address: StandardHDCoinAddress,
) -> MmResult<TendermintPrivKeyPolicy, TendermintInitError> {
    match priv_key_build_policy {
        PrivKeyBuildPolicy::IguanaPrivKey(iguana) => Ok(TendermintPrivKeyPolicy::Iguana(iguana)),
        PrivKeyBuildPolicy::GlobalHDAccount(global_hd) => {
            let derivation_path = conf.derivation_path.as_ref().or_mm_err(|| TendermintInitError {
                ticker: ticker.to_string(),
                kind: TendermintInitErrorKind::DerivationPathIsNotSet,
            })?;
            let activated_priv_key = global_hd
                .derive_secp256k1_secret(derivation_path, &path_to_address)
                .mm_err(|e| TendermintInitError {
                    ticker: ticker.to_string(),
                    kind: TendermintInitErrorKind::InvalidPrivKey(e.to_string()),
                })?;
            let bip39_secp_priv_key = global_hd.root_priv_key().clone();
            Ok(TendermintPrivKeyPolicy::HDWallet {
                derivation_path: derivation_path.clone(),
                activated_key: activated_priv_key,
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

#[cfg(test)]
pub mod tendermint_coin_tests {
    use super::*;

    use common::{block_on, wait_until_ms, DEX_FEE_ADDR_RAW_PUBKEY};
    use cosmrs::proto::cosmos::tx::v1beta1::{GetTxRequest, GetTxResponse, GetTxsEventResponse};
    use crypto::privkey::key_pair_from_seed;
    use std::mem::discriminant;

    pub const IRIS_TESTNET_HTLC_PAIR1_SEED: &str = "iris test seed";
    // pub const IRIS_TESTNET_HTLC_PAIR1_PUB_KEY: &str = &[
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
        let rpc_urls = vec![IRIS_TESTNET_RPC_URL.to_string()];

        let protocol_conf = get_iris_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let priv_key_policy = TendermintPrivKeyPolicy::Iguana(key_pair.private().secret);

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "IRIS".to_string(),
            conf,
            protocol_conf,
            rpc_urls,
            false,
            priv_key_policy,
        ))
        .unwrap();

        // << BEGIN HTLC CREATION
        let to: AccountId = IRIS_TESTNET_HTLC_PAIR2_ADDRESS.parse().unwrap();
        const UAMOUNT: u64 = 1;
        let amount: cosmrs::Decimal = UAMOUNT.into();
        let amount_dec = big_decimal_from_sat_unsigned(UAMOUNT, coin.decimals);

        let mut sec = [0u8; 32];
        common::os_rng(&mut sec).unwrap();
        drop_mutability!(sec);

        let time_lock = 1000;

        let create_htlc_tx = coin
            .gen_create_htlc_tx(coin.denom.clone(), &to, amount, sha256(&sec).as_slice(), time_lock)
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

        let send_tx_fut = coin.seq_safe_send_raw_tx_bytes(
            create_htlc_tx.msg_payload.clone(),
            fee,
            timeout_height,
            TX_DEFAULT_MEMO.into(),
        );
        block_on(async {
            send_tx_fut.await.unwrap();
        });
        // >> END HTLC CREATION

        let htlc_spent = block_on(
            coin.check_if_my_payment_sent(CheckIfMyPaymentSentArgs {
                time_lock: 0,
                other_pub: IRIS_TESTNET_HTLC_PAIR2_PUB_KEY,
                secret_hash: sha256(&sec).as_slice(),
                search_from_block: current_block,
                swap_contract_address: &None,
                swap_unique_data: &[],
                amount: &amount_dec,
                payment_instructions: &None,
            })
            .compat(),
        )
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

        let send_tx_fut =
            coin.seq_safe_send_raw_tx_bytes(claim_htlc_tx.msg_payload, fee, timeout_height, TX_DEFAULT_MEMO.into());

        let (tx_id, _tx_raw) = block_on(async { send_tx_fut.await.unwrap() });

        println!("Claim HTLC tx hash {}", tx_id);
        // >> END HTLC CLAIMING
    }

    #[test]
    fn try_query_claim_htlc_txs_and_get_secret() {
        let rpc_urls = vec![IRIS_TESTNET_RPC_URL.to_string()];

        let protocol_conf = get_iris_usdc_ibc_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let priv_key_policy = TendermintPrivKeyPolicy::Iguana(key_pair.private().secret);

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "USDC-IBC".to_string(),
            conf,
            protocol_conf,
            rpc_urls,
            false,
            priv_key_policy,
        ))
        .unwrap();

        let events = "claim_htlc.id='2B925FC83A106CC81590B3DB108AC2AE496FFA912F368FE5E29BC1ED2B754F2C'";
        let request = GetTxsEventRequest {
            events: vec![events.into()],
            pagination: None,
            order_by: TendermintResultOrder::Ascending as i32,
        };
        let path = AbciPath::from_str(ABCI_GET_TXS_EVENT_PATH).unwrap();
        let response = block_on(block_on(coin.rpc_client()).unwrap().abci_query(
            Some(path),
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

        let claim_htlc =
            crate::tendermint::iris::htlc_proto::ClaimHtlcProtoRep::decode(first_msg.value.as_slice()).unwrap();
        let expected_secret = [1; 32];
        let actual_secret = hex::decode(claim_htlc.secret).unwrap();

        assert_eq!(actual_secret, expected_secret);
    }

    #[test]
    fn wait_for_tx_spend_test() {
        let rpc_urls = vec![IRIS_TESTNET_RPC_URL.to_string()];

        let protocol_conf = get_iris_usdc_ibc_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let priv_key_policy = TendermintPrivKeyPolicy::Iguana(key_pair.private().secret);

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "USDC-IBC".to_string(),
            conf,
            protocol_conf,
            rpc_urls,
            false,
            priv_key_policy,
        ))
        .unwrap();

        // https://nyancat.iobscan.io/#/tx?txHash=2DB382CE3D9953E4A94957B475B0E8A98F5B6DDB32D6BF0F6A765D949CF4A727
        let create_tx_hash = "2DB382CE3D9953E4A94957B475B0E8A98F5B6DDB32D6BF0F6A765D949CF4A727";

        let request = GetTxRequest {
            hash: create_tx_hash.into(),
        };

        let path = AbciPath::from_str(ABCI_GET_TX_PATH).unwrap();
        let response = block_on(block_on(coin.rpc_client()).unwrap().abci_query(
            Some(path),
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
        let spend_tx = block_on(
            coin.wait_for_htlc_tx_spend(WaitForHTLCTxSpendArgs {
                tx_bytes: &encoded_tx,
                secret_hash: &secret_hash,
                wait_until: get_utc_timestamp() as u64,
                from_block: 0,
                swap_contract_address: &None,
                check_every: TAKER_PAYMENT_SPEND_SEARCH_INTERVAL,
                watcher_reward: false,
            })
            .compat(),
        )
        .unwrap();

        // https://nyancat.iobscan.io/#/tx?txHash=565C820C1F95556ADC251F16244AAD4E4274772F41BC13F958C9C2F89A14D137
        let expected_spend_hash = "565C820C1F95556ADC251F16244AAD4E4274772F41BC13F958C9C2F89A14D137";
        let hash = spend_tx.tx_hash();
        assert_eq!(hex::encode_upper(hash.0), expected_spend_hash);
    }

    #[test]
    fn validate_taker_fee_test() {
        let rpc_urls = vec![IRIS_TESTNET_RPC_URL.to_string()];

        let protocol_conf = get_iris_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let priv_key_policy = TendermintPrivKeyPolicy::Iguana(key_pair.private().secret);

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "IRIS-TEST".to_string(),
            conf,
            protocol_conf,
            rpc_urls,
            false,
            priv_key_policy,
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

        let invalid_amount = 1.into();
        let error = coin
            .validate_fee(ValidateFeeArgs {
                fee_tx: &create_htlc_tx,
                expected_sender: &[],
                fee_addr: &DEX_FEE_ADDR_RAW_PUBKEY,
                amount: &invalid_amount,
                min_block_number: 0,
                uuid: &[1; 16],
            })
            .wait()
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

        let error = coin
            .validate_fee(ValidateFeeArgs {
                fee_tx: &random_transfer_tx,
                expected_sender: &[],
                fee_addr: &DEX_FEE_ADDR_RAW_PUBKEY,
                amount: &invalid_amount,
                min_block_number: 0,
                uuid: &[1; 16],
            })
            .wait()
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

        let error = coin
            .validate_fee(ValidateFeeArgs {
                fee_tx: &dex_fee_tx,
                expected_sender: &[],
                fee_addr: &DEX_FEE_ADDR_RAW_PUBKEY,
                amount: &invalid_amount,
                min_block_number: 0,
                uuid: &[1; 16],
            })
            .wait()
            .unwrap_err()
            .into_inner();
        println!("{}", error);
        match error {
            ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("Invalid amount")),
            _ => panic!("Expected `WrongPaymentTx` invalid amount, found {:?}", error),
        }

        let valid_amount: BigDecimal = "0.0001".parse().unwrap();
        // valid amount but invalid sender
        let error = coin
            .validate_fee(ValidateFeeArgs {
                fee_tx: &dex_fee_tx,
                expected_sender: &DEX_FEE_ADDR_RAW_PUBKEY,
                fee_addr: &DEX_FEE_ADDR_RAW_PUBKEY,
                amount: &valid_amount,
                min_block_number: 0,
                uuid: &[1; 16],
            })
            .wait()
            .unwrap_err()
            .into_inner();
        println!("{}", error);
        match error {
            ValidatePaymentError::WrongPaymentTx(err) => assert!(err.contains("Invalid sender")),
            _ => panic!("Expected `WrongPaymentTx` invalid sender, found {:?}", error),
        }

        // invalid memo
        let error = coin
            .validate_fee(ValidateFeeArgs {
                fee_tx: &dex_fee_tx,
                expected_sender: &pubkey,
                fee_addr: &DEX_FEE_ADDR_RAW_PUBKEY,
                amount: &valid_amount,
                min_block_number: 0,
                uuid: &[1; 16],
            })
            .wait()
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
        let rpc_urls = vec![IRIS_TESTNET_RPC_URL.to_string()];

        let protocol_conf = get_iris_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let priv_key_policy = TendermintPrivKeyPolicy::Iguana(key_pair.private().secret);

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "IRIS-TEST".to_string(),
            conf,
            protocol_conf,
            rpc_urls,
            false,
            priv_key_policy,
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
        let validate_err = coin.validate_taker_payment(input).wait().unwrap_err();
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
        let validate_err = block_on(
            coin.validate_payment_for_denom(input, "nim".parse().unwrap(), 6)
                .compat(),
        )
        .unwrap_err();
        match validate_err.into_inner() {
            ValidatePaymentError::UnexpectedPaymentState(_) => (),
            unexpected => panic!("Unexpected error variant {:?}", unexpected),
        };
    }

    #[test]
    fn test_search_for_swap_tx_spend_spent() {
        let rpc_urls = vec![IRIS_TESTNET_RPC_URL.to_string()];

        let protocol_conf = get_iris_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let priv_key_policy = TendermintPrivKeyPolicy::Iguana(key_pair.private().secret);

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "IRIS-TEST".to_string(),
            conf,
            protocol_conf,
            rpc_urls,
            false,
            priv_key_policy,
        ))
        .unwrap();

        // https://nyancat.iobscan.io/#/tx?txHash=2DB382CE3D9953E4A94957B475B0E8A98F5B6DDB32D6BF0F6A765D949CF4A727
        let create_tx_hash = "2DB382CE3D9953E4A94957B475B0E8A98F5B6DDB32D6BF0F6A765D949CF4A727";

        let request = GetTxRequest {
            hash: create_tx_hash.into(),
        };

        let path = AbciPath::from_str(ABCI_GET_TX_PATH).unwrap();
        let response = block_on(block_on(coin.rpc_client()).unwrap().abci_query(
            Some(path),
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
        let hash = spend_tx.tx_hash();
        assert_eq!(hex::encode_upper(hash.0), expected_spend_hash);
    }

    #[test]
    fn test_search_for_swap_tx_spend_refunded() {
        let rpc_urls = vec![IRIS_TESTNET_RPC_URL.to_string()];

        let protocol_conf = get_iris_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let priv_key_policy = TendermintPrivKeyPolicy::Iguana(key_pair.private().secret);

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "IRIS-TEST".to_string(),
            conf,
            protocol_conf,
            rpc_urls,
            false,
            priv_key_policy,
        ))
        .unwrap();

        // https://nyancat.iobscan.io/#/tx?txHash=BD1A76F43E8E2C7A1104EE363D63455CD50C76F2BFE93B703235F0A973061297
        let create_tx_hash = "BD1A76F43E8E2C7A1104EE363D63455CD50C76F2BFE93B703235F0A973061297";

        let request = GetTxRequest {
            hash: create_tx_hash.into(),
        };

        let path = AbciPath::from_str(ABCI_GET_TX_PATH).unwrap();
        let response = block_on(block_on(coin.rpc_client()).unwrap().abci_query(
            Some(path),
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
        let rpc_urls = vec![IRIS_TESTNET_RPC_URL.to_string()];
        let protocol_conf = get_iris_usdc_ibc_protocol();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();
        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let priv_key_policy = TendermintPrivKeyPolicy::Iguana(key_pair.private().secret);

        let coin = common::block_on(TendermintCoin::init(
            &ctx,
            "USDC-IBC".to_string(),
            conf,
            protocol_conf,
            rpc_urls,
            false,
            priv_key_policy,
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
                discriminant(&cosmrs::tendermint::abci::Code::Err(61))
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

        let rpc_urls = vec![IRIS_TESTNET_RPC_URL.to_string()];
        let protocol_conf = get_iris_usdc_ibc_protocol();

        let conf = TendermintConf {
            avg_blocktime: AVG_BLOCKTIME,
            derivation_path: None,
        };

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default().into_mm_arc();
        let key_pair = key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap();
        let priv_key_policy = TendermintPrivKeyPolicy::Iguana(key_pair.private().secret);

        let coin = common::block_on(TendermintCoin::init(
            &ctx,
            "USDC-IBC".to_string(),
            conf,
            protocol_conf,
            rpc_urls,
            false,
            priv_key_policy,
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
}
