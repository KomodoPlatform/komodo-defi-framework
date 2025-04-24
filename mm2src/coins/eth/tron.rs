//! Minimal Tron placeholders for EthCoin integration.
//! These types will be expanded with full TRON logic in later steps.

mod address;
pub use address::Address as TronAddress;

/// Represents TRON chain/network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Network {
    Mainnet,
    Shasta,
    Nile,
    // TODO: Add more networks as needed.
}

/// Placeholder for a TRON client.
#[derive(Debug, Clone)]
pub struct TronClient;

/// Placeholder for TRON fee params.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronFeeParams {
    // TODO: Add TRON-specific fields in future steps.
}
