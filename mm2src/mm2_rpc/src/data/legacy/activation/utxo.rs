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
#[derive(Default)]
pub enum ElectrumProtocol {
    /// TCP
    #[default]
    TCP,
    /// SSL/TLS
    SSL,
    /// Insecure WebSocket.
    WS,
    /// Secure WebSocket.
    WSS,
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg(target_arch = "wasm32")]
impl Default for ElectrumProtocol {
    fn default() -> Self { ElectrumProtocol::WS }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
/// The priority of an electrum connection when selective policy is in effect.
///
/// Primary connections are considered first and only if all of them are faulty
/// will the secondary connections be considered.
pub enum Priority {
    Primary,
    #[default]
    Secondary,
}
