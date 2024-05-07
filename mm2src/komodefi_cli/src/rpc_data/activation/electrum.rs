use mm2_rpc::data::legacy::{ElectrumProtocol, UtxoMergeParams};
use serde::{Deserialize, Serialize};
use skip_serializing_none::skip_serializing_none;

use super::SetTxHistory;

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ElectrumRequest {
    coin: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) servers: Vec<Server>,
    mm2: Option<u8>,
    #[serde(default)]
    pub(crate) tx_history: bool,
    required_confirmations: Option<u64>,
    requires_notarization: Option<bool>,
    swap_contract_address: Option<String>,
    fallback_swap_contract: Option<String>,
    utxo_merge_params: Option<UtxoMergeParams>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Server {
    url: String,
    #[serde(default)]
    protocol: ElectrumProtocol,
    #[serde(default)]
    disable_cert_verification: bool,
}

impl SetTxHistory for ElectrumRequest {
    fn set_tx_history_impl(&mut self) { self.tx_history = true; }
}
