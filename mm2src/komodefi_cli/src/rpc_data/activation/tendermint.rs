use serde::{Deserialize, Serialize};
use skip_serializing_none::skip_serializing_none;
use std::collections::{HashMap, HashSet};

use common::true_f;

use super::{CoinBalance, SetTxHistory, TokenActivationRequest};

#[derive(Deserialize, Serialize)]
pub(crate) struct TendermintActivationParams {
    rpc_urls: Vec<String>,
    tokens_params: Vec<TokenActivationRequest<TendermintTokenActivationParams>>,
    #[serde(default)]
    tx_history: bool,
    #[serde(default = "true_f")]
    get_balances: bool,
}

impl SetTxHistory for TendermintActivationParams {
    fn set_tx_history_impl(&mut self) { self.tx_history = true; }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct TendermintTokenActivationParams {}

#[skip_serializing_none]
#[derive(Deserialize, Serialize)]
pub(crate) struct TendermintActivationResult {
    pub(crate) ticker: String,
    pub(crate) address: String,
    pub(crate) current_block: u64,
    pub(crate) balance: Option<CoinBalance>,
    pub(crate) tokens_balances: Option<HashMap<String, CoinBalance>>,
    pub(crate) tokens_tickers: Option<HashSet<String>>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct TendermintTokenInitResult {
    pub(crate) balances: HashMap<String, CoinBalance>,
    pub(crate) platform_coin: String,
}
