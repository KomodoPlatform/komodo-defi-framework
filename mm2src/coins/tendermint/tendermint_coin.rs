use super::htlc::{IrisHtlc, MsgCreateHtlc};
#[cfg(not(target_arch = "wasm32"))]
use super::tendermint_native_rpc::*;
#[cfg(target_arch = "wasm32")] use super::tendermint_wasm_rpc::*;
use crate::coin_errors::{MyAddressError, ValidatePaymentError};
use crate::tendermint::htlc::MsgClaimHtlc;
use crate::tendermint::htlc_proto::{CreateHtlcProtoRep, QueryHtlcRequestProto, QueryHtlcResponseProto};
use crate::utxo::sat_from_big_decimal;
use crate::utxo::utxo_common::big_decimal_from_sat;
use crate::{big_decimal_from_sat_unsigned, BalanceError, BalanceFut, BigDecimal, CoinBalance, CoinFutSpawner,
            FeeApproxStage, FoundSwapTxSpend, HistorySyncState, MarketCoinOps, MmCoin, NegotiateSwapContractAddrErr,
            PaymentInstructions, PaymentInstructionsErr, RawTransactionError, RawTransactionFut,
            RawTransactionRequest, RawTransactionRes, SearchForSwapTxSpendInput, SignatureResult, SwapOps, TradeFee,
            TradePreimageFut, TradePreimageResult, TradePreimageValue, TransactionDetails, TransactionEnum,
            TransactionErr, TransactionFut, TransactionType, TxFeeDetails, TxMarshalingErr,
            UnexpectedDerivationMethod, ValidateAddressResult, ValidateInstructionsErr, ValidateOtherPubKeyErr,
            ValidatePaymentFut, ValidatePaymentInput, VerificationResult, WatcherOps,
            WatcherSearchForSwapTxSpendInput, WatcherValidatePaymentInput, WithdrawError, WithdrawFut, WithdrawRequest};
use async_std::prelude::FutureExt as AsyncStdFutureExt;
use async_trait::async_trait;
use bitcrypto::{dhash160, sha256};
use common::executor::Timer;
use common::executor::{abortable_queue::AbortableQueue, AbortableSystem};
use common::{get_utc_timestamp, log, Future01CompatExt};
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
use derive_more::Display;
use futures::lock::Mutex as AsyncMutex;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use hex::FromHexError;
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::MmNumber;
use parking_lot::Mutex as PaMutex;
use prost::{DecodeError, Message};
use rpc::v1::types::Bytes as BytesJson;
use serde_json::Value as Json;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;
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
pub const GAS_LIMIT_DEFAULT: u64 = 100_000;
pub(crate) const TX_DEFAULT_MEMO: &str = "";

// https://github.com/irisnet/irismod/blob/5016c1be6fdbcffc319943f33713f4a057622f0a/modules/htlc/types/validation.go#L19-L22
const MAX_TIME_LOCK: i64 = 34560;
const MIN_TIME_LOCK: i64 = 50;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TendermintFeeDetails {
    pub coin: String,
    pub amount: BigDecimal,
    pub gas_limit: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TendermintProtocolInfo {
    decimals: u8,
    denom: String,
    pub account_prefix: String,
    chain_id: String,
    gas_price: Option<f64>,
}

#[derive(Clone)]
pub struct ActivatedTokenInfo {
    decimals: u8,
    denom: Denom,
}

pub struct TendermintCoinImpl {
    ticker: String,
    /// TODO
    /// Test Vec<String(rpc_urls)> instead of HttpClient and pick
    /// better one in terms of performance & resource consumption on runtime.
    rpc_clients: Vec<HttpClient>,
    /// My address
    avg_block_time: u8,
    pub account_id: AccountId,
    pub(super) account_prefix: String,
    priv_key: Vec<u8>,
    decimals: u8,
    pub(super) denom: Denom,
    chain_id: ChainId,
    gas_price: Option<f64>,
    pub(super) sequence_lock: AsyncMutex<()>,
    tokens_info: PaMutex<HashMap<String, ActivatedTokenInfo>>,
    /// This spawner is used to spawn coin's related futures that should be aborted on coin deactivation
    /// or on [`MmArc::stop`].
    pub(super) abortable_system: AbortableQueue,
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
    InvalidPrivKey(String),
    CouldNotGenerateAccountId(String),
    EmptyRpcUrls,
    RpcClientInitError(String),
    InvalidChainId(String),
    InvalidDenom(String),
    RpcError(String),
    #[display(fmt = "avg_block_time missing or invalid. Please provide it with min 1 or max 255 value.")]
    AvgBlockTimeMissingOrInvalid,
}

#[derive(Display, Debug)]
pub enum TendermintCoinRpcError {
    Prost(DecodeError),
    InvalidResponse(String),
    PerformError(String),
}

impl From<DecodeError> for TendermintCoinRpcError {
    fn from(err: DecodeError) -> Self { TendermintCoinRpcError::Prost(err) }
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
        }
    }
}

