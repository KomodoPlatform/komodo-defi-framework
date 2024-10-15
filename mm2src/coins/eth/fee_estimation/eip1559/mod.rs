//! Provides estimations of base and priority fee per gas or fetch estimations from a gas api provider
pub mod block_native;
pub mod infura;
pub mod simple;

use ethereum_types::U256;
use url::Url;

const FEE_PER_GAS_LEVELS: usize = 3;

/// Indicates which provider was used to get fee per gas estimations
#[derive(Clone, Debug)]
pub enum EstimationSource {
    /// filled by default values
    Empty,
    /// internal simple estimator
    Simple,
    Infura,
    Blocknative,
}

impl ToString for EstimationSource {
    fn to_string(&self) -> String {
        match self {
            EstimationSource::Empty => "empty".into(),
            EstimationSource::Simple => "simple".into(),
            EstimationSource::Infura => "infura".into(),
            EstimationSource::Blocknative => "blocknative".into(),
        }
    }
}

impl Default for EstimationSource {
    fn default() -> Self { Self::Empty }
}

enum PriorityLevelId {
    Low = 0,
    Medium = 1,
    High = 2,
}

/// Supported gas api providers
#[derive(Clone, Deserialize)]
pub enum GasApiProvider {
    Infura,
    Blocknative,
}

#[derive(Clone, Deserialize)]
pub struct GasApiConfig {
    /// gas api provider name to use
    pub provider: GasApiProvider,
    /// gas api provider or proxy base url (scheme, host and port without the relative part)
    pub url: Url,
}

/// Priority level estimated max fee per gas
#[derive(Clone, Debug, Default)]
pub struct FeePerGasLevel {
    /// estimated max priority tip fee per gas in wei
    pub max_priority_fee_per_gas: U256,
    /// estimated max fee per gas in wei
    pub max_fee_per_gas: U256,
    /// estimated transaction min wait time in mempool in ms for this priority level
    pub min_wait_time: Option<u32>,
    /// estimated transaction max wait time in mempool in ms for this priority level
    pub max_wait_time: Option<u32>,
}

/// Internal struct for estimated fee per gas for several priority levels, in wei
/// low/medium/high levels are supported
#[derive(Default, Debug, Clone)]
pub struct FeePerGasEstimated {
    /// base fee for the next block in wei
    pub base_fee: U256,
    /// estimated low priority fee
    pub low: FeePerGasLevel,
    /// estimated medium priority fee
    pub medium: FeePerGasLevel,
    /// estimated high priority fee
    pub high: FeePerGasLevel,
    /// which estimator used
    pub source: EstimationSource,
    /// base trend (up or down)
    pub base_fee_trend: String,
    /// priority trend (up or down)
    pub priority_fee_trend: String,
}
