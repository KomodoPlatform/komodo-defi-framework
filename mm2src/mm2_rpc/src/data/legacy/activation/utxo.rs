use common::serde_derive::{Deserialize, Serialize};
use common::{one_hundred, ten_f64};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UtxoMergeParams {
    pub merge_at: usize,
    #[serde(default = "ten_f64")]
    pub check_every: f64,
    #[serde(default = "one_hundred")]
    pub max_merge_at_once: usize,
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
/// Deserializable Electrum protocol representation for RPC
pub enum ElectrumProtocol {
    /// TCP
    #[cfg_attr(not(target_arch = "wasm32"), default)]
    TCP,
    /// SSL/TLS
    SSL,
    /// Insecure WebSocket.
    WS,
    /// Secure WebSocket.
    #[cfg_attr(target_arch = "wasm32", default)]
    WSS,
}