impl From<TendermintCoinRpcError> for ValidatePaymentError {
    fn from(err: TendermintCoinRpcError) -> Self {
        match err {
            TendermintCoinRpcError::InvalidResponse(e) => ValidatePaymentError::InvalidRpcResponse(e),
            TendermintCoinRpcError::Prost(e) => ValidatePaymentError::InvalidRpcResponse(e.to_string()),
            TendermintCoinRpcError::PerformError(e) => ValidatePaymentError::Transport(e),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<cosmrs::rpc::Error> for TendermintCoinRpcError {
    fn from(err: cosmrs::rpc::Error) -> Self { TendermintCoinRpcError::PerformError(err.to_string()) }
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

fn account_id_from_privkey(priv_key: &[u8], prefix: &str) -> MmResult<AccountId, TendermintInitErrorKind> {
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

pub struct AllBalancesResult {
    pub platform_balance: BigDecimal,
    pub tokens_balances: HashMap<String, BigDecimal>,
}

impl TendermintCoin {
    pub async fn init(
        ctx: &MmArc,
        ticker: String,
        avg_block_time: u8,
        protocol_info: TendermintProtocolInfo,
        rpc_urls: Vec<String>,
        priv_key: &[u8],
    ) -> MmResult<Self, TendermintInitError> {
        if rpc_urls.is_empty() {
            return MmError::err(TendermintInitError {
                ticker,
                kind: TendermintInitErrorKind::EmptyRpcUrls,
            });
        }

        let account_id =
            account_id_from_privkey(priv_key, &protocol_info.account_prefix).mm_err(|kind| TendermintInitError {
                ticker: ticker.clone(),
                kind,
            })?;

        let rpc_clients: Result<Vec<HttpClient>, _> = rpc_urls
            .iter()
            .map(|url| {
                HttpClient::new(url.as_str()).map_to_mm(|e| TendermintInitError {
                    ticker: ticker.clone(),
                    kind: TendermintInitErrorKind::RpcClientInitError(e.to_string()),
                })
            })
            .collect();

        let rpc_clients = rpc_clients?;

        let chain_id = ChainId::try_from(protocol_info.chain_id).map_to_mm(|e| TendermintInitError {
            ticker: ticker.clone(),
            kind: TendermintInitErrorKind::InvalidChainId(e.to_string()),
        })?;

        let denom = Denom::from_str(&protocol_info.denom).map_to_mm(|e| TendermintInitError {
            ticker: ticker.clone(),
            kind: TendermintInitErrorKind::InvalidDenom(e.to_string()),
        })?;

        // Create an abortable system linked to the `MmCtx` so if the context is stopped via `MmArc::stop`,
        // all spawned futures related to `TendermintCoin` will be aborted as well.
        let abortable_system = ctx.abortable_system.create_subsystem();

        Ok(TendermintCoin(Arc::new(TendermintCoinImpl {
            ticker,
            rpc_clients,
            account_id,
            account_prefix: protocol_info.account_prefix,
            priv_key: priv_key.to_vec(),
            decimals: protocol_info.decimals,
            denom,
            chain_id,
            gas_price: protocol_info.gas_price,
            avg_block_time,
            sequence_lock: AsyncMutex::new(()),
            tokens_info: PaMutex::new(HashMap::new()),
            abortable_system,
        })))
    }

    // TODO
    // Save one working client to the coin context, only try others once it doesn't
    // work anymore.
    // Also, try couple times more on health check errors.
    async fn rpc_client(&self) -> MmResult<HttpClient, TendermintCoinRpcError> {
        for rpc_client in self.rpc_clients.iter() {
            match rpc_client.perform(HealthRequest).timeout(Duration::from_secs(3)).await {
                Ok(Ok(_)) => return Ok(rpc_client.clone()),
                Ok(Err(e)) => log::warn!(
                    "Recieved error from Tendermint rpc node during health check. Error: {:?}",
                    e
                ),
                Err(_) => log::warn!("Tendermint rpc node: {:?} got timeout during health check", rpc_client),
            };
        }

        MmError::err(TendermintCoinRpcError::PerformError(
            "All the current rpc nodes are unavailable.".to_string(),
        ))
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
        tx_payload: Any,
        timeout_height: u64,
        memo: String,
    ) -> cosmrs::Result<Vec<u8>> {
        let fee_amount = Coin {
            denom: self.denom.clone(),
            amount: 0_u64.into(),
        };

        let fee = Fee::from_amount_and_gas(fee_amount, GAS_LIMIT_DEFAULT);

        let signkey = SigningKey::from_bytes(&self.priv_key)?;
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
        // Needs to be sorted if cointains multiple coins
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

    #[allow(deprecated)]
    pub(super) async fn calculate_fee(
        &self,
        base_denom: Denom,
        tx_bytes: Vec<u8>,
    ) -> MmResult<Fee, TendermintCoinRpcError> {
        let path = AbciPath::from_str(ABCI_SIMULATE_TX_PATH).expect("valid path");
        let request = SimulateRequest { tx_bytes, tx: None };
        let request = AbciRequest::new(
            Some(path),
            request.encode_to_vec(),
            ABCI_REQUEST_HEIGHT,
            ABCI_REQUEST_PROVE,
        );

        let response = self.rpc_client().await?.perform(request).await?;
        let response = SimulateResponse::decode(response.response.value.as_slice())?;

        let gas = response.gas_info.as_ref().ok_or_else(|| {
            TendermintCoinRpcError::InvalidResponse(format!(
                "Could not read gas_info. Invalid Response: {:?}",
                response
            ))
        })?;

        let amount = ((gas.gas_used as f64 * 1.5) * self.gas_price()).ceil();

        let fee_amount = Coin {
            denom: base_denom,
            amount: (amount as u64).into(),
        };

        Ok(Fee::from_amount_and_gas(fee_amount, GAS_LIMIT_DEFAULT))
    }

    #[allow(deprecated)]
    pub(super) async fn calculate_fee_amount_as_u64(&self, tx_bytes: Vec<u8>) -> MmResult<u64, TendermintCoinRpcError> {
        let path = AbciPath::from_str(ABCI_SIMULATE_TX_PATH).expect("valid path");
        let request = SimulateRequest { tx_bytes, tx: None };
        let request = AbciRequest::new(
            Some(path),
            request.encode_to_vec(),
            ABCI_REQUEST_HEIGHT,
            ABCI_REQUEST_PROVE,
        );

        let response = self.rpc_client().await?.perform(request).await?;
        let response = SimulateResponse::decode(response.response.value.as_slice())?;

        let gas = response.gas_info.as_ref().ok_or_else(|| {
            TendermintCoinRpcError::InvalidResponse(format!(
                "Could not read gas_info. Invalid Response: {:?}",
                response
            ))
        })?;

        Ok(((gas.gas_used as f64 * 1.5) * self.gas_price()).ceil() as u64)
    }

    pub(super) async fn my_account_info(&self) -> MmResult<BaseAccount, TendermintCoinRpcError> {
        let path = AbciPath::from_str(ABCI_QUERY_ACCOUNT_PATH).expect("valid path");
        let request = QueryAccountRequest {
            address: self.account_id.to_string(),
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
        Ok(BaseAccount::decode(account.value.as_slice())?)
    }

    pub(super) async fn balance_for_denom(&self, denom: String) -> MmResult<u64, TendermintCoinRpcError> {
        let path = AbciPath::from_str(ABCI_QUERY_BALANCE_PATH).expect("valid path");
        let request = QueryBalanceRequest {
            address: self.account_id.to_string(),
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

    pub async fn all_balances(&self) -> MmResult<AllBalancesResult, TendermintCoinRpcError> {
        let platform_balance_denom = self.balance_for_denom(self.denom.to_string()).await?;
        let platform_balance = big_decimal_from_sat_unsigned(platform_balance_denom, self.decimals);
        let ibc_assets_info = self.tokens_info.lock().clone();

        let mut result = AllBalancesResult {
            platform_balance,
            tokens_balances: HashMap::new(),
        };
        for (ticker, info) in ibc_assets_info {
            let balance_denom = self.balance_for_denom(info.denom.to_string()).await?;
            let balance_decimal = big_decimal_from_sat_unsigned(balance_denom, info.decimals);
            result.tokens_balances.insert(ticker, balance_decimal);
        }

        Ok(result)
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
        account_info: BaseAccount,
        tx_payload: Any,
        fee: Fee,
        timeout_height: u64,
        memo: String,
    ) -> cosmrs::Result<Raw> {
        let signkey = SigningKey::from_bytes(&self.priv_key)?;
        let tx_body = tx::Body::new(vec![tx_payload], memo, timeout_height as u32);
        let auth_info = SignerInfo::single_direct(Some(signkey.public_key()), account_info.sequence).auth_info(fee);
        let sign_doc = SignDoc::new(&tx_body, &auth_info, &self.chain_id, account_info.account_number)?;
        sign_doc.sign(&signkey)
    }

    pub fn add_activated_token_info(&self, ticker: String, decimals: u8, denom: Denom) {
        self.tokens_info
            .lock()
            .insert(ticker, ActivatedTokenInfo { decimals, denom });
    }

    fn estimate_blocks_from_duration(&self, duration: u64) -> i64 {
        let estimated_time_lock = (duration / self.avg_block_time as u64) as i64;

        if estimated_time_lock > MAX_TIME_LOCK {
            MAX_TIME_LOCK
        } else if estimated_time_lock < MIN_TIME_LOCK {
            MIN_TIME_LOCK
        } else {
            estimated_time_lock
        }
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
                0 | 1 | 2 => {},
                unexpected_state => return Err(format!("Unexpected state for HTLC {}", unexpected_state)),
            };

            let rpc_client = try_s!(coin.rpc_client().await);
            let q = format!("create_htlc.id = '{}'", htlc_id);

            let response = try_s!(
                // Search single tx
                rpc_client
                    .perform(TxSearchRequest::new(q, false, 1, 1, TendermintResultOrder::Descending))
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

            let _sequence_lock = coin.sequence_lock.lock().await;
            let current_block = try_tx_s!(coin.current_block().compat().await);
            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;
            let account_info = try_tx_s!(coin.my_account_info().await);

            let simulated_tx = try_tx_s!(coin.gen_simulated_tx(
                account_info.clone(),
                create_htlc_tx.msg_payload.clone(),
                timeout_height,
                TX_DEFAULT_MEMO.into(),
            ));

            let fee = try_tx_s!(coin.calculate_fee(coin.denom.clone(), simulated_tx).await);

            let tx_raw = try_tx_s!(coin.any_to_signed_raw_tx(
                account_info.clone(),
                create_htlc_tx.msg_payload.clone(),
                fee,
                timeout_height,
                TX_DEFAULT_MEMO.into(),
            ));

            let _tx_id = try_tx_s!(coin.send_raw_tx_bytes(&try_tx_s!(tx_raw.to_bytes())).compat().await);

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
            let _sequence_lock = coin.sequence_lock.lock().await;
            let account_info = try_tx_s!(coin.my_account_info().await);

            let current_block = try_tx_s!(coin.current_block().compat().await.map_to_mm(WithdrawError::Transport));
            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

            let simulated_tx = try_tx_s!(coin.gen_simulated_tx(
                account_info.clone(),
                tx_payload.clone(),
                timeout_height,
                TX_DEFAULT_MEMO.into(),
            ));

            let fee = try_tx_s!(coin.calculate_fee(coin.denom.clone(), simulated_tx).await);

            let tx_raw = try_tx_s!(coin
                .any_to_signed_raw_tx(account_info, tx_payload, fee, timeout_height, memo)
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string())));

            let tx_bytes = try_tx_s!(tx_raw
                .to_bytes()
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string())));

            let _tx_id = try_tx_s!(coin.send_raw_tx_bytes(&tx_bytes).compat().await);

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
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let tx = match fee_tx {
            TransactionEnum::CosmosTransaction(tx) => tx.clone(),
            invalid_variant => {
                return Box::new(futures01::future::err(ERRL!(
                    "Unexpected tx variant {:?}",
                    invalid_variant
                )))
            },
        };

        let uuid = try_fus!(Uuid::from_slice(uuid)).to_string();
        let sender_pubkey_hash = dhash160(expected_sender);
        let expected_sender_address =
            try_fus!(AccountId::new(&self.account_prefix, sender_pubkey_hash.as_slice())).to_string();

        let dex_fee_addr_pubkey_hash = dhash160(fee_addr);
        let expected_dex_fee_address = try_fus!(AccountId::new(
            &self.account_prefix,
            dex_fee_addr_pubkey_hash.as_slice()
        ))
        .to_string();

        let expected_amount = try_fus!(sat_from_big_decimal(amount, decimals));
        let expected_amount = CoinProto {
            denom,
            amount: expected_amount.to_string(),
        };

        let coin = self.clone();
        let fut = async move {
            let tx_body = try_s!(TxBody::decode(tx.data.body_bytes.as_slice()));
            if tx_body.messages.len() != 1 {
                return ERR!("Tx body must have exactly one message");
            }

            let msg = try_s!(MsgSendProto::decode(tx_body.messages[0].value.as_slice()));
            if msg.to_address != expected_dex_fee_address {
                return ERR!(
                    "Dex fee is sent to wrong address: {}, expected {}",
                    msg.to_address,
                    expected_dex_fee_address
                );
            }

            if msg.amount.len() != 1 {
                return ERR!("Msg must have exactly one Coin");
            }

            if msg.amount[0] != expected_amount {
                return ERR!("Invalid amount {:?}, expected {:?}", msg.amount[0], expected_amount);
            }

            if msg.from_address != expected_sender_address {
                return ERR!(
                    "Invalid sender: {}, expected {}",
                    msg.from_address,
                    expected_sender_address
                );
            }

            if tx_body.memo != uuid {
                return ERR!("Invalid memo: {}, expected {}", msg.from_address, uuid);
            }

            let encoded_tx = tx.data.encode_to_vec();
            let hash = hex::encode_upper(sha256(&encoded_tx).as_slice());
            let encoded_from_rpc = try_s!(coin.request_tx(hash).await).encode_to_vec();
            if encoded_tx != encoded_from_rpc {
                return ERR!("Transaction from RPC doesn't match the input");
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
                0 => Ok(()),
                unexpected_state => MmError::err(ValidatePaymentError::UnexpectedPaymentState(format!(
                    "{}",
                    unexpected_state
                ))),
            }
        };
        Box::new(fut.boxed().compat())
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

    async fn query_htlc(&self, id: String) -> MmResult<QueryHtlcResponseProto, TendermintCoinRpcError> {
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
            let balance_denom = coin.balance_for_denom(coin.denom.to_string()).await?;
            let balance_dec = big_decimal_from_sat_unsigned(balance_denom, coin.decimals);

            // << BEGIN TX SIMULATION FOR FEE CALCULATION
            let (amount_denom, amount_dec, total_amount) = if req.max {
                let amount_denom = balance_denom;
                (
                    amount_denom,
                    big_decimal_from_sat_unsigned(amount_denom, coin.decimals),
                    balance_dec.clone(),
                )
            } else {
                let total = req.amount.clone();

                (
                    sat_from_big_decimal(&req.amount, coin.decimals)?,
                    req.amount.clone(),
                    total,
                )
            };

            if !coin.is_tx_amount_enough(coin.decimals, &amount_dec) {
                return MmError::err(WithdrawError::AmountTooLow {
                    amount: amount_dec,
                    threshold: coin.min_tx_amount(),
                });
            }

            let received_by_me = if to_address == coin.account_id {
                amount_dec
            } else {
                BigDecimal::default()
            };

            let msg_send = MsgSend {
                from_address: coin.account_id.clone(),
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

            let _sequence_lock = coin.sequence_lock.lock().await;
            let account_info = coin.my_account_info().await?;

            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

            let simulated_tx = coin
                .gen_simulated_tx(account_info.clone(), msg_send.clone(), timeout_height, memo.clone())
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;
            // >> END TX SIMULATION FOR FEE CALCULATION

            let fee_amount_u64 = coin.calculate_fee_amount_as_u64(simulated_tx).await?;
            let fee_amount_dec = big_decimal_from_sat_unsigned(fee_amount_u64, coin.decimals());

            let fee_amount = Coin {
                denom: coin.denom.clone(),
                amount: fee_amount_u64.into(),
            };

            let fee = Fee::from_amount_and_gas(fee_amount, GAS_LIMIT_DEFAULT);

            let (amount_denom, amount_dec, total_amount) = if req.max {
                if balance_denom < fee_amount_u64 {
                    return MmError::err(WithdrawError::NotSufficientBalance {
                        coin: coin.ticker.clone(),
                        available: balance_dec,
                        required: fee_amount_dec,
                    });
                }
                let amount_denom = balance_denom - fee_amount_u64;
                (
                    amount_denom,
                    big_decimal_from_sat_unsigned(amount_denom, coin.decimals),
                    balance_dec,
                )
            } else {
                let total = &req.amount + &fee_amount_dec;
                if balance_dec < total {
                    return MmError::err(WithdrawError::NotSufficientBalance {
                        coin: coin.ticker.clone(),
                        available: balance_dec,
                        required: total,
                    });
                }

                (
                    sat_from_big_decimal(&req.amount, coin.decimals)?,
                    req.amount.clone(),
                    total,
                )
            };

            let msg_send = MsgSend {
                from_address: coin.account_id.clone(),
                to_address,
                amount: vec![Coin {
                    denom: coin.denom.clone(),
                    amount: amount_denom.into(),
                }],
            }
            .to_any()
            .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let tx_raw = coin
                .any_to_signed_raw_tx(account_info, msg_send, fee, timeout_height, memo)
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let tx_bytes = tx_raw
                .to_bytes()
                .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))?;

            let hash = sha256(&tx_bytes);
            Ok(TransactionDetails {
                tx_hash: hex::encode_upper(hash.as_slice()),
                tx_hex: tx_bytes.into(),
                from: vec![coin.account_id.to_string()],
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
                    gas_limit: GAS_LIMIT_DEFAULT,
                })),
                coin: coin.ticker.to_string(),
                internal_id: hash.to_vec().into(),
                kmd_rewards: None,
                transaction_type: TransactionType::default(),
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

    fn decimals(&self) -> u8 { self.decimals }

    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> { todo!() }

    fn validate_address(&self, address: &str) -> ValidateAddressResult { todo!() }

    fn process_history_loop(&self, ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> { todo!() }

    fn history_sync_status(&self) -> HistorySyncState { todo!() }

    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> { todo!() }

    // TODO
    // !! This function includes dummy implementation for P.O.C work
    async fn get_sender_trade_fee(
        &self,
        value: TradePreimageValue,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        Ok(TradeFee {
            coin: self.ticker().to_string(),
            amount: MmNumber::from(1_u64),
            paid_from_trading_vol: false,
        })
    }

    // TODO
    // !! This function includes dummy implementation for P.O.C work
    fn get_receiver_trade_fee(&self, stage: FeeApproxStage) -> TradePreimageFut<TradeFee> {
        let coin = self.clone();
        let fut = async move {
            Ok(TradeFee {
                coin: coin.ticker().to_string(),
                amount: MmNumber::from(1_u64),
                paid_from_trading_vol: false,
            })
        };

        Box::new(fut.boxed().compat())
    }

    // TODO
    // !! This function includes dummy implementation for P.O.C work
    async fn get_fee_to_send_taker_fee(
        &self,
        dex_fee_amount: BigDecimal,
        stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        Ok(TradeFee {
            coin: self.ticker().to_string(),
            amount: MmNumber::from(1_u64),
            paid_from_trading_vol: false,
        })
    }

    // TODO
    // !! This function includes dummy implementation for P.O.C work
    fn required_confirmations(&self) -> u64 { 0 }

    // TODO
    // !! This function includes dummy implementation for P.O.C work
    fn requires_notarization(&self) -> bool { false }

    fn set_required_confirmations(&self, confirmations: u64) { todo!() }

    fn set_requires_notarization(&self, requires_nota: bool) { todo!() }

    // TODO
    // !! This function includes dummy implementation for P.O.C work
    fn swap_contract_address(&self) -> Option<BytesJson> { None }

    fn mature_confirmations(&self) -> Option<u32> { None }

    fn coin_protocol_info(&self) -> Vec<u8> { Vec::new() }

    fn is_coin_protocol_supported(&self, info: &Option<Vec<u8>>) -> bool { true }
}

impl MarketCoinOps for TendermintCoin {
    fn ticker(&self) -> &str { &self.ticker }

    fn my_address(&self) -> MmResult<String, MyAddressError> { Ok(self.account_id.to_string()) }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> {
        let key = SigningKey::from_bytes(&self.priv_key).expect("privkey validity is checked on coin creation");
        Ok(key.public_key().to_string())
    }

    fn sign_message_hash(&self, _message: &str) -> Option<[u8; 32]> { todo!() }

    fn sign_message(&self, _message: &str) -> SignatureResult<String> { todo!() }

    fn verify_message(&self, _signature: &str, _message: &str, _address: &str) -> VerificationResult<bool> { todo!() }

    fn my_balance(&self) -> BalanceFut<CoinBalance> {
        let coin = self.clone();
        let fut = async move {
            let balance_denom = coin.balance_for_denom(coin.denom.to_string()).await?;
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

    fn wait_for_confirmations(
        &self,
        _tx: &[u8],
        _confirmations: u64,
        _requires_nota: bool,
        _wait_until: u64,
        _check_every: u64,
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        let fut = async move { Ok(()) };
        Box::new(fut.boxed().compat())
    }

    fn wait_for_htlc_tx_spend(
        &self,
        transaction: &[u8],
        secret_hash: &[u8],
        wait_until: u64,
        _from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
    ) -> TransactionFut {
        let tx = try_tx_fus!(cosmrs::Tx::from_bytes(transaction));
        let first_message = try_tx_fus!(tx.body.messages.first().ok_or("Tx body couldn't be read."));
        let htlc_proto = try_tx_fus!(CreateHtlcProtoRep::decode(first_message.value.as_slice()));
        let htlc = try_tx_fus!(MsgCreateHtlc::try_from(htlc_proto));
        let htlc_id = self.calculate_htlc_id(&htlc.sender, &htlc.to, htlc.amount, secret_hash);

        let events_string = format!("claim_htlc.id='{}'", htlc_id);
        let request = GetTxsEventRequest {
            events: vec![events_string],
            pagination: None,
            order_by: 0,
        };
        let encoded_request = request.encode_to_vec();

        let coin = self.clone();
        let path = try_tx_fus!(AbciPath::from_str(ABCI_GET_TXS_EVENT_PATH));
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

    fn display_priv_key(&self) -> Result<String, String> { Ok(hex::encode(&self.priv_key)) }

    fn min_tx_amount(&self) -> BigDecimal { big_decimal_from_sat(MIN_TX_SATOSHIS, self.decimals) }

    // TODO
    // !! This function includes dummy implementation for P.O.C work
    fn min_trading_vol(&self) -> MmNumber { MmNumber::from("0.00777") }
}

#[async_trait]
#[allow(unused_variables)]
impl SwapOps for TendermintCoin {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, uuid: &[u8]) -> TransactionFut {
        self.send_taker_fee_for_denom(fee_addr, amount, self.denom.clone(), self.decimals, uuid)
    }

    fn send_maker_payment(
        &self,
        time_lock_duration: u64,
        _time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
        _payment_instructions: &Option<PaymentInstructions>,
    ) -> TransactionFut {
        self.send_htlc_for_denom(
            time_lock_duration,
            taker_pub,
            secret_hash,
            amount,
            self.denom.clone(),
            self.decimals,
        )
    }

    fn send_taker_payment(
        &self,
        time_lock_duration: u64,
        _time_lock: u32,
        maker_pub: &[u8],
        secret_hash: &[u8],
        amount: BigDecimal,
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
        _payment_instructions: &Option<PaymentInstructions>,
    ) -> TransactionFut {
        self.send_htlc_for_denom(
            time_lock_duration,
            maker_pub,
            secret_hash,
            amount,
            self.denom.clone(),
            self.decimals,
        )
    }

    fn send_maker_spends_taker_payment(
        &self,
        taker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx = try_tx_fus!(cosmrs::Tx::from_bytes(taker_payment_tx));
        let msg = try_tx_fus!(tx.body.messages.first().ok_or("Tx body couldn't be read."));
        let htlc_proto: crate::tendermint::htlc_proto::CreateHtlcProtoRep =
            try_tx_fus!(prost::Message::decode(msg.value.as_slice()));
        let htlc = try_tx_fus!(MsgCreateHtlc::try_from(htlc_proto));

        let mut amount = htlc.amount.clone();
        amount.sort();
        drop_mutability!(amount);

        let coins_string = amount
            .iter()
            .map(|t| format!("{}{}", t.amount, t.denom))
            .collect::<Vec<String>>()
            .join(",");

        let htlc_id = self.calculate_htlc_id(&htlc.sender, &htlc.to, amount, secret_hash);

        let claim_htlc_tx = try_tx_fus!(self.gen_claim_htlc_tx(htlc_id, secret));
        let coin = self.clone();

        let fut = async move {
            let _sequence_lock = coin.sequence_lock.lock().await;
            let current_block = try_tx_s!(coin.current_block().compat().await);
            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

            let account_info = try_tx_s!(coin.my_account_info().await);

            let simulated_tx = try_tx_s!(coin.gen_simulated_tx(
                account_info.clone(),
                claim_htlc_tx.msg_payload.clone(),
                timeout_height,
                TX_DEFAULT_MEMO.into(),
            ));

            let fee = try_tx_s!(coin.calculate_fee(coin.denom.clone(), simulated_tx).await);

            let tx_raw = try_tx_s!(coin.any_to_signed_raw_tx(
                account_info,
                claim_htlc_tx.msg_payload,
                fee,
                timeout_height,
                TX_DEFAULT_MEMO.into(),
            ));

            let tx_id = try_tx_s!(coin.send_raw_tx_bytes(&try_tx_s!(tx_raw.to_bytes())).compat().await);

            Ok(TransactionEnum::CosmosTransaction(CosmosTransaction {
                data: tx_raw.into(),
            }))
        };

        Box::new(fut.boxed().compat())
    }

    fn send_taker_spends_maker_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        maker_pub: &[u8],
        secret: &[u8],
        secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        let tx = try_tx_fus!(cosmrs::Tx::from_bytes(maker_payment_tx));
        let msg = try_tx_fus!(tx.body.messages.first().ok_or("Tx body couldn't be read."));
        let htlc_proto: crate::tendermint::htlc_proto::CreateHtlcProtoRep =
            try_tx_fus!(prost::Message::decode(msg.value.as_slice()));
        let htlc = try_tx_fus!(MsgCreateHtlc::try_from(htlc_proto));

        let mut amount = htlc.amount.clone();
        amount.sort();
        drop_mutability!(amount);

        let coins_string = amount
            .iter()
            .map(|t| format!("{}{}", t.amount, t.denom))
            .collect::<Vec<String>>()
            .join(",");

        let htlc_id = self.calculate_htlc_id(&htlc.sender, &htlc.to, amount, secret_hash);

        let claim_htlc_tx = try_tx_fus!(self.gen_claim_htlc_tx(htlc_id, secret));
        let coin = self.clone();

        let fut = async move {
            let _sequence_lock = coin.sequence_lock.lock().await;
            let current_block = try_tx_s!(coin.current_block().compat().await);
            let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

            let account_info = try_tx_s!(coin.my_account_info().await);

            let simulated_tx = try_tx_s!(coin.gen_simulated_tx(
                account_info.clone(),
                claim_htlc_tx.msg_payload.clone(),
                timeout_height,
                TX_DEFAULT_MEMO.into(),
            ));

            let fee = try_tx_s!(coin.calculate_fee(coin.denom.clone(), simulated_tx).await);

            let tx_raw = try_tx_s!(coin.any_to_signed_raw_tx(
                account_info,
                claim_htlc_tx.msg_payload,
                fee,
                timeout_height,
                TX_DEFAULT_MEMO.into(),
            ));

            let tx_id = try_tx_s!(coin.send_raw_tx_bytes(&try_tx_s!(tx_raw.to_bytes())).compat().await);

            Ok(TransactionEnum::CosmosTransaction(CosmosTransaction {
                data: tx_raw.into(),
            }))
        };

        Box::new(fut.boxed().compat())
    }

    fn send_taker_refunds_payment(
        &self,
        _taker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        Box::new(futures01::future::err(TransactionErr::Plain(
            "Doesn't need transaction broadcast to refund IRIS HTLC".into(),
        )))
    }

    fn send_maker_refunds_payment(
        &self,
        maker_payment_tx: &[u8],
        time_lock: u32,
        taker_pub: &[u8],
        secret_hash: &[u8],
        swap_contract_address: &Option<BytesJson>,
        swap_unique_data: &[u8],
    ) -> TransactionFut {
        Box::new(futures01::future::err(TransactionErr::Plain(
            "Doesn't need transaction broadcast to refund IRIS HTLC".into(),
        )))
    }

    fn validate_fee(
        &self,
        fee_tx: &TransactionEnum,
        expected_sender: &[u8],
        fee_addr: &[u8],
        amount: &BigDecimal,
        _min_block_number: u64,
        uuid: &[u8],
    ) -> Box<dyn Future<Item = (), Error = String> + Send> {
        self.validate_fee_for_denom(
            fee_tx,
            expected_sender,
            fee_addr,
            amount,
            self.decimals,
            uuid,
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
        _time_lock: u32,
        other_pub: &[u8],
        secret_hash: &[u8],
        search_from_block: u64,
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
        amount: &BigDecimal,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        self.check_if_my_payment_sent_for_denom(self.decimals, self.denom.clone(), other_pub, secret_hash, amount)
    }

    async fn search_for_swap_tx_spend_my(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        todo!()
    }

    async fn search_for_swap_tx_spend_other(
        &self,
        input: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        todo!()
    }

    async fn extract_secret(&self, secret_hash: &[u8], spend_tx: &[u8]) -> Result<Vec<u8>, String> {
        let tx = try_s!(cosmrs::Tx::from_bytes(spend_tx));
        let msg = try_s!(tx.body.messages.first().ok_or("Tx body couldn't be read."));
        let htlc_proto: crate::tendermint::htlc_proto::ClaimHtlcProtoRep =
            try_s!(prost::Message::decode(msg.value.as_slice()));
        let htlc = try_s!(MsgClaimHtlc::try_from(htlc_proto));

        Ok(try_s!(hex::decode(htlc.secret)))
    }

    fn check_tx_signed_by_pub(&self, tx: &[u8], expected_pub: &[u8]) -> Result<bool, String> {
        unimplemented!();
    }

    fn negotiate_swap_contract_addr(
        &self,
        other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        Ok(None)
    }

    fn derive_htlc_key_pair(&self, swap_unique_data: &[u8]) -> KeyPair {
        key_pair_from_secret(&self.priv_key).expect("valid priv key")
    }

    fn validate_other_pubkey(&self, raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> {
        PublicKey::from_raw_secp256k1(raw_pubkey)
            .or_mm_err(|| ValidateOtherPubKeyErr::InvalidPubKey(hex::encode(raw_pubkey)))?;
        Ok(())
    }

    async fn payment_instructions(
        &self,
        _secret_hash: &[u8],
        _amount: &BigDecimal,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        Ok(None)
    }

    fn validate_instructions(
        &self,
        _instructions: &[u8],
        _secret_hash: &[u8],
        _amount: BigDecimal,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        MmError::err(ValidateInstructionsErr::UnsupportedCoin(self.ticker().to_string()))
    }

    fn is_supported_by_watchers(&self) -> bool { false }
}

#[async_trait]
#[allow(unused_variables)]
impl WatcherOps for TendermintCoin {
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

    fn send_taker_spends_maker_payment_preimage(&self, preimage: &[u8], secret: &[u8]) -> TransactionFut {
        unimplemented!();
    }

    fn create_taker_refunds_payment_preimage(
        &self,
        _taker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_watcher_refunds_taker_payment_preimage(&self, _taker_refunds_payment: &[u8]) -> TransactionFut {
        unimplemented!();
    }

    fn watcher_validate_taker_fee(&self, _taker_fee_hash: Vec<u8>, _verified_pub: Vec<u8>) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    async fn watcher_search_for_swap_tx_spend(
        &self,
        input: WatcherSearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!();
    }

    fn watcher_validate_taker_payment(&self, _input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }
}

#[cfg(test)]
pub mod tendermint_coin_tests {
    use super::*;
    use crate::tendermint::htlc_proto::ClaimHtlcProtoRep;
    use common::{block_on, DEX_FEE_ADDR_RAW_PUBKEY};
    use cosmrs::proto::cosmos::tx::v1beta1::{GetTxRequest, GetTxResponse, GetTxsEventResponse};
    use rand::{thread_rng, Rng};

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

    fn get_iris_usdc_ibc_protocol() -> TendermintProtocolInfo {
        TendermintProtocolInfo {
            decimals: 6,
            denom: String::from("ibc/5C465997B4F582F602CD64E12031C6A6E18CAF1E6EDC9B5D808822DC0B5F850C"),
            account_prefix: String::from("iaa"),
            chain_id: String::from("nyancat-9"),
            gas_price: None,
        }
    }

    fn get_iris_protocol() -> TendermintProtocolInfo {
        TendermintProtocolInfo {
            decimals: 6,
            denom: String::from("unyan"),
            account_prefix: String::from("iaa"),
            chain_id: String::from("nyancat-9"),
            gas_price: None,
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

        let protocol_conf = get_iris_usdc_ibc_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default()
            .with_secp256k1_key_pair(crypto::privkey::key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap())
            .into_mm_arc();

        let priv_key = &*ctx.secp256k1_key_pair().private().secret;

        let coin = common::block_on(TendermintCoin::init(
            &ctx,
            "USDC-IBC".to_string(),
            5,
            protocol_conf,
            rpc_urls,
            priv_key,
        ))
        .unwrap();

        // << BEGIN HTLC CREATION
        let base_denom: Denom = "unyan".parse().unwrap();
        let to: AccountId = IRIS_TESTNET_HTLC_PAIR2_ADDRESS.parse().unwrap();
        const UAMOUNT: u64 = 1;
        let amount: cosmrs::Decimal = UAMOUNT.into();
        let amount_dec = big_decimal_from_sat_unsigned(UAMOUNT, coin.decimals);
        let sec: [u8; 32] = thread_rng().gen();
        let time_lock = 1000;

        let create_htlc_tx = coin
            .gen_create_htlc_tx(coin.denom.clone(), &to, amount, sha256(&sec).as_slice(), time_lock)
            .unwrap();

        let current_block_fut = coin.current_block().compat();
        let current_block = block_on(async { current_block_fut.await.unwrap() });
        let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

        let account_info_fut = coin.my_account_info();
        let account_info = block_on(async { account_info_fut.await.unwrap() });

        let simulated_tx = coin
            .gen_simulated_tx(
                account_info.clone(),
                create_htlc_tx.msg_payload.clone(),
                timeout_height,
                TX_DEFAULT_MEMO.into(),
            )
            .unwrap();

        let fee = block_on(async { coin.calculate_fee(base_denom.clone(), simulated_tx).await.unwrap() });

        let raw_tx = block_on(async {
            coin.any_to_signed_raw_tx(
                account_info.clone(),
                create_htlc_tx.msg_payload.clone(),
                fee,
                timeout_height,
                TX_DEFAULT_MEMO.into(),
            )
            .unwrap()
        });
        let tx_bytes = raw_tx.to_bytes().unwrap();

        let send_tx_fut = coin.send_raw_tx_bytes(&tx_bytes).compat();
        block_on(async {
            send_tx_fut.await.unwrap();
        });
        // >> END HTLC CREATION

        let htlc_spent = block_on(
            coin.check_if_my_payment_sent(
                0,
                IRIS_TESTNET_HTLC_PAIR2_PUB_KEY,
                sha256(&sec).as_slice(),
                current_block,
                &None,
                &[],
                &amount_dec,
            )
            .compat(),
        )
        .unwrap();
        assert!(htlc_spent.is_some());

        // << BEGIN HTLC CLAIMING
        let claim_htlc_tx = coin.gen_claim_htlc_tx(create_htlc_tx.id, &sec).unwrap();

        let current_block_fut = coin.current_block().compat();
        let current_block = common::block_on(async { current_block_fut.await.unwrap() });
        let timeout_height = current_block + TIMEOUT_HEIGHT_DELTA;

        let account_info_fut = coin.my_account_info();
        let account_info = block_on(async { account_info_fut.await.unwrap() });

        let simulated_tx = coin
            .gen_simulated_tx(
                account_info.clone(),
                claim_htlc_tx.msg_payload.clone(),
                timeout_height,
                TX_DEFAULT_MEMO.into(),
            )
            .unwrap();

        let fee = block_on(async { coin.calculate_fee(base_denom.clone(), simulated_tx).await.unwrap() });

        let raw_tx = coin
            .any_to_signed_raw_tx(
                account_info,
                claim_htlc_tx.msg_payload,
                fee,
                timeout_height,
                TX_DEFAULT_MEMO.into(),
            )
            .unwrap();

        let tx_bytes = raw_tx.to_bytes().unwrap();
        let send_tx_fut = coin.send_raw_tx_bytes(&tx_bytes).compat();
        block_on(async {
            send_tx_fut.await.unwrap();
        });
        println!("Claim HTLC tx hash {}", hex::encode_upper(sha256(&tx_bytes).as_slice()));
        // >> END HTLC CLAIMING
    }

    #[test]
    fn try_query_claim_htlc_txs_and_get_secret() {
        let rpc_urls = vec![IRIS_TESTNET_RPC_URL.to_string()];

        let protocol_conf = get_iris_usdc_ibc_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default()
            .with_secp256k1_key_pair(crypto::privkey::key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap())
            .into_mm_arc();

        let priv_key = &*ctx.secp256k1_key_pair().private().secret;

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "USDC-IBC".to_string(),
            5,
            protocol_conf,
            rpc_urls,
            priv_key,
        ))
        .unwrap();

        let events = "claim_htlc.id='2B925FC83A106CC81590B3DB108AC2AE496FFA912F368FE5E29BC1ED2B754F2C'";
        let request = GetTxsEventRequest {
            events: vec![events.into()],
            pagination: None,
            order_by: 0,
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

        let claim_htlc = ClaimHtlcProtoRep::decode(first_msg.value.as_slice()).unwrap();
        let expected_secret = [1; 32];
        let actual_secret = hex::decode(claim_htlc.secret).unwrap();

        assert_eq!(actual_secret, expected_secret);
    }

    #[test]
    fn wait_for_tx_spend_test() {
        let rpc_urls = vec![IRIS_TESTNET_RPC_URL.to_string()];

        let protocol_conf = get_iris_usdc_ibc_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default()
            .with_secp256k1_key_pair(crypto::privkey::key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap())
            .into_mm_arc();

        let priv_key = &*ctx.secp256k1_key_pair().private().secret;

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "USDC-IBC".to_string(),
            5,
            protocol_conf,
            rpc_urls,
            priv_key,
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
            coin.wait_for_htlc_tx_spend(&encoded_tx, &secret_hash, get_utc_timestamp() as u64, 0, &None)
                .compat(),
        )
        .unwrap();

        // https://nyancat.iobscan.io/#/tx?txHash=565C820C1F95556ADC251F16244AAD4E4274772F41BC13F958C9C2F89A14D137
        let expected_spend_hash = "565C820C1F95556ADC251F16244AAD4E4274772F41BC13F958C9C2F89A14D137";
        let hash = spend_tx.tx_hash();
        assert_eq!(hex::encode_upper(&hash.0), expected_spend_hash);
    }

    #[test]
    fn validate_taker_fee_test() {
        let rpc_urls = vec![IRIS_TESTNET_RPC_URL.to_string()];

        let protocol_conf = get_iris_protocol();

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default()
            .with_secp256k1_key_pair(crypto::privkey::key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap())
            .into_mm_arc();

        let priv_key = &*ctx.secp256k1_key_pair().private().secret;

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "IRIS-TEST".to_string(),
            5,
            protocol_conf,
            rpc_urls,
            priv_key,
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
        let validate_err = coin
            .validate_fee(
                &create_htlc_tx,
                &[],
                &DEX_FEE_ADDR_RAW_PUBKEY,
                &invalid_amount,
                0,
                &[1; 16],
            )
            .wait()
            .unwrap_err();
        println!("{}", validate_err);
        assert!(validate_err.contains("failed to decode Protobuf message: MsgSend.amount"));

        // just a random transfer tx not related to AtomicDEX, should fail on recipient address check
        // https://nyancat.iobscan.io/#/tx?txHash=65815814E7D74832D87956144C1E84801DC94FE9A509D207A0ABC3F17775E5DF
        let random_transfer_tx_hash = "65815814E7D74832D87956144C1E84801DC94FE9A509D207A0ABC3F17775E5DF";
        let random_transfer_tx_bytes = block_on(coin.request_tx(random_transfer_tx_hash.into()))
            .unwrap()
            .encode_to_vec();

        let random_transfer_tx = TransactionEnum::CosmosTransaction(CosmosTransaction {
            data: TxRaw::decode(random_transfer_tx_bytes.as_slice()).unwrap(),
        });

        let validate_err = coin
            .validate_fee(
                &random_transfer_tx,
                &[],
                &DEX_FEE_ADDR_RAW_PUBKEY,
                &invalid_amount,
                0,
                &[1; 16],
            )
            .wait()
            .unwrap_err();
        println!("{}", validate_err);
        assert!(validate_err.contains("sent to wrong address"));

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

        let validate_err = coin
            .validate_fee(&dex_fee_tx, &[], &DEX_FEE_ADDR_RAW_PUBKEY, &invalid_amount, 0, &[1; 16])
            .wait()
            .unwrap_err();
        println!("{}", validate_err);
        assert!(validate_err.contains("Invalid amount"));

        let valid_amount: BigDecimal = "0.0001".parse().unwrap();
        // valid amount but invalid sender
        let validate_err = coin
            .validate_fee(
                &dex_fee_tx,
                &DEX_FEE_ADDR_RAW_PUBKEY,
                &DEX_FEE_ADDR_RAW_PUBKEY,
                &valid_amount,
                0,
                &[1; 16],
            )
            .wait()
            .unwrap_err();
        println!("{}", validate_err);
        assert!(validate_err.contains("Invalid sender"));

        // invalid memo
        let validate_err = coin
            .validate_fee(
                &dex_fee_tx,
                &pubkey,
                &DEX_FEE_ADDR_RAW_PUBKEY,
                &valid_amount,
                0,
                &[1; 16],
            )
            .wait()
            .unwrap_err();
        println!("{}", validate_err);
        assert!(validate_err.contains("Invalid memo"));

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

        let ctx = mm2_core::mm_ctx::MmCtxBuilder::default()
            .with_secp256k1_key_pair(crypto::privkey::key_pair_from_seed(IRIS_TESTNET_HTLC_PAIR1_SEED).unwrap())
            .into_mm_arc();

        let priv_key = &*ctx.secp256k1_key_pair().private().secret;

        let coin = block_on(TendermintCoin::init(
            &ctx,
            "IRIS-TEST".to_string(),
            5,
            protocol_conf,
            rpc_urls,
            priv_key,
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
}
