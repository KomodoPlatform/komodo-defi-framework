use common::serde_derive::{Deserialize, Serialize};

/// Using tagged representation to allow adding variants with coefficients, percentage, etc in the future.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "policy", content = "additional_data")]
pub enum GasStationPricePolicy {
    /// Use mean between average and fast values, default and recommended to use on ETH mainnet due to
    /// gas price big spikes.
    MeanAverageFast,
    /// Use average value only. Useful for non-heavily congested networks (Matic, etc.)
    Average,
}

impl Default for GasStationPricePolicy {
    fn default() -> Self { GasStationPricePolicy::MeanAverageFast }
}
