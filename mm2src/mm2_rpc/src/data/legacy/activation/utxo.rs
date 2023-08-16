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
#[derive(Clone, Debug, Deserialize, Serialize)]
/// Deserializable Electrum protocol representation for RPC
pub enum ElectrumProtocol {
    /// TCP
    TCP,
    /// SSL/TLS
    SSL,
    /// Insecure WebSocket.
    WS,
    /// Secure WebSocket.
    WSS,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for ElectrumProtocol {
    fn default() -> Self { ElectrumProtocol::TCP }
}

#[cfg(target_arch = "wasm32")]
impl Default for ElectrumProtocol {
    fn default() -> Self { ElectrumProtocol::WS }
}
