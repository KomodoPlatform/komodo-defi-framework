//! Minimal Tron placeholders for EthCoin integration.
//! These types will be expanded with full TRON logic in later steps.

mod address;
pub use address::Address as TronAddress;

/// Represents TRON chain/network.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Network {
    Mainnet,
    Shasta,
    Nile,
    // TODO: Add more networks as needed.
}

/// Placeholder for a TRON client.
#[derive(Clone, Debug)]
pub struct TronClient;

/// Placeholder for TRON fee params.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TronFeeParams {
    // TODO: Add TRON-specific fields in future steps.
}
