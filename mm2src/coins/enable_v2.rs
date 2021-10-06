use crate::utxo::rpc_clients::ElectrumRpcRequest;
use common::mm_error::prelude::*;
use ethereum_types::Address;

#[derive(Debug, Deserialize)]
pub enum UtxoActivationMode {
    Native,
    Electrum { servers: Vec<ElectrumRpcRequest> },
}

#[derive(Debug, Deserialize)]
pub enum ZcoinActivationMode {
    Native,
    // TODO extend in the future
    Zlite,
}

#[derive(Debug, Deserialize)]
pub enum EnableProtocolParams {
    Bch {
        #[serde(default)]
        allow_slp_unsafe_conf: bool,
        bchd_urls: Vec<String>,
        mode: UtxoActivationMode,
        with_tokens: Vec<String>,
    },
    SlpToken,
    Eth {
        urls: Vec<String>,
        swap_contract_address: Address,
        fallback_swap_contract: Option<Address>,
        with_tokens: Vec<String>,
    },
    Erc20,
    Utxo {
        mode: UtxoActivationMode,
    },
    Qrc20,
    Zcoin {
        mode: ZcoinActivationMode,
    },
}

#[derive(Debug, Deserialize)]
pub struct EnableRpcRequest {
    coin: String,
    tx_history: bool,
    required_confirmations: u64,
    requires_notarization: bool,
    protocol_params: EnableProtocolParams,
}

pub struct EnableResult {}

pub enum EnableError {}

pub async fn enable_v2() -> Result<EnableResult, MmError<EnableError>> { unimplemented!() }
