use rpc::v1::types::H256 as H256Json;
use serde::{Deserialize, Serialize};
use skip_serializing_none::skip_serializing_none;
use std::collections::HashMap;

use common::true_f;
use mm2_rpc::data::legacy::UtxoMergeParams;

use super::{CoinAddressInfo, CoinBalance, ElectrumRpcRequest, SetTxHistory, TokenActivationRequest, TokenBalances};

#[derive(Deserialize, Serialize)]
pub(crate) struct BchWithTokensActivationParams {
    #[serde(flatten)]
    platform_request: BchActivationRequest,
    slp_tokens_requests: Vec<TokenActivationRequest<SlpActivationRequest>>,
    #[serde(default = "true_f")]
    pub(crate) get_balances: bool,
}

impl SetTxHistory for BchWithTokensActivationParams {
    fn set_tx_history_impl(&mut self) { self.platform_request.utxo_params.tx_history = true; }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct BchActivationRequest {
    #[serde(default)]
    allow_slp_unsafe_conf: bool,
    bchd_urls: Vec<String>,
    #[serde(flatten)]
    pub(crate) utxo_params: UtxoActivationParams,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize)]
pub(crate) struct UtxoActivationParams {
    pub(crate) mode: UtxoRpcMode,
    pub(crate) utxo_merge_params: Option<UtxoMergeParams>,
    #[serde(default)]
    pub(crate) tx_history: bool,
    pub(crate) required_confirmations: Option<u64>,
    pub(crate) requires_notarization: Option<bool>,
    pub(crate) address_format: Option<UtxoAddressFormat>,
    pub(crate) gap_limit: Option<u32>,
    #[serde(flatten)]
    pub(crate) enable_params: EnabledCoinBalanceParams,
    #[serde(default)]
    pub(crate) priv_key_policy: PrivKeyActivationPolicy,
    pub(crate) check_utxo_maturity: Option<bool>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "rpc", content = "rpc_data")]
pub(crate) enum UtxoRpcMode {
    Native,
    Electrum { servers: Vec<ElectrumRpcRequest> },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "format")]
pub(crate) enum UtxoAddressFormat {
    #[serde(rename = "standard")]
    Standard,
    #[serde(rename = "segwit")]
    Segwit,
    #[serde(rename = "cashaddress")]
    CashAddress {
        network: String,
        #[serde(default)]
        pub_addr_prefix: u8,
        #[serde(default)]
        p2sh_addr_prefix: u8,
    },
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) enum PrivKeyActivationPolicy {
    ContextPrivKey,
    Trezor,
}

impl Default for PrivKeyActivationPolicy {
    fn default() -> Self { PrivKeyActivationPolicy::ContextPrivKey }
}

#[skip_serializing_none]
#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct EnabledCoinBalanceParams {
    #[serde(default)]
    pub(crate) scan_policy: EnableCoinScanPolicy,
    pub(crate) min_addresses_number: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EnableCoinScanPolicy {
    DoNotScan,
    ScanIfNewWallet,
    Scan,
}

impl Default for EnableCoinScanPolicy {
    fn default() -> Self { EnableCoinScanPolicy::ScanIfNewWallet }
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct SlpActivationRequest {
    pub(crate) required_confirmations: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BchWithTokensActivationResult {
    pub(crate) current_block: u64,
    pub(crate) bch_addresses_infos: HashMap<String, CoinAddressInfo<CoinBalance>>,
    pub(crate) slp_addresses_infos: HashMap<String, CoinAddressInfo<TokenBalances>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SlpInitResult {
    pub(crate) balances: HashMap<String, CoinBalance>,
    pub(crate) token_id: H256Json,
    pub(crate) platform_coin: String,
    pub(crate) required_confirmations: u64,
}
