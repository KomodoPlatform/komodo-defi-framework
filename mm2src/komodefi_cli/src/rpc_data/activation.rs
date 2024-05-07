//! Contains rpc data layer structures that are not ready to become a part of the mm2_rpc::data module
//!
//! *Note: it's expected that the following data types will be moved to mm2_rpc::data when mm2 is refactored to be able to handle them*
//!

#[path = "activation/bch.rs"] pub(crate) mod bch;
#[path = "activation/electrum.rs"] mod electrum;
#[path = "activation/enable.rs"] mod enable;
#[path = "activation/eth.rs"] pub(crate) mod eth;
#[path = "activation/tendermint.rs"] pub(crate) mod tendermint;
#[path = "activation/zcoin.rs"] pub(crate) mod zcoin;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use common::true_f;
use derive_more::Display;
use mm2_number::BigDecimal;
use mm2_rpc::data::legacy::{ElectrumProtocol, Mm2RpcResult};

use crate::rpc_data::eth::EthWithTokensActivationRequest;

#[derive(Default, Serialize)]
#[serde(tag = "method", rename = "get_enabled_coins")]
pub(crate) struct GetEnabledRequest {}

pub(crate) enum ActivationMethod {
    Legacy(ActivationMethodLegacy),
    V2(ActivationMethodV2),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub(crate) enum ActivationMethodLegacy {
    Enable(enable::EnableRequest),
    Electrum(electrum::ElectrumRequest),
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub(crate) enum ActivationMethodV2 {
    EnableBchWithTokens(EnablePlatformCoinWithTokensReq<bch::BchWithTokensActivationParams>),
    EnableSlp(EnableTokenRequest<bch::SlpActivationRequest>),
    EnableTendermintWithAssets(EnablePlatformCoinWithTokensReq<tendermint::TendermintActivationParams>),
    EnableTendermintToken(EnableTokenRequest<tendermint::TendermintTokenActivationParams>),
    EnableEthWithTokens(EnablePlatformCoinWithTokensReq<EthWithTokensActivationRequest>),
    EnableErc20(EnableTokenRequest<eth::Erc20TokenActivationRequest>),
    #[serde(rename = "task::enable_z_coin::init")]
    EnableZCoin(InitStandaloneCoinReq<zcoin::ZcoinActivationParams>),
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct EnablePlatformCoinWithTokensReq<T: Serialize> {
    ticker: String,
    #[serde(flatten)]
    request: T,
}

impl<T: SetTxHistory + Serialize> SetTxHistory for EnablePlatformCoinWithTokensReq<T> {
    fn set_tx_history_impl(&mut self) { self.request.set_tx_history_impl() }
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct TokenActivationRequest<Req> {
    ticker: String,
    #[serde(flatten)]
    request: Req,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct EnableTokenRequest<T> {
    ticker: String,
    activation_params: T,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(tag = "method", rename = "disable_coin")]
pub(crate) struct DisableCoinRequest {
    pub(crate) coin: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum DisableCoinResponse {
    Success(Mm2RpcResult<DisableCoinSuccess>),
    Failed(DisableCoinFailed),
}

#[derive(Debug, Deserialize)]
pub(crate) struct DisableCoinSuccess {
    pub(crate) coin: String,
    pub(crate) cancelled_orders: Vec<Uuid>,
    pub(crate) passivized: bool,
}

#[derive(Deserialize)]
pub(crate) struct DisableCoinFailed {
    pub(crate) error: String,
    pub(crate) orders: DisableFailedOrders,
    pub(crate) active_swaps: Vec<Uuid>,
}

#[derive(Deserialize)]
pub(crate) struct DisableFailedOrders {
    pub(crate) matching: Vec<Uuid>,
    pub(crate) cancelled: Vec<Uuid>,
}

#[derive(Deserialize)]
pub(crate) struct SetRequiredConfResponse {
    pub(crate) coin: String,
    pub(crate) confirmations: u64,
}

#[derive(Deserialize)]
pub(crate) struct SetRequiredNotaResponse {
    pub(crate) coin: String,
    pub(crate) requires_notarization: bool,
}

#[derive(Default, Serialize)]
#[serde(tag = "method", rename = "coins_needed_for_kick_start")]
pub(crate) struct CoinsToKickStartRequest {}

pub(crate) type CoinsToKickstartResponse = Vec<String>;

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct CoinBalance {
    pub(crate) spendable: BigDecimal,
    pub(crate) unspendable: BigDecimal,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CoinAddressInfo<Balance> {
    pub(crate) derivation_method: DerivationMethod,
    pub(crate) pubkey: String,
    pub(crate) balances: Option<Balance>,
    pub(crate) tickers: Option<HashSet<String>>,
}

pub(crate) type TokenBalances = HashMap<String, CoinBalance>;

#[derive(Display, Debug, Deserialize)]
#[serde(tag = "type", content = "data")]
pub(crate) enum DerivationMethod {
    Iguana,
    #[allow(dead_code)]
    HDWallet(String),
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ElectrumRpcRequest {
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) protocol: ElectrumProtocol,
    #[serde(default)]
    pub(crate) disable_cert_verification: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct InitStandaloneCoinReq<T> {
    ticker: String,
    activation_params: T,
}

#[derive(Deserialize)]
pub(crate) struct InitRpcTaskResponse {
    pub(crate) task_id: TaskId,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct RpcTaskStatusRequest {
    pub(crate) task_id: TaskId,
    #[serde(default = "true_f")]
    pub(crate) forget_if_finished: bool,
}

pub(crate) type TaskId = u64;

#[derive(Debug, Deserialize)]
#[serde(tag = "status", content = "details")]
pub(crate) enum RpcTaskStatus<Item, Error, InProgressStatus, AwaitingStatus> {
    Ok(Item),
    Error(Error),
    InProgress(InProgressStatus),
    UserActionRequired(AwaitingStatus),
}

#[derive(Serialize)]
pub(crate) struct CancelRpcTaskRequest {
    pub(crate) task_id: TaskId,
}

#[derive(Display, Deserialize)]
#[serde(tag = "error_type", content = "error_data")]
pub(crate) enum CancelRpcTaskError {
    #[display(fmt = "No such task '{}'", _0)]
    NoSuchTask(TaskId),
    #[display(fmt = "Task is finished already")]
    TaskFinished(TaskId),
    #[display(fmt = "Internal error: {}", _0)]
    Internal(String),
}

pub(crate) trait SetTxHistory {
    fn set_tx_history_impl(&mut self);
    fn set_tx_history(&mut self, tx_history: bool) {
        if tx_history {
            self.set_tx_history_impl();
        }
    }
}

impl SetTxHistory for ActivationMethod {
    fn set_tx_history_impl(&mut self) {
        match self {
            Self::Legacy(method) => method.set_tx_history_impl(),
            Self::V2(method) => method.set_tx_history_impl(),
        }
    }
}

impl SetTxHistory for ActivationMethodLegacy {
    fn set_tx_history_impl(&mut self) {
        match self {
            Self::Enable(ref mut method) => method.set_tx_history_impl(),
            Self::Electrum(ref mut method) => method.set_tx_history_impl(),
        }
    }
}

impl SetTxHistory for ActivationMethodV2 {
    fn set_tx_history_impl(&mut self) {
        match self {
            Self::EnableBchWithTokens(method) => method.set_tx_history_impl(),
            Self::EnableSlp(_) => {},
            Self::EnableEthWithTokens(_) => {},
            Self::EnableErc20(_) => {},
            Self::EnableTendermintWithAssets(method) => method.set_tx_history_impl(),
            Self::EnableTendermintToken(_) => {},
            Self::EnableZCoin(_) => {},
        }
    }
}
