use ethereum_types::Address;
use serde::{Deserialize, Serialize};
use skip_serializing_none::skip_serializing_none;
use std::collections::HashMap;

use common::true_f;
use mm2_rpc::data::legacy::GasStationPricePolicy;

use super::{CoinAddressInfo, CoinBalance, TokenActivationRequest, TokenBalances};

#[derive(Deserialize, Serialize)]
pub(crate) struct EthWithTokensActivationRequest {
    #[serde(flatten)]
    platform_request: EthActivationV2Request,
    erc20_tokens_requests: Vec<TokenActivationRequest<Erc20TokenActivationRequest>>,
    #[serde(default = "true_f")]
    pub(crate) get_balances: bool,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize)]
pub(crate) struct EthActivationV2Request {
    #[serde(default)]
    nodes: Vec<EthNode>,
    #[serde(default)]
    rpc_mode: EthRpcMode,
    swap_contract_address: Address,
    fallback_swap_contract: Option<Address>,
    #[serde(default)]
    contract_supports_watchers: bool,
    gas_station_url: Option<String>,
    gas_station_decimals: Option<u8>,
    #[serde(default)]
    gas_station_policy: GasStationPricePolicy,
    mm2: Option<u8>,
    required_confirmations: Option<u64>,
    #[serde(default)]
    priv_key_policy: EthPrivKeyActivationPolicy,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct EthNode {
    url: String,
    #[serde(default)]
    gui_auth: bool,
}

#[derive(Deserialize, Serialize)]
pub(crate) enum EthRpcMode {
    Http,
    #[cfg(target_arch = "wasm32")]
    Metamask,
}

impl Default for EthRpcMode {
    fn default() -> Self { EthRpcMode::Http }
}

#[derive(Deserialize, Serialize)]
pub(crate) enum EthPrivKeyActivationPolicy {
    ContextPrivKey,
    #[cfg(target_arch = "wasm32")]
    Metamask,
}

impl Default for EthPrivKeyActivationPolicy {
    fn default() -> Self { EthPrivKeyActivationPolicy::ContextPrivKey }
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize)]
pub(crate) struct Erc20TokenActivationRequest {
    required_confirmations: Option<u64>,
}

#[derive(Deserialize)]
pub(crate) struct Erc20InitResult {
    pub(crate) balances: HashMap<String, CoinBalance>,
    pub(crate) platform_coin: String,
    pub(crate) token_contract_address: String,
    pub(crate) required_confirmations: u64,
}

#[derive(Deserialize)]
pub(crate) struct EthWithTokensActivationResult {
    pub(crate) current_block: u64,
    pub(crate) eth_addresses_infos: HashMap<String, CoinAddressInfo<CoinBalance>>,
    pub(crate) erc20_addresses_infos: HashMap<String, CoinAddressInfo<TokenBalances>>,
}
